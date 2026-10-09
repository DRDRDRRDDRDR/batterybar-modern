// batterybar-modern — 入口与主窗口（任务栏嵌入条）
//
// 窗口：
//   1. bar window      —— 任务栏内嵌条（SetParent 进 Shell_TrayWnd）
//   2. message window  —— 托盘图标宿主（不可见）
//   3. popup window    —— Status Popup（原版 8 行字段复刻）
//   4. prefs window    —— Preferences（原版 Preferences 复刻）
#![allow(non_snake_case)]

mod config;
mod popup;
mod power;
mod prefs;
mod render;
mod taskbar;
mod tray;

use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_NOPREFIX, DT_SINGLELINE, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging as WAM;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetMessageW, GetParent, IsWindowVisible, KillTimer, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, ShowWindow, TrackPopupMenu,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, MENU_ITEM_FLAGS, SHOW_WINDOW_CMD, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSW, WS_CHILD, WS_VISIBLE,
};
use windows_core::{w, PCWSTR};

use crate::config::{Config, DisplayMode};
use crate::render::{palette_dark, palette_light, Palette};

// ---- 布局常量 ----
const BAR_WIDTH: i32 = 150;
const BAR_HEIGHT_PAD: i32 = 10;
const BAR_HEIGHT_VERTICAL: i32 = 92;
const TIMER_REFRESH: usize = 1;
const TIMER_HOVER: usize = 3;
const REFRESH_MS: u32 = 1000;
const HOVER_MS: u32 = 400;
const TRAY_TIMER: usize = 2;
const TRAY_POLL_MS: u32 = 30_000;

// 菜单 ID
const ID_MENU_MODE_TIME: usize = 0x1010;
const ID_MENU_MODE_PCT: usize = 0x1011;
const ID_MENU_MODE_RATE: usize = 0x1016;
const ID_MENU_MODE_CAP: usize = 0x1017;
const ID_MENU_MODE_ICON: usize = 0x1012;

const ID_MENU_SCHEME_BALANCED: usize = 0x1020;
const ID_MENU_SCHEME_SAVER: usize = 0x1021;
const ID_MENU_SCHEME_PERF: usize = 0x1022;

const ID_MENU_PREFS: usize = 0x1013;
const ID_MENU_TOGGLE: usize = 0x1014;
const ID_MENU_EXIT: usize = 0x1015;

// ---- 全局状态 ----
static BAR_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TRAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static REFRESH_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_SIG: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);
static DISPLAY_MODE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0); // 0=time 1=percent 2=rate 3=capacity 4=icon
static HOVER_TRACKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();

pub fn config_lock() -> &'static Mutex<Config> {
    CONFIG.get_or_init(|| Mutex::new(Config::load()))
}

pub fn set_mode(m: DisplayMode) {
    DISPLAY_MODE.store(match m {
        DisplayMode::TimeRemaining => 0,
        DisplayMode::Percent => 1,
        DisplayMode::DischargeRate => 2,
        DisplayMode::Capacity => 3,
        DisplayMode::Icon => 4,
    }, Ordering::Relaxed);
}

fn mode_now() -> DisplayMode {
    match DISPLAY_MODE.load(Ordering::Relaxed) {
        1 => DisplayMode::Percent,
        2 => DisplayMode::DischargeRate,
        3 => DisplayMode::Capacity,
        4 => DisplayMode::Icon,
        _ => DisplayMode::TimeRemaining,
    }
}

struct BarWnd {
    hwnd: HWND,
}

fn main() -> windows_core::Result<()> {
    render::enable_dpi_awareness();

    let hinstance = unsafe { GetModuleHandleW(PCWSTR::null())? };

    // 注册条窗口类
    let bar_class = w!("BBM_Bar");
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW | WAM::CS_DBLCLKS,
        lpfnWndProc: Some(bar_wndproc),
        hInstance: hinstance.into(),
        lpszClassName: PCWSTR::from_raw(bar_class.as_ptr()),
        ..Default::default()
    };
    unsafe {
        if RegisterClassW(&wc) == 0 {
            return Err(windows_core::Error::from_thread());
        }
    }

    // 注册托盘宿主类（隐藏消息窗口）
    let msg_class = w!("BBM_MsgHost");
    let wc2 = WNDCLASSW {
        lpfnWndProc: Some(msg_wndproc),
        hInstance: hinstance.into(),
        lpszClassName: PCWSTR::from_raw(msg_class.as_ptr()),
        ..Default::default()
    };
    unsafe {
        if RegisterClassW(&wc2) == 0 {
            return Err(windows_core::Error::from_thread());
        }
    }

    // 注册 Preferences 设置窗口类（BBM_Prefs）
    let prefs_class = w!("BBM_Prefs");
    let wc3 = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(prefs::prefs_wndproc),
        hInstance: hinstance.into(),
        lpszClassName: PCWSTR::from_raw(prefs_class.as_ptr()),
        ..Default::default()
    };
    unsafe {
        if RegisterClassW(&wc3) == 0 {
            return Err(windows_core::Error::from_thread());
        }
    }

    unsafe {
        // 1) 托盘宿主
        let msg_hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR::from_raw(msg_class.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        TRAY_HWND.store(msg_hwnd.0 as isize, Ordering::Relaxed);

        // 2) 条窗口（先挂宿主，嵌入成功后 SetParent 进任务栏）
        let bar_hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR::from_raw(bar_class.as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            BAR_WIDTH,
            40,
            Some(msg_hwnd),
            None,
            Some(hinstance.into()),
            None,
        )?;
        BAR_HWND.store(bar_hwnd.0 as isize, Ordering::Relaxed);

        // 3) 嵌入任务栏
        let mut embedded = false;
        if let Some(tb) = taskbar::find_taskbars().first() {
            if taskbar::reparent(bar_hwnd, tb.hwnd).is_ok() {
                embedded = true;
            }
        }
        if embedded {
            relayout(bar_hwnd);
        } else {
            // 嵌入失败：顶层悬浮在屏幕右下角，保证功能可用
            let (sw, sh) = render::screen_size();
            let _ = WAM::SetWindowPos(
                bar_hwnd,
                None,
                sw - BAR_WIDTH - 8,
                sh - 60,
                BAR_WIDTH,
                28,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        let _ = ShowWindow(bar_hwnd, SW_SHOW);

        // 4) 托盘图标
        if let Ok(t) = tray::TrayIcon::new(msg_hwnd) {
            std::mem::forget(t); // 常驻
        }

        // 5) 电池订阅 + 首次数据（UI 线程上做，回调线程只 PostMessage）
        if let Err(e) = power::init_battery(bar_hwnd) {
            eprintln!("battery init: {e}");
        }

        // 6) 定时器：1s 变化检测重绘 + 30s 兜底轮询
        let _ = SetTimer(Some(bar_hwnd), TIMER_REFRESH, REFRESH_MS, None);
        let _ = SetTimer(Some(msg_hwnd), TRAY_TIMER, TRAY_POLL_MS, None);

        // 7) 支持命令行直接打开设置面板
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--prefs" || a == "-p") {
            prefs::toggle_prefs();
        }

        // 8) 消息循环
        let mut msg = WAM::MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// 重新计算条的位置尺寸：
/// 横排任务栏 → 托盘左侧横条；竖排任务栏 → 托盘上方饱满横向卡片（原版风格）。
fn relayout(bar: HWND) {
    unsafe {
        let parent = match GetParent(bar) {
            Ok(p) => p,
            Err(_) => return,
        };
        let mut trc = RECT::default();
        if WAM::GetWindowRect(parent, &mut trc).is_err() {
            return;
        }
        let tb_w = trc.right - trc.left;
        let tb_h = trc.bottom - trc.top;
        if tb_h > tb_w {
            // 竖排任务栏：宽度填满任务栏宽度，高度为 30px 紧凑精致卡片，位于托盘上方
            let bar_h = 30i32;
            let (y, _unused) = taskbar::tray_left_estimate(parent);
            let _ = taskbar::place(bar, 0, y - bar_h - 4, tb_w, bar_h);
        } else {
            let (x, cy) = taskbar::tray_left_estimate(parent);
            let h = (cy - BAR_HEIGHT_PAD).max(12);
            let _ = taskbar::place(bar, x - BAR_WIDTH - 6, (cy - h) / 2, BAR_WIDTH, h);
        }
    }
}

unsafe extern "system" fn bar_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WAM::WM_SIZE => {
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WAM::WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client_rc = RECT::default();
            let _ = WAM::GetClientRect(hwnd, &mut client_rc);
            paint_bar(hdc, &client_rc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WAM::WM_LBUTTONUP => {
            // 原版行为：点击在 时间剩余 / 百分比 / 功率速率 / 剩余容量 之间顺畅循环切换
            let cfg = config_lock();
            let alt = {
                let c = cfg.lock().unwrap();
                c.alternate_on_click
            };
            if alt {
                let next = match mode_now() {
                    DisplayMode::TimeRemaining => DisplayMode::Percent,
                    DisplayMode::Percent => DisplayMode::DischargeRate,
                    DisplayMode::DischargeRate => DisplayMode::Capacity,
                    _ => DisplayMode::TimeRemaining,
                };
                set_mode(next);
                save_mode();
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WAM::WM_RBUTTONUP => {
            show_bar_menu(hwnd);
            LRESULT(0)
        }
        0x0200 /* WM_MOUSEMOVE */ => {
            // hover 唤起 Status Popup（原版 tooltip 行为）
            if !HOVER_TRACKED.swap(true, Ordering::Relaxed) {
                unsafe {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                }
                let _ = SetTimer(Some(hwnd), TIMER_HOVER, HOVER_MS, None);
            }
            LRESULT(0)
        }
        0x02A2 /* WM_MOUSELEAVE */ => {
            HOVER_TRACKED.store(false, Ordering::Relaxed);
            let _ = KillTimer(Some(hwnd), TIMER_HOVER);
            popup::close_popup();
            LRESULT(0)
        }
        WAM::WM_TIMER => {
            if wparam.0 == TIMER_REFRESH {
                let pct = power::PERCENT.load(Ordering::Relaxed) as i64;
                let eta_min = (power::ETA_SECONDS.load(Ordering::Relaxed) / 60) as i64;
                let chg = power::CHARGING.load(Ordering::Relaxed) as i64;
                let sig = pct * 10_000_000 + eta_min * 10 + chg;
                if LAST_SIG.swap(sig, Ordering::Relaxed) != sig {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    let tip = if pct < 0 {
                        "BatteryBar Modern".to_string()
                    } else {
                        format!("电池 {}% — BatteryBar Modern", pct)
                    };
                    let th = TRAY_HWND.load(Ordering::Relaxed);
                    if th != 0 {
                        let t = tray::TrayIcon { hwnd: HWND(th as *mut _) };
                        t.update_tooltip(&tip);
                    }
                }
                check_notifications(pct as i32, chg != 0);
            } else if wparam.0 == TIMER_HOVER {
                // hover 计时到：光标仍在条上则弹详情
                let _ = KillTimer(Some(hwnd), TIMER_HOVER);
                let mut pt = POINT::default();
                let _ = WAM::GetCursorPos(&mut pt);
                let mut rc = RECT::default();
                let _ = WAM::GetWindowRect(hwnd, &mut rc);
                if pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom {
                    popup::toggle_popup();
                }
            }
            LRESULT(0)
        }
        WAM::WM_LBUTTONDBLCLK => {
            // 双击电池条直接打开 Preferences 设置面板
            prefs::toggle_prefs();
            LRESULT(0)
        }
        WAM::WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_REFRESH);
            let _ = PostQuitMessage(0);
            LRESULT(0)
        }
        WAM::WM_COMMAND => {
            handle_menu_cmd(hwnd, (wparam.0 & 0xFFFF) as usize);
            LRESULT(0)
        }
        _ => {
            let reg = REFRESH_MSG.load(Ordering::Relaxed);
            if reg != 0 && msg == reg {
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

fn get_active_power_scheme() -> String {
    let out = std::process::Command::new("powercfg")
        .arg("/getactivescheme")
        .output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        if s.contains("381b4222") { return "平衡 (Balanced)".to_string(); }
        if s.contains("a1841308") { return "节能 (Power Saver)".to_string(); }
        if s.contains("8c5e7fda") { return "高性能 (High Performance)".to_string(); }
        if let Some(pos) = s.find('(') {
            if let Some(end) = s.find(')') {
                return s[pos+1..end].trim().to_string();
            }
        }
    }
    "平衡 (Balanced)".to_string()
}

fn set_active_power_scheme(guid: &str) {
    let _ = std::process::Command::new("powercfg")
        .args(&["/setactive", guid])
        .output();
}

fn show_bar_menu(hwnd: HWND) {
    unsafe {
        if let Ok(menu) = CreatePopupMenu() {
            let cur = mode_now();
            let mark = |on: bool| -> MENU_ITEM_FLAGS {
                if on { MENU_ITEM_FLAGS(0x00000008) /* MF_CHECKED */ } else { MENU_ITEM_FLAGS(0) /* MF_STRING | MF_ENABLED */ }
            };

            // 1. 原版 Pro 核心：Windows 电源方案一键切换 (Power Scheme Switcher)
            let scheme = get_active_power_scheme();
            let _ = AppendMenuW(menu, mark(scheme.contains("平衡") || scheme.contains("Balanced")), ID_MENU_SCHEME_BALANCED, w!("电源计划: 平衡 (Balanced)"));
            let _ = AppendMenuW(menu, mark(scheme.contains("节能") || scheme.contains("Saver")), ID_MENU_SCHEME_SAVER, w!("电源计划: 节能 (Power Saver)"));
            let _ = AppendMenuW(menu, mark(scheme.contains("高性能") || scheme.contains("Performance")), ID_MENU_SCHEME_PERF, w!("电源计划: 高性能 (High Performance)"));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0x800) /* MF_SEPARATOR */, 0, PCWSTR::null());

            // 2. 原版多模式切换单选组 (Toolbar Display Items)
            let _ = AppendMenuW(menu, mark(cur == DisplayMode::TimeRemaining), ID_MENU_MODE_TIME, w!("显示时间剩余 (Time Remaining)"));
            let _ = AppendMenuW(menu, mark(cur == DisplayMode::Percent), ID_MENU_MODE_PCT, w!("显示电量百分比 (Percent Remaining)"));
            let _ = AppendMenuW(menu, mark(cur == DisplayMode::DischargeRate), ID_MENU_MODE_RATE, w!("显示实时功率 (Discharge/Charge Rate)"));
            let _ = AppendMenuW(menu, mark(cur == DisplayMode::Capacity), ID_MENU_MODE_CAP, w!("显示剩余容量 (Capacity Remaining)"));
            let _ = AppendMenuW(menu, mark(cur == DisplayMode::Icon), ID_MENU_MODE_ICON, w!("只显示小图标 (Icon Only)"));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0x800) /* MF_SEPARATOR */, 0, PCWSTR::null());

            // 3. 原版常规选项
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_PREFS, w!("Preferences..."));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_TOGGLE, w!("显示/隐藏条"));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_EXIT, w!("退出"));

            let mut pt = POINT::default();
            let _ = WAM::GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | WAM::TPM_RETURNCMD,
                pt.x,
                pt.y,
                None,
                hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
            if cmd.0 != 0 {
                handle_menu_cmd(hwnd, cmd.0 as usize);
            }
        }
    }
}

fn handle_menu_cmd(hwnd: HWND, cmd: usize) {
    match cmd {
        ID_MENU_EXIT => {
            unsafe { let _ = PostQuitMessage(0); }
        }
        ID_MENU_PREFS => {
            prefs::toggle_prefs();
        }
        ID_MENU_SCHEME_BALANCED => {
            set_active_power_scheme("381b4222-f694-41f0-9685-ff5bb260df2e");
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_SCHEME_SAVER => {
            set_active_power_scheme("a1841308-3541-4fab-bc81-f71556f20b4a");
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_SCHEME_PERF => {
            set_active_power_scheme("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c");
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_MODE_TIME => {
            set_mode(DisplayMode::TimeRemaining);
            save_mode();
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_MODE_PCT => {
            set_mode(DisplayMode::Percent);
            save_mode();
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_MODE_RATE => {
            set_mode(DisplayMode::DischargeRate);
            save_mode();
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_MODE_CAP => {
            set_mode(DisplayMode::Capacity);
            save_mode();
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_MODE_ICON => {
            set_mode(DisplayMode::Icon);
            save_mode();
            unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
        }
        ID_MENU_TOGGLE => {
            unsafe {
                let vis = IsWindowVisible(hwnd).as_bool();
                let show_cmd = if vis { SW_HIDE } else { SW_SHOW };
                let _ = ShowWindow(hwnd, show_cmd);
            }
        }
        _ => {}
    }
}

/// 低电/临界/满电 通知（原版 Notifications + Warning Levels）
fn check_notifications(pct: i32, charging: bool) {
    use std::sync::atomic::{AtomicBool, Ordering as O};
    let cfg = config_lock();
    let (low, crit, n_low, n_crit, n_full, hide_full) = {
        let c = cfg.lock().unwrap();
        (c.low_percent, c.critical_percent, c.notify_low, c.notify_critical, c.notify_full, c.hide_when_full)
    };

    // 满电隐藏
    if hide_full {
        let bh = BAR_HWND.load(Ordering::Relaxed);
        if bh != 0 {
            let bar = HWND(bh as *mut _);
            let should_hide = pct >= 100 && charging;
            let vis = unsafe { IsWindowVisible(bar).as_bool() };
            if should_hide && vis {
                unsafe { let _ = ShowWindow(bar, SW_HIDE); }
            } else if !should_hide && !vis {
                unsafe { let _ = ShowWindow(bar, SW_SHOW); }
            }
        }
    }

    static FULL_LATCH: AtomicBool = AtomicBool::new(false);
    static LOW_LATCH: AtomicBool = AtomicBool::new(false);
    static CRIT_LATCH: AtomicBool = AtomicBool::new(false);

    let th = TRAY_HWND.load(Ordering::Relaxed);
    if th == 0 {
        return;
    }
    let t = tray::TrayIcon { hwnd: HWND(th as *mut _) };

    // 满电：充+满 边沿触发
    if n_full && pct >= 100 && charging {
        if !FULL_LATCH.swap(true, O::Relaxed) {
            t.balloon("BatteryBar Modern", "电池已充满，可以拔掉电源了。",
                windows::Win32::UI::Shell::NIIF_INFO);
        }
    } else if pct < 100 {
        FULL_LATCH.store(false, O::Relaxed);
    }
    // 临界：放+≤crit 边沿
    if n_crit && !charging && pct <= crit {
        if !CRIT_LATCH.swap(true, O::Relaxed) {
            t.balloon("电量临界", &format!("电池仅剩 {}%，请立即接通电源！", pct),
                windows::Win32::UI::Shell::NIIF_WARNING);
        }
    } else if charging || pct > crit {
        CRIT_LATCH.store(false, O::Relaxed);
    }
    // 低电：放+≤low 边沿
    if n_low && !charging && pct <= low {
        if !LOW_LATCH.swap(true, O::Relaxed) {
            t.balloon("电量不足", &format!("电池仅剩 {}%，建议接通电源。", pct),
                windows::Win32::UI::Shell::NIIF_WARNING);
        }
    } else if charging || pct > low {
        LOW_LATCH.store(false, O::Relaxed);
    }
}

unsafe extern "system" fn msg_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // 首次进入时注册广播消息（bar 窗口的刷新通道）
    if REFRESH_MSG.load(Ordering::Relaxed) == 0 {
        let wide: Vec<u16> = power::POWERDATA_CHANGED_MSG
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        REFRESH_MSG.store(
            RegisterWindowMessageW(PCWSTR::from_raw(wide.as_ptr())),
            Ordering::Relaxed,
        );
    }

    match msg {
        tray::TRAY_CALLBACK => {
            let ev = (lparam.0 & 0xFFFF) as u32;
            if ev == WAM::WM_LBUTTONUP {
                popup::toggle_popup();
            } else if ev == WAM::WM_RBUTTONUP {
                show_bar_menu(hwnd);
            }
            LRESULT(0)
        }
        WAM::WM_COMMAND => match (wparam.0 & 0xFFFF) as usize {
            ID_MENU_EXIT => {
                let _ = PostQuitMessage(0);
                LRESULT(0)
            }
            ID_MENU_PREFS => {
                prefs::toggle_prefs();
                LRESULT(0)
            }
            ID_MENU_MODE_TIME => {
                set_mode(DisplayMode::TimeRemaining);
                save_mode();
                let bh = BAR_HWND.load(Ordering::Relaxed);
                if bh != 0 {
                    let _ = InvalidateRect(Some(HWND(bh as *mut _)), None, false);
                }
                LRESULT(0)
            }
            ID_MENU_MODE_PCT => {
                set_mode(DisplayMode::Percent);
                save_mode();
                let bh = BAR_HWND.load(Ordering::Relaxed);
                if bh != 0 {
                    let _ = InvalidateRect(Some(HWND(bh as *mut _)), None, false);
                }
                LRESULT(0)
            }
            ID_MENU_MODE_ICON => {
                set_mode(DisplayMode::Icon);
                save_mode();
                let bh = BAR_HWND.load(Ordering::Relaxed);
                if bh != 0 {
                    let _ = InvalidateRect(Some(HWND(bh as *mut _)), None, false);
                }
                LRESULT(0)
            }
            ID_MENU_TOGGLE => {
                let bh = BAR_HWND.load(Ordering::Relaxed);
                if bh != 0 {
                    let bar = HWND(bh as *mut _);
                    let vis = IsWindowVisible(bar).as_bool();
                    let cmd = if vis { SW_HIDE } else { SW_SHOW };
                    let _ = ShowWindow(bar, cmd);
                }
                LRESULT(0)
            }
            _ => LRESULT(0),
        },
        WAM::WM_TIMER if wparam.0 == TRAY_TIMER => {
            // 兜底轮询电池状态（WinRT 事件可能丢）
            power::poll_once();
            LRESULT(0)
        }
        WAM::WM_DESTROY => {
            let t = tray::TrayIcon { hwnd };
            t.remove();
            let _ = PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint_bar(hdc: windows::Win32::Graphics::Gdi::HDC, rc: &RECT) {
    use windows::Win32::Graphics::Gdi::{GetTextExtentPoint32W, SetTextColor, SelectObject, DeleteObject};

    let dark = render::system_prefers_dark();
    let pal: Palette = if dark { palette_dark() } else { palette_light() };
    let cfg = config_lock();
    let c = cfg.lock().unwrap();

    // 任务栏底色铺底
    let (tr, tg, tb) = if dark { c.dark_taskbar_tint } else { c.light_taskbar_tint };
    let bg = render::rgb(tr, tg, tb);
    render::fill_solid(hdc, rc, bg);
    render::set_bk_transparent(hdc);

    let pct = power::PERCENT.load(Ordering::Relaxed);
    let charging = power::CHARGING.load(Ordering::Relaxed);

    // 电池卡片矩形（无论横竖，卡片四周均留精致微内边距）
    let pad_x = 5i32;
    let pad_y = 3i32;
    let card_l = rc.left + pad_x;
    let card_t = rc.top + pad_y;
    let card_r = rc.right - pad_x;
    let card_b = rc.bottom - pad_y;

    if card_r <= card_l + 10 || card_b <= card_t + 10 {
        return;
    }

    // 1. 拟物化实体电池（Skeuomorphic Battery：金属外壳 + 正极端子 + 内凹观察窗 + 液面）
    let low = pct <= c.low_percent;
    let critical = pct <= c.critical_percent;
    render::draw_skeuo_battery(
        hdc,
        card_l,
        card_t,
        card_r,
        card_b,
        pct,
        charging,
        low,
        critical,
        &pal,
    );

    // 2. 文字内容构建（大号加粗立体浮雕字，水平绝对居中）
    let eta = power::ETA_SECONDS.load(Ordering::Relaxed);
    let cur_mode = mode_now();

    let text = if cur_mode == DisplayMode::Icon {
        String::new()
    } else if pct < 0 {
        "—".to_string()
    } else {
        match cur_mode {
            DisplayMode::TimeRemaining => {
                if charging {
                    if pct >= 100 {
                        "On A/C".to_string()
                    } else if eta > 0 {
                        format!("⚡ {}:{:02}", eta / 3600, (eta % 3600) / 60)
                    } else {
                        "⚡ 充电中".to_string()
                    }
                } else if eta > 0 {
                    format!("{}:{:02}", eta / 3600, (eta % 3600) / 60)
                } else {
                    "放电中".to_string()
                }
            }
            DisplayMode::Percent => {
                if charging {
                    format!("⚡ {}%", pct)
                } else {
                    format!("{}%", pct)
                }
            }
            DisplayMode::DischargeRate => {
                let mw = power::CHARGE_RATE_MW.load(Ordering::Relaxed);
                if mw == 0 {
                    "0.0 W".to_string()
                } else if mw < 0 {
                    format!("+{:.1}W", (-mw as f64) / 1000.0)
                } else {
                    format!("-{:.1}W", (mw as f64) / 1000.0)
                }
            }
            DisplayMode::Capacity => {
                let rem = power::REMAIN_MWH.load(Ordering::Relaxed);
                if rem > 0 {
                    format!("{:.1}Wh", (rem as f64) / 1000.0)
                } else {
                    format!("{}%", pct)
                }
            }
            DisplayMode::Icon => String::new(),
        }
    };

    if !text.is_empty() {
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        let font = render::create_ui_font(13, 600);
        let old_font = SelectObject(hdc, font.into());

        let mut size = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &buf, &mut size);

        let cx = (card_l + card_r) / 2;
        let cy = (card_t + card_b) / 2;
        let tx = cx - size.cx / 2;
        let ty = cy - size.cy / 2;

        // Telegram 单色高清晰字体排版（无杂色毛边）
        SetTextColor(hdc, pal.text);
        let mut drc_text = RECT {
            left: tx,
            top: ty,
            right: tx + size.cx + 2,
            bottom: ty + size.cy + 2,
        };
        DrawTextW(hdc, &mut buf, &mut drc_text, DT_NOPREFIX | DT_SINGLELINE);

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font.into());
    } else if c.default_mode == DisplayMode::Icon {
        // 图标居中模式
        let icon_w = 26i32;
        let icon_h = 13i32;
        let ix = (card_l + card_r) / 2 - icon_w / 2;
        let iy = (card_t + card_b) / 2 - icon_h / 2;
        draw_battery_icon(hdc, ix, iy, icon_w, icon_h, pct, charging, render::rgb(76, 175, 80), &pal);
    }
}

/// 画小电池图标：外壳 + 电量填充 + 充电闪电
unsafe fn draw_battery_icon(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pct: i32,
    charging: bool,
    fill: COLORREF,
    pal: &Palette,
) {
    let body_w = w - 4; // 留 4px 给正极头
    // 外壳（描边）
    let pen = CreatePen(PS_SOLID, 1, pal.text_dim);
    let old_pen = SelectObject(hdc, pen.into());
    let old_brush_hollow = {
        use windows::Win32::Graphics::Gdi::{GetStockObject, HOLLOW_BRUSH};
        SelectObject(hdc, GetStockObject(HOLLOW_BRUSH))
    };
    let _ = WAM_StrokeRect(hdc, x, y, x + body_w, y + h);
    SelectObject(hdc, old_brush_hollow);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen.into());
    // 正极头
    render::fill_round(hdc, x + body_w, y + h / 2 - 2, x + w, y + h / 2 + 2, 1, pal.text_dim);
    // 内部填充（留 2px 内缩）
    let frac = pct.clamp(0, 100) as i64;
    let inner_w = ((body_w - 4) as i64 * frac / 100) as i32;
    if inner_w > 2 {
        render::fill_round(hdc, x + 2, y + 2, x + 2 + inner_w, y + h - 2, 1, fill);
    }
    let _ = charging;
}

unsafe fn WAM_StrokeRect(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    l: i32,
    t: i32,
    r: i32,
    b: i32,
) -> windows_core::Result<()> {
    use windows::Win32::Graphics::Gdi::{LineTo, MoveToEx};
    let _ = MoveToEx(hdc, l, t, None);
    let _ = LineTo(hdc, r, t);
    let _ = LineTo(hdc, r, b);
    let _ = LineTo(hdc, l, b);
    let _ = LineTo(hdc, l, t);
    Ok(())
}

fn lerp_color(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let ar = (a.0 & 0xFF) as f32;
    let ag = ((a.0 >> 8) & 0xFF) as f32;
    let ab = ((a.0 >> 16) & 0xFF) as f32;
    let br = (b.0 & 0xFF) as f32;
    let bg = ((b.0 >> 8) & 0xFF) as f32;
    let bb = ((b.0 >> 16) & 0xFF) as f32;
    let t = t.clamp(0.0, 1.0);
    let r = (ar + (br - ar) * t) as u32;
    let g = (ag + (bg - ag) * t) as u32;
    let bl = (ab + (bb - ab) * t) as u32;
    COLORREF(r | (g << 8) | (bl << 16))
}

/// 菜单切模式后保存到 options.json
fn save_mode() {
    let cfg = config_lock();
    let c = cfg.lock().unwrap().clone();
    c.save();
}
