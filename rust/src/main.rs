//! Claude 指纹切换器 · Rust + egui
//!
//! 模块划分：
//!   core.rs    —— 时区 / 区域语言 / 注册表 / 进程探测
//!   browser.rs —— 浏览器 Preferences 读写 + 备份还原
//!   ui.rs      —— egui 界面与主题
//!   main.rs    —— CLI 入口与程序启动

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod core;
mod ui;

use crate::browser::{load_backup, save_backup, set_browser_language};
use crate::core::*;

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
            "apply" => {
                match args.get(2) {
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
                }
            }
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([440.0, 700.0])
            .with_min_inner_size([400.0, 620.0])
            // 保留系统原生标题栏：自绘标题栏在 egui 0.31 上拖拽区与按钮会互相抢占，
            // 导致关闭/最小化点不到。原生标题栏的"独立一行"用下面的工具条抵消。
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
    println!("Chrome 运行 : {}", f.chrome_running);
    println!("Edge   运行 : {}", f.edge_running);
    println!("风险分      : {}/65  {}", s, l);
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
        println!(
            "⚠ 检测到浏览器正在运行: {}",
            running.join(", ")
        );
        println!("  已跳过浏览器语言设置 —— 运行中改写会被浏览器退出时覆盖。");
        println!("  请完全退出浏览器后重新运行本命令。");
        skip_browser = true;
    }

    match save_backup() {
        Ok(Some(m)) => println!("✓ {}", m),
        Ok(None) => {}
        Err(e) => { println!("✗ 备份失败: {}", e); fails += 1; }
    }
    let before = get_timezone();
    match set_timezone(inf.tz) {
        Ok(_) => println!("✓ 时区: {} -> {}", before, inf.tz),
        Err(e) => { println!("✗ 时区切换失败: {}", e); fails += 1; }
    }
    match set_culture(inf.culture) {
        Ok(_) => println!("✓ 区域语言: -> {}", inf.culture),
        Err(e) => { println!("✗ 区域语言失败: {}", e); fails += 1; }
    }
    // 首选 UI 语言刻意不动：需要语言包已安装且必须注销才生效，风险高于收益
    println!("· 首选 UI 语言：本工具不动（避免语言包缺失导致界面异常）");

    if skip_browser {
        println!("· 浏览器语言：已跳过（浏览器在运行）");
    } else {
        let (blog, bfail) = set_browser_language(inf.browser_lang);
        for l in blog {
            println!("{}", l);
        }
        fails += bfail;
    }
    if fails > 0 {
        println!("✗ 完成，但有 {} 项失败", fails);
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
        Err(e) => { println!("✗ 时区还原失败: {}", e); fails += 1; }
    }
    match set_culture(&b.culture) {
        Ok(_) => println!("✓ 区域语言 -> {}", b.culture),
        Err(e) => { println!("✗ 区域语言还原失败: {}", e); fails += 1; }
    }
    // 备份里的 ui_langs 不再还原：本工具已改为不写首选 UI 语言。
    // 保留字段读取只为兼容旧备份，这里明确告知而不是静默忽略。
    if b.ui_langs.is_some() {
        println!("· UI 语言：本工具不再改动（如需还原请在 Windows 语言设置里手动调整）");
    }
    match &b.browser_lang {
        Some(l) => {
            let (blog, bfail) = set_browser_language(l);
            for line in blog {
                println!("{}", line);
            }
            fails += bfail;
        }
        None => println!("· 浏览器语言：备份时未检测到，保持原样"),
    }
    if fails > 0 {
        println!("✗ 还原完成，但有 {} 项失败", fails);
        return 1;
    }
    println!("★ 还原完成");
    0
}
