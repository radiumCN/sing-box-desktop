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

/// Holds cloned handles to tray check-menu items so commands can update them.
pub struct TrayState {
    pub sys_proxy_item: Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
    pub tun_item:       Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
}

// ─── Sing-box Control ───────────────────────────────────────────────

/// Core start logic shared by the Tauri command and the tray menu handler.
pub async fn start_proxy_internal(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let config = state.app_config.lock().unwrap().clone();
    let outbounds = state.outbounds.lock().unwrap().clone();
    let nodes = state.nodes.lock().unwrap().clone();
    let active_tag = config.active_nodes.get("proxy").cloned();

    // TUN mode preconditions: requires elevated privileges (admin on Windows, root on
    // macOS/Linux). On Windows it additionally needs the WinTun driver.
    if config.tun_enabled {
        if !crate::tun::is_elevated() {
            return Err("TUN 模式需要管理员/root 权限。请在「设置 → TUN 模式」中点击「以管理员身份重启」后再启动。".to_string());
        }
        #[cfg(target_os = "windows")]
        if !crate::tun::wintun_available() {
            return Err("TUN 模式需要 WinTun 驱动。请在「设置 → TUN 模式」中点击「下载 WinTun」后再启动。".to_string());
        }
        crate::tun::cleanup_stale_tun_adapter().await;
    }

    let singbox_cfg = subscription::build_singbox_config(
        &outbounds,
        &config,
        active_tag.as_deref(),
        &nodes,
    );

    let config_path = config::singbox_config_path();
    config::ensure_dirs().map_err(|e| e.to_string())?;
    std::fs::write(&config_path, serde_json::to_string_pretty(&singbox_cfg).unwrap())
        .map_err(|e| e.to_string())?;

    start_singbox(app_handle, &config_path, state.singbox_state.clone())
        .await
        .map_err(|e| e.to_string())?;

    // System proxy and TUN are mutually exclusive routing paths.
    // When TUN is on it captures traffic at the network layer, so a WinINet
    // system proxy would create a conflicting double-proxy path — skip it.
    let enable_sys_proxy = !config.tun_enabled && config.proxy_mode != ProxyMode::Tun;
    if enable_sys_proxy {
        proxy::set_system_proxy(true, config.mixed_port)
            .map_err(|e| e.to_string())?;
    } else {
        // Ensure any stale system proxy is cleared so it never coexists with TUN.
        let _ = proxy::set_system_proxy(false, 0);
    }

    // Persist last running state for restore-on-startup
    {
        let mut cfg = state.app_config.lock().unwrap();
        cfg.last_proxy_running = true;
        cfg.last_system_proxy = enable_sys_proxy;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
    }

    Ok(())
}

#[tauri::command]
pub async fn cmd_start_singbox(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    start_proxy_internal(&app_handle, &state).await
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

    // Persist last running state
    {
        let mut cfg = state.app_config.lock().unwrap();
        cfg.last_proxy_running = false;
        cfg.last_system_proxy = false;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
    }

    Ok(())
}

/// Unified connection control shared by the tray menu (mirrors the dashboard's two
/// mutually-exclusive switches). `mode` is one of:
///   • "off"    → stop the core and clear the system proxy
///   • "system" → run with the Windows system proxy (TUN forced off)
///   • "tun"    → run in TUN mode (system proxy forced off)
/// Restarts the core so the new config takes effect. On failure the TUN flag is
/// rolled back so persisted state never diverges from what actually started.
pub async fn apply_connection_mode(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    mode: &str,
) -> Result<(), String> {
    if mode == "off" {
        stop_singbox(state.singbox_state.clone())
            .await
            .map_err(|e| e.to_string())?;
        let _ = proxy::set_system_proxy(false, 0);
        {
            let mut cfg = state.app_config.lock().unwrap();
            cfg.last_proxy_running = false;
            cfg.last_system_proxy = false;
            let cfg_clone = cfg.clone();
            drop(cfg);
            let _ = config::save_app_config(&cfg_clone);
        }
        return Ok(());
    }

    let want_tun = mode == "tun";
    // Persist the TUN flag first so the generated config matches the chosen mode.
    let prev_tun = {
        let mut cfg = state.app_config.lock().unwrap();
        let prev = cfg.tun_enabled;
        cfg.tun_enabled = want_tun;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
        prev
    };

    // Restart the core so the new (TUN vs system) config takes effect.
    let running = state.singbox_state.lock().unwrap().running;
    if running {
        let _ = stop_singbox(state.singbox_state.clone()).await;
    }

    // start_proxy_internal sets / clears the system proxy itself based on the flag.
    if let Err(e) = start_proxy_internal(app_handle, state).await {
        let mut cfg = state.app_config.lock().unwrap();
        cfg.tun_enabled = prev_tun;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
        return Err(e);
    }
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
pub fn cmd_save_subscription_settings(
    id: String,
    auto_update: bool,
    update_interval: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut subs = state.subscriptions.lock().unwrap();
    let sub = subs.iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| "订阅不存在".to_string())?;
    sub.auto_update = auto_update;
    sub.update_interval = update_interval;
    config::save_subscriptions(&subs).map_err(|e| e.to_string())
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

/// Switch a *running* sing-box selector to `name` via the Clash API so the change
/// takes effect immediately without restarting the core. Best-effort: callers
/// persist the choice first, so a later restart still applies it even if this fails.
async fn clash_select_proxy(api_port: u16, group: &str, name: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/proxies/{}", api_port, group);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .put(&url)
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Clash API 返回 {}", resp.status()))
    }
}

/// Force an immediate latency re-test of every node in an auto (urltest) group via
/// the Clash API. `group` is "auto" (all nodes) or "auto-<sub.id>" (one
/// subscription). Testing each member updates the core's delay history, which the
/// urltest group then uses to (re)select the fastest node. Best-effort per node.
#[tauri::command]
pub async fn cmd_test_group_delay(
    group: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (api_port, running, test_url, members) = {
        let cfg = state.app_config.lock().unwrap();
        let running = state.singbox_state.lock().unwrap().running;
        let nodes = state.nodes.lock().unwrap();
        let members: Vec<String> = if group == "auto" {
            nodes.iter().map(|n| n.name.clone()).collect()
        } else if let Some(sid) = group.strip_prefix("auto-") {
            nodes.iter()
                .filter(|n| n.subscription_id.as_deref() == Some(sid))
                .map(|n| n.name.clone())
                .collect()
        } else {
            vec![group.clone()]
        };
        let url = if cfg.auto_test_url.trim().is_empty() {
            "https://www.gstatic.com/generate_204".to_string()
        } else {
            cfg.auto_test_url.trim().to_string()
        };
        (cfg.api_port, running, url, members)
    };

    if !running {
        return Err("代理未运行".to_string());
    }
    if members.is_empty() {
        return Err("该分组没有可测试的节点".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    let mut tasks = Vec::new();
    for tag in members {
        let client = client.clone();
        let url = test_url.clone();
        tasks.push(tokio::spawn(async move {
            // Build the endpoint with path-segment encoding so node names containing
            // spaces / unicode / slashes don't corrupt the request path.
            let mut endpoint = match reqwest::Url::parse(&format!(
                "http://127.0.0.1:{}/proxies",
                api_port
            )) {
                Ok(u) => u,
                Err(_) => return,
            };
            if let Ok(mut seg) = endpoint.path_segments_mut() {
                seg.push(&tag).push("delay");
            }
            endpoint
                .query_pairs_mut()
                .append_pair("url", &url)
                .append_pair("timeout", "5000");
            let _ = client.get(endpoint).send().await;
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    Ok(())
}

#[tauri::command]
pub async fn cmd_set_active_node(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    /* Collect everything that needs locks first, then drop the guards before any
       `.await` — std Mutex guards are not Send and cannot be held across await. */
    let (tag, api_port, running) = {
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

        let mut config = state.app_config.lock().unwrap();
        config.active_nodes.insert("proxy".to_string(), tag.clone());
        config::save_app_config(&config).map_err(|e| e.to_string())?;
        let api_port = config.api_port;
        let running = state.singbox_state.lock().unwrap().running;
        (tag, api_port, running)
    };

    /* Apply live when the core is running so switching is instant (no restart). */
    if running {
        let _ = clash_select_proxy(api_port, "proxy", &tag).await;
    }
    Ok(())
}

/// Switch the proxy group to a dynamic urltest selection. `group` defaults to the
/// global "auto" group; pass "auto-<sub.id>" to use a per-subscription group. The
/// core then continuously health-checks that group's nodes and routes via the
/// fastest one.
#[tauri::command]
pub async fn cmd_set_auto_node(
    group: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let group = group.unwrap_or_else(|| "auto".to_string());
    let (api_port, running) = {
        let mut nodes = state.nodes.lock().unwrap();
        for node in nodes.iter_mut() {
            node.is_active = false;
        }
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;

        let mut config = state.app_config.lock().unwrap();
        config.active_nodes.insert("proxy".to_string(), group.clone());
        config::save_app_config(&config).map_err(|e| e.to_string())?;
        let api_port = config.api_port;
        let running = state.singbox_state.lock().unwrap().running;
        (api_port, running)
    };

    if running {
        let _ = clash_select_proxy(api_port, "proxy", &group).await;
    }
    Ok(())
}

/// Resolve the concrete node the "proxy" group is currently routing through by
/// following the selector → urltest chain via the Clash API. Returns `None` when
/// the core is not running or the chain cannot be resolved. Used by the UI/tray to
/// show which node an "auto" group actually picked.
#[tauri::command]
pub async fn cmd_get_active_proxy_now(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let (api_port, running) = {
        let cfg = state.app_config.lock().unwrap();
        let running = state.singbox_state.lock().unwrap().running;
        (cfg.api_port, running)
    };
    if !running {
        return Ok(None);
    }

    let url = format!("http://127.0.0.1:{}/proxies", api_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let json: Value = resp.json().await.map_err(|e| e.to_string())?;
    let proxies = match json.get("proxies") {
        Some(p) => p,
        None => return Ok(None),
    };

    /* Walk from "proxy" through any Selector/URLTest layers down to a real node.
       The hop cap prevents an infinite loop on a malformed/cyclic response. */
    let mut cur = "proxy".to_string();
    for _ in 0..6 {
        let node = match proxies.get(&cur) {
            Some(n) => n,
            None => break,
        };
        let typ = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if typ == "Selector" || typ == "URLTest" {
            match node.get("now").and_then(|n| n.as_str()) {
                Some(now) if !now.is_empty() => cur = now.to_string(),
                _ => break,
            }
        } else {
            break;
        }
    }
    Ok(Some(cur))
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

/* Cumulative traffic counters reported by the Clash API, in bytes. The core keeps
   these from the moment it starts, so they represent ALL traffic since the proxy
   service started — independent of which UI page is open. They reset to zero each
   time the core restarts. */
#[derive(serde::Serialize, Default)]
pub struct TrafficTotal {
    pub upload: u64,
    pub download: u64,
}

/// Returns total upload/download bytes since the sing-box core started by reading
/// the top-level `uploadTotal`/`downloadTotal` of the Clash `/connections` endpoint.
/// Returns zeros when the core is not running or the query fails.
#[tauri::command]
pub async fn cmd_get_traffic_total(
    state: State<'_, AppState>,
) -> Result<TrafficTotal, String> {
    let (api_port, running) = {
        let cfg = state.app_config.lock().unwrap();
        let running = state.singbox_state.lock().unwrap().running;
        (cfg.api_port, running)
    };
    if !running {
        return Ok(TrafficTotal::default());
    }

    let url = format!("http://127.0.0.1:{}/connections", api_port);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Ok(TrafficTotal::default()),
    };

    /* Best-effort: a transient API hiccup should not surface as a hard error in the
       UI — fall back to zeros so the poller simply keeps the last good display. */
    let body: Value = match client.get(&url).send().await {
        Ok(resp) => match resp.json().await {
            Ok(v) => v,
            Err(_) => return Ok(TrafficTotal::default()),
        },
        Err(_) => return Ok(TrafficTotal::default()),
    };

    Ok(TrafficTotal {
        upload: body["uploadTotal"].as_u64().unwrap_or(0),
        download: body["downloadTotal"].as_u64().unwrap_or(0),
    })
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
pub async fn cmd_check_singbox_update(force_refresh: Option<bool>) -> Result<crate::updater::ReleaseInfo, String> {
    crate::updater::fetch_latest_release(force_refresh.unwrap_or(false))
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

// ─── App Self-Updater ───────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_check_app_update(
    channel: Option<String>,
    force_refresh: Option<bool>,
) -> Result<crate::updater::AppReleaseInfo, String> {
    let ch = channel.as_deref().unwrap_or("stable");
    crate::updater::fetch_app_release(ch, force_refresh.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_download_app_update(
    app_handle: tauri::AppHandle,
    download_url: String,
) -> Result<(), String> {
    crate::updater::download_and_install_app(app_handle, download_url)
        .await
        .map_err(|e| e.to_string())
}

// ─── System Proxy ───────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_system_proxy_status() -> bool {
    crate::proxy::get_system_proxy_status()
}

#[tauri::command]
pub fn cmd_set_system_proxy(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if enabled {
        let running = state.singbox_state.lock().unwrap().running;
        if !running {
            return Err("sing-box 未运行，请先启动代理再开启系统代理".to_string());
        }
        if state.app_config.lock().unwrap().tun_enabled {
            return Err("TUN 模式已接管全部流量，无需且不能再开启系统代理".to_string());
        }
    }
    let port = state.app_config.lock().unwrap().mixed_port;
    crate::proxy::set_system_proxy(enabled, if enabled { port } else { 0 })
        .map_err(|e| e.to_string())
}

// ─── Tray ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_update_tray_tooltip(app_handle: tauri::AppHandle, tooltip: String) {
    if let Some(tray) = app_handle.tray_by_id("tray-main") {
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

/// Update the tray check-menu items to reflect current system-proxy and TUN state.
#[tauri::command]
pub fn cmd_sync_tray_menu(
    sys_proxy_enabled: bool,
    tun_enabled: bool,
    tray_state: State<'_, TrayState>,
) {
    if let Ok(guard) = tray_state.sys_proxy_item.lock() {
        if let Some(item) = guard.as_ref() {
            let _ = item.set_checked(sys_proxy_enabled);
        }
    }
    if let Ok(guard) = tray_state.tun_item.lock() {
        if let Some(item) = guard.as_ref() {
            let _ = item.set_checked(tun_enabled);
        }
    }
}

// ─── Process Memory ─────────────────────────────────────────────────

/// Returns the working-set memory (RSS) of the sing-box process in bytes,
/// or None if sing-box is not running or the query fails.
#[tauri::command]
pub fn cmd_get_memory_usage(state: State<AppState>) -> Option<u64> {
    let pid = state.singbox_state.lock().unwrap().pid?;

    #[cfg(target_os = "windows")]
    unsafe {
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::processthreadsapi::OpenProcess;
        use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(
            handle,
            &mut pmc,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        CloseHandle(handle);
        if ok != 0 { Some(pmc.WorkingSetSize as u64) } else { None }
    }

    // macOS: read resident set size (in KB) from `ps`.
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let kb: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        Some(kb * 1024)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
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
