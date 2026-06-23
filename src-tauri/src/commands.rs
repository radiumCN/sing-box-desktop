use tauri::State;
use std::sync::Mutex;
use anyhow::anyhow;
use serde_json::Value;

use crate::types::{self, *};
use crate::config;
use crate::subscription;
use crate::singbox::{SharedState, start_singbox, stop_singbox};
use crate::proxy;

pub struct AppState {
    pub singbox_state: SharedState,
    pub subscriptions: Mutex<Vec<Subscription>>,
    pub nodes: Mutex<Vec<ProxyNode>>,
    pub outbounds: Mutex<Vec<Value>>,
    pub app_config: Mutex<AppConfig>,
}

// ─── Sing-box Control ───────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_start_singbox(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let config = state.app_config.lock().unwrap().clone();
    let outbounds = state.outbounds.lock().unwrap().clone();
    let active_tag = config.active_nodes.get("proxy").cloned();

    // TUN mode preconditions: requires administrator privileges and the WinTun driver.
    // Fail early with a clear message instead of letting sing-box crash silently.
    if config.tun_enabled {
        if !crate::tun::is_elevated() {
            return Err("TUN 模式需要管理员权限。请在「设置 → TUN 模式」中点击「以管理员身份重启」后再启动。".to_string());
        }
        if !crate::tun::wintun_available() {
            return Err("TUN 模式需要 WinTun 驱动。请在「设置 → TUN 模式」中点击「下载 WinTun」后再启动。".to_string());
        }
    }

    let singbox_cfg = subscription::build_singbox_config(
        &outbounds,
        &config,
        active_tag.as_deref(),
    );

    let config_path = config::singbox_config_path();
    config::ensure_dirs().map_err(|e| e.to_string())?;
    std::fs::write(&config_path, serde_json::to_string_pretty(&singbox_cfg).unwrap())
        .map_err(|e| e.to_string())?;

    start_singbox(&app_handle, &config_path, state.singbox_state.clone())
        .await
        .map_err(|e| e.to_string())?;

    if config.proxy_mode != ProxyMode::Tun {
        proxy::set_system_proxy(true, config.mixed_port)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn cmd_stop_singbox(
    state: State<'_, AppState>,
) -> Result<(), String> {
    stop_singbox(state.singbox_state.clone())
        .await
        .map_err(|e| e.to_string())?;

    proxy::set_system_proxy(false, 0)
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn cmd_get_singbox_status(
    state: State<'_, AppState>,
) -> SingboxStatus {
    let s = state.singbox_state.lock().unwrap();
    SingboxStatus {
        running: s.running,
        uptime: s.start_time.map(|t| t.elapsed().as_secs()),
        pid: s.pid,
        version: s.version.clone(),
    }
}

#[tauri::command]
pub fn cmd_get_logs(state: State<'_, AppState>) -> Vec<String> {
    state.singbox_state.lock().unwrap().logs.clone()
}

// ─── Subscriptions ──────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_subscriptions(state: State<'_, AppState>) -> Vec<Subscription> {
    state.subscriptions.lock().unwrap().clone()
}

#[tauri::command]
pub async fn cmd_add_subscription(
    name: String,
    url: String,
    state: State<'_, AppState>,
) -> Result<Subscription, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let content = fetch_url(&url).await.map_err(|e| e.to_string())?;
    config::save_subscription_content(&id, &content).map_err(|e| e.to_string())?;
    let sub_type = subscription::detect_sub_type(&content, &url);
    let (nodes, outbounds) = subscription::parse_subscription(&content, &id)
        .map_err(|e| e.to_string())?;

    let sub = Subscription {
        id: id.clone(),
        name,
        url,
        sub_type,
        node_count: nodes.len(),
        last_update: Some(chrono::Utc::now()),
        auto_update: true,
        update_interval: 24,
    };

    {
        let mut subs = state.subscriptions.lock().unwrap();
        subs.push(sub.clone());
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
    }
    {
        let mut all_nodes = state.nodes.lock().unwrap();
        all_nodes.retain(|n| n.subscription_id.as_deref() != Some(&id));
        all_nodes.extend(nodes);
        config::save_nodes(&all_nodes).map_err(|e| e.to_string())?;
    }
    {
        let mut all_outbounds = state.outbounds.lock().unwrap();
        all_outbounds.retain(|ob| {
            !outbounds.iter().any(|new| new["tag"] == ob["tag"])
        });
        all_outbounds.extend(outbounds);
        config::save_outbounds(&all_outbounds).map_err(|e| e.to_string())?;
    }

    Ok(sub)
}

#[tauri::command]
pub async fn cmd_update_subscription(
    id: String,
    state: State<'_, AppState>,
) -> Result<Subscription, String> {
    let url = {
        let subs = state.subscriptions.lock().unwrap();
        subs.iter()
            .find(|s| s.id == id)
            .map(|s| s.url.clone())
            .ok_or_else(|| "订阅不存在".to_string())?
    };

    let content = fetch_url(&url).await.map_err(|e| e.to_string())?;
    config::save_subscription_content(&id, &content).map_err(|e| e.to_string())?;
    let sub_type = subscription::detect_sub_type(&content, &url);
    let (nodes, outbounds) = subscription::parse_subscription(&content, &id)
        .map_err(|e| e.to_string())?;

    let updated_sub = {
        let mut subs = state.subscriptions.lock().unwrap();
        let sub = subs.iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| "订阅不存在".to_string())?;
        sub.sub_type = sub_type;
        sub.node_count = nodes.len();
        sub.last_update = Some(chrono::Utc::now());
        let cloned = sub.clone();
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
        cloned
    };

    {
        let mut all_nodes = state.nodes.lock().unwrap();
        all_nodes.retain(|n| n.subscription_id.as_deref() != Some(&id));
        all_nodes.extend(nodes);
        config::save_nodes(&all_nodes).map_err(|e| e.to_string())?;
    }
    {
        let mut all_outbounds = state.outbounds.lock().unwrap();
        let new_tags: std::collections::HashSet<String> = outbounds.iter()
            .filter_map(|ob| ob["tag"].as_str().map(|s| s.to_string()))
            .collect();
        all_outbounds.retain(|ob| {
            ob["tag"].as_str()
                .map(|t| !new_tags.contains(t))
                .unwrap_or(true)
        });
        all_outbounds.extend(outbounds);
        config::save_outbounds(&all_outbounds).map_err(|e| e.to_string())?;
    }

    Ok(updated_sub)
}

#[tauri::command]
pub fn cmd_delete_subscription(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    config::delete_subscription_content(&id);
    {
        let mut subs = state.subscriptions.lock().unwrap();
        subs.retain(|s| s.id != id);
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
    }
    {
        let mut nodes = state.nodes.lock().unwrap();
        nodes.retain(|n| n.subscription_id.as_deref() != Some(&id));
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;
    }
    {
        let mut outbounds = state.outbounds.lock().unwrap();
        outbounds.retain(|ob| {
            ob.get("subscription_id")
                .and_then(|v| v.as_str())
                .map(|s| s != id)
                .unwrap_or(true)
        });
        config::save_outbounds(&outbounds).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Nodes ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_nodes(state: State<'_, AppState>) -> Vec<ProxyNode> {
    state.nodes.lock().unwrap().clone()
}

#[tauri::command]
pub async fn cmd_test_node_latency(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<u32, String> {
    let (server, port, mixed_port, is_running) = {
        let nodes = state.nodes.lock().unwrap();
        let node = nodes.iter().find(|n| n.id == node_id)
            .ok_or_else(|| "节点不存在".to_string())?;
        let cfg = state.app_config.lock().unwrap();
        let running = state.singbox_state.lock().unwrap().running;
        (node.server.clone(), node.port, cfg.mixed_port, running)
    };

    let latency_ms = if is_running {
        // Proxy is running: test via proxy to a reliable URL
        test_via_proxy(mixed_port).await
    } else {
        // Proxy not running: TCP connect to the server itself
        test_tcp_connect(&server, port).await
    };

    match latency_ms {
        Some(ms) => {
            let mut nodes = state.nodes.lock().unwrap();
            if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
                node.latency = Some(ms);
            }
            let _ = config::save_nodes(&nodes);
            Ok(ms)
        }
        None => {
            let mut nodes = state.nodes.lock().unwrap();
            if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
                node.latency = None;
            }
            let _ = config::save_nodes(&nodes);
            Err("超时".to_string())
        }
    }
}

/// Test latency by routing an HTTP request through the local mixed proxy port.
async fn test_via_proxy(mixed_port: u16) -> Option<u32> {
    let proxy_url = format!("http://127.0.0.1:{}", mixed_port);
    let proxy = reqwest::Proxy::http(&proxy_url).ok()?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let start = std::time::Instant::now();
    let result = client
        .get("http://cp.cloudflare.com/generate_204")
        .send()
        .await;
    match result {
        Ok(_) => {
            let ms = start.elapsed().as_millis() as u32;
            Some(ms.max(1))
        }
        Err(_) => None,
    }
}

/// Measure download speed in KB/s by downloading a file through the proxy.
async fn measure_download_speed(mixed_port: u16) -> Option<u32> {
    let proxy_url = format!("http://127.0.0.1:{}", mixed_port);
    // Use Proxy::all so HTTPS CONNECT tunneling works too
    let proxy = reqwest::Proxy::all(&proxy_url).ok()?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let start = std::time::Instant::now();
    let resp = client
        .get("https://speed.cloudflare.com/__down?bytes=2097152") // 2 MB
        .send()
        .await
        .ok()?;
    let bytes = resp.bytes().await.ok()?;
    let ms = start.elapsed().as_millis() as u64;
    if ms == 0 || bytes.is_empty() {
        return None;
    }
    // bytes / ms * 1000 = bytes/s → divide by 1024 → KB/s
    Some(((bytes.len() as u64 * 1000 / ms) / 1024) as u32)
}

#[tauri::command]
pub async fn cmd_test_node_speed(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<types::SpeedResult, String> {
    let (server, port, mixed_port, is_running) = {
        let nodes = state.nodes.lock().unwrap();
        let node = nodes.iter().find(|n| n.id == node_id)
            .ok_or_else(|| "节点不存在".to_string())?;
        let cfg = state.app_config.lock().unwrap();
        let running = state.singbox_state.lock().unwrap().running;
        (node.server.clone(), node.port, cfg.mixed_port, running)
    };

    let (latency_ms, download_kbps) = if is_running {
        // Run latency and download speed in parallel
        let (lat, spd) = tokio::join!(
            test_via_proxy(mixed_port),
            measure_download_speed(mixed_port),
        );
        (lat, spd)
    } else {
        // Proxy not running: only TCP connect, no meaningful speed
        (test_tcp_connect(&server, port).await, None)
    };

    {
        let mut nodes = state.nodes.lock().unwrap();
        if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
            node.latency = latency_ms;
            node.download_speed = download_kbps;
        }
        let _ = config::save_nodes(&nodes);
    }

    Ok(types::SpeedResult { latency_ms, download_kbps })
}

/// Test latency via TCP connect when proxy is not running.
async fn test_tcp_connect(server: &str, port: u16) -> Option<u32> {
    let addr = format!("{}:{}", server, port);
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(&addr),
    ).await;
    match result {
        Ok(Ok(_)) => {
            let micros = start.elapsed().as_micros() as u32;
            // Convert to ms, minimum 1ms to avoid showing 0
            Some((micros / 1000).max(1))
        }
        _ => None,
    }
}

/// Test all nodes and return the id of the fastest one.
#[tauri::command]
pub async fn cmd_auto_select_node(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let (node_ids, mixed_port, is_running) = {
        let nodes = state.nodes.lock().unwrap();
        let cfg = state.app_config.lock().unwrap();
        let running = state.singbox_state.lock().unwrap().running;
        let ids: Vec<(String, String, u16)> = nodes.iter()
            .map(|n| (n.id.clone(), n.server.clone(), n.port))
            .collect();
        (ids, cfg.mixed_port, running)
    };

    let mut tasks = Vec::new();
    for (id, server, port) in node_ids {
        let s = server.clone();
        tasks.push(tokio::spawn(async move {
            let ms = if is_running {
                test_via_proxy(mixed_port).await
            } else {
                test_tcp_connect(&s, port).await
            };
            (id, ms)
        }));
    }

    let mut best_id = None;
    let mut best_ms = u32::MAX;

    for task in tasks {
        if let Ok((id, Some(ms))) = task.await {
            // Update latency in state
            {
                let mut nodes = state.nodes.lock().unwrap();
                if let Some(node) = nodes.iter_mut().find(|n| n.id == id) {
                    node.latency = Some(ms);
                }
            }
            if ms < best_ms {
                best_ms = ms;
                best_id = Some(id);
            }
        }
    }

    let best = best_id.ok_or_else(|| "所有节点均不可达".to_string())?;

    // Set as active and persist both nodes and config
    {
        let tag = {
            let mut nodes = state.nodes.lock().unwrap();
            let mut found_tag = None;
            for node in nodes.iter_mut() {
                if node.id == best {
                    node.is_active = true;
                    found_tag = Some(node.name.clone());
                } else {
                    node.is_active = false;
                }
            }
            let tag = found_tag.ok_or_else(|| "节点不存在".to_string())?;
            config::save_nodes(&nodes).map_err(|e| e.to_string())?;
            tag
        };
        let mut config = state.app_config.lock().unwrap();
        config.active_nodes.insert("proxy".to_string(), tag);
        crate::config::save_app_config(&config).map_err(|e| e.to_string())?;
    }

    Ok(best)
}

#[tauri::command]
pub fn cmd_set_active_node(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let tag = {
        let mut nodes = state.nodes.lock().unwrap();
        let mut found_tag = None;
        for node in nodes.iter_mut() {
            if node.id == node_id {
                node.is_active = true;
                found_tag = Some(node.name.clone());
            } else {
                node.is_active = false;
            }
        }
        let tag = found_tag.ok_or_else(|| "节点不存在".to_string())?;
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;
        tag
    };

    let mut config = state.app_config.lock().unwrap();
    config.active_nodes.insert("proxy".to_string(), tag);
    config::save_app_config(&config).map_err(|e| e.to_string())?;

    Ok(())
}

// ─── Config ─────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_app_config(state: State<'_, AppState>) -> AppConfig {
    state.app_config.lock().unwrap().clone()
}

#[tauri::command]
pub fn cmd_save_app_config(
    new_config: AppConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    config::save_app_config(&new_config).map_err(|e| e.to_string())?;
    *state.app_config.lock().unwrap() = new_config;
    Ok(())
}

#[tauri::command]
pub fn cmd_set_proxy_mode(
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut config = state.app_config.lock().unwrap();
    config.proxy_mode = match mode.as_str() {
        "rule" => ProxyMode::Rule,
        "global" => ProxyMode::Global,
        "direct" => ProxyMode::Direct,
        "tun" => ProxyMode::Tun,
        _ => return Err(format!("未知模式: {}", mode)),
    };
    config::save_app_config(&config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_get_connections(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, String> {
    let port = state.app_config.lock().unwrap().api_port;
    crate::singbox::fetch_connections(port)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cmd_parse_subscription_from_text(
    content: String,
    sub_id: String,
) -> Result<Vec<ProxyNode>, String> {
    let (nodes, _) = subscription::parse_subscription(&content, &sub_id)
        .map_err(|e| e.to_string())?;
    Ok(nodes)
}

// ─── TUN / Admin ────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_is_elevated() -> bool {
    crate::tun::is_elevated()
}

#[tauri::command]
pub fn cmd_relaunch_as_admin() -> Result<(), String> {
    crate::tun::relaunch_as_admin().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cmd_wintun_available() -> bool {
    crate::tun::wintun_available()
}

#[tauri::command]
pub async fn cmd_download_wintun() -> Result<(), String> {
    let bin_dir = crate::updater::singbox_binary_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    crate::tun::download_wintun(&bin_dir)
        .await
        .map_err(|e| e.to_string())
}

// ─── Rules ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_rules() -> Vec<crate::rules::RouteRule> {
    crate::rules::load_rules()
}

#[tauri::command]
pub fn cmd_save_rules(rules: Vec<crate::rules::RouteRule>) -> Result<(), String> {
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cmd_add_rule(rule: crate::rules::RouteRule) -> Result<Vec<crate::rules::RouteRule>, String> {
    let mut rules = crate::rules::load_rules();
    rules.push(rule);
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

#[tauri::command]
pub fn cmd_delete_rule(id: String) -> Result<Vec<crate::rules::RouteRule>, String> {
    let mut rules = crate::rules::load_rules();
    rules.retain(|r| r.id != id);
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

#[tauri::command]
pub fn cmd_toggle_rule(id: String) -> Result<Vec<crate::rules::RouteRule>, String> {
    let mut rules = crate::rules::load_rules();
    if let Some(rule) = rules.iter_mut().find(|r| r.id == id) {
        rule.enabled = !rule.enabled;
    }
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

#[tauri::command]
pub fn cmd_reset_rules() -> Result<Vec<crate::rules::RouteRule>, String> {
    let rules = crate::rules::preset_rules();
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

// ─── Updater ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_check_singbox_update() -> Result<crate::updater::ReleaseInfo, String> {
    crate::updater::fetch_latest_release()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_get_installed_version() -> Option<String> {
    crate::updater::get_installed_version().await
}

#[tauri::command]
pub fn cmd_singbox_exists() -> bool {
    crate::updater::singbox_exists()
}

#[tauri::command]
pub async fn cmd_download_singbox(
    app_handle: tauri::AppHandle,
    download_url: String,
) -> Result<(), String> {
    let dest = crate::updater::singbox_binary_path();
    crate::updater::download_singbox(app_handle, download_url, dest)
        .await
        .map_err(|e| {
            // Emit failure event so frontend can handle it
            e.to_string()
        })
}

// ─── Helpers ────────────────────────────────────────────────────────

async fn fetch_url(url: &str) -> Result<String, anyhow::Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("ClashForWindows/0.20.39")
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    Ok(resp.text().await?)
}
