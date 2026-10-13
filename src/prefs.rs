// batterybar-modern — Preferences 设置中心（Telegram Desktop Settings 风格）
// 深度借鉴 Telegram Desktop 设置面板：分组圆角卡片、Telegram 蓝滑块、滚轮平滑滚动、无文字重叠。
//
// 滚动失效根因（v0.5.0 修复）：BBM_Bar 是 Shell_TrayWnd 的**子窗口，子窗口拿不到键盘焦点**，
// 从任务栏右键菜单打开的 BBM_Prefs 从未成为前台/焦点窗口；而 WM_MOUSEWHEEL 只投递给
// **焦点窗口**，所以滚轮永远到不了本面板。三重修复：
//   ① ShowWindow 后 SetForegroundWindow + WM_MOUSEACTIVATE 返回 MA_ACTIVATE（正常路径）；
//   ② BBM_Bar / BBM_MsgHost 的 WM_MOUSEWHEEL 里做光标命中判定后转发到 forward_wheel()（兜底）；
//   ③ 滚动条可拖拽 + 键盘 PageUp/PageDown/上下/Home/End。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC, PAINTSTRUCT,
    PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, SetForegroundWindow,
    ShowWindow, SW_SHOW, WS_CAPTION, WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows_core::{w, PCWSTR};

use crate::config::{Config, DisplayMode};
use crate::render;

const PREFS_W: i32 = 420;
const PREFS_VIEW_H: i32 = 520;
const ROW_H: i32 = 38;
const WHEEL_STEP: i32 = 45;
const SCROLLBAR_W: i32 = 10;

struct Prefs {
    palette: render::Palette,
    font_section: windows::Win32::Graphics::Gdi::HFONT,
    font_label: windows::Win32::Graphics::Gdi::HFONT,
    font_value: windows::Win32::Graphics::Gdi::HFONT,
}

struct PrefsBox(Prefs);
unsafe impl Send for PrefsBox {}
unsafe impl Sync for PrefsBox {}

static PREFS_HWND: AtomicIsize = AtomicIsize::new(0);
static PREFS_BOX: std::sync::OnceLock<PrefsBox> = std::sync::OnceLock::new();
static SCROLL_Y: AtomicI32 = AtomicI32::new(0);
/// 拖拽滚动条中：Some(抓取时的 thumb 顶端屏幕偏移)
static DRAG_OFFSET: AtomicI32 = AtomicI32::new(-1);

#[derive(Clone, Copy)]
enum SettingItem {
    Toggle(&'static str, &'static str), // (id, label)
    Choice(&'static str, &'static str), // (id, label)
    Action(&'static str, &'static str), // (id, label)
}

struct Section {
    title: &'static str,
    items: &'static [SettingItem],
}

const SECTIONS: &[Section] = &[
    Section {
        title: "常规设置 · GENERAL",
        items: &[
            SettingItem::Toggle("show_tooltip", "悬停自动显示详细信息卡片"),
            SettingItem::Toggle("hide_when_full", "充满电（100%）时自动隐藏电池条"),
            SettingItem::Toggle("run_at_startup", "开机随 Windows 启动"),
            SettingItem::Toggle("alternate_on_click", "单击电池条在 5 种模式间循环切换"),
        ],
    },
    Section {
        title: "显示与样式 · DISPLAY",
        items: &[
            SettingItem::Choice("mode", "默认显示模式"),
            SettingItem::Choice("font_size", "电池条文字字号"),
            SettingItem::Toggle("only_percent_when_charging", "充电时只显示电量百分比"),
            SettingItem::Choice("bar_opacity", "电池条背景不透明度"),
            SettingItem::Choice("bar_width", "横排电池条宽度"),
            SettingItem::Choice("battery_style", "视觉主题风格"),
        ],
    },
    Section {
        title: "警报与通知 · NOTIFICATIONS",
        items: &[
            SettingItem::Toggle("notify_low", "低电量气泡通知"),
            SettingItem::Toggle("notify_critical", "临界电量紧急通知"),
            SettingItem::Toggle("notify_full", "充满电完成提示"),
            SettingItem::Toggle("play_sound", "警报时播放系统提示音"),
            SettingItem::Choice("alarm_percent", "自定义电量警报"),
        ],
    },
    Section {
        title: "警告阈值 · WARNING LEVELS",
        items: &[
            SettingItem::Choice("low_percent", "低电量警告阈值"),
            SettingItem::Choice("critical_percent", "临界电量紧急阈值"),
            SettingItem::Choice("critical_action", "临界电量自动动作"),
        ],
    },
    Section {
        title: "行为 · BEHAVIOR",
        items: &[
            SettingItem::Choice("on_click_mode", "左键单击电池条"),
            SettingItem::Toggle("hide_on_ac", "接通电源时隐藏电池条"),
        ],
    },
    Section {
        title: "算法与工具 · BATTERY TOOLS",
        items: &[
            SettingItem::Toggle("show_rate_in_tooltip", "在详细卡片中展示实时功耗 (mW)"),
            SettingItem::Toggle("learn_profiles", "开启全寿命自学习预测算法"),
            SettingItem::Action("reset_history", "重置充放电学习历史记录"),
            SettingItem::Action("reset_config", "恢复全部设置为默认值"),
        ],
    },
];

fn compute_total_height() -> i32 {
    let mut h = 16;
    for s in SECTIONS {
        h += 28; // section header
        h += s.items.len() as i32 * ROW_H + 8; // card height
        h += 16; // spacing
    }
    h + 16
}

fn max_scroll() -> i32 {
    (compute_total_height() - PREFS_VIEW_H).max(0)
}

fn clamp_scroll(v: i32) -> i32 {
    v.clamp(0, max_scroll())
}

/// 任务栏条 / 消息宿主窗口的 WM_MOUSEWHEEL 兜底转发：
/// prefs 窗口没拿到键盘焦点时，滚轮会落到宿主窗口，这里做光标命中判定后转发。
pub fn cursor_over_prefs() -> bool {
    let existing = PREFS_HWND.load(Ordering::Relaxed);
    if existing == 0 {
        return false;
    }
    unsafe {
        let hwnd = HWND(existing as *mut _);
        if !windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let mut pt = POINT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt).is_err() {
            return false;
        }
        let mut rc = RECT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rc).is_err() {
            return false;
        }
        pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
    }
}

/// 由宿主窗口转发过来的滚轮增量（正数向上）
pub fn forward_wheel(wheel_delta: i32) {
    let existing = PREFS_HWND.load(Ordering::Relaxed);
    if existing == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(existing as *mut _);
        if !windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(hwnd)).as_bool() {
            return;
        }
        let steps = (wheel_delta / 120).max(-8).min(8);
        if steps == 0 {
            return;
        }
        let next = clamp_scroll(SCROLL_Y.load(Ordering::Relaxed) - steps * WHEEL_STEP);
        SCROLL_Y.store(next, Ordering::Relaxed);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

pub fn toggle_prefs() {
    unsafe {
        let existing = PREFS_HWND.load(Ordering::Relaxed);
        if existing != 0 {
            let ehwnd = HWND(existing as *mut _);
            if windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(ehwnd)).as_bool() {
                let _ = DestroyWindow(ehwnd);
                PREFS_HWND.store(0, Ordering::Relaxed);
                return;
            } else {
                PREFS_HWND.store(0, Ordering::Relaxed);
            }
        }

        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h,
            Err(_) => return,
        };
        let class_name = w!("BBM_Prefs");
        let dark = render::system_prefers_dark();
        let palette = if dark { render::palette_dark() } else { render::palette_light() };

        // 客户区精确 = PREFS_W x PREFS_VIEW_H（此前窗口高直接写 PREFS_VIEW_H，
        // 标题栏被算进总高 → 客户区只有 ~485px，底部卡片被裁且滚动算错）
        let style = WS_CAPTION | WS_SYSMENU;
        let mut wr = RECT {
            left: 0,
            top: 0,
            right: PREFS_W,
            bottom: PREFS_VIEW_H,
        };
        let _ = AdjustWindowRectEx(&mut wr, style, false, WS_EX_TOOLWINDOW);
        let outer_w = wr.right - wr.left;
        let outer_h = wr.bottom - wr.top;

        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            w!("BatteryBar · Preferences"),
            style,
            240,
            140,
            outer_w,
            outer_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("CreateWindowExW BBM_Prefs error: {e}");
                return;
            }
        };

        let _ = PREFS_BOX.set(PrefsBox(Prefs {
            palette,
            font_section: render::create_ui_font(12, 600),
            font_label: render::create_ui_font(13, 500),
            font_value: render::create_ui_font(13, 600),
        }));

        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const _, 4);

        SCROLL_Y.store(0, Ordering::Relaxed);
        DRAG_OFFSET.store(-1, Ordering::Relaxed);
        PREFS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = ShowWindow(hwnd, SW_SHOW);
        // 关键修复：必须显式抢前台，否则 WM_MOUSEWHEEL 不会投递给本窗口
        let _ = SetForegroundWindow(hwnd);
    }
}

fn scroll_by(delta_px: i32) {
    let next = clamp_scroll(SCROLL_Y.load(Ordering::Relaxed) + delta_px);
    if next != SCROLL_Y.load(Ordering::Relaxed) {
        SCROLL_Y.store(next, Ordering::Relaxed);
        let existing = PREFS_HWND.load(Ordering::Relaxed);
        if existing != 0 {
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(HWND(existing as *mut _)), None, false);
            }
        }
    }
}

fn scrollbar_geom(scroll: i32) -> (i32, i32, i32) {
    // 返回 (thumb_top, thumb_h, track_h)
    let total_h = compute_total_height();
    let track_h = PREFS_VIEW_H - 24;
    let ratio = if total_h > PREFS_VIEW_H {
        (PREFS_VIEW_H as f32 / total_h as f32).clamp(0.08, 1.0)
    } else {
        1.0
    };
    let thumb_h = ((track_h as f32) * ratio).max(28.0).min(track_h as f32) as i32;
    let ms = max_scroll();
    let frac = if ms > 0 { (scroll as f32 / ms as f32).clamp(0.0, 1.0) } else { 0.0 };
    let thumb_top = 12 + ((track_h - thumb_h) as f32 * frac) as i32;
    (thumb_top, thumb_h, track_h)
}

pub unsafe extern "system" fn prefs_wndproc(
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
                paint_prefs(&boxed.0, hdc);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        0x0021 /* WM_MOUSEACTIVATE */ => LRESULT(1 /* MA_ACTIVATE */),
        0x020A /* WM_MOUSEWHEEL */ => {
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let steps = (delta / 120).max(-8).min(8);
            scroll_by(-steps * WHEEL_STEP);
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ => {
            let x = (lparam.0 & 0xFFFF) as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i32;
            // 滚动条命中 → 进入拖拽模式
            if max_scroll() > 0 && x >= PREFS_W - 8 - SCROLLBAR_W {
                let (thumb_top, _, _) = scrollbar_geom(SCROLL_Y.load(Ordering::Relaxed));
                DRAG_OFFSET.store(y - thumb_top, Ordering::Relaxed);
                let _ = SetCapture(hwnd);
                LRESULT(0)
            } else {
                // 让本窗口拿到键盘焦点：WM_MOUSEWHEEL 只投递给焦点窗口，
                // 设置面板是从任务栏（子窗口，永远无焦点）用菜单打开的，不设焦点则滚轮无效。
                let _ = SetFocus(Some(hwnd));
                handle_click(hwnd, y);
                LRESULT(0)
            }
        }
        0x0202 /* WM_LBUTTONUP */ => {
            if DRAG_OFFSET.load(Ordering::Relaxed) >= 0 {
                DRAG_OFFSET.store(-1, Ordering::Relaxed);
                let _ = ReleaseCapture();
            }
            LRESULT(0)
        }
        0x0203 /* WM_LBUTTONDBLCLK */ => {
            let y = ((lparam.0 >> 16) & 0xFFFF) as i32;
            handle_click(hwnd, y);
            LRESULT(0)
        }
        0x0200 /* WM_MOUSEMOVE */ => {
            if DRAG_OFFSET.load(Ordering::Relaxed) >= 0 && max_scroll() > 0 {
                let y = ((lparam.0 >> 16) & 0xFFFF) as i32;
                let grab = DRAG_OFFSET.load(Ordering::Relaxed);
                let (_, thumb_h, track_h) = scrollbar_geom(SCROLL_Y.load(Ordering::Relaxed));
                let usable = (track_h - thumb_h).max(1);
                let mut frac = (y - grab - 12) as f32 / usable as f32;
                if frac < 0.0 {
                    frac = 0.0;
                }
                if frac > 1.0 {
                    frac = 1.0;
                }
                let next = clamp_scroll((frac * max_scroll() as f32) as i32);
                if next != SCROLL_Y.load(Ordering::Relaxed) {
                    SCROLL_Y.store(next, Ordering::Relaxed);
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        0x0100 /* WM_KEYDOWN */ => {
            match wparam.0 as u16 as i32 as u32 {
                0x21 /* VK_PRIOR */ => scroll_by(-PREFS_VIEW_H + 64),
                0x22 /* VK_NEXT */ => scroll_by(PREFS_VIEW_H - 64),
                0x26 /* VK_UP */ => scroll_by(-WHEEL_STEP),
                0x28 /* VK_DOWN */ => scroll_by(WHEEL_STEP),
                0x24 /* VK_HOME */ => scroll_by(-max_scroll()),
                0x23 /* VK_END */ => scroll_by(max_scroll()),
                _ => {}
            }
            LRESULT(0)
        }
        0x0010 /* WM_CLOSE */ => {
            let _ = DestroyWindow(hwnd);
            PREFS_HWND.store(0, Ordering::Relaxed);
            LRESULT(0)
        }
        0x0002 /* WM_DESTROY */ => {
            PREFS_HWND.store(0, Ordering::Relaxed);
            DRAG_OFFSET.store(-1, Ordering::Relaxed);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn handle_click(hwnd: HWND, click_y: i32) {
    let scroll = SCROLL_Y.load(Ordering::Relaxed);
    let mut cur_y = 16 - scroll;

    let mut clicked_id: Option<&'static str> = None;

    'outer: for s in SECTIONS {
        cur_y += 28; // title height
        cur_y += 4;  // top padding of card
        for it in s.items {
            if click_y >= cur_y && click_y < cur_y + ROW_H {
                clicked_id = match it {
                    SettingItem::Toggle(id, _) | SettingItem::Choice(id, _) | SettingItem::Action(id, _) => Some(*id),
                };
                break 'outer;
            }
            cur_y += ROW_H;
        }
        cur_y += 4 + 16; // bottom padding of card + margin
    }

    let Some(id) = clicked_id else { return };
    let mut save = false;
    // 该设置项是否影响条的几何尺寸（宽度/显隐）—— 是则改完必须重新布局，
    // 光 InvalidateRect 重绘不够：窗口尺寸没变，新宽度不会生效。
    let mut need_relayout = false;
    {
        let cfg = crate::config_lock();
        let mut c = cfg.lock().unwrap();
        match id {
            "show_tooltip" => { c.show_tooltip = !c.show_tooltip; save = true; }
            "hide_when_full" => { c.hide_when_full = !c.hide_when_full; save = true; }
            "run_at_startup" => {
                c.run_at_startup = !c.run_at_startup;
                let en = c.run_at_startup;
                let c2 = c.clone();
                c2.set_startup(en);
                save = true;
            }
            "alternate_on_click" => { c.alternate_on_click = !c.alternate_on_click; save = true; }
            "mode" => {
                c.default_mode = match c.default_mode {
                    DisplayMode::TimeRemaining => DisplayMode::Percent,
                    DisplayMode::Percent => DisplayMode::DischargeRate,
                    DisplayMode::DischargeRate => DisplayMode::Capacity,
                    DisplayMode::Capacity => DisplayMode::Icon,
                    DisplayMode::Icon => DisplayMode::TimeRemaining,
                };
                crate::set_mode(c.default_mode);
                save = true;
            }
            "font_size" => {
                c.font_size = (c.font_size + 1) % 3;
                save = true;
            }
            "only_percent_when_charging" => { c.only_percent_when_charging = !c.only_percent_when_charging; save = true; }
            "bar_opacity" => {
                c.bar_opacity = match c.bar_opacity {
                    100 => 80,
                    80 => 60,
                    60 => 40,
                    _ => 100,
                };
                save = true;
            }
            // 横排条宽度。竖排任务栏下此项无效（条宽必须等于任务栏宽度），
            // 但仍允许设置 —— 用户换回横排任务栏后即可生效。
            // 档位 100 → 150 → 200 → 300 → 400 → 100，默认值 150 在循环上。
            "bar_width" => {
                c.bar_width = match c.bar_width {
                    w if w >= 400 => 100,
                    w if w >= 300 => 400,
                    w if w >= 200 => 300,
                    w if w >= 150 => 200,
                    _ => 150,
                };
                save = true;
                need_relayout = true;
            }
            "battery_style" => {
                c.battery_style = if c.battery_style == 0 { 1 } else { 0 };
                save = true;
            }
            "notify_low" => { c.notify_low = !c.notify_low; save = true; }
            "notify_critical" => { c.notify_critical = !c.notify_critical; save = true; }
            "notify_full" => { c.notify_full = !c.notify_full; save = true; }
            "play_sound" => { c.play_sound = !c.play_sound; save = true; }
            "alarm_percent" => {
                c.alarm_percent = match c.alarm_percent {
                    0 => 50,
                    50 => 40,
                    40 => 30,
                    _ => 0,
                };
                save = true;
            }
            "low_percent" => {
                c.low_percent = match c.low_percent {
                    15 => 20,
                    20 => 25,
                    25 => 30,
                    30 => 35,
                    _ => 15,
                };
                if c.critical_percent >= c.low_percent { c.critical_percent = c.low_percent - 5; }
                save = true;
            }
            "critical_percent" => {
                c.critical_percent = match c.critical_percent {
                    5 => 10,
                    10 => 15,
                    _ => 5,
                };
                save = true;
            }
            "critical_action" => {
                c.critical_action = (c.critical_action + 1) % 5;
                save = true;
            }
            "on_click_mode" => {
                c.on_click_mode = if c.on_click_mode == 0 { 1 } else { 0 };
                save = true;
            }
            "hide_on_ac" => { c.hide_on_ac = !c.hide_on_ac; save = true; need_relayout = true; }
            "show_rate_in_tooltip" => { c.show_rate_in_tooltip = !c.show_rate_in_tooltip; save = true; }
            "learn_profiles" => {
                c.learn_profiles = !c.learn_profiles;
                // 立即同步到数据层：关闭后不再写 battery_history.jsonl、不再更新学习值
                crate::power::LEARN_ENABLED.store(c.learn_profiles, Ordering::Relaxed);
                save = true;
            }
            "reset_history" => {
                let mut p = std::env::current_exe().unwrap_or_default();
                p.pop();
                p.push("battery_history.jsonl");
                let _ = std::fs::remove_file(p);
            }
            "reset_config" => {
                let def = Config::default();
                *c = def;
                c.save();
                // 重置后把学习开关同步回默认值
                crate::power::LEARN_ENABLED.store(c.learn_profiles, Ordering::Relaxed);
                crate::set_mode(c.default_mode);
                need_relayout = true;
            }
            _ => {}
        }
        if save {
            c.save();
        }
    }
    // 几何类设置（条宽/显隐）改完必须重新布局，否则窗口尺寸不变、新宽度不生效。
    if need_relayout {
        crate::relayout_bar();
    }
    // 注意：不在这里直接执行临界动作（否则点一下选项就关机/睡眠）。
    // 动作只在电量真的掉到临界阈值时由 main.rs::check_notifications 触发。
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    crate::refresh_bar();
}

unsafe fn paint_prefs(p: &Prefs, hdc: HDC) {
    let pal = &p.palette;
    let rc = RECT { left: 0, top: 0, right: PREFS_W, bottom: PREFS_VIEW_H };
    FillRect(hdc, &rc, CreateSolidBrush(pal.bg));
    render::set_bk_transparent(hdc);

    let cfg = crate::config_lock();
    let c = cfg.lock().unwrap();
    let scroll = SCROLL_Y.load(Ordering::Relaxed);

    let mut cur_y = 16 - scroll;
    let card_l = 16;
    let card_r = PREFS_W - 24;

    for s in SECTIONS {
        // Section Title（优雅克制淡蓝灰大写小标）
        if cur_y + 24 >= 0 && cur_y <= PREFS_VIEW_H {
            SetTextColor(hdc, pal.accent_blue);
            SelectObject(hdc, p.font_section.into());
            let mut tbuf: Vec<u16> = s.title.encode_utf16().collect();
            let mut trc = RECT { left: card_l + 4, top: cur_y, right: card_r, bottom: cur_y + 22 };
            DrawTextW(hdc, &mut tbuf, &mut trc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);
        }
        cur_y += 28;

        // Group Card（Telegram 经典圆角卡片 #242F3D）
        let card_h = s.items.len() as i32 * ROW_H + 8;
        if cur_y + card_h >= 0 && cur_y <= PREFS_VIEW_H {
            render::fill_round(hdc, card_l, cur_y, card_r, cur_y + card_h, 8, pal.surface);
            render::stroke_round(hdc, card_l, cur_y, card_r, cur_y + card_h, 8, pal.border);
        }

        let item_start_y = cur_y + 4;
        for (idx, it) in s.items.iter().enumerate() {
            let row_y = item_start_y + idx as i32 * ROW_H;
            if row_y + ROW_H >= 0 && row_y <= PREFS_VIEW_H {
                // 行间微细分割线（最后一行不画）
                if idx + 1 < s.items.len() {
                    let pen = CreatePen(PS_SOLID, 1, pal.border);
                    let old_p = SelectObject(hdc, pen.into());
                    let _ = MoveToEx(hdc, card_l + 16, row_y + ROW_H, None);
                    let _ = LineTo(hdc, card_r - 16, row_y + ROW_H);
                    SelectObject(hdc, old_p);
                    let _ = DeleteObject(pen.into());
                }

                match it {
                    SettingItem::Toggle(id, label) => {
                        SetTextColor(hdc, pal.text);
                        SelectObject(hdc, p.font_label.into());
                        let mut lbuf: Vec<u16> = label.encode_utf16().collect();
                        let mut lrc = RECT { left: card_l + 16, top: row_y, right: card_r - 60, bottom: row_y + ROW_H };
                        DrawTextW(hdc, &mut lbuf, &mut lrc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);

                        let on = match *id {
                            "show_tooltip" => c.show_tooltip,
                            "hide_when_full" => c.hide_when_full,
                            "run_at_startup" => c.run_at_startup,
                            "alternate_on_click" => c.alternate_on_click,
                            "only_percent_when_charging" => c.only_percent_when_charging,
                            "notify_low" => c.notify_low,
                            "notify_critical" => c.notify_critical,
                            "notify_full" => c.notify_full,
                            "play_sound" => c.play_sound,
                            "show_rate_in_tooltip" => c.show_rate_in_tooltip,
                            "learn_profiles" => c.learn_profiles,
                            "hide_on_ac" => c.hide_on_ac,
                            _ => false,
                        };

                        // Telegram Desktop 标志性 Toggle Switch
                        let sw_w = 38i32;
                        let sw_h = 20i32;
                        let sx = card_r - 16 - sw_w;
                        let sy = row_y + (ROW_H - sw_h) / 2;
                        let track_c = if on { pal.accent_blue } else { render::rgb(30, 42, 54) };
                        render::fill_round(hdc, sx, sy, sx + sw_w, sy + sw_h, 10, track_c);

                        let dot_d = 14i32;
                        let dx = if on { sx + sw_w - dot_d - 3 } else { sx + 3 };
                        render::fill_round(hdc, dx, sy + 3, dx + dot_d, sy + 3 + dot_d, 7, render::rgb(255, 255, 255));
                    }
                    SettingItem::Choice(id, label) => {
                        SetTextColor(hdc, pal.text);
                        SelectObject(hdc, p.font_label.into());
                        let mut lbuf: Vec<u16> = label.encode_utf16().collect();
                        let mut lrc = RECT { left: card_l + 16, top: row_y, right: card_r - 140, bottom: row_y + ROW_H };
                        DrawTextW(hdc, &mut lbuf, &mut lrc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);

                        let val_str = choice_value(id, &c);

                        SetTextColor(hdc, pal.accent_blue);
                        SelectObject(hdc, p.font_value.into());
                        let mut vbuf: Vec<u16> = val_str.encode_utf16().collect();
                        let mut vrc = RECT { left: card_r - 150, top: row_y, right: card_r - 16, bottom: row_y + ROW_H };
                        DrawTextW(hdc, &mut vbuf, &mut vrc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER | windows::Win32::Graphics::Gdi::DT_RIGHT);
                    }
                    SettingItem::Action(_, label) => {
                        SetTextColor(hdc, pal.accent_warn);
                        SelectObject(hdc, p.font_label.into());
                        let mut abuf: Vec<u16> = format!("⚙ {}", label).encode_utf16().collect();
                        let mut arc = RECT { left: card_l + 16, top: row_y, right: card_r - 16, bottom: row_y + ROW_H };
                        DrawTextW(hdc, &mut abuf, &mut arc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);
                    }
                }
            }
        }
        cur_y += card_h + 16;
    }

    // 隐形细滚动条（Telegram Scrollbar），可拖拽
    if max_scroll() > 0 {
        let (thumb_top, thumb_h, _) = scrollbar_geom(scroll);
        let bar_x = PREFS_W - 12;
        let dragging = DRAG_OFFSET.load(Ordering::Relaxed) >= 0;
        let col = if dragging { render::rgb(120, 145, 175) } else { render::rgb(65, 80, 95) };
        render::fill_round(hdc, bar_x, thumb_top, bar_x + 4, thumb_top + thumb_h, 2, col);
    }
    drop(c);
}

fn choice_value(id: &str, c: &Config) -> String {
    match id {
        "mode" => match c.default_mode {
            DisplayMode::TimeRemaining => "时间剩余".into(),
            DisplayMode::Percent => "电量百分比".into(),
            DisplayMode::DischargeRate => "实时功率 (W)".into(),
            DisplayMode::Capacity => "剩余容量 (Wh)".into(),
            DisplayMode::Icon => "小图标模式".into(),
        },
        "font_size" => match c.font_size {
            0 => "小 10pt".into(),
            1 => "中 12pt".into(),
            _ => "大 14pt".into(),
        },
        "battery_style" => if c.battery_style == 0 { "经典拟物 Aero Glass".into() } else { "现代极简 Fluent".into() },
        "bar_opacity" => format!("{}%", c.bar_opacity),
        "bar_width" => format!("{} px", c.bar_width),
        "alarm_percent" => if c.alarm_percent == 0 { "关闭".into() } else { format!("{}%", c.alarm_percent) },
        "low_percent" => format!("{}%", c.low_percent),
        "critical_percent" => format!("{}%", c.critical_percent),
        "critical_action" => match c.critical_action {
            1 => "睡眠".into(),
            2 => "关机".into(),
            3 => "休眠".into(),
            4 => "重启".into(),
            _ => "不执行".into(),
        },
        "on_click_mode" => if c.on_click_mode == 0 { "循环切换显示模式".into() } else { "直接弹出详情卡片".into() },
        _ => String::new(),
    }
}
