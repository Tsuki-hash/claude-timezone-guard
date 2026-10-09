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
    /// ANTHROPIC_BASE_URL 的生效值（None = 未设置）。
    /// 原文点名的第二条识别路径；本工具**不改**它，只如实展示并计分。
    pub base_url: Option<String>,
    /// base_url 是否指向非官方地址（即经过第三方中转）
    pub proxy_like_base_url: bool,
    /// base_url 的来源（文件路径或"环境变量"），仅用于告诉用户改哪里
    pub base_url_hint: String,
    /// Windows 时间服务配置的 NTP 服务器（None = 未配置）
    pub ntp_server: Option<String>,
    /// NTP 是否看起来是国内的（= 校时会泄露真实时区）
    pub ntp_leaks: bool,
}

pub fn read_fingerprint() -> Fingerprint {
    let tz_id = get_timezone();
    let culture = get_culture();
    let browser_lang = crate::browser::read_chrome_lang().unwrap_or_else(|| "未检测到".into());
    let base_url = read_base_url();
    let (ntp_leaks, _) = ntp_risk(get_ntp_server().as_deref());

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
        proxy_like_base_url: is_proxy_like_base_url(base_url.as_deref()),
        base_url,
        base_url_hint: base_url_hint(),
        ntp_server: get_ntp_server(),
        ntp_leaks,
    }
}

/// 指纹风险评分。
///
/// 分母 100，覆盖四条本工具**能观察到**的本地特征路径：
///   时区 35 + 代理中转地址(ANTHROPIC_BASE_URL) 40 + 区域格式 15 + 直连 NTP 10
///
/// 为什么把 BASE_URL 给到最高的 40：原文点名的两条与出口 IP 无关的路径里，
/// 它是最直接的一条 —— 指向中转站等于自己报出用了什么服务，而且它比时区
/// **更容易修**（改一个环境变量即可）。
///
/// ACP(系统代码页) 与 Nation(区域位置) 仍不计入：前者要管理员+重启才能改，
/// 计进去会让分数永远降不到 0，与界面上的结论自相矛盾。
///
/// 注意：**分数为 0 不等于"在 Claude 眼里干净"** —— 出口 IP、DNS、WebRTC
/// 都不在本地可观测范围内。界面文案必须如实反映这一点。
pub fn risk_score(f: &Fingerprint) -> (u32, &'static str) {
    let s: u32 = f
        .risk_items()
        .iter()
        .filter(|i| i.tripped)
        .map(|i| i.weight)
        .sum();

    // 阈值按新分母重新标定：单项最高 40，最严重组合（时区+中转）= 75
    let level = if s >= 60 {
        "高危"
    } else if s >= 25 {
        "中危"
    } else if s > 0 {
        "低危"
    } else {
        "安全"
    };
    (s, level)
}

/// 评分上限（分母）。界面/CLI 都应当引用它而不是写死数字，
/// 否则将来调整权重时会出现"分数超过分母"或文案对不上。
pub const RISK_MAX: u32 = 100;

/// 一条可观察的风险项。`tripped` 为真时计入分数。
#[derive(Clone, Copy, Debug)]
pub struct RiskItem {
    pub weight: u32,
    pub tripped: bool,
}

impl Fingerprint {
    /// 当前命中的风险项（顺序与权重固定，便于界面稳定展示与单测断言）
    pub fn risk_items(&self) -> [RiskItem; 4] {
        [
            RiskItem {
                weight: 35,
                tripped: self.is_china_tz,
            },
            RiskItem {
                weight: 40,
                tripped: self.proxy_like_base_url,
            },
            RiskItem {
                weight: 15,
                tripped: self.culture == "zh-CN",
            },
            RiskItem {
                weight: 10,
                tripped: self.ntp_leaks,
            },
        ]
    }
}

// ============================================================
// 风险项 2：ANTHROPIC_BASE_URL（原文第二条识别路径）
// ============================================================
/// 官方地址。只有指向这里（或不设置）才不算暴露。
pub const OFFICIAL_BASE_URL: &str = "api.anthropic.com";

/// 判定 base_url 是否"像中转站"。
///
/// 语义与命名都很保守：我们**无法**内置那份混淆过的域名名单（原文说它
/// base64+XOR、约 147 项，而且会变），所以不做"是否在黑名单上"的判断。
/// 这里只回答一个能可靠回答的问题：**它是不是官方地址**。
/// 凡是非空、且主机不是 `api.anthropic.com` 的，一律视为"经过第三方"，
/// 由用户自己去确认那个中转站是否在名单里。
///
/// 空值 / 只有空白 → 视为未设置（按官方直连处理）。
pub fn is_proxy_like_base_url(raw: Option<&str>) -> bool {
    let Some(v) = raw else { return false };
    let v = v.trim();
    if v.is_empty() {
        return false;
    }
    match host_of(v) {
        Some(h) => !h.eq_ignore_ascii_case(OFFICIAL_BASE_URL),
        // 有值但解析不出主机名：宁可提示用户去看一眼，也不当作安全
        None => true,
    }
}

/// 从 URL / host[:port] / host/path 里取出主机名（小写，去端口与用户信息）
fn host_of(v: &str) -> Option<String> {
    let after_scheme = match v.find("://") {
        Some(i) => &v[i + 3..],
        None => v,
    };
    // 去掉路径 / 查询 / 片段
    let hostport = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // 去掉 user:pass@
    let hostport = match hostport.rfind('@') {
        Some(i) => &hostport[i + 1..],
        None => hostport,
    };
    let host = hostport.split(':').next().unwrap_or_default().trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// 读取 ANTHROPIC_BASE_URL 的生效值。
///
/// 两个来源（只查环境变量会漏报）：
///   1. 环境变量（进程级，最直接）
///   2. `~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL` —— Claude Code
///      支持在设置文件里注入环境变量，那种配置不会出现在本进程的环境块里。
///      实测这是最常见的配置方式（本机就是这一种）。
pub fn read_base_url() -> Option<String> {
    if let Ok(v) = std::env::var("ANTHROPIC_BASE_URL") {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    read_base_url_from_settings()
}

/// 找到设置了 ANTHROPIC_BASE_URL 的那个文件（仅用于告诉用户"改哪里"）
pub fn base_url_settings_path() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    for name in [r".claude\settings.json", ".claude.json"] {
        let p = PathBuf::from(&home).join(name);
        let Ok(txt) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else {
            continue;
        };
        let has = v
            .get("env")
            .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
            .and_then(|s| s.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if has {
            return Some(p);
        }
    }
    None
}

fn read_base_url_from_settings() -> Option<String> {
    let p = base_url_settings_path()?;
    let txt = std::fs::read_to_string(&p).ok()?;
    let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
    let s = v
        .get("env")?
        .get("ANTHROPIC_BASE_URL")?
        .as_str()?
        .trim()
        .to_string();
    if s.is_empty() {
        None
    } else {
        // 过一遍打码函数：base_url 理论上不含密钥，但它是从**同时存放密钥的文件**
        // 里读出来的，万一用户把 token 拼进了 URL（`https://token@relay/...`），
        // 这里就是唯一能拦住它被打印到屏幕上的地方。
        Some(redact_url_userinfo(&s))
    }
}

/// 去掉 URL 里的 `user:pass@` 部分（可以含密钥），其余原样保留。
/// 与 `redact_secret` 的分工：这个用于**仍要展示完整主机名**的场景。
pub fn redact_url_userinfo(u: &str) -> String {
    match (u.find("://"), u.find('@')) {
        (Some(scheme_end), Some(at)) if at > scheme_end => {
            let (head, tail) = u.split_at(scheme_end + 3);
            format!("{}***@{}", head, &tail[at - (scheme_end + 3) + 1..])
        }
        _ => u.to_string(),
    }
}

/// 把敏感值打码后再展示。
///
/// 为什么需要：读 `settings.json` 的 `env` 段时，同一段里通常还放着
/// `ANTHROPIC_AUTH_TOKEN` 之类的密钥。本工具只取 base_url，但**只要有任何一处
/// 把整段 env 打印出来，密钥就会随日志落到屏幕、剪贴板或截图里**。
/// 这里统一提供打码函数，凡展示来自该文件的字符串一律先过它。
pub fn redact_secret(v: &str) -> String {
    let n = v.chars().count();
    if n <= 8 {
        return "*".repeat(n);
    }
    let head: String = v.chars().take(4).collect();
    format!("{}…(已打码，共 {} 字符)", head, n)
}

/// 告诉用户"这个值是从哪读到的"，方便他自己去改
pub fn base_url_hint() -> String {
    match base_url_settings_path() {
        Some(p) => format!("来源: {}", p.display()),
        None => "来源: 环境变量 ANTHROPIC_BASE_URL".into(),
    }
}

// ============================================================
// 风险项 4：NTP 直连（原文方案一点名的时区泄露口）
// ============================================================
/// Windows 时间服务的 NTP 服务器主机名。
pub fn get_ntp_server() -> Option<String> {
    let raw = read_reg_str(
        winreg::enums::HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Services\W32Time\Parameters",
        "NtpServer",
    )?;
    // 形如 `time.windows.com,0x9`：逗号后是标志位，主机名在前
    let host = raw.split(',').next().unwrap_or_default().trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// NTP 服务器是否"像国内的"。
///
/// 为什么算风险：系统校时会向这台服务器暴露你的真实时区（原文方案一专门
/// 讲了这条），它若直连国内地址，代理出口再干净也白搭。
///
/// 判定只依据域名，**不解析 DNS、不发任何网络请求** —— 本工具全程离线，
/// 这一点必须保持。
pub fn ntp_looks_domestic(server: &str) -> bool {
    let s = server.to_ascii_lowercase();
    const CN_SUFFIXES: &[&str] = &[
        ".cn",
        ".com.cn",
        ".net.cn",
        ".org.cn",
        "aliyun.com",
        "tencent.com",
        "cn.pool.ntp.org",
        "ntp.ntsc.ac.cn",
        "time.edu.cn",
    ];
    CN_SUFFIXES.iter().any(|suf| {
        // 后缀匹配必须落在标签边界上，避免 "evilcn.com" 命中 ".cn"
        s.ends_with(suf) || s.contains(&format!(".{}", suf.trim_start_matches('.')))
    })
}

/// 判定当前 NTP 配置是否构成风险：没配 NTP（None）不算；配了国内地址才算。
pub fn ntp_risk(server: Option<&str>) -> (bool, String) {
    match server {
        None => (false, "未配置".into()),
        Some(s) => (ntp_looks_domestic(s), s.to_string()),
    }
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
///
/// 目录来源优先用 `GetSystemDirectoryW`（API，不信环境变量），
/// 只有 API 失败时才退回 `%SystemRoot%`。
fn sys_tool(name: &str) -> PathBuf {
    if let Some(dir) = system_directory() {
        return dir.join(name);
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}

/// 系统目录（通常是 `C:\Windows\System32`）—— 通过 API 获取，不读环境变量。
///
/// 为什么要这样：`SystemRoot` 是进程环境的一部分，任何启动者都能替换它
/// （CreateProcess 的 lpEnvironment），从而改变本工具**执行哪个 tzutil / tasklist**。
/// 当前没有提权收益，但这是一条不该留着的口子。
/// 返回 None 时调用方退回环境变量，所以不会因为 API 异常而完全不可用。
fn system_directory() -> Option<PathBuf> {
    #[link(name = "kernel32", kind = "dylib")]
    extern "system" {
        fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
    }
    const MAX_PATH: usize = 260;
    let mut buf = [0u16; MAX_PATH];
    // 返回值是写入的字符数（不含结尾 NUL）；0 或超出缓冲都算失败
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), MAX_PATH as u32) };
    if n == 0 || n as usize >= MAX_PATH {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..n as usize]);
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
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
            base_url: None,
            proxy_like_base_url: false,
            base_url_hint: String::new(),
            ntp_server: None,
            ntp_leaks: false,
        }
    }

    #[test]
    fn 风险分_中国大陆全中为高危() {
        // 时区 35 + 区域格式 15 = 50（没有中转、没有国内 NTP）
        let (s, l) = risk_score(&fp("China Standard Time", "zh-CN"));
        assert_eq!((s, l), (50, "中危"));
    }

    #[test]
    fn 风险分_切到新加坡后归零() {
        let (s, l) = risk_score(&fp("Singapore Standard Time", "en-SG"));
        assert_eq!((s, l), (0, "安全"));
    }

    #[test]
    fn 风险分_只中一项时分级正确() {
        // 只时区是中国：35
        assert_eq!(
            risk_score(&fp("China Standard Time", "en-US")),
            (35, "中危")
        );
        // 只区域格式是 zh-CN：15
        assert_eq!(
            risk_score(&fp("Singapore Standard Time", "zh-CN")),
            (15, "低危")
        );
        // 只 NTP 是国内：10
        let mut ntp_only = fp("Singapore Standard Time", "en-SG");
        ntp_only.ntp_leaks = true;
        assert_eq!(risk_score(&ntp_only), (10, "低危"));
    }

    #[test]
    fn 风险分_分数不超过自己的分母() {
        // 界面/CLI 都引用 RISK_MAX；分数一旦超过分母就是 bug
        let (s, _) = risk_score(&fp("China Standard Time", "zh-CN"));
        assert!(s <= RISK_MAX, "得分 {} 超过了分母 {}", s, RISK_MAX);
        // 四项全中也不能超
        let mut worst = fp("China Standard Time", "zh-CN");
        worst.proxy_like_base_url = true;
        worst.ntp_leaks = true;
        let (s2, l2) = risk_score(&worst);
        assert!(s2 <= RISK_MAX, "四项全中得分 {} 超过分母", s2);
        assert_eq!(l2, "高危");
    }

    #[test]
    fn 风险分_各权重加起来正好等于分母() {
        // 保证"四项全中 = 满分"，否则分母就没有意义
        let mut worst = fp("China Standard Time", "zh-CN");
        worst.proxy_like_base_url = true;
        worst.ntp_leaks = true;
        let (s, _) = risk_score(&worst);
        assert_eq!(s, RISK_MAX, "四项全中应等于分母");
    }

    // ---------- ANTHROPIC_BASE_URL ----------

    #[test]
    fn base_url_未设置或空值不算风险() {
        assert!(!is_proxy_like_base_url(None));
        assert!(!is_proxy_like_base_url(Some("")));
        assert!(!is_proxy_like_base_url(Some("   ")));
    }

    #[test]
    fn base_url_官方地址的各种写法都不算风险() {
        for ok in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "https://api.anthropic.com/v1/messages",
            "http://api.anthropic.com:443",
            "https://API.ANTHROPIC.COM",
            "api.anthropic.com",
        ] {
            assert!(
                !is_proxy_like_base_url(Some(ok)),
                "官方地址被误判为风险: {:?}",
                ok
            );
        }
    }

    #[test]
    fn base_url_中转站会被识别() {
        for bad in [
            "https://relay.example.com",
            "https://api.openai-proxy.cn/v1",
            "https://my-relay.workers.dev",
            "http://192.168.1.10:8080",
            "https://user:pass@relay.example.com/v1",
            // 不是官方的相似域名不能放过
            "https://api.anthropic.com.evil.com",
            "https://evil-api.anthropic.com.attacker.net",
        ] {
            assert!(
                is_proxy_like_base_url(Some(bad)),
                "中转地址未被识别: {:?}",
                bad
            );
        }
    }

    #[test]
    fn base_url_相似域名不能被当成官方() {
        // 逐字符比较主机名，前缀/后缀相似都不算官方
        assert!(is_proxy_like_base_url(Some("https://notapi.anthropic.com")));
        assert!(is_proxy_like_base_url(Some("https://api.anthropic.com.cn")));
        assert!(is_proxy_like_base_url(Some("https://xapi.anthropic.com")));
    }

    // ---------- 密钥打码 ----------

    #[test]
    fn 打码后不再包含原文() {
        let secret = "sk-7EXAMPLE-REDACTED-000000";
        let masked = redact_secret(secret);
        assert!(!masked.contains(secret), "打码后仍含完整密钥");
        assert!(masked.contains("sk-7"), "应保留少量可识别前缀");
        assert!(masked.contains(&secret.chars().count().to_string()));
    }

    #[test]
    fn 短字符串整体打码() {
        assert_eq!(redact_secret("abc"), "***");
        assert_eq!(redact_secret(""), "");
        assert_eq!(redact_secret("12345678"), "********");
    }

    #[test]
    fn url_里的用户信息会被抹掉() {
        // 用户可能把 token 拼进 URL；这一处是唯一拦住它被打印的地方
        let u = "https://sk-secret-token@relay.example.com/v1";
        let out = redact_url_userinfo(u);
        assert!(!out.contains("sk-secret-token"), "token 未被抹掉: {}", out);
        assert!(out.contains("relay.example.com"), "主机名应保留: {}", out);
        assert_eq!(out, "https://***@relay.example.com/v1", "抹掉后应保持可读");

        // 没有用户信息的 URL 原样返回
        for ok in [
            "https://api.anthropic.com/v1",
            "https://relay.example.com",
            "not-a-url",
        ] {
            assert_eq!(redact_url_userinfo(ok), ok, "不应改动 {:?}", ok);
        }
    }

    // ---------- NTP ----------

    #[test]
    fn ntp_国内服务器会被标记() {
        for bad in [
            "ntp.aliyun.com",
            "ntp1.aliyun.com",
            "time.tencent.com",
            "cn.pool.ntp.org",
            "ntp.ntsc.ac.cn",
            "210.72.145.44",
            "s1b.time.edu.cn",
        ] {
            // 注意 210.72.145.44 是纯 IP，无法按域名判断 —— 见下一条测试说明
            let got = ntp_looks_domestic(bad);
            if bad == "210.72.145.44" {
                assert!(!got, "纯 IP 不做判断（不能靠猜），实际: {}", got);
            } else {
                assert!(got, "国内 NTP 未被识别: {:?}", bad);
            }
        }
    }

    #[test]
    fn ntp_国外服务器不会被误判() {
        for ok in [
            "time.windows.com",
            "pool.ntp.org",
            "time.google.com",
            "time.cloudflare.com",
            "ntp.ubuntu.com",
            // 关键回归：'evilcn.com' 不能因为包含 'cn' 被当成 .cn
            "evilcn.com",
            "cname.example.com",
        ] {
            assert!(!ntp_looks_domestic(ok), "国外 NTP 被误判: {:?}", ok);
        }
    }

    #[test]
    fn ntp_未配置不算风险() {
        assert_eq!(ntp_risk(None), (false, "未配置".to_string()));
        assert_eq!(
            ntp_risk(Some("time.windows.com")),
            (false, "time.windows.com".to_string())
        );
        assert!(ntp_risk(Some("ntp.aliyun.com")).0);
    }

    #[test]
    fn 风险分_只中_base_url_时是中危() {
        let mut f = fp("Singapore Standard Time", "en-SG");
        f.proxy_like_base_url = true;
        // 单靠 base_url（40）就是最高权重的单项，应落入中危区间
        let (s, l) = risk_score(&f);
        assert_eq!(s, 40);
        assert_eq!(l, "中危");
    }

    #[test]
    fn 风险分_时区加中转即高危() {
        // 最典型的"挂了代理但没改时区"画像：时区 35 + 中转 40 = 75
        let mut f = fp("China Standard Time", "zh-CN");
        f.proxy_like_base_url = true;
        let (s, l) = risk_score(&f);
        assert_eq!(s, 90); // 35 + 40 + 15
        assert_eq!(l, "高危");
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
