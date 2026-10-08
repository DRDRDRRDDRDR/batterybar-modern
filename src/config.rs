// batterybar-modern — 配置与持久化（options.json）
// 对齐原版 BatteryBar Pro 的 Preferences 面板。
#![allow(non_snake_case)]

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    // ---- General（原版 General Settings）----
    pub show_tooltip: bool, // hover 显示详细信息
    pub hide_when_full: bool, // 满电时隐藏条
    pub run_at_startup: bool, // HKCU Run 自启

    // ---- Display（原版 Display 设置）----
    pub default_mode: DisplayMode, // 启动默认显示
    pub alternate_on_click: bool, // 点击在 时间/百分比 间切换（原版默认行为）
    pub only_percent_when_charging: bool, // 充电时只显示百分比
    pub bar_width: i32, // 横排条宽

    // ---- Theme ----
    pub dark_taskbar_tint: (u32, u32, u32), // 深色任务栏底色
    pub light_taskbar_tint: (u32, u32, u32),

    // ---- Warning Levels（原版 Warning Levels）----
    pub low_percent: i32, // 低电量阈值（默认 20）
    pub critical_percent: i32, // 临界阈值（默认 10）

    // ---- Notifications（原版 Notifications）----
    pub notify_low: bool,
    pub notify_critical: bool,
    pub notify_full: bool,
    pub play_sound: bool, // 警报播放提示音

    // ---- Tools & Profiles ----
    pub show_rate_in_tooltip: bool, // 悬停显示 mW 功耗
    pub learn_profiles: bool, // 电池全寿命学习预测
    pub battery_style: u32, // 0=经典拟物 Aero Glass, 1=现代 Fluent
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    TimeRemaining, // 原版默认
    Percent,
    Icon, // 只显示小电池图标
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_tooltip: true,
            hide_when_full: false,
            run_at_startup: false,
            default_mode: DisplayMode::TimeRemaining,
            alternate_on_click: true,
            only_percent_when_charging: false,
            bar_width: 150,
            dark_taskbar_tint: (0x20, 0x20, 0x20),
            light_taskbar_tint: (0xF3, 0xF3, 0xF3),
            low_percent: 20,
            critical_percent: 10,
            notify_low: true,
            notify_critical: true,
            notify_full: true,
            play_sound: true,
            show_rate_in_tooltip: true,
            learn_profiles: true,
            battery_style: 0,
        }
    }
}

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    p.pop();
    p.push("options.json");
    p
}

impl Config {
    /// 从 exe 同目录 options.json 读取；不存在/解析失败 → 默认值。
    /// 不引 serde：手写极简解析器（只支持平铺 "key": value）。
    pub fn load() -> Self {
        let mut c = Config::default();
        let Ok(text) = std::fs::read_to_string(config_path()) else {
            return c;
        };
        for line in text.lines() {
            let line = line.trim().trim_end_matches(',');
            let Some((k, v)) = line.split_once(':') else { continue };
            let k = k.trim().trim_matches('"');
            let v = v.trim().trim_matches('"').trim_matches(',');
            match k {
                "show_tooltip" => c.show_tooltip = v == "true",
                "hide_when_full" => c.hide_when_full = v == "true",
                "run_at_startup" => c.run_at_startup = v == "true",
                "default_mode" => {
                    c.default_mode = match v {
                        "percent" => DisplayMode::Percent,
                        "icon" => DisplayMode::Icon,
                        _ => DisplayMode::TimeRemaining,
                    }
                }
                "alternate_on_click" => c.alternate_on_click = v == "true",
                "only_percent_when_charging" => c.only_percent_when_charging = v == "true",
                "bar_width" => c.bar_width = v.parse().unwrap_or(150).clamp(60, 400),
                "low_percent" => c.low_percent = v.parse().unwrap_or(20).clamp(5, 50),
                "critical_percent" => c.critical_percent = v.parse().unwrap_or(10).clamp(1, 30),
                "notify_low" => c.notify_low = v == "true",
                "notify_critical" => c.notify_critical = v == "true",
                "notify_full" => c.notify_full = v == "true",
                "play_sound" => c.play_sound = v == "true",
                "show_rate_in_tooltip" => c.show_rate_in_tooltip = v == "true",
                "learn_profiles" => c.learn_profiles = v == "true",
                "battery_style" => c.battery_style = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        if c.critical_percent >= c.low_percent {
            c.critical_percent = c.low_percent - 5;
        }
        c
    }

    /// 写回 exe 同目录 options.json
    pub fn save(&self) {
        let mode = match self.default_mode {
            DisplayMode::TimeRemaining => "time",
            DisplayMode::Percent => "percent",
            DisplayMode::Icon => "icon",
        };
        let text = format!(
            "{{\n  \"show_tooltip\": {},\n  \"hide_when_full\": {},\n  \"run_at_startup\": {},\n  \"default_mode\": \"{}\",\n  \"alternate_on_click\": {},\n  \"only_percent_when_charging\": {},\n  \"bar_width\": {},\n  \"low_percent\": {},\n  \"critical_percent\": {},\n  \"notify_low\": {},\n  \"notify_critical\": {},\n  \"notify_full\": {},\n  \"play_sound\": {},\n  \"show_rate_in_tooltip\": {},\n  \"learn_profiles\": {},\n  \"battery_style\": {}\n}}\n",
            self.show_tooltip,
            self.hide_when_full,
            self.run_at_startup,
            mode,
            self.alternate_on_click,
            self.only_percent_when_charging,
            self.bar_width,
            self.low_percent,
            self.critical_percent,
            self.notify_low,
            self.notify_critical,
            self.notify_full,
            self.play_sound,
            self.show_rate_in_tooltip,
            self.learn_profiles,
            self.battery_style,
        );
        let _ = std::fs::write(config_path(), text);
    }

    /// HKCU\Software\Microsoft\Windows\CurrentVersion\Run 自启开关
    pub fn set_startup(&self, enable: bool) {
        use windows::Win32::System::Registry::{
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            KEY_READ, KEY_SET_VALUE, REG_SZ,
        };
        use windows_core::{w, PCWSTR};

        unsafe {
            let mut hkey = HKEY::default();
            let sub: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            if RegOpenKeyExW(HKEY_CURRENT_USER, sub, None, KEY_READ | KEY_SET_VALUE, &mut hkey)
                .is_err()
            {
                return;
            }
            let val: PCWSTR = w!("BatteryBarModern");
            if enable {
                let path = std::env::current_exe()
                    .map(|p| format!("\"{}\" /minimized", p.display()))
                    .unwrap_or_default();
                let mut wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                let bytes =
                    unsafe { std::slice::from_raw_parts_mut(wide.as_mut_ptr() as *mut u8, wide.len() * 2) };
                let _ = RegSetValueExW(hkey, val, None, REG_SZ, Some(bytes));
            } else {
                let _ = RegDeleteValueW(hkey, val);
            }
            let _ = RegCloseKey(hkey);
        }
    }
}
