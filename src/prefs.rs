// batterybar-modern — Preferences 设置中心
//
// 复刻原版 BatteryBar Pro Preferences 的分类分页结构。原版有 6 个分类页：
//   General / Display / Notifications / Warning Levels / Power Saving / Tools
// 左侧是分类导航，右侧是该分类的设置卡片。详见 docs/RESEARCH-original-batterybar.md。
//
// 滚动失效根因（v0.5.0 修复）：BBM_Bar 是 Shell_TrayWnd 的**子窗口，子窗口拿不到键盘焦点**，
// 从任务栏右键菜单打开的 BBM_Prefs 从未成为前台/焦点窗口；而 WM_MOUSEWHEEL 只投递给
// **焦点窗口**，所以滚轮永远到不了本面板。三重修复：
//   ① ShowWindow 后 SetForegroundWindow + WM_MOUSEACTIVATE 返回 MA_ACTIVATE（正常路径）；
//   ② BBM_Bar / BBM_MsgHost 的 WM_MOUSEWHEEL 里做光标命中判定后转发到 forward_wheel()（兜底）；
//   ③ 滚动条可拖拽 + 键盘 PageUp/PageDown/上下/Home/End。
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, LineTo,
    MoveToEx, SelectObject, SetTextColor, DT_END_ELLIPSIS, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, HDC, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, SetForegroundWindow,
    ShowWindow, SW_SHOW, WS_CAPTION, WS_EX_TOOLWINDOW, WS_SYSMENU,
};
use windows_core::{w, PCWSTR};

use crate::config::{
    Config, DisplayMode, DISPLAY_CAPACITY, DISPLAY_FINISHED, DISPLAY_PERCENT, DISPLAY_RATE,
    DISPLAY_TIME,
};
use crate::render;

const PREFS_W: i32 = 700;
const PREFS_VIEW_H: i32 = 560;
// 侧栏要放下「警告阈值 · Warning Levels」这类双语音节，178px 会把英文截掉。
const SIDEBAR_W: i32 = 214;
const CONTENT_L: i32 = SIDEBAR_W + 18;
const CONTENT_R: i32 = PREFS_W - 26;
// 38px 时英文标签与下方中文说明的矩形会相交，叠成一团；42px 才能分成上下两行。
const ROW_H: i32 = 42;
const HEAD_H: i32 = 30;
const WHEEL_STEP: i32 = 45;
const SCROLLBAR_W: i32 = 10;

struct Prefs {
    palette: render::Palette,
    font_tab: windows::Win32::Graphics::Gdi::HFONT,
    font_section: windows::Win32::Graphics::Gdi::HFONT,
    font_label: windows::Win32::Graphics::Gdi::HFONT,
    font_value: windows::Win32::Graphics::Gdi::HFONT,
    font_note: windows::Win32::Graphics::Gdi::HFONT,
}

struct PrefsBox(Prefs);
unsafe impl Send for PrefsBox {}
unsafe impl Sync for PrefsBox {}

static PREFS_HWND: AtomicIsize = AtomicIsize::new(0);
static PREFS_BOX: std::sync::OnceLock<PrefsBox> = std::sync::OnceLock::new();
static SCROLL_Y: AtomicI32 = AtomicI32::new(0);
static TAB: AtomicUsize = AtomicUsize::new(0);
/// 拖拽滚动条中：Some(抓取时的 thumb 顶端偏移)
static DRAG_OFFSET: AtomicI32 = AtomicI32::new(-1);

// ---------------- 设置页描述 ----------------

#[derive(Clone, Copy)]
enum Item {
    /// 组内小标题（缩进、灰色、不可点）
    Head(&'static str),
    /// 开关
    Toggle(&'static str, &'static str, &'static str), // (id, label, note)
    /// 复选位（Display 页「Information to Display on the Toolbar」）
    Bit(u32, &'static str),
    /// 多档选择
    Choice(&'static str, &'static str, &'static str),
    /// 触发动作
    Action(&'static str, &'static str),
}

impl Item {
    fn h(&self) -> i32 {
        match self {
            Item::Head(_) => HEAD_H,
            _ => ROW_H,
        }
    }
}

struct Group {
    title: &'static str,
    items: &'static [Item],
}

struct Page {
    key: &'static str,
    nav: &'static str,
    title: &'static str,
    groups: &'static [Group],
}

const PAGES: &[Page] = &[
    Page {
        key: "general",
        nav: "通用 · General",
        title: "General Settings",
        groups: &[Group {
            title: "常规",
            items: &[
                Item::Toggle("show_tooltip", "Enable status tooltip", "悬停电池条时显示详细信息卡片"),
                Item::Toggle("statistical_estimation", "Use statistical time estimation", "用历史放电数据估算剩余时间（比瞬时速率稳）"),
                Item::Toggle("profiles_per_scheme", "Use separate battery profiles per power scheme", "按电源计划分别记录电池曲线"),
                Item::Toggle("alternate_battery_id", "Use alternate method to determine unique battery ID", "双电池机器识别不准时打开"),
                Item::Toggle("hide_when_full", "Hide BatteryBar when the battery is fully charged", ""),
                Item::Toggle("force_rate_estimation", "Force estimation of discharge/recharge rates", "机器不上报速率时用容量变化估算"),
                Item::Toggle("run_at_startup", "开机随 Windows 启动", "写入 HKCU\\...\\Run"),
            ],
        }],
    },
    Page {
        key: "display",
        nav: "显示 · Display",
        title: "Display Settings",
        groups: &[
            Group {
                title: "Information to Display on the Toolbar",
                items: &[
                    Item::Head("勾选的项目会出现在电池条上；单击电池条在勾选项之间轮播"),
                    Item::Bit(DISPLAY_TIME, "Time Remaining（剩余时间）"),
                    Item::Bit(DISPLAY_PERCENT, "Percent Remaining（电量百分比）"),
                    Item::Bit(DISPLAY_FINISHED, "Time Finished（充满/放空的当日时刻）"),
                    Item::Bit(DISPLAY_RATE, "Discharge/Charge Rate（充放电功率）"),
                    Item::Bit(DISPLAY_CAPACITY, "Capacity Remaining（剩余容量）"),
                ],
            },
            Group {
                title: "外观",
                items: &[
                    Item::Choice("mode", "Default display", "启动时先显示哪一项"),
                    Item::Toggle("alternate_on_click", "单击电池条在勾选项之间轮播", ""),
                    Item::Toggle("only_percent_when_charging", "Only show percent remaining when battery is charged", "充电时只显示百分比"),
                    Item::Toggle("dual_battery_popups", "Show individual battery status popups for dual batteries", ""),
                    Item::Toggle("show_battery_id", "Display the battery ID in the status popup", ""),
                    Item::Choice("font_size", "BatteryBar font", "条上文字字号"),
                    Item::Choice("percent_precision", "Percentage precision", "百分比小数位数"),
                    Item::Choice("theme", "Theme", "电池外观主题，含外部 theme.xml 主题"),
                    Item::Choice("theme_index", "External theme", "外部主题序号"),
                    Item::Toggle("text_shadow", "条上文字加 1px 阴影", "底色浅时更易读"),
                    Item::Choice("bar_opacity", "Bar background opacity", "条背景不透明度"),
                    Item::Choice("bar_width", "Horizontal bar width", "仅横排任务栏生效"),
                    Item::Choice("on_click_mode", "左键单击电池条", ""),
                    Item::Toggle("hide_on_ac", "接通电源时隐藏电池条", ""),
                ],
            },
        ],
    },
    Page {
        key: "notifications",
        nav: "通知 · Notifications",
        title: "Notifications",
        groups: &[
            Group {
                title: "Critical battery warning",
                items: &[
                    Item::Toggle("crit_audio", "Audio alert", "播放提示音"),
                    Item::Toggle("crit_popup", "Show popup", "弹气泡通知"),
                    Item::Toggle("crit_custom", "Use custom sound", "改用自定义音频文件"),
                    Item::Action("crit_sound_pick", "选择自定义提示音文件…"),
                    Item::Toggle("crit_beep", "Beep the system speaker", "主板蜂鸣器响一声"),
                    Item::Toggle("crit_run", "Run a program", "警报时运行外部程序"),
                    Item::Action("crit_program_pick", "选择要运行的程序…"),
                ],
            },
            Group {
                title: "Low battery warning",
                items: &[
                    Item::Toggle("low_audio", "Audio alert", "播放提示音"),
                    Item::Toggle("low_popup", "Show popup", "弹气泡通知"),
                    Item::Toggle("low_custom", "Use custom sound", "改用自定义音频文件"),
                    Item::Action("low_sound_pick", "选择自定义提示音文件…"),
                    Item::Toggle("low_beep", "Beep the system speaker", "主板蜂鸣器响一声"),
                    Item::Toggle("low_run", "Run a program", "警报时运行外部程序"),
                    Item::Action("low_program_pick", "选择要运行的程序…"),
                ],
            },
            Group {
                title: "Finished charging",
                items: &[
                    Item::Toggle("full_audio", "Audio alert", "播放提示音"),
                    Item::Toggle("full_popup", "Show popup", "弹气泡通知"),
                    Item::Toggle("full_custom", "Use custom sound", "改用自定义音频文件"),
                    Item::Action("full_sound_pick", "选择自定义提示音文件…"),
                    Item::Toggle("full_beep", "Beep the system speaker", "主板蜂鸣器响一声"),
                    Item::Toggle("full_run", "Run a program", "警报时运行外部程序"),
                    Item::Action("full_program_pick", "选择要运行的程序…"),
                ],
            },
            Group {
                title: "其它",
                items: &[
                    Item::Toggle("auto_close_notifications", "Automatically close notifications", "气泡自动消失"),
                    Item::Choice("auto_close_seconds", "关闭延迟", "秒"),
                ],
            },
        ],
    },
    Page {
        key: "levels",
        nav: "警告阈值 · Levels",
        title: "Warning Levels",
        groups: &[Group {
            title: "Low and Critical battery levels defined by",
            items: &[
                Item::Choice("levels_by_time", "判定依据", "按百分比或按剩余时间"),
                Item::Choice("low_percent", "Low battery level", "低电量阈值"),
                Item::Choice("low_minutes", "Low battery level（时间）", "分钟"),
                Item::Choice("critical_percent", "Critical battery level", "临界电量阈值"),
                Item::Choice("critical_minutes", "Critical battery level（时间）", "分钟"),
                Item::Choice("soft_min_percent", "Soft minimum level", "计算 Full Runtime 的「空」下限"),
                Item::Choice("full_percent", "Full battery level", "认定为「满」的电平"),
                Item::Choice("alarm_percent", "额外自定义警报阈值", "0 = 关闭"),
                Item::Choice("critical_action", "临界电量自动动作", "仅在真的掉到临界值时执行"),
            ],
        }],
    },
    Page {
        key: "powersaving",
        nav: "电源 · Power Saving",
        title: "Power Saving",
        groups: &[
            Group {
                title: "On Battery…",
                items: &[
                    Item::Toggle("battery_change_scheme", "Change power scheme", "切到电池供电时切换电源计划"),
                    Item::Choice("battery_scheme", "目标电源计划", ""),
                    Item::Toggle("battery_disable_aero", "Disable Aero", "电池供电时关闭透明效果"),
                    Item::Toggle("battery_only_below", "Only when below", "仅当电量低于阈值时执行"),
                    Item::Choice("battery_below_percent", "阈值", "%"),
                ],
            },
            Group {
                title: "Plugged In…",
                items: &[
                    Item::Toggle("ac_change_scheme", "Change power scheme", "接上电源时切换电源计划"),
                    Item::Choice("ac_scheme", "目标电源计划", ""),
                ],
            },
        ],
    },
    Page {
        key: "tools",
        nav: "工具 · Tools",
        title: "Tools",
        groups: &[
            Group {
                title: "Battery Profile Graph",
                items: &[
                    Item::Action("open_graph", "打开电池曲线窗口…"),
                    Item::Action("reset_profile", "清空曲线采样数据"),
                    Item::Toggle("log_battery_data", "Log battery data", "写入 BatteryStatus.log"),
                    Item::Toggle("show_rate_in_tooltip", "Show discharge rate in the status popup", "详情卡里显示实时功耗行"),
                ],
            },
            Group {
                title: "主题",
                items: &[
                    Item::Action("theme_folder", "打开主题文件夹…"),
                    Item::Action("theme_reload", "重新扫描外部主题"),
                ],
            },
            Group {
                title: "维护",
                items: &[
                    Item::Action("reset_history", "重置充放电学习历史记录"),
                    Item::Action("reset_config", "恢复全部设置为默认值"),
                    Item::Action("open_folder", "打开程序所在文件夹"),
                ],
            },
        ],
    },
];

fn page(i: usize) -> &'static Page {
    &PAGES[i.min(PAGES.len() - 1)]
}

fn page_height(p: &Page) -> i32 {
    let mut h = 8;
    for g in p.groups {
        h += 26; // group title
        h += 8;
        for it in g.items {
            h += it.h();
        }
        h += 8 + 18;
    }
    h + 12
}

fn max_scroll() -> i32 {
    let h = page_height(page(TAB.load(Ordering::Relaxed)));
    (h - (PREFS_VIEW_H - 76)).max(0)
}

fn clamp_scroll(v: i32) -> i32 {
    v.clamp(0, max_scroll())
}

// ---------------- 对外接口 ----------------

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
        let steps = (wheel_delta / 120).clamp(-8, 8);
        if steps == 0 {
            return;
        }
        let next = clamp_scroll(SCROLL_Y.load(Ordering::Relaxed) - steps * WHEEL_STEP);
        SCROLL_Y.store(next, Ordering::Relaxed);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

/// 跳到指定分类页（供托盘/命令行 --prefs=tools 之类使用）
pub fn show_page(key: &str) {
    if let Some(i) = PAGES.iter().position(|p| p.key == key) {
        TAB.store(i, Ordering::Relaxed);
        SCROLL_Y.store(0, Ordering::Relaxed);
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
        let palette = if render::system_prefers_dark() {
            render::palette_dark()
        } else {
            render::palette_light()
        };

        // 客户区精确 = PREFS_W x PREFS_VIEW_H（此前窗口高直接写 PREFS_VIEW_H，
        // 标题栏被算进总高 → 客户区只有 ~485px，底部卡片被裁且滚动算错）
        let style = WS_CAPTION | WS_SYSMENU;
        let mut wr = RECT { left: 0, top: 0, right: PREFS_W, bottom: PREFS_VIEW_H };
        let _ = AdjustWindowRectEx(&mut wr, style, false, WS_EX_TOOLWINDOW);

        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            w!("BatteryBar · Preferences"),
            style,
            220,
            110,
            wr.right - wr.left,
            wr.bottom - wr.top,
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
            // 负高度 = 字符高度（不含内部行距）。改用真实 Segoe UI 后，正的
            // 单元格高度会让字面明显偏小（Segoe UI 的内部行距比原先被替换的
            // 点阵字体大得多），所以这里统一改负值。
            font_tab: render::create_ui_font(-15, 500),
            font_section: render::create_ui_font(-13, 600),
            font_label: render::create_ui_font(-14, 400),
            font_value: render::create_ui_font(-14, 600),
            font_note: render::create_ui_font(-12, 400),
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
    let view_h = PREFS_VIEW_H - 76;
    let track_t = 70;
    let track_h = view_h - 8;
    let total_h = page_height(page(TAB.load(Ordering::Relaxed)));
    let ratio = if total_h > view_h {
        (view_h as f32 / total_h as f32).clamp(0.08, 1.0)
    } else {
        1.0
    };
    let thumb_h = ((track_h as f32) * ratio).max(28.0).min(track_h as f32) as i32;
    let ms = max_scroll();
    let frac = if ms > 0 { (scroll as f32 / ms as f32).clamp(0.0, 1.0) } else { 0.0 };
    let thumb_top = track_t + ((track_h - thumb_h) as f32 * frac) as i32;
    (thumb_top, thumb_h, track_h)
}

// ---------------- 窗口过程 ----------------

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
            let steps = (delta / 120).clamp(-8, 8);
            scroll_by(-steps * WHEEL_STEP);
            LRESULT(0)
        }
        0x0201 /* WM_LBUTTONDOWN */ => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if max_scroll() > 0 && x >= PREFS_W - 8 - SCROLLBAR_W && y >= 62 {
                let (thumb_top, _, _) = scrollbar_geom(SCROLL_Y.load(Ordering::Relaxed));
                DRAG_OFFSET.store(y - thumb_top, Ordering::Relaxed);
                let _ = SetCapture(hwnd);
                LRESULT(0)
            } else {
                // 让本窗口拿到键盘焦点：WM_MOUSEWHEEL 只投递给焦点窗口，
                // 设置面板是从任务栏（子窗口，永远无焦点）用菜单打开的，不设焦点则滚轮无效。
                let _ = SetFocus(Some(hwnd));
                handle_click(hwnd, x, y);
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
        0x0200 /* WM_MOUSEMOVE */ => {
            if DRAG_OFFSET.load(Ordering::Relaxed) >= 0 && max_scroll() > 0 {
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let grab = DRAG_OFFSET.load(Ordering::Relaxed);
                let (_, thumb_h, track_h) = scrollbar_geom(SCROLL_Y.load(Ordering::Relaxed));
                let usable = (track_h - thumb_h).max(1);
                let mut frac = (y - grab - 70) as f32 / usable as f32;
                frac = frac.clamp(0.0, 1.0);
                let next = clamp_scroll((frac * max_scroll() as f32) as i32);
                if next != SCROLL_Y.load(Ordering::Relaxed) {
                    SCROLL_Y.store(next, Ordering::Relaxed);
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        0x0100 /* WM_KEYDOWN */ => {
            // 数字键 1..6 直接跳分类页
            if (0x31..=0x36).contains(&(wparam.0 as u32)) {
                let idx = (wparam.0 as usize) - 0x31;
                if idx < PAGES.len() {
                    TAB.store(idx, Ordering::Relaxed);
                    SCROLL_Y.store(0, Ordering::Relaxed);
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            match wparam.0 as u32 {
                0x21 /* VK_PRIOR */ => scroll_by(-(PREFS_VIEW_H - 140)),
                0x22 /* VK_NEXT */ => scroll_by(PREFS_VIEW_H - 140),
                0x26 /* VK_UP */ => scroll_by(-WHEEL_STEP),
                0x28 /* VK_DOWN */ => scroll_by(WHEEL_STEP),
                0x24 /* VK_HOME */ => scroll_by(-max_scroll()),
                0x23 /* VK_END */ => scroll_by(max_scroll()),
                0x09 /* VK_TAB */ => {
                    let n = PAGES.len();
                    TAB.store((TAB.load(Ordering::Relaxed) + 1) % n, Ordering::Relaxed);
                    SCROLL_Y.store(0, Ordering::Relaxed);
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
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

// ---------------- 命中测试与点击处理 ----------------

/// 侧栏分类项矩形（含顶部 62px 的标题带）
fn nav_rect(i: usize) -> RECT {
    let top = 70 + i as i32 * 44;
    RECT { left: 0, top, right: SIDEBAR_W, bottom: top + 40 }
}

unsafe fn handle_click(hwnd: HWND, x: i32, y: i32) {
    // 1) 侧栏切换分类
    if x < SIDEBAR_W {
        for i in 0..PAGES.len() {
            let r = nav_rect(i);
            if y >= r.top && y < r.bottom {
                TAB.store(i, Ordering::Relaxed);
                SCROLL_Y.store(0, Ordering::Relaxed);
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                return;
            }
        }
        return;
    }

    // 2) 内容区命中
    let scroll = SCROLL_Y.load(Ordering::Relaxed);
    let p = page(TAB.load(Ordering::Relaxed));
    let mut cur_y = 70 - scroll;
    let content_r = CONTENT_R;

    let mut hit_item: Option<Item> = None;
    'outer: for g in p.groups {
        cur_y += 26 + 8;
        for it in g.items {
            let h = it.h();
            if y >= cur_y && y < cur_y + h {
                hit_item = Some(*it);
                break 'outer;
            }
            cur_y += h;
        }
        cur_y += 8 + 18;
    }
    let _ = content_r;
    let Some(item) = hit_item else { return };

    let mut save = false;
    // 该设置项是否影响条的几何尺寸（宽度/显隐）—— 是则改完必须重新布局，
    // 光 InvalidateRect 重绘不够：窗口尺寸没变，新宽度不会生效。
    let mut need_relayout = false;
    let mut need_repaint = true;
    {
        let cfg = crate::config_lock();
        let mut c = cfg.lock().unwrap();
        match item {
            Item::Bit(bit, _) => {
                c.display_items ^= bit;
                // 勾空了就把显示模式落回第一个仍勾选的项目；一个都没勾 → 图标模式
                let still = mode_bit(crate::mode_now_pub()) & c.display_items != 0;
                if !still {
                    crate::set_mode(first_checked(&c));
                }
                save = true;
            }
            Item::Action(id, _) => match id {
                "open_graph" => crate::graph::toggle_graph(),
                "reset_profile" => {
                    crate::profile::clear();
                    crate::tray_balloon("BatteryBar Modern", "已清空电池曲线采样数据。");
                }
                "reset_history" => {
                    crate::power::reset_history();
                    crate::tray_balloon("BatteryBar Modern", "已重置充放电学习记录。");
                }
                "reset_config" => {
                    *c = Config::default();
                    crate::power::LEARN_ENABLED.store(c.statistical_estimation, Ordering::Relaxed);
                    crate::set_mode(c.default_mode);
                    save = true;
                    need_relayout = true;
                }
                "open_folder" => {
                    if let Ok(mut p) = std::env::current_exe() {
                        p.pop();
                        let _ = crate::hidden_command("explorer.exe").arg(p).spawn();
                    }
                }
                "theme_folder" => crate::theme::open_folder(),
                "theme_reload" => {
                    crate::theme::reload();
                    let n = crate::theme::count();
                    crate::tray_balloon(
                        "BatteryBar Modern",
                        if n == 0 {
                            "没找到外部主题。把主题文件夹放进 themes 目录后重试。"
                        } else {
                            "已重新扫描外部主题。"
                        },
                    );
                    need_repaint = true;
                }
                "crit_sound_pick" | "low_sound_pick" | "full_sound_pick" => {
                    let which = &id[..4].to_string();
                    if let Some(path) = pick_file("选择提示音文件", "音频文件\0*.wav;*.mp3\0所有文件\0*.*\0") {
                        notif_mut(&mut c, which).custom_sound = true;
                        notif_mut(&mut c, which).sound_path = path;
                        save = true;
                    }
                }
                "crit_program_pick" | "low_program_pick" | "full_program_pick" => {
                    let which = &id[..4].to_string();
                    if let Some(path) = pick_file("选择要运行的程序", "可执行文件\0*.exe;*.bat;*.cmd\0所有文件\0*.*\0") {
                        notif_mut(&mut c, which).run_program = true;
                        notif_mut(&mut c, which).program_path = path;
                        save = true;
                    }
                }
                _ => need_repaint = false,
            },
            Item::Toggle(id, _, _) => {
                let n = notif_by_id(&mut c, id);
                if let Some(n) = n {
                    match id.rsplit('_').next().unwrap_or("") {
                        "audio" => n.audio = !n.audio,
                        "popup" => n.popup = !n.popup,
                        "custom" => n.custom_sound = !n.custom_sound,
                        "beep" => n.beep = !n.beep,
                        "run" => n.run_program = !n.run_program,
                        _ => {}
                    }
                    save = true;
                } else {
                    match id {
                        "show_tooltip" => c.show_tooltip = !c.show_tooltip,
                        "statistical_estimation" => {
                            c.statistical_estimation = !c.statistical_estimation;
                            // 立即同步到数据层：关闭后不再写学习记录、不再更新学习值
                            crate::power::LEARN_ENABLED.store(c.statistical_estimation, Ordering::Relaxed);
                        }
                        "profiles_per_scheme" => c.profiles_per_scheme = !c.profiles_per_scheme,
                        "alternate_battery_id" => c.alternate_battery_id = !c.alternate_battery_id,
                        "hide_when_full" => c.hide_when_full = !c.hide_when_full,
                        "force_rate_estimation" => c.force_rate_estimation = !c.force_rate_estimation,
                        "run_at_startup" => {
                            c.run_at_startup = !c.run_at_startup;
                            let en = c.run_at_startup;
                            let c2 = c.clone();
                            // 注册表写失败必须回滚开关状态，否则 UI 会显示一个实际不存在的自启项。
                            if !c2.set_startup(en) {
                                c.run_at_startup = !en;
                                crate::tray_balloon("无法修改开机启动项",
                                    "写入注册表自启项被系统拒绝。若程序位于受限目录，请将 exe 移到普通目录后重试。");
                            }
                        }
                        "alternate_on_click" => c.alternate_on_click = !c.alternate_on_click,
                        "only_percent_when_charging" => {
                            c.only_percent_when_charging = !c.only_percent_when_charging
                        }
                        "dual_battery_popups" => c.dual_battery_popups = !c.dual_battery_popups,
                        "show_battery_id" => c.show_battery_id = !c.show_battery_id,
                        "text_shadow" => c.text_shadow = !c.text_shadow,
                        "hide_on_ac" => {
                            c.hide_on_ac = !c.hide_on_ac;
                            need_relayout = true;
                        }
                        "auto_close_notifications" => {
                            c.auto_close_notifications = !c.auto_close_notifications
                        }
                        "battery_change_scheme" => c.battery_change_scheme = !c.battery_change_scheme,
                        "battery_disable_aero" => c.battery_disable_aero = !c.battery_disable_aero,
                        "battery_only_below" => c.battery_only_below = !c.battery_only_below,
                        "ac_change_scheme" => c.ac_change_scheme = !c.ac_change_scheme,
                        "log_battery_data" => {
                            c.log_battery_data = !c.log_battery_data;
                            crate::power::LOG_TO_FILE.store(c.log_battery_data, Ordering::Relaxed);
                        }
                        "show_rate_in_tooltip" => {
                            c.show_rate_in_tooltip = !c.show_rate_in_tooltip
                        }
                        _ => need_repaint = false,
                    }
                    save = true;
                }
            }
            Item::Choice(id, _, _) => {
                match id {
                    "mode" => {
                        c.default_mode = next_mode(c.default_mode);
                        crate::set_mode(c.default_mode);
                    }
                    "font_size" => {
                        c.font_size = (c.font_size + 1) % 3;
                        // 竖排任务栏的条高由字号决定（见 bar_height_vertical），
                        // 改字号必须重跑 relayout，否则文字行会被裁掉。
                        need_relayout = true;
                    }
                    "percent_precision" => c.percent_precision = (c.percent_precision + 1) % 3,
                    "theme" => c.battery_style = (c.battery_style + 1) % 3,
                    "theme_index" => {
                        // 外部主题数量来自 themes 文件夹，随用户增删而变
                        let n = crate::theme::count();
                        if n > 0 {
                            c.theme_index = ((c.theme_index as usize + 1) % n) as u32;
                        } else {
                            c.theme_index = 0;
                        }
                    }
                    "bar_opacity" => {
                        c.bar_opacity = match c.bar_opacity {
                            100 => 80,
                            80 => 60,
                            60 => 40,
                            _ => 100,
                        }
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
                        need_relayout = true;
                    }
                    "on_click_mode" => c.on_click_mode = if c.on_click_mode == 0 { 1 } else { 0 },
                    "levels_by_time" => c.levels_by_time = !c.levels_by_time,
                    "low_percent" => step_cycle(&mut c.low_percent, &[15, 20, 25, 30, 35, 40], 30),
                    "low_minutes" => step_cycle(&mut c.low_minutes, &[15, 30, 45, 60, 90, 120], 45),
                    "critical_percent" => step_cycle(&mut c.critical_percent, &[5, 10, 15, 20, 25], 15),
                    "critical_minutes" => step_cycle(&mut c.critical_minutes, &[5, 10, 20, 30, 45], 20),
                    "soft_min_percent" => step_cycle(&mut c.soft_min_percent, &[0, 3, 5, 7, 10], 5),
                    "full_percent" => step_cycle(&mut c.full_percent, &[80, 90, 95, 100], 100),
                    "alarm_percent" => step_cycle(&mut c.alarm_percent, &[0, 50, 40, 30, 20], 0),
                    "critical_action" => c.critical_action = (c.critical_action + 1) % 5,
                    "auto_close_seconds" => {
                        step_cycle(&mut c.auto_close_seconds, &[60, 120, 300, 600], 300)
                    }
                    "battery_scheme" => c.battery_scheme = (c.battery_scheme + 1) % 3,
                    "ac_scheme" => c.ac_scheme = (c.ac_scheme + 1) % 3,
                    "battery_below_percent" => {
                        step_cycle(&mut c.battery_below_percent, &[20, 30, 40, 50, 60, 70], 50)
                    }
                    _ => need_repaint = false,
                }
                crate::config::clamp_levels(&mut c);
                save = true;
            }
            Item::Head(_) => need_repaint = false,
        }
        if save {
            c.save();
        }
    }
    if need_relayout {
        crate::relayout_bar();
    }
    if need_repaint {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
        crate::refresh_bar();
    }
}

fn step_cycle(v: &mut i32, steps: &[i32], fallback: i32) {
    let cur = *v;
    let idx = steps.iter().position(|&x| x == cur);
    *v = match idx {
        Some(i) => steps[(i + 1) % steps.len()],
        None => fallback,
    };
}

fn next_mode(m: DisplayMode) -> DisplayMode {
    match m {
        DisplayMode::TimeRemaining => DisplayMode::Percent,
        DisplayMode::Percent => DisplayMode::TimeFinished,
        DisplayMode::TimeFinished => DisplayMode::DischargeRate,
        DisplayMode::DischargeRate => DisplayMode::Capacity,
        DisplayMode::Capacity => DisplayMode::Icon,
        DisplayMode::Icon => DisplayMode::TimeRemaining,
    }
}

fn mode_bit(m: DisplayMode) -> u32 {
    match m {
        DisplayMode::TimeRemaining => DISPLAY_TIME,
        DisplayMode::Percent => DISPLAY_PERCENT,
        DisplayMode::TimeFinished => DISPLAY_FINISHED,
        DisplayMode::DischargeRate => DISPLAY_RATE,
        DisplayMode::Capacity => DISPLAY_CAPACITY,
        DisplayMode::Icon => 0,
    }
}

fn first_checked(c: &Config) -> DisplayMode {
    for bit in [DISPLAY_TIME, DISPLAY_PERCENT, DISPLAY_FINISHED, DISPLAY_RATE, DISPLAY_CAPACITY] {
        if c.display_items & bit != 0 {
            if let Some(m) = DisplayMode::from_bit(bit) {
                return m;
            }
        }
    }
    DisplayMode::Icon
}

/// 按 id 前缀（crit/low/full）取对应的通知组。
fn notif_mut<'a>(c: &'a mut Config, which: &str) -> &'a mut crate::config::Notif {
    match which {
        "crit" => &mut c.notif_critical,
        "full" => &mut c.notif_full,
        _ => &mut c.notif_low,
    }
}

fn notif_by_id<'a>(c: &'a mut Config, id: &str) -> Option<&'a mut crate::config::Notif> {
    let (prefix, field) = id.split_once('_')?;
    if !matches!(field, "audio" | "popup" | "custom" | "beep" | "run") {
        return None;
    }
    match prefix {
        "crit" => Some(&mut c.notif_critical),
        "low" => Some(&mut c.notif_low),
        "full" => Some(&mut c.notif_full),
        _ => None,
    }
}

/// 文件选择对话框（Notifications 页的 Choose… 按钮）。
unsafe fn pick_file(title: &str, filter: &str) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    let mut buf = [0u16; 1024];
    let mut title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    // 过滤器是双 NUL 结尾的成对字符串
    let mut filter_w: Vec<u16> = filter.encode_utf16().collect();
    filter_w.push(0);

    let mut ofn = OPENFILENAMEW::default();
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFilter = windows_core::PCWSTR::from_raw(filter_w.as_ptr());
    ofn.lpstrFile = windows_core::PWSTR::from_raw(buf.as_mut_ptr());
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = windows_core::PCWSTR::from_raw(title_w.as_mut_ptr());
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;

    if !GetOpenFileNameW(&mut ofn).as_bool() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    if end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..end]))
}

// ---------------- 绘制 ----------------

unsafe fn paint_prefs(p: &Prefs, hdc: HDC) {
    let pal = &p.palette;
    let full = RECT { left: 0, top: 0, right: PREFS_W, bottom: PREFS_VIEW_H };
    let bgbrush = CreateSolidBrush(pal.bg);
    FillRect(hdc, &full, bgbrush);
    let _ = DeleteObject(bgbrush.into());
    render::set_bk_transparent(hdc);

    let cfg = crate::config_lock();
    let c = cfg.lock().unwrap();
    let scroll = SCROLL_Y.load(Ordering::Relaxed);
    let tab = TAB.load(Ordering::Relaxed);

    // ---- 左侧分类导航 ----
    let side = RECT { left: 0, top: 0, right: SIDEBAR_W, bottom: PREFS_VIEW_H };
    let sidebrush = CreateSolidBrush(pal.surface_dim);
    FillRect(hdc, &side, sidebrush);
    let _ = DeleteObject(sidebrush.into());

    let hrc = RECT { left: 18, top: 16, right: SIDEBAR_W - 12, bottom: 44 };
    text(hdc, p.font_section, "Settings", hrc, pal.text_dim, DT_SINGLELINE | DT_VCENTER);

    for (i, pg) in PAGES.iter().enumerate() {
        let r = nav_rect(i);
        let active = i == tab;
        if active {
            render::fill_round(hdc, 8, r.top, SIDEBAR_W - 10, r.bottom, 8, pal.accent_blue);
        }
        let col = if active { render::rgb(0xFF, 0xFF, 0xFF) } else { pal.text };
        // 右边界要停在序号数字之前，否则双语的导航标题会被序号压住。
        let rc = RECT { left: 20, top: r.top, right: SIDEBAR_W - 32, bottom: r.bottom };
        text(hdc, p.font_tab, pg.nav, rc, col, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        // 序号提示，键盘 1..6 可跳转分类。
        let key = format!("{}", i + 1);
        let krc = RECT { left: SIDEBAR_W - 28, top: r.top, right: SIDEBAR_W - 12, bottom: r.bottom };
        text(hdc, p.font_note, &key, krc, if active { render::rgb(0xDD, 0xEA, 0xFF) } else { pal.text_dim }, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
    }

    // ---- 右侧内容 ----
    let pg = page(tab);
    let title_rc = RECT { left: CONTENT_L, top: 16, right: CONTENT_R, bottom: 44 };
    text(hdc, p.font_tab, pg.title, title_rc, pal.text, DT_SINGLELINE | DT_VCENTER);

    let mut cur_y = 70 - scroll;
    for g in pg.groups {
        // 组标题
        if cur_y + 26 >= 60 && cur_y <= PREFS_VIEW_H {
            let grc = RECT { left: CONTENT_L, top: cur_y, right: CONTENT_R, bottom: cur_y + 22 };
            text(hdc, p.font_section, g.title, grc, pal.accent_blue, DT_SINGLELINE | DT_VCENTER);
        }
        cur_y += 26 + 8;

        let card_h: i32 = g.items.iter().map(|i| i.h()).sum::<i32>() + 8;
        if cur_y + card_h >= 60 && cur_y <= PREFS_VIEW_H {
            render::fill_round(hdc, CONTENT_L, cur_y, CONTENT_R, cur_y + card_h, 8, pal.surface);
            render::stroke_round(hdc, CONTENT_L, cur_y, CONTENT_R, cur_y + card_h, 8, pal.border);
        }

        let item_start = cur_y + 4;
        let mut row_y = item_start;
        let mut drawn_idx = 0usize;
        for it in g.items {
            let h = it.h();
            let visible = row_y + h >= 60 && row_y <= PREFS_VIEW_H - 36;
            if visible {
                match *it {
                    Item::Head(text_s) => {
                        let rc = RECT { left: CONTENT_L + 16, top: row_y, right: CONTENT_R - 16, bottom: row_y + h };
                        text(hdc, p.font_note, text_s, rc, pal.text_dim, DT_SINGLELINE | DT_VCENTER);
                    }
                    Item::Toggle(id, label, note) => {
                        draw_row_text(hdc, p, label, note, row_y, 72, pal);
                        let on = toggle_value(&c, id);
                        draw_switch(hdc, pal, row_y, on);
                    }
                    Item::Bit(bit, label) => {
                        draw_row_text(hdc, p, label, "", row_y, 72, pal);
                        draw_check(hdc, pal, row_y, c.display_items & bit != 0);
                    }
                    Item::Choice(id, label, note) => {
                        draw_row_text(hdc, p, label, note, row_y, 190, pal);
                        let val = choice_value(id, &c);
                        let vrc = RECT { left: CONTENT_R - 180, top: row_y, right: CONTENT_R - 16, bottom: row_y + ROW_H };
                        text(hdc, p.font_value, &val, vrc, pal.accent_blue, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
                    }
                    Item::Action(id, label) => {
                        if id.ends_with("_pick") {
                            let path = action_path(&c, id);
                            draw_row_text(hdc, p, label, &path, row_y, 46, pal);
                        } else {
                            draw_row_text(hdc, p, label, "", row_y, 46, pal);
                        }
                        let col = if id == "reset_config" { pal.accent_crit } else { pal.accent_warn };
                        let arc = RECT { left: CONTENT_R - 60, top: row_y, right: CONTENT_R - 16, bottom: row_y + ROW_H };
                        text(hdc, p.font_value, "›", arc, col, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
                    }
                }
                // 行间微细分割线（每个卡片内最后一行不画）
                if drawn_idx + 1 < g.items.len() && !matches!(g.items[drawn_idx + 1], Item::Head(_)) {
                    let pen = CreatePen(PS_SOLID, 1, pal.border);
                    let old_p = SelectObject(hdc, pen.into());
                    let _ = MoveToEx(hdc, CONTENT_L + 16, row_y + h, None);
                    let _ = LineTo(hdc, CONTENT_R - 16, row_y + h);
                    SelectObject(hdc, old_p);
                    let _ = DeleteObject(pen.into());
                }
            }
            row_y += h;
            drawn_idx += 1;
        }
        cur_y += card_h + 18;
    }
    let _ = cur_y;

    // ---- 滚动条 ----
    if max_scroll() > 0 {
        let (thumb_top, thumb_h, _) = scrollbar_geom(scroll);
        let bar_x = PREFS_W - 12;
        let dragging = DRAG_OFFSET.load(Ordering::Relaxed) >= 0;
        let col = if dragging { render::rgb(120, 145, 175) } else { render::rgb(65, 80, 95) };
        render::fill_round(hdc, bar_x, thumb_top, bar_x + 4, thumb_top + thumb_h, 2, col);
    }

    // ---- 底部提示 ----
    let hint = format!("{} / {}  ·  滚轮滚动  ·  Tab 或数字键切换分类", tab + 1, PAGES.len());
    // 先铺一层窗口底色再写字：内容区滚动时最后一行会滑到底部提示的位置，
    // 不铺底就会出现提示文字与设置行互相叠印。
    let hrc_bg = RECT { left: CONTENT_L - 8, top: PREFS_VIEW_H - 34, right: PREFS_W, bottom: PREFS_VIEW_H };
    render::fill_solid(hdc, &hrc_bg, pal.bg);
    let hrc2 = RECT { left: CONTENT_L, top: PREFS_VIEW_H - 30, right: CONTENT_R, bottom: PREFS_VIEW_H - 8 };
    text(hdc, p.font_note, &hint, hrc2, pal.text_dim, DT_SINGLELINE | DT_VCENTER);
    drop(c);
}

unsafe fn text(
    hdc: HDC,
    font: windows::Win32::Graphics::Gdi::HFONT,
    s: &str,
    mut rc: RECT,
    color: windows::Win32::Foundation::COLORREF,
    flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    SelectObject(hdc, font.into());
    SetTextColor(hdc, color);
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(hdc, &mut buf, &mut rc, flags | DT_NOPREFIX);
}

/// 画一行的主标签 + 次要说明。
///
/// `right_pad` 是留给右侧开关、勾选框或当前值的宽度，主标签矩形必须在这里
/// 收住，否则长英文标签会压到开关上。
///
/// 主标签与说明的矩形是上下两段，而不是整体居中后再在底部叠一行 —— 后者在
/// 38px 行高下两段纵向区间相交，中文字会直接盖在英文标签的下缘。
unsafe fn draw_row_text(
    hdc: HDC,
    p: &Prefs,
    label: &str,
    note: &str,
    row_y: i32,
    right_pad: i32,
    pal: &render::Palette,
) {
    let right = CONTENT_R - right_pad;
    let has_note = !note.is_empty();
    // 有说明时主标签占上半段，没有说明时整行居中。
    let (l_top, l_bot) = if has_note {
        (row_y + 3, row_y + ROW_H / 2 + 4)
    } else {
        (row_y, row_y + ROW_H)
    };
    let r = RECT { left: CONTENT_L + 16, top: l_top, right, bottom: l_bot };
    // DT_END_ELLIPSIS：宁可显示成省略号收尾，也不要让文字越过矩形压到右侧开关上。
    text(hdc, p.font_label, label, r, pal.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    if has_note {
        let nr = RECT { left: CONTENT_L + 16, top: row_y + ROW_H / 2 + 4, right, bottom: row_y + ROW_H - 2 };
        text(hdc, p.font_note, note, nr, pal.text_dim, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
}

unsafe fn draw_switch(hdc: HDC, pal: &render::Palette, row_y: i32, on: bool) {
    let sw_w = 40i32;
    let sw_h = 22i32;
    let sx = CONTENT_R - 16 - sw_w;
    let sy = row_y + (ROW_H - sw_h) / 2;
    let track = if on { pal.accent_blue } else { render::darken(pal.surface, 0.25) };
    render::fill_round(hdc, sx, sy, sx + sw_w, sy + sw_h, 11, track);
    let dot = 16i32;
    let dx = if on { sx + sw_w - dot - 3 } else { sx + 3 };
    render::fill_round(hdc, dx, sy + 3, dx + dot, sy + 3 + dot, 8, render::rgb(255, 255, 255));
}

unsafe fn draw_check(hdc: HDC, pal: &render::Palette, row_y: i32, on: bool) {
    let d = 20i32;
    let x = CONTENT_R - 16 - d;
    let y = row_y + (ROW_H - d) / 2;
    if on {
        render::fill_round(hdc, x, y, x + d, y + d, 5, pal.accent_blue);
        // 对勾
        let pen = CreatePen(PS_SOLID, 2, render::rgb(0xFF, 0xFF, 0xFF));
        let old = SelectObject(hdc, pen.into());
        let _ = MoveToEx(hdc, x + 5, y + 10, None);
        let _ = LineTo(hdc, x + 9, y + 14);
        let _ = LineTo(hdc, x + 15, y + 6);
        SelectObject(hdc, old);
        let _ = DeleteObject(pen.into());
    } else {
        render::stroke_round(hdc, x, y, x + d, y + d, 5, render::lighten(pal.border, 0.2));
    }
}

fn toggle_value(c: &Config, id: &str) -> bool {
    match id {
        "show_tooltip" => c.show_tooltip,
        "statistical_estimation" => c.statistical_estimation,
        "profiles_per_scheme" => c.profiles_per_scheme,
        "alternate_battery_id" => c.alternate_battery_id,
        "hide_when_full" => c.hide_when_full,
        "force_rate_estimation" => c.force_rate_estimation,
        "run_at_startup" => c.run_at_startup,
        "alternate_on_click" => c.alternate_on_click,
        "only_percent_when_charging" => c.only_percent_when_charging,
        "dual_battery_popups" => c.dual_battery_popups,
        "show_battery_id" => c.show_battery_id,
        "text_shadow" => c.text_shadow,
        "hide_on_ac" => c.hide_on_ac,
        "auto_close_notifications" => c.auto_close_notifications,
        "battery_change_scheme" => c.battery_change_scheme,
        "battery_disable_aero" => c.battery_disable_aero,
        "battery_only_below" => c.battery_only_below,
        "ac_change_scheme" => c.ac_change_scheme,
        "log_battery_data" => c.log_battery_data,
        "show_rate_in_tooltip" => c.show_rate_in_tooltip,
        _ => {
            let Some((prefix, field)) = id.split_once('_') else { return false };
            let n = match prefix {
                "crit" => &c.notif_critical,
                "low" => &c.notif_low,
                "full" => &c.notif_full,
                _ => return false,
            };
            match field {
                "audio" => n.audio,
                "popup" => n.popup,
                "custom" => n.custom_sound,
                "beep" => n.beep,
                "run" => n.run_program,
                _ => false,
            }
        }
    }
}

/// 已选路径的显示文本（未选时给一句提示）
fn action_path(c: &Config, id: &str) -> String {
    let (which, kind) = (id[..4].to_string(), id[4..].to_string());
    let n = match which.as_str() {
        "crit" => &c.notif_critical,
        "low" => &c.notif_low,
        _ => &c.notif_full,
    };
    let p = if kind.starts_with("_sound") { &n.sound_path } else { &n.program_path };
    if p.is_empty() {
        "未选择…".to_string()
    } else {
        // 只显示文件名，路径太长会顶掉标签
        std::path::Path::new(p)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.clone())
    }
}

fn choice_value(id: &str, c: &Config) -> String {
    match id {
        "mode" => match c.default_mode {
            DisplayMode::TimeRemaining => "时间剩余".into(),
            DisplayMode::Percent => "电量百分比".into(),
            DisplayMode::TimeFinished => "充满时刻".into(),
            DisplayMode::DischargeRate => "实时功率 (W)".into(),
            DisplayMode::Capacity => "剩余容量 (Wh)".into(),
            DisplayMode::Icon => "小图标模式".into(),
        },
        "font_size" => match c.font_size {
            0 => "小 13px".into(),
            1 => "中 15px".into(),
            _ => "大 17px".into(),
        },
        "percent_precision" => format!("{} 位小数", c.percent_precision),
        "theme" => match c.battery_style {
            0 => "经典拟物 Aero Glass".into(),
            1 => "现代极简 Fluent".into(),
            _ => "外部主题 theme.xml".into(),
        },
        "theme_index" => {
            if crate::theme::count() == 0 {
                crate::theme::status_line()
            } else {
                crate::theme::name_of(c.theme_index as usize)
                    .unwrap_or_else(|| format!("#{}", c.theme_index))
            }
        }
        "bar_opacity" => format!("{}%", c.bar_opacity),
        "bar_width" => format!("{} px", c.bar_width),
        "on_click_mode" => if c.on_click_mode == 0 { "轮播勾选项".into() } else { "直接弹详情卡".into() },
        "levels_by_time" => if c.levels_by_time { "剩余时间".into() } else { "电量百分比".into() },
        "low_percent" => format!("{}%", c.low_percent),
        "low_minutes" => format!("{} 分钟", c.low_minutes),
        "critical_percent" => format!("{}%", c.critical_percent),
        "critical_minutes" => format!("{} 分钟", c.critical_minutes),
        "soft_min_percent" => format!("{}%", c.soft_min_percent),
        "full_percent" => format!("{}%", c.full_percent),
        "alarm_percent" => if c.alarm_percent == 0 { "关闭".into() } else { format!("{}%", c.alarm_percent) },
        "critical_action" => match c.critical_action {
            1 => "睡眠".into(),
            2 => "关机".into(),
            3 => "休眠".into(),
            4 => "重启".into(),
            _ => "不执行".into(),
        },
        "auto_close_seconds" => format!("{} 秒", c.auto_close_seconds),
        "battery_scheme" => scheme_name(c.battery_scheme),
        "ac_scheme" => scheme_name(c.ac_scheme),
        "battery_below_percent" => format!("{}%", c.battery_below_percent),
        _ => String::new(),
    }
}

fn scheme_name(v: u32) -> String {
    match v {
        0 => "节能 Power Saver".into(),
        2 => "高性能 High Performance".into(),
        _ => "平衡 Balanced".into(),
    }
}
