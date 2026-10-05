// batterybar-modern — 任务栏定位与嵌入
//
// 嵌入策略（TrafficMonitor 同思路）：
//   Shell_TrayWnd（主）/ Shell_SecondaryTrayWnd（副屏）→ SetParent 子窗口进任务栏，
//   定位到托盘区左侧。无符号代码在 Win11 上稳定的「真·任务栏内」方案，
//   不依赖 COM deskband（Deskband 在 Win11 22H2+ 已被微软移除支持）。
#![allow(non_snake_case)]

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClientRect, GetWindowRect, IsWindow, SetParent, SetWindowPos,
    IsWindowVisible, SWP_NOACTIVATE, SWP_NOZORDER,
};
use windows_core::{PCWSTR, w};

pub struct TaskbarInfo {
    pub hwnd: HWND,
    pub rect: RECT,
}

/// 枚举任务栏（主 + 副屏），返回 (hwnd, rect)
pub fn find_taskbars() -> Vec<TaskbarInfo> {
    unsafe {
        let mut out = Vec::new();
        let class: PCWSTR = w!("Shell_TrayWnd");
        if let Ok(main) = FindWindowW(class, PCWSTR::null()) {
            let mut rc = RECT::default();
            if GetWindowRect(main, &mut rc).is_ok() {
                out.push(TaskbarInfo { hwnd: main, rect: rc });
            }
        }
        let sec: PCWSTR = w!("Shell_SecondaryTrayWnd");
        if let Ok(sec_bar) = FindWindowW(sec, PCWSTR::null()) {
            let mut rc = RECT::default();
            if GetWindowRect(sec_bar, &mut rc).is_ok() {
                out.push(TaskbarInfo { hwnd: sec_bar, rect: rc });
            }
        }
        out
    }
}

/// 返回 (x, cy)：x 为条应放置的左边缘（任务栏客户区坐标），cy 为任务栏高度。
/// Win10 结构：ReBarWindow32 的右边缘即托盘左缘。
/// Win11 结构：TrayNotifyWnd 的左边缘。
pub fn tray_left_estimate(taskbar: HWND) -> (i32, i32) {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(taskbar, &mut rc);
        let cy = rc.bottom;

        let mut tb = RECT::default();
        let _ = GetWindowRect(taskbar, &mut tb);

        let rebar: PCWSTR = w!("ReBarWindow32");
        if let Ok(h) = FindWindowExW(Some(taskbar), None, rebar, PCWSTR::null()) {
            let mut rr = RECT::default();
            if GetWindowRect(h, &mut rr).is_ok() {
                return (rr.right - tb.left, cy);
            }
        }

        let tray: PCWSTR = w!("TrayNotifyWnd");
        if let Ok(h) = FindWindowExW(Some(taskbar), None, tray, PCWSTR::null()) {
            let mut rr = RECT::default();
            if GetWindowRect(h, &mut rr).is_ok() {
                return (rr.left - tb.left, cy);
            }
        }

        // 兜底：右端留 200 物理像素
        (rc.right - 200, cy)
    }
}

pub fn still_valid(hwnd: HWND) -> bool {
    unsafe { !hwnd.is_invalid() && IsWindow(Some(hwnd)).as_bool() }
}

pub fn visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn reparent(child: HWND, taskbar: HWND) -> windows_core::Result<()> {
    unsafe { SetParent(child, Some(taskbar))? };
    Ok(())
}

pub fn place(child: HWND, x: i32, y: i32, cx: i32, cy: i32) -> windows_core::Result<()> {
    unsafe { SetWindowPos(child, None, x, y, cx, cy, SWP_NOZORDER | SWP_NOACTIVATE)? };
    Ok(())
}
