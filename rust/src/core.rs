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
    pub tz: &'static str,      // Windows 时区 ID
    pub culture: &'static str, // 区域格式 (Set-Culture)
    pub browser_lang: &'static str, // Chrome/Edge accept_languages
                               // 刻意没有 ui_lang：首选 UI 语言 (PreferredUILanguages) 本工具只读不写。
                               // 写入它需要目标语言包已安装（HKLM\...\MUI\UILanguages），否则开始菜单/设置
                               // 会回退成英文或乱码，且必须注销才生效 —— 收益为零、风险很高，见 README。
}

pub const PROFILES: &[ProfileInfo] = &[
    ProfileInfo {
        key: Profile::Singapore,
        label: "新加坡",
        sub: "UTC+8 · 时钟零差异",
        tz: "Singapore Standard Time",
        culture: "en-SG",
        browser_lang: "en-SG,en;q=0.9,zh-CN;q=0.8",
    },
    ProfileInfo {
        key: Profile::California,
        label: "美国加州",
        sub: "Pacific Time · UTC-7/8",
        tz: "Pacific Standard Time",
        culture: "en-US",
        browser_lang: "en-US,en;q=0.9",
    },
    ProfileInfo {
        key: Profile::Taipei,
        label: "台北",
        sub: "UTC+8 · 时钟零差异",
        tz: "Taipei Standard Time",
        culture: "zh-TW",
        browser_lang: "zh-TW,zh;q=0.9,en;q=0.8",
    },
    ProfileInfo {
        key: Profile::Tokyo,
        label: "东京",
        sub: "UTC+9 · 时钟 +1h",
        tz: "Tokyo Standard Time",
        culture: "ja-JP",
        browser_lang: "ja-JP,ja;q=0.9,en;q=0.8",
    },
    ProfileInfo {
        key: Profile::NewYork,
        label: "纽约",
        sub: "Eastern Time · UTC-4/5",
        tz: "Eastern Standard Time",
        culture: "en-US",
        browser_lang: "en-US,en;q=0.9",
    },
    ProfileInfo {
        key: Profile::Shanghai,
        label: "上海 (默认)",
        sub: "China Standard Time",
        tz: "China Standard Time",
        culture: "zh-CN",
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

/// 把 "China Standard Time" 缩成 "China"，日志更短
pub fn short_tz(tz: &str) -> String {
    tz.replace(" Standard Time", "").replace(" Time", "")
}

// ============================================================
// 时区偏移（时钟对照条用）
// ============================================================
/// 目标时区相对 UTC 的偏移（含 DST），按真实规则现算。
///
/// 为什么不用固定值：固定值在夏令时期间会差 1 小时，界面上的"北京 → 目标"
/// 时钟对照就成了错的，而"零差异"正是这个工具的核心卖点。
///
/// 已知局限：`north_america_dst` 只实现 2007 年起的美国规则，且北美之外
/// （南半球、欧洲）的 DST 完全没考虑。当前画像表只用到新加坡/台北/上海/东京/
/// 加州/纽约，恰好都在这个局限之外或之内可控。新增画像时务必同步检查这里。
pub fn target_utc_offset(key: Profile) -> chrono::FixedOffset {
    target_utc_offset_at(key, chrono::Utc::now())
}

/// 同上，但把"当前时刻"作为参数传入 —— 让单测能固定在冬令时/夏令时上验证，
/// 而不是只能依赖"今天恰好是夏天"。
pub fn target_utc_offset_at(
    key: Profile,
    now: chrono::DateTime<chrono::Utc>,
) -> chrono::FixedOffset {
    if matches!(key, Profile::California | Profile::NewYork) {
        let is_dst = north_america_dst(now);
        let hours = match key {
            Profile::California => {
                if is_dst {
                    -7
                } else {
                    -8
                }
            }
            Profile::NewYork => {
                if is_dst {
                    -4
                } else {
                    -5
                }
            }
            _ => unreachable!(),
        };
        return chrono::FixedOffset::east_opt(hours * 3600).unwrap();
    }

    let hours = match key {
        Profile::Singapore | Profile::Taipei | Profile::Shanghai => 8, // 无 DST
        Profile::Tokyo => 9,                                           // 无 DST
        // 上面已提前返回；放在这里是让新增 Profile 时编译器强制你处理
        Profile::California | Profile::NewYork => unreachable!(),
    };
    chrono::FixedOffset::east_opt(hours * 3600).unwrap()
}

/// 判断给定 UTC 时刻是否处于北美夏令时。
///
/// 规则（美国 2007 年起）：3 月第二个周日**当地 02:00** 开始，
/// 11 月第一个周日**当地 02:00** 结束。
///
/// 关键点：当地 02:00 换算成 UTC 并不是 02:00。开始时刻按 PST(UTC-8) 算，
/// 结束时刻按 PDT(UTC-7) 算。早先的实现拿 UTC 02:00 当切换点，会让切换日
/// 当天约 8 小时的窗口里偏移算错 1 小时 —— 单测覆盖了这两个边界。
pub fn north_america_dst(utc: chrono::DateTime<chrono::Utc>) -> bool {
    use chrono::{Datelike, TimeZone, Weekday};
    let y = utc.year();

    // 某年第 nth 个周日
    let nth_sunday = |month: u32, nth: u32| -> chrono::NaiveDate {
        let mut d = chrono::NaiveDate::from_ymd_opt(y, month, 1).unwrap();
        while d.weekday() != Weekday::Sun {
            d = d.succ_opt().unwrap();
        }
        d + chrono::Duration::days(7 * (nth - 1) as i64)
    };

    // 3 月第二个周日 当地 02:00 PST = 10:00 UTC
    let start = chrono::Utc.from_utc_datetime(&nth_sunday(3, 2).and_hms_opt(10, 0, 0).unwrap());
    // 11 月第一个周日 当地 02:00 PDT = 09:00 UTC
    let end = chrono::Utc.from_utc_datetime(&nth_sunday(11, 1).and_hms_opt(9, 0, 0).unwrap());

    utc >= start && utc < end
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
    /// 当前首选 UI 语言列表，**只用于展示**（本工具不写它）
    pub ui_langs: String,
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
        now: chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string(),
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
        ui_langs: read_ui_languages().unwrap_or_else(|| "未知".into()),
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
/// 拼出任意路径去写系统其他位置。
///
/// 权限只要 `KEY_SET_VALUE`：本工具只设值，不需要建子键、不需要 KEY_ALL_ACCESS
/// （后者含 WRITE_DAC / WRITE_OWNER）。这一点是实测修正 —— 早先用的
/// `create_subkey()` 在 winreg 里等价于 `create_subkey_with_flags(KEY_ALL_ACCESS)`
/// （见 winreg-0.52 src/reg_key.rs:240），与"最小权限"的注释不符：在管理员
/// 刻意收紧该键 ACL 的环境里，KEY_ALL_ACCESS 允许改写 DACL 绕过限制。
/// 这三个值所在的 `Control Panel\International` 在任何 Windows 上都存在，
/// 用 `open_subkey_with_flags` 打开即可；万一打不开会明确报错。
///
/// 注意白名单里**没有** `Control Panel\Desktop\PreferredUILanguages` 与
/// `International\User Profile\Languages`：首选 UI 语言本工具只读不写，
/// 理由见 ProfileInfo 上方注释与 README。
fn write_hkcu_value(path: &str, name: &str, val: &str) -> Result<(), String> {
    use winreg::enums::KEY_SET_VALUE;

    const SUPPORTED: &[(&str, &[&str])] = &[(
        r"Control Panel\International",
        &["Locale", "LocaleName", "sLanguage"],
    )];

    let allowed = SUPPORTED
        .iter()
        .any(|(p, names)| *p == path && names.contains(&name));
    if !allowed {
        return Err(format!("拒绝写入未支持的注册表位置: {}\\{}", path, name));
    }

    let k = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let sub = k.open_subkey_with_flags(path, KEY_SET_VALUE).map_err(|e| {
        format!(
            "打开注册表键失败 {}: {}（该系统可能不存在此键，预期它应当存在）",
            path, e
        )
    })?;
    // REG_SZ
    sub.set_value(name, &val)
        .map_err(|e| format!("写入注册表失败 {}\\{}: {}", path, name, e))
}

/// 读取当前首选 UI 语言列表（REG_MULTI_SZ），失败返回 None。
///
/// **只读**。本工具不写这个键：`PreferredUILanguages` 是「覆盖语言选择」列表，
/// 值必须指向已安装的语言包（`HKLM\SYSTEM\CurrentControlSet\Control\MUI\UILanguages`），
/// 否则 Windows 界面会回退成英文或出现半截乱码，而且改动必须注销才生效。
/// 这里读出来只用于界面上如实展示"你的 UI 语言是什么"，让用户知道它没被动过。
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
        // tzutil /s 自己会广播时区变更，这里用 "intl" 再补一次覆盖那些只监听
        // 区域/格式变更的进程（Windows 的时间/时区同属 intl 范畴）。
        broadcast_setting_change("intl");
        Ok(())
    } else {
        Err(format!(
            "tzutil 报错: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// 广播 WM_SETTINGCHANGE（SendMessageTimeout HWND_BROADCAST）
///
/// `area` 必须与变更内容对应，否则监听者不会重读对应的东西：
///   - `"intl"`        —— 区域 / 语言 / 日期时间格式变更
///   - `"Environment"` —— 环境变量变更（不是区域变更！）
fn broadcast_setting_change(area: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

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

    let wide: Vec<u16> = OsStr::new(area)
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

/// 切区域格式：改 Locale / LocaleName / sLanguage 三个键
/// (等价 Set-Culture，但不需要 PowerShell)
pub fn set_culture(culture: &str) -> Result<(), String> {
    // culture 形如 "en-US"; Locale 是 "00000409" 这种十六进制 LCID
    let lcid = culture_to_lcid(culture)?;

    write_hkcu_value(r"Control Panel\International", "Locale", &lcid)?;
    write_hkcu_value(r"Control Panel\International", "LocaleName", culture)?;
    // sLanguage 只在能确定映射时写；未知 culture 宁可不写也不写错值
    if let Some(code) = culture_lang_code(culture) {
        write_hkcu_value(r"Control Panel\International", "sLanguage", code)?;
    }
    // sCountry 故意不碰：Set-Culture 也不写它，写空串会让部分程序读到空值
    // 改完必须广播 "intl"：正在运行的资源管理器等不会自己重读区域格式，
    // 不广播的话用户会以为没生效（Set-Culture 本身也是靠这个通知）。
    broadcast_setting_change("intl");
    Ok(())
}

/// culture -> 十六进制 LCID。
/// 查不到就报错，**绝不猜一个默认值**：把 `Locale` 写成 00000409(en-US) 而
/// `LocaleName` 是别的值，会让这两个键自相矛盾，比直接失败更难排查。
///
/// 大小写不敏感：`valid_culture`（备份校验）允许 `en-us` 这种写法，
/// 这里必须同样放行，否则会出现"备份校验通过但还原时失败"的口径不一致。
fn culture_to_lcid(c: &str) -> Result<String, String> {
    // 只列画像用到的；新增画像时这里必须同步补上，否则 set_culture 会明确报错
    const MAP: &[(&str, &str)] = &[
        ("zh-CN", "00000804"),
        ("zh-TW", "00000404"),
        ("en-US", "00000409"),
        ("en-SG", "00004809"),
        ("ja-JP", "00000411"),
    ];
    MAP.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(c))
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| {
            format!(
                "不支持的区域格式 {:?}（没有对应的 LCID 映射，已放弃写入以免注册表自相矛盾）",
                c
            )
        })
}

/// 按完整 culture 映射 sLanguage（三字母语言标识）。
/// zh-TW 必须是 CHT（繁体），不能按前缀猜成 CHS —— 那会和 LocaleName=zh-TW 自相矛盾。
/// 同样大小写不敏感，与 `culture_to_lcid` / `valid_culture` 保持一致。
fn culture_lang_code(c: &str) -> Option<&'static str> {
    if c.eq_ignore_ascii_case("zh-CN") {
        Some("CHS")
    } else if c.eq_ignore_ascii_case("zh-TW") {
        Some("CHT")
    } else if c.eq_ignore_ascii_case("en-US") || c.eq_ignore_ascii_case("en-SG") {
        Some("ENU")
    } else if c.eq_ignore_ascii_case("ja-JP") {
        Some("JPN")
    } else {
        None
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

// ============================================================
// 浏览器进程探测（写 Preferences 前的守卫）
// ============================================================
/// 已知的 Chromium 系浏览器主程序，与 browser.rs 的 browser_data_dirs 对应。
pub const BROWSER_EXES: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "chromium.exe",
    "vivaldi.exe",
];

/// 返回当前**正在运行**的浏览器主程序名（大写原样，便于直接展示）。
///
/// 为什么不能只看主进程：Chrome 的后台驻留和"继续运行后台应用"会让
/// 主进程退出后仍留下子进程持有 Preferences，只查 `chrome.exe` 会漏判。
/// 所以这里遍历进程快照，凡是 `chrome.exe` 或 `chrome.exe` 的子进程都算。
///
/// 用 tasklist 的 CSV 模式一次拿全表：原来的 process_running 每个名字都要
/// spawn 一次 tasklist.exe，5 个浏览器就是 5 次进程创建。
pub fn running_browsers() -> Vec<String> {
    let out = match Command::new(sys_tool("tasklist.exe"))
        .args(["/FO", "CSV", "/NH"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        // 查不到进程 ≠ 浏览器没运行。保守返回"全部在运行"，
        // 让写入口拒绝执行，避免在无法确认的情况下贸然改用户数据。
        _ => return BROWSER_EXES.iter().map(|s| s.to_uppercase()).collect(),
    };

    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let mut found: Vec<String> = Vec::new();
    for exe in BROWSER_EXES {
        if text.contains(exe) {
            found.push(exe.to_uppercase());
        }
    }
    found
}

// ============================================================
// 单元测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // ---------- 画像表一致性 ----------

    #[test]
    fn 每个画像都有对应的_info_条目() {
        // Profile 是有限枚举；漏一个就会在 info() 里 unreachable 崩溃
        for key in [
            Profile::Singapore,
            Profile::California,
            Profile::Taipei,
            Profile::Tokyo,
            Profile::NewYork,
            Profile::Shanghai,
        ] {
            let i = info(key);
            assert_eq!(i.key, key, "info() 返回了错误的条目");
            assert!(!i.label.is_empty());
            assert!(
                i.tz.ends_with(" Standard Time"),
                "非 Windows 时区 ID: {}",
                i.tz
            );
        }
    }

    #[test]
    fn 每个画像的区域格式都能映射到_lcid() {
        // 画像表加了新 culture 却忘了补 LCID 映射时，这里会失败。
        // 宁可测试失败，也不要运行时把 Locale 写成错的默认值。
        for i in PROFILES {
            assert!(
                culture_to_lcid(i.culture).is_ok(),
                "画像 {} 的 culture {:?} 没有 LCID 映射",
                i.label,
                i.culture
            );
        }
    }

    // ---------- LCID 映射 ----------

    #[test]
    fn lcid_映射正确() {
        assert_eq!(culture_to_lcid("zh-CN").unwrap(), "00000804");
        assert_eq!(culture_to_lcid("zh-TW").unwrap(), "00000404");
        assert_eq!(culture_to_lcid("en-US").unwrap(), "00000409");
        assert_eq!(culture_to_lcid("en-SG").unwrap(), "00004809");
        assert_eq!(culture_to_lcid("ja-JP").unwrap(), "00000411");
    }

    #[test]
    fn 未知_culture_报错而不是猜默认值() {
        // 这是回归防护：早先的实现会静默返回 en-US 的 LCID，
        // 让 Locale 和 LocaleName 两个键自相矛盾。
        for bad in ["de-DE", "", "en", "zh-Hans-CN", "xx-YY"] {
            assert!(
                culture_to_lcid(bad).is_err(),
                "{:?} 不该被接受（会写出错误的 Locale）",
                bad
            );
        }
    }

    #[test]
    fn culture_映射大小写不敏感() {
        // 回归防护：valid_culture（备份校验）放行 "en-us"，若这两个映射函数
        // 大小写敏感，就会出现"备份校验通过、还原时报不支持"的口径不一致。
        for (input, lcid) in [
            ("en-us", "00000409"),
            ("EN-US", "00000409"),
            ("En-Us", "00000409"),
            ("ZH-cn", "00000804"),
            ("zh-tw", "00000404"),
            ("ja-jp", "00000411"),
            ("en-sg", "00004809"),
        ] {
            assert_eq!(
                culture_to_lcid(input).unwrap(),
                lcid,
                "{:?} 应映射到 {}",
                input,
                lcid
            );
        }
        // sLanguage 同样要不敏感，且简繁不能混淆
        assert_eq!(culture_lang_code("zh-cn"), Some("CHS"));
        assert_eq!(culture_lang_code("ZH-TW"), Some("CHT"));
        assert_eq!(culture_lang_code("en-US"), Some("ENU"));
        assert_eq!(culture_lang_code("JA-jp"), Some("JPN"));
        assert_eq!(culture_lang_code("de-de"), None);
    }

    #[test]
    fn s_language_映射区分简繁() {
        // zh-TW 必须是 CHT：按前缀猜成 CHS 会和 LocaleName=zh-TW 矛盾
        assert_eq!(culture_lang_code("zh-CN"), Some("CHS"));
        assert_eq!(culture_lang_code("zh-TW"), Some("CHT"));
        assert_eq!(culture_lang_code("en-US"), Some("ENU"));
        assert_eq!(culture_lang_code("en-SG"), Some("ENU"));
        assert_eq!(culture_lang_code("ja-JP"), Some("JPN"));
        assert_eq!(culture_lang_code("de-DE"), None);
    }

    // ---------- 风险评分 ----------

    fn fp(tz: &str, culture: &str) -> Fingerprint {
        Fingerprint {
            tz_id: tz.into(),
            is_china_tz: tz == "China Standard Time",
            now: String::new(),
            culture: culture.into(),
            sys_locale: String::new(),
            geo_id: String::new(),
            browser_lang: String::new(),
            ui_langs: String::new(),
            chrome_running: false,
            edge_running: false,
        }
    }

    #[test]
    fn 风险分_中国大陆全中为高危() {
        let (s, l) = risk_score(&fp("China Standard Time", "zh-CN"));
        assert_eq!((s, l), (65, "高危"));
    }

    #[test]
    fn 风险分_切到新加坡后归零() {
        let (s, l) = risk_score(&fp("Singapore Standard Time", "en-SG"));
        assert_eq!((s, l), (0, "安全"));
    }

    #[test]
    fn 风险分_只中一项时分级正确() {
        // 只时区是中国
        assert_eq!(
            risk_score(&fp("China Standard Time", "en-US")),
            (50, "高危")
        );
        // 只区域格式是 zh-CN
        assert_eq!(
            risk_score(&fp("Singapore Standard Time", "zh-CN")),
            (15, "低危")
        );
    }

    #[test]
    fn 风险分_分数不超过自己的分母() {
        // 界面写死了 "/65"；分数一旦超过分母就成了 bug
        let (s, _) = risk_score(&fp("China Standard Time", "zh-CN"));
        assert!(s <= 65, "得分 {} 超过了界面标注的分母 65", s);
    }

    // ---------- 北美夏令时边界 ----------

    /// 2026-03-08 是 3 月第二个周日；2026-11-01 是 11 月第一个周日
    #[test]
    fn dst_春季切换边界按当地02点() {
        let before = chrono::Utc.with_ymd_and_hms(2026, 3, 8, 9, 59, 0).unwrap();
        let after = chrono::Utc.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap();
        assert!(!north_america_dst(before), "10:00 UTC 之前还是 PST");
        assert!(north_america_dst(after), "10:00 UTC 起进入 PDT");
    }

    #[test]
    fn dst_秋季切换边界按当地02点() {
        let before = chrono::Utc.with_ymd_and_hms(2026, 11, 1, 8, 59, 0).unwrap();
        let after = chrono::Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();
        assert!(north_america_dst(before), "09:00 UTC 之前还是 PDT");
        assert!(!north_america_dst(after), "09:00 UTC 起回到 PST");
    }

    #[test]
    fn dst_冬夏分明() {
        let winter = chrono::Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        let summer = chrono::Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        assert!(!north_america_dst(winter));
        assert!(north_america_dst(summer));
    }

    #[test]
    fn 时钟偏移_加州与纽约冬夏各差一小时() {
        // 固定时刻验证，不依赖"今天是不是夏天"
        let winter = chrono::Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        let summer = chrono::Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();

        let h = |p, t| target_utc_offset_at(p, t).local_minus_utc() / 3600;

        // 冬天：PST = UTC-8，EST = UTC-5
        assert_eq!(h(Profile::California, winter), -8);
        assert_eq!(h(Profile::NewYork, winter), -5);
        // 夏天：PDT = UTC-7，EDT = UTC-4
        assert_eq!(h(Profile::California, summer), -7);
        assert_eq!(h(Profile::NewYork, summer), -4);

        // 相对北京（UTC+8）差值：加州冬 -16h / 夏 -15h，纽约冬 -13h / 夏 -12h
        assert_eq!(h(Profile::California, winter) - 8, -16);
        assert_eq!(h(Profile::California, summer) - 8, -15);
        assert_eq!(h(Profile::NewYork, winter) - 8, -13);
        assert_eq!(h(Profile::NewYork, summer) - 8, -12);
    }

    #[test]
    fn 时钟偏移_东八区画像恒定零差异() {
        for key in [Profile::Singapore, Profile::Taipei, Profile::Shanghai] {
            assert_eq!(
                target_utc_offset(key).local_minus_utc(),
                8 * 3600,
                "{:?} 应该是 UTC+8（相对北京零差异）",
                key
            );
        }
        assert_eq!(
            target_utc_offset(Profile::Tokyo).local_minus_utc(),
            9 * 3600
        );
    }

    // ---------- 其他纯函数 ----------

    #[test]
    fn 时区名缩写去掉后缀() {
        assert_eq!(short_tz("China Standard Time"), "China");
        assert_eq!(short_tz("Singapore Standard Time"), "Singapore");
        assert_eq!(short_tz("UTC"), "UTC");
    }
}
