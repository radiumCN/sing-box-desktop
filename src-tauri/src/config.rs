use std::path::PathBuf;
use std::fs;
use anyhow::Result;
use serde_json::Value;
use crate::types::{AppConfig, Subscription, ProxyNode};

/// User-Agent used when fetching subscriptions. Many airports gate the returned content
/// on the client UA: a legacy "Clash" identifier (e.g. `ClashForWindows`) makes
/// protocol-rich airports (vless-reality / hysteria2 / tuic) serve a "please switch
/// client" placeholder config (fake `ss` nodes on 127.0.0.1) instead of the real nodes,
/// because the original Clash core cannot handle those protocols. A modern, widely
/// whitelisted client identifier makes them return the universal Base64 node list (or a
/// Clash.Meta YAML), both of which the parser fully supports. Our core is sing-box, which
/// supports every protocol these airports serve.
pub const SUBSCRIPTION_USER_AGENT: &str = "v2rayN/6.45";

/// Effective subscription User-Agent: the user-configured value, or the built-in default
/// when it is unset/blank. Read fresh from the persisted config so a settings change takes
/// effect on the next fetch without restarting.
pub fn subscription_user_agent() -> String {
    let ua = load_app_config().subscription_user_agent;
    if ua.trim().is_empty() {
        SUBSCRIPTION_USER_AGENT.to_string()
    } else {
        ua
    }
}

pub fn app_data_dir() -> PathBuf {
    let base = dirs_next::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Skylark")
}

pub fn singbox_config_path() -> PathBuf {
    app_data_dir().join("config.json")
}

pub fn subscriptions_dir() -> PathBuf {
    app_data_dir().join("subscriptions")
}

/// Directory holding the locally-bundled sing-box rule-set (.srs) files.
/// These are copied from the app resources on startup so the generated config
/// can reference them by absolute path even where the remote CDN is blocked.
pub fn rule_sets_dir() -> PathBuf {
    app_data_dir().join("rule-sets")
}

pub fn ensure_dirs() -> Result<()> {
    fs::create_dir_all(app_data_dir())?;
    fs::create_dir_all(subscriptions_dir())?;
    fs::create_dir_all(rule_sets_dir())?;
    Ok(())
}

/// Stable random secret guarding the Clash API (`external_controller`). Generated once
/// on first use and persisted to a dedicated file (NOT app_config.json, which round-trips
/// through the frontend and could otherwise be wiped on a settings save). Cached for the
/// process lifetime. Both the generated sing-box config and every Clash API caller read
/// this same value, so the `Authorization: Bearer <secret>` header always matches.
pub fn api_secret() -> String {
    static API_SECRET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    API_SECRET
        .get_or_init(|| {
            let path = app_data_dir().join("api_secret");
            if let Ok(s) = fs::read_to_string(&path) {
                let s = s.trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
            let secret = uuid::Uuid::new_v4().simple().to_string();
            let _ = ensure_dirs();
            let _ = fs::write(&path, &secret);
            secret
        })
        .clone()
}

/// Preferred UI language for a fresh install, derived from the OS locale. A Chinese
/// system (`zh*`) maps to `zh-CN`; everything else falls back to English. Only used when
/// no `app_config.json` exists yet — once the user has a config, their saved choice wins.
fn detect_system_language() -> String {
    let locale = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if locale.starts_with("zh") {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

pub fn load_app_config() -> AppConfig {
    let path = app_data_dir().join("app_config.json");
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig {
            language: detect_system_language(),
            ..AppConfig::default()
        }
    }
}

pub fn save_app_config(config: &AppConfig) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("app_config.json");
    let data = serde_json::to_string_pretty(config)?;
    fs::write(path, data)?;
    Ok(())
}

pub fn load_subscriptions() -> Vec<Subscription> {
    let path = app_data_dir().join("subscriptions.json");
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_subscriptions(subs: &[Subscription]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("subscriptions.json");
    let data = serde_json::to_string_pretty(subs)?;
    fs::write(path, data)?;
    Ok(())
}

pub fn load_nodes() -> Vec<ProxyNode> {
    let path = app_data_dir().join("nodes.json");
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_nodes(nodes: &[ProxyNode]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("nodes.json");
    let data = serde_json::to_string_pretty(nodes)?;
    fs::write(path, data)?;
    Ok(())
}

pub fn load_outbounds() -> Vec<Value> {
    let path = app_data_dir().join("outbounds.json");
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_outbounds(outbounds: &[Value]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("outbounds.json");
    let data = serde_json::to_string_pretty(outbounds)?;
    fs::write(path, data)?;
    Ok(())
}

pub fn load_proxy_groups() -> Vec<crate::types::ProxyGroup> {
    let path = app_data_dir().join("proxy_groups.json");
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_proxy_groups(groups: &[crate::types::ProxyGroup]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("proxy_groups.json");
    let data = serde_json::to_string_pretty(groups)?;
    fs::write(path, data)?;
    Ok(())
}

/// Cache the raw text content of a subscription so it can be re-parsed on startup.
pub fn save_subscription_content(id: &str, content: &str) -> Result<()> {
    ensure_dirs()?;
    let path = subscriptions_dir().join(format!("{}.txt", id));
    fs::write(path, content)?;
    Ok(())
}

pub fn load_subscription_content(id: &str) -> Option<String> {
    let path = subscriptions_dir().join(format!("{}.txt", id));
    fs::read_to_string(path).ok()
}

pub fn delete_subscription_content(id: &str) {
    let path = subscriptions_dir().join(format!("{}.txt", id));
    let _ = fs::remove_file(path);
}
