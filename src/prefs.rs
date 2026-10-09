// batterybar-modern — Preferences 设置中心（Telegram Desktop Settings 风格）
// 深度借鉴 Telegram Desktop 设置面板：分组圆角卡片、Telegram 蓝滑块、滚轮平滑滚动、无文字重叠。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC, PAINTSTRUCT,
    PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, ShowWindow,
    CS_HREDRAW, CS_VREDRAW, SW_SHOW, WNDCLASSW, WS_CAPTION, WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows_core::{w, PCWSTR};

use crate::config::{Config, DisplayMode};
use crate::render;

const PREFS_W: i32 = 420;
const PREFS_VIEW_H: i32 = 520;
const ROW_H: i32 = 38;

struct Prefs {
    palette: render::Palette,
    font_section: windows::Win32::Graphics::Gdi::HFONT,
    font_label: windows::Win32::Graphics::Gdi::HFONT,
    font_value: windows::Win32::Graphics::Gdi::HFONT,
    font_sub: windows::Win32::Graphics::Gdi::HFONT,
}

struct PrefsBox(Prefs);
unsafe impl Send for PrefsBox {}
unsafe impl Sync for PrefsBox {}

static PREFS_HWND: AtomicIsize = AtomicIsize::new(0);
static PREFS_BOX: std::sync::OnceLock<PrefsBox> = std::sync::OnceLock::new();
static SCROLL_Y: AtomicI32 = AtomicI32::new(0);

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
            SettingItem::Toggle("alternate_on_click", "单击电池条在 4 种模式间循环切换"),
        ],
    },
    Section {
        title: "显示与样式 · DISPLAY",
        items: &[
            SettingItem::Choice("mode", "默认显示模式"),
            SettingItem::Toggle("only_percent_when_charging", "充电时只显示电量百分比"),
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
        ],
    },
    Section {
        title: "警告阈值 · WARNING LEVELS",
        items: &[
            SettingItem::Choice("low_percent", "低电量警告阈值"),
            SettingItem::Choice("critical_percent", "临界电量紧急阈值"),
        ],
    },
    Section {
        title: "算法与工具 · BATTERY TOOLS",
        items: &[
            SettingItem::Toggle("show_rate_in_tooltip", "在详细卡片中展示实时功耗 (mW)"),
            SettingItem::Toggle("learn_profiles", "开启全寿命自学习预测算法"),
            SettingItem::Action("reset_history", "重置充放电学习历史记录"),
        ],
    },
];

fn compute_total_height() -> i32 {
    let mut h = 20;
    for s in SECTIONS {
        h += 28; // section header
        h += s.items.len() as i32 * ROW_H + 8; // card height
        h += 16; // spacing
    }
    h + 20
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

        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            w!("BatteryBar · Preferences"),
            WS_CAPTION | WS_SYSMENU,
            240,
            140,
            PREFS_W,
            PREFS_VIEW_H,
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
            font_sub: render::create_ui_font(11, 400),
        }));

        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const _, 4);

        SCROLL_Y.store(0, Ordering::Relaxed);
        PREFS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
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
        0x020A /* WM_MOUSEWHEEL */ => {
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let current = SCROLL_Y.load(Ordering::Relaxed);
            let total_h = compute_total_height();
            let max_scroll = (total_h - PREFS_VIEW_H + 40).max(0);
            let next = (current - (delta / 120) * 45).clamp(0, max_scroll);
            SCROLL_Y.store(next, Ordering::Relaxed);
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ => {
            let y = ((lparam.0 >> 16) & 0xFFFF) as i32;
            handle_click(hwnd, y);
            LRESULT(0)
        }
        0x0010 /* WM_CLOSE */ => {
            let _ = DestroyWindow(hwnd);
            PREFS_HWND.store(0, Ordering::Relaxed);
            LRESULT(0)
        }
        0x0002 /* WM_DESTROY */ => {
            PREFS_HWND.store(0, Ordering::Relaxed);
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
            "only_percent_when_charging" => { c.only_percent_when_charging = !c.only_percent_when_charging; save = true; }
            "battery_style" => {
                c.battery_style = if c.battery_style == 0 { 1 } else { 0 };
                save = true;
            }
            "notify_low" => { c.notify_low = !c.notify_low; save = true; }
            "notify_critical" => { c.notify_critical = !c.notify_critical; save = true; }
            "notify_full" => { c.notify_full = !c.notify_full; save = true; }
            "play_sound" => { c.play_sound = !c.play_sound; save = true; }
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
            "show_rate_in_tooltip" => { c.show_rate_in_tooltip = !c.show_rate_in_tooltip; save = true; }
            "learn_profiles" => { c.learn_profiles = !c.learn_profiles; save = true; }
            "reset_history" => {
                let mut p = std::env::current_exe().unwrap_or_default();
                p.pop();
                p.push("battery_history.jsonl");
                let _ = std::fs::remove_file(p);
            }
            _ => {}
        }
        if save {
            c.save();
        }
    }
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
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
                        // 标签
                        SetTextColor(hdc, pal.text);
                        SelectObject(hdc, p.font_label.into());
                        let mut lbuf: Vec<u16> = label.encode_utf16().collect();
                        let mut lrc = RECT { left: card_l + 16, top: row_y, right: card_r - 60, bottom: row_y + ROW_H };
                        DrawTextW(hdc, &mut lbuf, &mut lrc, DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER);

                        // 开关值
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

                        let val_str = match *id {
                            "mode" => match c.default_mode {
                                DisplayMode::TimeRemaining => "时间剩余",
                                DisplayMode::Percent => "电量百分比",
                                DisplayMode::DischargeRate => "实时功率 (W)",
                                DisplayMode::Capacity => "剩余容量 (Wh)",
                                DisplayMode::Icon => "小图标模式",
                            },
                            "battery_style" => if c.battery_style == 0 { "Telegram 优雅卡片" } else { "现代极简 Fluent" },
                            "low_percent" => match c.low_percent {
                                15 => "15%",
                                20 => "20%",
                                25 => "25%",
                                30 => "30%",
                                35 => "35%",
                                _ => "20%",
                            },
                            "critical_percent" => match c.critical_percent {
                                5 => "5%",
                                10 => "10%",
                                15 => "15%",
                                _ => "10%",
                            },
                            _ => "",
                        };

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

    // 优雅隐形细滑块（Telegram Scrollbar）
    let total_h = compute_total_height();
    if total_h > PREFS_VIEW_H {
        let bar_x = PREFS_W - 8;
        let track_h = PREFS_VIEW_H - 24;
        let thumb_h = ((PREFS_VIEW_H as f32 / total_h as f32) * track_h as f32).max(28.0) as i32;
        let max_scroll = total_h - PREFS_VIEW_H + 40;
        let thumb_y = 12 + ((scroll as f32 / max_scroll as f32) * (track_h - thumb_h) as f32) as i32;
        render::fill_round(hdc, bar_x, thumb_y, bar_x + 4, thumb_y + thumb_h, 2, render::rgb(65, 80, 95));
    }
}
