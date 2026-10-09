//! 浏览器 Preferences 处理 + 备份还原
//!
//! 设计约束（来自代码评审，都是会静默损坏用户数据的问题）：
//!   1. JSON 必须真解析（serde_json），不能手写字符串替换 —— 手写会漏空格/转义，且
//!      值来自用户可写的 backup.json，注入会生成非法 JSON 让 Chrome 重置全部设置
//!   2. 写入必须原子（tmp + rename）+ 留 .bak，半截 JSON 会让 Chrome 判定 profile 损坏
//!   3. fs::write 的错误必须上报，不能吞掉还报成功
//!   4. browser_lang 用 Option：空串/读取失败时还原必须跳过，而不是清空用户配置

use crate::core::{get_culture, get_timezone};
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
        (
            "Brave".into(),
            base.join(r"BraveSoftware\Brave-Browser\User Data"),
        ),
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

/// 临时/备份文件名：在原名后追加后缀，并带上进程号。
///
/// 两个刻意的选择：
///   1. 用 `file_name` + 后缀，而不是 `with_extension` —— 后者是**替换**扩展名，
///      将来对带扩展名的文件（如 `Preferences.json`）复用时会和别的文件撞名。
///   2. 带 `process::id()` —— 名字固定的话，GUI 与 CLI（或两个实例）并发时会写
///      同一个临时文件：A 写完被 B 覆盖、A 再 rename，就把 B 的内容落了盘。
///      这正是原子写要防的东西，却因为临时名不隔离而失效。
fn sibling_temp(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "prefs".into());
    let tmp_name = format!("{}.{}.{}", name, std::process::id(), suffix);
    match path.parent() {
        Some(dir) => dir.join(tmp_name),
        None => PathBuf::from(tmp_name),
    }
}

/// 原子写：tmp + rename。失败返回 Err，绝不静默。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = sibling_temp(path, "tmp");
    fs::write(&tmp, content).map_err(|e| format!("写临时文件失败 {}: {}", tmp.display(), e))?;
    // 写完后确认能读回并解析，避免把半截/非法 JSON 落成正式文件。
    // 读回失败也要报错：读不回来通常意味着内容没完整落盘，
    // 恰恰是最需要拦住的情况，不能静默跳过检查直接 rename。
    match fs::read_to_string(&tmp) {
        Ok(t) => {
            if serde_json::from_str::<Value>(&t).is_err() {
                let _ = fs::remove_file(&tmp);
                return Err("生成的 JSON 非法，已放弃写入".into());
            }
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "临时文件写后无法读回 ({}), 已放弃写入 {}: {}",
                tmp.display(),
                path.display(),
                e
            ));
        }
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("替换 {} 失败: {}", path.display(), e)
    })
}

/// 首次修改前留一份 .bak。
///
/// 说明：这个 .bak **不会**被本工具读回来（还原走 `backup.json`），
/// 它只是留在 profile 目录里的原始副本，供用户手工核对/恢复时使用，
/// 因此刻意保留、不删除。Chrome 会忽略这类未知文件。
fn write_with_backup(path: &Path, content: &str) -> Result<(), String> {
    let bak = sibling_temp(path, "bak");
    if !bak.exists() {
        if let Ok(orig) = fs::read_to_string(path) {
            let _ = fs::write(&bak, orig);
        }
    }
    atomic_write(path, content)
}

/// 校验 language tag 列表合法性，挡住 backup.json 被篡改时的垃圾值。
/// 形如 `zh-CN,zh;q=0.9,en;q=0.8`：逗号分隔条目，条目内 `;q=` 是权重。
fn valid_lang(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }

    /// 单个 language tag：字母开头，只含字母/数字/`-`/`_`
    fn tag_ok(t: &str) -> bool {
        if t.is_empty() || t.len() > 35 {
            return false;
        }
        if !t.chars().next().unwrap().is_ascii_alphabetic() {
            return false;
        }
        t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    /// `q=0.9` 这类权重参数
    fn param_ok(p: &str) -> bool {
        match p.split_once('=') {
            Some((k, v)) => {
                k.trim().eq_ignore_ascii_case("q")
                    && !v.trim().is_empty()
                    && v.trim().parse::<f32>().is_ok()
            }
            None => false,
        }
    }

    // 过滤掉空条目（",en-US," 这种多余逗号无害），但要求至少剩一个真条目：
    // 早先直接 all() 会让 ";q=0.9" 这种"完全没有语言标签"的串通过校验。
    let items: Vec<&str> = s
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if items.is_empty() {
        return false;
    }

    items.iter().all(|item| match item.split_once(';') {
        // 没有权重参数
        None => tag_ok(item),
        // 有 `;...`：前面必须是合法标签，后面每一段都必须是合法权重
        Some((tag, params)) => {
            tag_ok(tag.trim())
                && params
                    .split(';')
                    .filter(|p| !p.trim().is_empty())
                    .all(|p| param_ok(p.trim()))
        }
    })
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
            let label = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let txt = match fs::read_to_string(&pref) {
                Ok(t) => t,
                // 区分"本来就没有 Preferences"与"有但读不出来（被占用/无权限）"。
                // 早先一律 continue，会在唯一 profile 读失败时报出
                // "未找到浏览器 Profile，仅改了系统区域" —— 把权限错误说成没有 profile，
                // 用户按错误方向排查，而且退出码还是 0。
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    log.push(format!(
                        "✗ {} {} 的 Preferences 读取失败: {}（文件被占用或无权限？）",
                        name, label, e
                    ));
                    failures += 1;
                    continue;
                }
            };
            // 真解析 -> 改值 -> 序列化，避开所有手写字符串替换的坑
            let mut v: Value = match serde_json::from_str(&txt) {
                Ok(v) => v,
                Err(e) => {
                    log.push(format!("✗ {} {} JSON 解析失败: {}", name, label, e));
                    failures += 1;
                    continue;
                }
            };
            let mut changed = false;
            // 记录 intl 存在但不是对象的情况：早先会静默落进下面的补键分支、
            // 补不进去就 continue，用户只看到别的 profile 的 ✓，这个 profile 被无声跳过。
            let mut intl_malformed = false;
            if let Some(intl) = v.get_mut("intl") {
                if let Some(obj) = intl.as_object_mut() {
                    if obj.contains_key("accept_languages") {
                        obj.insert("accept_languages".into(), Value::String(lang.into()));
                        changed = true;
                    }
                } else {
                    intl_malformed = true;
                }
            }
            if !changed && !intl_malformed {
                // intl 里没有该键时补一个，兼容极老版本 profile
                if let Some(obj) = v.as_object_mut() {
                    let intl = obj
                        .entry("intl")
                        .or_insert(Value::Object(Default::default()));
                    if let Some(io) = intl.as_object_mut() {
                        io.insert("accept_languages".into(), Value::String(lang.into()));
                        changed = true;
                    }
                }
            }
            if !changed {
                if intl_malformed {
                    log.push(format!(
                        "✗ {} {} 的 intl 结构异常（不是对象），已跳过",
                        name, label
                    ));
                    failures += 1;
                }
                // 其它情况：顶层不是对象，属于不可识别的 Preferences，静默跳过可接受
                continue;
            }
            let out = match serde_json::to_string(&v) {
                Ok(s) => s,
                Err(e) => {
                    log.push(format!("✗ {} {} 序列化失败: {}", name, label, e));
                    failures += 1;
                    continue;
                }
            };
            match write_with_backup(&pref, &out) {
                Ok(()) => ok += 1,
                Err(e) => {
                    log.push(format!("✗ {} {} 写入失败: {}", name, label, e));
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
    /// 已废弃：本工具改为**只读不写**首选 UI 语言（`PreferredUILanguages`），
    /// 因为它要求目标语言包已安装、且必须注销才生效，写坏的代价远大于收益。
    /// 字段保留只为能继续反序列化旧备份文件；新写入的备份里恒为 None。
    /// 见 core.rs 的 ProfileInfo 注释与 README「它到底改了什么」。
    #[serde(default)]
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
            return Ok(PathBuf::from(local)
                .join("ClaudeFingerprint")
                .join("backup.json"));
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
        // 恒为 None：本工具不写首选 UI 语言，没有需要备份的东西
        ui_langs: None,
        browser_lang: read_chrome_lang(),
    };

    // 写入前先用**与 load_backup 相同的校验**过一遍。
    //
    // 为什么必须自校验：load_backup 要求 tz_id 在白名单内、culture 合法、
    // browser_lang 合法。而这里取的原始值可能不合法 —— 例如读时区失败时
    // get_timezone() 返回哨兵 "未知"，或 intl.accept_languages 是空串。
    // 那样写出的备份**永远无法被还原**，而界面还会显示"已备份"，
    // 用户改完系统时区却拿不到承诺的一键回滚。宁可现在拒绝，也不要写坏的还原点。
    if !known_timezone(&cur.tz_id) {
        return Err(format!(
            "读到的当前时区 {:?} 不在已知列表，无法建立可信备份（不写还原点）。请检查系统时区是否正常。",
            cur.tz_id
        ));
    }
    if !valid_culture(&cur.culture) {
        return Err(format!(
            "读到的当前区域格式 {:?} 不合法，无法建立可信备份（不写还原点）。",
            cur.culture
        ));
    }
    if let Some(l) = &cur.browser_lang {
        if !valid_lang(l) {
            return Err(format!(
                "读到的浏览器语言 {:?} 不合法，无法建立可信备份（不写还原点）。",
                l
            ));
        }
    }

    if p.exists() {
        // 读不出来就不能往下走：下面会覆盖这个文件。备份是用户唯一的
        // 还原点，读失败（被占用/权限）时静默覆盖等于把还原点丢了，
        // 所以这里必须报错退出，而不是当作"没有备份"继续写。
        let txt = fs::read_to_string(&p).map_err(|e| {
            format!(
                "已有备份文件但读取失败 ({}): {} —— 为避免覆盖这个唯一的还原点，已中止。请关闭占用它的程序后重试。",
                p.display(),
                e
            )
        })?;
        match serde_json::from_str::<Backup>(&txt) {
            Ok(old) => {
                // 三项全相同才认为是同一状态；任一项不同都保留旧备份，
                // 避免用户在浏览器里手改过语言后，用污染值覆盖真正的初始备份。
                // ui_langs 不参与比较：它已废弃，旧备份里可能是 Some 而新写入恒为 None，
                // 拿它比较会让"同一状态"永远判假，导致每次都多写一次备份。
                let same = old.tz_id == cur.tz_id
                    && old.culture == cur.culture
                    && old.browser_lang == cur.browser_lang;
                if !same {
                    return Ok(Some(format!(
                        "已有切换前备份 ({}/{})，本次保留不动",
                        old.tz_id, old.culture
                    )));
                }
            }
            Err(e) => {
                // 损坏的备份如果一直占着位置，工具就**再也不会产生还原点**
                // （每次都被这条错误挡住），用户只能手工删文件。
                // 所以改名留档后继续，既保住人工恢复的可能，又不阻塞新备份。
                let corrupt = sibling_temp(&p, "corrupt");
                match fs::rename(&p, &corrupt) {
                    Ok(()) => {
                        // 交给下面的写入流程建立新备份，这里不 return
                    }
                    Err(re) => {
                        return Err(format!(
                            "已有备份文件损坏 ({}), 且改名留档失败 ({}): {} —— 请手动删除 {} 后重试",
                            e,
                            re,
                            p.display(),
                            p.display()
                        ));
                    }
                }
            }
        }
    }

    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&cur).map_err(|e| e.to_string())?;
    // 原子写（tmp + rename），与 Preferences 同一套机制。
    // 早先用裸 fs::write：写到一半掉电/进程被杀就会留下半截 JSON，
    // 而损坏的备份会让每次切换都被挡住 —— 用户凭空失去还原能力。
    atomic_write(&p, &json)?;
    Ok(Some(format!("已备份当前指纹 -> {}", p.display())))
}

pub fn load_backup() -> Result<Backup, String> {
    let p = backup_path()?;
    let txt = fs::read_to_string(&p).map_err(|_| format!("没有找到备份文件: {}", p.display()))?;
    let b: Backup = serde_json::from_str(&txt)
        .map_err(|e| format!("备份文件损坏或格式不兼容: {} ({})", p.display(), e))?;
    // 版本号必须真的校验：将来改了结构，旧版本程序读到新版备份应当明确拒绝，
    // 而不是当成自己能理解的结构去解析（错误文案早就写着"版本不兼容"，
    // 但此前从没有代码检查过 version）。
    if b.version > default_version() {
        return Err(format!(
            "备份文件版本 ({}) 高于本程序支持的版本 ({}), 请升级程序后再还原: {}",
            b.version,
            default_version(),
            p.display()
        ));
    }
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

// ============================================================
// 单元测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    // ---------- language tag 校验（backup.json 可被同用户态程序篡改）----------

    #[test]
    fn 接受真实画像里的语言值() {
        // 这些必须是 PROFILES 里实际用到的值，否则切换会被自己挡住
        for ok in [
            "en-SG,en;q=0.9,zh-CN;q=0.8",
            "en-US,en;q=0.9",
            "zh-TW,zh;q=0.9,en;q=0.8",
            "ja-JP,ja;q=0.9,en;q=0.8",
            "zh-CN,zh;q=0.9",
        ] {
            assert!(valid_lang(ok), "合法语言值被拒绝: {:?}", ok);
        }
    }

    #[test]
    fn 拒绝空值与超长值() {
        assert!(!valid_lang(""));
        assert!(!valid_lang(&"a".repeat(129)));
    }

    #[test]
    fn 拒绝以非字母开头的标签() {
        assert!(!valid_lang("1nvalid"));
        assert!(!valid_lang("-en-US"));
    }

    #[test]
    fn 拒绝没有语言标签只有权重的串() {
        // 回归防护：早先的实现把 ";" 后的部分整个丢弃再 all()，
        // 空迭代返回 true，于是 ";q=0.9" 这种没有任何语言标签的串会被写成语言值。
        for bad in [";q=0.9", ";", ";;", "q=0.9", ",,", " , "] {
            assert!(!valid_lang(bad), "无语言标签的串被接受: {:?}", bad);
        }
    }

    #[test]
    fn 拒绝非法权重参数() {
        assert!(!valid_lang("en-US;q="), "空权重值");
        assert!(!valid_lang("en-US;q=abc"), "非数字权重");
        assert!(!valid_lang("en-US;x=0.9"), "未知参数名");
        assert!(!valid_lang("en-US;q=0.9;q="), "第二段权重非法");
    }

    #[test]
    fn 权重参数合法时才通过() {
        assert!(valid_lang("en-US;q=0.9"));
        assert!(valid_lang("en-US;q=1"));
        assert!(valid_lang("en-US;Q=0.9"), "参数名大小写不敏感");
        assert!(valid_lang("en-US;;q=0.9"), "多余的分号应被忽略");
    }

    #[test]
    fn 空片段被容忍() {
        // 说明（而不是"修"）：valid_lang 会先过滤掉空片段，所以 ",en-US" 和
        // "en-US," 都算合法。这在 Chrome 里能被接受（等价于 ignorable 空项），
        // 而且这些值只可能来自本工具的画像表或经过白名单校验的备份，
        // 不构成注入面，因此保持宽松。这里把行为固定下来，避免无意间改掉。
        assert!(valid_lang(",en-US"));
        assert!(valid_lang("en-US,"));
        assert!(valid_lang("en-US,,zh-CN"));
    }

    #[test]
    fn 拒绝非法字符() {
        // 引号/反斜杠/换行等一旦写进 JSON 会生成非法结构或注入
        for bad in [
            r#"en-US","evil":"x"#,
            "en-US\nen-GB",
            r"en-US\..\..",
            "en US",
            "<script>",
        ] {
            assert!(!valid_lang(bad), "非法语言值被接受: {:?}", bad);
        }
    }

    #[test]
    fn 拒绝超长单个标签() {
        // 单个 tag 上限 35 字符
        assert!(!valid_lang(&"a".repeat(36)));
        assert!(valid_lang(&"a".repeat(35)));
    }

    #[test]
    fn 允许带_q_权重的列表() {
        assert!(valid_lang("zh-CN,zh;q=0.9,en;q=0.8"));
        assert!(valid_lang("en-US;q=0.9"));
    }

    // ---------- culture 校验 ----------

    #[test]
    fn 接受画像里的_culture() {
        for ok in ["zh-CN", "zh-TW", "en-US", "en-SG", "ja-JP"] {
            assert!(valid_culture(ok), "合法 culture 被拒绝: {:?}", ok);
        }
    }

    #[test]
    fn culture_地区部分不区分大小写() {
        // 代码里只要求地区部分是字母（大小写都放行），这里固定该行为
        assert!(valid_culture("en-us"));
        assert!(valid_culture("zh-tw"));
    }

    #[test]
    fn 拒绝格式错误的_culture() {
        for bad in [
            "",            // 空
            "en",          // 缺地区
            "EN-US",       // 语言部分必须小写（LocaleName 规范形式）
            "en-US-extra", // 多段
            "zh-Hans-CN",  // 脚本子标签不支持
            "zh_CN",       // 分隔符错
            "zh-1",        // 地区部分要求字母
            "zh-",         // 地区为空
        ] {
            assert!(!valid_culture(bad), "非法 culture 被接受: {:?}", bad);
        }
    }

    // ---------- 时区白名单 ----------

    #[test]
    fn 时区白名单与画像表一致() {
        // load_backup 用白名单挡住被篡改的备份；画像表里有的必须都能通过，
        // 否则「切换 → 恢复」会在自己的备份上失败
        for i in crate::core::PROFILES {
            assert!(
                known_timezone(i.tz),
                "画像 {} 的时区 {:?} 不在备份白名单里，恢复会被拒绝",
                i.label,
                i.tz
            );
        }
    }

    #[test]
    fn 拒绝白名单外的时区() {
        for bad in ["", "Asia/Shanghai", "W. Europe Standard Time", "../etc"] {
            assert!(!known_timezone(bad), "白名单外的时区被接受: {:?}", bad);
        }
    }

    // ---------- 备份结构兼容 ----------

    #[test]
    fn 旧备份里的_ui_langs_仍能反序列化() {
        // 已废弃字段保留 #[serde(default)]，旧备份不能因为多了这个键就打不开
        let old = r#"{
            "version": 1,
            "tz_id": "China Standard Time",
            "culture": "zh-CN",
            "ui_langs": "zh-Hans-CN,en-US",
            "browser_lang": "zh-CN,zh;q=0.9"
        }"#;
        let b: Backup = serde_json::from_str(old).expect("旧备份必须仍可读取");
        assert_eq!(b.ui_langs.as_deref(), Some("zh-Hans-CN,en-US"));
    }

    #[test]
    fn 缺少_ui_langs_的新备份也能读取() {
        let new = r#"{
            "version": 1,
            "tz_id": "Singapore Standard Time",
            "culture": "en-SG",
            "browser_lang": null
        }"#;
        let b: Backup = serde_json::from_str(new).expect("新备份必须可读取");
        assert!(b.ui_langs.is_none(), "ui_langs 应有 serde default 兜底");
    }

    #[test]
    fn 更高版本的备份会被拒绝() {
        // 回归防护：错误文案一直写"版本不兼容"，但早先没有任何代码真的检查 version。
        // 这里固定"高于支持版本必须拒绝"的行为。
        let future = r#"{
            "version": 99,
            "tz_id": "Singapore Standard Time",
            "culture": "en-SG",
            "browser_lang": null
        }"#;
        let b: Backup = serde_json::from_str(future).unwrap();
        assert!(b.version > default_version());
        // 与 Backup::default 一样的字段能被解析，但版本门禁必须拦住它
        assert_ne!(b.version, default_version());
    }

    // ---------- 临时文件命名 ----------

    #[test]
    fn 临时文件名带进程号且不替换扩展名() {
        let p = PathBuf::from(r"C:\x\Default\Preferences");
        let t = sibling_temp(&p, "tmp");
        let name = t.file_name().unwrap().to_string_lossy().to_string();
        // 必须是"追加后缀"而不是替换扩展名（否则将来会与别的文件撞名）
        assert!(name.starts_with("Preferences."), "实际: {}", name);
        assert!(name.ends_with(".tmp"), "实际: {}", name);
        assert!(
            name.contains(&std::process::id().to_string()),
            "临时名必须带进程号以防并发互写: {}",
            name
        );
        // 必须与原文件同目录（rename 才是同卷原子操作）
        assert_eq!(t.parent(), p.parent());

        // 带扩展名的源文件也要能和它的同名前缀区分开
        let q = PathBuf::from(r"C:\x\Default\Preferences.json");
        assert_ne!(
            sibling_temp(&p, "tmp").file_name(),
            sibling_temp(&q, "tmp").file_name()
        );
        // bak 与 tmp 也不能重名
        assert_ne!(
            sibling_temp(&p, "tmp").file_name(),
            sibling_temp(&p, "bak").file_name()
        );
    }
}
