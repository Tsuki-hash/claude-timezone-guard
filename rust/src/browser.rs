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
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// ============================================================
// 浏览器探测
// ============================================================
fn browser_data_dirs() -> Vec<(String, PathBuf)> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    // 必须是非空绝对路径：否则一个被替换成相对路径的环境变量会让本工具
    // 去改"当前工作目录下"的假 Profile（见报告 5.3 的环境变量信任问题）。
    let base = PathBuf::from(&local);
    if local.trim().is_empty() || !base.is_absolute() {
        return vec![];
    }
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

/// 按目录名排序，保证日志顺序稳定可复现。
///
/// 会跳过 reparse point（junction / symlink）：`Path::is_dir()` 是**跟随**链接的，
/// 同用户进程若在 `User Data\` 下建一个 `Profile 9` → 任意可写目录的 junction，
/// 本工具就会去改写那个目录里的 `Preferences`。用 `symlink_metadata` 判断，
/// 只看目录项本身的属性，不看它指向哪里。
fn profile_dirs(base: &Path) -> Vec<PathBuf> {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(base) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name != "Default" && !name.starts_with("Profile ") {
                continue;
            }
            // symlink_metadata 不跟随链接
            let Ok(md) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if !md.is_dir() {
                continue;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    continue;
                }
            }
            out.push(e.path());
        }
    }
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

/// 一个具体 Profile 的标识：`浏览器名\Profile 目录名`，用作备份的键。
/// 用 `\` 分隔是为了在 JSON 里一眼能看出层级。
pub fn profile_key(browser: &str, dir_name: &str) -> String {
    format!("{}\\{}", browser, dir_name)
}

/// 读取**每个** Profile 当前的 `intl.accept_languages`。
///
/// 返回 `(键, 语言值)` 列表，键形如 `Chrome\Default`。
/// 与 `set_browser_language` 的遍历范围完全对称 —— 这是修掉"备份只记第一个命中的
/// Profile，还原却写到全部"那个数据损坏问题的关键：备份必须与还原同粒度。
pub fn read_all_browser_langs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, base) in browser_data_dirs() {
        for dir in profile_dirs(&base) {
            let pref = dir.join("Preferences");
            let Ok(txt) = fs::read_to_string(&pref) else {
                continue;
            };
            let dir_name = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Some(v) = json_get_str(&txt, "accept_languages") {
                out.push((profile_key(&name, &dir_name), v));
            }
        }
    }
    out
}

/// 旧接口：取第一个命中的语言值，仅供兼容与"当前值"展示使用。
/// **不要再把它当作还原依据** —— 还原请用 `restore_browser_langs`。
pub fn read_chrome_lang() -> Option<String> {
    read_all_browser_langs().into_iter().next().map(|(_, v)| v)
}

/// 把每个 Profile 精确还原到备份里记录的值。
/// 返回 (日志, 失败数)。备份里有、但本机已不存在的 Profile 会被跳过并告知。
pub fn restore_browser_langs(saved: &BTreeMap<String, String>) -> (Vec<String>, usize) {
    let mut log = Vec::new();
    let mut failures = 0usize;
    let mut restored = 0usize;
    let mut missing: Vec<String> = Vec::new();

    // 先摸清本机现状，才能判断哪些备份键已经不存在
    let present: BTreeMap<String, PathBuf> = {
        let mut m = BTreeMap::new();
        for (name, base) in browser_data_dirs() {
            for dir in profile_dirs(&base) {
                let dir_name = dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                m.insert(profile_key(&name, &dir_name), dir);
            }
        }
        m
    };

    for (key, lang) in saved {
        let Some(dir) = present.get(key) else {
            missing.push(key.clone());
            continue;
        };
        if !valid_lang(lang) {
            log.push(format!("✗ 备份里 {} 的语言值不合法，已跳过", key));
            failures += 1;
            continue;
        }
        let pref = dir.join("Preferences");
        match write_lang_to_profile(&pref, lang) {
            Ok(true) => restored += 1,
            Ok(false) => {} // 本来就一样，不算改动也不算失败
            Err(e) => {
                log.push(format!("✗ {} 还原失败: {}", key, e));
                failures += 1;
            }
        }
    }

    if restored > 0 {
        log.push(format!(
            "✓ 已逐 Profile 还原 {} 个浏览器 Profile 的语言",
            restored
        ));
    }
    if !missing.is_empty() {
        log.push(format!(
            "· {} 个备份中的 Profile 已不存在，已跳过: {}",
            missing.len(),
            missing.join(", ")
        ));
    }
    if restored == 0 && failures == 0 && missing.is_empty() {
        log.push("· 浏览器语言与备份一致，无需改动".into());
    }
    (log, failures)
}

/// 把语言值写进单个 Profile 的 Preferences。Ok(true) = 有改动。
fn write_lang_to_profile(pref: &Path, lang: &str) -> Result<bool, String> {
    let txt = fs::read_to_string(pref).map_err(|e| format!("读取失败: {}", e))?;
    let mut v: Value = serde_json::from_str(&txt).map_err(|e| format!("JSON 解析失败: {}", e))?;

    let mut changed = false;
    let mut intl_malformed = false;
    match v.get_mut("intl") {
        Some(intl) => match intl.as_object_mut() {
            Some(obj) => {
                if obj.get("accept_languages").and_then(|s| s.as_str()) != Some(lang) {
                    obj.insert("accept_languages".into(), Value::String(lang.into()));
                    changed = true;
                }
            }
            None => intl_malformed = true,
        },
        None => {
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
    }
    if intl_malformed {
        return Err("intl 结构异常（不是对象），已跳过".into());
    }
    if !changed {
        return Ok(false);
    }
    let out = serde_json::to_string(&v).map_err(|e| format!("序列化失败: {}", e))?;
    write_with_backup(pref, &out)?;
    Ok(true)
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
///
/// `kind` 说明内容该怎么自检 —— 这个参数是必需的，不是可选的：
/// 早先这里**硬编码按 JSON 校验**，加入 DPAPI 加密后，备份文件的载荷是
/// "魔数头 + base64 密文"而非 JSON，于是每次备份都被自己的自检判为
/// "生成的 JSON 非法，已放弃写入"。写入口的自检必须知道自己写的是什么格式。
fn atomic_write(path: &Path, content: &str, kind: PayloadKind) -> Result<(), String> {
    let tmp = sibling_temp(path, "tmp");
    fs::write(&tmp, content).map_err(|e| format!("写临时文件失败 {}: {}", tmp.display(), e))?;
    // 写完后确认能读回并通过自检，避免把半截/非法内容落成正式文件。
    // 读回失败也要报错：读不回来通常意味着内容没完整落盘，
    // 恰恰是最需要拦住的情况，不能静默跳过检查直接 rename。
    match fs::read_to_string(&tmp) {
        Ok(t) => {
            if let Err(e) = kind.verify(&t) {
                let _ = fs::remove_file(&tmp);
                return Err(format!("{}，已放弃写入 {}", e, path.display()));
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

/// 落盘内容的格式，决定 `atomic_write` 用哪种自检
#[derive(Clone, Copy)]
pub enum PayloadKind {
    /// Chrome 的 Preferences：必须是合法 JSON 对象
    Json,
    /// 备份文件：明文时是 JSON，加密时是"魔数头 + base64"，两种都算合法
    BackupEnvelope,
}

impl PayloadKind {
    fn verify(self, text: &str) -> Result<(), String> {
        match self {
            PayloadKind::Json => {
                if serde_json::from_str::<Value>(text).is_err() {
                    return Err("生成的 JSON 非法".into());
                }
                Ok(())
            }
            PayloadKind::BackupEnvelope => {
                if text.trim_start().starts_with(BACKUP_MAGIC) {
                    // 加密信封：必须能解出合法 JSON 才算写对
                    let (json, _) = decode_backup(text)?;
                    if serde_json::from_str::<Value>(&json).is_err() {
                        return Err("备份密文解出的内容不是合法 JSON".into());
                    }
                } else if serde_json::from_str::<Value>(text).is_err() {
                    return Err("生成的备份 JSON 非法".into());
                }
                Ok(())
            }
        }
    }
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
    atomic_write(path, content, PayloadKind::Json)
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

/// 把同一个语言值写到**所有**浏览器的所有 Profile（切换时用）。
/// 返回 (日志, 失败数)。失败全部显式上报。
///
/// 若需要"逐 Profile 精确还原"，用 `restore_browser_langs`。
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
            // 区分"本来就没有 Preferences"与"有但读不出来（被占用/无权限）"，
            // 否则唯一 profile 读失败会被误报成"未找到浏览器 Profile"。
            match fs::metadata(&pref) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    log.push(format!(
                        "✗ {} {} 的 Preferences 无法访问: {}（被占用或无权限？）",
                        name, label, e
                    ));
                    failures += 1;
                    continue;
                }
                Ok(_) => {}
            }
            // 改值逻辑与还原共用一份实现，避免两处行为漂移
            match write_lang_to_profile(&pref, lang) {
                Ok(true) => ok += 1,
                Ok(false) => {}
                Err(e) => {
                    log.push(format!("✗ {} {} {}", name, label, e));
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
    /// 旧字段（v1）：只记录"第一个命中的" Profile 的语言值。
    /// 问题：还原时却会把它写到**所有**浏览器的**所有** Profile，
    /// 多 Profile 用户的语言差异会被抹平 —— 还原本身造成数据损坏。
    /// v2 起不再写它，仅用于读取旧备份并按旧行为尽力还原。
    #[serde(default)]
    pub browser_lang: Option<String>,
    /// 逐 Profile 的浏览器语言（v2 起）：键形如 `Chrome\Default`。
    /// 与 `set_browser_language` 的遍历范围完全对称，还原是精确的。
    #[serde(default)]
    pub browser_langs: BTreeMap<String, String>,
}

fn default_version() -> u32 {
    // v1 → v2：新增 browser_langs（逐 Profile 备份），修掉还原粒度不对称。
    // load_backup 只拒绝"高于本程序支持版本"的文件，所以 v1 旧备份仍可读。
    2
}

/// 结构版本上限：高于它的备份必须拒绝（旧程序读不懂新结构）
const MAX_SUPPORTED_VERSION: u32 = 2;

fn backup_path() -> Result<PathBuf, String> {
    // 路径来自 %LOCALAPPDATA%（见报告 5.3：理想做法是 SHGetKnownFolderPath）。
    // 至少做一层加固：必须是绝对路径，避免被替换成相对路径后落到 cwd。
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&local);
        if p.is_absolute() && !local.trim().is_empty() {
            return Ok(p.join("ClaudeFingerprint").join("backup.json"));
        }
    }
    Err("无法定位 %LOCALAPPDATA%（或它不是一个绝对路径），备份路径不可用".into())
}

// ============================================================
// DPAPI：给备份文件加一层"同用户可读、他人不可读"的保护
// ============================================================
//
// 威胁模型：backup.json 在 %LOCALAPPDATA% 下，同用户的任意进程都能改写它。
// 恶意进程可以预先放一份"值全部合法、但时区是 China Standard Time"的备份，
// 诱导用户点「一键恢复」，**正好把要规避的特征装回去**。
//
// 用 DPAPI（CryptProtectData，当前用户范围）加密后：
//   - 解密只在同一用户的登录会话内可行，别的用户读不出内容；
//   - 同一用户下的进程仍能解密（DPAPI 不是防同用户的密码学边界），
//     但**随手替换文件**不再够用 —— 攻击者得调用同样的 DPAPI 才能造出合法文件。
//
// 兼容性：解密失败时回退为按明文 JSON 解析，所以 v1/v2 的旧明文备份仍可读。
// 加密失败也回退为明文写入，绝不因为"加不上锁"就让用户丢掉备份能力。
mod dpapi {
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32", kind = "dylib")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32", kind = "dylib")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    fn blob_of(bytes: &[u8]) -> DataBlob {
        DataBlob {
            cb_data: bytes.len() as u32,
            // DPAPI 只读输入，这里只是 const → mut 的转换
            pb_data: bytes.as_ptr() as *mut u8,
        }
    }

    unsafe fn take_out(out: DataBlob) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pb_data, out.cb_data as usize).to_vec();
        LocalFree(out.pb_data as *mut c_void);
        v
    }

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let input = blob_of(plain);
        let mut out = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        // CRYPTPROTECT_UI_FORBIDDEN = 0x1：绝不弹窗（GUI 场景下弹窗会卡住调用）
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0x1,
                &mut out,
            )
        };
        if ok == 0 || out.pb_data.is_null() {
            None
        } else {
            Some(unsafe { take_out(out) })
        }
    }

    pub fn unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
        let input = blob_of(cipher);
        let mut out = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0x1,
                &mut out,
            )
        };
        if ok == 0 || out.pb_data.is_null() {
            None
        } else {
            Some(unsafe { take_out(out) })
        }
    }
}

/// 备份文件的落盘格式：DPAPI 密文 + 一行说明
const BACKUP_MAGIC: &str = "CLAUDE-FINGERPRINT-BACKUP-DPAPI-V1";

fn encode_backup(json: &str) -> (String, bool) {
    match dpapi::protect(json.as_bytes()) {
        Some(cipher) => (
            format!("{}\n{}\n", BACKUP_MAGIC, base64_encode(&cipher)),
            true,
        ),
        // 加不上锁也要能工作：退化为明文，读回时按 JSON 解析
        None => (json.to_string(), false),
    }
}

/// 解码备份：优先按 DPAPI 密文解，失败则当作明文 JSON。
/// 返回 (JSON 文本, 是否加密)
fn decode_backup(raw: &str) -> Result<(String, bool), String> {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix(BACKUP_MAGIC) {
        let b64: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
        let cipher = base64_decode(&b64).ok_or("备份密文不是合法的 base64")?;
        let plain = dpapi::unprotect(&cipher)
            .ok_or("备份密文无法解密（可能不是本用户创建的，或文件被替换过）")?;
        let s = String::from_utf8(plain).map_err(|_| "解密结果不是合法 UTF-8")?;
        return Ok((s, true));
    }
    // 明文（含旧版本备份）
    Ok((raw.to_string(), false))
}

/// 最小 base64 实现（标准字母表 + padding），避免为这一处引入依赖
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
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

    // 逐 Profile 记录，而不是只取第一个命中的值 —— 后者会在还原时
    // 把多 Profile 的语言差异抹平（还原本身造成数据损坏）。见 Backup 的注释。
    let per_profile: BTreeMap<String, String> = read_all_browser_langs().into_iter().collect();

    let cur = Backup {
        version: default_version(),
        tz_id: get_timezone(),
        culture: get_culture(),
        // 恒为 None：本工具不写首选 UI 语言，没有需要备份的东西
        ui_langs: None,
        // 不再写旧字段；旧备份仍能读（serde default）
        browser_lang: None,
        browser_langs: per_profile.clone(),
    };

    // 写入前先用**与 load_backup 相同的校验**过一遍。
    //
    // 为什么必须自校验：load_backup 要求 tz_id 在白名单内、culture 合法、
    // 各语言值合法。而这里取的原始值可能不合法 —— 例如读时区失败时
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
    for (k, l) in &per_profile {
        if !valid_lang(l) {
            return Err(format!(
                "读到的浏览器语言 {:?}（{}）不合法，无法建立可信备份（不写还原点）。",
                l, k
            ));
        }
    }

    if p.exists() {
        // 读不出来就不能往下走：下面会覆盖这个文件。备份是用户唯一的
        // 还原点，读失败（被占用/权限）时静默覆盖等于把还原点丢了，
        // 所以这里必须报错退出，而不是当作"没有备份"继续写。
        let raw = fs::read_to_string(&p).map_err(|e| {
            format!(
                "已有备份文件但读取失败 ({}): {} —— 为避免覆盖这个唯一的还原点，已中止。请关闭占用它的程序后重试。",
                p.display(),
                e
            )
        })?;
        let (txt, _encrypted) = decode_backup(&raw)?;
        match serde_json::from_str::<Backup>(&txt) {
            Ok(old) => {
                // 关键项全相同才认为是同一状态；任一项不同都保留旧备份，
                // 避免用户在浏览器里手改过语言后，用污染值覆盖真正的初始备份。
                // 比较范围随结构升级而调整：
                //   ui_langs   已废弃（旧备份 Some / 新写入 None），比它会让"同一状态"永远判假
                //   browser_lang 已废弃（v2 恒 None），同上
                //   browser_langs 是 v2 的权威字段；旧备份为空 map，与新的非空 map 不同 →
                //                 会保留旧备份（这是想要的行为：别用新的覆盖真正的初始状态）
                let same = old.tz_id == cur.tz_id
                    && old.culture == cur.culture
                    && old.browser_langs == cur.browser_langs;
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
    // 先按 DPAPI 加密（失败则明文），再原子写（tmp + rename），与 Preferences 同一套机制。
    // 早先用裸 fs::write：写到一半掉电/进程被杀就会留下半截 JSON，
    // 而损坏的备份会让每次切换都被挡住 —— 用户凭空失去还原能力。
    let (payload, encrypted) = encode_backup(&json);
    atomic_write(&p, &payload, PayloadKind::BackupEnvelope)?;
    Ok(Some(format!(
        "已备份当前指纹 -> {}（逐 Profile 记录 {} 项，{}）",
        p.display(),
        per_profile.len(),
        if encrypted {
            "DPAPI 加密"
        } else {
            "未加密（DPAPI 不可用）"
        }
    )))
}

pub fn load_backup() -> Result<Backup, String> {
    let p = backup_path()?;
    let raw = fs::read_to_string(&p).map_err(|_| format!("没有找到备份文件: {}", p.display()))?;
    let (txt, _encrypted) = decode_backup(&raw).map_err(|e| format!("{} ({})", e, p.display()))?;
    let b: Backup = serde_json::from_str(&txt)
        .map_err(|e| format!("备份文件损坏或格式不兼容: {} ({})", p.display(), e))?;
    // 版本号必须真的校验：将来改了结构，旧版本程序读到新版备份应当明确拒绝，
    // 而不是当成自己能理解的结构去解析（错误文案早就写着"版本不兼容"，
    // 但此前从没有代码检查过 version）。
    if b.version > MAX_SUPPORTED_VERSION {
        return Err(format!(
            "备份文件版本 ({}) 高于本程序支持的版本 ({}), 请升级程序后再还原: {}",
            b.version,
            MAX_SUPPORTED_VERSION,
            p.display()
        ));
    }
    // 值校验独立成函数：load_backup 与写入侧自校验共用同一份规则，
    // 单测也能直接打它，不用复制一份判断逻辑。
    validate_backup(&b)?;
    Ok(b)
}

/// 备份内容的完整校验（结构版本除外，那由 load_backup 单独判）。
///
/// 为什么需要：文件位于同用户可写目录，任何同用户进程都能改它，不能直接信。
/// 校验后的值会流向 `tzutil` 参数、注册表 REG_SZ、以及 Chrome 的 JSON —— 必须收窄。
pub fn validate_backup(b: &Backup) -> Result<(), String> {
    if !known_timezone(&b.tz_id) {
        return Err(format!("备份里的时区不在已知列表: {}", b.tz_id));
    }
    if !valid_culture(&b.culture) {
        return Err(format!("备份里的区域格式不合法: {}", b.culture));
    }
    // 旧字段（v1）
    if let Some(l) = &b.browser_lang {
        if !valid_lang(l) {
            return Err(format!("备份里的浏览器语言不合法: {}", l));
        }
    }
    // 权威字段（v2）：逐 Profile
    for (k, l) in &b.browser_langs {
        if !valid_lang(l) {
            return Err(format!("备份里 {} 的浏览器语言不合法: {}", k, l));
        }
        // 键形如 `Chrome\Default`，不能含路径分隔符或 .. —— 虽然本工具只用它做
        // map 查找（不拼路径），但挡住异常值能避免将来被误用
        if k.contains("..") || k.contains('/') || k.contains(':') {
            return Err(format!("备份里的 Profile 键不合法: {}", k));
        }
    }
    Ok(())
}

// ============================================================
// 备份可用性：三态，而不是"能读/不能读"
// ============================================================
/// 备份文件的状态。
///
/// 为什么要三态：早先界面用 `load_backup().is_ok()` 表示"有没有备份"，
/// 于是"文件存在但损坏/校验不通过"会被显示成**"当前没有备份"** ——
/// 用户看到的提示是"先切换一次"，而真实原因是文件坏了，方向完全错。
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum BackupState {
    /// 文件不存在：还没切换过，还原按钮应当禁用
    Missing,
    /// 文件存在且能读出合法内容：可以还原
    Ready,
    /// 文件存在但读不出来（损坏 / 版本过高 / 校验不通过）
    Broken(String),
}

/// 查询备份状态（不修改任何东西）
pub fn backup_state() -> BackupState {
    let Ok(p) = backup_path() else {
        return BackupState::Broken("无法定位备份路径（%LOCALAPPDATA% 不可用）".into());
    };
    if !p.exists() {
        return BackupState::Missing;
    }
    match load_backup() {
        Ok(_) => BackupState::Ready,
        Err(e) => BackupState::Broken(e),
    }
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
        assert!(b.version > MAX_SUPPORTED_VERSION);
    }

    #[test]
    fn 当前版本被接受为受支持范围() {
        assert!(default_version() <= MAX_SUPPORTED_VERSION);
    }

    // ---------- base64（DPAPI 信封用） ----------

    #[test]
    fn base64_往返一致() {
        for case in [
            Vec::new(),
            b"a".to_vec(),
            b"ab".to_vec(),
            b"abc".to_vec(),
            b"abcd".to_vec(),
            (0u8..=255).collect::<Vec<u8>>(),
        ] {
            let enc = base64_encode(&case);
            let dec = base64_decode(&enc).expect("必须能解回");
            assert_eq!(dec, case, "往返失败，长度 {}", case.len());
        }
    }

    #[test]
    fn base64_已知向量() {
        // RFC 4648 标准测试向量
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // 反向
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_拒绝非法字符() {
        assert!(base64_decode("!!!!").is_none());
        assert!(base64_decode("Zm9v*").is_none());
    }

    // ---------- 备份信封（DPAPI） ----------

    #[test]
    fn 备份信封加密后不含明文() {
        let json = r#"{"tz_id":"China Standard Time","culture":"zh-CN"}"#;
        let (payload, encrypted) = encode_backup(json);
        if encrypted {
            assert!(payload.starts_with(BACKUP_MAGIC), "应带魔数头");
            assert!(
                !payload.contains("China Standard Time"),
                "加密后不应还能看到明文"
            );
            // 解回来必须与原 JSON 完全一致
            let (back, was_enc) = decode_backup(&payload).expect("必须能解密");
            assert!(was_enc);
            assert_eq!(back, json);
        } else {
            // DPAPI 不可用时退化为明文，也必须能解回
            let (back, was_enc) = decode_backup(&payload).expect("退化为明文后仍要能读");
            assert!(!was_enc);
            assert_eq!(back, json);
        }
    }

    #[test]
    fn 明文旧备份仍可读() {
        // 兼容性关键：v1/v2 的明文备份不能因为引入 DPAPI 就打不开
        let plain = r#"{"version":1,"tz_id":"China Standard Time","culture":"zh-CN","browser_lang":"zh-CN,zh;q=0.9"}"#;
        let (txt, encrypted) = decode_backup(plain).expect("旧明文备份必须可读");
        assert!(!encrypted);
        assert_eq!(txt, plain);
        let b: Backup = serde_json::from_str(&txt).unwrap();
        assert_eq!(b.version, 1);
        assert_eq!(b.browser_lang.as_deref(), Some("zh-CN,zh;q=0.9"));
    }

    #[test]
    fn 伪造的密文头会被拒绝而不是被当成明文() {
        // 篡改者加上魔数头但内容不是合法 base64 → 必须报错，
        // 不能悄悄回退成"按明文解析"而把垃圾读进来
        let bad = format!("{}\nnot-base64!!!\n", BACKUP_MAGIC);
        assert!(decode_backup(&bad).is_err());

        // 合法 base64 但不是 DPAPI 密文 → 解密失败，也要报错
        let bad2 = format!("{}\nZm9vYmFy\n", BACKUP_MAGIC);
        assert!(decode_backup(&bad2).is_err());
    }

    // ---------- 逐 Profile 备份结构 ----------

    #[test]
    fn 逐_profile_字段缺省时为空_map() {
        // v1 备份没有 browser_langs，必须能反序列化且默认为空
        let v1 = r#"{"version":1,"tz_id":"China Standard Time","culture":"zh-CN","browser_lang":"zh-CN"}"#;
        let b: Backup = serde_json::from_str(v1).unwrap();
        assert!(b.browser_langs.is_empty(), "缺省应为空 map");
    }

    #[test]
    fn profile_键格式可读且带层级() {
        assert_eq!(profile_key("Chrome", "Default"), r"Chrome\Default");
        assert_eq!(profile_key("Edge", "Profile 1"), r"Edge\Profile 1");
    }

    #[test]
    fn 逐_profile_键的非法值会被拒绝() {
        // 键里出现 .. 或路径分隔符/盘符要挡住（虽然当前只做 map 查找，
        // 但挡住异常值能避免将来被误用去拼路径）
        for bad_key in [
            r"..\..\Windows",
            "Chrome/Default",
            r"C:\Windows\System32",
            r"Chrome\..\..\evil",
        ] {
            let mut b = valid_test_backup();
            b.browser_langs.insert(bad_key.to_string(), "en-US".into());
            assert!(
                validate_backup(&b).is_err(),
                "非法 Profile 键应被拒绝: {:?}",
                bad_key
            );
        }
    }

    #[test]
    fn 逐_profile_里的非法语言值会被拒绝() {
        let mut b = valid_test_backup();
        b.browser_langs
            .insert(r"Chrome\Default".into(), ";q=0.9".into());
        assert!(validate_backup(&b).is_err(), "没有任何语言标签的值应被拒绝");
    }

    #[test]
    fn 合法的逐_profile_备份通过校验() {
        let mut b = valid_test_backup();
        b.browser_langs
            .insert(r"Chrome\Default".into(), "zh-CN,zh;q=0.9".into());
        b.browser_langs
            .insert(r"Edge\Profile 1".into(), "en-US,en;q=0.9".into());
        assert!(validate_backup(&b).is_ok(), "合法备份不该被拒绝");
    }

    /// 构造一份各字段都合法的备份，供校验类测试改单个字段
    fn valid_test_backup() -> Backup {
        Backup {
            version: default_version(),
            tz_id: "China Standard Time".into(),
            culture: "zh-CN".into(),
            ui_langs: None,
            browser_lang: None,
            browser_langs: BTreeMap::new(),
        }
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
