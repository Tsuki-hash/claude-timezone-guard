//! Claude 指纹切换器 · Rust + egui
//!
//! 模块划分：
//!   core.rs    —— 时区 / 区域语言 / 注册表 / 进程探测
//!   browser.rs —— 浏览器 Preferences 读写 + 备份还原
//!   ui.rs      —— egui 界面与主题
//!   main.rs    —— CLI 入口与程序启动
//!
//! # 为什么**不**声明 windows_subsystem = "windows"
//!
//! 早先 release 构建声明了 GUI 子系统，这在"同一个 exe 兼做 CLI"的设计下是错的：
//! GUI 子系统进程不附着任何控制台，从 PowerShell 里跑 `claude-fingerprint status`
//! 会拿不到任何输出（重定向到文件是 0 字节），`println!` 还会因为写不出去而 panic
//! （os error 232 管道正在被关闭），退出码也随之不可靠 —— 与"便于脚本化"的
//! 设计目标直接冲突。
//!
//! 现在的做法：保持控制台子系统（CLI 完全可用），仅在真正启动图形界面时
//! 把控制台窗口隐藏掉（见 hide_console_window），视觉上与 GUI 程序无异。

mod browser;
mod core;
mod ui;

use crate::browser::{load_backup, restore_browser_langs, save_backup, set_browser_language};
use crate::core::*;

/// 隐藏控制台窗口，但**仅当这个控制台是本进程独占时**。
///
/// 为什么必须判断独占：`GetConsoleWindow()` 返回的是「本进程所附着的那个控制台」
/// 的窗口。当用户从已有的 PowerShell/cmd 窗口里运行本程序时，那是**与父 shell
/// 共享的同一个 HWND** —— 直接 SW_HIDE 会把用户的终端窗口一起隐藏掉
/// （进程还在跑，窗口没了，只能从任务栏找回来）。只有"双击 exe"这种情况，
/// 系统才为本进程新建一个独占控制台，此时隐藏它才是安全且期望的行为。
///
/// `GetConsoleProcessList` 返回附着到该控制台的进程数：只有本进程（==1）才隐藏。
fn hide_console_window_if_exclusive() {
    use ::std::ffi::c_void;

    #[link(name = "kernel32", kind = "dylib")]
    extern "system" {
        fn GetConsoleWindow() -> *mut c_void;
        fn GetConsoleProcessList(process_list: *mut u32, count: u32) -> u32;
    }
    #[link(name = "user32", kind = "dylib")]
    extern "system" {
        fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
    }
    const SW_HIDE: i32 = 0;

    unsafe {
        let mut pids = [0u32; 2];
        let n = GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32);
        // n > 1 说明还有别的进程（父 shell）共用这个控制台，不能隐藏
        if n != 1 {
            return;
        }
        let hwnd = GetConsoleWindow();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// 加载微软雅黑，保证中文正常渲染
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut loaded = false;

    for path in [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\deng.ttf",
    ] {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert("msyh".to_owned(), egui::FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "msyh".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("msyh".to_owned());
            loaded = true;
            break;
        }
    }
    if loaded {
        ctx.set_fonts(fonts);
    }
}

fn main() -> Result<(), eframe::Error> {
    // CLI 模式：便于脚本化 / 自测
    //   claude-fingerprint.exe status            -> 打印当前指纹
    //   claude-fingerprint.exe apply <profile>   -> 切换并打印结果
    //   claude-fingerprint.exe restore           -> 还原
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "status" => {
                cli_status();
                std::process::exit(0);
            }
            "apply" => match args.get(2) {
                Some(name) => match parse_profile(name) {
                    Some(p) => std::process::exit(cli_apply(p)),
                    None => {
                        eprintln!("未知画像: {}", name);
                        cli_usage();
                        std::process::exit(2);
                    }
                },
                None => {
                    eprintln!("apply 需要一个画像名");
                    cli_usage();
                    std::process::exit(2);
                }
            },
            "restore" => {
                std::process::exit(cli_restore());
            }
            "--version" | "-V" | "version" => {
                println!("claude-fingerprint {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" | "help" => {
                cli_usage();
                std::process::exit(0);
            }
            // 未知子命令必须报错退出，不能静默 fall through 到 GUI
            other => {
                eprintln!("未知子命令: {}", other);
                cli_usage();
                std::process::exit(2);
            }
        }
    }

    // 走到这里说明没有 CLI 子命令 —— 启动图形界面。
    // 隐藏控制台窗口（仅当它是本进程独占时，否则会连带隐藏用户的终端）
    hide_console_window_if_exclusive();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([440.0, 700.0])
            .with_min_inner_size([400.0, 620.0])
            // 自绘标题栏（含最小化/最大化/关闭），窗口控制与界面融为同一行。
            // 早先保留原生标题栏，是因为当时的拖拽区实现会抢按钮的点击；
            // 改用 `ui.interact()` 先建拖拽响应、按钮后画（后画者交互优先）后
            // 两不误，见 ui.rs 的 draw_titlebar。
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "Claude 指纹切换器",
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(ui::App::new()))
        }),
    )
}

fn cli_usage() {
    eprintln!("Claude 指纹切换器 {}", env!("CARGO_PKG_VERSION"));
    eprintln!();
    eprintln!("用法:");
    eprintln!("  claude-fingerprint status              查看当前指纹与风险分");
    eprintln!("  claude-fingerprint apply <画像>        切换时区 / 区域语言 / 浏览器语言");
    eprintln!("  claude-fingerprint restore             还原到切换前的状态");
    eprintln!("  claude-fingerprint                     不带参数 = 启动图形界面");
    eprintln!();
    eprintln!("可用画像:");
    eprintln!("  singapore (sg)   新加坡   UTC+8，时钟零差异（推荐）");
    eprintln!("  california (ca)  美国加州 Pacific Time");
    eprintln!("  taipei (tw)      台北     UTC+8，时钟零差异");
    eprintln!("  tokyo (jp)       东京     UTC+9");
    eprintln!("  newyork (ny)     纽约     Eastern Time");
    eprintln!("  shanghai (cn)    上海     恢复中国大陆默认");
    eprintln!();
    eprintln!("注意: 浏览器正在运行时会自动跳过浏览器语言设置——");
    eprintln!("      运行中的 Chromium 会在退出时覆盖磁盘上的 Preferences。");
}

fn parse_profile(s: &str) -> Option<Profile> {
    match s.to_lowercase().as_str() {
        "singapore" | "sg" => Some(Profile::Singapore),
        "california" | "ca" | "us" => Some(Profile::California),
        "taipei" | "tw" => Some(Profile::Taipei),
        "tokyo" | "jp" => Some(Profile::Tokyo),
        "newyork" | "ny" => Some(Profile::NewYork),
        "shanghai" | "cn" => Some(Profile::Shanghai),
        _ => None,
    }
}

fn cli_status() {
    let f = read_fingerprint();
    let (s, l) = risk_score(&f);
    println!("时区        : {}", f.tz_id);
    println!("本地时间    : {}", f.now);
    println!("区域语言    : {}", f.culture);
    println!("系统区域    : ACP={} Geo={}", f.sys_locale, f.geo_id);
    println!("浏览器语言  : {}", f.browser_lang);
    println!("界面语言    : {}  (只读，本工具不改)", f.ui_langs);

    // 原文第二条路径：本工具改不了它，所以更要如实报出来
    match &f.base_url {
        Some(u) => {
            println!(
                "中转地址    : {}{}",
                u,
                if f.proxy_like_base_url {
                    "  ← 非官方地址，原文点名的识别路径之一"
                } else {
                    "  (官方)"
                }
            );
            println!("              {}", f.base_url_hint);
            if f.proxy_like_base_url {
                println!(
                    "              修法: 把该值改为空或 https://api.anthropic.com（本工具不代改）"
                );
                println!("              提醒: 同一个文件里通常还有 ANTHROPIC_AUTH_TOKEN 等密钥，");
                println!("                    分享截图/贴日志前请先打码。");
            }
        }
        None => println!("中转地址    : 未设置（走官方直连）"),
    }
    match &f.ntp_server {
        Some(s) => println!(
            "NTP 校时    : {}{}",
            s,
            if f.ntp_leaks {
                "  ← 国内校时服务器会暴露真实时区"
            } else {
                ""
            }
        ),
        None => println!("NTP 校时    : 未配置"),
    }

    println!("Chrome 运行 : {}", f.chrome_running);
    println!("Edge   运行 : {}", f.edge_running);
    println!("风险分      : {}/{}  {}", s, RISK_MAX, l);
    println!("              （只反映本机可观测项，不含出口 IP / DNS / WebRTC）");
}

fn cli_apply(p: Profile) -> i32 {
    let inf = info(p);
    let mut fails = 0;
    println!("===== 切换到 {} ({}) =====", inf.label, inf.tz);

    // 浏览器必须先完全退出：运行中的 Chromium 会在退出时把内存里的
    // Preferences 刷回磁盘，覆盖掉我们写入的语言设置（静默回滚）。
    // 这里采取"拒绝执行"而不是"先写后提示"，避免用户以为改成功了。
    let mut skip_browser = false;
    let running = running_browsers();
    if !running.is_empty() {
        println!("⚠ 检测到浏览器正在运行: {}", running.join(", "));
        println!("  已跳过浏览器语言设置 —— 运行中改写会被浏览器退出时覆盖。");
        println!("  请完全退出浏览器后重新运行本命令。");
        skip_browser = true;
    }

    match save_backup() {
        Ok(Some(m)) => println!("✓ {}", m),
        Ok(None) => {}
        Err(e) => {
            println!("✗ 备份失败: {}", e);
            fails += 1;
        }
    }
    let before = get_timezone();
    match set_timezone(inf.tz) {
        Ok(_) => println!("✓ 时区: {} -> {}", before, inf.tz),
        Err(e) => {
            println!("✗ 时区切换失败: {}", e);
            fails += 1;
        }
    }
    match set_culture(inf.culture) {
        Ok(_) => println!("✓ 区域语言: -> {}", inf.culture),
        Err(e) => {
            println!("✗ 区域语言失败: {}", e);
            fails += 1;
        }
    }
    // 首选 UI 语言刻意不动：需要语言包已安装且必须注销才生效，风险高于收益
    println!("· 首选 UI 语言：本工具不动（避免语言包缺失导致界面异常）");

    if skip_browser {
        // 必须计入失败：浏览器语言确实没写成功。原先只打印一行、不加计数，
        // 于是 `apply sg && echo OK` 会拿到退出码 0，脚本据此误判为完全成功。
        // GUI 侧同一情形是计入 failures 的，两套入口结论必须一致。
        println!("✗ 浏览器语言：未设置（浏览器在运行，已跳过）");
        fails += 1;
    } else {
        let (blog, bfail) = set_browser_language(inf.browser_lang);
        for l in blog {
            println!("{}", l);
        }
        fails += bfail;
    }
    if fails > 0 {
        println!("✗ 完成，但有 {} 项未完成（见上方 ✗ 行）", fails);
        return 1;
    }
    println!("★ 完成");
    0
}

fn cli_restore() -> i32 {
    let b = match load_backup() {
        Ok(b) => b,
        Err(e) => {
            println!("✗ {}", e);
            return 1;
        }
    };
    let mut fails = 0;
    println!("===== 还原到 {} / {} =====", b.tz_id, b.culture);
    match set_timezone(&b.tz_id) {
        Ok(_) => println!("✓ 时区 -> {}", b.tz_id),
        Err(e) => {
            println!("✗ 时区还原失败: {}", e);
            fails += 1;
        }
    }
    match set_culture(&b.culture) {
        Ok(_) => println!("✓ 区域语言 -> {}", b.culture),
        Err(e) => {
            println!("✗ 区域语言还原失败: {}", e);
            fails += 1;
        }
    }
    // 备份里的 ui_langs 不再还原：本工具已改为不写首选 UI 语言。
    // 保留字段读取只为兼容旧备份，这里明确告知而不是静默忽略。
    if b.ui_langs.is_some() {
        println!("· UI 语言：本工具不再改动（如需还原请在 Windows 语言设置里手动调整）");
    }
    // 浏览器语言：优先用逐 Profile 精确还原（v2 备份）。
    // 只有旧备份（没有逐 Profile 数据）才退回"单值写全部"的旧行为，并明确告知。
    // 与切换路径一致：浏览器在运行时先拒绝写入，否则会被浏览器退出时覆盖。
    if !b.browser_langs.is_empty() || b.browser_lang.is_some() {
        let running = running_browsers();
        if !running.is_empty() {
            println!(
                "✗ 浏览器正在运行（{}），已跳过浏览器语言还原",
                running.join(", ")
            );
            println!("  请完全退出浏览器后重新运行 restore。");
            fails += 1;
        } else if !b.browser_langs.is_empty() {
            let (blog, bfail) = restore_browser_langs(&b.browser_langs);
            for line in blog {
                println!("{}", line);
            }
            fails += bfail;
        } else if let Some(l) = &b.browser_lang {
            println!("· 这是旧版备份（只有单个语言值），按旧行为写到所有 Profile");
            let (blog, bfail) = set_browser_language(l);
            for line in blog {
                println!("{}", line);
            }
            fails += bfail;
        }
    } else {
        println!("· 浏览器语言：备份时未检测到，保持原样");
    }
    if fails > 0 {
        println!("✗ 还原完成，但有 {} 项失败", fails);
        return 1;
    }
    println!("★ 还原完成");
    0
}
