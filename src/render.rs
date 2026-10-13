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
    CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, FillRect, RoundRect, SelectObject,
    SetBkMode, ANTIALIASED_QUALITY, FONT_CHARSET, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION,
    FONT_QUALITY, HDC, HFONT, HGDIOBJ, TRANSPARENT, PS_SOLID, GetStockObject, HOLLOW_BRUSH,
    NULL_PEN,
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

/// 半透明铺底：用 AlphaBlend 把纯色按 alpha 混到**已有目标像素**上。
/// 用于「电池条背景不透明度」——alpha<100 时让 Windows 11 任务栏自身的
/// Acrylic 亚克力层透上来。返回 false 表示「调用方应回落到 fill_solid」。
///
/// 关键点：子窗口（非 layered）BeginPaint 得到的 HDC 是被裁剪到本窗口区域的
/// 显示 DC，AlphaBlend 读到的目标像素就是任务栏已画好的内容，所以混合有效。
pub unsafe fn fill_solid_alpha(hdc: HDC, rc: &RECT, color: COLORREF, alpha_pct: i32) -> bool {
    use windows::Win32::Graphics::Gdi::{
        AlphaBlend, CreateCompatibleDC, CreateDIBSection, DeleteDC, AC_SRC_OVER, BI_RGB,
        BITMAPINFO, BLENDFUNCTION, DIB_RGB_COLORS,
    };

    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    if w <= 0 || h <= 0 {
        return false;
    }
    let pct = alpha_pct.clamp(0, 100);
    if pct >= 100 {
        return false; // 全不透明 → 走 fill_solid
    }
    let alpha = ((pct * 255) / 100) as u8;
    if alpha == 0 {
        return true; // 全透明 → 什么都不画，任务栏自己透出来
    }

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // 负高度 = top-down DIB
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let hbm = match CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(h) => h,
        Err(_) => return false,
    };
    let mem = CreateCompatibleDC(Some(hdc));
    if bits.is_null() || mem.is_invalid() {
        let _ = DeleteObject(hbm.into());
        return false;
    }
    let old = SelectObject(mem, hbm.into());

    // COLORREF = 0x00BBGGRR；DIB 像素布局 = 0xAARRGGBB
    let r = (color.0 & 0xFF) as u8;
    let g = ((color.0 >> 8) & 0xFF) as u8;
    let b = ((color.0 >> 16) & 0xFF) as u8;
    let px = bits as *mut u8;
    for i in 0..(w as usize * h as usize) {
        *px.add(i * 4) = b;
        *px.add(i * 4 + 1) = g;
        *px.add(i * 4 + 2) = r;
        *px.add(i * 4 + 3) = 255;
    }

    let bf = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: 0,
    };
    let ok = AlphaBlend(hdc, rc.left, rc.top, rc.right, rc.bottom, mem, 0, 0, w, h, bf).as_bool();

    SelectObject(mem, old);
    let _ = DeleteDC(mem);
    let _ = DeleteObject(hbm.into());
    ok
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

/// 拟物化电池图标（Skeuomorphic Battery Icon）
///
/// 与「进度条」的本质区别：电池有**固定壳体轮廓**和**正的端子**，
/// 填充只是壳内的一部分，而不是整条被拉长。
///
/// 拟物感来自 5 件套（缺一件就塌成色块）：
///   1. 连续剪影：先铺深色外形（壳体 ∪ 端子，两者重叠 1px），再整体内缩 1px 填金属
///      —— 端子与壳体**共用一条外轮廓**，结构上不可能出现缝隙（原版就是被 1px 缝割裂的元凶）
///   2. 金属渐变：6 段竖向色标（顶部受光 → 底部接地暗），模拟圆柱面漫反射
///   3. 四向边缘光：上亮 / 左亮 / 下暗 / 右暗，让平面产生厚度
///   4. 观察窗凹陷：井底近黑 + 上沿内阴影 + 下沿内反光
///   5. 液面物理：底部深 → 液面浅 + 1px 弯月高光 + 左上柔光斑 + 底部反光暗带
pub unsafe fn draw_battery_icon(
    hdc: HDC,
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    pct: i32,
    charging: bool,
    low: bool,
    critical: bool,
    style: u32,
    pal: &Palette,
) {
    let w = r - l;
    let h = b - t;
    if w < 10 || h < 6 {
        return;
    }

    // ---- 状态主色 ----
    let base = if charging {
        pal.accent_blue
    } else if critical {
        pal.accent_crit
    } else if low {
        pal.accent_warn
    } else {
        pal.accent_green
    };

    // ---- 视觉主题风格（Preferences → DISPLAY → 视觉主题风格）----
    // 1 = 现代极简 Fluent：无金属壳体、无端子，扁平轨道 + 圆角填充条
    if style == 1 {
        draw_fluent_battery(hdc, l, t, r, b, pct, base, pal);
        return;
    }

    // 0 = 经典拟物 Aero Glass：以下走金属壳体 + 正极端子 + 内凹观察窗

    // 极小尺寸兜底：没有空间做拟物层次，只画一个带轮廓的色块
    if h < 12 || w < 20 {
        let radius = ((h as f32 * 0.30).round() as i32).max(1);
        stroke_round(hdc, l, t, r, b, radius, rgb(14, 17, 22));
        let pad = 2i32;
        let fw = ((w - pad * 2) * pct.clamp(0, 100) / 100).max(1);
        if fw > 1 {
            fill_round(hdc, l + pad, t + pad, l + pad + fw, b - pad, radius - 1, base);
        }
        return;
    }

    // ---- 几何：壳体 + 正极端子 ----
    let radius = ((h as f32 * 0.20).round() as i32).max(2);
    let term_w = ((w as f32 * 0.115).round() as i32).max(2);
    let term_h = ((h as f32 * 0.44).round() as i32).max(5);
    let term_t = t + (h - term_h) / 2;
    let term_b = term_t + term_h;
    let body_r = r - term_w; // 壳体右缘与端子左缘重合，零缝隙
    if body_r - l <= 8 {
        return;
    }

    // ---- 1. 连续深色剪影（壳体 ∪ 端子，端子左压 1px 压进壳体）----
    let rim = rgb(13, 16, 21);
    fill_round(hdc, body_r - 1, term_t, r, term_b, 1, rim);
    fill_round(hdc, l, t, body_r, b, radius, rim);

    // 金属内区（整体内缩 1px → 四周恒为 1px 深轮廓）
    let il = l + 1;
    let it = t + 1;
    let ir = body_r - 1;
    let ib = b - 1;
    let ih = ib - it;
    let rad = radius - 1;

    // ---- 2. 壳体金属：6 段竖向渐变（自底向上叠加，首尾两段提供圆角）----
    let stops = [
        rgb(190, 197, 209), // 0 顶面强受光
        rgb(152, 160, 173),
        rgb(114, 122, 136),
        rgb(80, 87, 100),
        rgb(51, 57, 68),
        rgb(28, 32, 40), // 5 底面接地暗
    ];
    fill_round(hdc, il, it, ir, ib, rad, stops[5]);
    for i in (0..5usize).rev() {
        let y0 = it + (ih * i as i32) / 6;
        let y1 = it + (ih * (i as i32 + 1)) / 6;
        if i == 0 {
            // 顶段用圆角矩形，向下多压 rad 保证顶角是圆的
            fill_round(hdc, il, y0, ir, (y1 + rad).min(ib), rad, stops[0]);
        } else {
            fill_solid(
                hdc,
                &RECT { left: il, top: y0, right: ir, bottom: y1.min(ib) },
                stops[i],
            );
        }
    }

    // ---- 3. 四向边缘光：制造厚度 ----
    fill_solid(
        hdc,
        &RECT { left: il + rad, top: it, right: ir - rad, bottom: it + 1 },
        rgb(232, 238, 247), // 顶：镜面高光
    );
    fill_solid(
        hdc,
        &RECT { left: il, top: it + rad, right: il + 1, bottom: ib - rad },
        rgb(203, 210, 221), // 左：受光面
    );
    fill_solid(
        hdc,
        &RECT { left: il + rad, top: ib - 1, right: ir - rad, bottom: ib },
        rgb(16, 19, 25), // 下：接地暗
    );
    fill_solid(
        hdc,
        &RECT { left: ir - 1, top: it + rad, right: ir, bottom: ib - rad },
        rgb(30, 34, 42), // 右：背光面
    );

    // ---- 4. 正极端子：同族金属小柱，左缘直接压在壳体上（无缝）----
    let tl = body_r - 1;
    let tt = term_t + 1;
    let tr = r - 1;
    let tb = term_b - 1;
    let th = tb - tt;
    fill_round(hdc, tl, tt, tr, tb, 1, rgb(62, 69, 80));
    fill_solid(hdc, &RECT { left: tl, top: tt + (th * 2) / 3, right: tr, bottom: tb }, rgb(116, 124, 137));
    fill_solid(hdc, &RECT { left: tl, top: tt, right: tr, bottom: tt + th / 3 }, rgb(170, 178, 191));
    fill_solid(hdc, &RECT { left: tl + 1, top: tt, right: tr, bottom: tt + 1 }, rgb(216, 223, 234));
    fill_solid(hdc, &RECT { left: tl, top: tb - 1, right: tr, bottom: tb }, rgb(20, 24, 30));

    // ---- 5. 观察窗（凹陷）----
    let inset_x = ((h as f32 * 0.17).round() as i32).max(2);
    let inset_y = ((h as f32 * 0.20).round() as i32).max(2);
    let win_l = il + inset_x;
    let win_t = it + inset_y;
    let win_r = ir - inset_x;
    let win_b = ib - inset_y;
    let win_w = win_r - win_l;
    let win_h = win_b - win_t;
    if win_w < 4 || win_h < 3 {
        return;
    }
    let wr = ((win_h as f32 * 0.18).round() as i32).max(1);
    fill_round(hdc, win_l, win_t, win_r, win_b, wr, rgb(19, 22, 27));
    fill_solid(hdc, &RECT { left: win_l + 1, top: win_t, right: win_r - 1, bottom: win_t + 1 }, rgb(6, 7, 10));
    fill_solid(hdc, &RECT { left: win_l, top: win_t + 1, right: win_l + 1, bottom: win_b - 1 }, rgb(10, 12, 15));
    fill_solid(hdc, &RECT { left: win_l + 1, top: win_b - 1, right: win_r - 1, bottom: win_b }, rgb(99, 106, 118));
    fill_solid(hdc, &RECT { left: win_r - 1, top: win_t + 1, right: win_r, bottom: win_b - 1 }, rgb(74, 81, 93));

    // ---- 6. 电解液 ----
    let liq_h = (win_h * pct.clamp(0, 100) / 100) as i32;
    if liq_h >= 2 {
        let fl = win_l + 1;
        let fr = win_r - 1;
        let fb = win_b - 1;
        let ft = fb - liq_h;
        let fr_rad = wr.min(liq_h / 2);

        // 液柱本体：底部深（液体厚）→ 上部本色
        fill_round(hdc, fl, ft, fr, fb, fr_rad, darken(base, 0.12));
        let split = ft + (liq_h * 38) / 100;
        if split > ft {
            fill_round(hdc, fl, ft, fr, split + fr_rad, fr_rad, base);
        }

        // 1px 弯月高光：液面与空气的界面反射（拟物的灵魂细节）
        if fr - fl >= 3 {
            fill_solid(
                hdc,
                &RECT { left: fl + 1, top: ft, right: fr - 1, bottom: ft + 1 },
                lighten(base, 0.50),
            );
        }
        // 左上柔光斑（宽度 30% 以内，避免读成白色划痕）
        if fr - fl >= 8 && split - ft >= 2 {
            fill_solid(
                hdc,
                &RECT { left: fl + 1, top: ft + 1, right: (fl + 1 + ((fr - fl) * 30) / 100).min(fr - 1), bottom: ft + 2 },
                lighten(base, 0.34),
            );
        }
        // 液柱底部反光暗带（贴壁的液体更暗）
        if liq_h >= 4 {
            fill_solid(
                hdc,
                &RECT { left: fl, top: fb - 1, right: fr, bottom: fb },
                darken(base, 0.34),
            );
        }
    }
}

/// 现代极简 Fluent 风格电池（Preferences → 视觉主题风格 = 现代极简 Fluent）。
/// 与拟物版的区别：没有金属壳体渐变、没有正极端子、没有内凹观察窗井，
/// 只保留「扁平圆角轨道 + 圆角填充条 + 1px 细边」的 Win11 Fluent 语言。
/// 这样在小尺寸下更干净，也和 Windows 11 设置页里的电池指示器一致。
unsafe fn draw_fluent_battery(
    hdc: HDC,
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    pct: i32,
    base: COLORREF,
    pal: &Palette,
) {
    let w = r - l;
    let h = b - t;
    let radius = ((h as f32 * 0.28).round() as i32).max(2);
    // 端子：Fluent 版做成贴边的细圆角块，不做金属高光
    let term_w = ((w as f32 * 0.075).round() as i32).max(2);
    let term_h = ((h as f32 * 0.40).round() as i32).max(4);
    let term_t = t + (h - term_h) / 2;
    let body_r = r - term_w - 1;

    // 轨道（未填充部分）：比边框更暗的中性槽
    fill_round(hdc, l, t, body_r, b, radius, pal.track);
    stroke_round(hdc, l, t, body_r, b, radius, pal.border);

    // 端子
    fill_round(hdc, body_r + 1, term_t, r, term_t + term_h, (term_w / 2).max(1), pal.text_dim);

    // 填充条：内缩 2px，按 pct 从左侧铺开
    let pad = 2i32;
    let inner_l = l + pad;
    let inner_r = body_r - pad;
    let inner_w = inner_r - inner_l;
    if inner_w > 2 && pct > 0 {
        let fw = (inner_w * pct.clamp(0, 100) / 100).max(1);
        let fr = (inner_l + fw).min(inner_r);
        let ir = (radius - pad).max(1);
        // 底部 1px 暗带制造轻微体积，但不做多段渐变
        fill_round(hdc, inner_l, t + pad, fr, b - pad, ir, base);
        if b - pad - (t + pad) > 5 {
            fill_solid(
                hdc,
                &RECT { left: inner_l, top: b - pad - 1, right: fr, bottom: b - pad },
                darken(base, 0.22),
            );
        }
    }
}

/// 充电时在电池上方叠加的闪电符号（Polygon 矢量绘制，非字体字形）
/// 7x7 像素闪电掩码（`#` = 点亮）。
/// 为什么不用多边形：电池观察窗在竖排任务栏里只有 ~13x16 px，此时 6 点归一化闪电
/// 的水平边（0.42→0.30、0.92→0.56）不足 1px，GDI 填充规则把它们并成一条竖杠，
/// 渲染结果是一个「⊢」而不是闪电。像素掩码在任意尺寸都能保持可辨识。
const BOLT_MASK: [&str; 7] = [
    "..##..",
    "..#...",
    ".##...",
    "####..",
    ".#....",
    "#.....",
    "##....",
];

/// 拟物化充电闪电：以最近邻缩放掩码填像素，保证小尺寸下轮廓不糊。
/// h/w 中取较小者作为等边边长并居中。
/// 先在 (+1,+1) 偏移处铺一层深色，再叠主色 —— 与本模块文字浮雕底衬同一套做法，
/// 闪电压在浅蓝液面上时靠这层暗边才立得住，否则白闪电直接消失在液面里。
pub unsafe fn draw_charge_bolt(hdc: HDC, cx: i32, cy: i32, h: i32, w: i32, color: COLORREF) {
    let side = h.min(w);
    if side < 5 {
        return;
    }
    let mh = BOLT_MASK.len() as i32;
    let mw = BOLT_MASK[0].len() as i32;
    let x0 = cx - side / 2;
    let y0 = cy - side / 2;

    let old_p = SelectObject(hdc, GetStockObject(NULL_PEN));
    // pass 0 = 暗色投影（偏移 +1,+1），pass 1 = 主色
    for pass in 0..2i32 {
        let c = if pass == 0 { darken(color, 0.74) } else { color };
        let off = if pass == 0 { 1 } else { 0 };
        let brush = CreateSolidBrush(c);
        let old_b = SelectObject(hdc, brush.into());
        for dy in 0..side {
            let sy = (dy * mh) / side;
            for dx in 0..side {
                let sx = (dx * mw) / side;
                if BOLT_MASK[sy as usize].as_bytes()[sx as usize] == b'#' {
                    fill_solid(
                        hdc,
                        &RECT {
                            left: x0 + dx + off,
                            top: y0 + dy + off,
                            right: x0 + dx + 1 + off,
                            bottom: y0 + dy + 1 + off,
                        },
                        c,
                    );
                }
            }
        }
        SelectObject(hdc, old_b);
        let _ = DeleteObject(brush.into());
    }
    SelectObject(hdc, old_p);
}

/// 创建高质量 UI 字体：Segoe UI Variable Text / Segoe UI（ClearType 亚像素）
pub fn create_ui_font(height_px: i32, weight: i32) -> HFONT {
    create_ui_font_q(height_px, weight, CLEARTYPE_QUALITY())
}

/// 同上，但用灰度抗锯齿。任务栏条上的文字落在深色底上，ClearType 的 RGB 亚像素
/// 边会在 10~12px 字号下显出彩边（实测截图里 "On A/C" 呈橙蓝重影），
/// 灰度 AA 在深底上干净得多。
pub fn create_ui_font_gray(height_px: i32, weight: i32) -> HFONT {
    create_ui_font_q(height_px, weight, ANTIALIASED_QUALITY)
}

fn create_ui_font_q(height_px: i32, weight: i32, quality: FONT_QUALITY) -> HFONT {
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
            quality,
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
                quality,
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

pub unsafe fn set_bk_transparent(hdc: HDC) {
    SetBkMode(hdc, TRANSPARENT);
}
