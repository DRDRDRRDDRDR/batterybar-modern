// batterybar-modern — 电源数据层
// WinRT AggregateBattery:BatteryReport(推送) + 充放电 ETA 估算。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Devices::Power::{Battery, BatteryReport};
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};
use windows_core::{Interface, PCWSTR, Ref};

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
}

impl EtaTracker {
    const fn new() -> Self {
        Self { last_sec: 0, last_mwh: 0, rate_mwh_per_s: 0.0, charging: false }
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
        let rate = self.rate_mwh_per_s;
        if rate.abs() < 1e-6 {
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
        let eta = (rem as f64 / rate.abs()) as u64;
        eta.min(86_400) // 上限 24h，超出视为不可靠
    }
}

static ETA: std::sync::Mutex<EtaTracker> = std::sync::Mutex::new(EtaTracker::new());

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
    if full > 0 && rem >= 0 {
        let pct = ((rem as f64 / full as f64) * 100.0).round().clamp(0.0, 100.0) as i32;
        PERCENT.store(pct, Ordering::Relaxed);
    }

    if let Ok(mut t) = ETA.lock() {
        let eta = t.feed(now_unix(), rem, charging || idle_ac);
        ETA_SECONDS.store(eta, Ordering::Relaxed);
    }
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

/// 兜底轮询（在 WM_TIMER 里调用）：WinRT 事件偶发丢失时保证 30s 内必有刷新。
pub fn poll_once() {
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
