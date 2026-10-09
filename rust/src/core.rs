//! 核心逻辑：指纹画像定义、时区/区域语言切换、注册表白名单写入、进程探测
//! 浏览器 Preferences 与备份还原见 browser.rs

use std::path::PathBuf;
use std::process::Command;

// ============================================================
// 指纹画像
// ============================================================
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Profile {
    Singapore,
    California,
    Taipei,
    Tokyo,
    NewYork,
    Shanghai,
}

pub struct ProfileInfo {
    pub key: Profile,
    pub label: &'static str,
    pub sub: &'static str,
    pub tz: &'static str,           // Windows 时区 ID
    pub culture: &'static str,      // 区域格式 (Set-Culture)
    pub ui_lang: &'static str,      // 首选 UI 语言列表 (zh-Hans-CN,en-US)
    pub browser_lang: &'static str, // Chrome/Edge accept_languages
}

pub const PROFILES: &[ProfileInfo] = &[
    ProfileInfo {
        key: Profile::Singapore,
        label: "新加坡",
        sub: "UTC+8 · 时钟零差异",
        tz: "Singapore Standard Time",
        culture: "en-SG",
        ui_lang: "en-US,zh-Hans-CN",
        browser_lang: "en-SG,en;q=0.9,zh-CN;q=0.8",
    },
    ProfileInfo {
        key: Profile::California,
        label: "美国加州",
        sub: "Pacific Time · UTC-7/8",
        tz: "Pacific Standard Time",
        culture: "en-US",
        ui_lang: "en-US,zh-Hans-CN",
        browser_lang: "en-US,en;q=0.9",
    },
    ProfileInfo {
        key: Profile::Taipei,
        label: "台北",
        sub: "UTC+8 · 时钟零差异",
        tz: "Taipei Standard Time",
        culture: "zh-TW",
        ui_lang: "zh-TW,zh-Hans-CN",
        browser_lang: "zh-TW,zh;q=0.9,en;q=0.8",
    },
    ProfileInfo {
        key: Profile::Tokyo,
        label: "东京",
        sub: "UTC+9 · 时钟 +1h",
        tz: "Tokyo Standard Time",
        culture: "ja-JP",
        ui_lang: "ja-JP,en-US",
        browser_lang: "ja-JP,ja;q=0.9,en;q=0.8",
    },
    ProfileInfo {
        key: Profile::NewYork,
        label: "纽约",
        sub: "Eastern Time · UTC-4/5",
        tz: "Eastern Standard Time",
        culture: "en-US",
        ui_lang: "en-US,zh-Hans-CN",
        browser_lang: "en-US,en;q=0.9",
    },
    ProfileInfo {
        key: Profile::Shanghai,
        label: "上海 (默认)",
        sub: "China Standard Time",
        tz: "China Standard Time",
        culture: "zh-CN",
        ui_lang: "zh-Hans-CN,en-US",
        browser_lang: "zh-CN,zh;q=0.9",
    },
];

pub fn info(p: Profile) -> &'static ProfileInfo {
    match PROFILES.iter().find(|i| i.key == p) {
        Some(i) => i,
        // Profile 是有限枚举，全部在表内；走到这里说明表被改坏了
        None => unreachable!("ProfileInfo 表缺少某个 Profile 条目"),
    }
}

// ============================================================
// 指纹读取
// ============================================================
#[derive(Clone, Debug)]
pub struct Fingerprint {
    pub tz_id: String,
    pub is_china_tz: bool,
    pub now: String,
    pub culture: String,
    pub sys_locale: String,
    pub geo_id: String,
    pub browser_lang: String,
    pub chrome_running: bool,
    pub edge_running: bool,
}

pub fn read_fingerprint() -> Fingerprint {
    let tz_id = get_timezone();
    let culture = get_culture();
    let browser_lang = crate::browser::read_chrome_lang().unwrap_or_else(|| "未检测到".into());

    Fingerprint {
        is_china_tz: tz_id == "China Standard Time",
        tz_id,
        now: chrono::Local::now().format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        culture,
        sys_locale: read_reg_str(
            winreg::enums::HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Nls\CodePage",
            "ACP",
        )
        .unwrap_or_default(),
        geo_id: read_reg_str(
            winreg::enums::HKEY_CURRENT_USER,
            r"Control Panel\International\Geo",
            "Nation",
        )
        .unwrap_or_default(),
        browser_lang,
        chrome_running: process_running("chrome.exe"),
        edge_running: process_running("msedge.exe"),
    }
}

/// 指纹风险评分。
/// 只统计**本工具实际能改变**的项（时区 + 区域语言），分母随之改为 65。
/// ACP(系统代码页) 与 Nation(区域位置) 工具不写（ACP 要管理员+重启），
/// 若把它们算进分数，切换后分数永远降不到 0，会与界面上的「已规避」自相矛盾。
pub fn risk_score(f: &Fingerprint) -> (u32, &'static str) {
    let mut s = 0;
    if f.is_china_tz {
        s += 50;
    }
    if f.culture == "zh-CN" {
        s += 15;
    }
    let level = if s >= 50 {
        "高危"
    } else if s >= 20 {
        "中危"
    } else if s > 0 {
        "低危"
    } else {
        "安全"
    };
    (s, level)
}

// ============================================================
// 注册表工具
// ============================================================
pub fn read_reg_str(hive: winreg::HKEY, path: &str, name: &str) -> Option<String> {
    let k = winreg::RegKey::predef(hive);
    let sub = k.open_subkey(path).ok()?;
    sub.get_value::<String, _>(name).ok()
}

/// 只写 HKCU 的受支持键位。
/// 白名单而非任意 path/name：这是唯一的注册表写入口，必须让调用方无法
/// 拼出任意路径去写系统其他位置。权限用 KEY_SET_VALUE 而非 KEY_ALL_ACCESS，
/// 本工具只需要设值，不需要改 DAC/所有者。
fn write_hkcu_value(path: &str, name: &str, val: &str) -> Result<(), String> {
    const SUPPORTED: &[(&str, &[&str])] = &[
        (
            r"Control Panel\International",
            &["Locale", "LocaleName", "sLanguage"],
        ),
        (r"Control Panel\Desktop", &["PreferredUILanguages"]),
        (
            r"Control Panel\International\User Profile",
            &["Languages"],
        ),
    ];

    let allowed = SUPPORTED
        .iter()
        .any(|(p, names)| *p == path && names.contains(&name));
    if !allowed {
        return Err(format!("拒绝写入未支持的注册表位置: {}\\{}", path, name));
    }

    let k = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (sub, _) = k
        .create_subkey(path)
        .map_err(|e| format!("打开注册表失败 {}: {}", path, e))?;
    // REG_SZ
    sub.set_value(name, &val)
        .map_err(|e| format!("写入注册表失败 {}\\{}: {}", path, name, e))
}

/// 写 REG_MULTI_SZ（多字符串）
fn write_hkcu_multi(path: &str, name: &str, vals: &[String]) -> Result<(), String> {
    const SUPPORTED: &[(&str, &[&str])] = &[
        (r"Control Panel\Desktop", &["PreferredUILanguages"]),
        (
            r"Control Panel\International\User Profile",
            &["Languages"],
        ),
    ];
    let allowed = SUPPORTED
        .iter()
        .any(|(p, names)| *p == path && names.contains(&name));
    if !allowed {
        return Err(format!("拒绝写入未支持的注册表位置: {}\\{}", path, name));
    }

    let k = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (sub, _) = k
        .create_subkey(path)
        .map_err(|e| format!("打开注册表失败 {}: {}", path, e))?;
    sub.set_value::<Vec<String>, _>(name, &vals.to_vec())
        .map_err(|e| format!("写入注册表失败 {}\\{}: {}", path, name, e))
}

/// 读取当前首选 UI 语言列表（REG_MULTI_SZ），失败返回 None
pub fn read_ui_languages() -> Option<String> {
    let k = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let sub = k.open_subkey(r"Control Panel\Desktop").ok()?;
    let v: Vec<String> = sub.get_value("PreferredUILanguages").ok()?;
    if v.is_empty() {
        None
    } else {
        Some(v.join(","))
    }
}

// ============================================================
// 时区
// ============================================================
pub fn get_timezone() -> String {
    // 优先读注册表，避免依赖本地化显示名；空值一律过滤，防止假阴性
    let from_reg = read_reg_str(
        winreg::enums::HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\TimeZoneInformation",
        "TimeZoneKeyName",
    )
    .filter(|s| !s.trim().is_empty());

    from_reg
        .or_else(|| {
            // 兜底: 用 tzutil 读，必须退出码成功且输出非空
            Command::new(sys_tool("tzutil.exe"))
                .arg("/g")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "未知".into())
}

/// 切换时区。系统级设置，普通用户即可改。
pub fn set_timezone(tz: &str) -> Result<(), String> {
    let out = Command::new(sys_tool("tzutil.exe"))
        .args(["/s", tz])
        .output()
        .map_err(|e| format!("执行 tzutil 失败: {}", e))?;
    if out.status.success() {
        // 广播 WM_SETTINGCHANGE，让已运行的进程知道时区变了
        broadcast_setting_change();
        Ok(())
    } else {
        Err(format!(
            "tzutil 报错: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// 广播 WM_SETTINGCHANGE（SendMessageTimeout HWND_BROADCAST）
/// tzutil /s 本身会广播时区变更，但区域/语言变更不会，这里补上
fn broadcast_setting_change() {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;

    #[link(name = "user32", kind = "dylib")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: *mut core::ffi::c_void,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    const WM_SETTINGCHANGE: u32 = 0x001A;
    const HWND_BROADCAST: *mut core::ffi::c_void = 0xFFFF as *mut core::ffi::c_void;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    let wide: Vec<u16> = OsStr::new("Environment")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut result: usize = 0;

    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            wide.as_ptr(),
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        );
    }
}

// ============================================================
// 语言 / 区域格式
// ============================================================
pub fn get_culture() -> String {
    // 读 Set-Culture 写入的那个键
    read_reg_str(
        winreg::enums::HKEY_CURRENT_USER,
        r"Control Panel\International",
        "LocaleName",
    )
    .unwrap_or_else(|| "未知".into())
}

/// 切区域格式：改 Locale / LocaleName / sLanguage / sCountry 四个键
/// (等价 Set-Culture，但不需要 PowerShell)
pub fn set_culture(culture: &str) -> Result<(), String> {
    // culture 形如 "en-US"; Locale 是 "00000409" 这种十六进制 LCID
    let lcid = culture_to_lcid(culture);

    write_hkcu_value(
        r"Control Panel\International",
        "Locale",
        &lcid,
    )?;
    write_hkcu_value(
        r"Control Panel\International",
        "LocaleName",
        culture,
    )?;
    // sLanguage 只在能确定映射时写；未知 culture 宁可不写也不写错值
    if let Some(code) = culture_lang_code(culture) {
        write_hkcu_value(
            r"Control Panel\International",
            "sLanguage",
            code,
        )?;
    }
    // sCountry 故意不碰：Set-Culture 也不写它，写空串会让部分程序读到空值
    Ok(())
}

/// 切首选 UI 语言列表（影响新进程的 CurrentUICulture / 应用语言）
pub fn set_ui_languages(list: &str) -> Result<(), String> {
    let items: Vec<String> = list.split(',').map(|s| s.trim().to_string()).collect();
    if items.iter().any(|s| s.is_empty()) {
        return Err("语言列表含有空项".into());
    }
    // Windows 用 REG_MULTI_SZ 存两处：Desktop\PreferredUILanguages（系统读）
    // 与 International\User Profile\Languages（PowerShell Get-WinUserLanguageList 读）
    write_hkcu_multi(r"Control Panel\Desktop", "PreferredUILanguages", &items)?;
    write_hkcu_multi(
        r"Control Panel\International\User Profile",
        "Languages",
        &items,
    )?;
    Ok(())
}

fn culture_to_lcid(c: &str) -> String {
    // 常用 LCID; 未知则退回 0x0409 (en-US)
    let map: &[(&str, &str)] = &[
        ("zh-CN", "00000804"),
        ("zh-TW", "00000404"),
        ("en-US", "00000409"),
        ("en-SG", "00004809"),
        ("ja-JP", "00000411"),
    ];
    map.iter()
        .find(|(k, _)| *k == c)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| "00000409".into())
}

/// 按完整 culture 映射 sLanguage（三字母语言标识）。
/// zh-TW 必须是 CHT（繁体），不能按前缀猜成 CHS —— 那会和 LocaleName=zh-TW 自相矛盾。
fn culture_lang_code(c: &str) -> Option<&'static str> {
    match c {
        "zh-CN" => Some("CHS"),
        "zh-TW" => Some("CHT"),
        "en-US" | "en-SG" => Some("ENU"),
        "ja-JP" => Some("JPN"),
        _ => None, // 未知 culture 不写 sLanguage，而不是猜一个错值
    }
}

// ============================================================
// 进程
// ============================================================
pub fn process_running(name: &str) -> bool {
    // 绝对路径 + 检查退出码：tasklist 失败时 stdout 为空，若不看退出码
    // 会误报"浏览器未运行"，把关键的"需重启浏览器"提示吞掉
    if let Ok(o) = Command::new(sys_tool("tasklist.exe"))
        .args(["/FI", &format!("IMAGENAME eq {}", name)])
        .output()
    {
        if !o.status.success() {
            return false;
        }
        let s = String::from_utf8_lossy(&o.stdout).to_lowercase();
        return s.contains(&name.to_lowercase());
    }
    false
}

/// 系统工具绝对路径。Command::new("tzutil") 会按 PATH 搜索，
/// 用户 PATH 前置目录里的同名 exe 就能劫持（Windows 上经典提权面）。
fn sys_tool(name: &str) -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}
