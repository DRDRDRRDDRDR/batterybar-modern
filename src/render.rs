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

/// 颜色混合：t=0 返回 a，t=1 返回 b（按 RGB 线性插值）
pub fn mix(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let t = t.clamp(0.0, 1.0);
    let ar = (a.0 & 0xFF) as f32;
    let ag = ((a.0 >> 8) & 0xFF) as f32;
    let ab = ((a.0 >> 16) & 0xFF) as f32;
    let br = (b.0 & 0xFF) as f32;
    let bg = ((b.0 >> 8) & 0xFF) as f32;
    let bb = ((b.0 >> 16) & 0xFF) as f32;
    let r = (ar + (br - ar) * t) as u32;
    let g = (ag + (bg - ag) * t) as u32;
    let bl = (ab + (bb - ab) * t) as u32;
    COLORREF(r | (g << 8) | (bl << 16))
}

/// 提亮：朝白色混合
pub fn lighten(c: COLORREF, amount: f32) -> COLORREF {
    mix(c, COLORREF(0x00FF_FFFF), amount)
}

/// 压暗：朝黑色混合
pub fn darken(c: COLORREF, amount: f32) -> COLORREF {
    mix(c, COLORREF(0), amount)
}

/// 拟物化实体电池渲染器（Skeuomorphic Battery）
///
/// 物理层次自外向内：
///   1. 金属外壳（圆角矩形，顶部受光亮边 + 底部暗边 → 圆柱受光错觉）
///   2. 正极端子（右侧凸起小方块，独立受光/背光）
///   3. 内凹观察窗（深色槽 + 上沿内阴影 + 下沿内高光 → 凹陷错觉）
///   4. 电解液液面（双层高光：上层玻璃反光 / 下层液面本色 + 1px 液面镜面线）
///   5. 玻璃罩反光带（横跨上部，模拟弧面玻璃的柔和高光）
///   6. 外轮廓描边
pub unsafe fn draw_skeuo_battery(
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
    if w <= 14 || h <= 8 {
        return;
    }

    let radius = 3i32;

    // ---- 状态主色（Telegram 设计令牌体系内提亮，保证拟物通透感）----
    let base = if charging {
        pal.accent_blue
    } else if critical {
        pal.accent_crit
    } else if low {
        pal.accent_warn
    } else {
        pal.accent_green
    };

    // ---- 几何：外壳主体 + 右侧正极端子 ----
    let term_w = 4i32;
    let term_h = (h * 46) / 100;
    let term_t = t + (h - term_h) / 2;
    let term_b = term_t + term_h;
    let body_r = r - term_w - 1;
    let body_w = body_r - l;
    if body_w <= 6 {
        return;
    }

    // ---- 1. 金属外壳：竖向渐变（顶亮 → 底暗），圆柱金属质感 ----
    let shell_top = lighten(pal.surface, 0.38);
    let shell_bot = darken(pal.surface, 0.26);
    fill_round(hdc, l, t, body_r, b, radius, shell_bot);
    let shell_split = t + (h * 46) / 100;
    fill_round(hdc, l, t, body_r, shell_split, radius, shell_top);

    // ---- 2. 正极端子：整体亮于外壳的金属凸起（凸起感靠底部 1px 投影，不靠半边明暗）----
    let term_l = body_r + 1;
    fill_round(hdc, term_l, term_t, r, term_b, 1, lighten(pal.surface, 0.46));
    let term_shadow = &RECT {
        left: term_l,
        top: term_b - 1,
        right: r,
        bottom: term_b,
    };
    fill_solid(hdc, term_shadow, darken(pal.surface, 0.30));

    // ---- 3. 内凹观察窗（深色凹槽）----
    let inset = 2i32;
    let win_l = l + inset;
    let win_t = t + inset;
    let win_r = body_r - inset;
    let win_b = b - inset;
    let win_w = win_r - win_l;
    let win_h = win_b - win_t;
    if win_w <= 4 || win_h <= 3 {
        return;
    }
    let win_radius = 2i32;
    let well = darken(pal.surface_dim, 0.42);
    fill_round(hdc, win_l, win_t, win_r, win_b, win_radius, well);

    // ---- 4. 电解液液面（双层高光 + 1px 镜面线 + 镜面斑）----
    let valid_pct = if pct < 0 { 0 } else { pct.clamp(0, 100) };
    let fill_w = (win_w * valid_pct / 100) as i32;
    if fill_w >= 2 {
        let fl = win_l;
        let fr = win_l + fill_w;
        let fr_radius = win_radius.min(fill_w / 2);

        // 下层：液面本色
        fill_round(hdc, fl, win_t, fr, win_b, fr_radius, darken(base, 0.22));
        // 上层：玻璃反光层（提亮幅度受控，避免洗成近白）
        let fluid_split = win_t + (win_h * 42) / 100;
        fill_round(hdc, fl, win_t, fr, fluid_split, fr_radius, lighten(base, 0.14));

        // 1px 液面镜面线（分界高光细线 —— 拟物感的关键细节）
        if fr - fl >= 4 && fluid_split > win_t {
            let line = &RECT {
                left: fl + 1,
                top: fluid_split - 1,
                right: fr - 1,
                bottom: fluid_split,
            };
            fill_solid(hdc, line, lighten(base, 0.42));
        }

        // 镜面斑：液面上部左侧一小段柔光，宽度压到 22% 以内，避免读成划痕
        if fill_w >= 16 && fluid_split - win_t >= 3 {
            let streak_r = fl + 2 + (fill_w * 22) / 100;
            let streak = &RECT {
                left: fl + 2,
                top: win_t + 1,
                right: streak_r,
                bottom: win_t + 2,
            };
            fill_solid(hdc, streak, lighten(base, 0.38));
        }
    }

    // 上沿内阴影（凹陷错觉）：观察窗顶部 1px 深线
    let shadow = &RECT {
        left: win_l + 1,
        top: win_t,
        right: win_r - 1,
        bottom: win_t + 1,
    };
    fill_solid(hdc, shadow, darken(well, 0.62));

    // 下沿内高光（凹陷底部反光）：观察窗底部 1px 亮线
    let inner_light = &RECT {
        left: win_l + 1,
        top: win_b - 1,
        right: win_r - 1,
        bottom: win_b,
    };
    fill_solid(hdc, inner_light, lighten(pal.surface, 0.16));

    // ---- 5. 外框：顶部受光亮边收窄变柔 + 底部接地暗边 + 1px 深轮廓 ----
    let top_edge = &RECT {
        left: l + 3,
        top: t,
        right: body_r - 3,
        bottom: t + 1,
    };
    fill_solid(hdc, top_edge, lighten(pal.surface, 0.34));

    let bottom_edge = &RECT {
        left: l + 2,
        top: b - 1,
        right: body_r - 2,
        bottom: b,
    };
    fill_solid(hdc, bottom_edge, darken(pal.surface, 0.62));

    stroke_round(hdc, l, t, body_r, b, radius, darken(pal.surface, 0.66));
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
