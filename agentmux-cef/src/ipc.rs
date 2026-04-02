// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// IPC bridge between frontend JavaScript and Rust backend.
//
// Phase 2: Embedded HTTP server (axum) on localhost with a random port.
//
// Architecture:
//   JS -> Rust:  fetch("http://127.0.0.1:{port}/ipc", { method: "POST", body: JSON.stringify({cmd, args}) })
//   Rust -> JS:  frame.execute_javascript("window.dispatchEvent(new CustomEvent('agentmux-event', {detail: ...}))")
//
// Why HTTP over CEF ProcessMessage:
//   - cef-rs does not wrap CefMessageRouter (C++ convenience class)
//   - Building a custom ProcessMessage router requires RenderProcessHandler + V8 bindings
//   - fetch() is natural for async/await frontend code
//   - Easy to debug: curl http://127.0.0.1:PORT/ipc -d '{"cmd":"get_platform"}'
//   - axum is already in the tokio ecosystem

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::commands;
use crate::state::AppState;

/// IPC command request from the frontend.
#[derive(Debug, serde::Deserialize)]
pub struct IpcRequest {
    /// Command name (maps to Tauri command names).
    pub cmd: String,
    /// Command arguments as JSON.
    #[serde(default)]
    pub args: serde_json::Value,
}

/// IPC response back to the frontend.
#[derive(Debug, serde::Serialize)]
pub struct IpcResponse {
    /// Whether the command succeeded.
    pub success: bool,
    /// Result data (on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Error message (on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Health check response.
#[derive(Debug, serde::Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

/// Start the IPC HTTP server on a random localhost port.
/// Returns the port number.
pub async fn start_ipc_server(state: Arc<AppState>) -> u16 {
    // Determine frontend static files directory (next to the executable)
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    // Check runtime/frontend/ (portable layout) then frontend/ (dev/flat layout)
    let runtime_dir = exe_dir.join("runtime");
    let frontend_dir = if runtime_dir.join("frontend").join("index.html").exists() {
        runtime_dir.join("frontend")
    } else {
        exe_dir.join("frontend")
    };
    let has_frontend = frontend_dir.join("index.html").exists();
    if has_frontend {
        tracing::info!("Serving static frontend from: {}", frontend_dir.display());
    }

    let mut app = Router::new()
        .route("/ipc", post(handle_ipc))
        .route("/health", get(health))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Serve built frontend as static files (for portable/production builds)
    if has_frontend {
        app = app.fallback_service(ServeDir::new(&frontend_dir));
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind IPC server");
    let port = listener
        .local_addr()
        .expect("Failed to get local address")
        .port();

    tracing::info!("IPC HTTP server started on 127.0.0.1:{}", port);

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("IPC server error");
    });

    port
}

/// Health check endpoint.
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Main IPC handler — routes commands to the appropriate handler.
///
/// Requires `Authorization: Bearer {ipc_token}` header to prevent
/// unauthorized local processes from accessing the IPC server.
async fn handle_ipc(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<IpcRequest>,
) -> (StatusCode, Json<IpcResponse>) {
    // Verify IPC token
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|token| token == state.ipc_token)
        .unwrap_or(false);

    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            Json(IpcResponse {
                success: false,
                data: None,
                error: Some("Unauthorized: invalid or missing IPC token".to_string()),
            }),
        );
    }

    tracing::debug!("IPC request: cmd={} args={}", req.cmd, req.args);

    let result = route_command(&state, &req.cmd, &req.args).await;

    match result {
        Ok(data) => (
            StatusCode::OK,
            Json(IpcResponse {
                success: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(error) => (
            StatusCode::OK, // Return 200 even on errors — frontend checks success field
            Json(IpcResponse {
                success: false,
                data: None,
                error: Some(error),
            }),
        ),
    }
}

/// Route a command to the appropriate handler.
///
/// Command names use snake_case to match the Tauri command names.
/// The frontend sends these exact names via invokeCommand().
async fn route_command(
    state: &Arc<AppState>,
    cmd: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Check stubs first
    if commands::stubs::is_stub_command(cmd) {
        return Ok(commands::stubs::handle_stub(cmd, args));
    }

    match cmd {
        // ---- Tier 1: Bootstrap (must work for frontend to load) ----
        "get_platform" => Ok(commands::platform::get_platform()),
        "get_auth_key" => {
            let key = state.auth_key.lock().unwrap().clone();
            tracing::debug!("Frontend requested auth key: {}...", &key[..8.min(key.len())]);
            Ok(serde_json::json!(key))
        }
        "get_is_dev" => Ok(commands::platform::get_is_dev()),
        "get_user_name" => Ok(commands::platform::get_user_name()),
        "get_host_name" => Ok(commands::platform::get_host_name()),
        "get_data_dir" => commands::platform::get_data_dir(state),
        "get_config_dir" => commands::platform::get_config_dir(state),
        "get_docsite_url" => Ok(commands::platform::get_docsite_url(state)),
        "get_zoom_factor" => Ok(commands::window::get_zoom_factor(state)),
        "get_about_modal_details" => Ok(commands::platform::get_about_modal_details(state)),
        "get_host_info" => Ok(commands::platform::get_host_info(state)),
        "get_backend_endpoints" => commands::backend::get_backend_endpoints(state),
        "get_wave_init_opts" => commands::backend::get_wave_init_opts(state),
        "set_window_init_status" => Ok(commands::backend::set_window_init_status(state, args)),
        "fe_log" => Ok(commands::backend::fe_log(args)),
        "fe_log_structured" => Ok(commands::backend::fe_log_structured(args)),

        // ---- Tier 2: Core functionality ----
        "get_backend_info" => Ok(commands::backend::get_backend_info(state)),
        "restart_backend" => commands::backend::restart_backend(state.clone()).await,
        "close_window" => commands::window::close_window(state),
        "minimize_window" => commands::window::minimize_window(state),
        "maximize_window" => commands::window::maximize_window(state),
        "set_zoom_factor" => commands::window::set_zoom_factor(state, args),
        "is_main_window" => Ok(commands::window::is_main_window(args)),
        "get_window_label" => Ok(commands::window::get_window_label(args)),
        "open_new_window" => commands::window::open_new_window(state),
        "get_instance_number" => Ok(commands::window::get_instance_number(state, args)),
        "get_window_count" => Ok(commands::window::get_window_count(state)),
        "get_env" => Ok(commands::platform::get_env(args)),
        "open_external" => commands::platform::open_external(args),
        "set_window_transparency" => commands::window::set_window_transparency(state, args),
        "start_window_drag" => commands::window::start_window_drag(state),
        "get_window_position" => commands::window::get_window_position(state),
        "move_window_by" => commands::window::move_window_by(state, args),
        "toggle_devtools" => commands::window::toggle_devtools(state),
        "show_context_menu" => {
            tracing::debug!("show_context_menu: handled in JS overlay");
            Ok(serde_json::Value::Null)
        }

        // ---- Cross-window drag ----
        "start_cross_drag" => commands::drag::start_cross_drag(state, args),
        "update_cross_drag" => commands::drag::update_cross_drag(state, args),
        "complete_cross_drag" => commands::drag::complete_cross_drag(state, args),
        "cancel_cross_drag" => commands::drag::cancel_cross_drag(state, args),
        "get_cursor_point" => commands::drag::get_cursor_point(),
        "get_mouse_button_state" => commands::drag::get_mouse_button_state(),
        "set_drag_cursor" => commands::drag::set_drag_cursor(),
        "restore_drag_cursor" => commands::drag::restore_drag_cursor(),
        "release_drag_capture" => commands::drag::release_drag_capture(state),
        "set_js_drag_active" => commands::drag::set_js_drag_active(args),
        "open_window_at_position" => commands::drag::open_window_at_position(state, args),
        "list_windows" => Ok(commands::window::list_windows(state)),
        "focus_window" => commands::window::focus_window(state, args),

        // ---- Clipboard (CEF can't use navigator.clipboard without permission policy) ----
        "read_clipboard" => commands::clipboard::read_clipboard(),
        "write_clipboard" => commands::clipboard::write_clipboard(args),

        // ---- Tier 3: Provider/CLI management ----
        "detect_installed_clis" => commands::providers::detect_installed_clis().await,
        "get_provider_config" => commands::providers::get_provider_config(state),
        "save_provider_config" => commands::providers::save_provider_config(state, args),
        "get_provider_install_info" => commands::providers::get_provider_install_info(args),
        "set_provider_auth" => commands::providers::set_provider_auth(state, args),
        "clear_provider_auth" => commands::providers::clear_provider_auth(state, args),
        "get_provider_auth_status" => commands::providers::get_provider_auth_status(state, args),
        "check_cli_auth_status" => commands::providers::check_cli_auth_status(args).await,
        "install_cli" => commands::providers::install_cli(state, args).await,
        "get_cli_path" => commands::providers::get_cli_path(state, args),
        "check_nodejs_available" => commands::providers::check_nodejs_available().await,
        "ensure_auth_dir" => commands::platform::ensure_auth_dir(state, args),
        "run_cli_login" => commands::platform::run_cli_login(state.clone(), args).await,
        "cancel_cli_login" => commands::platform::cancel_cli_login(state),
        "ensure_settings_file" => commands::platform::ensure_settings_file(state),
        "open_in_editor" => commands::platform::open_in_editor(args),
        "copy_file_to_dir" => commands::providers::copy_file_to_dir(args),

        // ---- Unknown command ----
        _ => Err(format!("Unknown command: {}", cmd)),
    }
}
