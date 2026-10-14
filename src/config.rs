// batterybar-modern — 配置与持久化（options.json）
//
// 字段按原版 BatteryBar Pro 的 6 个 Preferences 分类页组织：
//   General / Display / Notifications / Warning Levels / Power Saving / Tools
// 详见 docs/RESEARCH-original-batterybar.md。
#![allow(non_snake_case)]

use std::path::PathBuf;

// ---- Display 页「Information to Display on the Toolbar」的 5 个复选框位 ----
pub const DISPLAY_TIME: u32 = 1 << 0; // Time Remaining
pub const DISPLAY_PERCENT: u32 = 1 << 1; // Percent Remaining
pub const DISPLAY_FINISHED: u32 = 1 << 2; // Time Finished（充满/放空的当日时刻）
pub const DISPLAY_RATE: u32 = 1 << 3; // Discharge/Charge Rate
pub const DISPLAY_CAPACITY: u32 = 1 << 4; // Capacity Remaining

/// 原版 Notifications 页的三组告警结构完全一致，各 5 个控件 + 2 条路径。
#[derive(Clone, Debug)]
pub struct Notif {
    pub audio: bool, // Audio alert（播放提示音）
    pub popup: bool, // Show popup（弹气泡）
    pub custom_sound: bool, // Use custom sound
    pub sound_path: String,
    pub beep: bool, // Beep the system speaker
    pub run_program: bool, // Run a program
    pub program_path: String,
}

impl Notif {
    const fn silent() -> Self {
        Self {
            audio: false,
            popup: false,
            custom_sound: false,
            sound_path: String::new(),
            beep: false,
            run_program: false,
            program_path: String::new(),
        }
    }
    fn on() -> Self {
        Self { audio: true, popup: true, ..Self::silent() }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    // ======== General（原版 General Settings） ========
    pub show_tooltip: bool, // Enable status tooltip
    pub statistical_estimation: bool, // Use statistical time estimation
    pub profiles_per_scheme: bool, // Use separate battery profiles per power scheme
    pub alternate_battery_id: bool, // Use alternate method to determine unique battery ID
    pub hide_when_full: bool, // Hide BatteryBar when the battery is fully charged
    pub force_rate_estimation: bool, // Force estimation of discharge/recharge rates
    pub run_at_startup: bool, // 开机随 Windows 启动（HKCU Run）

    // ======== Display（原版 Display Settings） ========
    /// 「Information to Display on the Toolbar」的 5 个复选框（位掩码）。
    pub display_items: u32,
    pub default_mode: DisplayMode, // 启动时先显示哪一项
    pub alternate_on_click: bool, // Alternate time remaining and percent（点击轮播）
    pub only_percent_when_charging: bool,
    pub dual_battery_popups: bool, // Show individual battery status popups for dual batteries
    pub show_battery_id: bool, // Display the battery ID in the status popup
    pub font_size: u32, // BatteryBar font 字号档：0=小 1=中 2=大
    pub percent_precision: u32, // Percentage precision：小数位数 0/1/2
    pub battery_style: u32, // Theme：0=经典拟物 Aero Glass, 1=现代 Fluent
    pub text_shadow: bool, // 条上文字是否绘制 1px 阴影
    pub bar_width: i32, // 横排电池条宽度（px）
    pub bar_opacity: i32, // 条背景不透明度 0..=100
    pub on_click_mode: u32, // 0=轮播勾选项 1=直接弹详情卡
    pub hide_on_ac: bool, // 接通电源时隐藏条

    // ======== Warning Levels（原版 Warning Levels 页） ========
    pub levels_by_time: bool, // false=Percent remaining, true=Time remaining
    pub low_percent: i32,
    pub low_minutes: i32,
    pub critical_percent: i32,
    pub critical_minutes: i32,
    pub soft_min_percent: i32, // Soft minimum level：计算 Full Runtime 的"空"下限
    pub full_percent: i32, // Full battery level：认定为"满"的电平
    pub alarm_percent: i32, // 额外自定义警报阈值（0=关闭）
    pub critical_action: u32, // 临界电量自动动作：0=无 1=睡眠 2=关机 3=休眠 4=重启

    // ======== Notifications（原版 Notifications 页） ========
    pub notif_critical: Notif,
    pub notif_low: Notif,
    pub notif_full: Notif,
    pub auto_close_notifications: bool, // Automatically close notifications after …
    pub auto_close_seconds: i32,

    // ======== Power Saving（原版 Power Saving 页） ========
    pub battery_change_scheme: bool, // On Battery…: Change power scheme
    pub battery_scheme: u32,         // 0=节能 1=平衡 2=高性能
    pub battery_disable_aero: bool,  // Disable Aero
    pub battery_only_below: bool,    // Only when below N %
    pub battery_below_percent: i32,
    pub ac_change_scheme: bool, // Plugged In…: Change power scheme
    pub ac_scheme: u32,

    // ======== Tools（原版 Tools 页） ========
    pub log_battery_data: bool, // Log battery data → BatteryStatus.log
    pub show_rate_in_tooltip: bool, // 详情卡里显示实时功耗行
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    TimeRemaining, // 原版默认
    Percent,
    TimeFinished, // 充满/放空的当日时刻（原版 3.6.4 新增）
    DischargeRate, // 充/放电功率
    Capacity,      // 剩余容量
    Icon,          // 只显示小电池图标
}

impl DisplayMode {
    /// 位掩码中的一位 → DisplayMode。Icon 不属于 5 个复选框，单独处理。
    pub fn from_bit(bit: u32) -> Option<Self> {
        Some(match bit {
            DISPLAY_TIME => DisplayMode::TimeRemaining,
            DISPLAY_PERCENT => DisplayMode::Percent,
            DISPLAY_FINISHED => DisplayMode::TimeFinished,
            DISPLAY_RATE => DisplayMode::DischargeRate,
            DISPLAY_CAPACITY => DisplayMode::Capacity,
            _ => return None,
        })
    }
    pub fn key(self) -> &'static str {
        match self {
            DisplayMode::TimeRemaining => "time",
            DisplayMode::Percent => "percent",
            DisplayMode::TimeFinished => "finished",
            DisplayMode::DischargeRate => "rate",
            DisplayMode::Capacity => "capacity",
            DisplayMode::Icon => "icon",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // General
            show_tooltip: true,
            statistical_estimation: true,
            profiles_per_scheme: false,
            alternate_battery_id: false,
            hide_when_full: false,
            force_rate_estimation: false,
            run_at_startup: false,
            // Display
            display_items: DISPLAY_TIME | DISPLAY_PERCENT,
            default_mode: DisplayMode::TimeRemaining,
            alternate_on_click: true,
            only_percent_when_charging: false,
            dual_battery_popups: true,
            show_battery_id: false,
            font_size: 1,
            percent_precision: 1,
            battery_style: 0,
            text_shadow: false,
            bar_width: 150,
            bar_opacity: 100,
            on_click_mode: 0,
            hide_on_ac: false,
            // Warning Levels（对齐原版默认：低 30% / 临界 15% / 软下限 5% / 满 100%）
            levels_by_time: false,
            low_percent: 30,
            low_minutes: 45,
            critical_percent: 15,
            critical_minutes: 20,
            soft_min_percent: 5,
            full_percent: 100,
            alarm_percent: 0,
            critical_action: 0,
            // Notifications（原版默认：临界与低电量勾了 Audio + Popup，充满全部未勾）
            notif_critical: Notif::on(),
            notif_low: Notif::on(),
            notif_full: Notif::silent(),
            auto_close_notifications: false,
            auto_close_seconds: 300,
            // Power Saving
            battery_change_scheme: false,
            battery_scheme: 1,
            battery_disable_aero: false,
            battery_only_below: false,
            battery_below_percent: 50,
            ac_change_scheme: false,
            ac_scheme: 1,
            // Tools
            log_battery_data: false,
            show_rate_in_tooltip: true,
        }
    }
}

fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    p.pop();
    p.push("options.json");
    p
}

// JSON 字符串值里写入的路径会被转义为 `\\`，读回时还原。
fn unescape(v: &str) -> String {
    v.replace("\\\\", "\\")
}
fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 通用布尔解析：手写解析器把 `true`/`false`/`1`/`0`/`yes`/`no` 都认。
fn as_bool(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
}

impl Config {
    /// 从 exe 同目录 options.json 读取；不存在/解析失败 → 默认值。
    /// 不引 serde：手写极简解析器（只支持平铺 `"key": value`）。
    pub fn load() -> Self {
        let mut c = Config::default();
        let Ok(text) = std::fs::read_to_string(config_path()) else {
            return c;
        };
        for line in text.lines() {
            let line = line.trim().trim_end_matches(',');
            // 按第一个 ':' 切分 —— 值里可能含盘符冒号（C:\...），只切一次。
            let Some((k, v)) = line.split_once(':') else { continue };
            let k = k.trim().trim_matches('"');
            let raw = v.trim().trim_end_matches(',');
            let v = raw.trim().trim_matches('"');
            match k {
                // ---- General ----
                "show_tooltip" => c.show_tooltip = as_bool(v),
                "statistical_estimation" | "learn_profiles" => {
                    c.statistical_estimation = as_bool(v)
                }
                "profiles_per_scheme" => c.profiles_per_scheme = as_bool(v),
                "alternate_battery_id" => c.alternate_battery_id = as_bool(v),
                "hide_when_full" => c.hide_when_full = as_bool(v),
                "force_rate_estimation" => c.force_rate_estimation = as_bool(v),
                "run_at_startup" => c.run_at_startup = as_bool(v),
                // ---- Display ----
                "display_items" => c.display_items = v.parse().unwrap_or(3) & 0x1F,
                "default_mode" => {
                    c.default_mode = match v {
                        "percent" => DisplayMode::Percent,
                        "finished" => DisplayMode::TimeFinished,
                        "rate" => DisplayMode::DischargeRate,
                        "capacity" => DisplayMode::Capacity,
                        "icon" => DisplayMode::Icon,
                        _ => DisplayMode::TimeRemaining,
                    }
                }
                "alternate_on_click" => c.alternate_on_click = as_bool(v),
                "only_percent_when_charging" => c.only_percent_when_charging = as_bool(v),
                "dual_battery_popups" => c.dual_battery_popups = as_bool(v),
                "show_battery_id" => c.show_battery_id = as_bool(v),
                "font_size" => c.font_size = v.parse().unwrap_or(1).min(2),
                "percent_precision" => c.percent_precision = v.parse().unwrap_or(1).min(2),
                "battery_style" | "theme" => c.battery_style = v.parse().unwrap_or(0).min(3),
                "text_shadow" => c.text_shadow = as_bool(v),
                "bar_width" => c.bar_width = v.parse().unwrap_or(150).clamp(60, 400),
                "bar_opacity" => c.bar_opacity = v.parse().unwrap_or(100).clamp(0, 100),
                "on_click_mode" => c.on_click_mode = v.parse().unwrap_or(0).min(1),
                "hide_on_ac" => c.hide_on_ac = as_bool(v),
                // ---- Warning Levels ----
                "levels_by_time" => c.levels_by_time = as_bool(v),
                "low_percent" => c.low_percent = v.parse().unwrap_or(30).clamp(2, 99),
                "low_minutes" => c.low_minutes = v.parse().unwrap_or(45).clamp(1, 600),
                "critical_percent" => c.critical_percent = v.parse().unwrap_or(15).clamp(1, 90),
                "critical_minutes" => c.critical_minutes = v.parse().unwrap_or(20).clamp(1, 600),
                "soft_min_percent" => c.soft_min_percent = v.parse().unwrap_or(5).clamp(0, 50),
                "full_percent" => c.full_percent = v.parse().unwrap_or(100).clamp(50, 100),
                "alarm_percent" => c.alarm_percent = v.parse().unwrap_or(0).clamp(0, 100),
                "critical_action" => c.critical_action = v.parse().unwrap_or(0).min(4),
                // ---- Notifications ----
                "notify_critical" => c.notif_critical.popup = as_bool(v),
                "notify_low" => c.notif_low.popup = as_bool(v),
                "notify_full" => c.notif_full.popup = as_bool(v),
                "play_sound" => {
                    let b = as_bool(v);
                    c.notif_critical.audio = b;
                    c.notif_low.audio = b;
                }
                "auto_close_notifications" => c.auto_close_notifications = as_bool(v),
                "auto_close_seconds" => {
                    c.auto_close_seconds = v.parse().unwrap_or(300).clamp(1, 3600)
                }
                // 三组告警的完整字段（notif_<group>_<field>）
                "notif_critical_audio" => c.notif_critical.audio = as_bool(v),
                "notif_critical_custom" => c.notif_critical.custom_sound = as_bool(v),
                "notif_critical_sound" => c.notif_critical.sound_path = unescape(v),
                "notif_critical_beep" => c.notif_critical.beep = as_bool(v),
                "notif_critical_run" => c.notif_critical.run_program = as_bool(v),
                "notif_critical_prog" => c.notif_critical.program_path = unescape(v),
                "notif_low_audio" => c.notif_low.audio = as_bool(v),
                "notif_low_custom" => c.notif_low.custom_sound = as_bool(v),
                "notif_low_sound" => c.notif_low.sound_path = unescape(v),
                "notif_low_beep" => c.notif_low.beep = as_bool(v),
                "notif_low_run" => c.notif_low.run_program = as_bool(v),
                "notif_low_prog" => c.notif_low.program_path = unescape(v),
                "notif_full_audio" => c.notif_full.audio = as_bool(v),
                "notif_full_custom" => c.notif_full.custom_sound = as_bool(v),
                "notif_full_sound" => c.notif_full.sound_path = unescape(v),
                "notif_full_beep" => c.notif_full.beep = as_bool(v),
                "notif_full_run" => c.notif_full.run_program = as_bool(v),
                "notif_full_prog" => c.notif_full.program_path = unescape(v),
                // ---- Power Saving ----
                "battery_change_scheme" => c.battery_change_scheme = as_bool(v),
                "battery_scheme" => c.battery_scheme = v.parse().unwrap_or(1).min(2),
                "battery_disable_aero" => c.battery_disable_aero = as_bool(v),
                "battery_only_below" => c.battery_only_below = as_bool(v),
                "battery_below_percent" => {
                    c.battery_below_percent = v.parse().unwrap_or(50).clamp(1, 100)
                }
                "ac_change_scheme" => c.ac_change_scheme = as_bool(v),
                "ac_scheme" => c.ac_scheme = v.parse().unwrap_or(1).min(2),
                // ---- Tools ----
                "log_battery_data" => c.log_battery_data = as_bool(v),
                "show_rate_in_tooltip" => c.show_rate_in_tooltip = as_bool(v),
                _ => {}
            }
        }
        clamp_levels(&mut c);
        c
    }

    /// 写回 exe 同目录 options.json
    pub fn save(&self) {
        let text = format!(
            concat!(
                "{{\n",
                "  \"show_tooltip\": {},\n",
                "  \"statistical_estimation\": {},\n",
                "  \"profiles_per_scheme\": {},\n",
                "  \"alternate_battery_id\": {},\n",
                "  \"hide_when_full\": {},\n",
                "  \"force_rate_estimation\": {},\n",
                "  \"run_at_startup\": {},\n",
                "  \"display_items\": {},\n",
                "  \"default_mode\": \"{}\",\n",
                "  \"alternate_on_click\": {},\n",
                "  \"only_percent_when_charging\": {},\n",
                "  \"dual_battery_popups\": {},\n",
                "  \"show_battery_id\": {},\n",
                "  \"font_size\": {},\n",
                "  \"percent_precision\": {},\n",
                "  \"battery_style\": {},\n",
                "  \"text_shadow\": {},\n",
                "  \"bar_width\": {},\n",
                "  \"bar_opacity\": {},\n",
                "  \"on_click_mode\": {},\n",
                "  \"hide_on_ac\": {},\n",
                "  \"levels_by_time\": {},\n",
                "  \"low_percent\": {},\n",
                "  \"low_minutes\": {},\n",
                "  \"critical_percent\": {},\n",
                "  \"critical_minutes\": {},\n",
                "  \"soft_min_percent\": {},\n",
                "  \"full_percent\": {},\n",
                "  \"alarm_percent\": {},\n",
                "  \"critical_action\": {},\n",
                "  \"notif_critical_audio\": {},\n",
                "  \"notif_critical_popup\": {},\n",
                "  \"notif_critical_custom\": {},\n",
                "  \"notif_critical_sound\": \"{}\",\n",
                "  \"notif_critical_beep\": {},\n",
                "  \"notif_critical_run\": {},\n",
                "  \"notif_critical_prog\": \"{}\",\n",
                "  \"notif_low_audio\": {},\n",
                "  \"notif_low_popup\": {},\n",
                "  \"notif_low_custom\": {},\n",
                "  \"notif_low_sound\": \"{}\",\n",
                "  \"notif_low_beep\": {},\n",
                "  \"notif_low_run\": {},\n",
                "  \"notif_low_prog\": \"{}\",\n",
                "  \"notif_full_audio\": {},\n",
                "  \"notif_full_popup\": {},\n",
                "  \"notif_full_custom\": {},\n",
                "  \"notif_full_sound\": \"{}\",\n",
                "  \"notif_full_beep\": {},\n",
                "  \"notif_full_run\": {},\n",
                "  \"notif_full_prog\": \"{}\",\n",
                "  \"auto_close_notifications\": {},\n",
                "  \"auto_close_seconds\": {},\n",
                "  \"battery_change_scheme\": {},\n",
                "  \"battery_scheme\": {},\n",
                "  \"battery_disable_aero\": {},\n",
                "  \"battery_only_below\": {},\n",
                "  \"battery_below_percent\": {},\n",
                "  \"ac_change_scheme\": {},\n",
                "  \"ac_scheme\": {},\n",
                "  \"log_battery_data\": {},\n",
                "  \"show_rate_in_tooltip\": {}\n",
                "}}\n"
            ),
            self.show_tooltip,
            self.statistical_estimation,
            self.profiles_per_scheme,
            self.alternate_battery_id,
            self.hide_when_full,
            self.force_rate_estimation,
            self.run_at_startup,
            self.display_items,
            self.default_mode.key(),
            self.alternate_on_click,
            self.only_percent_when_charging,
            self.dual_battery_popups,
            self.show_battery_id,
            self.font_size,
            self.percent_precision,
            self.battery_style,
            self.text_shadow,
            self.bar_width,
            self.bar_opacity,
            self.on_click_mode,
            self.hide_on_ac,
            self.levels_by_time,
            self.low_percent,
            self.low_minutes,
            self.critical_percent,
            self.critical_minutes,
            self.soft_min_percent,
            self.full_percent,
            self.alarm_percent,
            self.critical_action,
            // notif_critical
            self.notif_critical.audio,
            self.notif_critical.popup,
            self.notif_critical.custom_sound,
            escape(&self.notif_critical.sound_path),
            self.notif_critical.beep,
            self.notif_critical.run_program,
            escape(&self.notif_critical.program_path),
            // notif_low
            self.notif_low.audio,
            self.notif_low.popup,
            self.notif_low.custom_sound,
            escape(&self.notif_low.sound_path),
            self.notif_low.beep,
            self.notif_low.run_program,
            escape(&self.notif_low.program_path),
            // notif_full
            self.notif_full.audio,
            self.notif_full.popup,
            self.notif_full.custom_sound,
            escape(&self.notif_full.sound_path),
            self.notif_full.beep,
            self.notif_full.run_program,
            escape(&self.notif_full.program_path),
            self.auto_close_notifications,
            self.auto_close_seconds,
            self.battery_change_scheme,
            self.battery_scheme,
            self.battery_disable_aero,
            self.battery_only_below,
            self.battery_below_percent,
            self.ac_change_scheme,
            self.ac_scheme,
            self.log_battery_data,
            self.show_rate_in_tooltip,
        );
        let _ = std::fs::write(config_path(), text);
    }

    /// 当前应显示的下一个模式（点击轮播用）——只走 `display_items` 里勾选的项。
    pub fn next_mode(&self, cur: DisplayMode) -> DisplayMode {
        let order: [u32; 5] = [
            DISPLAY_TIME,
            DISPLAY_PERCENT,
            DISPLAY_FINISHED,
            DISPLAY_RATE,
            DISPLAY_CAPACITY,
        ];
        let picked: Vec<DisplayMode> = order
            .iter()
            .filter(|b| self.display_items & *b != 0)
            .filter_map(|b| DisplayMode::from_bit(*b))
            .collect();
        match picked.len() {
            0 => DisplayMode::Icon,
            1 => picked[0],
            _ => {
                let idx = picked.iter().position(|m| *m == cur).unwrap_or(usize::MAX);
                picked[(idx + 1) % picked.len()]
            }
        }
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

/// 四个电平必须单调：critical < low < full，soft_min <= critical。
pub fn clamp_levels(c: &mut Config) {
    c.full_percent = c.full_percent.clamp(50, 100);
    c.soft_min_percent = c.soft_min_percent.clamp(0, 50);
    c.critical_percent = c.critical_percent.clamp(1, 90);
    c.low_percent = c.low_percent.clamp(2, 99);
    if c.soft_min_percent > c.critical_percent {
        c.soft_min_percent = c.critical_percent;
    }
    if c.critical_percent >= c.low_percent {
        c.critical_percent = (c.low_percent - 1).max(1);
    }
    if c.low_percent >= c.full_percent {
        c.low_percent = c.full_percent - 1;
    }
    c.auto_close_seconds = c.auto_close_seconds.clamp(1, 3600);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_levels_ordered() {
        let mut c = Config::default();
        c.critical_percent = 80;
        c.low_percent = 30;
        c.soft_min_percent = 40;
        c.full_percent = 20;
        clamp_levels(&mut c);
        assert!(c.soft_min_percent <= c.critical_percent);
        assert!(c.critical_percent < c.low_percent);
        assert!(c.low_percent < c.full_percent);
    }

    #[test]
    fn next_mode_cycles_only_checked_items() {
        let mut c = Config::default();
        c.display_items = DISPLAY_TIME | DISPLAY_PERCENT;
        assert_eq!(c.next_mode(DisplayMode::TimeRemaining), DisplayMode::Percent);
        assert_eq!(c.next_mode(DisplayMode::Percent), DisplayMode::TimeRemaining);

        c.display_items = DISPLAY_CAPACITY;
        assert_eq!(c.next_mode(DisplayMode::TimeRemaining), DisplayMode::Capacity);

        c.display_items = 0;
        assert_eq!(c.next_mode(DisplayMode::Percent), DisplayMode::Icon);
    }
}
