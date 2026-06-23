use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;

const GITHUB_API: &str = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";
const APP_GITHUB_STABLE_API: &str =
    "https://api.github.com/repos/radiumCN/sing-box-desktop/releases/latest";
const APP_GITHUB_ALL_API: &str =
    "https://api.github.com/repos/radiumCN/sing-box-desktop/releases";
/// Cache validity duration in seconds (1 hour)
const CACHE_TTL_SECS: u64 = 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub published_at: String,
    pub release_notes: String,
    pub download_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    cached_at_secs: u64,
    release: ReleaseInfo,
}

fn cache_path() -> PathBuf {
    crate::config::app_data_dir().join("update_cache.json")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_cache() -> Option<ReleaseInfo> {
    let data = std::fs::read_to_string(cache_path()).ok()?;
    let cache: UpdateCache = serde_json::from_str(&data).ok()?;
    if unix_now().saturating_sub(cache.cached_at_secs) < CACHE_TTL_SECS {
        Some(cache.release)
    } else {
        None
    }
}

fn save_cache(release: &ReleaseInfo) {
    let cache = UpdateCache {
        cached_at_secs: unix_now(),
        release: release.clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(cache_path(), data);
    }
}

/// Query latest sing-box release from GitHub, with 1-hour local cache.
/// Pass `force_refresh = true` to bypass the cache and always hit the API.
pub async fn fetch_latest_release(force_refresh: bool) -> Result<ReleaseInfo> {
    // Return cached result if still fresh
    if !force_refresh {
        if let Some(cached) = load_cache() {
            return Ok(cached);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("sing-box-win/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = client.get(GITHUB_API).send().await?;

    // Detect rate limit (HTTP 403 or 429)
    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        // Try to read reset time from headers
        let reset_hint = resp
            .headers()
            .get("X-RateLimit-Reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|reset_ts| {
                let wait = reset_ts.saturating_sub(unix_now());
                let mins = wait / 60;
                if mins > 0 {
                    format!("，请 {} 分钟后重试", mins)
                } else {
                    "，请稍后重试".to_string()
                }
            })
            .unwrap_or_else(|| "，请稍后重试".to_string());

        return Err(anyhow!(
            "GitHub API 请求频率超限（未认证 IP 每小时限 60 次）{}",
            reset_hint
        ));
    }

    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API 请求失败: HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().await?;

    // Check for GitHub error message in body
    if let Some(msg) = body["message"].as_str() {
        if msg.to_lowercase().contains("rate limit") {
            return Err(anyhow!(
                "GitHub API 请求频率超限（未认证 IP 每小时限 60 次），请稍后重试"
            ));
        }
        return Err(anyhow!("GitHub API 错误: {}", msg));
    }

    let version = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("无法解析版本号"))?
        .to_string();

    let published_at = body["published_at"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let release_notes = body["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .take(10)
        .collect::<Vec<_>>()
        .join("\n");

    // Find Windows amd64 asset
    let download_url = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("未找到下载资源"))?
        .iter()
        .find(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            name.contains("windows") && name.contains("amd64") && name.ends_with(".zip")
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| anyhow!("未找到 Windows x64 下载链接"))?
        .to_string();

    let release = ReleaseInfo {
        version,
        published_at,
        release_notes,
        download_url,
    };

    save_cache(&release);
    Ok(release)
}

/// Download and install sing-box binary with progress events
/// Emits: "singbox-download-progress" { percent: f64, downloaded: u64, total: u64 }
/// Emits: "singbox-download-done" { success: bool, message: String }
pub async fn download_singbox(
    app_handle: tauri::AppHandle,
    download_url: String,
    dest_path: PathBuf,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent(concat!("sing-box-win/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("下载失败: HTTP {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    // Ensure destination directory exists before writing
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| anyhow!("无法创建目录 {:?}: {}", parent, e))?;
    }

    // Download to a temp zip file in the same directory
    let zip_path = dest_path.parent()
        .unwrap_or(std::path::Path::new("."))
        .join("sing-box-download.zip");

    let mut file = tokio::fs::File::create(&zip_path).await
        .map_err(|e| anyhow!("无法创建临时文件 {:?}: {}", zip_path, e))?;
    let mut stream = resp.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("下载中断: {}", e))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let _ = app_handle.emit("singbox-download-progress", serde_json::json!({
            "percent": percent,
            "downloaded": downloaded,
            "total": total,
        }));
    }
    file.flush().await?;
    drop(file);

    // Extract sing-box.exe from zip
    let zip_data = std::fs::read(&zip_path)?;
    extract_exe_from_zip(&zip_data, &dest_path)?;
    let _ = std::fs::remove_file(&zip_path);

    let _ = app_handle.emit("singbox-download-done", serde_json::json!({
        "success": true,
        "message": "下载完成",
    }));

    Ok(())
}

fn extract_exe_from_zip(zip_data: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::{Cursor, Read};
    let cursor = Cursor::new(zip_data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow!("ZIP 解压失败: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_lowercase();
        if name.ends_with("sing-box.exe") || name == "sing-box.exe" {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // If sing-box is running, write to temp file then rename
            let tmp = dest.with_extension("exe.tmp");
            let mut out = std::fs::File::create(&tmp)?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            std::io::Write::write_all(&mut out, &buf)?;
            drop(out);
            std::fs::rename(&tmp, dest)?;
            return Ok(());
        }
    }

    Err(anyhow!("ZIP 包中未找到 sing-box.exe"))
}

/// Get sing-box binary path
pub fn singbox_binary_path() -> PathBuf {
    crate::config::app_data_dir().join("bin").join("sing-box.exe")
}

/// Check if sing-box binary exists
pub fn singbox_exists() -> bool {
    singbox_binary_path().exists()
}

/// Get current installed version by running sing-box version
pub async fn get_installed_version() -> Option<String> {
    let path = singbox_binary_path();
    if !path.exists() {
        return None;
    }
    let mut cmd = tokio::process::Command::new(&path);
    cmd.arg("version");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let output = cmd
        .output()
        .await
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Typical output: "sing-box version 1.10.0"
    stdout.lines().next().map(|s| s.to_string())
}

// ─── App Self-Updater ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppReleaseInfo {
    pub version: String,
    pub published_at: String,
    pub release_notes: String,
    pub download_url: String,
    pub is_prerelease: bool,
}

fn app_cache_path(channel: &str) -> PathBuf {
    crate::config::app_data_dir().join(format!("app_update_cache_{}.json", channel))
}

#[derive(Debug, Serialize, Deserialize)]
struct AppUpdateCache {
    cached_at_secs: u64,
    release: AppReleaseInfo,
}

fn load_app_cache(channel: &str) -> Option<AppReleaseInfo> {
    let data = std::fs::read_to_string(app_cache_path(channel)).ok()?;
    let cache: AppUpdateCache = serde_json::from_str(&data).ok()?;
    if unix_now().saturating_sub(cache.cached_at_secs) < CACHE_TTL_SECS {
        Some(cache.release)
    } else {
        None
    }
}

fn save_app_cache(channel: &str, release: &AppReleaseInfo) {
    let cache = AppUpdateCache {
        cached_at_secs: unix_now(),
        release: release.clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(app_cache_path(channel), data);
    }
}

fn parse_app_release(body: &serde_json::Value) -> Result<AppReleaseInfo> {
    let version = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("无法解析版本号"))?
        .to_string();

    let published_at = body["published_at"].as_str().unwrap_or("").to_string();
    let is_prerelease = body["prerelease"].as_bool().unwrap_or(false);

    let release_notes = body["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .take(12)
        .collect::<Vec<_>>()
        .join("\n");

    // Find Windows x64 installer asset (.exe)
    let download_url = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("未找到下载资源"))?
        .iter()
        .find(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            name.ends_with(".exe") && (name.contains("x64") || name.contains("setup") || name.contains("install"))
        })
        .or_else(|| {
            // fallback: first .exe asset
            body["assets"].as_array().and_then(|arr| {
                arr.iter().find(|a| {
                    a["name"].as_str().unwrap_or("").to_lowercase().ends_with(".exe")
                })
            })
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| anyhow!("未找到 Windows 安装包(.exe)"))?
        .to_string();

    Ok(AppReleaseInfo { version, published_at, release_notes, download_url, is_prerelease })
}

/// Fetch the latest app release for the given channel.
/// channel = "stable" → non-prerelease only; channel = "beta" → includes pre-release
pub async fn fetch_app_release(channel: &str, force_refresh: bool) -> Result<AppReleaseInfo> {
    if !force_refresh {
        if let Some(cached) = load_app_cache(channel) {
            return Ok(cached);
        }
    }

    // Bypass system proxy (which points to sing-box itself) to avoid circular dependency.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("sing-box-win/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let url = if channel == "beta" {
        APP_GITHUB_ALL_API
    } else {
        APP_GITHUB_STABLE_API
    };

    let resp = client.get(url).send().await?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(anyhow!("GitHub API 请求频率超限，请稍后重试"));
    }

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(anyhow!("暂无可用版本，请稍后再试"));
    }

    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API 请求失败: HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().await?;

    // For beta channel the API returns an array; pick the first (most recent) release
    let release_body = if channel == "beta" {
        let arr = body.as_array().ok_or_else(|| anyhow!("无法解析版本列表"))?;
        if arr.is_empty() {
            return Err(anyhow!("暂无可用版本"));
        }
        arr[0].clone()
    } else {
        body
    };

    if let Some(msg) = release_body["message"].as_str() {
        return Err(anyhow!("GitHub API 错误: {}", msg));
    }

    let release = parse_app_release(&release_body)?;
    save_app_cache(channel, &release);
    Ok(release)
}

/// Download app installer to temp directory and launch it.
/// Emits: "app-download-progress" { percent, downloaded, total }
/// Emits: "app-download-done"     { success, message }
pub async fn download_and_install_app(
    app_handle: tauri::AppHandle,
    download_url: String,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    // Bypass system proxy (which points to sing-box itself) to avoid circular dependency.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent(concat!("sing-box-win/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("下载失败: HTTP {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    let temp_dir = std::env::temp_dir();
    let file_name = download_url
        .split('/')
        .last()
        .unwrap_or("sing-box-win-setup.exe")
        .to_string();
    let installer_path = temp_dir.join(&file_name);

    let mut file = tokio::fs::File::create(&installer_path).await
        .map_err(|e| anyhow!("无法创建临时文件: {}", e))?;

    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("下载中断: {}", e))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let _ = app_handle.emit("app-download-progress", serde_json::json!({
            "percent": percent,
            "downloaded": downloaded,
            "total": total,
        }));
    }
    file.flush().await?;
    drop(file);

    let _ = app_handle.emit("app-download-done", serde_json::json!({
        "success": true,
        "message": "下载完成，即将启动安装程序",
        "path": installer_path.to_string_lossy(),
    }));

    // Launch installer — the NSIS installer handles closing and restarting the app
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", installer_path.to_str().unwrap_or("")])
            .spawn()
            .map_err(|e| anyhow!("无法启动安装程序: {}", e))?;
    }

    Ok(())
}
