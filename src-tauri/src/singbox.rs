use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::{Result, anyhow};
use serde_json::Value;
use tauri::{Manager, Emitter};
use tokio::process::Command as TokioCommand;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Open today's rolling log file in append mode under `app_data_dir/logs/`. Returns
/// `None` if the directory or file cannot be created (logging then stays in-memory only).
fn open_daily_log_file() -> Option<std::fs::File> {
    let dir = crate::config::app_data_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("skylark-{}.log", chrono::Local::now().format("%Y%m%d")));
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Windows CREATE_NO_WINDOW flag: prevents a console window from popping up
/// when spawning child processes (sing-box, taskkill).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Ask sing-box to shut down **gracefully** by delivering a console Ctrl+C, which Go maps
/// to `os.Interrupt`/SIGINT — triggering sing-box's own shutdown path, including TUN
/// teardown (`WintunDeleteAdapter`) and `strict_route` route cleanup. A hard `taskkill /F`
/// skips all of that and orphans the TUN adapter + routes, which later surfaces as
/// "TUN on, but no network" after a restart.
///
/// How it works: the core is spawned with `CREATE_NO_WINDOW`, so it owns a hidden console
/// we can attach to. Because we attach to that console, the broadcast Ctrl+C would also hit
/// this GUI process — so we first install a NULL "ignore" handler on ourselves, fire
/// `CTRL_C_EVENT` to the whole attached console group, then detach and restore our handler.
///
/// IMPORTANT: the core must NOT be started with `CREATE_NEW_PROCESS_GROUP`, since new
/// process groups have Ctrl+C disabled by default — it is intentionally absent at spawn.
///
/// Returns `false` if attaching/sending failed (process already gone, or no console), in
/// which case the caller should fall back to a force kill.
#[cfg(target_os = "windows")]
fn send_ctrl_c(pid: u32) -> bool {
    use winapi::um::consoleapi::SetConsoleCtrlHandler;
    use winapi::um::wincon::{AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, CTRL_C_EVENT};

    unsafe {
        // Detach from any console we might already own; a GUI build normally has none, and
        // AttachConsole fails with ERROR_ACCESS_DENIED if we are already attached elsewhere.
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return false;
        }
        // Disable Ctrl+C handling for OURSELVES so the broadcast below doesn't terminate the
        // GUI process along with the core.
        SetConsoleCtrlHandler(None, 1);
        let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) != 0;
        // Leave the core's console and re-enable normal Ctrl handling for ourselves.
        FreeConsole();
        SetConsoleCtrlHandler(None, 0);
        sent
    }
}

#[derive(Debug, Clone)]
pub struct SingboxState {
    pub running: bool,
    pub pid: Option<u32>,
    pub start_time: Option<Instant>,
    pub version: Option<String>,
    pub logs: Vec<String>,
    /// Whether the currently-running core was started with the TUN inbound. Used to
    /// decide whether a mode switch needs a config rebuild + restart (TUN) or can be
    /// applied without touching the core (system-proxy toggle on a persistent core).
    pub tun_mode: bool,
}

impl Default for SingboxState {
    fn default() -> Self {
        Self {
            running: false,
            pid: None,
            start_time: None,
            version: None,
            logs: Vec::new(),
            tun_mode: false,
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

/// Kill any orphaned sing-box.exe processes left over from a previous run or app update.
/// Uses process-name kill (not PID) so it catches instances started by any previous app version.
///
/// Only sleeps to let the OS release the bound ports when an orphan was actually found
/// and killed — the common case (no orphan) returns immediately, saving ~400ms on every start.
#[cfg(target_os = "windows")]
async fn kill_orphan_singbox() {
    // taskkill /F /IM sing-box.exe exits 0 when it killed at least one process,
    // and non-zero (128) when no matching process exists.
    let killed = TokioCommand::new("taskkill")
        .args(["/F", "/IM", "sing-box.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if killed {
        // Give the OS a moment to fully release the bound ports before we rebind.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[cfg(not(target_os = "windows"))]
async fn kill_orphan_singbox() {
    // Match the core's invocation signature ("…/sing-box run -c …") rather than the bare
    // name. The core is the only process ever launched with `run -c`, so this pattern
    // targets exactly the orphaned core and never the GUI app itself.
    let killed = TokioCommand::new("pkill")
        .args(["-f", "sing-box run -c"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    // On macOS the previous run's TUN core was root-owned, so the user-level pkill above
    // can't reap it. Clear it through the passwordless rule (no-op when the service isn't
    // installed, or when no such orphan exists). Best-effort; failure is ignored.
    #[cfg(target_os = "macos")]
    {
        let _ = TokioCommand::new("sudo")
            .args(["-n", "/usr/bin/pkill", "-KILL", "-f", crate::tun::TUN_ROOT_BIN])
            .output()
            .await;
    }
    if killed {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Poll the Clash API control port until it accepts a TCP connection, signalling that
/// sing-box has finished starting up. Returns as soon as the port is reachable (typically
/// ~100-200ms) instead of blocking on a fixed delay. Caps out after ~2s so a config that
/// never binds the port doesn't hang the caller indefinitely.
async fn wait_until_ready(api_port: u16) -> bool {
    let addr = format!("127.0.0.1:{}", api_port);
    for _ in 0..40 {
        if tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Start sing-box with the given config file
pub async fn start_singbox(
    app_handle: &tauri::AppHandle,
    config_path: &std::path::Path,
    state: SharedState,
    api_port: u16,
    tun_mode: bool,
) -> Result<()> {
    {
        let s = state.lock().unwrap();
        if s.running {
            return Err(anyhow!("sing-box 已在运行中"));
        }
    }

    // Kill any leftover sing-box processes (e.g. from app update / crash).
    // This prevents "address already in use" errors on the mixed/http/socks ports.
    kill_orphan_singbox().await;

    let binary = singbox_binary_path(app_handle)?;
    let config_path = config_path.to_path_buf();
    let state_clone = state.clone();
    let app_log = app_handle.clone();

    tokio::spawn(async move {
        let cfg_arg = config_path.to_str().unwrap_or("");
        // On macOS, TUN needs root. We launch the ROOT-OWNED core via passwordless `sudo -n`
        // (set up once by tun::install_tun_service) instead of running the whole GUI as root.
        // stderr is still piped straight to us, so the log view keeps working. Non-TUN runs,
        // and all runs on Windows/Linux, spawn the user-owned binary directly as before.
        #[cfg(target_os = "macos")]
        let mut cmd = if tun_mode {
            let mut c = TokioCommand::new("sudo");
            c.args(["-n", crate::tun::TUN_ROOT_BIN, "run", "-c", cfg_arg]);
            c
        } else {
            let mut c = TokioCommand::new(&binary);
            c.args(["run", "-c", cfg_arg]);
            c
        };
        #[cfg(not(target_os = "macos"))]
        let mut cmd = {
            let mut c = TokioCommand::new(&binary);
            c.args(["run", "-c", cfg_arg]);
            c
        };
        // Pin the working directory to the writable app data dir. A GUI app launched
        // from /Applications inherits cwd `/` (read-only on macOS), so any config field
        // that resolves a relative path — cache_file's db, external_ui — would otherwise
        // fail there. The config already passes absolute paths, but setting cwd makes the
        // core robust against any relative default sing-box might use.
        cmd.current_dir(crate::config::app_data_dir())
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
            s.tun_mode = tun_mode;
        }

        // Read stderr for logs. Each line is (a) appended to the in-memory ring buffer,
        // (b) optionally appended to today's rolling log file (crash-safe persistence),
        // and (c) pushed to the UI via a `singbox-log` event so the frontend does not
        // have to poll and re-clone the whole buffer every second.
        if let Some(stderr) = child.stderr.take() {
            let state_log = state_clone.clone();
            let app_log = app_log.clone();
            tokio::spawn(async move {
                // Read the persistence flag once at start; a runtime toggle takes effect
                // on the next core (re)start, which is acceptable for this setting.
                let mut log_file = if crate::config::load_app_config().log_to_file {
                    open_daily_log_file()
                } else {
                    None
                };
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if let Some(f) = log_file.as_mut() {
                        use std::io::Write;
                        let _ = writeln!(f, "{}", line);
                    }
                    {
                        let mut s = state_log.lock().unwrap();
                        s.logs.push(line.clone());
                        if s.logs.len() > 1000 {
                            s.logs.drain(0..100);
                        }
                    }
                    let _ = app_log.emit("singbox-log", line);
                }
            });
        }

        let _ = child.wait().await;
        let mut s = state_clone.lock().unwrap();
        s.running = false;
        s.pid = None;
        s.start_time = None;
        s.tun_mode = false;
    });

    // Confirm the core actually came up: the Clash API control port must accept a
    // connection. If it never binds — invalid config, the port still held by an orphan
    // right after an app upgrade, or TUN failing without admin rights — the process has
    // effectively failed even though `spawn()` succeeded. Returning Err here is critical:
    // otherwise the caller (apply_connection_mode) would enable the system proxy / treat
    // TUN as active on top of a DEAD core, which presents as "proxy on, but no network".
    if !wait_until_ready(api_port).await {
        kill_orphan_singbox().await;
        {
            let mut s = state.lock().unwrap();
            s.running = false;
            s.pid = None;
            s.start_time = None;
            s.tun_mode = false;
        }
        return Err(anyhow!(
            "sing-box 启动失败：控制端口未就绪（配置无效 / 端口被占用 / TUN 需要管理员权限）"
        ));
    }

    Ok(())
}

/// Stop sing-box.
///
/// `graceful` should be true only when the running instance is in TUN mode: a graceful
/// shutdown gives sing-box time to call WintunDeleteAdapter() and tear down the TUN driver
/// cleanly. For plain system-proxy / mixed-inbound runs there is no adapter to clean up, so
/// we force-kill immediately — that path is near-instant.
///
/// Liveness is detected by polling the in-memory `running` flag, which the background waiter
/// task flips to false the moment `child.wait()` returns. This avoids spawning a slow
/// `tasklist` process on every poll (the old approach cost up to ~3s).
pub async fn stop_singbox(state: SharedState, graceful: bool) -> Result<()> {
    let pid = {
        let s = state.lock().unwrap();
        s.pid
    };

    if let Some(pid) = pid {
        #[cfg(target_os = "windows")]
        {
            let mut exited = false;

            if graceful {
                // Graceful shutdown: deliver a real Ctrl+C (→ SIGINT) so sing-box runs its
                // own TUN teardown (WintunDeleteAdapter) and strict_route cleanup before
                // exiting — the previous `taskkill` (no /F) sent WM_CLOSE, which a windowless
                // console process never receives, so it always fell through to a force kill
                // and orphaned the TUN adapter/routes.
                if send_ctrl_c(pid) {
                    // TUN driver teardown can take a beat; poll the shared state (no process
                    // spawn) up to ~3s, returning as soon as the core has actually exited.
                    for _ in 0..60 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        if !state.lock().unwrap().running {
                            exited = true;
                            break;
                        }
                    }
                }
            }

            // Force-kill if not already gone (or whenever a graceful wait wasn't requested).
            if !exited {
                let _ = TokioCommand::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .await;
            }
        }
        // macOS: a TUN core runs as root (via sudo), so a direct `kill` from the non-root
        // GUI would be denied — and `pid` is sudo's, not sing-box's. Signal it through the
        // passwordless pkill rule instead. A non-TUN core runs as the user, so kill its pid.
        #[cfg(target_os = "macos")]
        {
            if graceful {
                // SIGTERM lets sing-box tear the utun device + auto_route down cleanly.
                let _ = TokioCommand::new("sudo")
                    .args(["-n", "/usr/bin/pkill", "-TERM", "-f", crate::tun::TUN_ROOT_BIN])
                    .output()
                    .await;
                let mut exited = false;
                for _ in 0..60 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if !state.lock().unwrap().running {
                        exited = true;
                        break;
                    }
                }
                if !exited {
                    let _ = TokioCommand::new("sudo")
                        .args(["-n", "/usr/bin/pkill", "-KILL", "-f", crate::tun::TUN_ROOT_BIN])
                        .output()
                        .await;
                }
            } else {
                let _ = TokioCommand::new("kill")
                    .args(["-SIGKILL", &pid.to_string()])
                    .output()
                    .await;
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let signal = if graceful { "-SIGTERM" } else { "-SIGKILL" };
            let _ = TokioCommand::new("kill")
                .args([signal, &pid.to_string()])
                .output()
                .await;
            if graceful {
                for _ in 0..30 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if !state.lock().unwrap().running {
                        break;
                    }
                }
            }
        }
    }

    let mut s = state.lock().unwrap();
    s.running = false;
    s.pid = None;
    s.start_time = None;

    Ok(())
}


/// Fetch connections from Clash API
pub async fn fetch_connections(api_port: u16) -> Result<Vec<crate::types::ConnectionInfo>> {
    let url = format!("http://127.0.0.1:{}/connections", api_port);
    let client = reqwest::Client::new();
    let resp = client.get(&url)
        .bearer_auth(crate::config::api_secret())
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

/// Close a single active connection via the Clash API (`DELETE /connections/{id}`).
pub async fn close_connection(api_port: u16, id: &str) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/connections/{}", api_port, id);
    let client = reqwest::Client::new();
    client.delete(&url)
        .bearer_auth(crate::config::api_secret())
        .timeout(Duration::from_secs(3))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Close all active connections via the Clash API (`DELETE /connections`).
pub async fn close_all_connections(api_port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/connections", api_port);
    let client = reqwest::Client::new();
    client.delete(&url)
        .bearer_auth(crate::config::api_secret())
        .timeout(Duration::from_secs(3))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
