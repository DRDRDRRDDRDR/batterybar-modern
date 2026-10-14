// batterybar-modern — 入口与主窗口（任务栏嵌入条）
//
// 窗口（对应原版 BatteryBar Pro 的各部件）：
//   1. bar window      —— 任务栏内嵌条（SetParent 进 Shell_TrayWnd）
//   2. message window  —— 托盘图标宿主（不可见）
//   3. popup window    —— Status Popup（原版 7 字段复刻）
//   4. prefs window    —— Preferences（原版 6 分类页复刻）
//   5. graph window    —— Battery Profile Graph（原版 Pro 的独立曲线窗口）
//
// 子系统：Release 必须显式声明 windows 子系统。
// windows-gnu 目标默认按 console 子系统链接，不声明的话启动 exe 会附带一个
// 控制台黑框窗口（PE Subsystem=3），任务栏里会多出一个 cmd 样的条目。
// debug 构建保留 console，方便 eprintln! 调试输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod clock;
mod config;
mod graph;
mod popup;
mod power;
mod prefs;
mod profile;
mod render;
mod taskbar;
mod tray;

use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DrawTextW, EndPaint, InvalidateRect, DT_NOPREFIX, DT_SINGLELINE, PAINTSTRUCT,
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
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, MENU_ITEM_FLAGS, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
    WS_CHILD, WS_VISIBLE,
};
use windows_core::{w, PCWSTR};

use crate::config::{Config, DisplayMode};
use crate::render::{palette_dark, palette_light, Palette};

// ---- 布局常量 ----
/// 横排任务栏下条宽度下限：再窄就放不下「图标 + 文字」的最小排布。
const BAR_WIDTH_MIN: i32 = 60;
/// 横排任务栏下条宽度上限：超过 400 会挤占任务栏按钮区，没有实用价值。
const BAR_WIDTH_MAX: i32 = 400;
const BAR_HEIGHT_PAD: i32 = 10;
/// 竖排任务栏下条的高度（随字号变化，见 `bar_height_vertical()`）。
///
/// 46px 是字号还是 11px 时的实测下限；改用真实 Segoe UI 并正名负高度字号后，
/// 文字行变高，再按 46px 排就会把电池图标压成 28×16 的扁块（原本约 1.8:1）。
/// 所以高度必须跟着字号走，保证「图标上 + 文字下」两行都拿到充足行高。
const BAR_ICON_BOX_V: i32 = 26;
const TIMER_REFRESH: usize = 1;
const TIMER_HOVER: usize = 3;
/// Explorer 重启自愈轮询间隔（ms）。Explorer 崩溃重启后 Shell_TrayWnd 会换新句柄，
/// 我们的条窗口会被连带销毁或悬空，需要周期性检查并重新嵌入。
const TIMER_REEMBED: usize = 4;
const REEMBED_MS: u32 = 5_000;
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
/// Battery Profile Graph（原版 Pro 的独立曲线窗口）
const ID_MENU_GRAPH: usize = 0x1030;
/// Tools 页：清空曲线采样数据
const ID_MENU_RESET_PROFILE: usize = 0x1031;

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

/// 当前生效的横排条宽度（读配置 + 夹到合法区间）。
///
/// 竖排任务栏不用这个值 —— 那里条宽必须等于任务栏宽度，否则会看到缝。
pub fn bar_width() -> i32 {
    let w = config_lock().lock().unwrap().bar_width;
    w.clamp(BAR_WIDTH_MIN, BAR_WIDTH_MAX)
}

/// 字号档位（0=小 1=中 2=大）→ 字符高度像素。三档为 13 / 15 / 17。
///
/// 纯函数，不读配置 —— `paint_bar` 已持有配置锁，再调读锁函数会自锁死。
pub fn font_px_of(font_size: u32) -> i32 {
    13 + (font_size as i32) * 2
}

/// Warning Levels 页的两个阈值（低电量 %、临界电量 %）。
///
/// 曲线窗口画参考线、通知判定都从这里取，保证 UI 与行为同源。
pub fn thresholds() -> (i32, i32) {
    match config_lock().lock() {
        Ok(c) => (c.low_percent, c.critical_percent),
        Err(_) => (20, 10),
    }
}

/// 竖排任务栏下条高度：图标盒 + 间隙 + 文字行，随字号伸缩。
///
/// 高度必须跟着字号走。写死 46px 时，中号字（15px）的文字行要吃 19px，
/// 留给图标的只剩 16px —— 电池会被压成 28×16 的扁块，完全看不出是电池。
pub fn bar_height_vertical() -> i32 {
    let fs = config_lock().lock().unwrap().font_size;
    BAR_ICON_BOX_V + 9 + (font_px_of(fs) + 4)
}

pub fn set_mode(m: DisplayMode) {
    DISPLAY_MODE.store(match m {
        DisplayMode::TimeRemaining => 0,
        DisplayMode::Percent => 1,
        DisplayMode::DischargeRate => 2,
        DisplayMode::Capacity => 3,
        DisplayMode::Icon => 4,
        DisplayMode::TimeFinished => 5,
    }, Ordering::Relaxed);
}

fn mode_now() -> DisplayMode {
    match DISPLAY_MODE.load(Ordering::Relaxed) {
        1 => DisplayMode::Percent,
        2 => DisplayMode::DischargeRate,
        3 => DisplayMode::Capacity,
        4 => DisplayMode::Icon,
        5 => DisplayMode::TimeFinished,
        _ => DisplayMode::TimeRemaining,
    }
}

/// `mode_now` 的公开版本（prefs.rs 勾选 display_items 时需要判断当前模式是否还在勾选列表里）。
pub fn mode_now_pub() -> DisplayMode {
    mode_now()
}

/// 拿当前托盘图标句柄发一条气泡提示。
///
/// 供 `prefs.rs` 这类没有托盘上下文的模块在出错时通知用户（例如写注册表
/// 自启项被拒绝）。托盘未就绪时静默返回 —— 提示失败不该再引发一次失败。
pub fn tray_balloon(title: &str, text: &str) {
    let th = TRAY_HWND.load(Ordering::Relaxed);
    if th == 0 {
        return;
    }
    let t = tray::TrayIcon { hwnd: HWND(th as *mut _) };
    t.balloon(title, text, windows::Win32::UI::Shell::NIIF_WARNING);
}

/// 重绘任务栏条（Preferences 改完设置后调用）
pub fn refresh_bar() {
    let h = BAR_HWND.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = InvalidateRect(Some(HWND(h as *mut std::ffi::c_void)), None, false);
        }
    }
}

/// 按当前配置重新计算条的位置与尺寸。
///
/// 改「电池条宽度」「接通电源时隐藏条」这类会影响几何的设置后必须调用 ——
/// 只 InvalidateRect 重绘是不够的，窗口尺寸没变，新宽度不会生效。
pub fn relayout_bar() {
    let h = BAR_HWND.load(Ordering::Relaxed);
    if h != 0 {
        relayout(HWND(h as *mut std::ffi::c_void));
        refresh_bar();
    }
}

/// 是否接通外部电源（充电中，或 Win32 报告 ACLineStatus=1）
fn on_ac_power() -> bool {
    power::CHARGING.load(Ordering::Relaxed) || crate::power::AC_ON.load(Ordering::Relaxed)
}

/// 滚轮兜底转发：鼠标悬停在 Preferences 上时，把宿主收到的 WM_MOUSEWHEEL 转发过去。
/// 起因：任务栏条是 Shell_TrayWnd 的子窗口，拿不到键盘焦点；而 WM_MOUSEWHEEL
/// 只投递给**焦点窗口**而非鼠标所在窗口，导致设置面板滚轮完全无效。
fn maybe_forward_wheel(wparam: WPARAM) {
    let delta = ((wparam.0 >> 16) as i16) as i32;
    if prefs::cursor_over_prefs() {
        prefs::forward_wheel(delta);
    } else if graph::cursor_over_graph() {
        graph::forward_wheel(delta);
    }
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

    // 注册 Battery Profile Graph 曲线窗口类（BBM_Graph）
    let graph_class = w!("BBM_Graph");
    let wc4 = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW | WAM::CS_DBLCLKS,
        lpfnWndProc: Some(graph::graph_wndproc),
        hInstance: hinstance.into(),
        lpszClassName: PCWSTR::from_raw(graph_class.as_ptr()),
        ..Default::default()
    };
    unsafe {
        if RegisterClassW(&wc4) == 0 {
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
            bar_width(),
            bar_height_vertical(),
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
            let bw = bar_width();
            let _ = WAM::SetWindowPos(
                bar_hwnd,
                None,
                sw - bw - 8,
                sh - 60,
                bw,
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

        // 5.5) 应用启动配置：显示模式 / 学习算法开关 / 开机自启注册项
        {
            let cfg = config_lock();
            let (mode, learn, startup) = {
                let c = cfg.lock().unwrap();
                (c.default_mode, c.statistical_estimation, c.run_at_startup)
            };
            set_mode(mode);
            power::LEARN_ENABLED.store(learn, Ordering::Relaxed);
            // 每次启动把注册表状态对齐到配置（用户可能在外部删过启动项）。
            // 写失败只能记为「未生效」，不改 options.json —— 下次启动会再试。
            let c = cfg.lock().unwrap().clone();
            if !c.set_startup(startup) {
                eprintln!("set_startup: 写入注册表自启项失败（进程完整性级别不足？）");
            }
        }

        // 6) 定时器：1s 变化检测重绘 + 30s 兜底轮询 + 5s Explorer 自愈检查
        let _ = SetTimer(Some(bar_hwnd), TIMER_REFRESH, REFRESH_MS, None);
        let _ = SetTimer(Some(msg_hwnd), TRAY_TIMER, TRAY_POLL_MS, None);
        let _ = SetTimer(Some(msg_hwnd), TIMER_REEMBED, REEMBED_MS, None);

        // 7) 支持命令行直接打开设置面板 / 曲线窗口
        //    batterybar-modern.exe --prefs            打开 Preferencges
        //    batterybar-modern.exe --prefs=tools      打开并直接跳到 Tools 分类页
        //    batterybar-modern.exe --graph            打开电池曲线窗口
        let args: Vec<String> = std::env::args().collect();
        if let Some(page) = args
            .iter()
            .find_map(|a| a.strip_prefix("--prefs=").or_else(|| a.strip_prefix("-p=")))
        {
            prefs::show_page(page);
            prefs::toggle_prefs();
        } else if args.iter().any(|a| a == "--prefs" || a == "-p") {
            prefs::toggle_prefs();
        }
        if args.iter().any(|a| a == "--graph" || a == "-g") {
            graph::toggle_graph();
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
            // 竖排任务栏：宽度填满任务栏宽度。
            // 高度随字号伸缩（见 bar_height_vertical）—— 固定值会把电池图标压扁。
            let bar_h = bar_height_vertical();
            let (y, _unused) = taskbar::tray_left_estimate(parent);
            let _ = taskbar::place(bar, 0, y - bar_h - 4, tb_w, bar_h);
        } else {
            let bw = bar_width();
            let (x, cy) = taskbar::tray_left_estimate(parent);
            let h = (cy - BAR_HEIGHT_PAD).max(12);
            let _ = taskbar::place(bar, x - bw - 6, (cy - h) / 2, bw, h);
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
            // 原版行为：点击在 时间剩余 / 百分比 / 功率速率 / 剩余容量 / 仅图标 之间循环切换。
            // Preferences → BEHAVIOR →「单击行为」可改为直接弹详情卡。
            let cfg = config_lock();
            let (alt, click_mode) = {
                let c = cfg.lock().unwrap();
                (c.alternate_on_click, c.on_click_mode)
            };
            if click_mode == 1 {
                popup::toggle_popup();
            } else if alt {
                let next = match mode_now() {
                    DisplayMode::TimeRemaining => DisplayMode::Percent,
                    DisplayMode::Percent => DisplayMode::DischargeRate,
                    DisplayMode::DischargeRate => DisplayMode::Capacity,
                    DisplayMode::Capacity => DisplayMode::Icon,
                    _ => DisplayMode::TimeRemaining,
                };
                set_mode(next);
                save_mode();
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        0x020A /* WM_MOUSEWHEEL */ => {
            maybe_forward_wheel(wparam);
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
                // hover 计时到：光标仍在条上则弹详情（Preferences → GENERAL 可关掉悬停弹卡）
                let _ = KillTimer(Some(hwnd), TIMER_HOVER);
                let want_tip = { config_lock().lock().unwrap().show_tooltip };
                if !want_tip {
                    return LRESULT(0);
                }
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

/// 静默启动子进程（CREATE_NO_WINDOW = 0x08000000）。
///
/// 本程序是 GUI 子系统，但 powercfg / shutdown / rundll32 都是控制台程序。
/// 直接 spawn 的话，每次调用都会闪一个黑色控制台窗口 —— 对每秒/每分钟轮询
/// 电源方案的场景尤其明显。加了这个 flag 后子进程不分配控制台。
pub fn hidden_command(exe: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = std::process::Command::new(exe);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

fn get_active_power_scheme() -> String {
    let out = hidden_command("powercfg")
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
    let _ = hidden_command("powercfg")
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

            // 3. 原版 Pro：独立的电池曲线窗口 + 工具
            let graph_label = if graph::is_open() {
                w!("关闭电池曲线窗口 (Battery Profile Graph)")
            } else {
                w!("打开电池曲线窗口… (Battery Profile Graph)")
            };
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_GRAPH, graph_label);
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_RESET_PROFILE, w!("清空曲线数据 (Reset Profile Data)"));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0x800) /* MF_SEPARATOR */, 0, PCWSTR::null());

            // 4. 原版常规选项
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_PREFS, w!("Preferences..."));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_TOGGLE, w!("显示/隐藏条"));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_EXIT, w!("退出"));

            let mut pt = POINT::default();
            let _ = WAM::GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
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
        ID_MENU_GRAPH => {
            graph::toggle_graph();
        }
        ID_MENU_RESET_PROFILE => {
            profile::clear();
            tray_balloon("BatteryBar Modern", "已清空电池曲线采样数据。");
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

/// 低电/临界/满电 通知（原版 Notifications 页 + Warning Levels 页）
///
/// 原版 Notifications 页对三组事件（Critical / Low / Finished charging）各给
/// 一组完全相同的控件：Audio alert、Show popup、Use custom sound、Beep the
/// system speaker、Run a program。这里按同构处理，见 config::Notif。
/// 另支持 Warning Levels 的自定义警报阈值（取 low 与 alarm 的更小者）与
/// 临界电量自动动作（睡眠/关机/休眠/重启）。
fn check_notifications(pct: i32, charging: bool) {
    use crate::config::Notif;
    use std::sync::atomic::{AtomicBool, Ordering as O};
    let cfg = config_lock();
    let (low, crit, n_low, n_crit, n_full, hide_full, alarm, crit_action, hide_on_ac) = {
        let c = cfg.lock().unwrap();
        (
            c.low_percent,
            c.critical_percent,
            c.notif_low.clone(),
            c.notif_critical.clone(),
            c.notif_full.clone(),
            c.hide_when_full,
            c.alarm_percent,
            c.critical_action,
            c.hide_on_ac,
        )
    };

    // 阈值：alarm_percent > 0 时取更严格的那个
    let low = if alarm > 0 { low.min(alarm) } else { low };
    let ac = on_ac_power();

    // 自动隐藏：满电 或 接通电源
    if hide_full || hide_on_ac {
        let bh = BAR_HWND.load(Ordering::Relaxed);
        if bh != 0 {
            let bar = HWND(bh as *mut _);
            let should_hide = (hide_full && pct >= 100 && charging) || (hide_on_ac && ac);
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
    static ALARM_LATCH: AtomicBool = AtomicBool::new(false);
    static ACT_LATCH: AtomicBool = AtomicBool::new(false);

    let th = TRAY_HWND.load(Ordering::Relaxed);
    if th == 0 {
        return;
    }
    let t = tray::TrayIcon { hwnd: HWND(th as *mut _) };

    // 临界动作文案：按用户选择的动作预告下一步
    let crit_hint = match crit_action {
        1 => "即将自动进入睡眠",
        2 => "即将自动关机",
        3 => "即将自动进入休眠",
        4 => "即将自动重启",
        _ => "请立即接通电源",
    };

    // 逐组投递：气泡 + 声音 + 外部程序，全部按 Notif 的勾选走。
    let fire = |n: &Notif, title: &str, body: &str, level: windows::Win32::UI::Shell::NOTIFY_ICON_INFOTIP_FLAGS| {
        if n.popup {
            t.balloon(title, body, level);
        }
        if n.audio {
            if n.custom_sound && !n.sound_path.is_empty() {
                tray::play_file(&n.sound_path);
            } else {
                t.sound();
            }
        }
        if n.beep {
            tray::beep();
        }
        if n.run_program && !n.program_path.is_empty() {
            let _ = hidden_command("cmd.exe")
                .args(["/C", n.program_path.as_str()])
                .spawn();
        }
    };

    // 满电：充+满 边沿触发
    if pct >= 100 && charging {
        if !FULL_LATCH.swap(true, O::Relaxed) {
            fire(&n_full, "BatteryBar Modern", "电池已充满，可以拔掉电源了。",
                windows::Win32::UI::Shell::NIIF_INFO);
        }
    } else if pct < 100 {
        FULL_LATCH.store(false, O::Relaxed);
    }

    // 临界：放+≤crit 边沿
    if !charging && pct <= crit {
        if !CRIT_LATCH.swap(true, O::Relaxed) {
            fire(&n_crit, "电量临界",
                &format!("电池仅剩 {}%，{}！", pct, crit_hint),
                windows::Win32::UI::Shell::NIIF_WARNING);
        }
    } else if charging || pct > crit {
        CRIT_LATCH.store(false, O::Relaxed);
    }

    // 低电：放+≤low 边沿
    if !charging && pct <= low {
        if !LOW_LATCH.swap(true, O::Relaxed) {
            fire(&n_low, "电量不足", &format!("电池仅剩 {}%，建议接通电源。", pct),
                windows::Win32::UI::Shell::NIIF_WARNING);
        }
    } else if charging || pct > low {
        LOW_LATCH.store(false, O::Relaxed);
    }

    // 自定义电量警报（alarm_percent，与低电阈值取更小者）
    if alarm > 0 && !charging && pct <= alarm {
        if !ALARM_LATCH.swap(true, O::Relaxed) {
            t.balloon("电量警报", &format!("电量已降至 {}%，达到自定义警报线。", pct),
                windows::Win32::UI::Shell::NIIF_WARNING);
        }
    } else if charging || pct > alarm {
        ALARM_LATCH.store(false, O::Relaxed);
    }

    // 临界电量自动动作（边沿触发，只执行一次）
    if crit_action > 0 && !charging && pct <= crit {
        if !ACT_LATCH.swap(true, O::Relaxed) {
            run_critical_action(crit_action);
        }
    } else {
        ACT_LATCH.store(false, O::Relaxed);
    }
}

/// 临界电量自动动作
/// 1=睡眠 2=关机 3=休眠 4=重启
pub fn run_critical_action(action: u32) {
    let run = |exe: &str, args: &[&str]| {
        let _ = hidden_command(exe).args(args).spawn();
    };
    match action {
        // SetSuspendState(Hibernate=0, ForceCritical=1, DisableWakeEvent=0) → 睡眠
        1 => run("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]),
        // SetSuspendState(Hibernate=1, ...) → 休眠
        3 => run("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,0,0"]),
        2 => run("shutdown.exe", &["/s", "/t", "0"]),
        4 => run("shutdown.exe", &["/r", "/t", "0"]),
        _ => {}
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
        WAM::WM_COMMAND => {
            // 统一走与右键菜单相同的命令分发，避免两处命令表各写一份而漂移
            // ——此前这里只列了 4 个命令，导致「电池曲线」「清空曲线数据」
            // 「电源计划」「实时功率/剩余容量」这些通过 WM_COMMAND 发进来的
            // 命令全部落进 _ => LRESULT(0)，点了没反应。
            //
            // 必须传条窗口而不是 MsgHost：handle_menu_cmd 的「显示/隐藏条」
            // 分支按传入窗口自身的可见性取反，传 MsgHost 会去隐藏一个本来就
            // 不可见的窗口。
            let cmd = (wparam.0 & 0xFFFF) as usize;
            let bh = BAR_HWND.load(Ordering::Relaxed);
            if bh != 0 {
                handle_menu_cmd(HWND(bh as *mut _), cmd);
            }
            LRESULT(0)
        }
        WAM::WM_TIMER if wparam.0 == TRAY_TIMER => {
            // 兜底轮询电池状态（WinRT 事件可能丢）
            power::poll_once();
            LRESULT(0)
        }
        WAM::WM_TIMER if wparam.0 == TIMER_REEMBED => {
            reembed_if_needed();
            LRESULT(0)
        }
        0x020A /* WM_MOUSEWHEEL */ => {
            // 光标在 Preferences 上时，把滚轮转发给设置面板
            maybe_forward_wheel(wparam);
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

/// Explorer 重启自愈。
///
/// Explorer 崩溃/重启后 Shell_TrayWnd 会换一个新句柄，旧任务栏连同我们 SetParent
/// 进去的条窗口一起被销毁，电池条就永久消失了。这里每 5 秒检查一次：
/// 条窗口还在不在？它的父窗口还是不是当前那个任务栏？不是就重建 + 重新嵌入。
fn reembed_if_needed() {
    unsafe {
        let bh = BAR_HWND.load(Ordering::Relaxed);
        let bar = HWND(bh as *mut _);
        let taskbars = taskbar::find_taskbars();
        let Some(tb) = taskbars.first() else { return };

        // 条窗口已销毁 → 重建
        if !taskbar::still_valid(bar) {
            let Ok(hinstance) = GetModuleHandleW(PCWSTR::null()) else { return };
            let Ok(new_bar) = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BBM_Bar"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                bar_width(),
                bar_height_vertical(),
                Some(tb.hwnd),
                None,
                Some(hinstance.into()),
                None,
            ) else {
                return;
            };
            BAR_HWND.store(new_bar.0 as isize, Ordering::Relaxed);
            let _ = SetTimer(Some(new_bar), TIMER_REFRESH, REFRESH_MS, None);
            let _ = ShowWindow(new_bar, SW_SHOW);
            relayout(new_bar);
            let _ = InvalidateRect(Some(new_bar), None, false);
            return;
        }

        // 条窗口还在，但父窗口已经不是当前任务栏 → 重新嵌入
        let parent = GetParent(bar).unwrap_or_default();
        if parent != tb.hwnd {
            if taskbar::reparent(bar, tb.hwnd).is_ok() {
                relayout(bar);
                let _ = InvalidateRect(Some(bar), None, false);
            }
        } else if !taskbar::visible(bar) {
            let _ = ShowWindow(bar, SW_SHOW);
        }
    }
}

unsafe fn paint_bar(hdc: windows::Win32::Graphics::Gdi::HDC, rc: &RECT) {
    use windows::Win32::Graphics::Gdi::{GetTextExtentPoint32W, SetTextColor, SelectObject, DeleteObject};

    let dark = render::system_prefers_dark();
    let pal: Palette = if dark { palette_dark() } else { palette_light() };
    let cfg = config_lock();
    let c = cfg.lock().unwrap();

    // 任务栏底色铺底（支持不透明度：<100 时 AlphaBlend 让任务栏自身 Acrylic 透上来）
    // Windows 11 任务栏底色：深色 #202020、浅色 #F3F3F3。这两个值跟着系统主题走，
    // 不由用户配置 —— 用别的色会与相邻的任务栏按钮明显割裂。
    let bg = if dark {
        render::rgb(0x20, 0x20, 0x20)
    } else {
        render::rgb(0xF3, 0xF3, 0xF3)
    };
    if !render::fill_solid_alpha(hdc, rc, bg, c.bar_opacity) {
        render::fill_solid(hdc, rc, bg);
    }
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

    let avail_w = card_r - card_l;
    let avail_h = card_b - card_t;

    // 1. 文字内容构建（先于布局计算，才能按文字长度决定字号）
    let eta = power::ETA_SECONDS.load(Ordering::Relaxed);
    let cur_mode = mode_now();

    // 充电时只显示电量百分比（Preferences → DISPLAY）：充电中强制走 Percent 分支
    let eff_mode = if charging && c.only_percent_when_charging {
        DisplayMode::Percent
    } else {
        cur_mode
    };

    let text = if eff_mode == DisplayMode::Icon {
        String::new()
    } else if pct < 0 {
        "—".to_string()
    } else {
        match eff_mode {
            DisplayMode::TimeRemaining => {
                if charging {
                    if pct >= 100 {
                        "On A/C".to_string()
                    } else if eta > 0 {
                        format!("{}:{:02}", eta / 3600, (eta % 3600) / 60)
                    } else {
                        "充电中".to_string()
                    }
                } else if eta > 0 {
                    format!("{}:{:02}", eta / 3600, (eta % 3600) / 60)
                } else {
                    "放电中".to_string()
                }
            }
            DisplayMode::Percent => {
                // 充电标识由电池图标上的闪电承担，文字不再重复加 ⚡（小字号会糊成一团）
                // Percentage precision（Display 页）决定小数位：0/1/2。
                match c.percent_precision {
                    0 => format!("{}%", pct),
                    2 => format!("{:.2}%", pct as f64),
                    _ => format!("{:.1}%", pct as f64),
                }
            }
            DisplayMode::TimeFinished => {
                // 原版 3.6.4：显示充满/放空的当日时刻，如 `@ 5:23 PM`
                let fin = power::FINISH_AT.load(Ordering::Relaxed);
                if fin > 0 {
                    format!("@ {}", clock::hm12(fin as i64))
                } else {
                    "—".to_string()
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
                let rem = power::remaining_mwh(pct);
                if rem > 0 {
                    format!("{:.1}Wh", (rem as f64) / 1000.0)
                } else {
                    format!("{}%", pct)
                }
            }
            DisplayMode::Icon => String::new(),
        }
    };

    // 2. 布局：字号档位 0=小 1=中 2=大（Preferences → DISPLAY 可调）
    //    宽条（横排任务栏，≥104px）侧排：图标左 + 文字右
    //    窄条（竖排任务栏 81px）堆叠：图标上 + 文字下（侧排会把文字挤没）
    let font_px = font_px_of(c.font_size);
    let (icon_l, icon_t, icon_r, icon_b, text_mid) = if avail_w >= 104 {
        let icon_h = (avail_h - 4).min(30);
        let icon_w = (icon_h * 18) / 10;
        let icon_l = card_l + 2;
        let icon_t = card_t + (avail_h - icon_h) / 2;
        let mid = (card_t + card_b) / 2;
        (icon_l, icon_t, icon_l + icon_w, icon_t + icon_h, mid)
    } else {
        // 81px 宽的竖排任务栏：图标上、文字下。条高由 bar_height_vertical() 保证
        // 图标盒有 BAR_ICON_BOX_V 的空间，这里不再压缩图标（压了会变扁块）。
        let icon_h = BAR_ICON_BOX_V;
        let icon_w = ((icon_h * 18) / 10).min(avail_w - 8);
        let icon_l = card_l + (avail_w - icon_w) / 2;
        let icon_t = card_t + 1;
        let icon_b = icon_t + icon_h;
        let text_mid = (icon_b + card_b) / 2;
        (icon_l, icon_t, icon_l + icon_w, icon_b, text_mid)
    };

    // 3. 拟物化实体电池（金属外壳 + 正极端子 + 内凹观察窗 + 电解液液面）
    //    或现代极简 Fluent（扁平轨道 + 圆角填充条），由 Preferences 的视觉主题风格决定
    let low = pct <= c.low_percent;
    let critical = pct <= c.critical_percent;
    render::draw_battery_icon(
        hdc,
        icon_l,
        icon_t,
        icon_r,
        icon_b,
        pct,
        charging,
        low,
        critical,
        c.battery_style,
        &pal,
    );

    // 充电时在观察窗内叠加拟物闪电（7x7 像素掩码最近邻缩放，小尺寸不糊）
    if charging {
        let side = ((icon_b - icon_t) * 58) / 100;
        if side >= 5 {
            render::draw_charge_bolt(
                hdc,
                (icon_l + icon_r) / 2,
                (icon_t + icon_b) / 2,
                side,
                side,
                render::lighten(pal.accent_blue, 0.74),
            );
        }
    }

    // 4. 文字：灰度抗锯齿 + Telegram 单色。文字只落在任务栏底色上（不压电池），
    //    对比度已足够，不需要浮雕底衬——小字号下 1px 暗影会变成脏重影。
    if !text.is_empty() {
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        // 负高度 = 字符高度（不含内部行距），见 render::create_ui_font 的说明。
        let font = render::create_ui_font_gray(-font_px, 600);
        let old_font = SelectObject(hdc, font.into());

        let mut size = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &buf, &mut size);

        let (text_l, text_w) = if avail_w >= 104 {
            (icon_r + 7, card_r - (icon_r + 7))
        } else {
            ((card_l + card_r) / 2 - size.cx / 2, size.cx)
        };
        let text_t = text_mid - size.cy / 2;

        let mut drc_text = RECT {
            left: text_l,
            top: text_t,
            right: text_l + text_w + 2,
            bottom: text_t + size.cy + 2,
        };
        SetTextColor(hdc, pal.text);
        DrawTextW(hdc, &mut buf, &mut drc_text, DT_NOPREFIX | DT_SINGLELINE);

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font.into());
    }
}

/// 菜单切模式后保存到 options.json
fn save_mode() {
    let cfg = config_lock();
    let c = cfg.lock().unwrap().clone();
    c.save();
}
