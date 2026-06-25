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

/// The sing-box release asset matching the platform we are currently running on.
/// sing-box publishes one archive per OS/arch on its GitHub releases, e.g.
///   sing-box-1.x.y-windows-amd64.zip   → sing-box.exe
///   sing-box-1.x.y-darwin-amd64.tar.gz → sing-box   (macOS Intel)
///   sing-box-1.x.y-darwin-arm64.tar.gz → sing-box   (macOS Apple Silicon)
///   sing-box-1.x.y-linux-amd64.tar.gz  → sing-box
struct PlatformAsset {
    /// OS keyword used in the asset file name.
    os: &'static str,
    /// CPU arch keyword used in the asset file name (sing-box uses Go names).
    arch: &'static str,
    /// Archive file extension, including the leading dot.
    ext: &'static str,
}

fn current_platform_asset() -> PlatformAsset {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        // x86_64 → amd64 (the only other arch we ship)
        "amd64"
    };
    let ext = if cfg!(target_os = "windows") { ".zip" } else { ".tar.gz" };
    PlatformAsset { os, arch, ext }
}

/// File name of the sing-box executable for the current platform.
fn singbox_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sing-box.exe"
    } else {
        "sing-box"
    }
}

/// On Unix, mark the freshly-extracted binary as executable (0755).
fn finalize_binary(_dest: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(_dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(_dest, perms)?;
    }
    Ok(())
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

    // Find the asset matching the current OS + CPU architecture.
    let p = current_platform_asset();
    let download_url = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("未找到下载资源"))?
        .iter()
        .find(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            name.contains(p.os) && name.contains(p.arch) && name.ends_with(p.ext)
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| anyhow!("未找到 {}-{} 平台的下载链接", p.os, p.arch))?
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

    // Download to a temp archive file in the same directory. The extension is
    // inferred from the download URL so we know how to extract it (.zip vs .tar.gz).
    let is_tar_gz = download_url.to_lowercase().ends_with(".tar.gz");
    let archive_name = if is_tar_gz {
        "sing-box-download.tar.gz"
    } else {
        "sing-box-download.zip"
    };
    let archive_path = dest_path.parent()
        .unwrap_or(std::path::Path::new("."))
        .join(archive_name);

    let mut file = tokio::fs::File::create(&archive_path).await
        .map_err(|e| anyhow!("无法创建临时文件 {:?}: {}", archive_path, e))?;
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

    // Extract the sing-box binary from the archive.
    let archive_data = std::fs::read(&archive_path)?;
    if is_tar_gz {
        extract_binary_from_tar_gz(&archive_data, &dest_path)?;
    } else {
        extract_binary_from_zip(&archive_data, &dest_path)?;
    }
    let _ = std::fs::remove_file(&archive_path);

    // Ensure the extracted binary is executable on Unix.
    finalize_binary(&dest_path)?;

    let _ = app_handle.emit("singbox-download-done", serde_json::json!({
        "success": true,
        "message": "下载完成",
    }));

    Ok(())
}

/// Write extracted bytes to `dest`, going through a temp file + rename so an
/// already-running sing-box binary doesn't block the write.
fn write_binary(dest: &PathBuf, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("tmp");
    let mut out = std::fs::File::create(&tmp)?;
    out.write_all(bytes)?;
    drop(out);
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Extract the sing-box executable from a Windows `.zip` archive.
fn extract_binary_from_zip(zip_data: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::{Cursor, Read};
    let target = singbox_binary_name().to_lowercase();
    let cursor = Cursor::new(zip_data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow!("ZIP 解压失败: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_lowercase();
        if name.ends_with(target.as_str()) {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            write_binary(dest, &buf)?;
            return Ok(());
        }
    }

    Err(anyhow!("压缩包中未找到 {}", singbox_binary_name()))
}

/// Extract the sing-box executable from a macOS/Linux `.tar.gz` archive.
fn extract_binary_from_tar_gz(data: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::{Cursor, Read};
    use flate2::read::GzDecoder;

    let target = singbox_binary_name();
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().map_err(|e| anyhow!("tar.gz 解压失败: {}", e))? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let is_match = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == target)
            .unwrap_or(false);
        if is_match {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_binary(dest, &buf)?;
            return Ok(());
        }
    }

    Err(anyhow!("压缩包中未找到 {}", target))
}

/// Get sing-box binary path
pub fn singbox_binary_path() -> PathBuf {
    crate::config::app_data_dir().join("bin").join(singbox_binary_name())
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

/// Pick the app installer asset for the current platform from a release's asset list.
/// Windows → `.exe`; macOS → `.dmg` (prefer arch-matched, then universal).
fn find_app_installer_url(assets: &[serde_json::Value]) -> Option<String> {
    let url_of = |a: &serde_json::Value| {
        a["browser_download_url"].as_str().map(|s| s.to_string())
    };

    #[cfg(target_os = "windows")]
    {
        // Prefer a clearly-named x64 installer, then fall back to any .exe.
        assets
            .iter()
            .find(|a| {
                let name = a["name"].as_str().unwrap_or("").to_lowercase();
                name.ends_with(".exe")
                    && (name.contains("x64") || name.contains("setup") || name.contains("install"))
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    a["name"].as_str().unwrap_or("").to_lowercase().ends_with(".exe")
                })
            })
            .and_then(url_of)
    }

    #[cfg(target_os = "macos")]
    {
        // sing-box-desktop releases ship per-arch .dmg files; match the running arch.
        let arch_kw = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x64" };
        let arch_alt = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
        assets
            .iter()
            .find(|a| {
                let name = a["name"].as_str().unwrap_or("").to_lowercase();
                name.ends_with(".dmg") && (name.contains(arch_kw) || name.contains(arch_alt))
            })
            .or_else(|| {
                // Universal or single-arch build.
                assets.iter().find(|a| {
                    a["name"].as_str().unwrap_or("").to_lowercase().ends_with(".dmg")
                })
            })
            .and_then(url_of)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = url_of;
        assets.iter().find_map(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            if name.ends_with(".appimage") || name.ends_with(".deb") {
                a["browser_download_url"].as_str().map(|s| s.to_string())
            } else {
                None
            }
        })
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

    let assets = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("未找到下载资源"))?;

    // Find the installer asset matching the current platform.
    let download_url = find_app_installer_url(assets)
        .ok_or_else(|| anyhow!("未找到当前平台的安装包"))?;

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

    // Cleanly tear down BEFORE the installer restarts the app. The NSIS installer
    // force-terminates the running process, which bypasses the window-close / tray-quit
    // handlers that normally run shutdown_core. Without this, the old core would be left
    // orphaned (still holding the mixed/API port and TUN adapter) and the Windows system
    // proxy would stay pointing at it — so the upgraded app restores "proxy on" on top of
    // a stale/dead core and ends up with no network. Stopping here clears both.
    {
        use tauri::Manager;
        let state = app_handle.state::<crate::commands::AppState>();
        crate::commands::shutdown_core(state.inner()).await;
    }

    // Launch the installer.
    // Windows: the NSIS installer handles closing and restarting the app.
    // Use CREATE_NO_WINDOW so the helper `cmd` doesn't flash a black console window.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", installer_path.to_str().unwrap_or("")])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| anyhow!("无法启动安装程序: {}", e))?;
    }

    // macOS: open the downloaded .dmg in Finder so the user can drag-install.
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| anyhow!("无法打开安装包: {}", e))?;
    }

    // Linux: open the downloaded package with the default handler.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| anyhow!("无法打开安装包: {}", e))?;
    }

    Ok(())
}
