// batterybar-modern — Preferences 窗口（原版复刻）
// General / Display / Notifications / Warning Levels 的可编辑开关。
#![allow(non_snake_case)]

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_NOPREFIX, DT_SINGLELINE, HGDIOBJ, HDC, PAINTSTRUCT,
    PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, RegisterClassW, ShowWindow,
    CS_HREDRAW, CS_VREDRAW, SW_SHOW, WNDCLASSW, WS_CAPTION, WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows_core::{w, PCWSTR};

use crate::config::{Config, DisplayMode};
use crate::render;

const PREFS_W: i32 = 360;
const ROW_H: i32 = 30;
const PREFS_H: i32 = 52 + ROW_H * 9 + 16;

struct Prefs {
    palette: render::Palette,
    font_title: windows::Win32::Graphics::Gdi::HFONT,
    font_row: windows::Win32::Graphics::Gdi::HFONT,
    font_val: windows::Win32::Graphics::Gdi::HFONT,
}

struct PrefsBox(Prefs);
unsafe impl Send for PrefsBox {}
unsafe impl Sync for PrefsBox {}

static PREFS_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static PREFS_BOX: std::sync::OnceLock<PrefsBox> = std::sync::OnceLock::new();

// 行定义: (组标签索引, 开关字段说明)
// 0-1 General / 2-5 Display / 6-8 Notifications+Warning
const ROW_GENERAL: usize = 0;
const ROW_DISPLAY: usize = 3;
const ROW_WARN: usize = 7;

fn row_kind(i: usize) -> &'static str {
    match i {
        0 => "general_head",
        1 => "show_tooltip",
        2 => "hide_when_full",
        3 => "display_head",
        4 => "mode",
        5 => "only_percent_when_charging",
        6 => "warn_head",
        7 => "low_percent",
        8 => "critical_percent",
        _ => "",
    }
}

pub fn toggle_prefs() {
    unsafe {
        let existing = PREFS_HWND.load(Ordering::Relaxed);
        if existing != 0 {
            let _ = DestroyWindow(HWND(existing as *mut _));
            PREFS_HWND.store(0, Ordering::Relaxed);
            return;
        }

        static REGISTERED: std::sync::Once = std::sync::Once::new();
        static mut OK: bool = false;
        REGISTERED.call_once(|| {
            let hinstance = match GetModuleHandleW(PCWSTR::null()) {
                Ok(h) => h,
                Err(_) => return,
            };
            let class_name = w!("BBM_Prefs");
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(prefs_wndproc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
                ..Default::default()
            };
            OK = RegisterClassW(&wc) != 0;
        });
        if unsafe { !OK } {
            return;
        }

        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h,
            Err(_) => return,
        };
        let class_name = w!("BBM_Prefs");
        let dark = render::system_prefers_dark();
        let palette = if dark { render::palette_dark() } else { render::palette_light() };

        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            w!("BatteryBar Modern · Preferences"),
            WS_CAPTION | WS_SYSMENU,
            200,
            200,
            PREFS_W,
            PREFS_H,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(_) => return,
        };

        let _ = PREFS_BOX.set(PrefsBox(Prefs {
            palette,
            font_title: render::create_ui_font(14, 600),
            font_row: render::create_ui_font(13, 400),
            font_val: render::create_ui_font(13, 600),
        }));

        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const _, 4);

        PREFS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

unsafe extern "system" fn prefs_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        0x000F /* WM_PAINT */ => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(boxed) = PREFS_BOX.get() {
                paint_prefs(&boxed.0, hdc, &ps.rcPaint);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ => {
            let l = (lparam.0 & 0xFFFF) as i32;
            let t = ((lparam.0 >> 16) & 0xFFFF) as i32;
            handle_click(hwnd, l, t);
            LRESULT(0)
        }
        0x0010 /* WM_CLOSE */ => {
            let _ = DestroyWindow(hwnd);
            PREFS_HWND.store(0, Ordering::Relaxed);
            LRESULT(0)
        }
        0x0002 /* WM_DESTROY */ => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn handle_click(hwnd: HWND, x: i32, y: i32) {
    let row = (y - 44) / ROW_H;
    if !(0..9).contains(&row) {
        return;
    }
    let kind = row_kind(row as usize);
    let mut save = false;
    {
        let cfg = crate::config_lock();
        let mut c = cfg.lock().unwrap();
        match kind {
            "show_tooltip" => { c.show_tooltip = !c.show_tooltip; save = true; }
            "hide_when_full" => { c.hide_when_full = !c.hide_when_full; save = true; }
            "only_percent_when_charging" => { c.only_percent_when_charging = !c.only_percent_when_charging; save = true; }
            "mode" => {
                // 循环 time -> percent -> icon
                c.default_mode = match c.default_mode {
                    DisplayMode::TimeRemaining => DisplayMode::Percent,
                    DisplayMode::Percent => DisplayMode::Icon,
                    DisplayMode::Icon => DisplayMode::TimeRemaining,
                };
                crate::set_mode(c.default_mode);
                save = true;
            }
            "low_percent" => {
                c.low_percent = if c.low_percent >= 45 { 10 } else { c.low_percent + 5 };
                if c.critical_percent >= c.low_percent { c.critical_percent = c.low_percent - 5; }
                save = true;
            }
            "critical_percent" => {
                c.critical_percent = if c.critical_percent >= c.low_percent - 5 { 5 } else { c.critical_percent + 5 };
                save = true;
            }
            _ => {}
        }
        if save {
            // 自启开关跟随 Run 键
            let enable = c.run_at_startup;
            let c2 = c.clone();
            c2.set_startup(enable);
            c2.save();
        }
    }
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
}

unsafe fn paint_prefs(p: &Prefs, hdc: HDC, rc: &RECT) {
    let pal = &p.palette;
    FillRect(hdc, rc, CreateSolidBrush(pal.bg));
    render::set_bk_transparent(hdc);

    let cfg = crate::config_lock();
    let c = cfg.lock().unwrap();

    // 组标题 + 行
    let heads: [(usize, &str); 3] = [
        (ROW_GENERAL, "GENERAL"),
        (ROW_DISPLAY, "DISPLAY"),
        (ROW_WARN, "WARNING LEVELS"),
    ];

    for (idx, name) in heads {
        let y = 44 + idx as i32 * ROW_H;
        SetTextColor(hdc, pal.text_dim);
        SelectObject(hdc, p.font_title.into());
        let mut t: Vec<u16> = name.encode_utf16().collect();
        let mut trc = RECT { left: 16, top: y, right: PREFS_W - 16, bottom: y + ROW_H };
        DrawTextW(hdc, &mut t, &mut trc, DT_NOPREFIX | DT_SINGLELINE);
    }

    // 开关行绘制: (行, 文本, 状态)
    let draw_switch = |row: i32, text: &str, on: Option<bool>, val: Option<String>| {
        let y = 44 + row * ROW_H;
        // 文本
        SetTextColor(hdc, pal.text);
        SelectObject(hdc, p.font_row.into());
        let mut b: Vec<u16> = text.encode_utf16().collect();
        let mut brc = RECT { left: 16, top: y, right: PREFS_W - 80, bottom: y + ROW_H };
        DrawTextW(hdc, &mut b, &mut brc, DT_NOPREFIX | DT_SINGLELINE);
        // 右侧：开关/值
        SelectObject(hdc, p.font_val.into());
        if let Some(on) = on {
            // 胶囊开关
            let sw_w = 40i32;
            let sw_h = 20i32;
            let x = PREFS_W - 16 - sw_w;
            let yy = y + (ROW_H - sw_h) / 2;
            let color = if on { pal.fill } else { pal.track };
            render::fill_round(hdc, x, yy, x + sw_w, yy + sw_h, 10, color);
            // 圆点
            let dot = 14i32;
            let dx = if on { x + sw_w - dot - 3 } else { x + 3 };
            render::fill_round(hdc, dx, yy + 3, dx + dot, yy + 3 + dot, 7, pal.bg);
        } else if let Some(v) = val {
            SetTextColor(hdc, pal.fill);
            let mut s: Vec<u16> = v.encode_utf16().collect();
            let mut src = RECT { left: PREFS_W - 80, top: y, right: PREFS_W - 16, bottom: y + ROW_H };
            DrawTextW(hdc, &mut s, &mut src, DT_NOPREFIX | DT_SINGLELINE);
        }
    };

    draw_switch(1, "悬停显示详细信息", Some(c.show_tooltip), None);
    draw_switch(2, "满电时隐藏条", Some(c.hide_when_full), None);
    draw_switch(4, "条上显示", None, Some(mode_text(&c)));
    draw_switch(5, "充电时只显示百分比", Some(c.only_percent_when_charging), None);
    draw_switch(7, "低电量阈值", None, Some(format!("{}%", c.low_percent)));
    draw_switch(8, "临界电量阈值", None, Some(format!("{}%", c.critical_percent)));

    // 底部说明
    SetTextColor(hdc, pal.text_dim);
    SelectObject(hdc, p.font_row.into());
    let mut hint: Vec<u16> = "点击行切换 · 配置保存至 options.json".encode_utf16().collect();
    let mut hrc = RECT { left: 16, top: PREFS_H - 34, right: PREFS_W - 16, bottom: PREFS_H - 10 };
    DrawTextW(hdc, &mut hint, &mut hrc, DT_NOPREFIX | DT_SINGLELINE);
}

fn mode_text(c: &Config) -> String {
    match c.default_mode {
        DisplayMode::TimeRemaining => "时间剩余".into(),
        DisplayMode::Percent => "百分比".into(),
        DisplayMode::Icon => "图标".into(),
    }
}
