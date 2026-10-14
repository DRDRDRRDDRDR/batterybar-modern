// batterybar-modern — 电源数据层
// WinRT AggregateBattery:BatteryReport(推送) + 充放电 ETA 估算。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Devices::Power::{Battery, BatteryReport};
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};
use windows_core::{PCWSTR, Ref};

use crate::profile;

pub const POWERDATA_CHANGED_MSG: &str = "BatteryBarModern.PowerDataChanged";

// ---- 全局快照（UI 线程读取，事件线程写入，全部原子量）----

pub static PERCENT: AtomicI32 = AtomicI32::new(-1); // -1 = 未知
pub static CHARGING: AtomicBool = AtomicBool::new(false);
pub static CHARGE_RATE_MW: AtomicI32 = AtomicI32::new(0); // WinRT 约定：正=放电, 负=充电
pub static REMAIN_MWH: AtomicI64 = AtomicI64::new(0);
pub static FULL_MWH: AtomicI64 = AtomicI64::new(0);
pub static DESIGN_MWH: AtomicI64 = AtomicI64::new(0);
pub static ETA_SECONDS: AtomicU64 = AtomicU64::new(0); // 0 = 未知/满电
pub static AVAILABLE: AtomicBool = AtomicBool::new(false); // 本机有无电池
/// 电池硬件标识，取自 WinRT 的 Battery.DeviceId，是一个设备实例路径。
/// 原版 Display 页可让它在状态弹窗里显示。
pub static BATTERY_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub static FULL_LIFETIME_SECONDS: AtomicU64 = AtomicU64::new(0); // 满充可用时长(学习值)
// ---- 当前充/放电段的时间账（对应原版 Status Popup 的 Elapsed Time）----
/// 本段起点 unix 秒（0 = 尚无段）
pub static SESSION_START_SEC: AtomicU64 = AtomicU64::new(0);
/// 本段起点电量 %（对应原版 `(since 70.4%)`）
pub static SESSION_START_PCT: AtomicI32 = AtomicI32::new(-1);
/// 本段已经历秒数（放电中=已放电时长，充电中=已充电时长）
pub static ELAPSED_SECONDS: AtomicU64 = AtomicU64::new(0);
/// 预计充满（充电中）或放空（放电中）的 unix 时刻；0 = 未知
pub static FINISH_AT: AtomicU64 = AtomicU64::new(0);
/// 本段是什么状态：true = 充电段
pub static SESSION_CHARGING: AtomicBool = AtomicBool::new(true);
/// Win32 ACLineStatus: 1=接通外部电源，0=使用电池（255=未知）
pub static AC_ON: AtomicBool = AtomicBool::new(true);

/// 全寿命自学习预测算法开关（Preferences → BATTERY TOOLS）。
/// 关闭时不记录充放电历史、不计算学习值，ETA 只走系统/EMA 估算。
pub static LEARN_ENABLED: AtomicBool = AtomicBool::new(true);

// ---- ETA 估算（指数滑动平均，防跳变）----

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct EtaTracker {
    last_sec: u64,
    last_mwh: i64,
    rate_mwh_per_s: f64, // 滑动平均；充电为正
    charging: bool,
    discharge_start: Option<(u64, i64)>, // (开始秒, 开始mWh)
}

impl EtaTracker {
    const fn new() -> Self {
        Self { last_sec: 0, last_mwh: 0, rate_mwh_per_s: 0.0, charging: false, discharge_start: None }
    }

    fn feed(&mut self, sec: u64, mwh: i64, charging: bool) -> u64 {
        if self.last_sec == 0 || charging != self.charging {
            self.last_sec = sec;
            self.last_mwh = mwh;
            self.charging = charging;
            return 0;
        }
        let dt = sec.saturating_sub(self.last_sec);
        if dt < 5 {
            return self.current_eta();
        }
        let dm = mwh - self.last_mwh; // 充电为正
        let inst = dm as f64 / dt as f64;
        // EMA，半衰期 60s
        if self.rate_mwh_per_s == 0.0 {
            self.rate_mwh_per_s = inst;
        } else {
            let a = 1.0 - 0.5_f64.powf(dt as f64 / 60.0);
            self.rate_mwh_per_s = a * inst + (1.0 - a) * self.rate_mwh_per_s;
        }
        self.last_sec = sec;
        self.last_mwh = mwh;
        self.current_eta()
    }

    fn current_eta(&self) -> u64 {
        let mut rate = self.rate_mwh_per_s.abs();
        // 全寿命自学习：放电时把历史平均速率与实时 EMA 各半融合。
        // 目的：刚拔电源的头几分钟 EMA 还在爬坡，纯 EMA 的 ETA 会剧烈跳动；
        // 混入历史值后 ETA 从一开始就落在合理区间。
        if !self.charging && LEARN_ENABLED.load(Ordering::Relaxed) {
            let learned = learned_rate_cached();
            if learned > 1e-6 {
                rate = if rate < 1e-6 { learned } else { rate * 0.5 + learned * 0.5 };
            }
        }
        if rate < 1e-6 {
            return 0;
        }
        let rem = if self.charging {
            FULL_MWH.load(Ordering::Relaxed) - self.last_mwh
        } else {
            self.last_mwh
        };
        if rem <= 0 {
            return 0;
        }
        let eta = (rem as f64 / rate) as u64;
        eta.min(86_400) // 上限 24h，超出视为不可靠
    }

    /// 放电区段记录：充电开始时把进行中的放电段写入历史文件。
    fn track_discharge(&mut self, sec: u64, mwh: i64, charging: bool, full: i64) {
        if charging {
            if let Some(rec) = self.discharge_start.take() {
                // 放电结束：落一条记录
                if sec > rec.0 && full > 0 {
                    let d = DischargeRecord {
                        from_mwh: rec.1,
                        to_mwh: mwh,
                        seconds: sec - rec.0,
                    };
                    if d.seconds > 300 && d.to_mwh < d.from_mwh {
                        append_history(&d);
                    }
                }
            }
        } else if self.discharge_start.is_none() {
            self.discharge_start = Some((sec, mwh));
        } else {
            // 进行中：更新 last_sec 供 ELAPSED 使用
            self.last_sec = sec;
        }
    }
}

static ETA: std::sync::Mutex<EtaTracker> = std::sync::Mutex::new(EtaTracker::new());

// 历史学习：记录最近 N 次完整放电的 (起始%, 结束%, 秒数) 用于全寿命估算。
// 持久化到 exe 同目录 battery_history.json（极简格式）。
const HISTORY_MAX: usize = 32;
struct DischargeRecord {
    from_mwh: i64,
    to_mwh: i64,
    seconds: u64,
}

/// 把 BatteryReport 写入全局快照。
pub fn apply_report(report: &BatteryReport) {
    let status = report.Status().map(|s| s.0).unwrap_or(0);
    // BatteryStatus: 0=NotPresent 1=Discharging 2=Idle 3=Charging
    let present = status != 0;
    AVAILABLE.store(present, Ordering::Relaxed);
    if !present {
        PERCENT.store(-1, Ordering::Relaxed);
        return;
    }
    let charging = status == 3;
    let idle_ac = status == 2; // 接交流电但不在充电（满电）
    CHARGING.store(charging || idle_ac, Ordering::Relaxed);

    if let Ok(r) = report.RemainingCapacityInMilliwattHours() {
        if let Ok(v) = r.Value() {
            REMAIN_MWH.store(v as i64, Ordering::Relaxed);
        }
    }
    if let Ok(r) = report.FullChargeCapacityInMilliwattHours() {
        if let Ok(v) = r.Value() {
            FULL_MWH.store(v as i64, Ordering::Relaxed);
        }
    }
    if let Ok(r) = report.DesignCapacityInMilliwattHours() {
        if let Ok(v) = r.Value() {
            DESIGN_MWH.store(v as i64, Ordering::Relaxed);
        }
    }
    if let Ok(r) = report.ChargeRateInMilliwatts() {
        if let Ok(v) = r.Value() {
            CHARGE_RATE_MW.store(v, Ordering::Relaxed);
        }
    }

    let full = FULL_MWH.load(Ordering::Relaxed);
    let rem = REMAIN_MWH.load(Ordering::Relaxed);
    let now = now_unix();
    if full > 0 && rem >= 0 {
        let pct = ((rem as f64 / full as f64) * 100.0).round().clamp(0.0, 100.0) as i32;
        PERCENT.store(pct, Ordering::Relaxed);
    } else {
        sync_win32_power();
    }
    let pct = PERCENT.load(Ordering::Relaxed);
    let on_ac = charging || idle_ac;

    // ---- 当前充/放电段（原版 Status Popup 的 Elapsed Time + since X%）----
    // 段的定义：充放电状态一旦翻转就开新段；起点记下当时的电量。
    let start = SESSION_START_SEC.load(Ordering::Relaxed);
    if start == 0 || SESSION_CHARGING.load(Ordering::Relaxed) != on_ac {
        SESSION_START_SEC.store(now, Ordering::Relaxed);
        SESSION_START_PCT.store(pct, Ordering::Relaxed);
        SESSION_CHARGING.store(on_ac, Ordering::Relaxed);
        ELAPSED_SECONDS.store(0, Ordering::Relaxed);
    } else {
        ELAPSED_SECONDS.store(now.saturating_sub(start), Ordering::Relaxed);
    }

    if let Ok(mut t) = ETA.lock() {
        let eta = t.feed(now, rem, on_ac);
        ETA_SECONDS.store(eta, Ordering::Relaxed);
        // 预计充满/放空的当日时刻（原版 3.6.4「Show the time of day that the
        // battery will be fully (dis)charged (e.g, @ 5:23 PM)」）
        FINISH_AT.store(if eta > 0 { now.saturating_add(eta) } else { 0 }, Ordering::Relaxed);
        // 全寿命学习值：EMA 的稳定速率下满电可用秒数（学习算法关闭时不写入）
        if LEARN_ENABLED.load(Ordering::Relaxed) && t.rate_mwh_per_s > 1.0 && full > 0 {
            let full_life = (full as f64 / t.rate_mwh_per_s) as u64;
            FULL_LIFETIME_SECONDS.store(full_life.min(86_400 * 2), Ordering::Relaxed);
        }
        // 放电记录（充→放切换时开新记录，放→充切换时落盘）；学习关闭则不落盘
        if LEARN_ENABLED.load(Ordering::Relaxed) {
            t.track_discharge(now, rem, on_ac, full);
        }
    }

    // ---- Battery Profile Graph 的采样（画曲线用，始终记）----
    // 与「Log battery data」分开：前者是 Pro 的曲线功能数据源，后者是给人看的文本日志。
    profile::record(now, pct, rem, full, CHARGE_RATE_MW.load(Ordering::Relaxed), on_ac);
    // ---- Tools 页「Log battery data」→ BatteryStatus.log ----
    if LOG_TO_FILE.load(Ordering::Relaxed) {
        append_status_log(now, pct, rem, full, on_ac);
    }
}

/// Tools 页「Log battery data」开关（原版写到 BatteryStatus.log）。
pub static LOG_TO_FILE: AtomicBool = AtomicBool::new(false);

fn status_log_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("."));
    p.pop();
    p.push("BatteryStatus.log");
    p
}

/// 保留最近约 2000 行，超了就截断重写（原版也会控制日志体积）。
fn append_status_log(t: u64, pct: i32, rem: i64, full: i64, on_ac: bool) {
    use std::io::Write;
    const MAX_LINES: usize = 2000;
    let stamp = local_stamp(t);
    let rate = CHARGE_RATE_MW.load(Ordering::Relaxed);
    let line = format!(
        "{stamp}\tpercent={pct}%\tremain={rem}mWh\tfull={full}mWh\trate={rate}mW\tac={}\n",
        on_ac as u8
    );
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(status_log_path())
    else {
        return;
    };
    let _ = f.write_all(line.as_bytes());
    drop(f);
    if let Ok(text) = std::fs::read_to_string(status_log_path()) {
        let n = text.lines().count();
        if n > MAX_LINES {
            let keep: Vec<&str> = text.lines().skip(n - MAX_LINES).collect();
            let _ = std::fs::write(status_log_path(), keep.join("\n") + "\n");
        }
    }
}

fn local_stamp(t: u64) -> String {
    crate::clock::full(t as i64)
}

/// 满电可用时长，按原版用 Soft minimum / Full battery level 两个电平修正：
/// 从"满"放到"软下限"的可用容量 ÷ 当前速率。
/// `soft_min` 与 `full_level` 来自 Warning Levels 页。
pub fn full_runtime_seconds(soft_min: i32, full_level: i32) -> u64 {
    let full = FULL_MWH.load(Ordering::Relaxed);
    if full <= 0 {
        return 0;
    }
    let usable_frac =
        ((full_level.clamp(50, 100) - soft_min.clamp(0, 50)) as f64 / 100.0).clamp(0.01, 1.0);
    let usable_mwh = full as f64 * usable_frac;
    // 速率优先用历史学习值（有则更稳），否则用实时 EMA
    let rate = if LEARN_ENABLED.load(Ordering::Relaxed) {
        let l = learned_rate_cached();
        if l > 1e-6 {
            l
        } else {
            ETA.lock().map(|t| t.rate_mwh_per_s.abs()).unwrap_or(0.0)
        }
    } else {
        ETA.lock().map(|t| t.rate_mwh_per_s.abs()).unwrap_or(0.0)
    };
    if rate < 1e-6 {
        return 0;
    }
    ((usable_mwh / rate) as u64).min(86_400 * 4)
}

/// 在 UI 线程调用一次（注册消息 + 订阅事件 + 首次取报告）。
/// WinRT 事件回调来自线程池线程，通过 PostMessage 通知 UI 线程刷新。
pub fn init_battery(notify_hwnd: HWND) -> windows_core::Result<()> {
    let wide: Vec<u16> = POWERDATA_CHANGED_MSG
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let msg = unsafe { RegisterWindowMessageW(PCWSTR::from_raw(wide.as_ptr())) };

    let battery = Battery::AggregateBattery()?;
    let hwnd = notify_hwnd.0 as isize;

    // 电池硬件标识。AggregateBattery 在同时装多块电池时返回的是聚合对象，
    // DeviceId 会拿不到；此时保持未初始化，弹窗退回按容量构造的伪 ID。
    if let Ok(id) = battery.DeviceId() {
        let s = id.to_string();
        if !s.is_empty() {
            let _ = BATTERY_ID.set(s);
        }
    }

    // 先立即用系统内核同步一次基础电量（确保持久保底可用）
    sync_win32_power();

    // 立即取一次当前报告
    if let Ok(r) = battery.GetReport() {
        apply_report(&r);
    }

    let handler = TypedEventHandler::new(
        move |sender: Ref<'_, Battery>, _args: Ref<'_, windows_core::IInspectable>| {
            // sender 为 null 时（罕见）直接返回，下一轮 30s 兜底轮询会补
            if let Some(s) = sender.as_ref() {
                if let Ok(r) = s.GetReport() {
                    apply_report(&r);
                }
            }
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut _)),
                    msg,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            Ok(())
        },
    );
    battery.ReportUpdated(&handler)?;
    Ok(())
}

/// 兜底 Win32 电源同步（直接读取系统内核，不依赖 WinRT 驱动汇报完整性）
pub fn sync_win32_power() {
    unsafe {
        let mut sps = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        if windows::Win32::System::Power::GetSystemPowerStatus(&mut sps).is_ok() {
            if sps.BatteryLifePercent != 255 {
                PERCENT.store(sps.BatteryLifePercent as i32, Ordering::Relaxed);
            }
            if sps.BatteryFlag & 0x10 == 0 {
                // 0x10 = no system battery
                AVAILABLE.store(true, Ordering::Relaxed);
            }
            if sps.ACLineStatus != 255 {
                AC_ON.store(sps.ACLineStatus == 1, Ordering::Relaxed);
            }
            if sps.ACLineStatus == 1 || (sps.BatteryFlag & 8) != 0 {
                CHARGING.store(true, Ordering::Relaxed);
            } else if sps.ACLineStatus == 0 {
                CHARGING.store(false, Ordering::Relaxed);
            }
            if sps.BatteryLifeTime != u32::MAX && sps.BatteryLifeTime > 0 {
                ETA_SECONDS.store(sps.BatteryLifeTime as u64, Ordering::Relaxed);
            }
        }
    }
}

/// 兜底轮询（在 WM_TIMER 里调用）：WinRT 事件偶发丢失时保证 30s 内必有刷新。
pub fn poll_once() {
    sync_win32_power();
    if let Ok(b) = Battery::AggregateBattery() {
        if let Ok(r) = b.GetReport() {
            apply_report(&r);
        }
    }
}

/// 电池健康度 = 满充容量 / 设计容量
pub fn health_percent() -> i32 {
    let full = FULL_MWH.load(Ordering::Relaxed);
    let design = DESIGN_MWH.load(Ordering::Relaxed);
    if full > 0 && design > 0 {
        ((full as f64 / design as f64) * 100.0).round().clamp(1.0, 100.0) as i32
    } else {
        0
    }
}

/// 剩余容量，单位 mWh，带脏值兜底。
///
/// WinRT 的 RemainingCapacity 在部分机型上会给出与电量百分比严重不符的读数，
/// 实测满电时只有 1,884 mWh 而满充容量是 61,883 mWh。判断依据是剩余容量不应
/// 显著低于「满充容量 × 百分比」的一半；一旦触发就按百分比折算，避免界面出现
/// 「100% 却只剩 1.9Wh」这种自相矛盾的数字。
pub fn remaining_mwh(pct: i32) -> i64 {
    let rem = REMAIN_MWH.load(Ordering::Relaxed);
    let full = FULL_MWH.load(Ordering::Relaxed);
    if full <= 0 {
        return rem;
    }
    let expect = (full as f64) * (pct as f64) / 100.0;
    if rem > 0 && (rem as f64) >= expect * 0.5 {
        rem
    } else {
        (full as i64) * (pct as i64) / 100
    }
}

// ---- 放电历史持久化（极简 JSONL：每行一条 JSON）----

fn history_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("."));
    p.pop();
    p.push("battery_history.jsonl");
    p
}

fn append_history(d: &DischargeRecord) {
    use std::io::Write;
    let line = format!(
        "{{\"from\":{},\"to\":{},\"sec\":{}}}\n",
        d.from_mwh, d.to_mwh, d.seconds
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_path())
    {
        let _ = f.write_all(line.as_bytes());
    }
    trim_history();
}

/// 超过 HISTORY_MAX 行删最旧的
fn trim_history() {
    let Ok(content) = std::fs::read_to_string(history_path()) else { return };
    let mut lines: Vec<&str> = content.lines().collect();
    if lines.len() <= HISTORY_MAX {
        return;
    }
    lines = lines[lines.len() - HISTORY_MAX..].to_vec();
    let _ = std::fs::write(history_path(), lines.join("\n") + "\n");
}

/// 历史平均放电速率的进程内缓存（单位 1e-6 mWh/s）。
/// learned_discharge_rate() 要读文件并逐行解析，而 current_eta() 每秒都会被调用；
/// 这里缓存 5 分钟，避免每秒一次磁盘 IO。
/// 放在模块级是因为 Tools 页「重置学习记录」要把缓存一并失效，否则清完文件
/// 后 5 分钟内仍会用旧值估时。
static HISTORY_CACHE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // -1 = 未初始化
static HISTORY_STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn learned_rate_cached() -> f64 {
    let now = now_unix();
    let age = now.saturating_sub(HISTORY_STAMP.load(Ordering::Relaxed));
    if HISTORY_CACHE.load(Ordering::Relaxed) >= 0 && age < 300 {
        return HISTORY_CACHE.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    }
    let v = learned_discharge_rate();
    HISTORY_CACHE.store((v * 1_000_000.0) as i64, Ordering::Relaxed);
    HISTORY_STAMP.store(now, Ordering::Relaxed);
    v
}

/// Tools 页「重置充放电学习历史记录」：删掉历史文件并让缓存立即失效。
pub fn reset_history() {
    let _ = std::fs::remove_file(history_path());
    HISTORY_CACHE.store(-1, Ordering::Relaxed);
    HISTORY_STAMP.store(0, Ordering::Relaxed);
}

/// 历史平均放电速率（mWh/s）；无数据返回 0
pub fn learned_discharge_rate() -> f64 {
    let Ok(content) = std::fs::read_to_string(history_path()) else { return 0.0 };
    let mut total_mwh = 0i64;
    let mut total_sec = 0u64;
    for line in content.lines().rev().take(HISTORY_MAX) {
        // 极简解析:{"from":N,"to":N,"sec":N}
        let get = |key: &str| -> i64 {
            line.split(&format!("\"{}\":", key))
                .nth(1)
                .and_then(|s| s.split(['}', ',']).next())
                .and_then(|s| s.trim().parse::<i64>().ok())
                .unwrap_or(0)
        };
        let from = get("from");
        let to = get("to");
        let sec = get("sec").max(1);
        total_mwh += (from - to).max(0);
        total_sec += sec as u64;
    }
    if total_sec > 0 {
        total_mwh as f64 / total_sec as f64
    } else {
        0.0
    }
}
