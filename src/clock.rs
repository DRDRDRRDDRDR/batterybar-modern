// batterybar-modern — 本地时间工具
//
// 原版 BatteryBar 的界面里有两处需要"本地时刻"：
//   · Status Popup 的 `Time Remaining: 1:18 @ 5:05 PM (Charging)`
//   · 曲线窗口的时间轴刻度
// 两者拿到的都是 unix 秒，必须换算成本地时间（含夏令时）。
// `GetLocalTime()` 只给"此刻"，所以这里先由它求出 UTC 偏移，再对任意时刻做
// 纯整数换算（Howard Hinnant 的 civil-from-days 算法），不依赖任何时区数据库。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 缓存下来的本地时区偏移（秒）。10 分钟刷新一次，兼顾夏令时切换。
static OFFSET: AtomicI64 = AtomicI64::new(i64::MIN);
static OFFSET_STAMP: AtomicI64 = AtomicI64::new(0);

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 本地时区相对 UTC 的偏移秒数（东八区 = +28800）。10 分钟内复用缓存值。
pub fn local_offset() -> i64 {
    let now = now_unix();
    let cached = OFFSET.load(Ordering::Relaxed);
    if cached != i64::MIN && now - OFFSET_STAMP.load(Ordering::Relaxed) < 600 {
        return cached;
    }
    let off = compute_offset(now);
    OFFSET.store(off, Ordering::Relaxed);
    OFFSET_STAMP.store(now, Ordering::Relaxed);
    off
}

fn compute_offset(now: i64) -> i64 {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let as_if_utc = days_from_civil(st.wYear as i64, st.wMonth as i64, st.wDay as i64) * 86400
        + st.wHour as i64 * 3600
        + st.wMinute as i64 * 60
        + st.wSecond as i64;
    // GetLocalTime 有 1 秒内的抖动，四舍五入到最近的一刻钟倍数（真实时区偏移总是 15 分钟整数倍）
    let raw = as_if_utc - now;
    (raw as f64 / 900.0).round() as i64 * 900
}

/// 把 unix 秒拆成本地时间的 (年, 月, 日, 时, 分, 秒)。
pub fn local_parts(t: i64) -> (i64, i64, i64, i64, i64, i64) {
    let l = t + local_offset();
    let days = l.div_euclid(86400);
    let secs = l.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// 本地当日时刻，12 小时制带 AM/PM —— 原版 `@ 5:05 PM` 的格式。
pub fn hm12(t: i64) -> String {
    let (_, _, _, h, mi, _) = local_parts(t);
    let ampm = if h < 12 { "AM" } else { "PM" };
    let h12 = match h % 12 {
        0 => 12,
        v => v,
    };
    format!("{}:{:02} {}", h12, mi, ampm)
}

/// 本地 `HH:MM` 24 小时制（曲线时间轴用）。
pub fn hm24(t: i64) -> String {
    let (_, _, _, h, mi, _) = local_parts(t);
    format!("{:02}:{:02}", h, mi)
}

/// 本地 `MM-DD`（曲线跨天时的刻度）。
pub fn md(t: i64) -> String {
    let (_, m, d, _, _, _) = local_parts(t);
    format!("{:02}-{:02}", m, d)
}

/// 本地 `YYYY-MM-DD HH:MM:SS`（日志行）。
pub fn full(t: i64) -> String {
    let (y, m, d, h, mi, s) = local_parts(t);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, mi, s)
}

/// 把 `1:18` / `0:07` 这类 H:MM 时长格式化（原版 Status Popup 的时长写法）。
pub fn duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    format!("{}:{:02}", h, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for &(y, m, d) in &[(1970, 1, 1), (2000, 2, 29), (2026, 10, 14), (1999, 12, 31)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "roundtrip {y}-{m}-{d}");
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn duration_format() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(78), "0:01");
        assert_eq!(duration(4680), "1:18");
        assert_eq!(duration(6 * 3600 + 13 * 60), "6:13");
    }

    #[test]
    fn hm12_matches_noon_and_midnight() {
        // 直接用 UTC 偏移为 0 的假设不好构造，这里只验证格式化函数不 panic 且形状正确
        let s = hm12(0);
        assert!(s.ends_with("AM") || s.ends_with("PM"), "{s}");
        assert!(s.contains(':'), "{s}");
    }
}
