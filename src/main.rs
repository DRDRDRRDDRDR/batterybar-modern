// batterybar-modern — 入口与主窗口（任务栏嵌入条）
//
// 三个窗口：
//   1. bar window      —— 实际显示电量条的窗口，SetParent 进 Shell_TrayWnd
//   2. message window  —— 托盘图标宿主（不可见）
//   3. popup window    —— 详情弹窗（popup.rs）
#![allow(non_snake_case)]

mod popup;
mod power;
mod render;
mod taskbar;
mod tray;

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, DeleteObject, DrawTextW, EndPaint, InvalidateRect, LineTo, MoveToEx,
    SelectObject, SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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

use crate::render::{palette_dark, palette_light, Palette};

// ---- 配置常量（后续可提为 options.json）----
const BAR_WIDTH: i32 = 132; // 物理像素
const BAR_HEIGHT_PAD: i32 = 10; // 条高 = 任务栏高 - 2*PAD
const TIMER_REFRESH: usize = 1; // 周期刷新
const REFRESH_MS: u32 = 1000;
const TRAY_TIMER: usize = 2;
const TRAY_POLL_MS: u32 = 30_000;

const ID_MENU_EXIT: usize = 0x1001;
const ID_MENU_TOGGLE: usize = 0x1002;

struct BarWnd {
    hwnd: HWND,
}

static BAR_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TRAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static REFRESH_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_SIG: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);

fn main() -> windows_core::Result<()> {
    render::enable_dpi_awareness();

    let hinstance = unsafe { GetModuleHandleW(PCWSTR::null())? };

    // 注册条窗口类
    let bar_class = w!("BBM_Bar");
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
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

        // 7) 消息循环
        let mut msg = WAM::MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// 重新计算条的位置尺寸（任务栏内，托盘左侧）
fn relayout(bar: HWND) {
    unsafe {
        let parent = match GetParent(bar) {
            Ok(p) => p,
            Err(_) => return,
        };
        let (x, cy) = taskbar::tray_left_estimate(parent);
        let h = (cy - BAR_HEIGHT_PAD).max(12);
        let _ = taskbar::place(bar, x - BAR_WIDTH - 6, (cy - h) / 2, BAR_WIDTH, h);
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
            paint_bar(hdc, &ps.rcPaint);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WAM::WM_LBUTTONUP => {
            popup::toggle_popup();
            LRESULT(0)
        }
        WAM::WM_RBUTTONUP => {
            show_bar_menu(hwnd);
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
            }
            LRESULT(0)
        }
        WAM::WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_REFRESH);
            let _ = PostQuitMessage(0);
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

fn show_bar_menu(hwnd: HWND) {
    unsafe {
        if let Ok(menu) = CreatePopupMenu() {
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_TOGGLE, w!("显示/隐藏"));
            let _ = AppendMenuW(menu, MENU_ITEM_FLAGS(0), ID_MENU_EXIT, w!("退出"));
            let mut pt = POINT::default();
            let _ = WAM::GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(
                menu,
                TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                None,
                hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
        }
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
    let dark = render::system_prefers_dark();
    let pal: Palette = if dark { palette_dark() } else { palette_light() };

    // Win11 任务栏底色：深色主题 ≈ #202020，浅色 ≈ #F3F3F3；直接贴同色实现"透明"
    let bg = if dark {
        render::rgb(0x20, 0x20, 0x20)
    } else {
        render::rgb(0xF3, 0xF3, 0xF3)
    };
    render::fill_solid(hdc, rc, bg);

    let pct = power::PERCENT.load(Ordering::Relaxed);
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    let cy = rc.top + h / 2;

    if pct < 0 {
        // 无电池：哑色短横线
        render::fill_round(hdc, rc.left + 8, cy - 2, rc.right - 8, cy + 2, 2, pal.track);
        return;
    }

    // 轨道
    let bar_left = rc.left + 8;
    let bar_right = rc.right - 8;
    let bar_top = cy - 5;
    let bar_bottom = cy + 5;
    render::fill_round(hdc, bar_left, bar_top, bar_right, bar_bottom, 5, pal.track);

    // 填充：充电 = 蓝；≤15% = 红；其余 蓝→绿 按电量插值
    let charging = power::CHARGING.load(Ordering::Relaxed);
    let fill = if charging {
        pal.fill
    } else if pct <= 15 {
        pal.fill_low
    } else {
        lerp_color(pal.fill, render::rgb(0x4C, 0xAF, 0x50), pct as f32 / 100.0)
    };
    let span = (bar_right - bar_left) as i64;
    let fw = (span * pct.clamp(2, 100) as i64 / 100) as i32;
    if fw > 4 {
        render::fill_round(hdc, bar_left, bar_top, bar_left + fw, bar_bottom, 5, fill);
    }

    // 居中百分比文字（白字，深色任务栏可读；浅色主题切黑字）
    let text = format!("{}%", pct);
    let mut buf: Vec<u16> = text.encode_utf16().collect();
    SetTextColor(hdc, if dark { pal.text } else { pal.text });
    let font = render::create_ui_font(12, 600);
    let old = SelectObject(hdc, font.into());
    let mut trc = RECT::default();
    DrawTextW(hdc, &mut buf, &mut trc, DT_CALCRECT | DT_NOPREFIX | DT_SINGLELINE);
    let tw = trc.right - trc.left;
    let th = trc.bottom - trc.top;
    let mut drc = RECT {
        left: (rc.left + rc.right) / 2 - tw / 2,
        top: cy - th / 2,
        right: (rc.left + rc.right) / 2 - tw / 2 + tw + 2,
        bottom: cy - th / 2 + th + 2,
    };
    DrawTextW(hdc, &mut buf, &mut drc, DT_NOPREFIX | DT_SINGLELINE);
    SelectObject(hdc, old);
    let _ = DeleteObject(font.into());

    // 轨道描边（细线增强可读性）
    let pen = CreatePen(PS_SOLID, 1, if dark { render::rgb(0x40, 0x40, 0x40) } else { render::rgb(0xC8, 0xC8, 0xC8) });
    let old_pen = SelectObject(hdc, pen.into());
    let _ = WAM_StrokeRect(hdc, bar_left, bar_top, bar_right, bar_bottom);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen.into());
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
