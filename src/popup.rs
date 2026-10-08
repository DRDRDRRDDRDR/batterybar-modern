// batterybar-modern — Status Popup（原版复刻）
// 原版字段：Percent / Time Remaining / Capacity / Discharge Rate / Battery / Full Lifetime / A/C / Battery Wear
#![allow(non_snake_case)]

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK,
    DRAW_TEXT_FORMAT, HGDIOBJ, HDC, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, RegisterClassW,
    SetForegroundWindow, SetTimer, ShowWindow, CS_HREDRAW, CS_VREDRAW, SW_SHOWNOACTIVATE,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_core::{w, PCWSTR};

use crate::power;
use crate::render;

const POPUP_W: i32 = 320;
const ROW_H: i32 = 24;
const POPUP_H: i32 = 44 + ROW_H * 8 + 14;
const TIMER_ID: usize = 42;

struct Popup {
    palette: render::Palette,
    font_title: windows::Win32::Graphics::Gdi::HFONT,
    font_key: windows::Win32::Graphics::Gdi::HFONT,
    font_val: windows::Win32::Graphics::Gdi::HFONT,
}

// HFONT/HDC raw handle 不是 Send/Sync；弹窗只在 UI 线程访问。
struct PopupBox(Popup);
unsafe impl Send for PopupBox {}
unsafe impl Sync for PopupBox {}

static POPUP_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static POPUP_BOX: std::sync::OnceLock<PopupBox> = std::sync::OnceLock::new();

/// hover/点击唤起（已开则刷新位置）
pub fn toggle_popup() {
    unsafe {
        let existing = POPUP_HWND.load(Ordering::Relaxed);
        if existing != 0 {
            // 已开：只刷新数据（重绘），不关闭 —— hover 唤起时保持
            let hwnd = HWND(existing as *mut _);
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
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
            OK = RegisterClassW(&wc) != 0;
        });
        if unsafe { !OK } {
            return;
        }

        // 定位：光标上方，贴屏幕边缘裁剪
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut pt);
        let (sw, sh) = render::screen_size();
        let mut x = pt.x + 12;
        let mut y = pt.y - POPUP_H - 8;
        if x + POPUP_W > sw - 4 {
            x = pt.x - POPUP_W - 12;
        }
        x = x.clamp(4, sw - POPUP_W - 4);
        y = y.max(4);
        if y > sh - POPUP_H - 40 {
            y = pt.y + ROW_H + 4;
        }

        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h,
            Err(_) => return,
        };
        let class_name = w!("BBM_Popup");
        let dark = render::system_prefers_dark();
        let palette = if dark { render::palette_dark() } else { render::palette_light() };

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
            font_title: render::create_ui_font(16, 600),
            font_key: render::create_ui_font(13, 400),
            font_val: render::create_ui_font(13, 600),
        }));

        // Win11 圆角
        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const _, 4);

        POPUP_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = SetTimer(Some(hwnd), TIMER_ID, 6000, None); // 6s 自动消失
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn close_popup() {
    let existing = POPUP_HWND.load(Ordering::Relaxed);
    if existing != 0 {
        unsafe {
            let _ = DestroyWindow(HWND(existing as *mut _));
        }
        POPUP_HWND.store(0, Ordering::Relaxed);
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
                let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), TIMER_ID);
                POPUP_HWND.store(0, Ordering::Relaxed);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ | 0x0204 /* WM_RBUTTONDOWN */ => {
            let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), TIMER_ID);
            POPUP_HWND.store(0, Ordering::Relaxed);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        0x0002 /* WM_DESTROY */ => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

struct Row {
    key: &'static str,
    val: String,
    color: Option<windows::Win32::Foundation::COLORREF>,
}

unsafe fn paint_popup(p: &Popup, hdc: HDC, rc: &RECT) {
    let pal = &p.palette;
    FillRect(hdc, rc, CreateSolidBrush(pal.bg));
    render::set_bk_transparent(hdc);

    let pct = power::PERCENT.load(Ordering::Relaxed);
    let charging = power::CHARGING.load(Ordering::Relaxed);
    let eta = power::ETA_SECONDS.load(Ordering::Relaxed);
    let full_life = power::FULL_LIFETIME_SECONDS.load(Ordering::Relaxed);
    let elapsed = power::ELAPSED_ON_BATTERY.load(Ordering::Relaxed);
    let rate = power::CHARGE_RATE_MW.load(Ordering::Relaxed);
    let rem = power::REMAIN_MWH.load(Ordering::Relaxed);
    let full = power::FULL_MWH.load(Ordering::Relaxed);
    let health = power::health_percent();

    let fmt_hms = |s: u64| -> String {
        if s == 0 { "—".into() } else { format!("{}h {:02}m", s / 3600, (s % 3600) / 60) }
    };

    let state_color = if charging {
        render::rgb(0x6D, 0xC2, 0x4B)
    } else if (0..=10).contains(&pct) {
        render::rgb(0xE0, 0x48, 0x38)
    } else if (0..=20).contains(&pct) {
        render::rgb(0xF0, 0xA0, 0x30)
    } else {
        render::rgb(0x33, 0xC5, 0xF0)
    };

    let rows = [
        Row { key: "电量", val: if pct < 0 { "—".into() } else { format!("{}%", pct) }, color: Some(state_color) },
        Row { key: "剩余时间", val: if charging { fmt_hms(eta) .replace("—", "充电中") } else { fmt_hms(eta) }, color: None },
        Row { key: "容量", val: if full > 0 { format!("{} / {} mWh", rem, full) } else { "—".into() }, color: None },
        Row { key: if charging { "充电速率" } else { "放电速率" }, val: format!("{} mW", rate), color: None },
        Row { key: "电池状态", val: if charging { "充电中 ⚡".into() } else if pct >= 100 { "已充满".into() } else { "使用电池".into() }, color: None },
        Row { key: "满电可用", val: fmt_hms(full_life), color: None },
        Row { key: "电源", val: if charging { "A/C 已接通".into() } else { "使用电池".into() }, color: None },
        Row { key: "电池损耗", val: if health > 0 { format!("{}%（健康 {}%）", 100 - health, health) } else { "—".into() }, color: None },
    ];

    // 标题
    SetTextColor(hdc, pal.text);
    SelectObject(hdc, p.font_title.into());
    let mut title: Vec<u16> = "BatteryBar Modern".encode_utf16().collect();
    let mut trc = RECT { left: 16, top: 10, right: POPUP_W - 16, bottom: 34 };
    DrawTextW(hdc, &mut title, &mut trc, DT_NOPREFIX | DT_SINGLELINE);

    // 分隔线
    let pen = CreatePen(PS_SOLID, 1, pal.track);
    let old_pen = SelectObject(hdc, pen.into());
    let _ = MoveToEx(hdc, 16, 38, None);
    let _ = LineTo(hdc, POPUP_W - 16, 38);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(HGDIOBJ(pen.0));

    // 行
    let mut y = 44;
    for (i, row) in rows.iter().enumerate() {
        let row_y = y + i as i32 * ROW_H;
        // key
        SetTextColor(hdc, pal.text_dim);
        SelectObject(hdc, p.font_key.into());
        let mut k: Vec<u16> = row.key.encode_utf16().collect();
        let mut krc = RECT { left: 16, top: row_y, right: 120, bottom: row_y + ROW_H };
        DrawTextW(hdc, &mut k, &mut krc, DT_NOPREFIX | DT_SINGLELINE);
        // val
        SetTextColor(hdc, row.color.unwrap_or(pal.text));
        SelectObject(hdc, p.font_val.into());
        let mut v: Vec<u16> = row.val.encode_utf16().collect();
        let mut vrc = RECT { left: 124, top: row_y, right: POPUP_W - 16, bottom: row_y + ROW_H };
        DrawTextW(hdc, &mut v, &mut vrc, DT_NOPREFIX | DT_SINGLELINE);
    }
    let _ = DT_WORDBREAK;
    let _ = DT_CALCRECT;
}
