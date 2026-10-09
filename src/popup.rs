// batterybar-modern — Status Popup（Telegram Acrylic Card 风格）
// 深度借鉴 Telegram Desktop 视效：优雅圆角卡片、大号 Hero 数据、内嵌槽位分组、克制对比度。
#![allow(non_snake_case)]

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DrawTextW, EndPaint, FillRect, SelectObject, SetTextColor,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC, PAINTSTRUCT,
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

const POPUP_W: i32 = 330;
const POPUP_H: i32 = 360;
const TIMER_ID: usize = 42;

struct Popup {
    palette: render::Palette,
    font_hero: windows::Win32::Graphics::Gdi::HFONT,
    font_sub: windows::Win32::Graphics::Gdi::HFONT,
    font_title: windows::Win32::Graphics::Gdi::HFONT,
    font_key: windows::Win32::Graphics::Gdi::HFONT,
    font_val: windows::Win32::Graphics::Gdi::HFONT,
}

struct PopupBox(Popup);
unsafe impl Send for PopupBox {}
unsafe impl Sync for PopupBox {}

static POPUP_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static POPUP_BOX: std::sync::OnceLock<PopupBox> = std::sync::OnceLock::new();

pub fn toggle_popup() {
    unsafe {
        let existing = POPUP_HWND.load(Ordering::Relaxed);
        if existing != 0 {
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
            y = pt.y + 24;
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
            font_hero: render::create_ui_font(28, 700),
            font_sub: render::create_ui_font(12, 500),
            font_title: render::create_ui_font(14, 600),
            font_key: render::create_ui_font(12, 400),
            font_val: render::create_ui_font(12, 600),
        }));

        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const _, 4);

        POPUP_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = SetTimer(Some(hwnd), TIMER_ID, 6000, None);
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
                paint_popup(&boxed.0, hdc);
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
        0x0002 /* WM_DESTROY */ => {
            POPUP_HWND.store(0, Ordering::Relaxed);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
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

unsafe fn paint_popup(p: &Popup, hdc: HDC) {
    let pal = &p.palette;
    let rc = RECT { left: 0, top: 0, right: POPUP_W, bottom: POPUP_H };
    FillRect(hdc, &rc, CreateSolidBrush(pal.bg));
    render::stroke_round(hdc, 0, 0, POPUP_W, POPUP_H, 10, pal.border);
    render::set_bk_transparent(hdc);

    let pct = power::PERCENT.load(Ordering::Relaxed);
    let charging = power::CHARGING.load(Ordering::Relaxed);
    let eta = power::ETA_SECONDS.load(Ordering::Relaxed);
    let full_life = power::FULL_LIFETIME_SECONDS.load(Ordering::Relaxed);
    let rate = power::CHARGE_RATE_MW.load(Ordering::Relaxed);
    let rem = power::REMAIN_MWH.load(Ordering::Relaxed);
    let full = power::FULL_MWH.load(Ordering::Relaxed);
    let health = power::health_percent();

    let fmt_hms = |s: u64| -> String {
        if s == 0 { "—".into() } else { format!("{}h {:02}m", s / 3600, (s % 3600) / 60) }
    };

    // 1. 顶部 Header：优雅 Telegram 蓝小徽章 + 应用标题
    let badge_x = 16;
    let badge_y = 14;
    render::fill_round(hdc, badge_x, badge_y, badge_x + 28, badge_y + 28, 8, pal.accent_blue);
    SetTextColor(hdc, render::rgb(255, 255, 255));
    SelectObject(hdc, p.font_title.into());
    let mut icon_s: Vec<u16> = "⚡".encode_utf16().collect();
    let mut irc = RECT { left: badge_x, top: badge_y + 4, right: badge_x + 28, bottom: badge_y + 28 };
    DrawTextW(hdc, &mut icon_s, &mut irc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_CENTER);

    SetTextColor(hdc, pal.text);
    SelectObject(hdc, p.font_title.into());
    let mut title: Vec<u16> = "BatteryBar Modern".encode_utf16().collect();
    let mut trc = RECT { left: 52, top: 12, right: POPUP_W - 16, bottom: 30 };
    DrawTextW(hdc, &mut title, &mut trc, DT_NOPREFIX | DT_SINGLELINE);

    SetTextColor(hdc, pal.text_dim);
    SelectObject(hdc, p.font_sub.into());
    let mut sub: Vec<u16> = "Windows 11 Battery Monitor".encode_utf16().collect();
    let mut ssubrc = RECT { left: 52, top: 28, right: POPUP_W - 16, bottom: 44 };
    DrawTextW(hdc, &mut sub, &mut ssubrc, DT_NOPREFIX | DT_SINGLELINE);

    // 2. 核心大数字 Hero Card
    let hero_l = 16;
    let hero_t = 50;
    let hero_r = POPUP_W - 16;
    let hero_b = 120;
    render::fill_round(hdc, hero_l, hero_t, hero_r, hero_b, 10, pal.surface_dim);
    render::stroke_round(hdc, hero_l, hero_t, hero_r, hero_b, 10, pal.border);

    // 大号数字
    SetTextColor(hdc, pal.text);
    SelectObject(hdc, p.font_hero.into());
    let hero_text = if pct < 0 { "—".to_string() } else { format!("{}%", pct) };
    let mut hbuf: Vec<u16> = hero_text.encode_utf16().collect();
    let mut hrc = RECT { left: hero_l, top: hero_t + 10, right: hero_r, bottom: hero_t + 44 };
    DrawTextW(hdc, &mut hbuf, &mut hrc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_CENTER);

    // 状态小徽章
    let status_str = if charging {
        if pct >= 100 { "● 已充满 · 接通电源".to_string() } else if eta > 0 { format!("● 充电中 · 剩余 {}", fmt_hms(eta)) } else { "● 正在充电 · 接通电源".to_string() }
    } else {
        if eta > 0 { format!("● 使用电池 · 剩余可用 {}", fmt_hms(eta)) } else { "● 使用电池放电中".to_string() }
    };
    SetTextColor(hdc, if charging { pal.accent_blue } else if pct <= 20 { pal.accent_warn } else { pal.accent_green });
    SelectObject(hdc, p.font_sub.into());
    let mut stbuf: Vec<u16> = status_str.encode_utf16().collect();
    let mut strc = RECT { left: hero_l, top: hero_t + 46, right: hero_r, bottom: hero_b - 6 };
    DrawTextW(hdc, &mut stbuf, &mut strc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_CENTER);

    // 3. 分组列表（Grouped Cells）
    let draw_cell_group = |start_y: i32, rows: &[(&str, String, Option<windows::Win32::Foundation::COLORREF>)]| -> i32 {
        let group_l = 16;
        let group_r = POPUP_W - 16;
        let row_h = 28;
        let group_h = rows.len() as i32 * row_h + 8;
        render::fill_round(hdc, group_l, start_y, group_r, start_y + group_h, 8, pal.surface);
        render::stroke_round(hdc, group_l, start_y, group_r, start_y + group_h, 8, pal.border);

        for (idx, (k, v, color)) in rows.iter().enumerate() {
            let y = start_y + 4 + idx as i32 * row_h;
            // key
            SetTextColor(hdc, pal.text_dim);
            SelectObject(hdc, p.font_key.into());
            let mut kbuf: Vec<u16> = k.encode_utf16().collect();
            let mut krc = RECT { left: group_l + 12, top: y, right: group_l + 130, bottom: y + row_h };
            DrawTextW(hdc, &mut kbuf, &mut krc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);

            // value
            SetTextColor(hdc, color.unwrap_or(pal.text));
            SelectObject(hdc, p.font_val.into());
            let mut vbuf: Vec<u16> = v.encode_utf16().collect();
            let mut vrc = RECT { left: group_l + 130, top: y, right: group_r - 12, bottom: y + row_h };
            DrawTextW(hdc, &mut vbuf, &mut vrc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER | windows::Win32::Graphics::Gdi::DT_RIGHT);
        }
        start_y + group_h + 10
    };

    let group1 = [
        ("电量容量", if full > 0 { format!("{:.1} / {:.1} Wh", rem as f64 / 1000.0, full as f64 / 1000.0) } else { "—".into() }, None),
        ("实时功耗", if rate == 0 { "0 mW".into() } else { format!("{} mW", rate) }, Some(if rate < 0 { pal.accent_blue } else { pal.text })),
        ("满电可用时长", fmt_hms(full_life), None),
    ];
    let next_y = draw_cell_group(130, &group1);

    let group2 = [
        ("电池损耗率", if health > 0 { format!("{}% (健康度 {}%)", 100 - health, health) } else { "—".into() }, Some(if health < 80 { pal.accent_warn } else { pal.accent_green })),
        ("供电模式", if charging { "A/C 交流电已接通".into() } else { "便携电池供电".into() }, None),
        ("电源计划", get_active_power_scheme(), Some(pal.accent_blue)),
    ];
    draw_cell_group(next_y, &group2);
}
