# BatteryBar Modern

现代开源版 BatteryBar：把电量条直接嵌进 Windows 10/11 任务栏，带系统托盘图标与详情弹窗。Rust + windows-rs 单文件实现，无运行时依赖。

![screenshot](shot_taskbar.png)

## 功能

- **真·任务栏嵌入**：条窗口 `SetParent` 进 `Shell_TrayWnd`（TrafficMonitor 同款方案），不依赖 COM Deskband（Win11 22H2+ 已移除其支持），无需签名/管理员
- **电量百分比居中显示**，轨道 + 圆角填充：充电 = 蓝，≤15% = 红，其余按电量蓝→绿渐变
- **点击弹详情**：大字号百分比、充/放电状态、剩余/充满时间估算（EMA 滑动平均防跳变）、电池健康度
- **托盘图标**：shell32 电池图标 + 实时 tooltip；右键菜单退出
- **数据源**：WinRT `Battery.AggregateBattery` 推送（`ReportUpdated`）+ 30s 兜底轮询，事件不丢
- **主题跟随**：读取 `AppsUseLightTheme` 自动切深/浅配色（配色基准 Telegram 主题）
- **Per-Monitor V2 DPI 感知**

## 构建

需要 Rust（GNU 或 MSVC toolchain 均可）：

```
cargo build --release
```

产物 `target/release/batterybar-modern.exe`（约 270 KB），仅依赖系统 DLL，单文件分发。

## 运行

双击 `batterybar-modern.exe` 即可；退出走托盘右键菜单。

## 已知限制

- 竖直任务栏：条按横向绘制（布局可用但宽度偏大），v0.2 适配竖排
- 任务栏底色按主题近似贴色（#202020 / #F3F3F3），极端自定义主题下可能有轻微色差

## License

MIT
