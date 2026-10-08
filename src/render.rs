// batterybar-modern — GDI 渲染原语
// 颜色、圆角条、文字、DPI。风格基准：Telegram 深浅两主题的克制配色。
#![allow(non_snake_case)]

use windows::Win32::Foundation::{HWND, COLORREF};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, RoundRect, SelectObject,
    SetBkMode, SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
    BACKGROUND_MODE, DRAW_TEXT_FORMAT, FONT_CHARSET, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION,
    FONT_QUALITY, HDC, HFONT, HBRUSH, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: COLORREF,
    pub track: COLORREF,
    pub fill: COLORREF,
    pub fill_low: COLORREF,
    pub text: COLORREF,
    pub text_dim: COLORREF,
    pub border: COLORREF,
}

pub const fn palette_dark() -> Palette {
    Palette {
        // Telegram 深色主题基准 (#17212B 聊天背景 / #2B5278 气泡 / #64B5EF 链接蓝)
        bg: rgb(0x17, 0x21, 0x2B),
        track: rgb(0x24, 0x2F, 0x3D),
        fill: rgb(0x64, 0xB5, 0xEF),
        fill_low: rgb(0xE1, 0x70, 0x5F),
        text: rgb(0xFF, 0xFF, 0xFF),
        text_dim: rgb(0x70, 0x84, 0x97),
        border: rgb(0x10, 0x18, 0x20),
    }
}

pub const fn palette_light() -> Palette {
    // Telegram 浅色主题基准 (#FFFFFF 背景 / #E6ECF0 分隔 / #3390EC 主蓝)
    Palette {
        bg: rgb(0xFF, 0xFF, 0xFF),
        track: rgb(0xE6, 0xEC, 0xF0),
        fill: rgb(0x33, 0x90, 0xEC),
        fill_low: rgb(0xE1, 0x50, 0x52),
        text: rgb(0x00, 0x00, 0x00),
        text_dim: rgb(0x70, 0x74, 0x78),
        border: rgb(0xD7, 0xDD, 0xE2),
    }
}

pub const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    COLORREF(r | (g << 8) | (b << 16))
}

/// Win11 深色偏好探测（AppsUseLightTheme；取不到默认深色）。
pub fn system_prefers_dark() -> bool {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        REG_VALUE_TYPE,
    };
    use windows_core::{w, PCWSTR};

    unsafe {
        let mut hkey = HKEY::default();
        let sub: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let err = RegOpenKeyExW(HKEY_CURRENT_USER, sub, None, KEY_READ, &mut hkey);
        if err.is_err() {
            return true;
        }
        let val: PCWSTR = w!("AppsUseLightTheme");
        let mut data: u32 = 1;
        let mut size = 4u32;
        let mut ty = REG_VALUE_TYPE::default();
        let err2 = RegQueryValueExW(
            hkey,
            val,
            None,
            Some(&mut ty),
            Some((&mut data as *mut u32) as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        err2.is_err() || data == 0
    }
}

/// DPI 感知：Per-Monitor V2。
pub fn enable_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn dpi_for_window(hwnd: HWND) -> u32 {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    unsafe { GetDpiForWindow(hwnd).max(96) }
}

pub fn scale(hwnd: HWND, v: i32) -> i32 {
    (v as i64 * dpi_for_window(hwnd) as i64 / 96) as i32
}

pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

// ---- 绘制原语 ----

pub unsafe fn fill_solid(hdc: HDC, rc: &windows::Win32::Foundation::RECT, color: COLORREF) {
    use windows::Win32::Graphics::Gdi::HGDIOBJ;
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rc, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

/// 圆角实心块（条形/胶囊）
pub unsafe fn fill_round(
    hdc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    color: COLORREF,
) {
    use windows::Win32::Graphics::Gdi::{GetStockObject, NULL_PEN};
    let brush = CreateSolidBrush(color);
    let old_brush = SelectObject(hdc, brush.into());
    let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
    let _ = RoundRect(hdc, left, top, right, bottom, radius * 2, radius * 2);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(brush.into());
}

/// 创建 UI 字体：Segoe UI Variable 回落 Segoe UI（Telegram Desktop 同款字体族）
pub fn create_ui_font(height_px: i32, weight: i32) -> HFONT {
    unsafe {
        let face: windows_core::HSTRING = "Segoe UI Variable Display".into();
        let h = CreateFontW(
            height_px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET(),
            OUT_DEFAULT_PRECIS(),
            CLIP_DEFAULT_PRECIS(),
            CLEARTYPE_QUALITY(),
            0,
            windows_core::PCWSTR::from_raw(face.as_ptr()),
        );
        if h.is_invalid() {
            let fallback: windows_core::HSTRING = "Segoe UI".into();
            CreateFontW(
                height_px,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET(),
                OUT_DEFAULT_PRECIS(),
                CLIP_DEFAULT_PRECIS(),
                CLEARTYPE_QUALITY(),
                0,
                windows_core::PCWSTR::from_raw(fallback.as_ptr()),
            )
        } else {
            h
        }
    }
}

// 0.62 里这些常量是具体类型的关联常量（如 FONT_CHARSET(1u8) 的自由常量同名导出在 Gdi 模块），
// 用自由常量函数封装避免路径书写错误。
pub fn DEFAULT_CHARSET() -> FONT_CHARSET {
    windows::Win32::Graphics::Gdi::DEFAULT_CHARSET
}
pub fn OUT_DEFAULT_PRECIS() -> FONT_OUTPUT_PRECISION {
    windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS
}
pub fn CLIP_DEFAULT_PRECIS() -> FONT_CLIP_PRECISION {
    windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS
}
pub fn CLEARTYPE_QUALITY() -> FONT_QUALITY {
    windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY
}

pub unsafe fn set_text_color(hdc: HDC, c: COLORREF) {
    SetTextColor(hdc, c);
}

pub unsafe fn set_bk_transparent(hdc: HDC) {
    SetBkMode(hdc, TRANSPARENT);
}

/// 量测文本宽度（DT_CALCRECT），返回 (w, h)
pub unsafe fn text_size(hdc: HDC, text: &[u16]) -> (i32, i32) {
    let mut rc = windows::Win32::Foundation::RECT::default();
    let mut buf = text.to_vec();
    DrawTextW(hdc, &mut buf, &mut rc, DT_CALCRECT | DT_NOPREFIX | DT_SINGLELINE);
    (rc.right - rc.left, rc.bottom - rc.top)
}

/// 绘制单行文本
pub unsafe fn draw_text_line(hdc: HDC, text: &mut [u16], rc: *mut windows::Win32::Foundation::RECT) {
    DrawTextW(hdc, text, rc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);
}

pub const DT_VCENTER_FLAG: DRAW_TEXT_FORMAT = DT_VCENTER;
pub const DT_WORDBREAK_FLAG: DRAW_TEXT_FORMAT = DT_WORDBREAK;

/// 原版 BatteryBar 拟物玻璃分层高光（Gloss Split）实体电池绘制器
pub unsafe fn draw_glass_battery_bar(
    hdc: HDC,
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    pct: i32,
    charging: bool,
    low: bool,
    critical: bool,
    dark_mode: bool,
) {
    use windows::Win32::Graphics::Gdi::{
        CreatePen, GetStockObject, LineTo, MoveToEx, RoundRect, SelectObject, HGDIOBJ,
        HOLLOW_BRUSH, PS_SOLID,
    };

    let total_w = r - l;
    let h = b - t;
    if total_w <= 12 || h <= 8 {
        return;
    }

    // 右侧金属正极端子（3px 凸起，让它一眼看过去就是真正的实体电池）
    let term_w = 3i32;
    let body_r = r - term_w - 1;
    let term_h = 10i32;
    let term_t = t + (h - term_h) / 2;
    let term_b = term_t + term_h;
    let term_color = if dark_mode { rgb(120, 130, 142) } else { rgb(180, 190, 200) };
    fill_round(hdc, body_r, term_t, r, term_b, 1, term_color);

    let radius = 4i32;

    // 1. 电池晶莹外壳底槽（未填充电量的透明暗凹槽）
    let track_bg_top = if dark_mode { rgb(32, 40, 48) } else { rgb(220, 226, 232) };
    let track_bg_bot = if dark_mode { rgb(18, 24, 30) } else { rgb(195, 202, 210) };
    let split_h = t + (h * 45) / 100;

    fill_round(hdc, l, t, body_r, b, radius, track_bg_bot);
    fill_round(hdc, l + 1, t + 1, body_r - 1, split_h, radius.max(2), track_bg_top);

    // 2. 拟物电量流体填充（Progress Fill）
    let valid_pct = if pct < 0 { 0 } else { pct.clamp(0, 100) };
    let inner_w = (body_r - l - 2) as i64;
    let fw = (inner_w * valid_pct as i64 / 100) as i32;

    if fw > 2 {
        let fl = l + 1;
        let fr = fl + fw;
        let (gloss_top, gloss_bot, gloss_line) = if charging {
            // 充电：晶莹通透天蓝（原版充电经典色）
            (rgb(90, 210, 255), rgb(24, 125, 225), rgb(180, 240, 255))
        } else if critical {
            // 临界低电：鲜明珊瑚红 / 深警示红
            (rgb(255, 115, 105), rgb(195, 30, 30), rgb(255, 175, 170))
        } else if low {
            // 低电：透亮金橙 / 浓琥珀橙
            (rgb(255, 200, 75), rgb(225, 125, 12), rgb(255, 230, 140))
        } else {
            // 正常放电：晶莹透亮翠绿（原版默认）/ 浓郁深翠绿
            (rgb(130, 228, 115), rgb(42, 160, 52), rgb(185, 248, 175))
        };

        // 下半层主色
        fill_round(hdc, fl, t + 1, fr, b - 1, radius.max(2), gloss_bot);
        // 上半层透亮高光反射
        fill_round(hdc, fl, t + 1, fr, split_h, radius.max(2), gloss_top);

        // 1px 晶莹高光分割线
        let pen_gloss = CreatePen(PS_SOLID, 1, gloss_line);
        let old_p = SelectObject(hdc, pen_gloss.into());
        let _ = MoveToEx(hdc, fl + 1, split_h, None);
        let _ = LineTo(hdc, fr - 1, split_h);
        SelectObject(hdc, old_p);
        let _ = DeleteObject(HGDIOBJ(pen_gloss.0));
    }

    // 3. 拟物水晶外框（双层倒角：外层深暗边框，必须用 HOLLOW_BRUSH 防白色覆盖）
    let border_color = if dark_mode { rgb(50, 60, 72) } else { rgb(175, 185, 195) };
    let pen_border = CreatePen(PS_SOLID, 1, border_color);
    let old_pen = SelectObject(hdc, pen_border.into());
    let old_brush = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH)); // 必须用空画刷！
    let _ = RoundRect(hdc, l, t, body_r, b, radius * 2, radius * 2);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(HGDIOBJ(pen_border.0));
}

#[allow(unused)]
fn _use(b: HBRUSH) -> HBRUSH {
    b
}
