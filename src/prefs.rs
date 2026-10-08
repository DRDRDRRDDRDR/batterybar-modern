// batterybar-modern — Preferences 窗口（原版完整复刻版）
// 支持鼠标滚轮平滑滑动、全功能项、无文字重叠、优雅细滚动条。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

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
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, ShowWindow,
    CS_HREDRAW, CS_VREDRAW, SW_SHOW, WNDCLASSW, WS_CAPTION, WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows_core::{w, PCWSTR};

use crate::config::{Config, DisplayMode};
use crate::render;

const PREFS_W: i32 = 400;
const PREFS_VIEW_H: i32 = 500;
const ITEM_H: i32 = 36;
const HEADER_H: i32 = 32;

struct Prefs {
    palette: render::Palette,
    font_head: windows::Win32::Graphics::Gdi::HFONT,
    font_row: windows::Win32::Graphics::Gdi::HFONT,
    font_val: windows::Win32::Graphics::Gdi::HFONT,
    font_hint: windows::Win32::Graphics::Gdi::HFONT,
}

struct PrefsBox(Prefs);
unsafe impl Send for PrefsBox {}
unsafe impl Sync for PrefsBox {}

static PREFS_HWND: AtomicIsize = AtomicIsize::new(0);
static PREFS_BOX: std::sync::OnceLock<PrefsBox> = std::sync::OnceLock::new();
static SCROLL_Y: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowType {
    Header(&'static str),
    Toggle(&'static str, &'static str), // (id, label)
    Choice(&'static str, &'static str), // (id, label)
    Action(&'static str, &'static str), // (id, label)
}

// 完整的设置表（依次排列，绝无重叠）
const ITEMS: &[RowType] = &[
    RowType::Header("GENERAL · 常规设置"),
    RowType::Toggle("show_tooltip", "悬停显示详细信息卡片"),
    RowType::Toggle("hide_when_full", "满电时自动隐藏电池条"),
    RowType::Toggle("run_at_startup", "开机自动启动"),
    RowType::Toggle("alternate_on_click", "单击电池条切换 时间/百分比"),
    
    RowType::Header("DISPLAY · 显示与外观"),
    RowType::Choice("mode", "默认显示模式"),
    RowType::Toggle("only_percent_when_charging", "充电时只显示百分比"),
    RowType::Choice("battery_style", "电池条外观风格"),

    RowType::Header("NOTIFICATIONS · 警报与通知"),
    RowType::Toggle("notify_low", "低电量气泡通知"),
    RowType::Toggle("notify_critical", "临界电量紧急通知"),
    RowType::Toggle("notify_full", "满电完成提示"),
    RowType::Toggle("play_sound", "警报时播放系统提示音"),

    RowType::Header("WARNING LEVELS · 警告阈值"),
    RowType::Choice("low_percent", "低电量警告阈值"),
    RowType::Choice("critical_percent", "临界电量紧急阈值"),

    RowType::Header("BATTERY TOOLS · 电池工具"),
    RowType::Toggle("show_rate_in_tooltip", "详细卡片中显示功耗 (mW)"),
    RowType::Toggle("learn_profiles", "开启全寿命续航预测学习"),
    RowType::Action("reset_history", "重置充放电学习历史数据"),
];

fn total_content_height() -> i32 {
    let mut h = 16;
    for it in ITEMS {
        h += match it {
            RowType::Header(_) => HEADER_H,
            _ => ITEM_H,
        };
    }
    h + 30
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
            240,
            160,
            PREFS_W,
            PREFS_VIEW_H,
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
            font_head: render::create_ui_font(12, 700),
            font_row: render::create_ui_font(13, 500),
            font_val: render::create_ui_font(13, 600),
            font_hint: render::create_ui_font(11, 400),
        }));

        let pref = DWMWCP_ROUND.0;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const i32 as *const _, 4);

        SCROLL_Y.store(0, Ordering::Relaxed);
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
                paint_prefs(&boxed.0, hdc);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        0x020A /* WM_MOUSEWHEEL */ => {
            // 鼠标滚轮平滑滑动
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let current = SCROLL_Y.load(Ordering::Relaxed);
            let total_h = total_content_height();
            let max_scroll = (total_h - PREFS_VIEW_H + 50).max(0);
            let next = (current - (delta / 120) * 40).clamp(0, max_scroll);
            SCROLL_Y.store(next, Ordering::Relaxed);
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ => {
            let _x = (lparam.0 & 0xFFFF) as i32;
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
    let mut cur_y = 12 - scroll;

    let mut hit_id: Option<&'static str> = None;
    for it in ITEMS {
        match it {
            RowType::Header(_) => {
                cur_y += HEADER_H;
            }
            RowType::Toggle(id, _) | RowType::Choice(id, _) | RowType::Action(id, _) => {
                if click_y >= cur_y && click_y < cur_y + ITEM_H {
                    hit_id = Some(*id);
                    break;
                }
                cur_y += ITEM_H;
            }
        }
    }

    let Some(id) = hit_id else { return };
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
                    DisplayMode::Percent => DisplayMode::Icon,
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
    let mut rc = RECT { left: 0, top: 0, right: PREFS_W, bottom: PREFS_VIEW_H };
    FillRect(hdc, &rc, CreateSolidBrush(pal.bg));
    render::set_bk_transparent(hdc);

    let cfg = crate::config_lock();
    let c = cfg.lock().unwrap();
    let scroll = SCROLL_Y.load(Ordering::Relaxed);

    let mut cur_y = 14 - scroll;
    let right_limit = PREFS_W - 24;

    for it in ITEMS {
        match it {
            RowType::Header(title) => {
                // 仅当可见时绘制
                if cur_y + HEADER_H >= 0 && cur_y <= PREFS_VIEW_H {
                    // 分隔线
                    let pen = CreatePen(PS_SOLID, 1, render::rgb(42, 50, 60));
                    let old_p = SelectObject(hdc, pen.into());
                    let _ = MoveToEx(hdc, 16, cur_y + 4, None);
                    let _ = LineTo(hdc, right_limit, cur_y + 4);
                    SelectObject(hdc, old_p);
                    let _ = DeleteObject(HGDIOBJ(pen.0));

                    // 分组大标题
                    SetTextColor(hdc, render::rgb(82, 148, 226)); // 高亮蓝
                    SelectObject(hdc, p.font_head.into());
                    let mut t: Vec<u16> = title.encode_utf16().collect();
                    let mut trc = RECT { left: 16, top: cur_y + 8, right: right_limit, bottom: cur_y + HEADER_H };
                    DrawTextW(hdc, &mut t, &mut trc, DT_NOPREFIX | DT_SINGLELINE);
                }
                cur_y += HEADER_H;
            }
            RowType::Toggle(id, label) => {
                if cur_y + ITEM_H >= 0 && cur_y <= PREFS_VIEW_H {
                    // 标签文本
                    SetTextColor(hdc, pal.text);
                    SelectObject(hdc, p.font_row.into());
                    let mut lbl: Vec<u16> = label.encode_utf16().collect();
                    let mut lrc = RECT { left: 20, top: cur_y, right: right_limit - 50, bottom: cur_y + ITEM_H };
                    DrawTextW(hdc, &mut lbl, &mut lrc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_VCENTER);

                    // 开关状态
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

                    // 现代滑动胶囊开关
                    let sw_w = 40i32;
                    let sw_h = 20i32;
                    let sx = right_limit - sw_w;
                    let sy = cur_y + (ITEM_H - sw_h) / 2;
                    let fill_c = if on { render::rgb(51, 153, 236) } else { render::rgb(42, 50, 60) };
                    render::fill_round(hdc, sx, sy, sx + sw_w, sy + sw_h, 10, fill_c);
                    let dot_r = 14i32;
                    let dx = if on { sx + sw_w - dot_r - 3 } else { sx + 3 };
                    render::fill_round(hdc, dx, sy + 3, dx + dot_r, sy + 3 + dot_r, 7, render::rgb(255, 255, 255));
                }
                cur_y += ITEM_H;
            }
            RowType::Choice(id, label) => {
                if cur_y + ITEM_H >= 0 && cur_y <= PREFS_VIEW_H {
                    SetTextColor(hdc, pal.text);
                    SelectObject(hdc, p.font_row.into());
                    let mut lbl: Vec<u16> = label.encode_utf16().collect();
                    let mut lrc = RECT { left: 20, top: cur_y, right: right_limit - 110, bottom: cur_y + ITEM_H };
                    DrawTextW(hdc, &mut lbl, &mut lrc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_VCENTER);

                    let val_str = match *id {
                        "mode" => match c.default_mode {
                            DisplayMode::TimeRemaining => "时间剩余",
                            DisplayMode::Percent => "电量百分比",
                            DisplayMode::Icon => "小图标模式",
                        },
                        "battery_style" => if c.battery_style == 0 { "经典 Aero Glass" } else { "现代 Fluent" },
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

                    SetTextColor(hdc, render::rgb(51, 153, 236));
                    SelectObject(hdc, p.font_val.into());
                    let mut v: Vec<u16> = val_str.encode_utf16().collect();
                    let mut vrc = RECT { left: right_limit - 130, top: cur_y, right: right_limit, bottom: cur_y + ITEM_H };
                    DrawTextW(hdc, &mut v, &mut vrc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_VCENTER | windows::Win32::Graphics::Gdi::DT_RIGHT);
                }
                cur_y += ITEM_H;
            }
            RowType::Action(_, label) => {
                if cur_y + ITEM_H >= 0 && cur_y <= PREFS_VIEW_H {
                    SetTextColor(hdc, render::rgb(240, 140, 60));
                    SelectObject(hdc, p.font_row.into());
                    let mut lbl: Vec<u16> = format!("⚙ {}", label).encode_utf16().collect();
                    let mut lrc = RECT { left: 20, top: cur_y, right: right_limit, bottom: cur_y + ITEM_H };
                    DrawTextW(hdc, &mut lbl, &mut lrc, DT_NOPREFIX | DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_VCENTER);
                }
                cur_y += ITEM_H;
            }
        }
    }

    // 绘制平滑滚动条（半透明细长小滚动槽与滑块）
    let total_h = total_content_height();
    if total_h > PREFS_VIEW_H {
        let bar_x = PREFS_W - 8;
        let track_h = PREFS_VIEW_H - 20;
        let thumb_h = ((PREFS_VIEW_H as f32 / total_h as f32) * track_h as f32).max(24.0) as i32;
        let max_scroll = total_h - PREFS_VIEW_H + 50;
        let thumb_y = 10 + ((scroll as f32 / max_scroll as f32) * (track_h - thumb_h) as f32) as i32;
        render::fill_round(hdc, bar_x, thumb_y, bar_x + 4, thumb_y + thumb_h, 2, render::rgb(70, 85, 100));
    }
}
