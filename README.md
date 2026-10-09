# Claude 指纹切换器

Windows 桌面小工具：一键切换**系统时区**（附区域格式、浏览器语言），用来减少本机
暴露给 Claude Code 的「中国大陆」特征 —— 依据社区爆料，它疑似读取**系统时区**
（而非 IP）来判断用户所在地。

依据分析：<https://ip.net.coffee/claude/timezone.html> ·
检测信号参考：<https://github.com/LinXiaoTao/FuckClaude>

> 该机制来自社区逆向与爆料，**尚未经 Anthropic 官方证实**。本工具也**不能**让你
> 变得"安全"：它只改本机特征，不管你的出口 IP。详见下面的「能力边界」。

- 技术栈：**Rust + egui**
- 形态：图形界面 + 命令行，同一个 exe

## 📖 文档

**完整使用手册见 [rust/README.md](rust/README.md)** —— 安装、命令行、画像表、
它到底改了哪些注册表、以及「切换完不等于安全」必须自己检查的事项，都在那里。

## ⚡ 30 秒上手

**方式一：下载预编译版**

从 [Releases](https://github.com/Tsuki-hash/claude-timezone-guard/releases) 下载
`claude-fingerprint.exe`，建议先用 `Get-FileHash` 核对那里给出的 SHA256：

```powershell
Get-FileHash .\claude-fingerprint.exe -Algorithm SHA256

.\claude-fingerprint.exe status                 # 只看状态，不改任何设置
.\claude-fingerprint.exe status --remote        # 同上，追加出口 IP 侧的风险估算
.\claude-fingerprint.exe apply singapore        # 切换到新加坡（UTC+8，时钟零差异）
.\claude-fingerprint.exe restore                # 还原
```

不带参数运行即启动图形界面；双击 `claude-fingerprint.exe` 效果相同。

**方式二：从源码构建**

```powershell
cd rust
cargo build --release

# 体检（只读）
.\target\release\claude-fingerprint.exe status

# 切换到新加坡（UTC+8，时钟与北京时间零差异）
.\target\release\claude-fingerprint.exe apply singapore

# 还原
.\target\release\claude-fingerprint.exe restore
```

## 🗂️ 目录结构

```
rust/            源码（唯一在维护的实现）
  src/core.rs      时区 / 区域语言 / 字体与浏览器检测 / 注册表 / 风险评分
  src/browser.rs   浏览器 Preferences 读写 + 备份还原
  src/remote.rs    出口侧风险估算（FuckClaude /api/check 客户端）
  src/ui.rs        egui 界面与主题
  src/main.rs      命令行入口与程序启动
  README.md        使用手册（完整版）
  package.ps1      打包成可分发目录
  setup-signing.ps1 本机代码签名证书创建（SmartScreen 治理）
docs/            开发计划（plans/）与评审报告（reviews/，按日期归档）
dist/            本地打包输出（不入库；发布产物见 Releases）
```

## 🔍 检测原理

社区逆向分析指出：Claude Code 经 `ANTHROPIC_BASE_URL` 走中转时，会读取**系统时区**
与**中转 hostname**，并把结果**隐写**进 system prompt 的 `Today's date is …` 一行——

- 命中中国时区时，日期分隔符 `-` 变成 `/`；
- 撇号在 4 种视觉几乎相同的 Unicode 变体间切换，编码「域名清单 / AI 实验室关键词」是否命中。

也就是说，防住的关键是**时区名字符串本身**与**中转域名**，而不是 UTC 偏移量——这正是
本工具切「新加坡 / 台北（同为 UTC+8，时钟零差异）」也能规避的原理。

### 本工具的风险评分（6 项，合计 100）

| 风险项 | 权重 | 可修性 |
| --- | --- | --- |
| 中转地址（`ANTHROPIC_BASE_URL`） | 32 | 检测但不代改：改环境变量/配置文件即可归零 |
| 系统时区 | 30 | ✅ 本工具可修（大陆时区满分；港澳 60%；Taipei 等不计分） |
| 字体环境残留 | 18 | 检测但不可修：国产厂商字体 / 非标配中文字体的存在性 |
| 区域格式 | 10 | ✅ 本工具可修（zh-Hans 1.0 / 港澳繁体 0.5 / zh-TW 0） |
| NTP 校时服务器 | 5 | 检测但不可修：国内校时服务器会暴露真实时区 |
| 国产浏览器已装 | 5 | 半可修：卸载或避免日常使用（工具不代劳） |

时区分级的依据：社区验证 Claude 只读取 `Asia/Shanghai` / `Asia/Urumqi`；
**`Asia/Taipei` 不在判定范围**（台湾是 Anthropic 完全支持的地区）；港澳属受限地区记部分分。

### 出口侧估算

`status --remote`（或界面「出口侧 ›」）会调用
[FuckClaude](https://github.com/LinXiaoTao/FuckClaude) 的公开 `/api/check` 接口，
基于你的**出口 IP 与请求头**给出服务端估算。它与本机读数**口径不同、互为补充**：
本机分看「设备像不像中国用户」，出口侧看「IP 像不像中国用户」。

延伸阅读（环境纯化、注册支付避坑、申诉 SOP 等指南，含官方规则来源）：
<https://fuck-claude.vercel.app/zh/guides/> —— 情报与名单参考自
[LinXiaoTao/FuckClaude](https://github.com/LinXiaoTao/FuckClaude)（MIT）。

## ⚠️ 先看能力边界

**本工具能改**：系统时区、区域格式、Chromium 系浏览器的语言设置。

**能检测但不代改**：中转地址（`ANTHROPIC_BASE_URL`，改配置即可归零）、
NTP 校时服务器、字体环境残留、已安装的国产浏览器。

**完全不管**：网络出口 —— 不做代理、不隐藏 IP、不检测 DNS / WebRTC 泄露；
出口 IP 侧的风险请用 `status --remote` 或
<https://fuck-claude.vercel.app/zh/> 复测。

所以：**改完不等于安全**，仍可能被识别。

## ⚖️ 免责声明

- 本工具会修改**系统级时区设置**，机器上所有程序都会受影响。请自行确认你理解这个改动，
  并保留好还原手段（本工具自带 `restore`；备份丢失时见手册的「卸载与手动还原」一节）。
- 它只改本机特征，不改变网络出口。**这可能违反相关服务的使用条款，也可能被目标服务识别出来**，
  后果由使用者自行承担；请遵守所在地法律与相关服务条款。
- 本文档引用的「Claude Code 读取系统时区」机制来自**社区逆向分析与爆料**，
  截至撰写时**尚未经 Anthropic 官方证实**，仅供参考。
- 软件按 MIT 许可证以"现状"提供，不附带任何担保。

## 📄 许可证

[MIT](LICENSE)
