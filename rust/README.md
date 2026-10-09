# Claude 指纹切换器 · 使用手册

> 依据 https://ip.net.coffee/claude/timezone.html 的分析：Claude Code 疑似读取**系统时区**判断用户所在地，`Asia/Shanghai` / `Asia/Urumqi` 是被点名的高风险特征。
>
> **本文档描述的是 Rust 版。** 历史上还有一版 PowerShell 实现，已完全被 Rust 版取代，本文不再描述它。

## ⚠️ 先读这段：它到底能解决什么，不能解决什么

原文列出了**两条**与出口 IP 无关的判定路径：

| 路径 | 怎么暴露 | 本工具 |
|---|---|---|
| **系统时区** | 时区写着 `Asia/Shanghai` / `Asia/Urumqi` | ✅ **能改** |
| **`ANTHROPIC_BASE_URL`** | 该变量指向中转站/国内域名（名单约 147 项） | ❌ **管不了，只能你自己检查** |

**本工具不是代理**，它不隐藏 IP，也不检测 DNS / WebRTC / NTP 泄露。它只做一件事：把你本机的时区（以及区域格式、浏览器语言）从"中国大陆特征"改成别的。

所以切换完**不等于安全**，请务必自己确认这四件事：

1. `ANTHROPIC_BASE_URL` 是否为空或指向官方 `api.anthropic.com`（指向中转站就是明牌）；
2. 出口 IP 的归属地，是否和你设的时区对得上（"时区一致"才是关键）；
3. NTP 校时、`sentry.io` / `statsigapi.net` 等遥测域名是否走代理；
4. **永远**自己去检测页复测：<https://ip.net.coffee/claude/>

## ✨ 它是什么

一个 Rust + egui 的 Windows 桌面小工具，**一键切换系统时区**，附带区域格式和浏览器语言。

| 形态 | 说明 |
|---|---|
| `claude-fingerprint.exe` | 带图形界面 + 命令行，同一个文件 |
| `启动指纹切换器.bat` | 双击启动图形界面，不用记命令 |

**界面主打**：🇸🇬 新加坡 + 🇺🇸 美国加州 **两个一级大按钮**，一键切换、一键恢复。

```powershell
# 启动图形界面
.\claude-fingerprint.exe
# 或者直接双击 启动指纹切换器.bat
```

**核心思路（原文推荐方案二）**：时区换成同为 **UTC+8** 的新加坡或台北 —— 时钟显示的时间和北京时间**分毫不差**，日程闹钟完全不受影响，但系统时区名不再是 `Asia/Shanghai`。

## 🚀 快速开始

> **本仓库不提供预编译的 exe。** 这是个要读你注册表和浏览器配置的程序，请从源码构建，
> 或用发布页给出的 SHA256 核对二进制（见 [从源码构建](#-从源码构建)）。
> 下面的 `.\claude-fingerprint.exe` 指打包/构建后的产物。

```powershell
# 1. 体检（只读，不改任何设置）
.\claude-fingerprint.exe status

# 2. 一键切换（推荐新加坡，UTC+8 时钟零差异）
.\claude-fingerprint.exe apply singapore

# 3. 还原到切换前
.\claude-fingerprint.exe restore
```

> **时区是系统级设置**，普通用户权限即可修改，**通常不需要管理员**。
> （例外：域策略限制了 `SeTimeZonePrivilege`，或系统开启了"自动设置时区"时可能失败或被回滚，见下文。）

### 命令行参考

```
claude-fingerprint status              查看当前指纹与风险分
claude-fingerprint apply <画像>        切换时区 / 区域语言 / 浏览器语言
claude-fingerprint restore             还原到切换前的状态
claude-fingerprint --help  (-h/help)   查看帮助
claude-fingerprint --version (-V)      查看版本
claude-fingerprint                     不带参数 = 启动图形界面
```

> 图形界面模式下控制台窗口会被自动隐藏；从已有的终端窗口启动时不会隐藏（以免连带隐藏你的终端）。

## 🎯 可用画像

每个画像会改**三样**东西，全部列在这里（不只是时区）：

| 画像 | 别名 | 时区 ID | 区域格式 | 浏览器语言 (`accept_languages`) | 时钟影响 |
|---|---|---|---|---|---|
| `singapore` | `sg` | `Singapore Standard Time` | `en-SG` | `en-SG,en;q=0.9,zh-CN;q=0.8` | **零差异** (UTC+8) |
| `california` | `ca` / `us` | `Pacific Standard Time` | `en-US` | `en-US,en;q=0.9` | -15/-16 小时 |
| `taipei` | `tw` | `Taipei Standard Time` | `zh-TW` | `zh-TW,zh;q=0.9,en;q=0.8` | **零差异** (UTC+8) |
| `tokyo` | `jp` | `Tokyo Standard Time` | `ja-JP` | `ja-JP,ja;q=0.9,en;q=0.8` | +1 小时 |
| `newyork` | `ny` | `Eastern Standard Time` | `en-US` | `en-US,en;q=0.9` | -12/-13 小时 |
| `shanghai` | `cn` | `China Standard Time` | `zh-CN` | `zh-CN,zh;q=0.9` | 原始状态 |

时区 ID 使用 Windows 格式（不是 `Asia/Singapore` 这种 IANA 名字）；界面上显示的就是这个 ID。

> ⚠️ **注意 `singapore` 的浏览器语言里保留了 `zh-CN;q=0.8`**（尽量不干扰中文网页，
> 并让 `navigator.language` 与新加坡仍有中文使用者的现实相符）。如果你希望完全不带
> `zh-CN`，请用命令行画像 `taipei`，或改完后再到浏览器设置里自行调整语言顺序。

## 🔧 它到底改了什么

| 项目 | 改动方式 | 影响范围 | 需要重启什么 |
|---|---|---|---|
| **时区** | `tzutil /s <ID>` | 系统级，全局生效 | Claude Code、浏览器 |
| **区域格式** | 写 `HKCU\Control Panel\International` 的 `Locale` / `LocaleName` / `sLanguage` | 用户级，新开进程生效 | 新开的程序 |
| **浏览器语言** | 改 Chrome/Edge/Brave/Chromium/Vivaldi 各 Profile 的 `Preferences` → `intl.accept_languages` | 仅这些浏览器 | 浏览器（且必须先完全退出，见下） |
| **首选 UI 语言** | **不动** | — | — |
| **系统时区以外的任何东西** | **不动** | — | — |

### 为什么"首选 UI 语言"故意不碰

早先的实现会写 `HKCU\Control Panel\Desktop\PreferredUILanguages`，这是**有害**的，已移除：

- 这个键是"覆盖语言选择"列表，里面的值**必须指向已安装的语言包**（`HKLM\SYSTEM\CurrentControlSet\Control\MUI\UILanguages`）。把 `ja-JP` 写在第一位而机器上只有 `zh-CN` 语言包时，开始菜单、设置、文件资源管理器会回退成英文或出现半截乱码；
- 它**必须注销或重启才生效**，不像时区改完就完；
- 而改它对 Claude 的判断**没有任何帮助** —— 原文点名的是时区，不是界面语言。

现在界面上「界面语言」一行是**只读展示**，让你能亲眼确认它没被动过。需要改请在 Windows 的「语言和区域」设置里改。

## 📌 注意事项

- **先关掉"自动设置时区"**：Windows 的 *设置 → 时间和语言 → 日期和时间 → 自动设置时区*
  会在联网后按你的出口 IP 把时区**自动改回去**，看起来就像"工具失效了"。切换前先关掉它
  （以及"自动设置时间"里的时区同步）。这是最容易被误判成 bug 的场景。
- **浏览器必须先完全退出**：运行中的 Chromium 会在退出时用内存里的配置覆盖磁盘上的 `Preferences`，把我们的改动**静默回滚掉**。所以工具在检测到浏览器正在运行时会**直接跳过**浏览器语言设置并明确提示，而不是先写再提醒你重启。完全退出浏览器后再点一次即可。
- **`restore` 不还原界面语言**：本工具已不写这个键，所以没有需要还原的东西（旧备份里若记录了该字段，会被忽略并提示）。
- **还原不删除备份**：`restore` 之后备份文件仍然保留，所以「一键恢复」始终还原到**最初**切换前的状态，而不是上一次切换前。若你想以"当前状态"为新的还原点，请手动删除 `backup.json` 再切换一次。
- **浏览器语言的备份/还原是"粗粒度"的**：备份只记录**第一个命中的**浏览器 Profile 的语言值
  （Chrome → Edge → Brave → Chromium → Vivaldi 顺序），还原时却会把该值写到**所有**浏览器的
  **所有** Profile。如果你有多个 Profile 且各自语言不同，还原会把它们统一成同一个值。
  单个 Profile 的原始值另有一份 `.bak` 留在同目录（见下），可用于手动核对。
- **区域格式映射是白名单**：画像表里没有的 culture 会明确报错，而不是悄悄写成 en-US —— 那会让 `Locale` 和 `LocaleName` 两个注册表值自相矛盾，比直接失败更难排查。
- **不支持 Firefox**：只处理 Chromium 系浏览器。Firefox 的语言存在 `prefs.js`，机制不同，请自行到 `about:config` 调 `intl.accept_languages`。
- **备份位置**：`%LOCALAPPDATA%\ClaudeFingerprint\backup.json`（主题偏好在同目录 `theme.txt`）。已有备份时不会被覆盖，始终保留"切换前"的原始状态。
- **`.bak` 文件不是自动还原点**：改写浏览器 `Preferences` 前，会在同目录留一份
  `Preferences.<进程号>.bak`。本工具**不会**读它，它只是给你手工核对/恢复用的原始副本。
  它带进程号且每次只写一次，属正常现象，介意可自行删除。
- **风险分只统计本工具能改的两项**（时区 50 分 + 区域格式 15 分，满分 65）。分数变 0 **不代表你在 Claude 眼里就是干净的** —— 它只是说"这两项本地特征已经规避"，`ANTHROPIC_BASE_URL`、NTP、DNS、出口 IP 都不在分数里。

## ⚠️ 切换后必做

1. **重启 Claude Code** —— 它是常驻进程，不重启不会重新读时区。
2. **完全重启浏览器**（如果刚才因为浏览器在运行跳过了语言设置，需要退出后重新点一次）。
3. 去 <https://ip.net.coffee/claude/> 刷新，看"设备信息"卡片里 **时区**那行是否变绿/不再黄标。
4. 顺手检查 `ANTHROPIC_BASE_URL`：`echo $env:ANTHROPIC_BASE_URL`（为空或 `api.anthropic.com` 才正常）。
5. 过一段时间回来复查一次时区 —— 若被系统自动改回去了，说明"自动设置时区"没关（见注意事项第一条）。

## 🖥️ 界面说明

- 📋 **当前指纹卡** —— 时区 / 本地时间 / 区域语言 / 系统区域(ACP·Geo) / 浏览器语言 / 界面语言(只读)。
  在启动时、每次操作完成后、以及点「刷新」时读取（不是持续轮询）
- 🎨 **风险徽章** —— 本地指纹风险分 + 等级（高危红 / 中危橙 / 低危灰 / 安全绿）
- 🌸 **新加坡** 大按钮 —— UTC+8，时钟与北京时间零差异
- 🌴 **美国加州** 大按钮 —— Pacific Time，配加州纯净 IP 用
- ⏪ **一键恢复** 大按钮 —— 还原到切换前的原始状态
- 🎀 **其他** —— 台北 / 东京 / 纽约 / 上海(默认) / 刷新 / 打开检测页
- 📝 **操作日志** —— 每一步改了什么都写清楚，没成功的项会明确标 ✗
- 🌗 右上角可切换深色 / 浅色主题（偏好会记住）

## 🔨 从源码构建

需要 **Rust 工具链**（[rustup](https://rustup.rs/)，edition 2021）与
**PowerShell 7**（仅打包脚本需要；Windows 自带的 5.1 不满足 `#requires -Version 7.0`）。
首次构建要联网拉依赖。仅支持 64 位 Windows。

```powershell
cd rust
cargo test --release      # 35 个纯函数单元测试
cargo build --release     # 产物: rust\target\release\claude-fingerprint.exe
```

打包成可分发目录（exe + 启动器 + 说明 + LICENSE + SHA256SUMS）：

```powershell
# 在仓库根目录执行：
.\rust\package.ps1
# 或者已经 cd 进 rust/ 的话：
.\package.ps1
```

脚本会依次：跑单元测试 → 编译（并抹掉二进制里的构建机路径）→ 校验产物不比源码旧 →
写入 SHA256SUMS → 自检有无本机路径残留。**测试不通过或产物落后于源码时会拒绝打包。**

> **为什么不把 exe 提交进仓库**：二进制无法审计，而这是个要读你注册表和浏览器配置的程序。
> 请从源码构建，或从**发布页（Releases）** 下载并核对那里给出的 SHA256。
> 目前仓库尚未发布任何 Release，所以现阶段请自行构建。

## 🧪 已知局限

- 北美夏令时只实现了 **2007 年起**的美国规则，且只用于界面上的时钟对照条，北美以外的 DST 规则没考虑。时区本身由 Windows 负责，显示时间永远正确。
- 浏览器语言只改 `intl.accept_languages`，不改 Chromium 的界面语言（`Local State` 里的 `intl.app_locale`）。
- 浏览器语言**不是逐 Profile 精确还原**（见注意事项）。
- 时区切换是**系统级**的，会影响机器上所有程序（这正是目的，但请知悉）。
- 单元测试只覆盖**纯函数**（校验、映射、夏令时边界、风险分）。真正改系统的路径
  （注册表写入、`tzutil`、`Preferences` 原子写）没有自动化测试，需手工验证。

## 🗑️ 卸载与手动还原

**备份丢了也能恢复**，请按下面任选其一。

**方式一：用工具还原**
```powershell
.\claude-fingerprint.exe restore
```

**方式二：手动改回**
```powershell
# 时区（换成你原来的，例如中国大陆）
tzutil /s "China Standard Time"

# 区域格式（PowerShell 7）
Set-Culture zh-CN

# 浏览器语言：手动改，或删掉 Profile 里的 Preferences.<pid>.bak 后
# 把同目录的 Preferences 恢复回来（先完全退出浏览器）
```

**彻底卸载**
1. 先 `restore` 还原设置（否则时区会一直停在你切走的那个）。
2. 删除 `%LOCALAPPDATA%\ClaudeFingerprint\`（里面是 `backup.json` 与 `theme.txt`）。
3. 删除程序所在目录（`dist\` 或你解压的位置）。
4. 检查浏览器 Profile 目录里是否残留 `Preferences.<pid>.bak` / `.tmp` 文件，可一并删除。
5. 本工具**不写**系统服务、计划任务或开机项，无需额外清理。

## ⚖️ 免责声明与许可

- 本工具修改**系统级时区设置**，机器上所有程序都受影响。请确认你理解这个改动。
- 它**只改本机特征，不改变你的网络出口**：不做代理、不隐藏 IP、不检测 DNS/WebRTC/NTP。
  **这可能违反相关服务的使用条款，也可能仍被目标服务识别出来**，后果由使用者自行承担。
- 文中引用的"Claude Code 读取系统时区"机制来自**社区逆向分析与爆料**，尚未经
  Anthropic 官方证实，仅供参考。
- 使用者需自行承担使用后果。
- 许可证：[MIT](../LICENSE)（打包分发时该链接会被改写为同目录的 `LICENSE`）。
