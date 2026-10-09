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
                if let Some(name) = args.get(2) {
                    if let Some(p) = parse_profile(name) {
                        std::process::exit(cli_apply(p));
                    }
                }
                eprintln!("未知画像: {:?}", args.get(2));
                eprintln!("可用: singapore / california / taipei / tokyo / newyork / shanghai");
                std::process::exit(2);
            }
            "restore" => {
                std::process::exit(cli_restore());
            }
            // 未知子命令必须报错退出，不能静默 fall through 到 GUI
            other => {
                eprintln!("未知子命令: {}", other);
                eprintln!("用法: claude-fingerprint [status | apply <profile> | restore]");
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
    println!("Chrome 运行 : {}", f.chrome_running);
    println!("Edge   运行 : {}", f.edge_running);
    println!("风险分      : {}/65  {}", s, l);
}

fn cli_apply(p: Profile) -> i32 {
    let inf = info(p);
    let mut fails = 0;
    println!("===== 切换到 {} ({}) =====", inf.label, inf.tz);
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
    match set_ui_languages(inf.ui_lang) {
        Ok(_) => println!("✓ 首选 UI 语言: -> {}", inf.ui_lang),
        Err(e) => { println!("✗ UI 语言失败: {}", e); fails += 1; }
    }
    let (blog, bfail) = set_browser_language(inf.browser_lang);
    for l in blog {
        println!("{}", l);
    }
    fails += bfail;
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
    match &b.ui_langs {
        Some(l) => {
            match set_ui_languages(l) {
                Ok(_) => println!("✓ UI 语言 -> {}", l),
                Err(e) => { println!("✗ UI 语言还原失败: {}", e); fails += 1; }
            }
        }
        None => println!("· UI 语言：备份时未读取到，保持原样"),
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
