//! UI 层：egui 界面 + 主题
//! 设计方向：「网络监控仪表盘」—— 工具本质是检测/修改系统指纹，
//! 美学取自网络诊断仪器（示波器、链路监控台），而非通用设置面板。
//!
//! 签名元素：时区对照条 —— 一条横向时间轴同时显示北京/当前/目标时区，
//! 把「时钟零差异」这件抽象的事变成看得见的刻度。

use crate::browser::{
    backup_state, load_backup, restore_browser_langs, save_backup, set_browser_language,
    BackupState,
};
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

/// 设计令牌：所有颜色集中在这里，两个主题共用同一套语义。
/// 视觉语言对齐「明亮工作台」：柔和蓝灰底 + 纯白卡片 + 品牌蓝强调 + 彩色数据。
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: egui::Color32,      // 页面底
    pub panel: egui::Color32,   // 卡片
    pub well: egui::Color32,    // 凹陷区（迷你瓦片/日志）
    pub line: egui::Color32,    // 卡片描边
    pub fg: egui::Color32,      // 主文字
    pub fg_dim: egui::Color32,  // 次文字
    pub fg_mute: egui::Color32, // 弱文字（标签）
    pub accent: egui::Color32,  // 品牌蓝（CTA / hover 强调）
    pub sig_sg: egui::Color32,  // 信号色 A —— 新加坡（安全/零差异）
    pub sig_us: egui::Color32,  // 信号色 B —— 加州（琥珀）
    pub warn: egui::Color32,    // 中危
    pub danger: egui::Color32,  // 高危
    pub ok: egui::Color32,      // 安全
}

impl Palette {
    pub fn for_theme(t: Theme) -> Self {
        match t {
            // 深色：深海军夜色（参考 dark 令牌），卡片是抬升的蓝灰面
            Theme::Dark => Self {
                bg: egui::Color32::from_rgb(0x0E, 0x17, 0x24),
                panel: egui::Color32::from_rgb(0x1E, 0x2C, 0x40),
                well: egui::Color32::from_rgb(0x24, 0x35, 0x4C),
                line: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 32),
                fg: egui::Color32::from_rgb(0xF0, 0xF6, 0xFF),
                fg_dim: egui::Color32::from_rgb(0xA7, 0xB8, 0xCD),
                fg_mute: egui::Color32::from_rgb(0x7A, 0x8C, 0xA3),
                accent: egui::Color32::from_rgb(0x7B, 0xA4, 0xFF),
                sig_sg: egui::Color32::from_rgb(0x4D, 0xDC, 0x95),
                sig_us: egui::Color32::from_rgb(0xFF, 0xAB, 0x6B),
                warn: egui::Color32::from_rgb(0xFF, 0xAB, 0x6B),
                danger: egui::Color32::from_rgb(0xFF, 0x8A, 0x8A),
                ok: egui::Color32::from_rgb(0x4D, 0xDC, 0x95),
            },
            // 浅色：柔和蓝灰底 + 纯白卡片（参考 light 令牌）
            Theme::Light => Self {
                bg: egui::Color32::from_rgb(0xE9, 0xEF, 0xF6),
                panel: egui::Color32::from_rgb(0xFF, 0xFF, 0xFF),
                well: egui::Color32::from_rgb(0xED, 0xF3, 0xFA),
                line: egui::Color32::from_rgba_unmultiplied(126, 154, 184, 82),
                fg: egui::Color32::from_rgb(0x12, 0x26, 0x3C),
                fg_dim: egui::Color32::from_rgb(0x40, 0x58, 0x74),
                fg_mute: egui::Color32::from_rgb(0x6E, 0x82, 0x98),
                accent: egui::Color32::from_rgb(0x25, 0x63, 0xEB),
                sig_sg: egui::Color32::from_rgb(0x08, 0xA6, 0x74),
                sig_us: egui::Color32::from_rgb(0xF5, 0x82, 0x0A),
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

/// 页面层级：主页只放「状态 + 主要操作」，一屏放下不需要滚动；
/// 完整读数、待处理清单与日志收进详情页。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page {
    Home,
    Detail,
}

pub struct App {
    pub theme: Theme,
    /// 已经写进 egui 全局样式的主题。None = 还没应用过。
    /// 用来把 apply_theme 从"每帧"降为"仅主题变化时"。
    applied_theme: Option<Theme>,
    /// 当前页面（主页 / 详情页）
    pub page: Page,
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
}

pub enum TaskResult {
    Done { lines: Vec<(String, LogKind)> },
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
            corners_applied: false,
            log: vec![(format!("就绪 · 当前时区 {}", fp.tz_id), LogKind::Info)],
            busy: false,
            backup: backup.clone(),
            task_rx: None,
            fp,
        };
        match &backup {
            BackupState::Missing => app.push_log(
                "当前没有备份，切换后「一键恢复」才会启用".into(),
                LogKind::Info,
            ),
            BackupState::Ready => {
                app.push_log("检测到已有备份，一键恢复可用".into(), LogKind::Info)
            }
            BackupState::Broken(e) => {
                app.push_log(format!("⚠ 备份文件无法使用: {}", e), LogKind::Bad)
            }
        }
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
        self.fp = read_fingerprint();
        self.refresh_backup_state();
        self.push_log("已刷新指纹与备份状态".into(), LogKind::Info);
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
                self.refresh_backup_state();
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

        // 页面路由：主页一屏放下（零滚动），详情页内部才滚动。
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ctx, |ui| match self.page {
                Page::Home => draw_home(ui, self, p),
                Page::Detail => draw_detail(ui, self, p),
            });

        // F5 手动刷新：挂了代理、改了环境变量或在外面跑了 CLI 之后，
        // 不用滚动找按钮，直接重读本机状态
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) && !self.busy {
            self.refresh_fingerprint();
        }

        if self.busy {
            ctx.request_repaint();
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
        .frame(
            egui::Frame::NONE
                .fill(p.bg)
                .inner_margin(egui::Margin::symmetric(14, 7)),
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

            ui.horizontal(|ui| {
                ui.set_height(26.0);

                // 左：应用名，低声处理（12px、次级灰、不加图标不加粗）——
                // 「静仪」哲学：标题栏不许抢数据的戏，绿色方块装饰已移除
                ui.label(
                    egui::RichText::new("Claude 指纹切换器")
                        .size(11.5)
                        .color(p.fg_dim),
                );
                ui.label(egui::RichText::new("v1.0").size(9.5).color(p.fg_mute));

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
                    // 刷新入口常驻标题栏（参考项目的 header 按钮）
                    let rbtn =
                        egui::Button::new(egui::RichText::new("刷新").color(p.fg_dim).size(10.5))
                            .fill(blend(p.fg, p.bg, 0.93))
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(5.0);
                    if ui
                        .add_sized([46.0, 22.0], rbtn)
                        .on_hover_text("重新读取本机指纹与备份状态（快捷键 F5）")
                        .clicked()
                        && !app.busy
                    {
                        app.refresh_fingerprint();
                    }
                    ui.add_space(4.0);
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
    let r = resp.rect.shrink2(egui::vec2(11.0, 7.0));
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

// ============================================================
// 主页（一级页面）
// ============================================================
/// 窗口默认/最小尺寸（逻辑点）。主页内容高度由布局回归测试实测校准：
/// `主页布局右边缘不溢出` 会按这套尺寸断言内容四边都在窗口内。
pub const WINDOW_SIZE: [f32; 2] = [440.0, 632.0];
pub const MIN_WINDOW_SIZE: [f32; 2] = [420.0, 628.0];
/// 主页 —— 「明亮工作台」卡片仪表盘（视觉语言对齐参考项目）：
/// 纯白卡片浮在蓝灰底上，大号彩色数字 + 状态胶囊 + 迷你数据瓦片。
fn draw_home(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    draw_hero(ui, app, p);
    ui.add_space(8.0);
    draw_readings(ui, app, p);
    ui.add_space(8.0);
    draw_exits(ui, app, p);
    ui.add_space(8.0);
    draw_latest_log(ui, app, p);
}

/// 小节标题：小号、次级灰、加粗——只做路标，不抢内容的戏
fn section_label(ui: &mut egui::Ui, p: Palette, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .strong()
            .size(10.0)
            .color(p.fg_mute),
    );
    ui.add_space(4.0);
}

/// 状态胶囊：风险色 15% 底 + 同色墨字 + 圆点（参考 status-pill）
fn status_pill(ui: &mut egui::Ui, p: Palette, text: &str, ink: egui::Color32) {
    let btn = egui::Button::new(
        egui::RichText::new(format!("● {}", text))
            .size(10.0)
            .color(ink),
    )
    .fill(blend(ink, p.panel, 0.85))
    .stroke(egui::Stroke::NONE)
    .corner_radius(9.0);
    ui.add(btn);
}

/// hero 风险卡：小标题 + 状态胶囊 + 大号彩色数字 + 两块迷你数据瓦片。
/// 大数字是全界面唯一的响亮元素，语言对齐参考项目的「账户余额」卡。
fn draw_hero(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let f = &app.fp;
    let (score, level) = risk_score(f);
    let color = p.risk(score);
    let tripped = f.risk_items().iter().filter(|i| i.score >= 0.25).count();

    egui::Frame::NONE
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            // 标题行：小标 + 右侧状态胶囊
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("指纹风险").size(10.0).color(p.fg_mute));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if tripped > 0 {
                        status_pill(ui, p, &format!("{} 项未规避", tripped), p.danger);
                    } else {
                        status_pill(ui, p, "已规避", p.ok);
                    }
                });
            });
            ui.add_space(3.0);

            // 大数字行
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(score.to_string())
                        .font(egui::FontId::monospace(26.0))
                        .strong()
                        .color(color),
                );
                ui.label(
                    egui::RichText::new(format!("/{}", RISK_MAX))
                        .font(egui::FontId::monospace(10.5))
                        .color(p.fg_mute),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new(level).size(11.5).strong().color(color));
            });
            ui.add_space(7.0);

            // 迷你瓦片 ×2：两处最需要盯着的特征
            let tz_color = match f.tz_tier() {
                TzTier::Full => p.danger,
                TzTier::Partial => p.warn,
                TzTier::None => p.ok,
            };
            let bu = match &f.base_url {
                Some(u) => truncate(
                    u.trim_start_matches("https://")
                        .trim_start_matches("http://"),
                    24,
                ),
                None => "未设置".into(),
            };
            let tw = (ui.available_width() - 6.0) / 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                mini_tile(ui, p, tw, "当前时区", &truncate(&f.tz_id, 22), tz_color);
                mini_tile(
                    ui,
                    p,
                    tw,
                    "中转地址",
                    &bu,
                    if f.proxy_like_base_url {
                        p.danger
                    } else {
                        p.ok
                    },
                );
            });
        });
}

/// 迷你数据瓦片：凹陷底 + 小标签 + 彩色等宽值。
/// 用 `add_sized` 锁定精确尺寸：`set_min_width` 只是下限，长文本的自然宽度
/// 会撑破瓦片（实测把右边缘推出窗口 22.5px）；`painter_at` 自带矩形裁剪，
/// 超长值在瓦片边缘截断，绝不外溢。
fn mini_tile(
    ui: &mut egui::Ui,
    p: Palette,
    w: f32,
    caption: &str,
    value: &str,
    color: egui::Color32,
) {
    let resp = ui.add_sized(
        [w, 42.0],
        egui::Button::new("")
            .fill(p.well)
            .stroke(egui::Stroke::NONE),
    );
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    let cx = rect.left() + 10.0;
    painter.text(
        egui::pos2(cx, rect.top() + 11.0),
        egui::Align2::LEFT_CENTER,
        caption,
        egui::FontId::proportional(9.0),
        p.fg_mute,
    );
    painter.text(
        egui::pos2(cx, rect.top() + 28.0),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::monospace(10.0),
        color,
    );
}

/// 本机指纹读数卡：白卡内标签右对齐成列，值共享左边缘；右上角进入详情页。
fn draw_readings(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let f = &app.fp;
    egui::Frame::NONE
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("本机指纹").size(10.0).color(p.fg_mute));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("详情 ›").size(10.0).color(p.accent),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    {
                        app.page = Page::Detail;
                    }
                });
            });
            ui.add_space(3.0);

            let bu = match &f.base_url {
                Some(u) => u.clone(),
                None => "未设置".into(),
            };
            let ntp = f.ntp_server.clone().unwrap_or_else(|| "未配置".into());
            let (tz_txt, tz_color) = match f.tz_tier() {
                TzTier::Full => (f.tz_id.clone(), p.danger),
                TzTier::Partial => (format!("{}（港澳·受限地区）", f.tz_id), p.warn),
                TzTier::None => (f.tz_id.clone(), p.ok),
            };
            let (font_txt, font_color) = if !f.fonts_vendor.is_empty() {
                (truncate(&f.fonts_vendor.join("、"), 30), p.danger)
            } else if !f.fonts_extra.is_empty() {
                (truncate(&f.fonts_extra.join("、"), 30), p.warn)
            } else {
                ("未命中".into(), p.ok)
            };
            let (browser_txt, browser_color) = if f.cn_browsers.is_empty() {
                ("未安装".into(), p.ok)
            } else {
                (truncate(&f.cn_browsers.join("、"), 30), p.danger)
            };

            fingerprint_row(ui, p, "时区", &tz_txt, Some(tz_color));
            fingerprint_row(
                ui,
                p,
                "中转地址",
                &bu,
                Some(if f.proxy_like_base_url {
                    p.danger
                } else {
                    p.ok
                }),
            );
            fingerprint_row(
                ui,
                p,
                "区域语言",
                &f.culture,
                Some(if f.culture == "zh-CN" { p.danger } else { p.ok }),
            );
            fingerprint_row(
                ui,
                p,
                "NTP 校时",
                &ntp,
                Some(if f.ntp_leaks { p.warn } else { p.ok }),
            );
            fingerprint_row(ui, p, "字体环境", &font_txt, Some(font_color));
            fingerprint_row(ui, p, "国产浏览器", &browser_txt, Some(browser_color));
            // 只读展示（本工具不改）：中性墨色，不带状态点
            fingerprint_row(ui, p, "浏览器语言", &f.browser_lang, None);
            fingerprint_row(ui, p, "界面语言", &f.ui_langs, None);
        });
}

/// 一行读数：标签右对齐入 72px 列，值等宽、共享左边缘，全宽不截断。
/// 颜色纪律：只有「有发现」的值才着风险色，其余一律正墨。
fn fingerprint_row(
    ui: &mut egui::Ui,
    p: Palette,
    label: &str,
    value: &str,
    status: Option<egui::Color32>,
) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(72.0, 17.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).size(9.5).color(p.fg_mute));
            },
        );
        ui.spacing_mut().item_spacing.x = 12.0;
        let c = status.unwrap_or(p.fg);
        ui.label(
            egui::RichText::new(value)
                .font(egui::FontId::monospace(10.5))
                .color(c),
        );
    });
    ui.add_space(2.0);
}

/// 切换出口：两张主出口卡（时钟对照是唯一的彩色瞬间）+ 次要出口 + 一键恢复。
fn draw_exits(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    section_label(ui, p, "切换出口");
    draw_exit_selector(ui, app, p);
    ui.add_space(5.0);

    // 次要出口与检测页：无边框文字钮，与主卡拉开层级
    let others = [
        (Profile::Taipei, "台北"),
        (Profile::Tokyo, "东京"),
        (Profile::NewYork, "纽约"),
        (Profile::Shanghai, "上海"),
    ];
    let n = 5.0;
    let bw = (ui.available_width() - (n - 1.0) * 5.0) / n;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (key, label) in others {
            let btn = egui::Button::new(egui::RichText::new(label).color(p.fg_dim).size(10.5))
                .fill(p.panel)
                .stroke(egui::Stroke::new(1.0_f32, p.line))
                .corner_radius(6.0);
            if ui
                .add_sized([bw, 26.0], btn)
                .on_hover_text(info(key).sub)
                .clicked()
                && !app.busy
            {
                app.switch_to(key);
            }
        }
        let web = egui::Button::new(egui::RichText::new("检测页").color(p.fg_dim).size(10.5))
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.line))
            .corner_radius(6.0);
        if ui.add_sized([bw, 24.0], web).clicked() {
            // 用 webbrowser crate（走 ShellExecuteW）而不是 `cmd /C start`：
            //   - `Command::new("cmd")` 是裸名，会按 CreateProcess 的搜索顺序
            //     （应用目录 → 当前目录 → System32 → …）找 cmd.exe，同目录或
            //     当前目录里的假 cmd.exe 能劫持它 —— 与本项目 sys_tool() 的
            //     绝对路径加固理念自相矛盾。
            //   - `cmd /C start <串>` 会重新解析该字符串，将来 URL 一旦变成
            //     可配置/可拼接，`&`、`^`、`"` 立刻变成命令注入面。
            match webbrowser::open(DETECT_PAGE_URL) {
                Ok(()) => app.push_log("已在浏览器打开检测页".into(), LogKind::Info),
                Err(e) => app.push_log(
                    format!("打开检测页失败: {} —— 请手动访问 {}", e, DETECT_PAGE_URL),
                    LogKind::Warn,
                ),
            }
        }
    });

    ui.add_space(5.0);
    draw_restore(ui, app, p);
}

/// 按字符数截断（汉字按字符计，不截半字节）
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

/// 前景/底色实色混合（bg_ratio = 底色占比），egui 透明度不稳时用它
fn blend(fg: egui::Color32, bg: egui::Color32, bg_ratio: f32) -> egui::Color32 {
    let t = 1.0 - bg_ratio;
    let mix = |a: u8, b: u8| (a as f32 * t + b as f32 * bg_ratio).round() as u8;
    egui::Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

/// 最近一条日志 —— 主页底部单行；多行日志只取首行截断，完整日志在详情页。
fn draw_latest_log(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let Some((msg, kind)) = app.log.last() else {
        return;
    };
    let c = match kind {
        LogKind::Ok => p.ok,
        LogKind::Warn => p.warn,
        LogKind::Bad => p.danger,
        LogKind::Info => p.fg_dim,
    };
    let first_line = msg.lines().next().unwrap_or_default();
    let text = truncate(first_line, 72);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("›")
                .font(egui::FontId::monospace(10.5))
                .color(p.fg_mute),
        );
        ui.label(
            egui::RichText::new(text)
                .font(egui::FontId::monospace(10.5))
                .color(c),
        );
    });
}

// ============================================================
// 详情页（二级页面）
// ============================================================
/// 详情页 —— 完整指纹读数、待处理清单与操作日志，内部可滚动。
fn draw_detail(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    ui.horizontal(|ui| {
        let back = egui::Button::new(egui::RichText::new("← 返回").size(11.5).color(p.fg_dim))
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.line))
            .corner_radius(6.0);
        if ui.add_sized([64.0, 24.0], back).clicked() {
            app.page = Page::Home;
        }
        ui.label(
            egui::RichText::new("指纹详情")
                .strong()
                .size(14.0)
                .color(p.fg),
        );
    });
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_status_panel(ui, app, p);
            ui.add_space(11.0);
            draw_advice(ui, app, p);
            ui.add_space(11.0);
            draw_log(ui, app, p);
        });
}

/// 当前状态卡 —— 仪器读数风格：等宽数字 + 细分隔线
fn draw_status_panel(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    // 克隆而非借用：卡头新增的「刷新」按钮需要 &mut app，而 f 贯穿整个卡片。
    // 每帧最多一次、十来个短字符串，开销可忽略。
    let f = app.fp.clone();
    let (score, level) = risk_score(&f);
    let risk_color = p.risk(score);

    egui::Frame::NONE
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(15, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            // 卡头：标题（风险结论与刷新入口在主页 hero，详情页只负责完整读数）
            ui.label(
                egui::RichText::new("当前指纹")
                    .strong()
                    .size(12.5)
                    .color(p.fg),
            );

            ui.add_space(9.0);
            hairline(ui, p);
            ui.add_space(7.0);

            // 读数行
            reading(
                ui,
                "时区",
                &f.tz_id,
                p,
                Some(match f.tz_tier() {
                    TzTier::Full => p.danger,
                    TzTier::Partial => p.warn,
                    TzTier::None => p.ok,
                }),
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
            // 环境残留：本工具改不了，但必须让用户知情
            let font_txt = if !f.fonts_vendor.is_empty() {
                f.fonts_vendor.join("、")
            } else if !f.fonts_extra.is_empty() {
                f.fonts_extra.join("、")
            } else {
                "未命中".into()
            };
            reading(
                ui,
                "字体环境",
                &font_txt,
                p,
                Some(if !f.fonts_vendor.is_empty() {
                    p.danger
                } else if !f.fonts_extra.is_empty() {
                    p.warn
                } else {
                    p.ok
                }),
            );
            let browser_txt = if f.cn_browsers.is_empty() {
                "未安装".into()
            } else {
                f.cn_browsers.join("、")
            };
            reading(
                ui,
                "国产浏览器",
                &browser_txt,
                p,
                Some(if f.cn_browsers.is_empty() {
                    p.ok
                } else {
                    p.danger
                }),
            );

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
        [w, 62.0],
        egui::Button::new("")
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.line)),
    );
    // hover 时点亮品牌蓝边框，并给出副标说明（卡内只留名称与时钟）
    let clicked = resp.clicked();
    let rect = resp.rect;
    if resp.hovered() {
        ui.painter_at(rect).rect_stroke(
            rect,
            egui::CornerRadius::same(10),
            egui::Stroke::new(1.4_f32, p.accent),
            egui::StrokeKind::Inside,
        );
        resp.clone()
            .on_hover_text(format!("{} · 点击切换", inf.sub));
    }

    let painter = ui.painter_at(rect);
    let x0 = rect.left();
    let top = rect.top();

    let cx = x0 + 14.0;

    painter.text(
        egui::pos2(cx, top + 17.0),
        egui::Align2::LEFT_CENTER,
        inf.label,
        egui::FontId::proportional(13.5),
        p.fg,
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
        egui::pos2(cx, top + 38.0),
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

    if clicked && !app.busy {
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
                redact_secret("sk-EXAMPLE-0000000000000000000000")
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
    match f.tz_tier() {
        TzTier::Full => rows.push((
            p.danger,
            "系统时区".into(),
            "点下面的「新加坡」或「台北」即可（UTC+8，时钟零差异）".into(),
        )),
        TzTier::Partial => rows.push((
            p.warn,
            "系统时区".into(),
            "港澳属受限地区（部分风险）——切换到「新加坡」可完全规避".into(),
        )),
        TzTier::None => {}
    }
    if f.culture == "zh-CN" {
        rows.push((p.warn, "区域格式".into(), "点任一境外画像时一并修改".into()));
    }
    if !f.fonts_vendor.is_empty() {
        rows.push((
            p.warn,
            "字体环境".into(),
            format!(
                "已装国产厂商字体（{}，不可修）：来自国产设备同步或 WPS，需要时可在「设置 → 个性化 → 字体」卸载",
                f.fonts_vendor.join("、")
            ),
        ));
    } else if f.fonts_extra.len() >= 2 {
        rows.push((
            p.warn,
            "字体环境".into(),
            format!(
                "装有多个非 Windows 标配的中文字体（{}）：通常来自设计软件，弱信号，可不处理",
                f.fonts_extra.join("、")
            ),
        ));
    }
    if !f.cn_browsers.is_empty() {
        rows.push((
            p.warn,
            "国产浏览器".into(),
            format!(
                "已安装 {}——卸载或避免日常使用（卸载请自行在系统里确认，本工具不代劳）",
                f.cn_browsers.join("、")
            ),
        ));
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
    let enabled = app.backup_ready() && !app.busy;
    // 实心品牌蓝 CTA：全界面唯一的高对比按钮，恢复操作值得这个分量
    let (fill, txt_color) = if enabled {
        (p.accent, p.bg)
    } else {
        (p.well, p.fg_mute)
    };
    let btn = egui::Button::new(
        egui::RichText::new("↩  一键恢复 · 还原到切换前")
            .size(12.0)
            .strong()
            .color(txt_color),
    )
    .fill(fill)
    .stroke(egui::Stroke::NONE)
    .corner_radius(8.0)
    .min_size(egui::vec2(ui.available_width(), 30.0));

    let resp = ui.add(btn);
    let clicked = resp.clicked();
    if !enabled {
        // 三态提示：别再对"文件损坏"说"还没有备份：先切换一次"，那会把人带偏
        let tip = match &app.backup {
            BackupState::Missing => "还没有备份：先切换一次，这里才能还原".to_string(),
            BackupState::Broken(e) => format!(
                "备份文件无法使用：{}\n\
                 请删除 %LOCALAPPDATA%\\ClaudeFingerprint\\backup.json 后重新切换一次；\n\
                 若同目录下有 backup.json.<pid>.corrupt，那是留档的旧文件，可手工查看。",
                e
            ),
            BackupState::Ready => "正在处理中…".to_string(),
        };
        resp.on_hover_text(tip);
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
            // 固定视口高度 + 内部滚动。不能用 available_height：整页已在
            // ScrollArea 里，那里的剩余高度是无界的，会把日志框撑到无限高。
            egui::ScrollArea::vertical()
                .max_height(180.0)
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

// ============================================================
// 布局回归测试（无头 egui，不需要窗口/OpenGL）
// ============================================================
#[cfg(test)]
mod layout_tests {
    use super::*;

    /// 主页内容右边缘不得超出窗口内边界（CentralPanel 右侧 14px 边距内）。
    /// 用无头 egui 真实走一遍布局，量出实际右边缘。
    #[test]
    fn 主页布局右边缘不溢出() {
        let (win_w, win_h) = (WINDOW_SIZE[0], WINDOW_SIZE[1]);
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(win_w, win_h),
            )),
            ..Default::default()
        };
        let mut app = App::new();
        let inner_right = win_w - 14.0;
        let inner_bottom = win_h - 12.0;
        let mut edges: Vec<(&str, f32, f32)> = Vec::new();

        let _ = ctx.run(raw, |ctx| {
            let p = Palette::for_theme(app.theme);
            draw_titlebar(ctx, &mut app, p);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::central_panel(&ctx.style())
                        .fill(p.bg)
                        .inner_margin(egui::Margin::symmetric(14, 12)),
                )
                .show(ctx, |ui| match app.page {
                    Page::Home => {
                        draw_hero(ui, &mut app, p);
                        edges.push(("hero 卡", ui.min_rect().right(), ui.min_rect().bottom()));
                        draw_readings(ui, &mut app, p);
                        edges.push(("readings 卡", ui.min_rect().right(), ui.min_rect().bottom()));
                        draw_exits(ui, &mut app, p);
                        edges.push(("exits 区", ui.min_rect().right(), ui.min_rect().bottom()));
                        draw_latest_log(ui, &mut app, p);
                        edges.push(("日志行", ui.min_rect().right(), ui.min_rect().bottom()));
                    }
                    Page::Detail => {}
                });
        });

        // 每个分区的右/下边缘都必须落在窗口内边界以内（0.5px 容差）
        for (name, right, bottom) in &edges {
            assert!(
                *right <= inner_right + 0.5,
                "{name} 右边缘 {right:.1} 超出内边界 {inner_right:.1}——布局溢出会把内容截断在窗口外"
            );
            assert!(
                *bottom <= inner_bottom + 0.5,
                "{name} 底边缘 {bottom:.1} 超出内边界 {inner_bottom:.1}——窗口高度不足以容纳主页内容（WINDOW_SIZE 需调大或布局需收紧）"
            );
        }
        println!(
            "[layout] 内容底边: {:.1} / 内边界 {inner_bottom:.1}",
            edges[3].2
        );
    }
}
