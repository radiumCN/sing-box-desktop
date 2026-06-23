use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::{Result, anyhow};
use serde_json::Value;
use tauri::Manager;
use tokio::process::Command as TokioCommand;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Windows CREATE_NO_WINDOW flag: prevents a console window from popping up
/// when spawning child processes (sing-box, taskkill).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct SingboxState {
    pub running: bool,
    pub pid: Option<u32>,
    pub start_time: Option<Instant>,
    pub version: Option<String>,
    pub logs: Vec<String>,
}

impl Default for SingboxState {
    fn default() -> Self {
        Self {
            running: false,
            pid: None,
            start_time: None,
            version: None,
            logs: Vec::new(),
        }
    }
}

pub type SharedState = Arc<Mutex<SingboxState>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(Mutex::new(SingboxState::default()))
}

/// Get the sing-box binary path.
/// Priority: user-downloaded binary in app data dir > bundled sidecar
pub fn singbox_binary_path(app_handle: &tauri::AppHandle) -> Result<std::path::PathBuf> {
    // 1. User-downloaded binary takes priority
    let user_path = crate::updater::singbox_binary_path();
    if user_path.exists() {
        return Ok(user_path);
    }

    // 2. Fall back to bundled sidecar
    let resource_path = app_handle
        .path()
        .resource_dir()
        .map_err(|e| anyhow!("无法获取资源目录: {}", e))?;

    #[cfg(target_os = "windows")]
    let binary = "binaries/sing-box.exe";
    #[cfg(not(target_os = "windows"))]
    let binary = "binaries/sing-box";

    let path = resource_path.join(binary);
    if path.exists() {
        return Ok(path);
    }

    Err(anyhow!(
        "未找到 sing-box 可执行文件。请在设置页面下载。\n检查路径: {:?}",
        user_path
    ))
}

/// Fetch sing-box version
#[allow(dead_code)]
pub async fn get_version(binary_path: &std::path::Path) -> Result<String> {
    let mut cmd = TokioCommand::new(binary_path);
    cmd.arg("version");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let version = stdout.lines()
        .next()
        .unwrap_or("unknown")
        .to_string();
    Ok(version)
}

/// Start sing-box with the given config file
pub async fn start_singbox(
    app_handle: &tauri::AppHandle,
    config_path: &std::path::Path,
    state: SharedState,
) -> Result<()> {
    {
        let s = state.lock().unwrap();
        if s.running {
            return Err(anyhow!("sing-box 已在运行中"));
        }
    }

    let binary = singbox_binary_path(app_handle)?;
    let config_path = config_path.to_path_buf();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let mut cmd = TokioCommand::new(&binary);
        cmd.args(["run", "-c", config_path.to_str().unwrap_or("")])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let mut child = match cmd.spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::error!("启动 sing-box 失败: {}", e);
                return;
            }
        };

        let pid = child.id();
        {
            let mut s = state_clone.lock().unwrap();
            s.running = true;
            s.pid = pid;
            s.start_time = Some(Instant::now());
        }

        // Read stderr for logs
        if let Some(stderr) = child.stderr.take() {
            let state_log = state_clone.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let mut s = state_log.lock().unwrap();
                    s.logs.push(line.clone());
                    if s.logs.len() > 1000 {
                        s.logs.drain(0..100);
                    }
                }
            });
        }

        let _ = child.wait().await;
        let mut s = state_clone.lock().unwrap();
        s.running = false;
        s.pid = None;
        s.start_time = None;
    });

    // Wait briefly for startup
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(())
}

/// Stop sing-box
pub async fn stop_singbox(state: SharedState) -> Result<()> {
    let pid = {
        let s = state.lock().unwrap();
        s.pid
    };

    if let Some(pid) = pid {
        #[cfg(target_os = "windows")]
        {
            TokioCommand::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .await?;
        }
        #[cfg(not(target_os = "windows"))]
        {
            TokioCommand::new("kill")
                .args(["-SIGTERM", &pid.to_string()])
                .output()
                .await?;
        }
    }

    let mut s = state.lock().unwrap();
    s.running = false;
    s.pid = None;
    s.start_time = None;

    Ok(())
}

/// Fetch real-time stats from sing-box API (Clash API compatible)
#[allow(dead_code)]
pub async fn fetch_traffic_stats(api_port: u16) -> Result<crate::types::TrafficStats> {
    let url = format!("http://127.0.0.1:{}/traffic", api_port);
    let client = reqwest::Client::new();

    // The traffic endpoint is a streaming endpoint; for simplicity we use /memory
    let memory_url = format!("http://127.0.0.1:{}/memory", api_port);
    let resp = client.get(&memory_url)
        .timeout(Duration::from_secs(2))
        .send()
        .await?;
    let _body: Value = resp.json().await?;

    let _ = url;
    // Return placeholder — real traffic data comes via WebSocket stream
    Ok(crate::types::TrafficStats {
        upload_bytes: 0,
        download_bytes: 0,
        upload_speed: 0,
        download_speed: 0,
        connections: 0,
    })
}

/// Fetch connections from Clash API
pub async fn fetch_connections(api_port: u16) -> Result<Vec<crate::types::ConnectionInfo>> {
    let url = format!("http://127.0.0.1:{}/connections", api_port);
    let client = reqwest::Client::new();
    let resp = client.get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await?;
    let body: Value = resp.json().await?;

    let mut result = Vec::new();
    if let Some(connections) = body["connections"].as_array() {
        for c in connections {
            result.push(crate::types::ConnectionInfo {
                id: c["id"].as_str().unwrap_or("").to_string(),
                network: c["metadata"]["network"].as_str().unwrap_or("").to_string(),
                conn_type: c["metadata"]["type"].as_str().unwrap_or("").to_string(),
                source: format!(
                    "{}:{}",
                    c["metadata"]["sourceIP"].as_str().unwrap_or(""),
                    c["metadata"]["sourcePort"].as_str().unwrap_or("")
                ),
                destination: format!(
                    "{}:{}",
                    c["metadata"]["destinationIP"].as_str().unwrap_or(""),
                    c["metadata"]["destinationPort"].as_str().unwrap_or("")
                ),
                host: c["metadata"]["host"].as_str().unwrap_or("").to_string(),
                rule: c["rule"].as_str().unwrap_or("").to_string(),
                rule_payload: c["rulePayload"].as_str().unwrap_or("").to_string(),
                chains: c["chains"].as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default(),
                upload: c["upload"].as_u64().unwrap_or(0),
                download: c["download"].as_u64().unwrap_or(0),
                start: c["start"].as_str().unwrap_or("").to_string(),
            });
        }
    }

    Ok(result)
}
