# Claude 指纹切换工具 · 使用手册

> 依据 https://ip.net.coffee/claude/timezone.html 的分析：Claude Code 疑似读取**系统时区**（而非 IP）判断用户所在地，`Asia/Shanghai` / `Asia/Urumqi` 是被点名的高风险特征。

## ✨ 它是什么

两套界面，同一套逻辑，任君选择：

| 文件 | 形态 | 适合 |
|---|---|---|
| `claude-fingerprint-gui.ps1` | **WPF 图形窗口**（深色主题） | 日常使用，双击即用 |
| `claude-fingerprint.ps1` | 命令行 | 脚本 / 计划任务 / 无人值守 |
| `启动指纹切换器.bat` | 双击启动 GUI | 最省事，不用记命令 |

**GUI 版主打**：🇸🇬 新加坡 + 🇺🇸 美国加州 **两个一级大按钮**，一键切换、一键恢复，不用记任何参数 (◕‿◕)

```powershell
# 启动 GUI（也可以直接双击"启动指纹切换器.bat"）
pwsh -File claude-fingerprint-gui.ps1
```

**核心思路（文章推荐方案二）**：时区换成同为 **UTC+8** 的新加坡或台北 —— 时钟显示的时间和北京时间**分毫不差**，日程闹钟完全不受影响，但系统时区名不再是 `Asia/Shanghai`。

## 🚀 快速开始

```powershell
# 1. 体检（只读，不改任何设置）
pwsh -File claude-fingerprint.ps1 -Action status

# 2. 一键切换（推荐新加坡，UTC+8 时钟零差异）
pwsh -File claude-fingerprint.ps1 -Action profile -Target Singapore

# 3. 还原到切换前
pwsh -File claude-fingerprint.ps1 -Action restore
```

> 需要用到 pwsh（PowerShell 7）。普通用户权限即可切换时区，不需要管理员。

## 🎯 可用画像

| Target | 时区 ID | 时钟影响 | 适用场景 |
|---|---|---|---|
| `Singapore` (推荐) | `Singapore Standard Time` | **零差异** (UTC+8) | 日常使用首选 |
| `California` ★ | `Pacific Standard Time` | -15/-16 小时 | **美国加州纯净 IP 专用** |
| `Taipei` | `Taipei Standard Time` | **零差异** (UTC+8) | 同上，二选一 |
| `Tokyo` | `Tokyo Standard Time` | +1 小时 | 出口在日本 |
| `NewYork` | `Eastern Standard Time` | -12/-13 小时 | 出口在美东 |
| `LosAngeles` | `Pacific Standard Time` | -15/-16 小时 | 同 California |
| `Shanghai` | `China Standard Time` | 原始状态 | 一键回默认 |

自定义：
```powershell
pwsh -File claude-fingerprint.ps1 -Action profile -Target Custom `
  -CustomTimeZone "W. Europe Standard Time" -CustomCulture "en-US" -CustomLocale "en-US"
```

## ⚠️ 切换后必做（3 件事）

1. **重启 Claude Code** —— 它是常驻进程，不重启不会重新读时区。
2. **完全重启 Chrome / Edge** —— 浏览器语言写在 Profile 的 `Preferences` 里，要全退出再开。
3. 去 https://ip.net.coffee/claude/ 刷新，看"设备信息"卡片里 **时区**那行是否变绿/不再黄标。

## 🔧 它到底改了什么

| 项目 | 改动方式 | 影响范围 |
|---|---|---|
| 时区 | `tzutil /s <ID>` | 系统级，全局生效 |
| 区域格式 | `Set-Culture` | 用户级，新开进程生效（日期/数字格式） |
| 浏览器语言 | 改 Chrome 各 Profile 的 `Preferences` → `intl.accept_languages` | 需重启浏览器 |
| UI 界面语言 | **不动** | 你的 Windows 照常显示简体中文，零学习成本 |

## 📌 注意事项

- **不碰 UI 语言**：Windows 界面仍是简体中文，只有"格式/时区/浏览器语言"变，日常使用无感。
- **`Set-Culture` 可能失败**：如果目标语言包没装（比如 `en-SG`），脚本会跳过并提示，时区仍然切成功（时区才是核心）。
- **备份位置**：`%LOCALAPPDATA%\ClaudeFingerprint\backup.json`，`-Action restore` 从这还原。已有备份时不会被覆盖，始终保留"切换前"的原始状态。
- **这是指纹工具，不是代理**：它不隐藏 IP。IP 纯净度请配合分流/代理，并用上面的检测页复查。

## 🎀 交互模式

```powershell
pwsh -File claude-fingerprint.ps1 -Action gui
```
菜单式命令行交互，适合不想记参数的场合。

**更推荐 GUI 窗口版**（深色主题，新加坡 / 美国加州两个一级按钮）：

```powershell
pwsh -File claude-fingerprint-gui.ps1
# 或双击 启动指纹切换器.bat
```

GUI 界面包含：

- 📋 **当前状态卡** —— 时区 / 本地时间 / 区域格式 / 系统区域 / 界面语言，实时刷新
- 🎨 **风险徽章** —— 本地指纹风险分 + 等级（高危红 / 中危橙 / 低危蓝 / 安全绿）
- 🌸 **新加坡** 大按钮 —— UTC+8，时钟与北京时间零差异
- 🌴 **美国加州** 大按钮 —— Pacific Time，配加州纯净 IP 用
- ⏪ **一键恢复** 大按钮 —— 还原到切换前的原始状态（有备份时才亮起）
- 🎀 **其他** —— 台北 / 东京 / 纽约 / 上海(默认) / 刷新状态 / 检测页复查
- 📝 **操作日志** —— 每一步改了什么都写清楚

## 无人值守 / 计划任务

```powershell
# 静默切换，配合 schtasks 定时或开机触发
pwsh -File claude-fingerprint.ps1 -Action profile -Target Singapore -Silent
```
