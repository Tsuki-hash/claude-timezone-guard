# Claude 指纹切换器

Windows 桌面小工具：一键切换**系统时区**（附区域格式、浏览器语言），用于规避
Claude Code 通过系统时区识别中国大陆用户的特征。

依据分析：<https://ip.net.coffee/claude/timezone.html>

- 技术栈：**Rust + egui**
- 形态：图形界面 + 命令行，同一个 exe

## 📖 文档

**完整使用手册见 [rust/README.md](rust/README.md)** —— 安装、命令行、画像表、
它到底改了哪些注册表、以及「切换完不等于安全」必须自己检查的事项，都在那里。

## ⚡ 30 秒上手

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

不带参数运行即启动图形界面：

```powershell
.\target\release\claude-fingerprint.exe
```

## 🗂️ 目录结构

```
rust/            源码（Rust 版，唯一在维护的实现）
  src/core.rs      时区 / 区域语言 / 注册表 / 进程探测
  src/browser.rs   浏览器 Preferences 读写 + 备份还原
  src/ui.rs        egui 界面与主题
  src/main.rs      CLI 入口与程序启动
  README.md        使用手册（完整版）
  package.ps1      打包成可分发目录
dist/            打包输出（.exe 不入库，请从源码构建）
```

## ⚠️ 两句话记住它的边界

1. **它不是代理** —— 不隐藏 IP，不检测 DNS/WebRTC/NTP 泄露。
2. **原文有两条泄露路径，它只管一条** —— 系统时区能改；`ANTHROPIC_BASE_URL`
   指向中转站/国内域名这条，工具管不了，得你自己查。

改完请去 <https://ip.net.coffee/claude/> 复测。
