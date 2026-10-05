// batterybar-modern — 详情弹窗
// 点击任务栏条/托盘图标弹出：大百分比 + 状态 + ETA + 电池健康。
// 视觉基准：Telegram 弹出面板（圆角卡片、深色 #17212B、克制留白）。
#![allow(non_snake_case)]

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DRAW_TEXT_FORMAT,
    HGDIOBJ, HDC, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, RegisterClassW,
    SetForegroundWindow, SetTimer, ShowWindow, CS_HREDRAW, CS_VREDRAW, SW_SHOWNOACTIVATE,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_core::{w, PCWSTR};

use crate::power;
use crate::render::{self, palette_dark, palette_light, Palette};

const POPUP_W: i32 = 300;
const POPUP_H: i32 = 170;
const TIMER_ID: usize = 42;

struct Popup {
    palette: Palette,
    font_big: windows::Win32::Graphics::Gdi::HFONT,
    font_med: windows::Win32::Graphics::Gdi::HFONT,
    font_small: windows::Win32::Graphics::Gdi::HFONT,
}

// HFONT/HDC 等 raw handle 不是 Send/Sync；弹窗只在 UI 线程访问，
// 用 unsafe 包装声明单线程约束（窗口销毁时不再读取）。
struct PopupBox(Popup);
unsafe impl Send for PopupBox {}
unsafe impl Sync for PopupBox {}

static POPUP_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static POPUP_BOX: std::sync::OnceLock<PopupBox> = std::sync::OnceLock::new();

/// 点击切换显示；已开则关。
pub fn toggle_popup() {
    unsafe {
        let existing = POPUP_HWND.load(Ordering::Relaxed);
        if existing != 0 {
            let hwnd = HWND(existing as *mut _);
            let _ = DestroyWindow(hwnd);
            POPUP_HWND.store(0, Ordering::Relaxed);
            return;
        }

        static REGISTERED: std::sync::Once = std::sync::Once::new();
        static mut OK: bool = false;
        REGISTERED.call_once(|| {
            let hinstance = match GetModuleHandleW(PCWSTR::null()) {
                Ok(h) => h,
                Err(_) => return,
            };
            let class_name = w!("BBM_Popup");
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(popup_wndproc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
                ..Default::default()
            };
            let atom = RegisterClassW(&wc);
            OK = atom != 0;
        });
        if unsafe { !OK } {
            return;
        }

        // 定位：光标上方，水平居中，贴屏幕边缘裁剪
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut pt);
        let (sw, sh) = render::screen_size();
        let mut x = pt.x - POPUP_W / 2;
        let mut y = pt.y - POPUP_H - 12;
        x = x.clamp(4, sw - POPUP_W - 4);
        y = y.max(4);
        if y > sh - POPUP_H - 40 {
            y = pt.y + 24;
        }

        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h,
            Err(_) => return,
        };
        let class_name = w!("BBM_Popup");
        let dark = render::system_prefers_dark();
        let palette = if dark { palette_dark() } else { palette_light() };

        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let hwnd = match CreateWindowExW(
            ex,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            x,
            y,
            POPUP_W,
            POPUP_H,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(_) => return,
        };

        let _ = POPUP_BOX.set(PopupBox(Popup {
            palette,
            font_big: render::create_ui_font(52, 600),
            font_med: render::create_ui_font(18, 500),
            font_small: render::create_ui_font(14, 400),
        }));

        // Win11 圆角
        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const i32 as *const _,
            4,
        );

        POPUP_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = SetTimer(Some(hwnd), TIMER_ID, 5000, None); // 5s 自动消失
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetForegroundWindow(hwnd); // 临时提权把 NOACTIVATE 窗口带到最前
    }
}

unsafe extern "system" fn popup_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        0x000F /* WM_PAINT */ => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(boxed) = POPUP_BOX.get() {
                paint_popup(&boxed.0, hdc, &ps.rcPaint);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        0x0113 /* WM_TIMER */ => {
            if wparam.0 == TIMER_ID {
                let _ = Kill_Timer(hwnd);
                POPUP_HWND.store(0, Ordering::Relaxed);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ | 0x0204 /* WM_RBUTTONDOWN */ | 0x0084 if false => LRESULT(0),
        0x0201 | 0x0204 => {
            let _ = Kill_Timer(hwnd);
            POPUP_HWND.store(0, Ordering::Relaxed);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        0x0002 /* WM_DESTROY */ => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn Kill_Timer(hwnd: HWND) -> windows_core::Result<()> {
    windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), TIMER_ID)
}

unsafe fn paint_popup(p: &Popup, hdc: HDC, rc: &RECT) {
    let pal = &p.palette;

    // 背景直接实心（窗口有 DWM 圆角，四角由 DWM 裁掉）
    FillRect(hdc, rc, CreateSolidBrush(pal.bg));

    let pct = power::PERCENT.load(Ordering::Relaxed);
    let charging = power::CHARGING.load(Ordering::Relaxed);
    let eta = power::ETA_SECONDS.load(Ordering::Relaxed);
    let health = power::health_percent();

    // 左侧大数字
    let mut big: Vec<u16> = if pct < 0 {
        "—".encode_utf16().collect()
    } else {
        format!("{}%", pct).encode_utf16().collect()
    };
    let fill = if charging {
        pal.fill
    } else if (0..=15).contains(&pct) {
        pal.fill_low
    } else {
        pal.fill
    };
    SetTextColor(hdc, fill);
    let old = SelectObject(hdc, p.font_big.into());
    let mut rc_m = RECT { left: 24, top: 18, right: 170, bottom: 110 };
    let mut buf = big.clone();
    DrawTextW(hdc, &mut buf, &mut rc_m, DT_CALCRECT_FLAG() | DT_NOPREFIX | DT_SINGLELINE);
    let w = rc_m.right - rc_m.left;
    let h = rc_m.bottom - rc_m.top;
    let mut rc_d = RECT { left: 24, top: 18, right: 24 + w, bottom: 18 + h };
    DrawTextW(hdc, &mut big, &mut rc_d, DT_NOPREFIX | DT_SINGLELINE);

    // 右侧状态文本
    SetTextColor(hdc, pal.text);
    SelectObject(hdc, p.font_med.into());
    let status = if !power::AVAILABLE.load(Ordering::Relaxed) {
        "未检测到电池".to_string()
    } else if charging {
        "充电中 ⚡".to_string()
    } else if pct >= 100 {
        "已充满".to_string()
    } else {
        "使用电池".to_string()
    };
    let mut st: Vec<u16> = status.encode_utf16().collect();
    let mut rc_st = RECT { left: 160, top: 34, right: POPUP_W - 20, bottom: 62 };
    DrawTextW(hdc, &mut st, &mut rc_st, DT_NOPREFIX | DT_SINGLELINE);

    // ETA 行
    SetTextColor(hdc, pal.text_dim);
    SelectObject(hdc, p.font_small.into());
    let eta_text = if eta > 0 && power::AVAILABLE.load(Ordering::Relaxed) {
        let hh = (eta / 3600) as i32;
        let mm = ((eta % 3600) / 60) as i32;
        if charging {
            format!("约 {}小时{:02}分充满", hh, mm)
        } else {
            format!("剩余约 {}小时{:02}分", hh, mm)
        }
    } else {
        "—".to_string()
    };
    let mut eta_w: Vec<u16> = eta_text.encode_utf16().collect();
    let mut rc_eta = RECT { left: 160, top: 62, right: POPUP_W - 20, bottom: 86 };
    DrawTextW(hdc, &mut eta_w, &mut rc_eta, DT_NOPREFIX | DT_SINGLELINE);

    // 分隔线
    let pen = CreatePen(PS_SOLID, 1, pal.track);
    let old_pen = SelectObject(hdc, pen.into());
    let _ = MoveToEx(hdc, 24, 104, None);
    let _ = LineTo(hdc, POPUP_W - 24, 104);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(HGDIOBJ(pen.0));

    // 底部信息行：健康度
    SetTextColor(hdc, pal.text_dim);
    SelectObject(hdc, p.font_small.into());
    let cap_text = format!("电池健康 {}%", health);
    let mut info: Vec<u16> = cap_text.encode_utf16().collect();
    let mut rc_info = RECT { left: 24, top: 116, right: POPUP_W - 24, bottom: 142 };
    DrawTextW(hdc, &mut info, &mut rc_info, DT_NOPREFIX | DT_SINGLELINE);

    SelectObject(hdc, old);
}

const fn DT_CALCRECT_FLAG() -> DRAW_TEXT_FORMAT {
    DT_CALCRECT
}
