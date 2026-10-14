// batterybar-modern — Battery Profile Graph（电池曲线窗口）
//
// 复刻原版 BatteryBar Pro 的独立曲线窗口。原版规格（来自官方 Release Notes 与
// 界面截图，详见 docs/RESEARCH-original-batterybar.md）：
//   · 独立窗口，画当前电池的放电/充电曲线（电量 % 对时间）
//   · 打开时若选中的曲线正处在当前状态，图形会**实时刷新**
//   · 打开时窗口显示当前电池状态
//   · 看充电曲线时图例文案是 "Time to full charge"
//   · 曲线标签用系统默认字体（原版曾因强制 Arial 在部分机器上显示异常）
//   · 支持缩放与导出
//
// 实现要点：
//   · 数据来自 profile::snapshot()，按睡眠断口切段（见 profile.rs）
//   · 曲线用 GDI Polyline 连；断口处不连线，避免把睡眠时长画成一条斜线
//   · 缩放/平移只作用于时间轴，电量轴固定 0..100%
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FillRect, GetStockObject, InvalidateRect,
    LineTo, MoveToEx, Polyline, SelectObject, SetBkMode, SetTextColor, UpdateWindow, DT_CENTER,
    DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DRAW_TEXT_FORMAT, HDC,
    HFONT, HGDIOBJ, HOLLOW_BRUSH, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos,
    GetWindowRect, IsWindow, KillTimer, SetForegroundWindow, SetTimer, ShowWindow, SW_SHOW,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEWHEEL, WM_PAINT, WM_TIMER, WS_CAPTION,
    WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_SYSMENU, WS_THICKFRAME,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::clock;
use crate::power;
use crate::profile;
use crate::render::{self, Palette};

pub const GRAPH_W: i32 = 800;
pub const GRAPH_H: i32 = 520;

static GRAPH_HWND: AtomicIsize = AtomicIsize::new(0);
static TAB: AtomicI32 = AtomicI32::new(0); // 0=放电 1=充电
static ZOOM: AtomicI32 = AtomicI32::new(100); // 100 = 全览
static PAN: AtomicI32 = AtomicI32::new(0); // 右端相对最新样本的回退秒数
const TIMER_LIVE: usize = 31;

// ---- 工具栏几何（绘制与命中测试共用） ----
const TOOLBAR_Y: i32 = 16;
const BTN_H: i32 = 30;
const TAB_W: i32 = 132;
const PLOT_L: i32 = 66;
const PLOT_R: i32 = GRAPH_W - 26;
const PLOT_T: i32 = TOOLBAR_Y + BTN_H + 30;
// 底部要留三段互不重叠的文字：时间轴刻度、图例、操作提示。
// 早先把 PLOT_B 放在 GRAPH_H - 64，图例占 PLOT_B + 28 而提示占 GRAPH_H - 26，
// 两者纵向区间相交，渲染出来叠成一团。
const AXIS_H: i32 = 16;
const LEGEND_H: i32 = 20;
const HINT_H: i32 = 18;
const PLOT_B: i32 = GRAPH_H - 92;
const AXIS_Y: i32 = PLOT_B + 6;
const LEGEND_Y: i32 = PLOT_B + 30;
const HINT_Y: i32 = GRAPH_H - 22;

const ID_TAB_DISCHARGE: u32 = 1;
const ID_TAB_CHARGE: u32 = 2;
const ID_ZOOM_IN: u32 = 3;
const ID_ZOOM_OUT: u32 = 4;
const ID_ZOOM_RESET: u32 = 5;
const ID_EXPORT: u32 = 6;
const ID_CLEAR: u32 = 7;

const WM_CREATE_: u32 = 0x0001;
const WM_DESTROY_: u32 = 0x0002;
const WM_KEYDOWN: u32 = 0x0100;

#[derive(Clone, Copy)]
struct Btn {
    id: u32,
    label: &'static str,
    rect: RECT,
}

fn rect(l: i32, t: i32, w: i32, h: i32) -> RECT {
    RECT { left: l, top: t, right: l + w, bottom: t + h }
}

/// 工具栏按钮：两个 tab 在左，操作按钮自右向左排。
fn buttons() -> Vec<Btn> {
    let mut out = vec![
        Btn { id: ID_TAB_DISCHARGE, label: "放电 Discharging", rect: rect(18, TOOLBAR_Y, TAB_W, BTN_H) },
        Btn { id: ID_TAB_CHARGE, label: "充电 Charging", rect: rect(18 + TAB_W + 6, TOOLBAR_Y, TAB_W, BTN_H) },
    ];
    let mut x = GRAPH_W - 20;
    let mut push = |id: u32, label: &'static str, w: i32, out: &mut Vec<Btn>| {
        x -= w;
        out.push(Btn { id, label, rect: rect(x, TOOLBAR_Y, w, BTN_H) });
        x -= 6;
    };
    push(ID_CLEAR, "清空", 62, &mut out);
    push(ID_EXPORT, "导出 CSV", 88, &mut out);
    push(ID_ZOOM_RESET, "全览", 62, &mut out);
    push(ID_ZOOM_IN, "＋", 34, &mut out);
    push(ID_ZOOM_OUT, "－", 34, &mut out);
    out
}

struct Graph {
    palette: Palette,
    font_btn: HFONT,
    font_small: HFONT,
    font_tiny: HFONT,
}

// HFONT 是裸指针句柄，不带 Send/Sync；字体只在 UI 线程用，包装后放进全局槽位。
struct GraphBox(Graph);
unsafe impl Send for GraphBox {}
unsafe impl Sync for GraphBox {}

static GRAPH: OnceLock<Mutex<GraphBox>> = OnceLock::new();

fn palette_now() -> Palette {
    if render::system_prefers_dark() {
        render::palette_dark()
    } else {
        render::palette_light()
    }
}

// ---------- 绘制基元 ----------

unsafe fn line(hdc: HDC, a: (i32, i32), b: (i32, i32), color: COLORREF, width: i32) {
    if a.0 == b.0 && a.1 == b.1 {
        return;
    }
    let pen = CreatePen(PS_SOLID, width, color);
    let old = SelectObject(hdc, pen.into());
    let _ = MoveToEx(hdc, a.0, a.1, None);
    let _ = LineTo(hdc, b.0, b.1);
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

unsafe fn polyline(hdc: HDC, pts: &[POINT], color: COLORREF, width: i32) {
    if pts.len() < 2 {
        return;
    }
    let pen = CreatePen(PS_SOLID, width, color);
    let old = SelectObject(hdc, pen.into());
    let _ = Polyline(hdc, pts);
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

unsafe fn dot(hdc: HDC, c: POINT, r: i32, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    let old = SelectObject(hdc, brush.into());
    let pen = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH));
    let _ = Ellipse(hdc, c.x - r, c.y - r, c.x + r, c.y + r);
    SelectObject(hdc, pen);
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

unsafe fn solid_fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rc as *const RECT, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

unsafe fn text_at(hdc: HDC, font: HFONT, s: &str, rc: &mut RECT, color: COLORREF, flags: DRAW_TEXT_FORMAT) {
    SelectObject(hdc, font.into());
    SetTextColor(hdc, color);
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(hdc, &mut buf, rc as *mut RECT, flags | DT_NOPREFIX);
}

unsafe fn text_line(hdc: HDC, font: HFONT, s: &str, mut rc: RECT, color: COLORREF, flags: DRAW_TEXT_FORMAT) {
    text_at(hdc, font, s, &mut rc, color, flags | DT_SINGLELINE);
}

// ---------- 数据窗口 ----------

#[derive(Clone, Copy)]
struct View {
    t0: u64,
    t1: u64,
}

/// 按当前 tab / 缩放 / 平移算出时间轴可视窗口；右侧贴最新样本，缩放围绕右端。
fn view_window(samples: &[profile::Sample], charging: bool) -> Option<View> {
    let sel: Vec<&profile::Sample> = samples.iter().filter(|s| s.charging == charging).collect();
    if sel.len() < 2 {
        return None;
    }
    let first = sel[0].t;
    let last = sel[sel.len() - 1].t;
    if last <= first {
        return None;
    }
    let zoom = ZOOM.load(Ordering::Relaxed).clamp(100, 2000);
    let full = last - first;
    let span = ((full as i64) * 100 / zoom as i64).max(120) as u64;
    let max_back = full.saturating_sub(span);
    let back = (PAN.load(Ordering::Relaxed).max(0) as u64).min(max_back);
    let t1 = last.saturating_sub(back);
    let t0 = t1.saturating_sub(span);
    Some(View { t0, t1 })
}

fn x_of(t: u64, v: View) -> i32 {
    if v.t1 <= v.t0 {
        return PLOT_L;
    }
    let frac = (t.saturating_sub(v.t0)) as f64 / (v.t1 - v.t0) as f64;
    PLOT_L + (frac * (PLOT_R - PLOT_L) as f64).round() as i32
}

fn y_of(pct: i32) -> i32 {
    let p = pct.clamp(0, 100) as f64 / 100.0;
    PLOT_B - (p * (PLOT_B - PLOT_T) as f64).round() as i32
}

// ---------- 主绘制 ----------

unsafe fn paint_graph(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut crc = RECT::default();
    let _ = GetClientRect(hwnd, &mut crc);
    let (w, h) = (crc.right - crc.left, crc.bottom - crc.top);
    if w <= 0 || h <= 0 {
        let _ = EndPaint(hwnd, &ps);
        return;
    }

    // 双缓冲，避免实时刷新闪烁
    let mem = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem, bmp.into());

    let guard = GRAPH.get().and_then(|m| m.lock().ok());
    let palette = guard.as_ref().map(|g| g.0.palette).unwrap_or_else(palette_now);
    let (font_btn, font_small, font_tiny) = match guard.as_ref() {
        Some(g) => (g.0.font_btn, g.0.font_small, g.0.font_tiny),
        None => (
            render::create_ui_font(-15, 600),
            render::create_ui_font(-13, 400),
            render::create_ui_font(-12, 400),
        ),
    };

    let full = RECT { left: 0, top: 0, right: w, bottom: h };
    solid_fill(mem, &full, palette.bg);
    SetBkMode(mem, TRANSPARENT);

    let samples = profile::snapshot();
    let st = profile::stats();
    let charging = TAB.load(Ordering::Relaxed) == 1;

    // ---- 工具栏 ----
    let selected_color = if charging { palette.accent_warn } else { palette.accent_blue };
    for b in buttons() {
        let is_tab = b.id == ID_TAB_DISCHARGE || b.id == ID_TAB_CHARGE;
        let active = (b.id == ID_TAB_DISCHARGE && !charging) || (b.id == ID_TAB_CHARGE && charging);
        let mut r = b.rect;
        if is_tab {
            let (fill, fg) = if active {
                (selected_color, render::rgb(0xFF, 0xFF, 0xFF))
            } else {
                (palette.surface, palette.text_dim)
            };
            render::fill_round(mem, r.left, r.top, r.right, r.bottom, 8, fill);
            text_at(mem, font_btn, b.label, &mut r, fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        } else {
            render::fill_round(mem, r.left, r.top, r.right, r.bottom, 7, palette.surface);
            render::stroke_round(mem, r.left, r.top, r.right, r.bottom, 7, palette.border);
            text_at(mem, font_small, b.label, &mut r, palette.text, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }

    // ---- 当前电池状态（原版：打开曲线窗口时显示当前状态）----
    let pct = power::PERCENT.load(Ordering::Relaxed);
    let on_ac = power::AC_ON.load(Ordering::Relaxed);
    let chg = power::CHARGING.load(Ordering::Relaxed);
    let state_text = format!(
        "当前  {pct}%  ·  {}  ·  {}",
        if chg {
            "充电中"
        } else if on_ac {
            "已接电源"
        } else {
            "放电中"
        },
        if on_ac { "A/C" } else { "Battery" }
    );
    let srect = rect(PLOT_L, TOOLBAR_Y + BTN_H + 6, GRAPH_W - PLOT_L - 26, 18);
    text_line(mem, font_tiny, &state_text, srect, palette.text_dim, DT_RIGHT | DT_VCENTER);

    // ---- 绘图区 ----
    let plot = RECT { left: PLOT_L, top: PLOT_T, right: PLOT_R, bottom: PLOT_B };
    solid_fill(mem, &plot, palette.track);

    for p in (0..=100).step_by(25) {
        let y = y_of(p);
        line(mem, (PLOT_L, y), (PLOT_R, y), render::mix(palette.track, palette.border, 0.6), 1);
        let lr = rect(6, y - 9, PLOT_L - 10, y + 9);
        text_line(mem, font_tiny, &format!("{p}%"), lr, palette.text_dim, DT_RIGHT | DT_VCENTER);
    }
    // Warning Levels 里的低电量/临界阈值参考线
    let (low, crit) = crate::thresholds();
    for (lvl, col) in [(low, palette.accent_warn), (crit, palette.accent_crit)] {
        if (0..=100).contains(&lvl) {
            let y = y_of(lvl);
            line(mem, (PLOT_L, y), (PLOT_R, y), render::darken(col, 0.30), 1);
        }
    }

    let view = view_window(&samples, charging);
    match view {
        None => {
            // 两种空态要分开说，否则明明攒了十几个点却提示「至少需要 2 个」，
            // 看起来像程序算错了 —— 实际是当前分类一个点都没有。
            let in_tab = samples.iter().filter(|s| s.charging == charging).count();
            let msg = if samples.is_empty() {
                "还没有任何采样数据。\n\n程序每 20 秒记一个点，充放电切换与电量变化时会立刻补记。\n让它在后台跑一段时间，再回到这个窗口。"
                    .to_string()
            } else if in_tab == 0 {
                format!(
                    "「{}」分类还没有采样数据。\n\n程序只在{}时记录曲线，当前共 {} 个点记在另一分类下。",
                    if charging { "充电 Charging" } else { "放电 Discharging" },
                    if charging { "充电" } else { "放电" },
                    samples.len()
                )
            } else {
                format!(
                    "「{}」分类采样点不足，当前只有 {} 个，至少需要 2 个才能连线。",
                    if charging { "充电 Charging" } else { "放电 Discharging" },
                    in_tab
                )
            };
            let mut mr = rect(PLOT_L + 30, PLOT_T + 80, PLOT_R - PLOT_L - 60, PLOT_B - PLOT_T - 160);
            text_at(mem, font_small, &msg, &mut mr, palette.text_dim, DT_CENTER | DT_WORDBREAK);
        }
        Some(v) => {
            for i in 0..=4u64 {
                let t = v.t0 + (v.t1 - v.t0) * i / 4;
                let x = x_of(t, v);
                line(mem, (x, PLOT_T), (x, PLOT_B), render::mix(palette.track, palette.border, 0.35), 1);
                let label = if v.t1 - v.t0 > 20 * 3600 {
                    clock::md(t as i64)
                } else {
                    clock::hm24(t as i64)
                };
                let tr = rect(x - 44, AXIS_Y, 88, AXIS_H);
                text_line(mem, font_tiny, &label, tr, palette.text_dim, DT_CENTER | DT_VCENTER);
            }

            let curve = if charging { palette.accent_warn } else { palette.accent_green };
            let segs = profile::segments(&samples, charging);
            let mut drawn = 0usize;
            for seg in &segs {
                let pts: Vec<POINT> = seg
                    .iter()
                    .filter(|s| s.t >= v.t0 && s.t <= v.t1)
                    .map(|s| POINT { x: x_of(s.t, v), y: y_of(s.pct) })
                    .collect();
                drawn += pts.len();
                polyline(mem, &pts, curve, 2);
            }

            // 最新样本处的当前状态标记
            if let Some(last) = samples.iter().rev().find(|s| s.charging == charging) {
                if last.t >= v.t0 && last.t <= v.t1 {
                    let x = x_of(last.t, v);
                    line(mem, (x, PLOT_T), (x, PLOT_B), render::lighten(curve, 0.45), 1);
                    dot(mem, POINT { x, y: y_of(last.pct) }, 4, curve);
                    let tag = format!("{}%  {}", last.pct, clock::hm24(last.t as i64));
                    let tr = rect((x - 60).max(PLOT_L + 2), PLOT_T + 4, 120, 18);
                    text_line(mem, font_tiny, &tag, tr, palette.text, DT_LEFT | DT_VCENTER);
                }
            }

            if drawn < 2 {
                let mr = rect(PLOT_L + 20, PLOT_T + 40, PLOT_R - PLOT_L - 40, 20);
                text_line(mem, font_small, "当前缩放下采样点不够，点「全览」看看。", mr, palette.text_dim, DT_CENTER | DT_VCENTER);
            }
        }
    }
    render::stroke_round(mem, plot.left, plot.top, plot.right, plot.bottom, 0, palette.border);

    // ---- 图例 ----
    // 原版在充电曲线上把图例文案写成 Time to full charge；
    // 放电曲线则给剩余时间与平均放电功率。末段统一附上「本次视图采样 / 分段」
    // 与「累计入库样本」，后者走 profile::count，不克隆样本本身。
    let segs_n = match view {
        Some(_) => profile::segments(&samples, charging).len(),
        None => 0,
    };
    let total = profile::count();
    let legend = if charging {
        format!(
            "Time to full charge: {}   ·   本视图 {} 点 / {} 段   ·   累计 {} 点",
            clock::duration(st.charge_secs),
            st.samples,
            segs_n,
            total
        )
    } else {
        format!(
            "Time remaining: {}   ·   平均放电 {:.1} W   ·   本视图 {} 点 / {} 段   ·   累计 {} 点",
            clock::duration(st.discharge_secs),
            st.avg_discharge_mw as f64 / 1000.0,
            st.samples,
            segs_n,
            total
        )
    };
    let lr = rect(PLOT_L, LEGEND_Y, GRAPH_W - PLOT_L - 26, LEGEND_H);
    text_line(mem, font_small, &legend, lr, palette.text, DT_LEFT | DT_VCENTER);

    let zoom_pct = ZOOM.load(Ordering::Relaxed);
    let hint = format!(
        "滚轮缩放，当前 {zoom_pct}%　·　←/→ 平移　·　双击图表回到全览　·　数据文件 profile.csv"
    );
    let hr = rect(PLOT_L, HINT_Y, GRAPH_W - PLOT_L - 26, HINT_H);
    text_line(mem, font_tiny, &hint, hr, palette.text_dim, DT_LEFT | DT_VCENTER);

    let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(HGDIOBJ(bmp.0));
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

// ---------- 交互 ----------

unsafe fn hit(pt: POINT) -> Option<u32> {
    buttons()
        .into_iter()
        .find(|b| pt.x >= b.rect.left && pt.x < b.rect.right && pt.y >= b.rect.top && pt.y < b.rect.bottom)
        .map(|b| b.id)
}

fn export_csv() {
    let charging = TAB.load(Ordering::Relaxed) == 1;
    let all = profile::snapshot();
    let mut out = String::from("timestamp,local_time,percent,remaining_mwh,full_mwh,rate_mw,charging\n");
    for s in &all {
        if s.charging != charging {
            continue;
        }
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            s.t,
            clock::full(s.t as i64),
            s.pct,
            s.rem,
            s.full,
            s.rate_mw,
            s.charging as u8
        ));
    }
    let mut p = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("."));
    p.pop();
    p.push(if charging {
        "battery-profile-charging.csv"
    } else {
        "battery-profile-discharging.csv"
    });
    if std::fs::write(&p, out).is_ok() {
        let _ = crate::hidden_command("explorer.exe")
            .arg(format!("/select,{}", p.display()))
            .spawn();
    }
}

fn zoom_by(factor_num: i32, factor_den: i32) {
    let z = ZOOM.load(Ordering::Relaxed);
    let nz = (z * factor_num / factor_den).clamp(100, 2000);
    ZOOM.store(nz, Ordering::Relaxed);
    if nz <= 100 {
        PAN.store(0, Ordering::Relaxed);
    }
}

pub unsafe extern "system" fn graph_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE_ => {
            GRAPH_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_PAINT => {
            paint_graph(hwnd);
            LRESULT(0)
        }
        WM_DESTROY_ => {
            let _ = KillTimer(Some(hwnd), TIMER_LIVE);
            GRAPH_HWND.store(0, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_LIVE {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if let Some(id) = hit(POINT { x, y }) {
                match id {
                    ID_TAB_DISCHARGE => {
                        TAB.store(0, Ordering::Relaxed);
                        PAN.store(0, Ordering::Relaxed);
                    }
                    ID_TAB_CHARGE => {
                        TAB.store(1, Ordering::Relaxed);
                        PAN.store(0, Ordering::Relaxed);
                    }
                    ID_ZOOM_IN => zoom_by(3, 2),
                    ID_ZOOM_OUT => zoom_by(2, 3),
                    ID_ZOOM_RESET => {
                        ZOOM.store(100, Ordering::Relaxed);
                        PAN.store(0, Ordering::Relaxed);
                    }
                    ID_EXPORT => export_csv(),
                    ID_CLEAR => {
                        profile::clear();
                        PAN.store(0, Ordering::Relaxed);
                    }
                    _ => {}
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => LRESULT(0),
        WM_LBUTTONDBLCLK => {
            ZOOM.store(100, Ordering::Relaxed);
            PAN.store(0, Ordering::Relaxed);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if delta > 0 {
                zoom_by(3, 2);
            } else {
                zoom_by(2, 3);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let z = ZOOM.load(Ordering::Relaxed);
            let step = ((GRAPH_W as i64 * 100) / z as i64) as i32 * 2;
            match wparam.0 as u32 {
                0x25 => {
                    let max_back = {
                        let s = profile::snapshot();
                        let charging = TAB.load(Ordering::Relaxed) == 1;
                        let sel: Vec<&profile::Sample> =
                            s.iter().filter(|x| x.charging == charging).collect();
                        match (sel.first(), sel.last()) {
                            (Some(f), Some(l)) => (l.t.saturating_sub(f.t)) as i32,
                            _ => 0,
                        }
                    };
                    PAN.store((PAN.load(Ordering::Relaxed) + step).min(max_back), Ordering::Relaxed);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                0x27 => {
                    PAN.store((PAN.load(Ordering::Relaxed) - step).max(0), Ordering::Relaxed);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                0x24 => {
                    ZOOM.store(100, Ordering::Relaxed);
                    PAN.store(0, Ordering::Relaxed);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                0x23 => {
                    ZOOM.store(2000, Ordering::Relaxed);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                _ => {}
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 曲线窗口是否开着且鼠标在其上（供任务栏条转发滚轮）。
pub fn cursor_over_graph() -> bool {
    let h = GRAPH_HWND.load(Ordering::Relaxed);
    if h == 0 {
        return false;
    }
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return false;
        }
        let mut rc = RECT::default();
        if GetWindowRect(HWND(h as *mut _), &mut rc).is_err() {
            return false;
        }
        pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
    }
}

/// 滚轮兜底转发：曲线窗口若没拿到键盘焦点，宿主窗口收到的 WM_MOUSEWHEEL
/// 会落到这里（与 prefs 的滚动问题同源）。
pub fn forward_wheel(wheel_delta: i32) {
    let h = GRAPH_HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    let steps = (wheel_delta / 120).clamp(-8, 8);
    if steps == 0 {
        return;
    }
    for _ in 0..steps.abs() {
        if steps > 0 {
            zoom_by(3, 2);
        } else {
            zoom_by(2, 3);
        }
    }
    unsafe {
        let _ = InvalidateRect(Some(HWND(h as *mut _)), None, false);
    }
}

/// 曲线窗口是否开着。
pub fn is_open() -> bool {
    let h = GRAPH_HWND.load(Ordering::Relaxed);
    h != 0 && unsafe { IsWindow(Some(HWND(h as *mut _))).as_bool() }
}

pub fn toggle_graph() {
    unsafe {
        let existing = GRAPH_HWND.load(Ordering::Relaxed);
        if existing != 0 {
            let ehwnd = HWND(existing as *mut _);
            if IsWindow(Some(ehwnd)).as_bool() {
                let _ = DestroyWindow(ehwnd);
                return;
            }
            GRAPH_HWND.store(0, Ordering::Relaxed);
        }
        create_graph_window();
    }
}

unsafe fn create_graph_window() {
    let Ok(hinstance) = GetModuleHandleW(windows::core::PCWSTR::null()) else {
        return;
    };
    let style = WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX & !WS_SYSMENU | WS_CAPTION;
    let mut wr = rect(0, 0, GRAPH_W, GRAPH_H);
    let _ = AdjustWindowRectEx(&mut wr, style, false, WS_EX_TOOLWINDOW);

    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        w!("BBM_Graph"),
        w!("BatteryBar · Battery Profile Graph"),
        style,
        300,
        120,
        wr.right - wr.left,
        wr.bottom - wr.top,
        None,
        None,
        Some(hinstance.into()),
        None,
    ) else {
        eprintln!("CreateWindowExW BBM_Graph failed");
        return;
    };

    let _ = GRAPH.set(Mutex::new(GraphBox(Graph {
        palette: palette_now(),
        font_btn: render::create_ui_font(-15, 600),
        font_small: render::create_ui_font(-13, 400),
        font_tiny: render::create_ui_font(-12, 400),
    })));

    let pref = windows::Win32::Graphics::Dwm::DWMWCP_ROUND.0;
    let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
        hwnd,
        windows::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE,
        &pref as *const _ as *const _,
        std::mem::size_of::<u32>() as u32,
    );

    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = UpdateWindow(hwnd);
    let _ = SetTimer(Some(hwnd), TIMER_LIVE, 5_000, None);
    let _ = SetForegroundWindow(hwnd);
}
