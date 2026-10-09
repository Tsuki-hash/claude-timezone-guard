//! UI 层：egui 界面 + 主题
//! 设计方向：「网络监控仪表盘」—— 工具本质是检测/修改系统指纹，
//! 美学取自网络诊断仪器（示波器、链路监控台），而非通用设置面板。
//!
//! 签名元素：时区对照条 —— 一条横向时间轴同时显示北京/当前/目标时区，
//! 把「时钟零差异」这件抽象的事变成看得见的刻度。

use crate::browser::{load_backup, save_backup, set_browser_language};
use crate::core::*;
use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

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

/// 设计令牌：所有颜色集中在这里，两个主题共用同一套语义
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: egui::Color32,      // 页面底
    pub panel: egui::Color32,   // 卡片
    pub well: egui::Color32,    // 凹陷区（日志/读数）
    pub line: egui::Color32,    // 分隔线 / 边框
    pub fg: egui::Color32,      // 主文字
    pub fg_dim: egui::Color32,  // 次文字
    pub fg_mute: egui::Color32, // 弱文字（标签）
    pub sig_sg: egui::Color32,  // 信号色 A —— 新加坡（安全/零差异）
    pub sig_us: egui::Color32,  // 信号色 B —— 加州（琥珀）
    pub warn: egui::Color32,    // 中危
    pub danger: egui::Color32,  // 高危
    pub ok: egui::Color32,      // 安全
}

impl Palette {
    pub fn for_theme(t: Theme) -> Self {
        match t {
            // 深色：仪器面板 —— 深蓝灰底，读数用高对比亮色
            Theme::Dark => Self {
                bg: egui::Color32::from_rgb(0x10, 0x13, 0x18),
                panel: egui::Color32::from_rgb(0x1A, 0x1E, 0x25),
                well: egui::Color32::from_rgb(0x0B, 0x0E, 0x12),
                line: egui::Color32::from_rgb(0x2A, 0x30, 0x3A),
                fg: egui::Color32::from_rgb(0xE8, 0xEE, 0xF5),
                fg_dim: egui::Color32::from_rgb(0xA8, 0xB4, 0xC4),
                fg_mute: egui::Color32::from_rgb(0x6C, 0x7A, 0x8C),
                sig_sg: egui::Color32::from_rgb(0x4E, 0xC9, 0x8E),
                sig_us: egui::Color32::from_rgb(0xE8, 0x9B, 0x3C),
                warn: egui::Color32::from_rgb(0xE8, 0x9B, 0x3C),
                danger: egui::Color32::from_rgb(0xE0, 0x5C, 0x5C),
                ok: egui::Color32::from_rgb(0x4E, 0xC9, 0x8E),
            },
            // 浅色：技术文档纸 —— 暖白底，同色系降饱和
            Theme::Light => Self {
                bg: egui::Color32::from_rgb(0xF4, 0xF6, 0xF8),
                panel: egui::Color32::from_rgb(0xFF, 0xFF, 0xFF),
                well: egui::Color32::from_rgb(0xEC, 0xEF, 0xF3),
                line: egui::Color32::from_rgb(0xD8, 0xDE, 0xE6),
                fg: egui::Color32::from_rgb(0x1A, 0x1E, 0x25),
                fg_dim: egui::Color32::from_rgb(0x44, 0x4E, 0x5C),
                fg_mute: egui::Color32::from_rgb(0x7A, 0x86, 0x94),
                sig_sg: egui::Color32::from_rgb(0x1E, 0x8E, 0x5E),
                sig_us: egui::Color32::from_rgb(0xB4, 0x6E, 0x14),
                warn: egui::Color32::from_rgb(0xB4, 0x6E, 0x14),
                danger: egui::Color32::from_rgb(0xC0, 0x39, 0x39),
                ok: egui::Color32::from_rgb(0x1E, 0x8E, 0x5E),
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

pub struct App {
    pub theme: Theme,
    /// 已经写进 egui 全局样式的主题。None = 还没应用过。
    /// 用来把 apply_theme 从"每帧"降为"仅主题变化时"。
    applied_theme: Option<Theme>,
    pub fp: Fingerprint,
    pub log: Vec<(String, LogKind)>,
    pub busy: bool,
    pub has_backup: bool,
    /// 当前在跑的任务的接收端。**不长期持有 Sender** —— 见 poll_tasks 的说明。
    task_rx: Option<Receiver<TaskResult>>,
}

pub enum TaskResult {
    Done { lines: Vec<(String, LogKind)> },
}

impl App {
    pub fn new() -> Self {
        let fp = read_fingerprint();
        // 默认浅色（技术文档纸），深色用右上角按钮切换
        let theme = load_theme_pref().unwrap_or(Theme::Light);
        let mut app = Self {
            theme,
            applied_theme: None,
            log: vec![(format!("就绪 · 当前时区 {}", fp.tz_id), LogKind::Info)],
            busy: false,
            has_backup: load_backup().is_ok(),
            task_rx: None,
            fp,
        };
        if !app.has_backup {
            app.push_log(
                "当前没有备份，切换后「一键恢复」才会启用".into(),
                LogKind::Info,
            );
        } else {
            app.push_log("检测到已有备份，一键恢复可用".into(), LogKind::Info);
        }
        app
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

    fn poll_tasks(&mut self) {
        let Some(rx) = self.task_rx.as_ref() else {
            return;
        };
        // 每个分支都直接 return，所以这里不是循环：
        // 一次只可能有一个任务，而 Done/Disconnected 都代表它已经结束。
        match rx.try_recv() {
            Ok(TaskResult::Done { lines }) => {
                for (m, k) in lines {
                    self.push_log(m, k);
                }
                self.busy = false;
                self.task_rx = None;
                self.fp = read_fingerprint();
                self.has_backup = load_backup().is_ok();
            }
            // Empty = 还没结果，下一帧再看
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
                    "请完全退出浏览器后，再点一次对应按钮即可写入语言".into(),
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
                    format!("结束，但有 {} 项未完成，请看上方 ✗ 行", failures),
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
            match &b.browser_lang {
                Some(l) => {
                    // 还原同样受浏览器运行限制，否则会被浏览器退出时覆盖
                    let running = running_browsers();
                    if running.is_empty() {
                        let (blog, bfail) = set_browser_language(l);
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
                }
                None => out.push(("浏览器语言：备份时未检测到，保持原样".into(), LogKind::Info)),
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
    style.visuals.widgets.hovered.bg_fill = p.well;
    style.visuals.widgets.active.bg_fill = p.well;
    style.visuals.widgets.inactive.fg_stroke.color = p.fg_dim;
    style.visuals.widgets.active.fg_stroke.color = p.fg;

    // 圆角与间距：偏紧致，配合仪表盘密度
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(9.0, 5.0);

    ctx.set_style(style);
}

// ============================================================
// egui 入口
// ============================================================
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        // 内容自然展开（不滚动），若高于当前窗口则把窗口撑高，保证一屏看全
        let resp = egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(18, 16)),
            )
            .show(ctx, |ui| {
                draw_header(ui, self, p);
                ui.add_space(11.0);

                draw_status_panel(ui, self, p);
                ui.add_space(11.0);

                draw_advice(ui, self, p);

                draw_exit_selector(ui, self, p);
                ui.add_space(11.0);

                draw_restore(ui, self, p);
                ui.add_space(11.0);

                draw_others(ui, self, p);
                ui.add_space(11.0);

                draw_log(ui, self, p);
            })
            .response
            .rect
            .height();

        let want = resp + 16.0 * 2.0 + 4.0;
        let cur = ctx.screen_rect().height();
        let w = ctx.screen_rect().width();
        // 内容比窗口高 -> 撑大窗口；矮太多 -> 收回。
        // 上限取屏幕高度，避免高 DPI 下窗口被撑到屏幕外（日志框被切）。
        let max_h = (cur - 20.0).max(700.0);
        if (want > cur + 2.0 && want < max_h) || (want < cur - 40.0 && cur > 700.0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, want)));
        }

        if self.busy {
            ctx.request_repaint();
        }
    }
}

// ============================================================
// 区块
// ============================================================
/// 工具条 —— 应用标识 + 主题切换。
/// 原生标题栏保留（自绘的在 egui 0.31 上拖拽区会抢按钮，关闭点不到），
/// 这里只做一条紧凑的内联工具条，让标识和主题切换贴近内容而非散在窗口角落。
fn draw_titlebar(ctx: &egui::Context, app: &mut App, p: Palette) {
    // 只画标识与主题按钮；窗口控制交给系统标题栏
    egui::TopBottomPanel::top("titlebar")
        .frame(
            egui::Frame::NONE
                .fill(p.bg)
                .inner_margin(egui::Margin::symmetric(18, 6)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // 应用标识：小色块 + 名称
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(13.0, 13.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(4), p.sig_sg);
                ui.label(
                    egui::RichText::new("Claude 指纹切换器")
                        .size(11.5)
                        .strong()
                        .color(p.fg),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, hover) = match app.theme {
                        Theme::Dark => ("浅色", "切换到浅色主题"),
                        Theme::Light => ("深色", "切换到深色主题"),
                    };
                    let btn =
                        egui::Button::new(egui::RichText::new(label).color(p.fg_dim).size(11.0))
                            .fill(p.panel)
                            .stroke(egui::Stroke::new(1.0_f32, p.line))
                            .corner_radius(6.0);
                    if ui
                        .add_sized([46.0, 22.0], btn)
                        .on_hover_text(hover)
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

fn draw_header(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    ui.horizontal(|ui| {
        // 左侧：标题 + 副标题（主题/窗口按钮已移到自绘标题栏）
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("Claude 指纹切换器")
                    .size(20.0)
                    .strong()
                    .color(p.fg),
            );
            ui.label(
                egui::RichText::new("时区 · 区域语言 · 浏览器语言")
                    .size(10.5)
                    .color(p.fg_mute),
            );
        });
    });
    let _ = app;
}

/// 当前状态卡 —— 仪器读数风格：等宽数字 + 细分隔线
fn draw_status_panel(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let f = &app.fp;
    let (score, level) = risk_score(f);
    let risk_color = p.risk(score);

    egui::Frame::NONE
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(15, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            // 卡头：标题 + 状态点
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("当前指纹")
                        .strong()
                        .size(12.5)
                        .color(p.fg),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (dot, txt) = if f.is_china_tz {
                        (p.danger, "中国大陆特征")
                    } else {
                        (p.ok, "已规避")
                    };
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.5, dot);
                    ui.label(egui::RichText::new(txt).size(11.0).color(dot));
                });
            });

            ui.add_space(9.0);
            hairline(ui, p);
            ui.add_space(7.0);

            // 读数行
            reading(
                ui,
                "时区",
                &f.tz_id,
                p,
                Some(if f.is_china_tz { p.danger } else { p.ok }),
            );
            reading(ui, "本地时间", &f.now, p, None);
            reading(ui, "区域语言", &f.culture, p, None);
            reading(
                ui,
                "系统区域",
                &format!("ACP {} · Geo {}", f.sys_locale, f.geo_id),
                p,
                None,
            );
            reading(ui, "浏览器语言", &f.browser_lang, p, None);
            // 只读展示：让用户亲眼确认界面语言没被动过（曾经会写，现已移除）
            reading(ui, "界面语言", &f.ui_langs, p, Some(p.fg_mute));

            ui.add_space(9.0);
            hairline(ui, p);
            ui.add_space(7.0);

            // 原文第二条识别路径：本工具不改它，但必须让用户看见它
            let bu_txt = match &f.base_url {
                Some(u) => u.clone(),
                None => "未设置（走官方直连）".into(),
            };
            reading(
                ui,
                "中转地址",
                &bu_txt,
                p,
                Some(if f.proxy_like_base_url {
                    p.danger
                } else {
                    p.ok
                }),
            );
            if f.proxy_like_base_url {
                ui.label(
                    egui::RichText::new(
                        "⚠ 请求经过第三方地址，这是原文点名的第二条识别路径；且本工具管不了它",
                    )
                    .size(9.5)
                    .color(p.danger),
                );
                ui.add_space(2.0);
            }

            // NTP 校时：原文方案一点名的时区泄露口
            let ntp_txt = match &f.ntp_server {
                Some(s) => s.clone(),
                None => "未配置".into(),
            };
            reading(
                ui,
                "NTP 校时",
                &ntp_txt,
                p,
                Some(if f.ntp_leaks { p.warn } else { p.ok }),
            );
            if f.ntp_leaks {
                ui.label(
                    egui::RichText::new("⚠ 直连国内校时服务器，会向它暴露你的真实时区")
                        .size(9.5)
                        .color(p.warn),
                );
                ui.add_space(2.0);
            }

            ui.add_space(9.0);
            hairline(ui, p);
            ui.add_space(8.0);

            // 风险读数
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("指纹风险分")
                        .size(11.0)
                        .color(p.fg_mute),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("{}  ", level))
                            .size(11.0)
                            .color(risk_color),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}/{}", score, RISK_MAX))
                            .font(egui::FontId::monospace(15.0))
                            .strong()
                            .color(risk_color),
                    );
                });
            });

            // 分数为 0 也不等于"安全"：出口 IP / DNS / WebRTC 都不在本地可观测范围。
            // 这句常驻，避免用户把低分误读成"在 Claude 眼里干净"。
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new("分数只反映本机可观测项 · 不含出口 IP / DNS / WebRTC")
                    .size(9.5)
                    .color(p.fg_mute),
            );
        });
}

/// 时区对照条 —— 本工具的签名元素
/// 一条横向时间轴，标出北京 / 当前 / 目标三个时区的钟点，
/// 让「时钟零差异」变成看得见的刻度对齐。
fn draw_exit_selector(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let w = (ui.available_width() - 10.0) / 2.0;
    ui.horizontal(|ui| {
        exit_card(ui, app, w, Profile::Singapore, p);
        exit_card(ui, app, w, Profile::California, p);
    });
}

/// 单个出口卡片：色条 + 名称 + 时区 + 时钟对照
fn exit_card(ui: &mut egui::Ui, app: &mut App, w: f32, key: Profile, p: Palette) {
    let inf = info(key);
    let accent = match key {
        Profile::Singapore => p.sig_sg,
        Profile::California => p.sig_us,
        _ => p.fg_dim,
    };

    let resp = ui.add_sized(
        [w, 70.0],
        egui::Button::new("")
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.line)),
    );
    // hover 时点亮边框
    if resp.hovered() {
        ui.painter().rect_stroke(
            resp.rect,
            egui::CornerRadius::same(9),
            egui::Stroke::new(1.6_f32, accent),
            egui::StrokeKind::Inside,
        );
    }

    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    let x0 = rect.left();
    let top = rect.top();

    // 左侧竖向色条（4px）
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(x0 + 1.0, top + 1.0),
            egui::vec2(3.0, rect.height() - 2.0),
        ),
        egui::CornerRadius::same(0),
        accent,
    );

    let pad = 14.0;
    let cx = x0 + pad + 4.0;

    painter.text(
        egui::pos2(cx, top + 19.0),
        egui::Align2::LEFT_CENTER,
        inf.label,
        egui::FontId::proportional(15.0),
        p.fg,
    );
    painter.text(
        egui::pos2(cx, top + 35.0),
        egui::Align2::LEFT_CENTER,
        inf.sub,
        egui::FontId::proportional(10.0),
        p.fg_mute,
    );

    // 时钟对照：北京 ↔ 目标（按目标时区真实偏移，含 DST）
    let beijing = chrono::Utc::now() + chrono::Duration::hours(8);
    let target_off = target_utc_offset(key);
    let target = chrono::Utc::now().with_timezone(&target_off);
    let diff_h = (target_off.local_minus_utc() - 8 * 3600) as f32 / 3600.0;
    let diff_txt = if diff_h.abs() < 0.01 {
        "零差异".to_string()
    } else {
        format!("{:+.0}h", diff_h)
    };
    painter.text(
        egui::pos2(cx, top + 54.0),
        egui::Align2::LEFT_CENTER,
        format!(
            "北京 {}  →  {}  ({})",
            beijing.format("%H:%M"),
            target.format("%H:%M"),
            diff_txt
        ),
        egui::FontId::monospace(10.5),
        accent,
    );

    if resp.clicked() && !app.busy {
        app.switch_to(key);
    }
}

/// 「待处理」清单 —— 把风险分拆成"哪一项、能不能用本工具解决"。
///
/// 存在的意义：分数本身不足以指导行动。`ANTHROPIC_BASE_URL` 与 NTP 是权重最高的
/// 两项，但**本工具改不了**，必须明确区分"点按钮就能修"和"得你自己动手"，
/// 否则用户会以为切换完时区就等于分数归零。
fn draw_advice(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let f = &app.fp;
    let mut rows: Vec<(egui::Color32, String, String)> = Vec::new();

    if f.proxy_like_base_url {
        rows.push((
            p.danger,
            "中转地址".into(),
            format!(
                "把 ANTHROPIC_BASE_URL 改成空或官方 api.anthropic.com（本工具改不了）· {}",
                f.base_url_hint
            ),
        ));
        // 那个文件里通常还放着 ANTHROPIC_AUTH_TOKEN 之类的密钥。
        // 这里给一个"已打码长这样"的样例，提醒用户贴截图前先处理。
        rows.push((
            p.fg_dim,
            "注意".into(),
            format!(
                "该文件通常还含 ANTHROPIC_AUTH_TOKEN 等密钥，分享截图前请打码（形如 {}）",
                redact_secret("sk-7EXAMPLE-REDACTED-000000")
            ),
        ));
    }
    if f.ntp_leaks {
        rows.push((
            p.warn,
            "NTP 校时".into(),
            "把 Windows 时间服务换成境外 NTP，或让 NTP 走代理（本工具改不了）".into(),
        ));
    }
    if f.is_china_tz {
        rows.push((
            p.danger,
            "系统时区".into(),
            "点下面的「新加坡」或「台北」即可（UTC+8，时钟零差异）".into(),
        ));
    }
    if f.culture == "zh-CN" {
        rows.push((p.warn, "区域格式".into(), "点任一境外画像时一并修改".into()));
    }

    if rows.is_empty() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("✓ 本机可观测项均已规避")
                    .size(11.0)
                    .strong()
                    .color(p.ok),
            );
            ui.label(
                egui::RichText::new("（出口 IP / DNS / WebRTC 仍需自行检查）")
                    .size(9.5)
                    .color(p.fg_mute),
            );
        });
        ui.add_space(11.0);
        return;
    }

    egui::Frame::NONE
        .fill(p.well)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!("待处理 · {} 项", rows.len()))
                    .strong()
                    .size(11.5)
                    .color(p.fg),
            );
            ui.add_space(5.0);
            for (color, name, how) in rows {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    ui.label(
                        egui::RichText::new(format!("• {}", name))
                            .size(10.5)
                            .strong()
                            .color(color),
                    );
                    ui.label(egui::RichText::new(how).size(10.0).color(p.fg_dim));
                });
                ui.add_space(2.0);
            }
        });
    ui.add_space(11.0);
}

fn draw_restore(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let enabled = app.has_backup && !app.busy;
    let txt_color = if enabled { p.fg } else { p.fg_mute };
    let btn = egui::Button::new(
        egui::RichText::new("↩  一键恢复 · 还原到切换前")
            .size(12.5)
            .strong()
            .color(txt_color),
    )
    .fill(if enabled { p.panel } else { p.well })
    .stroke(egui::Stroke::new(
        1.0_f32,
        if enabled { p.line } else { p.well },
    ))
    .corner_radius(8.0)
    .min_size(egui::vec2(ui.available_width(), 34.0));

    let resp = ui.add(btn);
    let clicked = resp.clicked();
    if !enabled {
        resp.on_hover_text("还没有备份：先切换一次，这里才能还原");
    } else {
        // 备份路径/免责说明收进 hover：需要时才看，不占界面一行
        resp.on_hover_text(
            "还原到切换前的时区 / 区域语言 / 浏览器语言\n\
             备份位于 %LOCALAPPDATA%\\ClaudeFingerprint\\backup.json\n\
             本工具只改本机指纹，不代理 IP；界面语言不受影响",
        );
        if clicked {
            app.restore();
        }
    }
}

fn draw_others(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    ui.label(
        egui::RichText::new("其他出口")
            .strong()
            .size(12.0)
            .color(p.fg),
    );
    ui.add_space(6.0);

    // 单行紧凑 chips：4 个地区 + 2 个动作
    let others = [
        (Profile::Taipei, "台北"),
        (Profile::Tokyo, "东京"),
        (Profile::NewYork, "纽约"),
        (Profile::Shanghai, "上海"),
    ];
    let n = 6.0;
    let bw = (ui.available_width() - (n - 1.0) * 5.0) / n;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (key, label) in others {
            let btn = egui::Button::new(egui::RichText::new(label).color(p.fg_dim).size(11.0))
                .fill(p.panel)
                .stroke(egui::Stroke::new(1.0_f32, p.line))
                .corner_radius(6.0);
            if ui
                .add_sized([bw, 28.0], btn)
                .on_hover_text(info(key).sub)
                .clicked()
                && !app.busy
            {
                app.switch_to(key);
            }
        }
        let refresh = egui::Button::new(egui::RichText::new("刷新").color(p.fg_dim).size(11.0))
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.line))
            .corner_radius(6.0);
        if ui.add_sized([bw, 28.0], refresh).clicked() {
            app.fp = read_fingerprint();
            app.push_log("已刷新指纹状态".into(), LogKind::Info);
        }

        let web = egui::Button::new(egui::RichText::new("检测页").color(p.fg_dim).size(11.0))
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.line))
            .corner_radius(6.0);
        if ui.add_sized([bw, 28.0], web).clicked() {
            // 用 webbrowser crate（走 ShellExecuteW）而不是 `cmd /C start`：
            //   - `Command::new("cmd")` 是裸名，会按 CreateProcess 的搜索顺序
            //     （应用目录 → 当前目录 → System32 → …）找 cmd.exe，同目录或
            //     当前目录里的假 cmd.exe 能劫持它 —— 与本项目 sys_tool() 的
            //     绝对路径加固理念自相矛盾。
            //   - `cmd /C start <串>` 会重新解析该字符串，将来 URL 一旦变成
            //     可配置/可拼接，`&`、`^`、`"` 立刻变成命令注入面。
            // webbrowser 已在依赖树里（egui-winit 引入），直接用它的 API 最干净。
            match webbrowser::open(DETECT_PAGE_URL) {
                Ok(()) => app.push_log("已在浏览器打开检测页".into(), LogKind::Info),
                Err(e) => app.push_log(
                    format!("打开检测页失败: {} —— 请手动访问 {}", e, DETECT_PAGE_URL),
                    LogKind::Warn,
                ),
            }
        }
    });
}

/// 日志 —— 终端风格：等宽、左对齐、按级别着色
fn draw_log(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("操作日志")
                .strong()
                .size(12.5)
                .color(p.fg),
        );
        // 常驻角标：把原来独占一行的"重启提示"并到这里，既显眼又不占行
        ui.label(
            egui::RichText::new("切换后需重启 Claude Code 与浏览器")
                .size(10.0)
                .color(p.warn),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.busy {
                ui.label(egui::RichText::new("处理中…").size(10.5).color(p.warn));
            }
        });
    });
    ui.add_space(6.0);

    egui::Frame::NONE
        .fill(p.well)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // 填满剩余高度：内容多时内部滚动，内容少时也不会留一大片空白
            let avail = ui.available_height().max(76.0);
            egui::ScrollArea::vertical()
                .max_height(avail)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (m, k) in &app.log {
                        let c = match k {
                            LogKind::Ok => p.ok,
                            LogKind::Warn => p.warn,
                            LogKind::Bad => p.danger,
                            LogKind::Info => p.fg_dim,
                        };
                        ui.label(
                            egui::RichText::new(m)
                                .font(egui::FontId::monospace(10.5))
                                .color(c),
                        );
                    }
                });
        });
}

// ============================================================
// 小组件
// ============================================================
/// 一行读数：左标签 + 右等宽值
fn reading(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    p: Palette,
    value_color: Option<egui::Color32>,
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [70.0, 18.0],
            egui::Label::new(egui::RichText::new(label).size(11.0).color(p.fg_mute)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .font(egui::FontId::monospace(11.0))
                    .color(value_color.unwrap_or(p.fg_dim)),
            );
        });
    });
    ui.add_space(2.0);
}

/// 细分隔线
fn hairline(ui: &mut egui::Ui, p: Palette) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(0), p.line);
}
