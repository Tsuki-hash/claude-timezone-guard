//! 浏览器 Preferences 处理 + 备份还原
//!
//! 设计约束（来自代码评审，都是会静默损坏用户数据的问题）：
//!   1. JSON 必须真解析（serde_json），不能手写字符串替换 —— 手写会漏空格/转义，且
//!      值来自用户可写的 backup.json，注入会生成非法 JSON 让 Chrome 重置全部设置
//!   2. 写入必须原子（tmp + rename）+ 留 .bak，半截 JSON 会让 Chrome 判定 profile 损坏
//!   3. fs::write 的错误必须上报，不能吞掉还报成功
//!   4. browser_lang 用 Option：空串/读取失败时还原必须跳过，而不是清空用户配置

use crate::core::{get_culture, get_timezone, read_ui_languages};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

// ============================================================
// 浏览器探测
// ============================================================
fn browser_data_dirs() -> Vec<(String, PathBuf)> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    if local.is_empty() {
        return vec![];
    }
    let base = PathBuf::from(local);
    vec![
        ("Chrome".into(), base.join(r"Google\Chrome\User Data")),
        ("Edge".into(), base.join(r"Microsoft\Edge\User Data")),
        ("Brave".into(), base.join(r"BraveSoftware\Brave-Browser\User Data")),
        ("Chromium".into(), base.join(r"Chromium\User Data")),
        ("Vivaldi".into(), base.join(r"Vivaldi\User Data")),
    ]
}

/// 按目录名排序，保证日志顺序稳定可复现
fn profile_dirs(base: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(base) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name == "Default" || name.starts_with("Profile ") {
                out.push(p);
            }
        }
    }
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

/// 读浏览器当前语言。遍历所有 profile（与写入路径对称），取第一个命中的值。
pub fn read_chrome_lang() -> Option<String> {
    for (_, base) in browser_data_dirs() {
        for dir in profile_dirs(&base) {
            let pref = dir.join("Preferences");
            if let Ok(txt) = fs::read_to_string(&pref) {
                if let Some(v) = json_get_str(&txt, "accept_languages") {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// 用 serde_json 真解析，取 intl.accept_languages 或顶层 accept_languages
fn json_get_str(txt: &str, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(txt).ok()?;
    // 优先 intl.accept_languages（Chrome 实际位置），退回顶层
    v.get("intl")
        .and_then(|i| i.get(key))
        .and_then(|s| s.as_str())
        .or_else(|| v.get(key).and_then(|s| s.as_str()))
        .map(|s| s.to_string())
}

/// 原子写：tmp + rename。失败返回 Err，绝不静默。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).map_err(|e| format!("写临时文件失败 {}: {}", tmp.display(), e))?;
    // 写完后确认能被解析，避免把半截/非法 JSON 落成正式文件
    if let Ok(t) = fs::read_to_string(&tmp) {
        if serde_json::from_str::<Value>(&t).is_err() {
            let _ = fs::remove_file(&tmp);
            return Err("生成的 JSON 非法，已放弃写入".into());
        }
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("替换 {} 失败: {}", path.display(), e)
    })
}

/// 首次修改前留一份 .bak，rename 成功后删除
fn write_with_backup(path: &Path, content: &str) -> Result<(), String> {
    let bak = path.with_extension("json.bak");
    if !bak.exists() {
        if let Ok(orig) = fs::read_to_string(path) {
            let _ = fs::write(&bak, orig);
        }
    }
    atomic_write(path, content)
}

/// 校验 language tag 合法性，挡住 backup.json 被篡改时的垃圾值
fn valid_lang(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    // 形如 zh-CN,zh;q=0.9 / en-US,en;q=0.9
    let tag_ok = |t: &str| -> bool {
        let t = t.trim();
        if t.is_empty() || t.len() > 35 {
            return false;
        }
        let mut chars = t.chars();
        let first = chars.next().unwrap();
        if !first.is_ascii_alphabetic() {
            return false;
        }
        t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    s.split(',')
        .flat_map(|part| part.split(';'))
        .filter(|p| !p.trim().is_empty() && !p.contains('='))
        .all(tag_ok)
}

/// 设置浏览器语言。返回 (日志, 失败数)。失败全部显式上报。
pub fn set_browser_language(lang: &str) -> (Vec<String>, usize) {
    let mut log = Vec::new();
    let mut failures = 0usize;

    if !valid_lang(lang) {
        log.push(format!("✗ 语言值不合法，已跳过: {:?}", lang));
        return (log, 1);
    }

    for (name, base) in browser_data_dirs() {
        if !base.exists() {
            continue;
        }
        let mut ok = 0;
        for dir in profile_dirs(&base) {
            let pref = dir.join("Preferences");
            let txt = match fs::read_to_string(&pref) {
                Ok(t) => t,
                Err(_) => continue, // 该 profile 没有 Preferences，跳过即可
            };
            // 真解析 -> 改值 -> 序列化，避开所有手写字符串替换的坑
            let mut v: Value = match serde_json::from_str(&txt) {
                Ok(v) => v,
                Err(e) => {
                    log.push(format!("✗ {} {} JSON 解析失败: {}", name, dir.file_name().unwrap_or_default().to_string_lossy(), e));
                    failures += 1;
                    continue;
                }
            };
            let mut changed = false;
            if let Some(intl) = v.get_mut("intl") {
                if let Some(obj) = intl.as_object_mut() {
                    if obj.contains_key("accept_languages") {
                        obj.insert("accept_languages".into(), Value::String(lang.into()));
                        changed = true;
                    }
                }
            }
            if !changed {
                // intl 里没有该键时补一个，兼容极老版本 profile
                if let Some(obj) = v.as_object_mut() {
                    let intl = obj.entry("intl").or_insert(Value::Object(Default::default()));
                    if let Some(io) = intl.as_object_mut() {
                        io.insert("accept_languages".into(), Value::String(lang.into()));
                        changed = true;
                    }
                }
            }
            if !changed {
                continue;
            }
            let out = match serde_json::to_string(&v) {
                Ok(s) => s,
                Err(e) => {
                    log.push(format!("✗ {} 序列化失败: {}", name, e));
                    failures += 1;
                    continue;
                }
            };
            match write_with_backup(&pref, &out) {
                Ok(()) => ok += 1,
                Err(e) => {
                    log.push(format!("✗ {} {} 写入失败: {}", name, dir.file_name().unwrap_or_default().to_string_lossy(), e));
                    failures += 1;
                }
            }
        }
        if ok > 0 {
            log.push(format!("✓ {} {} 个 Profile 语言 -> {}", name, ok, lang));
        }
    }

    if log.is_empty() {
        log.push("未找到浏览器 Profile，仅改了系统区域".into());
    }
    (log, failures)
}

// ============================================================
// 备份 / 还原
// ============================================================
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backup {
    /// 格式版本，将来改结构时能识别旧文件
    #[serde(default = "default_version")]
    pub version: u32,
    pub tz_id: String,
    pub culture: String,
    /// None = 备份时读取失败，还原时不动这项设置
    pub ui_langs: Option<String>,
    /// None = 备份时未检测到浏览器，还原时不动浏览器设置
    pub browser_lang: Option<String>,
}

fn default_version() -> u32 {
    1
}

fn backup_path() -> Result<PathBuf, String> {
    // 用 SHGetKnownFolderPath 拿不到时退回 LOCALAPPDATA；都没有就报错，
    // 绝不落到当前工作目录（可能只读，或把指纹备份散落到任意 cwd）
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.trim().is_empty() {
            return Ok(PathBuf::from(local).join("ClaudeFingerprint").join("backup.json"));
        }
    }
    Err("无法定位 %LOCALAPPDATA%，备份路径不可用".into())
}

/// 校验 culture 形如 zh-CN / en-US，挡住 backup.json 被篡改
fn valid_culture(c: &str) -> bool {
    let parts: Vec<&str> = c.split('-').collect();
    parts.len() == 2
        && parts[0].len() == 2
        && parts[0].chars().all(|c| c.is_ascii_lowercase())
        && parts[1].len() >= 2
        && parts[1].chars().all(|c| c.is_ascii_alphabetic())
}

/// 已知时区白名单（与 PROFILES 一致 + 中国默认）
fn known_timezone(tz: &str) -> bool {
    const KNOWN: &[&str] = &[
        "Singapore Standard Time",
        "Pacific Standard Time",
        "Taipei Standard Time",
        "Tokyo Standard Time",
        "Eastern Standard Time",
        "China Standard Time",
    ];
    KNOWN.contains(&tz)
}

pub fn save_backup() -> Result<Option<String>, String> {
    let p = backup_path()?;
    let cur = Backup {
        version: default_version(),
        tz_id: get_timezone(),
        culture: get_culture(),
        ui_langs: read_ui_languages(),
        browser_lang: read_chrome_lang(),
    };

    if p.exists() {
        if let Ok(txt) = fs::read_to_string(&p) {
            match serde_json::from_str::<Backup>(&txt) {
                Ok(old) => {
                    // 四项全相同才认为是同一状态；任一项不同都保留旧备份，
                    // 避免用户在浏览器里手改过语言后，用污染值覆盖真正的初始备份
                    let same = old.tz_id == cur.tz_id
                        && old.culture == cur.culture
                        && old.ui_langs == cur.ui_langs
                        && old.browser_lang == cur.browser_lang;
                    if !same {
                        return Ok(Some(format!(
                            "已有切换前备份 ({}/{})，本次保留不动",
                            old.tz_id, old.culture
                        )));
                    }
                }
                Err(e) => {
                    // 备份损坏：不静默跳过，明确告知
                    return Err(format!("已有备份文件损坏 ({}), 请手动删除后重试: {}", e, p.display()));
                }
            }
        }
    }

    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&cur).map_err(|e| e.to_string())?;
    fs::write(&p, json).map_err(|e| e.to_string())?;
    Ok(Some(format!("已备份当前指纹 -> {}", p.display())))
}

pub fn load_backup() -> Result<Backup, String> {
    let p = backup_path()?;
    let txt = fs::read_to_string(&p).map_err(|_| {
        format!("没有找到备份文件: {}", p.display())
    })?;
    let b: Backup = serde_json::from_str(&txt).map_err(|e| {
        format!("备份文件损坏或版本不兼容: {} ({})", p.display(), e)
    })?;
    // 值白名单校验：文件是同用户态任意程序可写的，不能直接信
    if !known_timezone(&b.tz_id) {
        return Err(format!("备份里的时区不在已知列表: {}", b.tz_id));
    }
    if !valid_culture(&b.culture) {
        return Err(format!("备份里的区域格式不合法: {}", b.culture));
    }
    if let Some(l) = &b.browser_lang {
        if !valid_lang(l) {
            return Err(format!("备份里的浏览器语言不合法: {}", l));
        }
    }
    Ok(b)
}
