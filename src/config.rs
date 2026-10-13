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

    // ---- Advanced（原版 Advanced Settings 扩展）----
    /// 自定义电量警报阈值（0 = 关闭），掉到该电量时弹气泡 + 播提示音
    pub alarm_percent: i32,
    /// 临界电量自动动作：0=无 1=睡眠 2=关机 3=休眠 4=重启
    pub critical_action: u32,
    /// 任务栏条背景不透明度 0..=100（0 = 完全透明）
    pub bar_opacity: i32,
    /// 文字字号档位：0=小(10) 1=中(12) 2=大(14)
    pub font_size: u32,
    /// 左键单击行为：0=循环切换显示模式 1=直接弹出详情卡片
    pub on_click_mode: u32,
    /// 接通电源时隐藏电池条
    pub hide_on_ac: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    TimeRemaining, // 原版默认
    Percent,
    DischargeRate, // 功耗/充电速率 (W / mW)
    Capacity,      // 剩余容量 (Wh / mWh)
    Icon,          // 只显示小电池图标
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
            alarm_percent: 0,
            critical_action: 0,
            bar_opacity: 100,
            font_size: 1,
            on_click_mode: 0,
            hide_on_ac: false,
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
                        "rate" => DisplayMode::DischargeRate,
                        "capacity" => DisplayMode::Capacity,
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
                "alarm_percent" => c.alarm_percent = v.parse().unwrap_or(0).clamp(0, 100),
                "critical_action" => c.critical_action = v.parse().unwrap_or(0).min(4),
                "bar_opacity" => c.bar_opacity = v.parse().unwrap_or(100).clamp(0, 100),
                "font_size" => c.font_size = v.parse().unwrap_or(1).min(2),
                "on_click_mode" => c.on_click_mode = v.parse().unwrap_or(0).min(1),
                "hide_on_ac" => c.hide_on_ac = v == "true",
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
            DisplayMode::DischargeRate => "rate",
            DisplayMode::Capacity => "capacity",
            DisplayMode::Icon => "icon",
        };
        let text = format!(
            "{{\n  \"show_tooltip\": {},\n  \"hide_when_full\": {},\n  \"run_at_startup\": {},\n  \"default_mode\": \"{}\",\n  \"alternate_on_click\": {},\n  \"only_percent_when_charging\": {},\n  \"bar_width\": {},\n  \"low_percent\": {},\n  \"critical_percent\": {},\n  \"notify_low\": {},\n  \"notify_critical\": {},\n  \"notify_full\": {},\n  \"play_sound\": {},\n  \"show_rate_in_tooltip\": {},\n  \"learn_profiles\": {},\n  \"battery_style\": {},\n  \"alarm_percent\": {},\n  \"critical_action\": {},\n  \"bar_opacity\": {},\n  \"font_size\": {},\n  \"on_click_mode\": {},\n  \"hide_on_ac\": {}\n}}\n",
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
            self.alarm_percent,
            self.critical_action,
            self.bar_opacity,
            self.font_size,
            self.on_click_mode,
            self.hide_on_ac,
        );
        let _ = std::fs::write(config_path(), text);
    }

    /// HKCU\Software\Microsoft\Windows\CurrentVersion\Run 自启开关。
    ///
    /// 返回 `true` 表示注册表已按 `enable` 落地。返回 `false` 时必须由调用方
    /// 回滚配置并告知用户 —— 之前这里把所有错误都吞掉，结果是「开关看着是开
    /// 的、options.json 里也写着 true，但注册表里根本没有这一项」，属于最难
    /// 排查的一类静默失败。
    ///
    /// 已知失败场景：进程完整性级别低于中等级别（例如 exe 位于带
    /// `Mandatory Label\Low Mandatory Level` 的目录，由 DSH 文件沙箱标记）
    /// 时，`RegOpenKeyExW` 对 HKCU 的写入请求会被拒（ACCESS_DENIED，即
    /// `WIN32_ERROR(5)`）。此时只能如实返回 false。
    pub fn set_startup(&self, enable: bool) -> bool {
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
                return false;
            }
            let val: PCWSTR = w!("BatteryBarModern");
            let ok = if enable {
                let path = std::env::current_exe()
                    .map(|p| format!("\"{}\" /minimized", p.display()))
                    .unwrap_or_default();
                let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                // REG_SZ 需要字节视图（含结尾 NUL），字符串本身作为字节序列写入
                let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
                RegSetValueExW(hkey, val, None, REG_SZ, Some(bytes)).is_ok()
            } else {
                // 关闭时「值本来就不存在」等同于成功，不算失败。
                // ERROR_FILE_NOT_FOUND == WIN32_ERROR(2)
                let rc = RegDeleteValueW(hkey, val);
                rc.is_ok() || rc == windows::Win32::Foundation::WIN32_ERROR(2)
            };
            let _ = RegCloseKey(hkey);
            ok
        }
    }
}
