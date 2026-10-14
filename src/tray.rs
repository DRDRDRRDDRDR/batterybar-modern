// batterybar-modern — 系统托盘图标（NOTIFYICONDATAW）
#![allow(non_snake_case)]

use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    ExtractIconW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};
use windows_core::{w, PCWSTR};

pub const TRAY_CALLBACK: u32 = 0xB00B; // 自定义回调消息
const TRAY_ID: u32 = 1;

// ---- 自定义提示音 ----
// windows-rs 0.62 没有导出 PlaySoundW，这里直接声明 winmm 的导入。
// SND_ASYNC：立即返回，不阻塞调用线程（电池回调线程不能卡住）。
const SND_ASYNC: u32 = 0x0001;
const SND_NODEFAULT: u32 = 0x0002;
const SND_FILENAME: u32 = 0x0002_0000;

#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(pszsound: *const u16, hmod: isize, fdwsound: u32) -> i32;
}

/// 播放用户指定的自定义提示音（Notifications 页「Use custom sound」）。
pub fn play_file(path: &str) {
    let mut w: Vec<u16> = path.encode_utf16().collect();
    w.push(0);
    unsafe {
        let _ = PlaySoundW(w.as_ptr(), 0, SND_FILENAME | SND_ASYNC | SND_NODEFAULT);
    }
}

/// Notifications 页「Beep the system speaker」。
/// 这是主板蜂鸣器的 Beep()，与 MessageBeep（播放系统声音方案里的 wav）不是一回事。
pub fn beep() {
    use windows::Win32::System::Diagnostics::Debug::Beep;
    unsafe {
        let _ = Beep(880, 180);
    }
}

pub struct TrayIcon {
    pub hwnd: HWND,
}

impl TrayIcon {
    /// shell32 电池图标（索引 41，Win10/11 实测），失败回落应用默认图标。
    pub fn new(hwnd: HWND) -> windows_core::Result<Self> {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = TRAY_CALLBACK;

        unsafe {
            let hinstance = GetModuleHandleW(PCWSTR::null())?;
            let shell = match windows::Win32::System::LibraryLoader::LoadLibraryW(w!("shell32.dll")) {
                Ok(h) => h,
                Err(_) => Default::default(),
            };
            let hicon = if !shell.is_invalid() {
                ExtractIconW(Some(hinstance.into()), w!("shell32.dll"), 41)
            } else {
                Default::default()
            };
            nid.hIcon = if hicon.is_invalid() {
                LoadIconW(Some(hinstance.into()), IDI_APPLICATION).unwrap_or_default()
            } else {
                hicon
            };
            if !shell.is_invalid() {
                let _ = windows::Win32::Foundation::FreeLibrary(shell);
            }
            // tooltip（含 NUL 结尾，上限 128）
            let tip: Vec<u16> = "BatteryBar Modern\0".encode_utf16().collect();
            for (i, c) in tip.iter().take(128).enumerate() {
                nid.szTip[i] = *c;
            }
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
        Ok(Self { hwnd })
    }

    pub fn update_tooltip(&self, text: &str) {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_TIP;
        let mut tip = text.encode_utf16().collect::<Vec<u16>>();
        tip.push(0);
        tip.truncate(128);
        for (i, c) in tip.iter().enumerate() {
            nid.szTip[i] = *c;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// 播放系统提示音（低电/临界/自定义警报时用）
    pub fn sound(&self) {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONEXCLAMATION, MB_ICONWARNING};
        // MB_ICONWARNING == MB_ICONEXCLAMATION == MESSAGEBOX_STYLE(0x30)；连响两声更像警报
        unsafe {
            let _ = MessageBeep(MB_ICONWARNING);
            let _ = MessageBeep(MB_ICONEXCLAMATION);
        }
    }

    pub fn remove(&self) {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    /// 气泡通知（低电/临界/满电）
    pub fn balloon(&self, title: &str, text: &str, flag: windows::Win32::UI::Shell::NOTIFY_ICON_INFOTIP_FLAGS) {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = windows::Win32::UI::Shell::NIF_INFO;
        nid.dwInfoFlags = flag;
        let mut t = title.encode_utf16().collect::<Vec<u16>>();
        t.push(0);
        t.truncate(64);
        for (i, c) in t.iter().enumerate() {
            nid.szInfoTitle[i] = *c;
        }
        let mut txt = text.encode_utf16().collect::<Vec<u16>>();
        txt.push(0);
        txt.truncate(256);
        for (i, c) in txt.iter().enumerate() {
            nid.szInfo[i] = *c;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}