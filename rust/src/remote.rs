//! 出口侧风险估算：调用 FuckClaude 的公开 `/api/check` 端点（MIT）。
//!
//! 本工具只能观察本机特征；出口 IP / 请求头侧的风险由该服务端估算补齐。
//! 口径差异必须在 UI 上如实标注：它基于 IP 归属地与请求头，
//! 与本机读数（操作系统层面）不是同一回事。
//!
//! 解析全部防御性：对方接口是第三方公益部署，升级/改字段时给出可读错误，
//! 绝不 panic、绝不阻塞主流程。

use serde_json::Value;

pub const REMOTE_CHECK_URL: &str = "https://fuck-claude.vercel.app/api/check?format=json";

#[derive(Clone, Debug)]
pub struct RemoteEstimate {
    pub score: u32,
    pub band: String,
    pub verdict: String,
    pub message: String,
    pub geo_country: Option<String>,
    pub geo_tz: Option<String>,
    pub measured_weight: u32,
    pub total_weight: u32,
}

impl RemoteEstimate {
    /// 读数行的一句话摘要
    pub fn headline(&self) -> String {
        format!("{}/100 {}（出口侧估算）", self.score, self.verdict)
    }

    /// 地理归属摘要
    pub fn geo_summary(&self) -> String {
        match (&self.geo_country, &self.geo_tz) {
            (Some(c), Some(t)) => format!("{c} · {t}"),
            (Some(c), None) => c.clone(),
            (None, Some(t)) => t.clone(),
            _ => "未知".into(),
        }
    }
}

/// 请求出口侧估算。超时 5s；非 2xx 视为失败（ureq 2 的默认语义）。
pub fn fetch_remote_estimate() -> Result<RemoteEstimate, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let resp = agent
        .get(REMOTE_CHECK_URL)
        .set("Accept-Language", "zh")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("HTTP {code}"),
            other => format!("{other}"),
        })?;
    let body = resp
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    parse_remote_json(&body)
}

/// 防御性解析：字段缺失/类型变化都给出可读错误，绝不 panic。
pub fn parse_remote_json(body: &str) -> Result<RemoteEstimate, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("响应不是合法 JSON: {e}"))?;
    let score = v
        .get("score")
        .and_then(|x| x.as_f64())
        .ok_or("响应缺少 score 字段")?;
    let get_str = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Ok(RemoteEstimate {
        score: score.round() as u32,
        band: get_str("band"),
        verdict: get_str("verdict"),
        message: get_str("message"),
        geo_country: v
            .pointer("/geo/country")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        geo_tz: v
            .pointer("/geo/timezone")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        measured_weight: v
            .pointer("/coverage/measuredWeight")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0) as u32,
        total_weight: v
            .pointer("/coverage/totalWeight")
            .and_then(|x| x.as_f64())
            .unwrap_or(100.0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析_完整响应() {
        let body = r#"{
            "app": "Fuck Claude", "estimate": true, "lang": "zh",
            "score": 45, "band": "medium", "verdict": "中危", "message": "…",
            "coverage": { "measuredWeight": 62, "totalWeight": 100 },
            "geo": { "country": "CN", "timezone": "Asia/Shanghai" },
            "signals": [], "note": "…", "docs": "…"
        }"#;
        let est = parse_remote_json(body).unwrap();
        assert_eq!(est.score, 45);
        assert_eq!(est.band, "medium");
        assert_eq!(est.verdict, "中危");
        assert_eq!(est.geo_country.as_deref(), Some("CN"));
        assert_eq!(est.geo_tz.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(est.measured_weight, 62);
        assert_eq!(est.total_weight, 100);
        assert_eq!(est.headline(), "45/100 中危（出口侧估算）");
        assert_eq!(est.geo_summary(), "CN · Asia/Shanghai");
    }

    #[test]
    fn 解析_geo_null_不崩() {
        let body = r#"{ "score": 12.4, "band": "low", "verdict": "低危", "message": "",
            "coverage": {}, "geo": { "country": null, "timezone": null } }"#;
        let est = parse_remote_json(body).unwrap();
        assert_eq!(est.score, 12); // 12.4 四舍五入
        assert_eq!(est.geo_summary(), "未知");
        assert_eq!(est.total_weight, 100); // 缺省回退
    }

    #[test]
    fn 解析_缺_score_与坏_json_报可读错误() {
        assert!(parse_remote_json(r#"{ "band": "low" }"#)
            .unwrap_err()
            .contains("score"));
        assert!(parse_remote_json("not json").unwrap_err().contains("JSON"));
    }
}
