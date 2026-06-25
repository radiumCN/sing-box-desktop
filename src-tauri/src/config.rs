use std::path::PathBuf;
use std::fs;
use anyhow::Result;
use serde_json::Value;
use crate::types::{AppConfig, Subscription, ProxyNode};

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

pub fn load_app_config() -> AppConfig {
    let path = app_data_dir().join("app_config.json");
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig::default()
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
