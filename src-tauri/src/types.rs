use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sub_type: SubType,
    pub node_count: usize,
    pub last_update: Option<DateTime<Utc>>,
    pub auto_update: bool,
    pub update_interval: u32, // hours
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubType {
    Clash,
    V2ray,
    Sip008,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub group: String,
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub latency: Option<u32>,        // ms
    pub download_speed: Option<u32>, // KB/s
    pub is_active: bool,
    pub subscription_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedResult {
    pub latency_ms: Option<u32>,
    pub download_kbps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyGroup {
    pub name: String,
    pub group_type: String, // Selector, URLTest, Fallback
    pub current: String,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficStats {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub upload_speed: u64,   // bytes/s
    pub download_speed: u64, // bytes/s
    pub connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub network: String,
    pub conn_type: String,
    pub source: String,
    pub destination: String,
    pub host: String,
    pub rule: String,
    pub rule_payload: String,
    pub chains: Vec<String>,
    pub upload: u64,
    pub download: u64,
    pub start: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
    pub time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub proxy_mode: ProxyMode,
    pub startup_with_system: bool,
    pub startup_minimized: bool,
    pub allow_lan: bool,
    pub http_port: u16,
    pub socks_port: u16,
    pub mixed_port: u16,
    pub api_port: u16,
    pub tun_enabled: bool,
    pub log_level: String,
    pub theme: String,
    pub language: String,
    pub selected_subscription: Option<String>,
    pub active_nodes: std::collections::HashMap<String, String>,
    /// 0 = disabled, otherwise check every N hours
    pub auto_update_interval: u32,
    pub auto_update_notify: bool,
    /// true = close button minimizes to tray; false = exits the app
    pub close_to_tray: bool,
    /// Restore proxy running state (sing-box + system proxy) on next startup
    pub restore_proxy_on_startup: bool,
    /// Last known sing-box running state (written on start/stop)
    #[serde(default)]
    pub last_proxy_running: bool,
    /// Last known system proxy enabled state (written on start/stop)
    #[serde(default)]
    pub last_system_proxy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    Rule,
    Global,
    Direct,
    Tun,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            proxy_mode: ProxyMode::Rule,
            startup_with_system: false,
            startup_minimized: false,
            allow_lan: false,
            http_port: 7890,
            socks_port: 7891,
            mixed_port: 7890,
            api_port: 9090,
            tun_enabled: false,
            log_level: "info".to_string(),
            theme: "system".to_string(),
            language: "zh-CN".to_string(),
            selected_subscription: None,
            active_nodes: std::collections::HashMap::new(),
            auto_update_interval: 24,
            auto_update_notify: true,
            close_to_tray: true,
            restore_proxy_on_startup: false,
            last_proxy_running: false,
            last_system_proxy: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingboxStatus {
    pub running: bool,
    pub uptime: Option<u64>, // seconds
    pub pid: Option<u32>,
    pub version: Option<String>,
}
