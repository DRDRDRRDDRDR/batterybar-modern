# 原版 BatteryBar / BatteryBar Pro 功能与外观调研

> 调研目的：为 `batterybar-modern` 重做提供原版功能面与视觉模型的**事实依据**。
> 原版作者 Osiris Development，稳定版 **3.6.6（2014-01-20 发布）**，免费版 + Pro 版（$8 终身授权）。
> 站点 `osirisdevelopment.com` / `batterybarpro.com` 现已停运（DNS 指向 74.48.19.213），
> `web.archive.org` 在本机网络下不可达；本文结论来自**原版设置界面实机截图**、
> **官方 Wiki 的搜索引擎快照**、**官方 Release Notes 摘要**、**第三方主题仓库中的 `theme.xml`**。

---

## 1. 设置界面：6 个分类页

原版 Preferences 是一个 **630x652** 的对话框，顶部标题栏 `BatteryBar Pro Settings`，
下方横幅 `BatteryBar configuration options` + 铅笔图标，再下是 **6 个带图标的 Tab**，
底部固定 `Save` / `Cancel`。

| # | Tab | 图标 | 内容 |
|---|---|---|---|
| 1 | General | 齿轮 | 语言、状态提示、统计算法、配置文件、自启 |
| 2 | Display | 电池图 | 显示项多选、字体、精度、主题 |
| 3 | Notifications | 气泡 | 三档告警的音/弹窗/自定义声音/运行程序 |
| 4 | Warning Levels | 量杯 | 电量/时间双基准、四个电平滑块 |
| 5 | Power Saving | 闪电 | 电池供电 / 接通电源 两套自动动作 |
| 6 | Tools | 扳手+螺丝刀 | 电池数据记录、曲线、反馈 |

### 1.1 General（常规设置）

```
General Settings
Language                    [English              v]

[x] Enable status tooltip
[x] Use statistical time estimation
[ ] Use separate battery profiles per power scheme
[ ] Use alternate method to determine unique battery ID
[ ] Hide BatteryBar when the battery is fully charged
[ ] Force estimation of discharge/recharge rates
```

语义（官方 Wiki 原文摘要）：
- **Enable status tooltip** — 勾选：鼠标悬停在 BatteryBar 上时弹出 Status Popup，移开即消失；不勾选：永不弹出。
- **Use statistical time estimation** — 勾选：剩余时间基于**历史使用数据**（电池配置文件）计算；不勾选：基于**当前放电速率**直接算。
- **Use separate battery profiles per power scheme** — 每个电源计划各存一份电池配置数据（3.2 Beta 引入）。
- **Hide BatteryBar when the battery is fully charged** — 满电时隐藏；3.6.4 起若无电池也一并关闭。
- **Force estimation of discharge/recharge rates** — 强制用估算速率（设备不报速率时）。

### 1.2 Display（显示与样式）

```
Display Settings

[ ] Alternate time remaining and percent
[ ] Only show percent remaining when battery is charged
[x] Show individual battery status popups for dual batteries
[ ] Display the battery ID in the status popup

BatteryBar font:      [Tahoma 10pt        ]
Percentage precision: [1] digits
Theme:                [(Default) BatteryBar v]

Information to Display on the Toolbar
 [x] Time Remaining      [x] Percent Remaining   [ ] Time Finished
 [ ] Discharge/Charge Rate  [ ] Capacity Remaining
```

关键点：
- **"Information to Display on the Toolbar" 是 5 个独立复选框**，不是单选。选多个时条上轮播
  （`Alternate time remaining and percent`）。
- 5 个显示项 —— 与工具栏轮播对应：`Time Remaining` / `Percent Remaining` / `Time Finished` /
  `Discharge/Charge Rate` / `Capacity Remaining`。
- **Time Finished** = 充满（或放空）的**当日时刻**，形如 `@ 5:23 PM`（3.6.4 新增）。
- **Percentage precision** = 小数位数（0/1/2）。
- 原版 Wiki 旧版说明：「Show time remaining on startup — 勾选：默认显示剩余时间；不勾选：默认显示百分比。
  使用中**点击 BatteryBar 可在百分比与剩余时间之间切换**。」

### 1.3 Notifications（警报与通知）

三组结构完全相同，各含 4 个控件：

```
Critical battery warning:
  [x] Audio alert              [x] Show popup
      [ ] Use custom sound:                 [Choose...]
      [ ] Beep the system speaker
      [ ] Run a program:                    [Choose...]

Low battery warning:            （同上）
Finished charging:              （同上，默认全部未勾选）

[ ] Automatically close notifications after  [300] seconds
```

### 1.4 Warning Levels（警告阈值）

```
Low and Critical battery levels defined by:
  (o) Percent remaining
  ( ) Time remaining

Low battery level:                        Critical battery level:
  [====|=================]  30%             [==|=================]  15%
  Minutes remaining: [45]                   Minutes remaining: [20]

Soft minimum level:                       Full battery level:
  [=|====================]   5%             [==============|=====] 100%
```

- **Soft minimum level**：计算 Full Runtime 时的"空"下限。电池到 5% 就算耗尽，
  `Full Runtime = 可用容量 × (Full% − SoftMin%) / 放电速率`。避免把电量用到 0% 伤的电池。
- **Full battery level**：认定为"满"的电平。部分带充电限幅的机器只充到 80/95%，
  设了它满电判定与 Full Runtime 才准。
- Low / Critical 同时支持 % 与分钟两种表达，切换基准时另一套仍保留。

### 1.5 Power Saving（省电）

左右两栏，结构对称：

```
On Battery...                          Plugged In...
[x] Change power scheme                [x] Change power scheme
    [Balanced          v]                  [Balanced          v]
[ ] Disable Aero
    [ ] Only when below  [50] %
```

即：**电池供电时**自动切到某个电源计划（可选仅低于 N% 才切）、可关 Aero；
**接通电源时**自动切回某个电源计划。

### 1.6 Tools（工具）

- `Log battery data` —— 把电池状态写入 `BatteryStatus.log`（官方 Release Notes：
  "FIX: Logging battery status (in Prefs->Tools) now works as expected"、
  "FIX: Don't delete the battery status log when sending a feedback report if the user
  has the Log Battery Data option turned on"）。
- `Battery profile graph` —— 打开独立的电池曲线窗口。
- 反馈报告相关入口与"重置电池配置数据"。

> 该页未取得原版截图，以上由 Release Notes 与 Wiki 摘要交叉推断。

---

## 2. 状态弹窗（Status Popup）

原版实机截图（`ref/taskbar_surface.png`）逐字段还原：

```
 ┌──────────────────────────────────────────┐
 │ [电池图标]  BatteryBar Pro v3.6.6         │
 │                                          │
 │ Percent:         72.5%                   │
 │ Capacity:        30,780 mWh of 42,454 mWh│
 │ Charge Rate:     9,880 mW                │
 │ Time Remaining:  1:18 @ 5:05 PM (Charging)│
 │ Elapsed Time:    0:07 (since 70.4%)      │
 │ Full Runtime:    6:13                    │
 │ Battery Wear:    0.0% of 42,454 mWh      │
 └──────────────────────────────────────────┘
```

字段定义（官方 Glossary + Wiki）：
- **Percent** = `Current Capacity / Full Capacity`（当前容量 ÷ 满电容量）。
- **Capacity** = 当前容量 mWh **of** 满电容量 mWh。
- **Charge Rate** = 实时充/放电功率，毫瓦（正=充电，负=放电）。带 `Discharge/Charge Rate` 时同义。
- **Time Remaining** = 剩余时间，形如 `1:18`；后接放电/充满的**当日时刻** `@ 5:05 PM`；
  括号内是状态（`Charging` / `Discharging` / `On A/C`）。
- **Elapsed Time** = 当前这段放电（充电）已持续多久，括号内 `since 70.4%` = 该段起点电量。
- **Full Runtime** = 从满电放到 *Soft minimum* 的全部可用时长（用到 SoftMin%，不是 0%）。
- **Battery Wear** = 电池**已失去**的设计容量百分比，括号内给出绝对值 mWh。
  Glossary 原文：`The percentage of the battery's designed capacity that your battery has lost.`

满电接通电源时，时间行显示 `On A/C`（官方 Wiki Preferences 页明确）。

---

## 3. 工具栏本体

- 形态：**电池形状的条**（壳体 + 正极端子 + 内部填充），填充长度 = 电量。
- 充满时填充为**蓝色渐变**；放电为绿/黄/红按阈值递进。
- 文字叠在条上，字体族与字号由 Display 页的 `BatteryBar font` 决定（默认 `Tahoma 10pt`）。
- 交互：
  - 单击 → 在 Display 页勾选的信息之间轮播。
  - 悬停 → 弹出 Status Popup（受 `Enable status tooltip` 控制），移开即关闭。
  - 右键 → 菜单（Preferences… / About / 电源计划 / 隐藏 / 退出）。Pro 版右键菜单可直接切电源计划。
- Windows 11 起微软移除了任务栏工具栏（toolbar）能力，**原版在 Win11 必须选 "Floating" 模式**；
  这正是本项目的价值点——用 `SetParent` 直接嵌进 `Shell_TrayWnd`。

---

## 4. 主题系统（theme.xml）

官方 Wiki「How to create a theme」+ 真实主题仓库
（`bsdev90/batterybar-win10battery`、`ducalex/batterybar-theme-w10-light`、`dyakovk/batterybar-theme-w10-*`）反推：

**一个主题 = `theme.xml` + 若干 png。`theme.xml` 定义 5 个状态，
每个状态给出 Background / ProgressBar / HoverOverlay / PressedOverlay 的图片与边距。**

```xml
<Theme>
  <Author>BenDavidson90</Author>
  <Date>2019-03-23</Date>
  <Description>Styled to match the Windows 10 battery tray icon.</Description>
  <Name>W10 Colored - Dark</Name>
  <Version>1.2</Version>
  <MaxHeight>14</MaxHeight>            <!-- 可选：主题声明的高度上限 -->

  <States>
    <Default> | <Discharging> | <LowBattery> | <CriticalBattery> | <Charging>
      <Font><Name>Segoe UI</Name><Size>6</Size></Font>   <!-- Size 单位 pt -->
      <TextColor>0,0,0,0</TextColor>                     <!-- RGB；四元组全 0 = 不显示文字 -->
      <TextShadow>False</TextShadow>
      <Padding>8,50,0,0</Padding>                        <!-- 文字内边距 左,上,右,下 -->
      <Background>
        <!-- 形式 A：单张 Center 图整体缩放 -->
        <Center>resources\\dark\\_battery.png</Center>
        <Margin>0,0,0,0</Margin>
        <!-- 形式 B：九宫格（BoxTile）拉伸 -->
        <CornerTopLeft>/<CornerTopRight>/<CornerBottomLeft>/<CornerBottomRight>
        <LeftBorder>/<RightBorder>/<TopBorder>/<BottomBorder>
        <Center>…</Center>
        <Margin>0</Margin>
      </Background>
      <ProgressBar><Center>…png</Center><Margin>24,35,26,35</Margin></ProgressBar>
      <HoverOverlay><Center>…png</Center><Margin>0,0,0,0</Margin></HoverOverlay>
      <PressedOverlay><Center>…png</Center><Margin>0,0,0,0</Margin></PressedOverlay>
    </Default>
  </States>
</Theme>
```

要点：
- **5 个状态**：`Default` / `Discharging` / `LowBattery` / `CriticalBattery` / `Charging`。
  主题作者只给 `Default` 时其余继承默认（bsdev90 主题即如此）。
- **Margin 是四元组，单位相对 `MaxHeight`**（官方 Release Notes：
  "Margins are relative to MaxHeight"）；官方还修过
  "FIX: New theme is squished in vertical taskbars" / "…in small taskbars"，
  即**竖排与小任务栏下主题需要单独适配尺寸档**。ducalex 主题就提供了
  `Default / Small / Tiny` 三档 xml。
- **HoverOverlay / PressedOverlay** = 悬停与按下两个瞬时视觉层。
- 原版内置主题：`(Default) BatteryBar`、`Windows 7 Small theme`。
- 语言：`Translations` —— 官方按系统语言自动加载对应翻译文件（数十种语言，
  含 `swiss-german` 等）。

---

## 5. Battery Profile Graph（电池曲线，Pro 专属）

用户明确点名要复刻的独立窗口。已知事实：

- 官方 Release Notes 摘要：
  - "NEW (Pro): When the Battery Profile Graph is open, the graph will update with
    real-time data if the selected profile is active" —— **打开期间实时刷新**。
  - "CHG: Battery Profile Graph window now shows the current battery state when opened"。
  - "CHG: When looking at battery charging profile, the legend will say
    'Time to full charge'" —— **图例随放电/充电曲线切换文案**。
  - "CHG: Battery Profile Graph now uses default system font to display the graph labels.
    Some systems had problems displaying Arial Regular font."
  - "FIX (Pro): Fixed a few minor issues with the Battery Profile Graph feature"。
- 第三方评测描述：「选择一个电池配置文件，以**图形方式**展示」；
  「可以**缩放**，可以**导出为图片或 CSV**」；「查看电池性能的详细统计」。

**结论性功能模型（本项目据此实现）**：
1. 独立窗口，标题 `Battery Profile Graph`。
2. 曲线数据来自**历史采样**（放电/充电过程中定期记录的 电量%、容量 mWh、速率 mW、时间戳、是否接 AC）。
3. 有 **Discharging / Charging 两套曲线**，可切换；充电曲线图例为 `Time to full charge`。
4. **X 轴 = 时间，Y 轴 = 电量%**（可切到容量/速率）。
5. **当前状态标记**：打开时立即标出当前所在点；所选配置文件处于活动状态时**实时追加**。
6. 支持缩放与导出。

---

## 6. 原版其他行为（Release Notes 交叉验证）

- 3.2 Beta：浮动工具栏、充满电通知、电池配置数据**按电源计划分别存储**、重设计 Preferences。
- 3.6.4：显示充满（放空）的当日时刻 `@ 5:23 PM`；`Hide When Full` 在无电池时也隐藏条。
- 3.6.5：新增「何时」触发通知的选项。
- 3.6.6：Pro 新增**电量通知时运行程序**；Pro 在 Preferences 中**预览字体改动**；
  改进特定电池的状态侦测。
- 「Added detection when the computer goes into suspend mode or resumes, which helps make
  sure that the data recorded about your battery usage is accurate」——
  **睡眠/唤醒要断开曲线记录**，否则会算出虚假的放电时长。
- 「NEW: FullLifetime and Time Remaining will now use a simple calculation based on the
  current discharge rate if no profile data is available」—— 无历史数据时退化为瞬时速率。

---

## 7. 对本项目的可执行结论

| 原版能力 | 本项目现状 | 动作 |
|---|---|---|
| 6 分类设置页 | 单页滚动列表 | **重写为 6 个 Tab** |
| 5 项工具栏显示（含 Time Finished） | 5 态，缺 Time Finished | 补 |
| Battery Profile Graph | **完全没有** | **新建 `graph.rs` 独立窗口** |
| 历史采样落盘 | 只有 EMA 速率 | 新增采样 + `profile.csv` |
| 通知按 低/临界/充满 三档细分 | 只有开关 | 按原版三档展开 |
| Warning Levels 四电平 + 双基准 | 只有 low/critical 两个 | 补 soft_min / full_level / 时间基准 |
| Power Saving 自动切计划 | 只有手动切 | 补自动切换 |
| 主题 5 状态 + 图片 | 内置 2 种硬编码风格 | 至少补齐 5 状态配色模型 |
| 字体族/字号/精度/内边距/文字阴影 | 只有 3 档字号 | 补精度与内边距 |
| 睡眠/唤醒断开记录 | 无 | 随曲线一起实现 |
