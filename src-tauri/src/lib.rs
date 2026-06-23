mod types;
mod config;
mod subscription;
mod singbox;
mod proxy;
mod updater;
mod tun;
mod rules;
mod auto_update;
mod commands;

use std::sync::Mutex;
use tauri::Manager;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use commands::{AppState, TrayState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    config::ensure_dirs().expect("无法创建数据目录");

    let app_config = config::load_app_config();
    let subscriptions = config::load_subscriptions();
    let mut nodes = config::load_nodes();
    let mut outbounds = config::load_outbounds();

    // Restore is_active flag on nodes from the persisted active_nodes config.
    // This ensures that whichever node was active (including auto-selected) survives a restart.
    if let Some(active_tag) = app_config.active_nodes.get("proxy").cloned() {
        for node in nodes.iter_mut() {
            node.is_active = node.name == active_tag;
        }
    }

    // If nodes are empty but subscriptions exist with cached content, re-parse
    if nodes.is_empty() && !subscriptions.is_empty() {
        let mut reparsed_nodes: Vec<types::ProxyNode> = Vec::new();
        let mut reparsed_outbounds: Vec<serde_json::Value> = Vec::new();
        for sub in &subscriptions {
            if let Some(content) = config::load_subscription_content(&sub.id) {
                if let Ok((ns, obs)) = subscription::parse_subscription(&content, &sub.id) {
                    reparsed_nodes.extend(ns);
                    reparsed_outbounds.extend(obs);
                }
            }
        }
        if !reparsed_nodes.is_empty() {
            let _ = config::save_nodes(&reparsed_nodes);
            let _ = config::save_outbounds(&reparsed_outbounds);
            nodes = reparsed_nodes;
            outbounds = reparsed_outbounds;
        }
    }

    let app_state = AppState {
        singbox_state: singbox::new_shared_state(),
        subscriptions: Mutex::new(subscriptions),
        nodes: Mutex::new(nodes),
        outbounds: Mutex::new(outbounds),
        app_config: Mutex::new(app_config),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .manage(TrayState {
            sys_proxy_item: Mutex::new(None),
            tun_item:       Mutex::new(None),
        })
        .setup(|app| {
            // Restore proxy state if configured
            {
                let cfg = crate::config::load_app_config();
                if cfg.restore_proxy_on_startup && cfg.last_proxy_running {
                    let handle = app.handle().clone();
                    let outbounds = app.state::<AppState>().outbounds.lock().unwrap().clone();
                    let active_tag = cfg.active_nodes.get("proxy").cloned();
                    let mixed_port = cfg.mixed_port;
                    let restore_sys_proxy = cfg.last_system_proxy;
                    let singbox_state = app.state::<AppState>().singbox_state.clone();
                    let config_path = crate::config::singbox_config_path();
                    let singbox_cfg = crate::subscription::build_singbox_config(
                        &outbounds, &cfg, active_tag.as_deref(),
                    );
                    let _ = std::fs::write(
                        &config_path,
                        serde_json::to_string_pretty(&singbox_cfg).unwrap(),
                    );
                    tauri::async_runtime::spawn(async move {
                        // Small delay to let the window/tray finish initializing
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        if let Ok(()) = crate::singbox::start_singbox(
                            &handle, &config_path, singbox_state,
                        ).await {
                            if restore_sys_proxy {
                                let _ = crate::proxy::set_system_proxy(true, mixed_port);
                            }
                        }
                    });
                }
            }

            // Spawn auto-update checker via Tauri's runtime (not bare tokio::spawn)
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let interval = {
                    let cfg = crate::config::load_app_config();
                    cfg.auto_update_interval
                };
                crate::auto_update::start_auto_update_checker(handle, interval).await;
            });

            // ── Tray context menu ────────────────────────────────────────
            let initial_sys_proxy = crate::proxy::get_system_proxy_status();
            let initial_tun = {
                let state = app.state::<AppState>();
                let tun = state.app_config.lock().unwrap().tun_enabled;
                tun
            };

            let dashboard_item = MenuItemBuilder::with_id("dashboard", "仪表盘").build(app)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let sys_proxy_item = CheckMenuItemBuilder::with_id("system_proxy", "系统代理")
                .checked(initial_sys_proxy)
                .build(app)?;
            let tun_item = CheckMenuItemBuilder::with_id("tun_mode", "TUN 模式")
                .checked(initial_tun)
                .build(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            let tray_menu = MenuBuilder::new(app)
                .item(&dashboard_item)
                .item(&sep1)
                .item(&sys_proxy_item)
                .item(&tun_item)
                .item(&sep2)
                .item(&quit_item)
                .build()?;

            // Store handles in TrayState so commands can update check states
            {
                let ts = app.state::<TrayState>();
                *ts.sys_proxy_item.lock().unwrap() = Some(sys_proxy_item.clone());
                *ts.tun_item.lock().unwrap()       = Some(tun_item.clone());
            }

            // Clone handles for use inside the event closure
            let sys_proxy_item_c = sys_proxy_item.clone();
            let tun_item_c = tun_item.clone();

            let tray = TrayIconBuilder::with_id("tray-main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("sing-box-win\n● 已停止")
                .menu(&tray_menu)
                .show_menu_on_left_click(false) // left click = show window; right click = menu
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "dashboard" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "system_proxy" => {
                            // is_checked() reflects the NEW state after the click
                            let enabled = sys_proxy_item_c.is_checked().unwrap_or(false);
                            let port = app
                                .state::<AppState>()
                                .app_config
                                .lock()
                                .unwrap()
                                .mixed_port;
                            let _ = crate::proxy::set_system_proxy(
                                enabled,
                                if enabled { port } else { 0 },
                            );
                        }
                        "tun_mode" => {
                            let new_tun = tun_item_c.is_checked().unwrap_or(false);
                            let state = app.state::<AppState>();
                            let mut cfg = state.app_config.lock().unwrap();
                            cfg.tun_enabled = new_tun;
                            let _ = crate::config::save_app_config(&cfg);
                        }
                        "quit" => {
                            // Stop sing-box and remove system proxy before exiting.
                            // Use std::process::exit instead of app.exit to avoid the
                            // WebView2 "Failed to unregister Chrome_WidgetWin_0" error
                            // that occurs when Tauri tries to tear down the window class
                            // while it still has live windows (Windows Error 1412).
                            let app_c = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let state = app_c.state::<AppState>();
                                let sb_state = state.singbox_state.clone();
                                let _ = crate::singbox::stop_singbox(sb_state).await;
                                let _ = crate::proxy::set_system_proxy(false, 0);
                                std::process::exit(0);
                            });
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::TrayIconEvent;
                    if let TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;
            let _ = tray;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cmd_start_singbox,
            commands::cmd_stop_singbox,
            commands::cmd_get_singbox_status,
            commands::cmd_get_logs,
            commands::cmd_get_subscriptions,
            commands::cmd_add_subscription,
            commands::cmd_update_subscription,
            commands::cmd_delete_subscription,
            commands::cmd_get_nodes,
            commands::cmd_test_node_latency,
            commands::cmd_test_node_speed,
            commands::cmd_auto_select_node,
            commands::cmd_set_active_node,
            commands::cmd_get_app_config,
            commands::cmd_save_app_config,
            commands::cmd_set_proxy_mode,
            commands::cmd_get_connections,
            commands::cmd_parse_subscription_from_text,
            commands::cmd_check_singbox_update,
            commands::cmd_get_installed_version,
            commands::cmd_singbox_exists,
            commands::cmd_download_singbox,
            commands::cmd_is_elevated,
            commands::cmd_relaunch_as_admin,
            commands::cmd_wintun_available,
            commands::cmd_download_wintun,
            commands::cmd_get_rules,
            commands::cmd_save_rules,
            commands::cmd_add_rule,
            commands::cmd_delete_rule,
            commands::cmd_toggle_rule,
            commands::cmd_reset_rules,
            commands::cmd_update_tray_tooltip,
            commands::cmd_get_system_proxy_status,
            commands::cmd_set_system_proxy,
            commands::cmd_sync_tray_menu,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
