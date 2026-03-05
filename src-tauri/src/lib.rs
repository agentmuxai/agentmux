mod commands;
mod crash;
mod drag;
mod heartbeat;
mod menu;
mod sidecar;
mod state;
// mod tray; // Tray now managed by backend (cmd/server/tray.go)

use tauri::Emitter;
use tauri::Manager;

/// Initialize and run the AgentMux Tauri application.
///
/// This replaces the Electron main process (emain/emain.ts).
/// The Rust backend (agentmuxsrv-rs) is spawned as a sidecar process,
/// and the React frontend connects to it via WebSocket/HTTP.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // In dev mode, disable WebView2 HTTP cache to prevent stale bundle loading.
    // Without this, the webview caches production bundles and ignores Vite dev server updates.
    #[cfg(all(debug_assertions, target_os = "windows"))]
    {
        // SAFETY: Called before any threads are spawned, at the very start of main.
        unsafe { std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--disable-http-cache --disk-cache-size=0") };
    }

    let builder = tauri::Builder::default()
        // Plugins
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_websocket::init())
        // Managed state
        .manage(state::AppState::default())
        // Commands (IPC handlers replacing Electron's ipcMain)
        .invoke_handler(tauri::generate_handler![
            // Platform commands
            commands::platform::get_platform,
            commands::platform::get_user_name,
            commands::platform::get_host_name,
            commands::platform::get_is_dev,
            commands::platform::get_data_dir,
            commands::platform::get_config_dir,
            commands::platform::get_env,
            commands::platform::get_about_modal_details,
            commands::platform::get_docsite_url,
            // Auth commands
            commands::auth::get_auth_key,
            // Claude Code auth commands
            commands::claudecode::open_claude_code_auth,
            commands::claudecode::get_claude_code_auth,
            commands::claudecode::disconnect_claude_code,
            // Window commands
            commands::window::open_new_window,
            commands::window::close_window,
            commands::window::minimize_window,
            commands::window::maximize_window,
            commands::window::get_window_label,
            commands::window::is_main_window,
            commands::window::list_windows,
            commands::window::focus_window,
            commands::window::get_zoom_factor,
            commands::window::set_zoom_factor,
            commands::window::get_cursor_point,
            commands::window::get_instance_number,
            commands::window::get_window_count,
            // Backend commands
            commands::backend::get_backend_endpoints,
            commands::backend::get_wave_init_opts,
            commands::backend::fe_log,
            // Devtools commands
            commands::devtools::toggle_devtools,
            commands::devtools::is_devtools_open,
            // Context menu
            commands::contextmenu::show_context_menu,
            // Stub commands (to be implemented in later phases)
            commands::stubs::download_file,
            commands::stubs::quicklook,
            commands::stubs::update_wco,
            commands::stubs::set_keyboard_chord_mode,
            commands::stubs::register_global_webview_keys,
            commands::stubs::create_workspace,
            commands::stubs::switch_workspace,
            commands::stubs::delete_workspace,
            commands::stubs::set_active_tab,
            commands::stubs::create_tab,
            commands::stubs::close_tab,
            commands::stubs::set_window_init_status,
            commands::stubs::set_waveai_open,
            commands::stubs::install_update,
            // Provider commands
            commands::providers::detect_installed_clis,
            commands::providers::get_provider_config,
            commands::providers::save_provider_config,
            commands::providers::get_provider_install_info,
            commands::providers::set_provider_auth,
            commands::providers::clear_provider_auth,
            commands::providers::get_provider_auth_status,
            commands::providers::check_cli_auth_status,
            // CLI installer commands
            commands::cli_installer::install_cli,
            commands::cli_installer::get_cli_path,
        ])
        // Application setup
        .setup(|app| {
            let handle = app.handle().clone();

            // Initialize logging
            let log_dir = init_logging(&handle);

            // In dev mode, clear the WebView2 cache to prevent stale bundles
            #[cfg(all(debug_assertions, target_os = "windows"))]
            {
                if let Ok(data_dir) = handle.path().app_local_data_dir() {
                    let cache_dir = data_dir.join("EBWebView").join("Default").join("Cache");
                    if cache_dir.exists() {
                        match std::fs::remove_dir_all(&cache_dir) {
                            Ok(_) => tracing::info!("Cleared WebView2 cache for dev mode"),
                            Err(e) => tracing::debug!("Could not clear WebView2 cache: {}", e),
                        }
                    }
                }
            }

            // Initialize crash handler
            crash::init_crash_handler(log_dir.clone());

            // Start heartbeat monitoring
            let data_dir = handle.path().app_data_dir().unwrap_or_else(|_| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });
            tauri::async_runtime::spawn(heartbeat::start_heartbeat(data_dir.clone()));

            // Menu disabled for frameless window build
            // match menu::build_app_menu(&handle) {
            //     Ok(app_menu) => {
            //         if let Err(e) = handle.set_menu(app_menu) {
            //             tracing::error!("Failed to set application menu: {}", e);
            //         }
            //     }
            //     Err(e) => {
            //         tracing::error!("Failed to build application menu: {}", e);
            //     }
            // }

            // System tray now managed by backend (agentmuxsrv)
            // See cmd/server/tray.go for implementation

            // Set window title with version
            if let Some(window) = handle.get_webview_window("main") {
                let version = env!("CARGO_PKG_VERSION");
                let title = format!("AgentMux {}", version);
                if let Err(e) = window.set_title(&title) {
                    tracing::error!("Failed to set window title: {}", e);
                }

                // On Linux: attach native GTK drag handler to this window.
                #[cfg(target_os = "linux")]
                drag::attach_drag_handler(&window);

                // On Linux: center the window if no saved position exists.
                // tauri_plugin_window_state auto-restores saved state at window creation.
                // If no state was saved yet, the window lands at (0,0) on Linux because
                // X11 has no default centering behavior (unlike macOS/Windows). So if
                // the window is still at the origin after plugin restoration, center it.
                #[cfg(target_os = "linux")]
                if window.outer_position().map(|p| p.x == 0 && p.y == 0).unwrap_or(true) {
                    let _ = window.center();
                }
            }

            // Register deep link handler for OAuth callback (agentmux://auth?code=...)
            // TODO: Deep link registration needs proper Tauri v2 configuration
            // For now, this is a placeholder for future implementation
            // #[cfg(target_os = "macos")]
            // {
            //     let handle_clone = handle.clone();
            //     tauri::async_runtime::spawn(async move {
            //         handle_clone.listen("deep-link-urls", move |event| {
            //             if let Some(urls) = event.payload().as_array() {
            //                 for url_value in urls {
            //                     if let Some(url) = url_value.as_str() {
            //                         tracing::info!("Received deep link: {}", url);
            //                         handle_deep_link(handle_clone.clone(), url);
            //                     }
            //                 }
            //             }
            //         });
            //     });
            // }

            // Spawn the Go backend as a sidecar
            tauri::async_runtime::spawn(async move {
                match sidecar::spawn_backend(&handle).await {
                    Ok(backend_state) => {
                        // Store backend endpoints in app state
                        let state = handle.state::<state::AppState>();
                        let mut endpoints = state.backend_endpoints.lock().unwrap();
                        endpoints.ws_endpoint = backend_state.ws_endpoint;
                        endpoints.web_endpoint = backend_state.web_endpoint;
                        endpoints.is_reused = backend_state.is_reused;
                        tracing::info!("Backend ready: ws={}, web={}",
                            endpoints.ws_endpoint, endpoints.web_endpoint);

                        // TODO: Query backend RPC for client data instead of database
                        // For now, let backend create client/window/tab on first connection
                        // The frontend will need to query the backend for these IDs
                        tracing::info!("Backend will create client/window/tab on first connection");
                        tracing::info!("Frontend should call backend RPC to get initialized client data");

                        // Emit event to frontend that backend is ready
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.emit("backend-ready", serde_json::json!({
                                "ws": endpoints.ws_endpoint.clone(),
                                "web": endpoints.web_endpoint.clone(),
                                "is_reused": backend_state.is_reused,
                            }));
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to start backend: {}", e);
                        // Show error dialog and quit
                        let _ = handle.emit("backend-error", e.clone());
                    }
                }
            });

            Ok(())
        })
        // Menu event handling
        .on_menu_event(|app, event| {
            menu::handle_menu_event(app, event);
        })
        // Window event handling
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // Count remaining windows (excluding the one being closed)
                    let closing_label = window.label().to_string();
                    let remaining_windows = window.app_handle().webview_windows()
                        .keys()
                        .filter(|label| **label != closing_label)
                        .count();

                    // Unregister the instance number and notify remaining windows.
                    {
                        let state = window.app_handle().state::<state::AppState>();
                        let mut reg = state.window_instance_registry.lock().unwrap();
                        reg.unregister(&closing_label);
                        let count = reg.count();
                        drop(reg);
                        let _ = window.app_handle().emit("window-instances-changed", count);
                    }

                    tracing::info!("Window {} closing, {} other window(s) remaining", closing_label, remaining_windows);

                    // Only shut down the backend sidecar when the last window is closing
                    if remaining_windows == 0 {
                        let state = window.app_handle().state::<state::AppState>();
                        let mut sidecar = state.sidecar_child.lock().unwrap();
                        if let Some(child) = sidecar.take() {
                            tracing::info!("Shutting down backend sidecar (last window closing)");
                            let _ = child.kill();

                            // Clean up endpoints file to prevent stale file race conditions
                            if let Ok(config_dir) = window.app_handle().path().app_config_dir() {
                                let version = env!("CARGO_PKG_VERSION");
                                let endpoints_file = config_dir
                                    .join("instances")
                                    .join(format!("v{}", version))
                                    .join("wave-endpoints.json");
                                if endpoints_file.exists() {
                                    if let Err(e) = std::fs::remove_file(&endpoints_file) {
                                        tracing::warn!("Failed to remove endpoints file on shutdown: {}", e);
                                    } else {
                                        tracing::info!("Removed endpoints file on shutdown: {}", endpoints_file.display());
                                    }
                                }
                            }
                        }

                        // Clean up heartbeat file
                        if let Ok(data_dir) = window.app_handle().path().app_data_dir() {
                            heartbeat::cleanup_heartbeat(&data_dir);
                        }
                    }

                    // Allow the close to proceed
                    let _ = api;
                }
                tauri::WindowEvent::Focused(focused) => {
                    if let Some(w) = window.app_handle().get_webview_window("main") {
                        let _ = w.emit("window-focused", focused);
                    }
                }
                tauri::WindowEvent::Resized(size) => {
                    let _ = window.emit("window-resized", serde_json::json!({
                        "width": size.width,
                        "height": size.height,
                    }));
                }
                _ => {}
            }
        });

    builder
        .run(tauri::generate_context!())
        .expect("error while running AgentMux");
}

// Deep link handler removed — auth is now handled by `claude auth login` via shell controller.
// See docs/SPEC_CLAUDE_CLI_INTEGRATION.md for the auth flow.

fn init_logging(handle: &tauri::AppHandle) -> std::path::PathBuf {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    let log_dir = handle
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));

    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::daily(&log_dir, "agentmux.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Keep the guard alive for the lifetime of the app
    // by leaking it (acceptable for a long-running app)
    std::mem::forget(_guard);

    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("agentmux=info,warn")),
        )
        .with(fmt::layer().with_writer(non_blocking))
        .with(fmt::layer().with_writer(std::io::stderr));

    tracing::subscriber::set_global_default(subscriber).ok();
    tracing::info!("AgentMux starting");

    log_dir
}
