mod types;
mod cn_direct;
mod config;
mod subscription;
mod singbox;
mod proxy;
mod updater;
mod tun;
mod rules;
mod auto_update;
mod stats;
mod commands;

use std::sync::Mutex;
use tauri::Manager;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use commands::{AppState, TrayState};

/// Update both tray check items to reflect the real runtime state. The two are
/// mutually exclusive: at most one is checked, and only while sing-box is running.
fn sync_tray_checks(
    app: &tauri::AppHandle,
    sys_item: &tauri::menu::CheckMenuItem<tauri::Wry>,
    tun_item: &tauri::menu::CheckMenuItem<tauri::Wry>,
) {
    // With the persistent-core model the core is almost always running, so the checks
    // must reflect the actual *proxying* state: the system proxy registry toggle and
    // whether the running core is in TUN mode — not merely whether the core is up.
    let state = app.state::<AppState>();
    let in_tun = {
        let s = state.singbox_state.lock().unwrap();
        s.running && s.tun_mode
    };
    let sys_on = crate::proxy::get_system_proxy_status();
    let _ = sys_item.set_checked(sys_on && !in_tun);
    let _ = tun_item.set_checked(in_tun);
}

/// Reconcile the tray checks by pulling the menu-item handles out of `TrayState`.
/// Used where we don't hold direct references — notably at the end of the async
/// startup restore, so the initial checkmarks reflect the REAL running state
/// (`singbox_state`) rather than the persisted `tun_enabled` flag.
fn sync_tray_from_state(app: &tauri::AppHandle) {
    let ts = app.state::<TrayState>();
    let sys = ts.sys_proxy_item.lock().unwrap().clone();
    let tun = ts.tun_item.lock().unwrap().clone();
    if let (Some(sys), Some(tun)) = (sys, tun) {
        sync_tray_checks(app, &sys, &tun);
    }
}

/// Push the outcome of a tray-driven connection-mode change to the frontend so the
/// dashboard reconciles its reactive state (tray toggles previously never reached the
/// UI) and surfaces failures (e.g. enabling TUN without admin rights, which used to
/// fail silently from the tray).
fn emit_tray_mode_result(app: &tauri::AppHandle, res: Result<(), String>) {
    use tauri::Emitter;
    match res {
        Ok(()) => {
            let _ = app.emit("connection-mode-changed", ());
        }
        Err(e) => {
            let _ = app.emit("connection-mode-error", e);
        }
    }
}

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
        // Single-instance MUST be the first plugin registered (Tauri requirement). A second
        // launch is redirected here instead of starting a rival process — which would fight
        // over the mixed/API ports and the TUN adapter — and we surface the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
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
        // Global hotkeys are registered/handled from the frontend via the plugin's JS API;
        // the Rust side only needs the default plugin initialised.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(app_state)
        .manage(TrayState {
            sys_proxy_item: Mutex::new(None),
            tun_item:       Mutex::new(None),
        })
        /* With the native title bar, the window's close button fires CloseRequested
           directly. Honor the user's "close to tray" preference here so the behavior
           matches the old self-drawn close button on every platform: hide instead of
           quit when enabled, otherwise let the window close normally. */
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let close_to_tray = window
                    .state::<AppState>()
                    .app_config
                    .lock()
                    .unwrap()
                    .close_to_tray;
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    // Real exit: tear down the persistent core and clear the system proxy
                    // first (otherwise the registry would keep pointing at a dead port and
                    // break connectivity). Prevent the default close, clean up, then exit.
                    api.prevent_close();
                    let app_c = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_c.state::<AppState>();
                        crate::commands::shutdown_core(state.inner()).await;
                        std::process::exit(0);
                    });
                }
            }
        })
        .setup(|app| {
            // Copy bundled rule-set (.srs) files into the app data dir so the
            // generated sing-box config can reference them by absolute path.
            // This makes CN/non-CN routing work offline and behind the GFW,
            // where the previous remote (jsDelivr) download always failed.
            {
                let _ = crate::config::ensure_dirs();
                if let Ok(res_dir) = app.path().resource_dir() {
                    let src_dir = res_dir.join("resources").join("rule-sets");
                    let dst_dir = crate::config::rule_sets_dir();
                    for name in [
                        "geoip-cn.srs",
                        "geosite-cn.srs",
                    ] {
                        let src = src_dir.join(name);
                        if src.exists() {
                            let _ = std::fs::copy(&src, dst_dir.join(name));
                        }
                    }
                }
            }

            // "Start minimized": hide the main window on launch so the app lives in the
            // tray only. The window is created visible by tauri.conf.json, so we hide it
            // here rather than starting hidden (keeps the normal show/focus path intact).
            if crate::config::load_app_config().startup_minimized {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }

            // Persistent core: start sing-box on launch so toggling the proxy later is
            // instant. An idle core (mixed inbound only, no system proxy, no TUN) has no
            // effect on the system network. If the user had a proxy running last session
            // and enabled "restore on startup", re-apply that exact mode; otherwise start
            // idle. The core is torn down on app exit (quit / window-close handlers).
            {
                let cfg = crate::config::load_app_config();
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Small delay to let the window/tray finish initializing.
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let state = handle.state::<AppState>();
                    let restore = cfg.restore_proxy_on_startup && cfg.last_proxy_running;
                    if restore {
                        let mode = if cfg.last_system_proxy {
                            "system"
                        } else if cfg.tun_enabled {
                            "tun"
                        } else {
                            "system"
                        };
                        // Restoring TUN right after an app upgrade is the one case where a
                        // previous, force-killed core may have left a stale TUN adapter and
                        // strict routes behind (a pre-fix old version won't have cleaned up
                        // during its update teardown). Proactively remove the leftover and
                        // let Windows settle before re-creating TUN, so the restored tunnel
                        // doesn't layer onto dying routes and black-hole all traffic. This is
                        // a once-per-launch cold path, so it doesn't affect toggle latency.
                        if mode == "tun" {
                            crate::tun::cleanup_stale_tun_adapter().await;
                            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                        }
                        // Fall back to an idle core if restoring the saved mode fails
                        // (e.g. TUN without admin rights this session).
                        if crate::commands::apply_connection_mode(&handle, state.inner(), mode)
                            .await
                            .is_err()
                        {
                            let _ = crate::commands::start_idle_core(&handle, state.inner()).await;
                        }
                    } else {
                        let _ = crate::commands::start_idle_core(&handle, state.inner()).await;
                    }
                    // Tray is built after this task is spawned but before the 1s delay
                    // elapses, so its item handles are available here. Reconcile the
                    // checkmarks with the real running state instead of the persisted flag.
                    sync_tray_from_state(&handle);
                });
            }

            // Spawn sing-box binary auto-update checker
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let interval = {
                    let cfg = crate::config::load_app_config();
                    cfg.auto_update_interval
                };
                crate::auto_update::start_auto_update_checker(handle, interval).await;
            });

            // Spawn subscription auto-update checker
            let handle_sub = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::auto_update::start_subscription_auto_updater(handle_sub).await;
            });

            // Spawn app self-update checker (runs once, 45s after launch)
            let handle_app_upd = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::auto_update::start_app_update_checker(handle_app_upd).await;
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

            // macOS menu-bar requires a monochrome *template* icon: it uses only the
            // alpha channel and auto-inverts with light/dark menu bars. The full-color
            // app icon (dark) was invisible on a dark menu bar — hence "no tray icon".
            // Other platforms keep the colored window icon.
            #[cfg(target_os = "macos")]
            let tray_icon = tauri::image::Image::from_bytes(
                include_bytes!("../icons/tray-template.png")
            ).expect("无效的托盘模板图标");
            #[cfg(not(target_os = "macos"))]
            let tray_icon = app.default_window_icon().unwrap().clone();

            let tray = TrayIconBuilder::with_id("tray-main")
                .icon(tray_icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Skylark\n● 已停止")
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
                            // is_checked() reflects the NEW state after the click.
                            let enabled = sys_proxy_item_c.is_checked().unwrap_or(false);
                            let app_c = app.clone();
                            let sys_item = sys_proxy_item_c.clone();
                            let tun_it = tun_item_c.clone();
                            tauri::async_runtime::spawn(async move {
                                let state = app_c.state::<AppState>();
                                let mode = if enabled { "system" } else { "off" };
                                let res = crate::commands::apply_connection_mode(&app_c, &state, mode).await;
                                sync_tray_checks(&app_c, &sys_item, &tun_it);
                                emit_tray_mode_result(&app_c, res);
                            });
                        }
                        "tun_mode" => {
                            // is_checked() reflects the NEW state after the click.
                            let enabled = tun_item_c.is_checked().unwrap_or(false);
                            let app_c = app.clone();
                            let sys_item = sys_proxy_item_c.clone();
                            let tun_it = tun_item_c.clone();
                            tauri::async_runtime::spawn(async move {
                                let state = app_c.state::<AppState>();
                                let mode = if enabled { "tun" } else { "off" };
                                let res = crate::commands::apply_connection_mode(&app_c, &state, mode).await;
                                sync_tray_checks(&app_c, &sys_item, &tun_it);
                                emit_tray_mode_result(&app_c, res);
                            });
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
                                crate::commands::shutdown_core(state.inner()).await;
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
            commands::cmd_set_connection_mode,
            commands::cmd_get_singbox_status,
            commands::cmd_get_logs,
            commands::cmd_export_config,
            commands::cmd_import_config,
            commands::cmd_list_profiles,
            commands::cmd_save_profile,
            commands::cmd_load_profile,
            commands::cmd_delete_profile,
            commands::cmd_get_subscriptions,
            commands::cmd_add_subscription,
            commands::cmd_import_subscription_from_text,
            commands::cmd_update_subscription,
            commands::cmd_delete_subscription,
            commands::cmd_get_nodes,
            commands::cmd_test_node_latency,
            commands::cmd_test_node_speed,
            commands::cmd_set_active_node,
            commands::cmd_set_auto_node,
            commands::cmd_get_active_proxy_now,
            commands::cmd_test_group_delay,
            commands::cmd_get_app_config,
            commands::cmd_save_app_config,
            commands::cmd_set_proxy_mode,
            commands::cmd_export_logs,
            commands::cmd_get_connections,
            commands::cmd_close_connection,
            commands::cmd_close_all_connections,
            commands::cmd_get_traffic_total,
            commands::cmd_add_traffic_sample,
            commands::cmd_get_traffic_history,
            commands::cmd_run_diagnostics,
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
            commands::cmd_get_rule_providers,
            commands::cmd_add_rule_provider,
            commands::cmd_delete_rule_provider,
            commands::cmd_toggle_rule_provider,
            commands::cmd_get_proxy_groups,
            commands::cmd_save_proxy_groups,
            commands::cmd_update_tray_tooltip,
            commands::cmd_get_system_proxy_status,
            commands::cmd_set_system_proxy,
            commands::cmd_sync_tray_menu,
            commands::cmd_get_memory_usage,
            commands::cmd_save_subscription_settings,
            commands::cmd_set_subscription_filters,
            commands::cmd_check_app_update,
            commands::cmd_download_app_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
