use std::time::Duration;
use tauri::{Emitter, Manager};

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

// ─── Subscription Auto-Updater ───────────────────────────────────────

/// Background task: periodically refresh subscriptions that have auto_update enabled.
/// Checks every 30 minutes; updates any subscription whose last_update is older than its interval.
pub async fn start_subscription_auto_updater(app_handle: tauri::AppHandle) {
    // Wait for app to settle before first check
    tokio::time::sleep(Duration::from_secs(60)).await;

    loop {
        let state = app_handle.state::<crate::commands::AppState>();

        let to_update: Vec<(String, String)> = {
            let subs = state.subscriptions.lock().unwrap();
            let now = chrono::Utc::now();
            subs.iter()
                .filter(|s| s.auto_update && s.update_interval > 0)
                .filter(|s| match s.last_update {
                    None => true,
                    Some(last) => (now - last).num_hours() >= s.update_interval as i64,
                })
                .map(|s| (s.id.clone(), s.url.clone()))
                .collect()
        };

        for (id, url) in to_update {
            match do_update_subscription(&app_handle, &id, &url).await {
                Ok(_) => log::info!("订阅自动更新成功: {}", id),
                Err(e) => log::warn!("订阅自动更新失败 [{}]: {}", id, e),
            }
        }

        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    }
}

async fn do_update_subscription(
    app_handle: &tauri::AppHandle,
    id: &str,
    url: &str,
) -> anyhow::Result<()> {
    let content = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("ClashForWindows/0.20.39")
        .build()?
        .get(url)
        .send()
        .await?
        .text()
        .await?;

    crate::config::save_subscription_content(id, &content)?;

    let state = app_handle.state::<crate::commands::AppState>();
    let sub_type = crate::subscription::detect_sub_type(&content, url);
    let (nodes, outbounds) = crate::subscription::parse_subscription(&content, id)?;

    {
        let mut subs = state.subscriptions.lock().unwrap();
        if let Some(sub) = subs.iter_mut().find(|s| s.id == id) {
            sub.sub_type = sub_type;
            sub.node_count = nodes.len();
            sub.last_update = Some(chrono::Utc::now());
        }
        crate::config::save_subscriptions(&subs)?;
    }
    {
        let mut all_nodes = state.nodes.lock().unwrap();
        all_nodes.retain(|n| n.subscription_id.as_deref() != Some(id));
        all_nodes.extend(nodes);
        crate::config::save_nodes(&all_nodes)?;
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
        crate::config::save_outbounds(&all_outbounds)?;
    }

    Ok(())
}

// ─── App Self-Update Checker ─────────────────────────────────────────

/// Background task: check for app updates once at startup (after a short delay).
/// Emits "app-update-available" { version, download_url, release_notes, is_prerelease }
/// when the latest release version differs from the running app version.
pub async fn start_app_update_checker(app_handle: tauri::AppHandle) {
    // Wait 45 seconds after launch so the window is fully up before showing anything
    tokio::time::sleep(std::time::Duration::from_secs(45)).await;

    let channel = {
        let cfg = crate::config::load_app_config();
        cfg.update_channel.clone()
    };

    match crate::updater::fetch_app_release(&channel, false).await {
        Ok(release) => {
            let current = env!("CARGO_PKG_VERSION");
            let latest = release.version.trim_start_matches('v');
            if latest != current {
                let _ = app_handle.emit("app-update-available", serde_json::json!({
                    "version": release.version,
                    "download_url": release.download_url,
                    "release_notes": release.release_notes,
                    "published_at": release.published_at,
                    "is_prerelease": release.is_prerelease,
                    "current_version": current,
                }));
            }
        }
        Err(e) => {
            log::debug!("应用更新检查失败: {}", e);
        }
    }
}

// ─── sing-box Binary Updater ─────────────────────────────────────────

async fn check_and_emit(app_handle: &tauri::AppHandle) {
    match crate::updater::fetch_latest_release(false).await {
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
