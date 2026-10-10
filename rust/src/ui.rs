//! UI 层：egui 界面 + 主题
//! 地区设置工作台：状态概览 → 选择地区 → 变更预览 → 明确应用。
//! 主页与详情页均支持滚动；浅深色共用语义令牌，布局由无头 egui 测试覆盖。

use crate::browser::{
    backup_state, load_backup, restore_browser_langs, save_backup, set_browser_language,
    BackupState,
};
use crate::core::*;
use crate::remote::{self, RemoteEstimate};
use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[cfg(feature = "ui-preview")]
mod preview;
mod workbench;
pub const WINDOW_SIZE: [f32; 2] = [1120.0, 840.0];
pub const MIN_WINDOW_SIZE: [f32; 2] = [760.0, 660.0];

// ============================================================
// 主题
// ============================================================
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Theme {
    Dark,
    Light,
}

/// 主题偏好持久化到 %LOCALAPPDATA%\ClaudeFingerprint\theme.txt
/// 让下次启动保持用户选的主题（默认浅色）
fn theme_pref_path() -> Option<std::path::PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            std::path::PathBuf::from(s)
                .join("ClaudeFingerprint")
                .join("theme.txt")
        })
}

fn load_theme_pref() -> Option<Theme> {
    let p = theme_pref_path()?;
    let t = std::fs::read_to_string(p).ok()?;
    match t.trim() {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

fn save_theme_pref(t: Theme) {
    if let Some(p) = theme_pref_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let s = match t {
            Theme::Dark => "dark",
            Theme::Light => "light",
        };
        let _ = std::fs::write(p, s);
    }
}

/// 设计令牌：所有颜色集中在这里，两个主题共用同一套语义。
/// 视觉语言对齐「明亮工作台」：柔和蓝灰底 + 纯白卡片 + 品牌蓝强调 + 彩色数据。
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: egui::Color32,         // 页面底
    pub panel: egui::Color32,      // 卡片
    pub well: egui::Color32,       // 凹陷区（迷你瓦片/日志）
    pub line: egui::Color32,       // 卡片描边
    pub fg: egui::Color32,         // 主文字
    pub fg_dim: egui::Color32,     // 次文字
    pub fg_mute: egui::Color32,    // 弱文字（标签）
    pub accent: egui::Color32,     // 品牌蓝（CTA / hover 强调）
    pub accent_ink: egui::Color32, // 强调按钮前景
    pub warn: egui::Color32,       // 中危
    pub danger: egui::Color32,     // 高危
    pub ok: egui::Color32,         // 安全
}

impl Palette {
    pub fn for_theme(t: Theme) -> Self {
        match t {
            // 深色：深海军夜色（参考 dark 令牌），卡片是抬升的蓝灰面
            Theme::Dark => Self {
                bg: egui::Color32::from_rgb(0x12, 0x16, 0x21),
                panel: egui::Color32::from_rgb(0x1B, 0x20, 0x2E),
                well: egui::Color32::from_rgb(0x22, 0x29, 0x3A),
                line: egui::Color32::from_rgb(0x30, 0x38, 0x4B),
                fg: egui::Color32::from_rgb(0xF0, 0xF6, 0xFF),
                fg_dim: egui::Color32::from_rgb(0xA7, 0xB8, 0xCD),
                fg_mute: egui::Color32::from_rgb(0x7A, 0x8C, 0xA3),
                accent: egui::Color32::from_rgb(0xA5, 0xAE, 0xFF),
                accent_ink: egui::Color32::from_rgb(0x15, 0x1A, 0x30),
                warn: egui::Color32::from_rgb(0xFF, 0xAB, 0x6B),
                danger: egui::Color32::from_rgb(0xFF, 0x8A, 0x8A),
                ok: egui::Color32::from_rgb(0x4D, 0xDC, 0x95),
            },
            // 浅色：柔和蓝灰底 + 纯白卡片（参考 light 令牌）
            Theme::Light => Self {
                bg: egui::Color32::from_rgb(0xF5, 0xF6, 0xFA),
                panel: egui::Color32::from_rgb(0xFF, 0xFF, 0xFF),
                well: egui::Color32::from_rgb(0xF1, 0xF3, 0xF9),
                line: egui::Color32::from_rgb(0xE3, 0xE7, 0xF0),
                fg: egui::Color32::from_rgb(0x23, 0x2B, 0x40),
                fg_dim: egui::Color32::from_rgb(0x60, 0x6A, 0x80),
                fg_mute: egui::Color32::from_rgb(0x74, 0x7E, 0x93),
                accent: egui::Color32::from_rgb(0x58, 0x63, 0xD8),
                accent_ink: egui::Color32::WHITE,
                warn: egui::Color32::from_rgb(0xC5, 0x6F, 0x06),
                danger: egui::Color32::from_rgb(0xDC, 0x33, 0x23),
                ok: egui::Color32::from_rgb(0x08, 0xA6, 0x74),
            },
        }
    }

    /// 风险等级对应的颜色
    pub fn risk(&self, score: u32) -> egui::Color32 {
        if score >= 50 {
            self.danger
        } else if score >= 20 {
            self.warn
        } else if score > 0 {
            self.fg_dim
        } else {
            self.ok
        }
    }
}

// ============================================================
// 应用状态
// ============================================================
/// 参考检测页地址（硬编码常量，不可配置 —— 避免任何拼接/注入面）
pub const DETECT_PAGE_URL: &str = "https://ip.net.coffee/claude/";

#[derive(Clone, Copy, PartialEq)]
pub enum LogKind {
    Info,
    Ok,
    Warn,
    Bad,
}

/// 页面层级：主页呈现状态、变更预览与主要操作，窄窗口支持滚动；
/// 完整读数、待处理清单与日志收进详情页。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page {
    Home,
    Detail,
    Log,
}

pub struct App {
    pub theme: Theme,
    /// 已经写进 egui 全局样式的主题。None = 还没应用过。
    /// 用来把 apply_theme 从"每帧"降为"仅主题变化时"。
    applied_theme: Option<Theme>,
    /// 当前页面（主页 / 详情页）
    pub page: Page,
    selected_profile: Profile,
    restore_pending: bool,
    log_filter: u8,
    operation_label: String,
    #[cfg(feature = "ui-preview")]
    preview_capture: Option<preview::PreviewCapture>,
    /// 圆角窗口属性是否已设置（只需一次）
    corners_applied: bool,
    pub fp: Fingerprint,
    pub log: Vec<(String, LogKind)>,
    pub busy: bool,
    /// 备份状态三态（无 / 可用 / 损坏）。
    /// 早先用 bool，把"文件损坏"显示成"没有备份"，用户会照提示去"先切换一次"，
    /// 而真实原因是文件坏了 —— 排查方向完全错。
    pub backup: BackupState,
    /// 当前在跑的任务的接收端。**不长期持有 Sender** —— 见 poll_tasks 的说明。
    task_rx: Option<Receiver<TaskResult>>,
    /// 出口侧估算（FuckClaude /api/check）的最近一次结果；None = 未查询。
    /// 与主任务通道分离：5s 的网络请求不该把切换按钮一起锁死。
    remote: Option<Result<RemoteEstimate, String>>,
    remote_rx: Option<Receiver<Result<RemoteEstimate, String>>>,
    remote_busy: bool,
}

pub enum TaskResult {
    Done {
        lines: Vec<(String, LogKind)>,
    },
    Refreshed {
        fp: Box<Fingerprint>,
        backup: BackupState,
    },
}

impl App {
    pub fn new() -> Self {
        let fp = read_fingerprint();
        // 默认浅色（技术文档纸），深色用右上角按钮切换
        let theme = load_theme_pref().unwrap_or(Theme::Light);
        let backup = backup_state();
        let mut app = Self {
            theme,
            applied_theme: None,
            page: Page::Home,
            selected_profile: PROFILES
                .iter()
                .find(|p| p.tz == fp.tz_id)
                .map(|p| p.key)
                .unwrap_or(Profile::Singapore),
            restore_pending: false,
            log_filter: 0,
            operation_label: String::new(),
            #[cfg(feature = "ui-preview")]
            preview_capture: None,
            corners_applied: false,
            log: vec![(format!("就绪 · 当前时区 {}", fp.tz_id), LogKind::Info)],
            busy: false,
            backup: backup.clone(),
            task_rx: None,
            remote: None,
            remote_rx: None,
            remote_busy: false,
            fp,
        };
        match &backup {
            BackupState::Missing => {
                app.push_log("就绪 · 选择地区，预览后应用设置".into(), LogKind::Info)
            }
            BackupState::Ready => {
                app.push_log("就绪 · 原设置已备份，可随时恢复".into(), LogKind::Info)
            }
            BackupState::Broken(e) => {
                app.push_log(format!("⚠ 备份文件无法使用: {}", e), LogKind::Bad)
            }
        }
        #[cfg(feature = "ui-preview")]
        app.configure_preview();
        app
    }

    /// 备份是否可用于还原
    fn backup_ready(&self) -> bool {
        matches!(self.backup, BackupState::Ready)
    }

    fn refresh_backup_state(&mut self) {
        let st = backup_state();
        if let BackupState::Broken(e) = &st {
            // 只在状态变成损坏时提示一次，避免每次刷新都刷屏
            if !matches!(self.backup, BackupState::Broken(_)) {
                self.push_log(format!("⚠ 备份文件无法使用: {}", e), LogKind::Bad);
            }
        }
        self.backup = st;
    }

    /// 重读指纹与备份状态。所有「刷新」入口（状态卡按钮 / 底部按钮 / F5）
    /// 共用这一个方法，保证指纹、备份状态、日志三项总是一起更新。
    pub fn refresh_fingerprint(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.operation_label = "正在刷新本机环境".into();
        self.spawn_task(|tx| {
            let _ = tx.send(TaskResult::Refreshed {
                fp: Box::new(read_fingerprint()),
                backup: backup_state(),
            });
        });
    }

    fn push_log(&mut self, msg: String, kind: LogKind) {
        self.log.push((msg, kind));
    }

    /// 开一个后台任务，并把接收端存起来。
    ///
    /// 关键设计：**每次任务新建一个 channel，App 只保存 Receiver，不保存 Sender**。
    /// 早先 App 长期持有 `tx`，而 mpsc 只有在「所有 Sender 都被丢弃」时才返回
    /// `Disconnected` —— 于是工作线程 panic、它的 tx 被丢弃之后，App 那份 tx 还活着，
    /// `try_recv()` 只会一直返回 `Empty`，`busy` 永远停在 true，界面彻底卡死。
    /// 不持有 Sender 后，worker 一死（含 panic）channel 立刻断开，poll_tasks
    /// 就能收到 `Disconnected` 复位状态。
    fn spawn_task<F>(&mut self, work: F)
    where
        F: FnOnce(Sender<TaskResult>) + Send + 'static,
    {
        let (tx, rx) = channel();
        self.task_rx = Some(rx);
        thread::spawn(move || work(tx));
    }

    /// 请求出口侧估算：独立通道 + 独立忙碌位，5s 的网络请求不锁切换按钮。
    fn request_remote(&mut self) {
        if self.remote_busy {
            return;
        }
        self.remote_busy = true;
        let (tx, rx) = channel::<Result<RemoteEstimate, String>>();
        self.remote_rx = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(remote::fetch_remote_estimate());
        });
    }

    fn poll_tasks(&mut self) {
        // 出口侧估算结果（独立通道）
        if let Some(rx) = self.remote_rx.as_ref() {
            match rx.try_recv() {
                Ok(result) => {
                    match &result {
                        Ok(est) => self.push_log(
                            format!("出口侧估算：{} · Geo {}", est.headline(), est.geo_summary()),
                            LogKind::Info,
                        ),
                        Err(e) => self.push_log(format!("出口侧估算失败：{e}"), LogKind::Warn),
                    }
                    self.remote = Some(result);
                    self.remote_busy = false;
                    self.remote_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.remote_busy = false;
                    self.remote_rx = None;
                    self.remote = Some(Err("查询线程异常退出，请重试".into()));
                    self.push_log(
                        "内部错误：出口侧查询线程异常退出，请重试。".into(),
                        LogKind::Bad,
                    );
                }
            }
        }
        let Some(rx) = self.task_rx.as_ref() else {
            return;
        };
        // 每个分支都直接 return，所以这里不是循环：
        // 一次只可能有一个任务，而 Done/Disconnected 都代表它已经结束。
        match rx.try_recv() {
            Ok(TaskResult::Refreshed { fp, backup }) => {
                self.fp = *fp;
                self.backup = backup;
                self.busy = false;
                self.task_rx = None;
                self.push_log("已刷新本机环境与备份状态".into(), LogKind::Info);
            }
            Ok(TaskResult::Done { lines }) => {
                for (m, k) in lines {
                    self.push_log(m, k);
                }
                self.busy = false;
                self.task_rx = None;
                self.fp = read_fingerprint();
                self.refresh_backup_state();
            }
            // Remote 走独立通道（remote_rx），不会出现在主通道里
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            // Disconnected = 工作线程已退出却没发结果，即它 panic 了。
            // 必须复位 busy，否则所有按钮从此失效、界面看似卡死。
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.busy = false;
                self.task_rx = None;
                self.push_log(
                    "内部错误：后台任务异常退出，本次操作可能未完成。请重新操作，若反复出现请反馈。"
                        .into(),
                    LogKind::Bad,
                );
            }
        }
    }

    pub fn switch_to(&mut self, p: Profile) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.operation_label = format!("正在应用 {} 设置", info(p).label);
        self.push_log(format!("── 切换到 {} ──", info(p).label), LogKind::Info);
        let inf = info(p);
        let tz = inf.tz.to_string();
        let culture = inf.culture.to_string();
        let bl = inf.browser_lang.to_string();

        self.spawn_task(move |tx| {
            let mut out: Vec<(String, LogKind)> = Vec::new();
            // 必须在备份之前声明：备份失败也是失败，早期版本把它声明在备份之后，
            // 导致备份没成功却依然报"完成"，用户以为有还原点。
            let mut failures = 0usize;

            // 浏览器必须先完全退出才能改 Preferences：运行中的 Chromium 会在
            // 退出时用内存里的配置覆盖磁盘，把我们的改动静默回滚掉。
            // 所以改成"先检测、在运行时直接拒绝写入"，而不是先写后提示。
            let running = running_browsers();
            let browser_busy = !running.is_empty();

            match save_backup() {
                Ok(Some(m)) => out.push((m, LogKind::Ok)),
                Ok(None) => {}
                Err(e) => {
                    out.push((format!("备份失败: {}", e), LogKind::Bad));
                    out.push((
                        "没有备份就无法一键还原，建议先解决备份问题再切换".into(),
                        LogKind::Bad,
                    ));
                    failures += 1;
                }
            }

            let before = get_timezone();
            match set_timezone(&tz) {
                Ok(_) => {
                    if before != tz {
                        out.push((format!("时区  {} → {}", short_tz(&before), tz), LogKind::Ok));
                    } else {
                        out.push((format!("时区已是 {}", tz), LogKind::Info));
                    }
                }
                Err(e) => {
                    out.push((format!("时区切换失败: {}", e), LogKind::Bad));
                    failures += 1;
                }
            }

            match set_culture(&culture) {
                Ok(_) => out.push((format!("区域语言  → {}", culture), LogKind::Ok)),
                Err(e) => {
                    out.push((format!("区域语言失败: {}", e), LogKind::Bad));
                    failures += 1;
                }
            }

            // 首选 UI 语言刻意不动（语言包缺失会让界面异常，且需注销才生效）
            out.push((
                "首选 UI 语言：保持原样（本工具不改这里）".into(),
                LogKind::Info,
            ));

            if browser_busy {
                out.push((
                    format!("浏览器正在运行（{}），已跳过语言设置", running.join(", ")),
                    LogKind::Warn,
                ));
                out.push((
                    "请完全退出浏览器后，再次应用相同地区设置即可写入语言".into(),
                    LogKind::Warn,
                ));
                failures += 1;
            } else {
                let (blog, bfail) = set_browser_language(&bl);
                for l in blog {
                    let k = if l.starts_with('✗') {
                        LogKind::Bad
                    } else {
                        LogKind::Ok
                    };
                    out.push((l, k));
                }
                failures += bfail;
            }

            out.push((
                "请重启 Claude Code（常驻进程不重读时区）".into(),
                LogKind::Warn,
            ));

            // 有失败就不说"完成"，别让用户以为全好了
            if failures > 0 {
                out.push((
                    format!("结束，但有 {} 项未完成，请查看详情日志", failures),
                    LogKind::Bad,
                ));
            } else {
                out.push(("完成 · 去检测页刷新复测".into(), LogKind::Ok));
            }

            let _ = tx.send(TaskResult::Done { lines: out });
        });
    }

    pub fn restore(&mut self) {
        if self.busy {
            return;
        }
        let b = match load_backup() {
            Ok(b) => b,
            Err(e) => {
                self.push_log(format!("还原中止: {}", e), LogKind::Bad);
                return;
            }
        };
        self.busy = true;
        self.operation_label = "正在恢复备份".into();
        self.push_log(format!("── 还原到 {} ──", b.culture), LogKind::Info);

        self.spawn_task(move |tx| {
            let mut out: Vec<(String, LogKind)> = Vec::new();
            let mut fails = 0;

            match set_timezone(&b.tz_id) {
                Ok(_) => out.push((format!("时区  → {}", b.tz_id), LogKind::Ok)),
                Err(e) => {
                    out.push((format!("✗ 时区还原失败: {}", e), LogKind::Bad));
                    fails += 1;
                }
            }
            match set_culture(&b.culture) {
                Ok(_) => out.push((format!("区域语言  → {}", b.culture), LogKind::Ok)),
                Err(e) => {
                    out.push((format!("✗ 区域语言还原失败: {}", e), LogKind::Bad));
                    fails += 1;
                }
            }
            // 备份里的 ui_langs 不再还原：本工具已改为不写首选 UI 语言，
            // 保留字段读取只为兼容旧备份。明确告知，不静默忽略。
            if b.ui_langs.is_some() {
                out.push((
                    "UI 语言：本工具不再改动（如需还原请在 Windows 语言设置里调整）".into(),
                    LogKind::Info,
                ));
            }
            // 浏览器语言还原：同样受"浏览器必须先退出"的限制，否则会被覆盖。
            // 优先用逐 Profile 精确还原（v2）；旧备份才退回"单值写全部"。
            let has_per_profile = !b.browser_langs.is_empty();
            let legacy_single = b.browser_lang.clone();
            if has_per_profile || legacy_single.is_some() {
                let running = running_browsers();
                if running.is_empty() {
                    let (blog, bfail) = if has_per_profile {
                        restore_browser_langs(&b.browser_langs)
                    } else {
                        out.push((
                            "这是旧版备份（只有单个语言值），按旧行为写到所有 Profile".into(),
                            LogKind::Warn,
                        ));
                        set_browser_language(legacy_single.as_deref().unwrap_or_default())
                    };
                    for line in blog {
                        let k = if line.starts_with('✗') {
                            LogKind::Bad
                        } else {
                            LogKind::Ok
                        };
                        out.push((line, k));
                    }
                    fails += bfail;
                } else {
                    out.push((
                        format!(
                            "浏览器正在运行（{}），已跳过浏览器语言还原",
                            running.join(", ")
                        ),
                        LogKind::Warn,
                    ));
                    out.push((
                        "请完全退出浏览器后，再点一次「一键恢复」".into(),
                        LogKind::Warn,
                    ));
                    fails += 1;
                }
            } else {
                out.push(("浏览器语言：备份时未检测到，保持原样".into(), LogKind::Info));
            }

            if fails > 0 {
                out.push((format!("还原结束，但有 {} 项失败", fails), LogKind::Bad));
            } else {
                out.push(("还原完成".into(), LogKind::Ok));
            }
            let _ = tx.send(TaskResult::Done { lines: out });
        });
    }
}

// ============================================================
// 主题应用
// ============================================================
/// 把 egui 全局样式对齐到当前主题的 Palette
fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let p = Palette::for_theme(theme);
    let mut style = (*ctx.style()).clone();

    match theme {
        Theme::Dark => style.visuals = egui::Visuals::dark(),
        Theme::Light => style.visuals = egui::Visuals::light(),
    }

    style.visuals.panel_fill = p.bg;
    style.visuals.window_fill = p.bg;
    style.visuals.extreme_bg_color = p.well;
    style.visuals.faint_bg_color = p.panel;
    style.visuals.widgets.inactive.bg_fill = p.panel;
    style.visuals.widgets.inactive.weak_bg_fill = p.panel;
    style.visuals.widgets.hovered.weak_bg_fill = p.well;
    style.visuals.widgets.active.weak_bg_fill = p.well;
    style.visuals.widgets.hovered.bg_fill = p.well;
    style.visuals.widgets.active.bg_fill = p.well;
    style.visuals.widgets.inactive.fg_stroke.color = p.fg_dim;
    style.visuals.widgets.active.fg_stroke.color = p.fg;

    // 圆角与间距：偏紧致，配合仪表盘密度
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.interact_size = egui::vec2(36.0, 32.0);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.line);
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, p.line);
    style.visuals.widgets.noninteractive.fg_stroke.color = p.fg;
    style.visuals.override_text_color = Some(p.fg);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, p.accent_ink);
    style.visuals.window_corner_radius = egui::CornerRadius::same(14);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
    style.spacing.button_padding = egui::vec2(10.0, 7.0);
    style.visuals.selection.bg_fill = p.accent;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, p.accent);
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5_f32, p.accent);

    ctx.set_style(style);
}

// ============================================================
// egui 入口
// ============================================================
impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(Palette::for_theme(self.theme).bg).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 圆角窗口只需设置一次（Win11 生效，Win10 静默忽略保持直角）
        if !self.corners_applied {
            apply_round_corners();
            self.corners_applied = true;
        }
        self.poll_tasks();
        let p = Palette::for_theme(self.theme);

        // 只在主题真正变化时重建全局样式：apply_theme 会 clone 整个 Style
        // （内含 BTreeMap 等堆分配）再写回全局，每帧做一次纯属浪费。
        if self.applied_theme != Some(self.theme) {
            apply_theme(ctx, self.theme);
            self.applied_theme = Some(self.theme);
        }

        // 时区对照条要显示"北京 HH:MM → 目标 HH:MM"。egui 默认只在有输入时重绘，
        // 空闲时分钟不会跳，钟点会一直停在启动那一刻 —— 而这恰是本工具的卖点。
        // 每秒请求一次重绘，代价很低。
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // 自绘标题栏（替代系统原生标题栏那一行）
        draw_titlebar(ctx, self, p);

        workbench::draw_shell(ctx, self, p);
        #[cfg(feature = "ui-preview")]
        self.capture_preview(ctx);

        // F5 手动刷新：挂了代理、改了环境变量或在外面跑了 CLI 之后，
        // 不用滚动找按钮，直接重读本机状态
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) && !self.busy {
            self.refresh_fingerprint();
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.restore_pending {
                self.restore_pending = false;
            } else {
                self.page = Page::Home;
            }
        }

        if self.busy || self.remote_busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

// ============================================================
// 区块
// ============================================================
/// Win11 圆角窗口：`with_decorations(false)` 的窗口默认直角，
/// 按窗口标题拿到 HWND 后向 DWM 申请圆角（DWMWCP_ROUND）。
/// Win10 无此属性，调用失败即静默忽略（保持直角，不影响功能）。
/// 只在首个绘制帧调用一次。不走 raw-window-handle：0.6 把取句柄的方法
/// 挪进了 deprecated 层，绕一圈不如按标题直取 HWND。
fn apply_round_corners() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut core::ffi::c_void;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut core::ffi::c_void,
            attr: u32,
            val: *const u32,
            size: u32,
        ) -> i32;
    }

    // 与 main.rs 里 run_native 的窗口名一致
    let title: Vec<u16> = OsStr::new("Claude 指纹切换器")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        return;
    }
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND, 4);
    }
}

/// 自绘标题栏 —— 应用标识、主题切换与窗口控制（最小化/最大化/关闭）合并为一行。
/// 窗口为 `with_decorations(false)`，不再有系统标题栏那一行。
fn draw_titlebar(ctx: &egui::Context, app: &mut App, p: Palette) {
    egui::TopBottomPanel::top("titlebar")
        .exact_height(40.0)
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(p.panel)
                .inner_margin(egui::Margin::symmetric(18, 6)),
        )
        .show(ctx, |ui| {
            // 拖拽 / 双击最大化：对整条标题栏先建一个交互响应。`ui.interact`
            // 不参与布局，且**后画的控件交互优先**——按钮不会被拖拽区抢点击。
            // 早先用 allocate_response 先占满矩形，按钮才点不到，于是退回了
            // 系统标题栏；换 interact 后自绘整行是安全的。
            let titlebar_rect = ui.max_rect();
            let drag = ui.interact(
                titlebar_rect,
                ui.id().with("titlebar_drag"),
                egui::Sense::click_and_drag(),
            );
            if drag.dragged() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }

            ui.spacing_mut().interact_size.y = 26.0;
            ui.spacing_mut().button_padding.y = 4.0;
            ui.horizontal(|ui| {
                ui.set_height(28.0);

                // 左：应用名，低声处理（12px、次级灰、不加图标不加粗）——
                // 「静仪」哲学：标题栏不许抢数据的戏，绿色方块装饰已移除
                ui.label(
                    egui::RichText::new("Claude 指纹切换器")
                        .size(11.5)
                        .color(p.fg_dim),
                );
                ui.label(
                    egui::RichText::new(env!("CARGO_PKG_VERSION"))
                        .size(11.0)
                        .color(p.fg_mute),
                );

                // 右：主题切换 + 窗口控制
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 关闭：hover 红，Windows 惯例
                    if win_ctrl_button(
                        ui,
                        WinIcon::Close,
                        p.bg,
                        p.fg_dim,
                        p.danger,
                        egui::Color32::WHITE,
                    ) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    // 最大化 / 还原（按当前状态切换图标）
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    let max_icon = if maximized {
                        WinIcon::Restore
                    } else {
                        WinIcon::Max
                    };
                    if win_ctrl_button(ui, max_icon, p.bg, p.fg_dim, p.well, p.fg) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if win_ctrl_button(ui, WinIcon::Min, p.bg, p.fg_dim, p.well, p.fg) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }

                    ui.add_space(4.0);
                    // 主题切换：现画太阳/月亮（评审 2-10——文字按钮与整体
                    // 图标语言不齐）。图标表示点击后去往的主题。
                    let to_dark = app.theme == Theme::Light;
                    let tip = if to_dark {
                        "切换到深色主题"
                    } else {
                        "切换到浅色主题"
                    };
                    if theme_icon_button(ui, p, to_dark)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        app.theme = match app.theme {
                            Theme::Dark => Theme::Light,
                            Theme::Light => Theme::Dark,
                        };
                        save_theme_pref(app.theme);
                    }
                });
            });
        });
}

/// 主题切换图标按钮：浅色→月亮，深色→太阳。画笔现画，与窗口控制按钮同款交互。
fn theme_icon_button(ui: &mut egui::Ui, p: Palette, to_dark: bool) -> egui::Response {
    let btn = egui::Button::new("")
        .min_size(egui::vec2(24.0, 22.0))
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(5.0);
    let resp = ui.add(btn);
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    let c = rect.center();
    let ink = p.fg_dim;
    let stroke = egui::Stroke::new(1.2_f32, ink);
    if to_dark {
        // 月亮：满圆被按钮底色圆遮出月牙
        painter.circle_filled(c, 5.5, ink);
        painter.circle_filled(c + egui::vec2(3.0, -2.0), 4.8, p.panel);
    } else {
        // 太阳：圆心 + 8 条射线
        painter.circle_filled(c, 3.6, ink);
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let dir = egui::vec2(a.cos(), a.sin());
            painter.line_segment([c + dir * 5.2, c + dir * 7.2], stroke);
        }
    }
    resp
}

/// 窗口控制图标（最小化 / 最大化 / 还原 / 关闭）
#[derive(Clone, Copy)]
enum WinIcon {
    Min,
    Max,
    Restore,
    Close,
}

/// 窗口控制按钮。图标用 painter 现画，不依赖字体字形 —— 「✕/❐」这类
/// 符号在部分字体环境里渲染成方块（实测出现过「— □ □」），现画则任何
/// 环境下形状一致，也更接近原生 Windows 窗口的观感。
fn win_ctrl_button(
    ui: &mut egui::Ui,
    icon: WinIcon,
    bg: egui::Color32,
    idle_fg: egui::Color32,
    hover_bg: egui::Color32,
    hover_fg: egui::Color32,
) -> bool {
    let btn = egui::Button::new("")
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .min_size(egui::vec2(32.0, 24.0));
    let resp = ui.add(btn);
    let hovered = resp.hovered();
    if hovered {
        ui.painter()
            .rect_filled(resp.rect, egui::CornerRadius::same(5), hover_bg);
    }
    let fg = if hovered { hover_fg } else { idle_fg };
    // 「还原」图标的两个方框互相遮挡，前框要用按钮当前底色先盖掉后框
    let clear = if hovered { hover_bg } else { bg };
    let r = egui::Rect::from_center_size(resp.rect.center(), egui::vec2(10.0, 10.0));
    let stroke = egui::Stroke::new(1.3_f32, fg);
    let painter = ui.painter();
    match icon {
        WinIcon::Min => {
            let y = r.center().y + 2.0;
            painter.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], stroke);
        }
        WinIcon::Max => {
            painter.rect_stroke(
                r,
                egui::CornerRadius::same(0),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        WinIcon::Restore => {
            let back = r.translate(egui::vec2(2.0, -2.0));
            painter.rect_stroke(
                back,
                egui::CornerRadius::same(0),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(r, egui::CornerRadius::same(0), clear);
            painter.rect_stroke(
                r,
                egui::CornerRadius::same(0),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        WinIcon::Close => {
            painter.line_segment([r.left_top(), r.right_bottom()], stroke);
            painter.line_segment([r.right_top(), r.left_bottom()], stroke);
        }
    }
    resp.clicked()
}
