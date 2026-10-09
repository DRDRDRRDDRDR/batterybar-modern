// batterybar-modern — 现代视效与材质渲染系统（Telegram Design System）
// 深度借鉴 Telegram Desktop (TDesktop) 视效架构与调色板：
// 1. 材质：优雅深色半透底座 (#17212B / #0E1621)，无刺眼死黑；
// 2. 槽位：Telegram 高阶深蓝灰卡片 (#242F3D / #1F2936)；
// 3. 强调色：Telegram 经典天空湛蓝 (#5288C1) 与活力生机翠绿 (#4FAE4E)；
// 4. 文本层级：纯白主字 (#F5F5F5) 与高舒适度蓝灰次级字 (#7F90A0)；
// 5. 几何：连续平滑曲线药丸卡片 (Smooth Floating Pill)，摒弃粗陋切线与硬突起。
#![allow(non_snake_case)]

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, LineTo, MoveToEx,
    RoundRect, SelectObject, SetBkMode, SetTextColor, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, DRAW_TEXT_FORMAT, FONT_CHARSET, FONT_CLIP_PRECISION,
    FONT_OUTPUT_PRECISION, FONT_QUALITY, HDC, HFONT, HGDIOBJ, TRANSPARENT, PS_SOLID,
    GetStockObject, HOLLOW_BRUSH, NULL_PEN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: COLORREF,          // 主窗口底色 #17212B
    pub surface: COLORREF,     // 浮动卡片/面板底槽 #242F3D
    pub surface_dim: COLORREF, // 内嵌单元槽 #1F2936
    pub track: COLORREF,       // 进度条轨道槽 #1B242F
    pub border: COLORREF,      // 极细微边线 #2D3A49
    pub accent_blue: COLORREF, // Telegram 经典湛蓝 #5288C1
    pub accent_green: COLORREF,// Telegram 生机翠绿 #4FAE4E
    pub accent_warn: COLORREF, // 柔和暖琥珀 #E58C38
    pub accent_crit: COLORREF, // 优雅珊瑚红 #E05347
    pub text: COLORREF,        // 纯净主字 #F5F5F5
    pub text_dim: COLORREF,    // 舒缓蓝灰次级字 #7F90A0
}

pub const fn palette_dark() -> Palette {
    Palette {
        bg: rgb(0x17, 0x21, 0x2B),
        surface: rgb(0x24, 0x2F, 0x3D),
        surface_dim: rgb(0x1F, 0x29, 0x36),
        track: rgb(0x1B, 0x24, 0x2F),
        border: rgb(0x2D, 0x3A, 0x49),
        accent_blue: rgb(0x52, 0x88, 0xC1),
        accent_green: rgb(0x4F, 0xAE, 0x4E),
        accent_warn: rgb(0xE5, 0x8C, 0x38),
        accent_crit: rgb(0xE0, 0x53, 0x47),
        text: rgb(0xF5, 0xF5, 0xF5),
        text_dim: rgb(0x7F, 0x90, 0xA0),
    }
}

pub const fn palette_light() -> Palette {
    Palette {
        bg: rgb(0xF0, 0xF2, 0xF5),
        surface: rgb(0xFF, 0xFF, 0xFF),
        surface_dim: rgb(0xEA, 0xEE, 0xF2),
        track: rgb(0xDE, 0xE4, 0xEA),
        border: rgb(0xCF, 0xD6, 0xDE),
        accent_blue: rgb(0x33, 0x90, 0xEC),
        accent_green: rgb(0x38, 0xA3, 0x44),
        accent_warn: rgb(0xEB, 0x8A, 0x26),
        accent_crit: rgb(0xDE, 0x47, 0x3A),
        text: rgb(0x11, 0x14, 0x18),
        text_dim: rgb(0x6E, 0x7B, 0x8B),
    }
}

pub const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    COLORREF(r | (g << 8) | (b << 16))
}

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
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        err2.is_err() || data == 0
    }
}

pub fn enable_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

// ---- 高品质绘图原语 ----

pub unsafe fn fill_solid(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rc, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

/// 连续平滑圆角块（选入 NULL_PEN，杜绝任何 GDI 黑色外轮廓）
pub unsafe fn fill_round(
    hdc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    color: COLORREF,
) {
    if right <= left || bottom <= top {
        return;
    }
    let brush = CreateSolidBrush(color);
    let old_brush = SelectObject(hdc, brush.into());
    let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
    let _ = RoundRect(hdc, left, top, right, bottom, radius * 2, radius * 2);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(brush.into());
}

/// 精细 1px 微描边圆角矩形
pub unsafe fn stroke_round(
    hdc: HDC,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    border_color: COLORREF,
) {
    if right <= left || bottom <= top {
        return;
    }
    let pen = CreatePen(PS_SOLID, 1, border_color);
    let old_pen = SelectObject(hdc, pen.into());
    let old_brush = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH));
    let _ = RoundRect(hdc, left, top, right, bottom, radius * 2, radius * 2);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

/// Telegram Desktop 优雅药丸电量条渲染器（Smooth Floating Pill）
pub unsafe fn draw_telegram_battery_pill(
    hdc: HDC,
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    pct: i32,
    charging: bool,
    low: bool,
    critical: bool,
    pal: &Palette,
) {
    let w = r - l;
    let h = b - t;
    if w <= 8 || h <= 6 {
        return;
    }

    let radius = h / 2; // 纯圆弧平滑药丸胶囊

    // 1. 底槽：Telegram 次级深色半透槽
    fill_round(hdc, l, t, r, b, radius, pal.track);

    // 2. 平滑流体进度填充（Fluid Progress）
    let valid_pct = if pct < 0 { 0 } else { pct.clamp(0, 100) };
    let inner_w = (w - 2) as i64;
    let fw = (inner_w * valid_pct as i64 / 100) as i32;

    if fw >= 4 {
        let fl = l + 1;
        let fr = fl + fw;
        
        let fill_color = if charging {
            pal.accent_blue  // 优雅湛蓝 (Telegram Blue)
        } else if critical {
            pal.accent_crit  // 珊瑚红 (Warning Red)
        } else if low {
            pal.accent_warn  // 琥珀金 (Warning Amber)
        } else {
            pal.accent_green // 生机翠绿 (Telegram Green)
        };

        fill_round(hdc, fl, t + 1, fr, b - 1, radius.min(fw / 2), fill_color);
    }

    // 3. Telegram 风格极其克制的 1px 微外边框
    stroke_round(hdc, l, t, r, b, radius, pal.border);
}

/// 创建高质量 UI 字体：Segoe UI Variable Text / Segoe UI
pub fn create_ui_font(height_px: i32, weight: i32) -> HFONT {
    unsafe {
        let face: windows_core::HSTRING = "Segoe UI Variable Text".into();
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

pub const DT_VCENTER_FLAG: DRAW_TEXT_FORMAT = DT_VCENTER;
pub const DT_WORDBREAK_FLAG: DRAW_TEXT_FORMAT = DT_WORDBREAK;
