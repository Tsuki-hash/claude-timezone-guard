# Claude 指纹切换器

Windows 桌面小工具：一键切换**系统时区**（附区域格式、浏览器语言），用来减少本机
暴露给 Claude Code 的「中国大陆」特征 —— 依据社区爆料，它疑似读取**系统时区**
（而非 IP）来判断用户所在地。

依据分析：<https://ip.net.coffee/claude/timezone.html>

> 该机制来自社区逆向与爆料，**尚未经 Anthropic 官方证实**。本工具也**不能**让你
> 变得"安全"：它只改本机特征，不管你的出口 IP。详见下面的「能力边界」。

- 技术栈：**Rust + egui**
- 形态：图形界面 + 命令行，同一个 exe

## 📖 文档

**完整使用手册见 [rust/README.md](rust/README.md)** —— 安装、命令行、画像表、
它到底改了哪些注册表、以及「切换完不等于安全」必须自己检查的事项，都在那里。

**项目评审报告见 [docs/评审报告.md](docs/评审报告.md)** —— 一次全面评审的完整记录：
发现的问题、修复方式、未处理项、以及经核实"看似有问题但其实正确"的地方。

## ⚡ 30 秒上手

**方式一：下载预编译版**

从 [Releases](https://github.com/Tsuki-hash/claude-timezone-guard/releases) 下载
`claude-fingerprint.exe`，建议先用 `Get-FileHash` 核对那里给出的 SHA256：

```powershell
Get-FileHash .\claude-fingerprint.exe -Algorithm SHA256

.\claude-fingerprint.exe status                 # 只看状态，不改任何设置
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
  src/core.rs      时区 / 区域语言 / 注册表 / 进程探测
  src/browser.rs   浏览器 Preferences 读写 + 备份还原
  src/ui.rs        egui 界面与主题
  src/main.rs      命令行入口与程序启动
  README.md        使用手册（完整版）
  package.ps1      打包成可分发目录
docs/            项目进度与已知问题记录
dist/            本地打包输出（不入库；发布产物见 Releases）
```

## ⚠️ 先看能力边界

**本工具只改本机特征**：系统时区、区域格式、Chromium 系浏览器的语言设置。

它**不改变你的网络出口** —— 不做代理、不隐藏 IP、不检测 DNS / WebRTC / NTP 泄露。
原文提到的第二条识别路径（`ANTHROPIC_BASE_URL` 指向中转站/国内域名）**不在本工具能力范围内**，
需要你自己检查。

所以：**改完不等于安全**，仍可能被识别。最终以 <https://ip.net.coffee/claude/> 的复测结果为准。

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
