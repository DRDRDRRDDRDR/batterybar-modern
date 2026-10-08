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

#[allow(unused)]
fn _use(b: HBRUSH) -> HBRUSH {
    b
}
