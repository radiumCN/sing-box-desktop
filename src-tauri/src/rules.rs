use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::config;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Proxy,
    Direct,
    Block,
    Dns,
}

impl std::fmt::Display for RuleAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proxy => write!(f, "proxy"),
            Self::Direct => write!(f, "direct"),
            Self::Block => write!(f, "block"),
            Self::Dns => write!(f, "dns-out"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub action: RuleAction,
    /// Match by domain (exact)
    pub domain: Vec<String>,
    /// Match by domain suffix
    pub domain_suffix: Vec<String>,
    /// Match by domain keyword
    pub domain_keyword: Vec<String>,
    /// Match by GeoSite tag (e.g. "cn", "google")
    pub geosite: Vec<String>,
    /// Match by GeoIP country code (e.g. "cn", "private")
    pub geoip: Vec<String>,
    /// Match by CIDR block
    pub ip_cidr: Vec<String>,
    /// Match by port (single or range like "80,443,8080-8090")
    pub port: Vec<String>,
    /// Match by protocol (tcp/udp)
    pub network: Option<String>,
    /// Match by process name
    pub process_name: Vec<String>,
}

impl RouteRule {
    pub fn new_empty(name: &str, action: RuleAction) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            enabled: true,
            action,
            domain: vec![],
            domain_suffix: vec![],
            domain_keyword: vec![],
            geosite: vec![],
            geoip: vec![],
            ip_cidr: vec![],
            port: vec![],
            network: None,
            process_name: vec![],
        }
    }
}

/// Built-in preset rules
pub fn preset_rules() -> Vec<RouteRule> {
    vec![
        // ── 基础 ────────────────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("DNS 流量", RuleAction::Dns);
            r.port = vec!["53".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("拦截广告", RuleAction::Block);
            r.geosite = vec!["category-ads-all".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("私有地址直连", RuleAction::Direct);
            r.geoip = vec!["private".into()];
            r.ip_cidr = vec![
                "127.0.0.0/8".into(),
                "10.0.0.0/8".into(),
                "172.16.0.0/12".into(),
                "192.168.0.0/16".into(),
                "169.254.0.0/16".into(),
                "::1/128".into(),
                "fc00::/7".into(),
            ];
            r
        },
        // ── 国际服务走代理 ──────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("Telegram", RuleAction::Proxy);
            r.geosite = vec!["telegram".into()];
            r.geoip   = vec!["telegram".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("AI 服务", RuleAction::Proxy);
            r.domain_suffix = vec![
                "openai.com".into(),
                "oaistatic.com".into(),
                "oaiusercontent.com".into(),
                "chatgpt.com".into(),
                "anthropic.com".into(),
                "claude.ai".into(),
                "perplexity.ai".into(),
                "gemini.google.com".into(),
            ];
            r.domain_keyword = vec!["openai".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("YouTube", RuleAction::Proxy);
            r.geosite = vec!["youtube".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Google", RuleAction::Proxy);
            r.geosite = vec!["google".into()];
            r.geoip   = vec!["google".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("GitHub", RuleAction::Proxy);
            r.geosite = vec!["github".into(), "githubusercontent".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Twitter / X", RuleAction::Proxy);
            r.geosite = vec!["twitter".into()];
            r.geoip   = vec!["twitter".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Meta (Facebook / Instagram)", RuleAction::Proxy);
            r.geosite = vec!["facebook".into(), "instagram".into()];
            r.geoip   = vec!["facebook".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("TikTok", RuleAction::Proxy);
            r.geosite = vec!["tiktok".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Netflix", RuleAction::Proxy);
            r.geosite = vec!["netflix".into()];
            r.domain_suffix = vec!["netflix.com".into(), "netflix.net".into(), "nflximg.net".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("国际流媒体", RuleAction::Proxy);
            r.geosite = vec!["disney".into(), "hbo".into()];
            r.domain_suffix = vec![
                "disneyplus.com".into(),
                "hulu.com".into(),
                "hbomax.com".into(),
                "max.com".into(),
                "primevideo.com".into(),
                "spotify.com".into(),
                "twitch.tv".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("Steam 游戏", RuleAction::Proxy);
            r.geosite = vec!["steam".into()];
            r.domain_suffix = vec![
                "steampowered.com".into(),
                "steamcommunity.com".into(),
                "steamstatic.com".into(),
                "steamcdn-a.akamaihd.net".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("Speedtest", RuleAction::Proxy);
            r.domain_suffix = vec![
                "speedtest.net".into(),
                "fast.com".into(),
            ];
            r
        },
        // ── 国内服务直连 ─────────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("苹果服务直连", RuleAction::Direct);
            r.geosite = vec!["apple".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("微软服务直连", RuleAction::Direct);
            r.geosite = vec!["microsoft".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("哔哩哔哩", RuleAction::Direct);
            r.geosite = vec!["bilibili".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("国内流媒体直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "iqiyi.com".into(),
                "iq.com".into(),
                "youku.com".into(),
                "mgtv.com".into(),
                "migu.cn".into(),
                "douyu.com".into(),
                "huya.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("百度系直连", RuleAction::Direct);
            r.geosite = vec!["baidu".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("腾讯服务直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "qq.com".into(),
                "weixin.qq.com".into(),
                "wx.qq.com".into(),
                "tencent.com".into(),
                "qcloud.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("阿里系直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "alibaba.com".into(),
                "aliyun.com".into(),
                "taobao.com".into(),
                "tmall.com".into(),
                "alipay.com".into(),
                "alicdn.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("字节跳动直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "bytedance.com".into(),
                "toutiao.com".into(),
                "douyin.com".into(),
                "feishu.cn".into(),
                "feishu.com".into(),
            ];
            r
        },
        // ── 兜底 ────────────────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("中国大陆直连", RuleAction::Direct);
            r.geosite = vec!["cn".into()];
            r.geoip   = vec!["cn".into()];
            r
        },
    ]
}

pub fn load_rules() -> Vec<RouteRule> {
    let path = config::app_data_dir().join("rules.json");
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        // First run: use preset rules
        preset_rules()
    }
}

pub fn save_rules(rules: &[RouteRule]) -> Result<()> {
    config::ensure_dirs()?;
    let path = config::app_data_dir().join("rules.json");
    let data = serde_json::to_string_pretty(rules)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Convert RouteRule list to sing-box route.rules JSON array
pub fn rules_to_singbox(rules: &[RouteRule]) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut result = Vec::new();

    // DNS rule always first
    result.push(json!({ "protocol": "dns", "outbound": "dns-out" }));

    for rule in rules.iter().filter(|r| r.enabled) {
        let mut obj = serde_json::Map::new();

        if !rule.domain.is_empty() {
            obj.insert("domain".into(), json!(rule.domain));
        }
        if !rule.domain_suffix.is_empty() {
            obj.insert("domain_suffix".into(), json!(rule.domain_suffix));
        }
        if !rule.domain_keyword.is_empty() {
            obj.insert("domain_keyword".into(), json!(rule.domain_keyword));
        }
        if !rule.geosite.is_empty() {
            obj.insert("geosite".into(), json!(rule.geosite));
        }
        if !rule.geoip.is_empty() {
            obj.insert("geoip".into(), json!(rule.geoip));
        }
        if !rule.ip_cidr.is_empty() {
            obj.insert("ip_cidr".into(), json!(rule.ip_cidr));
        }
        if !rule.port.is_empty() {
            // Flatten port ranges to individual values
            let ports: Vec<serde_json::Value> = rule.port.iter()
                .flat_map(|p| {
                    if p.contains('-') {
                        let parts: Vec<&str> = p.split('-').collect();
                        if parts.len() == 2 {
                            if let (Ok(s), Ok(e)) = (parts[0].parse::<u16>(), parts[1].parse::<u16>()) {
                                return (s..=e).map(|n| json!(n)).collect::<Vec<_>>();
                            }
                        }
                        vec![]
                    } else if let Ok(n) = p.parse::<u16>() {
                        vec![json!(n)]
                    } else {
                        vec![]
                    }
                })
                .collect();
            if !ports.is_empty() {
                obj.insert("port".into(), json!(ports));
            }
        }
        if let Some(ref net) = rule.network {
            obj.insert("network".into(), json!(net));
        }
        if !rule.process_name.is_empty() {
            obj.insert("process_name".into(), json!(rule.process_name));
        }

        if !obj.is_empty() {
            obj.insert("outbound".into(), json!(rule.action.to_string()));
            result.push(serde_json::Value::Object(obj));
        }
    }

    // Clash mode overrides always at end
    result.push(json!({ "clash_mode": "direct", "outbound": "direct" }));
    result.push(json!({ "clash_mode": "global", "outbound": "proxy" }));

    result
}
