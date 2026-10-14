//! 原版 BatteryBar 的 theme.xml 主题加载与绘制。
//!
//! 一个主题等于一个 theme.xml 加若干 png。xml 里定义 5 个状态：
//! Default / Discharging / LowBattery / CriticalBattery / Charging。
//! 每个状态给出背景图、进度条图、悬停层、按下层与文字样式。
//!
//! 图像既可以是单张 Center 整体拉伸，也可以是九宫格拉伸，
//! 也就是四角加四边加中心共 9 张图。两种形式都支持。
//!
//! 尺寸规则：Margin 与 Padding 的数值都相对 MaxHeight。实际像素等于
//! 数值乘以 实际高度除以 MaxHeight。xml 未声明 MaxHeight 时按 100 处理。
//!
//! 主题放在 exe 同目录的 themes 子文件夹下，一个主题一个子目录。

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, CreateCompatibleDC, DeleteDC, SelectObject, SetStretchBltMode, AC_SRC_ALPHA,
    AC_SRC_OVER, BLENDFUNCTION, HALFTONE, HBITMAP, HDC,
};
use windows::Win32::Graphics::GdiPlus::{
    GdipCreateBitmapFromFile, GdipCreateHBITMAPFromBitmap, GdipDisposeImage, GdipGetImageHeight,
    GdipGetImageWidth, GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GdiplusStartupOutput,
    GpBitmap, GpImage,
};

// ---------------------------------------------------------------- GDI+ 生命周期

static GDIP_TOKEN: Mutex<Option<usize>> = Mutex::new(None);

/// 启动 GDI+。整个进程只需要一次，重复调用直接返回。
fn ensure_gdiplus() -> bool {
    let mut guard = match GDIP_TOKEN.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if guard.is_some() {
        return true;
    }
    let mut token: usize = 0;
    let mut input = GdiplusStartupInput::default();
    input.GdiplusVersion = 1;
    let mut output = GdiplusStartupOutput::default();
    let status = unsafe { GdiplusStartup(&mut token, &input, &mut output) };
    if status.0 != 0 {
        return false;
    }
    *guard = Some(token);
    true
}

/// 让 GDI+ 在退出时正常收尾。
pub fn shutdown() {
    if let Ok(mut g) = GDIP_TOKEN.lock() {
        if let Some(token) = g.take() {
            unsafe { GdiplusShutdown(token) };
        }
    }
}

// ---------------------------------------------------------------- 位图

/// 已载入的位图。HBITMAP 是裸指针，需要手动声明 Send 与 Sync 才能放进全局。
struct Img {
    hbmp: HBITMAP,
    w: i32,
    h: i32,
}

unsafe impl Send for Img {}
unsafe impl Sync for Img {}

impl Img {
    /// 从 png 载入。失败返回 None，调用方跳过该层即可。
    fn load(path: &Path) -> Option<Img> {
        if !path.is_file() || !ensure_gdiplus() {
            return None;
        }
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let mut bmp: *mut GpBitmap = std::ptr::null_mut();
            if GdipCreateBitmapFromFile(PCWSTR(wide.as_ptr()), &mut bmp).0 != 0 || bmp.is_null() {
                return None;
            }
            let mut w: u32 = 0;
            let mut h: u32 = 0;
            let _ = GdipGetImageWidth(bmp as *mut GpImage, &mut w);
            let _ = GdipGetImageHeight(bmp as *mut GpImage, &mut h);
            // background 传 0 表示保留 alpha，得到预乘 alpha 的 32bpp 位图
            let mut hbmp = HBITMAP::default();
            let status = GdipCreateHBITMAPFromBitmap(bmp, &mut hbmp, 0);
            let _ = GdipDisposeImage(bmp as *mut GpImage);
            if status.0 != 0 || hbmp.is_invalid() {
                return None;
            }
            Some(Img { hbmp, w: w as i32, h: h as i32 })
        }
    }
}

// ---------------------------------------------------------------- 数据模型

/// 一层图。四角与四边齐全时走九宫格，否则只拉伸 Center。
#[derive(Default)]
struct Layer {
    center: Option<Img>,
    corner_tl: Option<Img>,
    corner_tr: Option<Img>,
    corner_bl: Option<Img>,
    corner_br: Option<Img>,
    left: Option<Img>,
    right: Option<Img>,
    top: Option<Img>,
    bottom: Option<Img>,
    margin: [i32; 4],
}

impl Layer {
    fn is_empty(&self) -> bool {
        self.center.is_none()
            && self.corner_tl.is_none()
            && self.corner_tr.is_none()
            && self.corner_bl.is_none()
            && self.corner_br.is_none()
    }

    fn is_nine_slice(&self) -> bool {
        self.corner_tl.is_some()
            && self.corner_tr.is_some()
            && self.corner_bl.is_some()
            && self.corner_br.is_some()
            && self.left.is_some()
            && self.right.is_some()
            && self.top.is_some()
            && self.bottom.is_some()
    }
}

struct State {
    font_face: String,
    font_pt: i32,
    font_bold: bool,
    /// None 表示 xml 的 TextColor alpha 为 0，即明确要求不显示文字
    text_color: Option<COLORREF>,
    text_shadow: bool,
    padding: [i32; 4],
    background: Layer,
    progress: Layer,
    hover: Layer,
    pressed: Layer,
}

impl Default for State {
    fn default() -> Self {
        State {
            font_face: String::from("Segoe UI"),
            font_pt: 10,
            font_bold: false,
            text_color: Some(COLORREF(0x00FF_FFFF)),
            text_shadow: false,
            padding: [0; 4],
            background: Layer::default(),
            progress: Layer::default(),
            hover: Layer::default(),
            pressed: Layer::default(),
        }
    }
}

struct Theme {
    name: String,
    max_height: i32,
    states: [Option<State>; 5],
}

// ---------------------------------------------------------------- 极简扫描器

/// 取出 `<tag>...</tag>` 之间的正文。标签带属性时也认。找不到返回 None。
fn block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let open_attr = format!("<{tag} ");
    let start = if let Some(i) = xml.find(&open) {
        i + open.len()
    } else {
        let i = xml.find(&open_attr)?;
        i + xml[i..].find('>')? + 1
    };
    let close = format!("</{tag}>");
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// 取出标签正文并去掉首尾空白。
fn text(xml: &str, tag: &str) -> Option<String> {
    block(xml, tag).map(|s| s.trim().to_string())
}

/// 解析逗号分隔的整数列表。
fn numbers(s: &str) -> Vec<i32> {
    s.split(',')
        .filter_map(|p| p.trim().parse::<i32>().ok())
        .collect()
}

/// 解析边距，顺序为 左,上,右,下。单个数字表示四边相同。
fn margin_of(s: &str) -> [i32; 4] {
    let v = numbers(s);
    match v.len() {
        0 => [0; 4],
        1 => [v[0]; 4],
        2 => [v[0], v[1], v[0], v[1]],
        _ => [v[0], v[1], v[2], v[3]],
    }
}

/// 解析颜色。三个数是 RGB，四个数是 RGBA，alpha 为 0 表示不要文字。
fn color_of(s: &str) -> Option<COLORREF> {
    let v = numbers(s);
    let rgb = |r: i32, g: i32, b: i32| {
        COLORREF(((b as u32) << 16) | ((g as u32) << 8) | (r as u32))
    };
    match v.len() {
        3 => Some(rgb(v[0], v[1], v[2])),
        4 => {
            if v[3] == 0 {
                None
            } else {
                Some(rgb(v[0], v[1], v[2]))
            }
        }
        _ => None,
    }
}

/// 把 xml 里的相对路径解析成绝对路径。xml 用反斜杠分隔，正斜杠也接受。
fn resolve(dir: &Path, rel: &str) -> PathBuf {
    dir.join(rel.trim().replace('\\', "/"))
}

/// 解析一层图。
fn parse_layer(xml: &str, dir: &Path) -> Layer {
    let mut l = Layer::default();
    if let Some(m) = text(xml, "Margin") {
        l.margin = margin_of(&m);
    }
    let take = |tag: &str| text(xml, tag).and_then(|p| Img::load(&resolve(dir, &p)));
    l.center = take("Center");
    l.corner_tl = take("CornerTopLeft");
    l.corner_tr = take("CornerTopRight");
    l.corner_bl = take("CornerBottomLeft");
    l.corner_br = take("CornerBottomRight");
    l.left = take("LeftBorder");
    l.right = take("RightBorder");
    l.top = take("TopBorder");
    l.bottom = take("BottomBorder");
    l
}

/// 解析一个状态。缺项从 Default 继承。
fn parse_state(xml: &str, dir: &Path, fallback: Option<&State>) -> State {
    let mut st = State::default();
    if let Some(f) = fallback {
        st.font_face = f.font_face.clone();
        st.font_pt = f.font_pt;
        st.font_bold = f.font_bold;
        st.text_color = f.text_color;
        st.text_shadow = f.text_shadow;
        st.padding = f.padding;
    }
    if let Some(font) = block(xml, "Font") {
        if let Some(n) = text(font, "Name") {
            st.font_face = n;
        }
        if let Some(sz) = text(font, "Size").and_then(|s| s.parse::<f32>().ok()) {
            st.font_pt = sz.round().max(1.0) as i32;
        }
        if let Some(style) = text(font, "Style") {
            st.font_bold = style.to_ascii_lowercase().contains("bold");
        }
    }
    if let Some(c) = text(xml, "TextColor") {
        st.text_color = color_of(&c);
    }
    if let Some(sh) = text(xml, "TextShadow") {
        st.text_shadow = sh.eq_ignore_ascii_case("true");
    }
    if let Some(p) = text(xml, "Padding") {
        st.padding = margin_of(&p);
    }
    if let Some(b) = block(xml, "Background") {
        st.background = parse_layer(b, dir);
    }
    if let Some(b) = block(xml, "ProgressBar") {
        st.progress = parse_layer(b, dir);
    }
    if let Some(b) = block(xml, "HoverOverlay") {
        st.hover = parse_layer(b, dir);
    }
    if let Some(b) = block(xml, "PressedOverlay") {
        st.pressed = parse_layer(b, dir);
    }
    st
}

// 状态下标：0 Default 1 Discharging 2 LowBattery 3 CriticalBattery 4 Charging
const STATE_TAGS: [&str; 5] = [
    "Default",
    "Discharging",
    "LowBattery",
    "CriticalBattery",
    "Charging",
];

/// 从 theme.xml 载入一个主题。文件不存在或没有 States 段返回 None。
fn load_theme(xml_path: &Path) -> Option<Theme> {
    let raw = std::fs::read_to_string(xml_path).ok()?;
    let dir = xml_path.parent()?.to_path_buf();
    let states_xml = block(&raw, "States")?;

    let name = text(&raw, "Name").unwrap_or_else(|| {
        xml_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| String::from("Unnamed"))
    });
    let max_height = text(&raw, "MaxHeight")
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(100);

    let default_state = block(states_xml, "Default").map(|d| parse_state(d, &dir, None));
    let mut states: [Option<State>; 5] = Default::default();
    states[0] = default_state;
    for i in 1..5 {
        if let Some(body) = block(states_xml, STATE_TAGS[i]) {
            states[i] = Some(parse_state(body, &dir, states[0].as_ref()));
        }
    }
    if states.iter().all(|s| s.is_none()) {
        return None;
    }
    Some(Theme { name, max_height, states })
}

// ---------------------------------------------------------------- 主题仓库

static THEMES: Mutex<Option<Vec<Theme>>> = Mutex::new(None);
static EMPTY: Vec<Theme> = Vec::new();

/// exe 同目录下的 themes 文件夹。
pub fn themes_dir() -> PathBuf {
    crate::config::exe_dir().join("themes")
}

/// 扫描 themes 文件夹。每个子目录优先取 theme.xml，否则取第一个能解析的 xml。
fn scan() -> Vec<Theme> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(themes_dir()) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let preferred = d.join("theme.xml");
        if preferred.is_file() {
            if let Some(t) = load_theme(&preferred) {
                out.push(t);
                continue;
            }
        }
        let mut xmls: Vec<PathBuf> = std::fs::read_dir(&d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map(|x| x.to_string_lossy().eq_ignore_ascii_case("xml"))
                    .unwrap_or(false)
            })
            .collect();
        xmls.sort();
        for x in xmls {
            if let Some(t) = load_theme(&x) {
                out.push(t);
                break;
            }
        }
    }
    out
}

/// 在主题列表上取值，首次访问时扫描并缓存。
fn with_themes<R>(f: impl FnOnce(&Vec<Theme>) -> R) -> R {
    let mut g = match THEMES.lock() {
        Ok(g) => g,
        Err(_) => return f(&EMPTY),
    };
    if g.is_none() {
        *g = Some(scan());
    }
    f(g.as_ref().unwrap_or(&EMPTY))
}

/// 强制重新扫描，新增或删除主题后调用。
pub fn reload() {
    if let Ok(mut g) = THEMES.lock() {
        *g = Some(scan());
    }
}

/// 已载入的主题数量。
pub fn count() -> usize {
    with_themes(|t| t.len())
}

/// 第 i 个主题的名字。
pub fn name_of(i: usize) -> Option<String> {
    with_themes(|t| t.get(i).map(|x| x.name.clone()))
}

/// 供设置面板显示的一句话状态。
pub fn status_line() -> String {
    let n = count();
    if n == 0 {
        format!("未找到主题，把主题文件夹放进 {}", themes_dir().display())
    } else {
        format!("已载入 {} 个外部主题", n)
    }
}

/// 打开 themes 文件夹，不存在时先建出来。
pub fn open_folder() {
    let dir = themes_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = crate::hidden_command("explorer.exe")
        .arg(dir.as_os_str())
        .spawn();
}

// ---------------------------------------------------------------- 绘制

/// 把一个位图拉伸画到目标矩形，按 alpha 混合。
unsafe fn blit(hdc: HDC, img: &Img, l: i32, t: i32, r: i32, b: i32) {
    let (w, h) = (r - l, b - t);
    if w <= 0 || h <= 0 || img.w <= 0 || img.h <= 0 {
        return;
    }
    let mem = CreateCompatibleDC(Some(hdc));
    if mem.is_invalid() {
        return;
    }
    let old = SelectObject(mem, img.hbmp.into());
    SetStretchBltMode(hdc, HALFTONE);
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = AlphaBlend(hdc, l, t, w, h, mem, 0, 0, img.w, img.h, blend);
    SelectObject(mem, old);
    let _ = DeleteDC(mem);
}

/// 按 factor 缩放后的边距。
fn scaled(m: [i32; 4], factor: f32) -> [i32; 4] {
    [
        ((m[0] as f32) * factor).round() as i32,
        ((m[1] as f32) * factor).round() as i32,
        ((m[2] as f32) * factor).round() as i32,
        ((m[3] as f32) * factor).round() as i32,
    ]
}

/// 画一层图。九宫格时四角固定尺寸，四边与中心拉伸，否则整图拉伸。
unsafe fn draw_layer(hdc: HDC, layer: &Layer, rc: RECT, factor: f32) {
    if layer.is_empty() {
        return;
    }
    let m = scaled(layer.margin, factor);
    let (l, t, r, b) = (
        rc.left + m[0],
        rc.top + m[1],
        rc.right - m[2],
        rc.bottom - m[3],
    );
    if r <= l || b <= t {
        return;
    }

    if !layer.is_nine_slice() {
        if let Some(i) = &layer.center {
            blit(hdc, i, l, t, r, b);
        }
        return;
    }

    let corner_w = layer
        .corner_tl
        .as_ref()
        .map(|i| ((i.w as f32) * factor).round() as i32)
        .unwrap_or(4)
        .clamp(1, ((r - l) / 2).max(1));
    let corner_h = layer
        .corner_tl
        .as_ref()
        .map(|i| ((i.h as f32) * factor).round() as i32)
        .unwrap_or(4)
        .clamp(1, ((b - t) / 2).max(1));
    let (mid_l, mid_r) = (l + corner_w, r - corner_w);
    let (mid_t, mid_b) = (t + corner_h, b - corner_h);

    let parts: [(&Option<Img>, i32, i32, i32, i32); 9] = [
        (&layer.corner_tl, l, t, mid_l, mid_t),
        (&layer.corner_tr, mid_r, t, r, mid_t),
        (&layer.corner_bl, l, mid_b, mid_l, b),
        (&layer.corner_br, mid_r, mid_b, r, b),
        (&layer.top, mid_l, t, mid_r, mid_t),
        (&layer.bottom, mid_l, mid_b, mid_r, b),
        (&layer.left, l, mid_t, mid_l, mid_b),
        (&layer.right, mid_r, mid_t, r, mid_b),
        (&layer.center, mid_l, mid_t, mid_r, mid_b),
    ];
    for (img, x0, y0, x1, y1) in parts {
        if let Some(i) = img {
            blit(hdc, i, x0, y0, x1, y1);
        }
    }
}

/// 选出当前应当使用的状态下标。
fn pick_state(charging: bool, low: bool, critical: bool) -> usize {
    if charging {
        4
    } else if critical {
        3
    } else if low {
        2
    } else {
        1
    }
}

/// 任务栏条与主题相关的状态量。
pub struct RenderState {
    pub pct: i32,
    pub charging: bool,
    pub low: bool,
    pub critical: bool,
    pub hovered: bool,
    pub pressed: bool,
}

/// 用外部主题画一次任务栏条。
///
/// index 是主题序号，从 0 开始。主题不存在或该主题完全没有可用图像时返回 false，
/// 调用方应当回退到内置画法。
pub unsafe fn draw_with_theme(hdc: HDC, rc: RECT, st: &RenderState, index: usize) -> bool {
    let vert = (rc.bottom - rc.top) > (rc.right - rc.left);
    with_themes(|themes| {
        let theme = match themes.get(index) {
            Some(t) => t,
            None => return false,
        };
        let mut idx = pick_state(st.charging, st.low, st.critical);
        if theme.states[idx].is_none() {
            idx = 0;
        }
        let Some(state) = theme.states[idx].as_ref().or(theme.states[0].as_ref()) else {
            return false;
        };
        if state.background.is_empty() && state.progress.is_empty() {
            return false;
        }

        // Margin 与 Padding 都相对 MaxHeight 缩放
        let factor = (rc.bottom - rc.top) as f32 / (theme.max_height.max(1) as f32);
        draw_layer(hdc, &state.background, rc, factor);

        // 进度条按电量比例缩短。竖排任务栏下从下往上长。
        if !state.progress.is_empty() && st.pct > 0 {
            let m = scaled(state.progress.margin, factor);
            let inner = RECT {
                left: rc.left + m[0],
                top: rc.top + m[1],
                right: rc.right - m[2],
                bottom: rc.bottom - m[3],
            };
            if inner.right > inner.left && inner.bottom > inner.top {
                let fill = if vert {
                    RECT {
                        left: inner.left,
                        top: inner.bottom - (inner.bottom - inner.top) * (100 - st.pct) / 100,
                        right: inner.right,
                        bottom: inner.bottom,
                    }
                } else {
                    RECT {
                        left: inner.left,
                        top: inner.top,
                        right: inner.left + (inner.right - inner.left) * st.pct / 100,
                        bottom: inner.bottom,
                    }
                };
                draw_layer(hdc, &state.progress, fill, factor);
            }
        }

        if st.pressed && !state.pressed.is_empty() {
            draw_layer(hdc, &state.pressed, rc, factor);
        } else if st.hovered && !state.hover.is_empty() {
            draw_layer(hdc, &state.hover, rc, factor);
        }
        true
    })
}

/// 取主题给出的文字样式：字体族、像素字号、是否加粗、文字色、内边距、是否带阴影。
/// xml 用 TextColor 的四元组 alpha 为 0 表示不显示文字，此时返回 None。
pub fn text_style(
    index: usize,
    charging: bool,
    low: bool,
    critical: bool,
    height: i32,
    dpi: i32,
) -> Option<(String, i32, bool, COLORREF, [i32; 4], bool)> {
    with_themes(|themes| {
        let theme = themes.get(index)?;
        let mut idx = pick_state(charging, low, critical);
        if theme.states[idx].is_none() {
            idx = 0;
        }
        let state = theme.states[idx].as_ref().or(theme.states[0].as_ref())?;
        let color = state.text_color?;
        let factor = height as f32 / (theme.max_height.max(1) as f32);
        // 字号单位是 pt，先按 dpi 换成像素，再按主题比例缩放
        let px = ((state.font_pt as f32) * (dpi as f32) / 72.0 * factor).round() as i32;
        if px < 5 {
            return None;
        }
        Some((
            state.font_face.clone(),
            px,
            state.font_bold,
            color,
            scaled(state.padding, factor),
            state.text_shadow,
        ))
    })
}
