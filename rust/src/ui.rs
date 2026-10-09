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
    pub bg: egui::Color32,        // 页面底
    pub panel: egui::Color32,     // 卡片
    pub well: egui::Color32,      // 凹陷区（日志/读数）
    pub line: egui::Color32,      // 分隔线 / 边框
    pub fg: egui::Color32,        // 主文字
    pub fg_dim: egui::Color32,    // 次文字
    pub fg_mute: egui::Color32,   // 弱文字（标签）
    pub sig_sg: egui::Color32,    // 信号色 A —— 新加坡（安全/零差异）
    pub sig_us: egui::Color32,    // 信号色 B —— 加州（琥珀）
    pub warn: egui::Color32,      // 中危
    pub danger: egui::Color32,    // 高危
    pub ok: egui::Color32,        // 安全
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
#[derive(Clone, Copy, PartialEq)]
pub enum LogKind {
    Info,
    Ok,
    Warn,
    Bad,
}

pub struct App {
    pub theme: Theme,
    pub fp: Fingerprint,
    pub log: Vec<(String, LogKind)>,
    pub busy: bool,
    pub has_backup: bool,
    tx: Sender<TaskResult>,
    rx: Receiver<TaskResult>,
}

pub enum TaskResult {
    Done { lines: Vec<(String, LogKind)> },
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let fp = read_fingerprint();
        // 默认浅色（技术文档纸），深色用右上角按钮切换
        let theme = load_theme_pref().unwrap_or(Theme::Light);
        let mut app = Self {
            theme,
            log: vec![(format!("就绪 · 当前时区 {}", fp.tz_id), LogKind::Info)],
            busy: false,
            has_backup: load_backup().is_ok(),
            tx,
            rx,
            fp,
        };
        if !app.has_backup {
            app.push_log("当前没有备份，切换后「一键恢复」才会启用".into(), LogKind::Info);
        } else {
            app.push_log("检测到已有备份，一键恢复可用".into(), LogKind::Info);
        }
        app
    }

    fn push_log(&mut self, msg: String, kind: LogKind) {
        self.log.push((msg, kind));
    }

    fn poll_tasks(&mut self) {
        while let Ok(res) = self.rx.try_recv() {
            match res {
                TaskResult::Done { lines } => {
                    for (m, k) in lines {
                        self.push_log(m, k);
                    }
                    self.busy = false;
                    self.fp = read_fingerprint();
                    self.has_backup = load_backup().is_ok();
                }
            }
        }
    }

    pub fn switch_to(&mut self, p: Profile) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.push_log(format!("── 切换到 {} ──", info(p).label), LogKind::Info);
        let tx = self.tx.clone();
        let inf = info(p);
        let tz = inf.tz.to_string();
        let culture = inf.culture.to_string();
        let ui = inf.ui_lang.to_string();
        let bl = inf.browser_lang.to_string();

        thread::spawn(move || {
            let mut out: Vec<(String, LogKind)> = Vec::new();

            match save_backup() {
                Ok(Some(m)) => out.push((m, LogKind::Ok)),
                Ok(None) => {}
                Err(e) => out.push((format!("备份失败: {}", e), LogKind::Warn)),
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
                Err(e) => out.push((format!("时区切换失败: {}", e), LogKind::Bad)),
            }

            match set_culture(&culture) {
                Ok(_) => out.push((format!("区域语言  → {}", culture), LogKind::Ok)),
                Err(e) => out.push((format!("区域语言失败: {}", e), LogKind::Warn)),
            }

            match set_ui_languages(&ui) {
                Ok(_) => out.push((format!("首选 UI 语言  → {}", ui), LogKind::Ok)),
                Err(e) => out.push((format!("UI 语言失败: {}", e), LogKind::Warn)),
            }

            let (blog, bfail) = set_browser_language(&bl);
            for l in blog {
                let k = if l.starts_with('✗') { LogKind::Bad } else { LogKind::Ok };
                out.push((l, k));
            }

            if process_running("chrome.exe") || process_running("msedge.exe") {
                out.push(("浏览器正在运行，需完全退出重开才生效".into(), LogKind::Warn));
            }
            out.push(("请重启 Claude Code（常驻进程不重读时区）".into(), LogKind::Warn));

            // 有失败就不说"完成"，别让用户以为全好了
            if bfail > 0 {
                out.push((format!("结束，但有 {} 项失败，请看上方 ✗ 行", bfail), LogKind::Bad));
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
        let tx = self.tx.clone();

        thread::spawn(move || {
            let mut out: Vec<(String, LogKind)> = Vec::new();
            let mut fails = 0;

            match set_timezone(&b.tz_id) {
                Ok(_) => out.push((format!("时区  → {}", b.tz_id), LogKind::Ok)),
                Err(e) => { out.push((format!("✗ 时区还原失败: {}", e), LogKind::Bad)); fails += 1; }
            }
            match set_culture(&b.culture) {
                Ok(_) => out.push((format!("区域语言  → {}", b.culture), LogKind::Ok)),
                Err(e) => { out.push((format!("✗ 区域语言还原失败: {}", e), LogKind::Bad)); fails += 1; }
            }
            match &b.ui_langs {
                Some(l) => {
                    match set_ui_languages(l) {
                        Ok(_) => out.push((format!("UI 语言  → {}", l), LogKind::Ok)),
                        Err(e) => { out.push((format!("✗ UI 语言还原失败: {}", e), LogKind::Bad)); fails += 1; }
                    }
                }
                None => out.push(("UI 语言：备份时未读取到，保持原样".into(), LogKind::Info)),
            }
            match &b.browser_lang {
                Some(l) => {
                    let (blog, bfail) = set_browser_language(l);
                    for line in blog {
                        let k = if line.starts_with('✗') { LogKind::Bad } else { LogKind::Ok };
                        out.push((line, k));
                    }
                    fails += bfail;
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

/// 把 "China Standard Time" 缩成 "China" 之类，日志更短
fn short_tz(tz: &str) -> String {
    tz.replace(" Standard Time", "").replace(" Time", "")
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
        apply_theme(ctx, self.theme);

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
                    let btn = egui::Button::new(
                        egui::RichText::new(label).color(p.fg_dim).size(11.0),
                    )
                    .fill(p.panel)
                    .stroke(egui::Stroke::new(1.0_f32, p.line))
                    .corner_radius(6.0);
                    if ui.add_sized([46.0, 22.0], btn).on_hover_text(hover).clicked() {
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
                egui::RichText::new("时区 · 区域语言 · UI 语言 · 浏览器语言")
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
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(8.0, 8.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().circle_filled(rect.center(), 3.5, dot);
                    ui.label(egui::RichText::new(txt).size(11.0).color(dot));
                });
            });

            ui.add_space(9.0);
            hairline(ui, p);
            ui.add_space(7.0);

            // 读数行
            reading(ui, "时区", &f.tz_id, p, Some(if f.is_china_tz { p.danger } else { p.ok }));
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
                        egui::RichText::new(format!("{}/65", score))
                            .font(egui::FontId::monospace(15.0))
                            .strong()
                            .color(risk_color),
                    );
                });
            });
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

/// 目标时区相对北京的偏移小时数
/// 目标时区相对 UTC 的偏移小时数（含 DST）。
/// 用 chrono 按目标时区规则现算，不用固定值 —— 固定值在冬令时会差 1 小时，
/// 时钟对照条就成了错的。时区 ID 与 Windows 的对应关系见 tz_rule。
fn target_utc_offset(key: Profile) -> chrono::FixedOffset {
    let now = chrono::Utc::now();
    let rule = match key {
        Profile::Singapore | Profile::Taipei | Profile::Shanghai => (8, 0), // 无 DST
        Profile::Tokyo => (9, 0),                                           // 无 DST
        Profile::California | Profile::NewYork => (0, 0),                    // 由 dst 标志决定
    };
    let base = chrono::FixedOffset::east_opt(rule.0 * 3600 + rule.1 * 60).unwrap();

    if matches!(key, Profile::California | Profile::NewYork) {
        // 北美 DST: 3 月第二个周日 02:00 ~ 11 月第一个周日 02:00 (当地时间)
        let is_dst = north_america_dst(now);
        let hours = match key {
            Profile::California => if is_dst { -7 } else { -8 },
            Profile::NewYork => if is_dst { -4 } else { -5 },
            _ => unreachable!(),
        };
        return chrono::FixedOffset::east_opt(hours * 3600).unwrap();
    }
    base
}

/// 判断给定 UTC 时刻是否处于北美夏令时
fn north_america_dst(utc: chrono::DateTime<chrono::Utc>) -> bool {
    use chrono::{Datelike, TimeZone, Weekday};
    let y = utc.year();
    // 3 月第二个周日 / 11 月第一个周日，按 UTC 07:00/06:00 折算当地 02:00
    let nth_weekday = |month: u32, nth: u32| -> chrono::DateTime<chrono::Utc> {
        let mut d = chrono::NaiveDate::from_ymd_opt(y, month, 1).unwrap();
        while d.weekday() != Weekday::Sun {
            d = d.succ_opt().unwrap();
        }
        d += chrono::Duration::days(7 * (nth - 1) as i64);
        chrono::Utc.from_utc_datetime(&d.and_hms_opt(2, 0, 0).unwrap())
    };
    utc >= nth_weekday(3, 2) && utc < nth_weekday(11, 1)
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
    .stroke(egui::Stroke::new(1.0_f32, if enabled { p.line } else { p.well }))
    .corner_radius(8.0)
    .min_size(egui::vec2(ui.available_width(), 34.0));

    let resp = ui.add(btn);
    let clicked = resp.clicked();
    if !enabled {
        resp.on_hover_text("还没有备份：先切换一次，这里才能还原");
    } else {
        // 备份路径/免责说明收进 hover：需要时才看，不占界面一行
        resp.on_hover_text(
            "还原到切换前的时区 / 区域语言 / UI 语言 / 浏览器语言\n\
             备份位于 %LOCALAPPDATA%\\ClaudeFingerprint\\backup.json\n\
             本工具只改本机指纹，不代理 IP",
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
            let btn = egui::Button::new(
                egui::RichText::new(label).color(p.fg_dim).size(11.0),
            )
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
        let refresh = egui::Button::new(
            egui::RichText::new("刷新").color(p.fg_dim).size(11.0),
        )
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(6.0);
        if ui.add_sized([bw, 28.0], refresh).clicked() {
            app.fp = read_fingerprint();
            app.push_log("已刷新指纹状态".into(), LogKind::Info);
        }

        let web = egui::Button::new(
            egui::RichText::new("检测页").color(p.fg_dim).size(11.0),
        )
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(6.0);
        if ui.add_sized([bw, 28.0], web).clicked() {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "https://ip.net.coffee/claude/"])
                .spawn();
            app.push_log("已在浏览器打开检测页".into(), LogKind::Info);
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
                ui.label(
                    egui::RichText::new("处理中…")
                        .size(10.5)
                        .color(p.warn),
                );
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
fn reading(ui: &mut egui::Ui, label: &str, value: &str, p: Palette, value_color: Option<egui::Color32>) {
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
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 1.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, egui::CornerRadius::same(0), p.line);
}
