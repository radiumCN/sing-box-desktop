use std::time::Duration;
use tauri::Emitter;

/// Background task: check for sing-box updates on startup and periodically.
/// Emits "singbox-update-available" { version, download_url, release_notes } when a new version is found.
pub async fn start_auto_update_checker(app_handle: tauri::AppHandle, interval_hours: u32) {
    // First check: 30 seconds after launch (let app settle)
    tokio::time::sleep(Duration::from_secs(30)).await;
    check_and_emit(&app_handle).await;

    if interval_hours == 0 {
        return;
    }

    let interval = Duration::from_secs(interval_hours as u64 * 3600);
    loop {
        tokio::time::sleep(interval).await;
        check_and_emit(&app_handle).await;
    }
}

async fn check_and_emit(app_handle: &tauri::AppHandle) {
    match crate::updater::fetch_latest_release().await {
        Ok(release) => {
            let installed = crate::updater::get_installed_version().await;
            let installed_ver = installed
                .as_deref()
                .and_then(|s| s.split_whitespace().last())
                .unwrap_or("");
            let latest_ver = release.version.trim_start_matches('v');

            if !installed_ver.is_empty() && installed_ver != latest_ver {
                let _ = app_handle.emit("singbox-update-available", serde_json::json!({
                    "version": release.version,
                    "download_url": release.download_url,
                    "release_notes": release.release_notes,
                    "installed_version": installed_ver,
                }));
            } else if installed_ver.is_empty() {
                // Not installed at all — notify frontend
                let _ = app_handle.emit("singbox-not-installed", serde_json::json!({
                    "latest_version": release.version,
                    "download_url": release.download_url,
                }));
            }
        }
        Err(e) => {
            log::warn!("自动更新检查失败: {}", e);
        }
    }
}
