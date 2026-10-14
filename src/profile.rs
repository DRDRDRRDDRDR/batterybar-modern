// batterybar-modern — 电池曲线数据层
//
// 对应原版 BatteryBar Pro 的「Battery Profile Graph」功能：把电池状态按时间
// 采样、落盘成 profile.csv，供 graph.rs 画放电/充电曲线。
//
// 睡眠/唤醒处理（对齐原版 Release Notes「Added detection when the computer goes
// into suspend mode or resumes, which helps make sure that the data recorded about
// your battery usage is accurate」）：相邻样本时间差超过 GAP_LIMIT 秒时判定发生了
// 睡眠，曲线在该处**断开**，不把睡眠时长算进放电时长。
#![allow(non_snake_case)]

use std::path::PathBuf;
use std::sync::Mutex;

/// 相邻两点超过这个秒数就认为中间发生了睡眠/休眠 —— 曲线断开。
pub const GAP_LIMIT: u64 = 300;
/// 采样节流：最短间隔（秒）。状态跳变（充放电切换、电量变化 ≥1%）时立即补记。
const SAMPLE_INTERVAL: u64 = 20;
/// 内存中保留的最大样本数。
pub const MAX_SAMPLES: usize = 8000;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub t: u64,        // unix 秒
    pub pct: i32,      // 电量 %
    pub rem: i64,      // 剩余容量 mWh
    pub full: i64,     // 满充容量 mWh
    pub rate_mw: i32,  // 充/放电功率（正=放电）
    pub charging: bool, // true = 充电中（含接交流电已满）
}

static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());
static LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST: Mutex<Option<Sample>> = Mutex::new(None);

fn profile_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    p.pop();
    p.push("profile.csv");
    p
}

/// 从 profile.csv 载入内存（进程内只做一次）。格式：`t,pct,rem,full,rate,charging`
fn ensure_loaded() {
    use std::sync::atomic::Ordering;
    if LOADED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(profile_path()) else { return };
    let mut v: Vec<Sample> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 6 {
            continue;
        }
        let Ok(t) = f[0].trim().parse::<u64>() else { continue };
        v.push(Sample {
            t,
            pct: f[1].trim().parse().unwrap_or(0),
            rem: f[2].trim().parse().unwrap_or(0),
            full: f[3].trim().parse().unwrap_or(0),
            rate_mw: f[4].trim().parse().unwrap_or(0),
            charging: f[5].trim() == "1",
        });
    }
    if v.len() > MAX_SAMPLES {
        v = v[v.len() - MAX_SAMPLES..].to_vec();
    }
    if let Ok(mut s) = SAMPLES.lock() {
        *s = v;
    }
}

/// 记一个采样点。节流：距上一点不足 SAMPLE_INTERVAL 秒且状态无跳变时跳过。
pub fn record(t: u64, pct: i32, rem: i64, full: i64, rate_mw: i32, charging: bool) {
    ensure_loaded();
    let s = Sample { t, pct, rem, full, rate_mw, charging };

    let mut last = match LAST.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(p) = *last {
        let dt = t.saturating_sub(p.t);
        let jumped = p.charging != s.charging || (p.pct - s.pct).abs() >= 1;
        // 睡眠跨越：强制记一点，让曲线在 gap 两侧各有端点，画得出来断口。
        let slept = dt > GAP_LIMIT;
        if !slept && !jumped && dt < SAMPLE_INTERVAL {
            return;
        }
    }
    *last = Some(s);
    drop(last);

    let trimmed: Option<Vec<Sample>> = {
        let mut v = match SAMPLES.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        v.push(s);
        if v.len() > MAX_SAMPLES {
            let drop_n = v.len() - MAX_SAMPLES;
            Some(v.split_off(drop_n)) // 返回被移除的旧样本，用于重写 CSV
        } else {
            None
        }
    };

    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(profile_path()) {
        let _ = writeln!(f, "{},{},{},{},{},{}", s.t, s.pct, s.rem, s.full, s.rate_mw, s.charging as u8);
    }
    // 超出上限才重写整个文件（很少发生）
    if trimmed.is_some() {
        rewrite_all();
    }
}

fn rewrite_all() {
    use std::io::Write;
    let Ok(v) = SAMPLES.lock().map(|g| g.clone()) else { return };
    let Ok(mut f) = std::fs::File::create(profile_path()) else { return };
    for s in v {
        let _ = writeln!(f, "{},{},{},{},{},{}", s.t, s.pct, s.rem, s.full, s.rate_mw, s.charging as u8);
    }
}

/// 供曲线窗口读取的样本快照。
pub fn snapshot() -> Vec<Sample> {
    ensure_loaded();
    SAMPLES.lock().map(|g| g.clone()).unwrap_or_default()
}

/// 样本总数（不克隆，供界面显示计数）。
pub fn count() -> usize {
    ensure_loaded();
    SAMPLES.lock().map(|g| g.len()).unwrap_or(0)
}

/// 清空曲线数据（Tools 页「Reset battery profile data」）。
pub fn clear() {
    if let Ok(mut v) = SAMPLES.lock() {
        v.clear();
    }
    if let Ok(mut l) = LAST.lock() {
        *l = None;
    }
    let _ = std::fs::remove_file(profile_path());
}

/// 把样本按睡眠断口切成若干连续段。每段是一条要连线的折线。
pub fn segments(v: &[Sample], charging_wanted: bool) -> Vec<Vec<Sample>> {
    let mut out: Vec<Vec<Sample>> = Vec::new();
    let mut cur: Vec<Sample> = Vec::new();
    for s in v.iter().copied().filter(|s| s.charging == charging_wanted) {
        if let Some(p) = cur.last() {
            if s.t.saturating_sub(p.t) > GAP_LIMIT {
                if cur.len() >= 2 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
        cur.push(s);
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// 统计信息（Tools 页与曲线窗口标题栏展示）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub samples: usize,
    pub discharge_secs: u64,
    pub charge_secs: u64,
    pub min_pct: i32,
    pub max_pct: i32,
    pub avg_discharge_mw: i32,
}

pub fn stats() -> Stats {
    let v = snapshot();
    let mut st = Stats { samples: v.len(), min_pct: 101, max_pct: -1, ..Default::default() };
    let mut dsum: i64 = 0;
    let mut dn: i64 = 0;
    for w in v.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dt = b.t.saturating_sub(a.t);
        if dt > GAP_LIMIT {
            continue; // 跨睡眠，不计时长
        }
        if b.charging {
            st.charge_secs += dt;
        } else {
            st.discharge_secs += dt;
            dsum += b.rate_mw.max(0) as i64;
            dn += 1;
        }
    }
    for s in &v {
        st.min_pct = st.min_pct.min(s.pct);
        st.max_pct = st.max_pct.max(s.pct);
    }
    if st.max_pct < 0 {
        st.min_pct = 0;
    }
    if dn > 0 {
        st.avg_discharge_mw = (dsum / dn) as i32;
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: u64, pct: i32, charging: bool) -> Sample {
        Sample { t, pct, rem: 0, full: 1000, rate_mw: 100, charging }
    }

    #[test]
    fn segments_break_on_sleep_gap() {
        let v = vec![
            s(0, 90, false),
            s(20, 89, false),
            s(40, 88, false),
            s(40 + GAP_LIMIT + 1, 80, false), // 睡眠跨越
            s(40 + GAP_LIMIT + 21, 79, false),
        ];
        let segs = segments(&v, false);
        assert_eq!(segs.len(), 2, "睡眠断口应切成两段");
        assert_eq!(segs[0].len(), 3);
        assert_eq!(segs[1].len(), 2);
    }

    #[test]
    fn segments_split_by_charging_state() {
        let v = vec![s(0, 90, false), s(20, 89, false), s(40, 90, true), s(60, 91, true)];
        assert_eq!(segments(&v, false).len(), 1);
        assert_eq!(segments(&v, true).len(), 1);
        let all = segments(&v, false);
        assert!(all[0].iter().all(|x| !x.charging));
    }

    #[test]
    fn segments_drop_singletons() {
        let v = vec![s(0, 90, false), s(1000, 89, false)];
        // 两点相隔超 GAP_LIMIT，各自成段但长度 < 2 ⇒ 都不保留
        assert!(segments(&v, false).is_empty());
    }
}
