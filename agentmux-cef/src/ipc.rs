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
        .route("/health", get(health));
    // Browser DOM API routes (`/agentmux/browser/*`). Token auth is
    // enforced inside each handler — same bearer scheme as /ipc.
    app = crate::browser_api::register_routes(app);
    let mut app = app
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
            let key = state.auth_key.lock().clone();
            tracing::debug!("Frontend requested auth key: {}...", &key[..8.min(key.len())]);
            Ok(serde_json::json!(key))
        }
        "get_is_dev" => Ok(commands::platform::get_is_dev()),
        "get_user_name" => Ok(commands::platform::get_user_name()),
        "get_host_name" => Ok(commands::platform::get_host_name()),
        "get_data_dir" => commands::platform::get_data_dir(state),
        "get_config_dir" => commands::platform::get_config_dir(state),
        "get_user_home_dir" => commands::platform::get_user_home_dir(state),
        "get_docsite_url" => Ok(commands::platform::get_docsite_url(state)),
        "get_zoom_factor" => Ok(commands::window::get_zoom_factor(state)),
        "get_about_modal_details" => Ok(commands::platform::get_about_modal_details(state)),
        "get_host_info" => Ok(commands::platform::get_host_info(state)),
        "get_backend_endpoints" => commands::backend::get_backend_endpoints(state),
        "get_wave_init_opts" => commands::backend::get_wave_init_opts(state),
        "set_window_init_status" => Ok(commands::backend::set_window_init_status(state, args)),
        "report_first_paint" => Ok(commands::backend::report_first_paint(state, args)),
        "fe_log" => Ok(commands::backend::fe_log(args)),
        "fe_log_structured" => Ok(commands::backend::fe_log_structured(args)),

        // ---- Tier 2: Core functionality ----
        "get_backend_info" => Ok(commands::backend::get_backend_info(state)),
        "restart_backend" => commands::backend::restart_backend(state.clone()).await,
        "run_migrations" => commands::backend::run_migrations(state.clone()).await,
        "run_saga_vacuum" => commands::backend::run_saga_vacuum(state.clone()).await,
        "close_window" => commands::window::close_window(state, args),
        // Issue #2977 WS4 — hand the frontend whatever the background
        // service did while no window was open, so it can tell the user.
        "background_audit_take" => crate::background_audit::background_audit_take(state),
        "minimize_window" => commands::window::minimize_window(state, args),
        "maximize_window" => commands::window::maximize_window(state, args),
        "toggle_floating_maximize" => commands::window::toggle_floating_maximize(state, args),
        "set_zoom_factor" => commands::window::set_zoom_factor(state, args),
        "is_main_window" => Ok(commands::window::is_main_window(args)),
        "get_window_label" => Ok(commands::window::get_window_label(args)),
        "open_new_window" => commands::window::open_new_window(state, args),
        "open_subwindow" => {
            // Agent / backend-only API — creates a sub-window tied to a full
            // instance. Hidden from the taskbar. Not exposed in user UI.
            let parent = args
                .get("parent_instance_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "open_subwindow: parent_instance_id required".to_string())?
                .to_string();
            let initial_view = args.get("initial_view").and_then(|v| v.as_str());
            let initial_meta = args.get("initial_meta").and_then(|v| v.as_str());
            commands::window::open_subwindow(state, parent, initial_view, initial_meta)
        }
        "open_floating_pane_window" => {
            // Floating-pane tear-off — a chromeless window showing just the
            // torn-off pane. Windows: unowned WS_POPUP+WS_EX_TOOLWINDOW HWND
            // with explicit cascade hook. macOS/Linux (Phase A): a frameless
            // CEF Views window with ?floatingPaneId= in the URL.
            // Specs: SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md +
            // SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md.
            commands::floating_pane::open_floating_pane_window(state, args)
        }
        "get_pane_debug_state" => {
            // Diagnostic snapshot: active floaters, pane-closing gate, pool
            // queue size, pending window creations. Called by the frontend
            // before/after every tear-off to surface intermittent failures.
            Ok(commands::floating_pane::get_pane_debug_state(state))
        }
        "debug:hang_ui" => {
            // SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 2 verification hook —
            // deliberately wedge the CEF UI thread so the launcher's armed
            // J0 teardown can be reproduced live (close the last window,
            // watch the backstop terminate the tree within GRACE + 2 probe
            // intervals). Double-gated: the env var must be explicitly set
            // AND the command explicitly invoked — never reachable in
            // normal operation.
            if std::env::var("AGENTMUX_DEBUG_HANG").as_deref() != Ok("1") {
                Err("debug:hang_ui refused — set AGENTMUX_DEBUG_HANG=1 to enable".to_string())
            } else {
                tracing::warn!("[debug:hang_ui] parking the CEF UI thread in a 1h sleep — teardown-backstop verification only");
                crate::ui_tasks::post_hang_ui_thread(state);
                Ok(serde_json::Value::Null)
            }
        }
        "get_instance_number" => Ok(commands::window::get_instance_number(state, args)),
        "register_backend_window" => Ok(commands::window::register_backend_window(state, args)),
        "get_env" => Ok(commands::platform::get_env(args)),
        "open_external" => commands::platform::open_external(args),
        "reveal_in_file_explorer" => commands::platform::reveal_in_file_explorer(args),
        "show_open_file_dialog" => commands::platform::show_open_file_dialog(args).await,
        "show_open_bundle_dialog" => commands::platform::show_open_bundle_dialog(args).await,
        "set_window_transparency" => commands::window::set_window_transparency(state, args),
        "set_window_opacity" => commands::window::set_window_opacity(state, args),
        "get_window_opacity" => commands::window::get_window_opacity(state, args).await,
        "start_window_drag" => commands::window::start_window_drag(state, args),
        "get_window_rect" => commands::window::get_window_rect(state, args),
        "set_window_rect" => commands::window::set_window_rect(state, args),
        "get_window_position" => {
            // Wrap in spawn_blocking — on macOS/Linux `get_window_position`
            // bounces a CEF Views `bounds()` read through the UI thread and
            // waits on a bounded channel for up to 250 ms
            // (ui_tasks::get_window_position_blocking). Without this wrap it
            // would block a Tokio worker, same as `tear_off_sc_move_handshake`
            // above. (Windows reads GetWindowRect directly — cheap — but the
            // wrap is harmless there and keeps the dispatch uniform.)
            let state_clone = state.clone();
            let args_clone = args.clone();
            tokio::task::spawn_blocking(move || {
                commands::window::get_window_position(&state_clone, &args_clone)
            })
            .await
            .map_err(|e| format!("get_window_position join error: {}", e))?
        }
        "resolve_window_at_cursor" => {
            // spawn_blocking — on macOS/Linux this bounces a CEF Views bounds
            // hit-test through the UI thread (up to 250 ms), same as
            // get_window_position above. Windows reads HWND rects directly.
            let state_clone = state.clone();
            let args_clone = args.clone();
            tokio::task::spawn_blocking(move || {
                commands::window::resolve_window_at_cursor(&state_clone, &args_clone)
            })
            .await
            .map_err(|e| format!("resolve_window_at_cursor join error: {}", e))?
        }
        "update_floating_redock_hover" => {
            // spawn_blocking — calls resolve_window_at_cursor internally, which
            // blocks on the UI thread for up to 250 ms on macOS/Linux.
            let state_clone = state.clone();
            let args_clone = args.clone();
            tokio::task::spawn_blocking(move || {
                commands::window::update_floating_redock_hover(&state_clone, &args_clone)
            })
            .await
            .map_err(|e| format!("update_floating_redock_hover join error: {}", e))?
        }
        "clear_floating_redock_hover" => commands::window::clear_floating_redock_hover(state, args),
        "set_floating_redock_target" => commands::window::set_floating_redock_target(state, args),
        "get_floating_redock_target" => commands::window::get_floating_redock_target(state, args),
        "move_window_by" => commands::window::move_window_by(state, args),
        "set_window_position" => commands::window::set_window_position(state, args),
        "toggle_devtools" => commands::window::toggle_devtools(state, args),
        "inspect_element_at" => commands::window::inspect_element_at(state, args),
        // Dev-only memory-infra GPU tracing (#2218 diagnostics) — see
        // docs/specs/SPEC_GPU_MEMORY_TRACING_SCAFFOLDING_2026_07_24.md.
        "begin_gpu_trace" => commands::window::begin_gpu_trace(state, args),
        "end_gpu_trace" => commands::window::end_gpu_trace(state, args),
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
        "tear_off_pool_promote" => commands::drag::tear_off_pool_promote(state, args),
        "pool_window_ready" => commands::drag::pool_window_ready(state, args),
        "start_tab_drag_tracking" => {
            // spawn_blocking — blocks briefly (~ms) on the hook thread's
            // install-ready channel, same rationale as
            // tear_off_sc_move_handshake below.
            let state_clone = state.clone();
            let args_clone = args.clone();
            tokio::task::spawn_blocking(move || {
                commands::drag::start_tab_drag_tracking(&state_clone, &args_clone)
            })
            .await
            .map_err(|e| format!("start_tab_drag_tracking join error: {}", e))?
        }
        "stop_tab_drag_tracking" => {
            // spawn_blocking too, matching start_tab_drag_tracking above —
            // macOS's stop_active_hook_session now blocks on a lock shared
            // with start (reagent PR #2310 P1: without serializing them, a
            // fast drag-then-release could have this run — and no-op, since
            // ACTIVE_HOOK_RUNLOOP isn't set yet — before start's spawned
            // hook thread finishes CGEventTapCreate, leaving a zombie hook
            // alive). Running that (bounded, but real) block on an async
            // worker thread instead of the blocking pool would be its own
            // problem.
            tokio::task::spawn_blocking(commands::drag::stop_tab_drag_tracking)
                .await
                .map_err(|e| format!("stop_tab_drag_tracking join error: {}", e))?
        }
        "pane_pool_window_ready" => commands::drag::pane_pool_window_ready(state, args),
        "tear_off_sc_move_handshake" => {
            // Wrap in spawn_blocking — the handler polls state.browsers
            // for up to 2s waiting for the destination window's HWND to
            // register (cold path, gone after Phase 6 warm pool). Without
            // this wrap it would block a Tokio worker.
            let state_clone = state.clone();
            let args_clone = args.clone();
            tokio::task::spawn_blocking(move || {
                commands::drag::tear_off_sc_move_handshake(&state_clone, &args_clone)
            })
            .await
            .map_err(|e| format!("tear_off_sc_move_handshake join error: {}", e))?
        }
        "list_windows" => Ok(commands::window::list_windows(state)),
        "list_window_instances" => Ok(commands::window::list_window_instances(state)),
        "get_double_click_time" => Ok(commands::window::get_double_click_time()),
        "focus_window" => commands::window::focus_window(state, args),
        "close_window_by_label" => commands::window::close_window_by_label(state, args),

        // ---- Clipboard (CEF can't use navigator.clipboard without permission policy) ----
        "read_clipboard" => commands::clipboard::read_clipboard(),
        "write_clipboard" => commands::clipboard::write_clipboard(args),

        // ---- Tier 3: Provider/CLI management ----
        "detect_installed_clis" => commands::providers::detect_installed_clis().await,
        "get_provider_config" => commands::providers::get_provider_config(state),
        "save_provider_config" => commands::providers::save_provider_config(state, args),
        "get_provider_install_info" => commands::providers::get_provider_install_info(args),
        "set_provider_auth" => commands::providers::set_provider_auth(state, args).await,
        "clear_provider_auth" => commands::providers::clear_provider_auth(state, args),
        "get_provider_auth_status" => commands::providers::get_provider_auth_status(state, args),
        "check_cli_auth_status" => commands::providers::check_cli_auth_status(args).await,
        "install_cli" => commands::providers::install_cli(state, args).await,
        "get_cli_path" => commands::providers::get_cli_path(state, args),
        "check_nodejs_available" => commands::providers::check_nodejs_available().await,
        "ensure_auth_dir" => commands::platform::ensure_auth_dir(state, args),
        "run_cli_login" => commands::cli_login::run_cli_login(state.clone(), args).await,
        "cancel_cli_login" => commands::cli_login::cancel_cli_login(state),
        "get_cli_login_status" => commands::cli_login::get_cli_login_status(state),
        "open_login_terminal" => commands::cli_login::open_login_terminal(args),
        "ensure_settings_file" => commands::platform::ensure_settings_file(state),
        "open_in_editor" => commands::platform::open_in_editor(args),
        "copy_file_to_dir" => commands::providers::copy_file_to_dir(args),
        "consume_drag_paths" => Ok(serde_json::json!(crate::drag_stash::take())),

        // ---- Command palette ----
        "run_command" => commands::palette::run_command(state, args),

        // ---- App API (frontend-driven) ----
        "open_agent" => commands::palette::open_agent(state, args),

        // ---- Browser panes (native CefBrowserView) ----
        "browser_pane_create" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("about:blank");
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let w = args.get("width").and_then(|v| v.as_i64()).unwrap_or(800) as i32;
            let h = args.get("height").and_then(|v| v.as_i64()).unwrap_or(600) as i32;
            let rect = cef::Rect { x, y, width: w, height: h };
            // window_label: which window the pane should be attached to
            // (Linux/macOS Views path looks this up in state.windows). Default
            // to "main" for backward compat with frontends that don't send it.
            // Windows path ignores it (find_own_top_level_window resolves the
            // calling window's HWND directly).
            let window_label = args
                .get("window_label")
                .and_then(|v| v.as_str())
                .unwrap_or("main");
            state.browser_panes.create(state, block_id, url, rect, window_label)?;
            Ok(serde_json::json!(true))
        }
        "browser_pane_navigate" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.navigate(block_id, url, state)?;
            Ok(serde_json::json!(true))
        }
        "browser_pane_resize" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let w = args.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let h = args.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            // debug! not info! — fires on every pixel during a window-resize
            // drag (the lastSentRect gate skips no-op calls but a real drag
            // emits many genuine rect changes). Reagent P2 on PR #788.
            tracing::debug!(
                "[ipc] browser_pane_resize block_id={} rect=({},{},{},{})",
                block_id, x, y, w, h
            );
            state.browser_panes.resize(block_id, cef::Rect { x, y, width: w, height: h }, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_close" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            // Window-aware close (tear-off / redock race fix). During a
            // cross-window move the pane is closed in the old window and
            // re-created in the new one (see browser_panes::create's
            // AlreadyLiveElsewhere arm). The OLD window's browser-view then
            // unmounts and fires this close — but by now the pane's entry
            // points at the NEW window. Honoring that stale close would destroy
            // the just-moved pane, leaving the new window black. So: if the
            // caller passed its window_label and it no longer matches the pane's
            // current window, ignore the close. Frontends that don't send
            // window_label keep the old unconditional behavior.
            let requesting_window = args
                .get("window_label")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !requesting_window.is_empty() {
                if let Some(current) = state.browser_pane_window_label(block_id) {
                    if current != requesting_window {
                        tracing::info!(
                            block_id,
                            requesting_window,
                            current_window = %current,
                            "[browser_pane_close] ignoring stale close from a window that no longer owns the pane (moved via tear-off/redock)"
                        );
                        crate::browser_pane::trace::pane_trace(
                            block_id,
                            "close-ignored-stale-window",
                            &format!("from={requesting_window} current={current}"),
                        );
                        return Ok(serde_json::json!(true));
                    }
                }
            }
            // Cancel any pending HTTP-auth callbacks parked for this
            // pane before tearing it down. Without this, closing a
            // pane mid-auth-prompt leaks the CEF AuthCallback refcount
            // until the 5-minute TTL fires.
            crate::browser_pane::auth::cancel_for_block(block_id);
            // Un-join this pane's request from any pending credential
            // approval it was riding (possibly shared with a sibling pane
            // hitting the same protection space — see
            // `credential_broker::approval`'s coalescing). If this was the
            // last request on that approval, close its now-pointless
            // subwindow too.
            // Plural: one pane can ride several approvals at once (a
            // proxy-auth and an origin-auth challenge from the same page),
            // and closing it can empty more than one of them. The earlier
            // singular form dropped every emptied approval past the first
            // (reagent P2 on PR #2824).
            for window_id in crate::credential_broker::approval::cancel_for_block(block_id) {
                if let Err(e) =
                    commands::window::close_window_by_label(state, &serde_json::json!({ "label": window_id }))
                {
                    tracing::warn!("[credential-broker] failed to close orphaned approval window: {e}");
                }
            }
            state.browser_panes.close(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_go_back" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.go_back(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_go_forward" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.go_forward(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_reload" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.reload(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_print" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.print(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_view_source" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.view_source(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_inspect_element" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            state.browser_panes.inspect_element(block_id, state, x, y);
            Ok(serde_json::json!(true))
        }
        "browser_pane_copy" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.copy(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_cut" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.cut(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_paste" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.paste(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_focus" => {
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            tracing::info!("[ipc] browser_pane_focus block_id={}", block_id);
            state.browser_panes.focus(block_id, state);
            Ok(serde_json::json!(true))
        }
        "browser_pane_auth_submit" => {
            // Renderer collected credentials from the modal. Resolve the
            // CEF AuthCallback parked under request_id and continue.
            // Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
            let request_id = args.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            let username = args.get("username").and_then(|v| v.as_str()).unwrap_or("");
            let password = args.get("password").and_then(|v| v.as_str()).unwrap_or("");
            // Don't log username/password length — host logs are
            // retained 7 days per CLAUDE.md and the length is sensitive
            // metadata that could narrow a brute-force window.
            // request_id alone is enough to trace flow.
            tracing::info!(
                "[browser-pane-auth] submit request_id={}",
                request_id,
            );
            if let Some(cb) = crate::browser_pane::auth::take(request_id) {
                use cef::ImplAuthCallback;
                let u = cef::CefString::from(username);
                let p = cef::CefString::from(password);
                cb.cont(Some(&u), Some(&p));
                Ok(serde_json::json!(true))
            } else {
                tracing::warn!(
                    "[browser-pane-auth] submit for unknown request_id {} — already resolved?",
                    request_id
                );
                Ok(serde_json::json!(false))
            }
        }
        "browser_pane_auth_cancel" => {
            let request_id = args.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            tracing::info!("[browser-pane-auth] cancel request_id={}", request_id);
            if let Some(cb) = crate::browser_pane::auth::take(request_id) {
                use cef::ImplAuthCallback;
                cb.cancel();
                Ok(serde_json::json!(true))
            } else {
                Ok(serde_json::json!(false))
            }
        }
        "browser_pane_auth_save" => {
            // Opt-in save after a manual credential submit — a wholly new
            // command rather than a flag on browser_pane_auth_submit, so
            // that IPC's contract stays byte-for-byte unchanged.
            // `frontend/app/view/browser/use-browser-auth.ts` fires this
            // right after `browser_pane_auth_submit` when the user checked
            // "save this credential." A failure here never affects the
            // page load — auth already succeeded via the untouched submit
            // path — so this only ever surfaces as a non-blocking toast on
            // the renderer side.
            let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
            let origin = args.get("origin").and_then(|v| v.as_str()).unwrap_or("");
            let realm = args.get("realm").and_then(|v| v.as_str()).unwrap_or("");
            let is_proxy = args.get("is_proxy").and_then(|v| v.as_bool()).unwrap_or(false);
            let username = args.get("username").and_then(|v| v.as_str()).unwrap_or("");
            let password = args.get("password").and_then(|v| v.as_str()).unwrap_or("");
            match crate::credential_broker::save_credential(
                state, block_id, origin, realm, is_proxy, username, password,
            )
            .await
            {
                Ok(()) => Ok(serde_json::json!(true)),
                Err(e) => {
                    tracing::warn!("[credential-broker] browser_pane_auth_save failed: {e}");
                    Err(e)
                }
            }
        }
        "credential_approval_decide" => {
            // The human resolved the credential-approval subwindow —
            // approve (Fill + cont() every coalesced parked callback) or
            // deny (cancel() each). See `credential_broker::approval` for
            // the coalescing rationale (multiple panes hitting the same
            // protection space near-simultaneously share one approval).
            let approval_id = args.get("approval_id").and_then(|v| v.as_str()).unwrap_or("");
            let approve = args.get("approve").and_then(|v| v.as_bool()).unwrap_or(false);
            tracing::info!(
                "[credential-broker] approval_decide approval_id={} approve={}",
                approval_id,
                approve,
            );
            let Some(resolved) = crate::credential_broker::approval::take(approval_id) else {
                tracing::warn!(
                    "[credential-broker] decide for unknown/expired approval_id {} — already \
                     resolved or timed out",
                    approval_id,
                );
                return Ok(serde_json::json!(false));
            };

            if approve {
                match crate::credential_broker::fill_credential(
                    state,
                    &resolved.identity_id,
                    &resolved.origin,
                    &resolved.realm,
                    resolved.is_proxy,
                )
                .await
                {
                    Ok((username, password)) => {
                        use cef::ImplAuthCallback;
                        let u = cef::CefString::from(username.as_str());
                        let p = cef::CefString::from(password.as_str());
                        for parked in &resolved.auth_requests {
                            if let Some(cb) = crate::browser_pane::auth::take(&parked.request_id) {
                                cb.cont(Some(&u), Some(&p));
                            }
                        }
                    }
                    Err(e) => {
                        // The human already said yes; the secret just could not
                        // be read (credential deleted between approval and
                        // Fill, transient keychain/backend failure, srv
                        // restart). Cancelling here would render the bare 401
                        // with no way to type credentials — strictly worse than
                        // pre-feature behaviour, and a direct violation of this
                        // feature's "can only make auth more automatic, never
                        // break the baseline" guarantee (codex P2 → reagent P1
                        // on PR #2824).
                        //
                        // So leave every callback PARKED in
                        // `browser_pane::auth` and emit the ordinary
                        // `browser-pane-auth-required` prompt for each, one per
                        // originating pane — coalesced requests can come from
                        // different panes. The manual prompt then resolves the
                        // very same parked callbacks, exactly as it does when
                        // no stored credential exists at all.
                        tracing::warn!(
                            "[credential-broker] Fill failed for approval_id {}: {e} — \
                             falling {} parked auth request(s) through to the manual prompt",
                            approval_id,
                            resolved.auth_requests.len(),
                        );
                        for parked in &resolved.auth_requests {
                            crate::credential_broker::fall_through_to(
                                state,
                                &parked.block_id,
                                &parked.request_id,
                                &resolved.origin,
                                &resolved.host,
                                resolved.port,
                                &resolved.realm,
                                resolved.is_proxy,
                            );
                        }
                    }
                }
            } else {
                use cef::ImplAuthCallback;
                for parked in &resolved.auth_requests {
                    if let Some(cb) = crate::browser_pane::auth::take(&parked.request_id) {
                        cb.cancel();
                    }
                }
            }

            // The approval subwindow's job is done — close it regardless
            // of approve/deny. Best-effort: a failure here just leaves a
            // dead window the human closes manually, never blocks the
            // decision that already took effect above.
            if let Some(window_id) = resolved.window_id {
                if let Err(e) =
                    commands::window::close_window_by_label(state, &serde_json::json!({ "label": window_id }))
                {
                    tracing::warn!("[credential-broker] failed to close approval window: {e}");
                }
            }

            Ok(serde_json::json!(true))
        }
        "browser_panes_set_overlay_clip" => {
            // Apply a clip region to every pane HWND that excludes the given
            // overlay rectangles. The pane stays visible everywhere except
            // under the overlays — DOM overlays render through the holes.
            // Empty list restores full visibility. See
            // BROWSER_PANE_Z_ORDER_FOCUS_REPORT.md Issue 1.
            //
            // `window_label` scopes the clip to panes owned by the
            // requesting window so a modal in window B doesn't also
            // hide panes in window A (Codex P1 on PR #544). Defaults to
            // "main" for back-compat with older frontends that omit it.
            //
            // Each rect: { x, y, w, h } in main-window client pixel coords.
            let window_label = args
                .get("window_label")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string();
            let rects: Vec<(i32, i32, i32, i32)> = args
                .get("rects")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let x = item.get("x")?.as_i64()? as i32;
                            let y = item.get("y")?.as_i64()? as i32;
                            let w = item.get("w")?.as_i64()? as i32;
                            let h = item.get("h")?.as_i64()? as i32;
                            Some((x, y, w, h))
                        })
                        .collect()
                })
                .unwrap_or_default();
            state
                .browser_panes
                .set_pane_overlay_clip(state, &window_label, &rects);
            Ok(serde_json::json!(true))
        }
        "pane_media_revoke" => {
            // User-facing revoke. Two steps, and BOTH are required:
            //
            // 1. drop the grant, so the page cannot silently re-acquire; and
            // 2. reload the pane, which is the ONLY guaranteed way to stop a
            //    capture already in flight. CEF exposes no media-capture
            //    termination API (verified against the headers), and asking the
            //    page to stop its own tracks would make a security control
            //    depend on the page's cooperation.
            //
            // Grant first, then reload — the reverse order lets the reloaded
            // page re-request and be auto-allowed by the grant that is about to
            // be removed.
            //
            // SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md §3.7.
            let block_id = args
                .get("blockId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if block_id.is_empty() {
                return Ok(serde_json::json!(false));
            }
            state.media_grants.lock().clear_pane(&block_id);
            crate::browser_panes::media_prompt::cancel_pane_any_thread(&block_id);
            tracing::info!(
                target: "pane-media",
                %block_id,
                "revoking media access — grants cleared, reloading pane to stop any live capture"
            );
            state.browser_panes.reload(&block_id, state);
            Ok(serde_json::json!(true))
        }
        "pane_media_permission_respond" => {
            // The user's answer to a prompt raised by the browser-pane media
            // permission handler. `allow` records a grant for
            // (pane, origin, exactly the bits requested) and continues the
            // page's getUserMedia; anything else denies.
            //
            // Hops to the CEF UI thread because the parked callback lives in a
            // UI-thread-only registry (CefMediaAccessCallback is neither Send
            // nor Sync). Unknown/duplicate ids resolve to a no-op there — an
            // answer racing the timeout is ordinary, not an error.
            //
            // SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md §3.4.
            let request_id = args
                .get("requestId")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let allow = args
                .get("allow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if request_id == 0 {
                return Ok(serde_json::json!(false));
            }
            let mut task = crate::browser_panes::media_prompt::RespondTask::new(
                request_id,
                allow,
                state.clone(),
            );
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
            Ok(serde_json::json!(true))
        }
        "main_window_focus" => {
            // Move keyboard focus back to the main browser when the user
            // clicks a main-DOM input (address bar, etc). Previously this
            // called SetFocus(top_level) where top_level was the outer CEF
            // Views window — which does NOT route keyboard to the embedded
            // render widget. Keys kept arriving at the pane's HWND.
            //
            // Correct path: tell Chromium that main's Browser has focus.
            // CEF internally calls SetFocus on the right Chrome_RenderWidgetHostHWND.
            // Also defocus every pane browser so Chromium stops routing input
            // to them.
            //
            // `window_label` arg: routes focus to THE window that sent the
            // IPC. Without it, we'd iterate state.browsers and take the
            // first non-pane entry (always `label=main`), so clicking an
            // input in window 2 would reclaim focus to window 1. Default
            // "main" for back-compat if an older frontend omits the arg.
            let window_label = args
                .get("window_label")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string();
            tracing::info!("[ipc] main_window_focus window_label={}", window_label);

            // Phase H.2.b — reducer-aware lookup with fallback. Try the
            // requested label first; on miss, pick any non-pane browser
            // (covers the corner case where the requested window label
            // isn't registered yet — race on first DOM focus before
            // registerBackendWindow lands).
            let target_browser = state
                .get_browser(&window_label)
                .map(|b| (window_label.clone(), b))
                .or_else(|| {
                    state
                        .list_browsers()
                        .into_iter()
                        .find(|(k, _)| !k.starts_with("browser-pane-"))
                });

            if let Some((label, _browser)) = target_browser {
                // Full focus reclaim has to run on the CEF UI thread:
                // `browser_view_get_for_browser`, `host.set_focus`, and
                // walking the HWND tree all require it. The task also
                // handles `defocus_all` on panes.
                let mut task = crate::ui_tasks::MainFocusReclaimTask::new(
                    state.clone(),
                    label.clone(),
                );
                cef::post_task(cef::ThreadId::UI, Some(&mut task));
                tracing::info!("[ipc] main_window_focus: posted MainFocusReclaimTask for label={}", label);
            } else {
                tracing::warn!("[ipc] main_window_focus: no browser found for label={}", window_label);
            }

            Ok(serde_json::json!(true))
        }

        // ---- Unknown command ----
        _ => Err(format!("Unknown command: {}", cmd)),
    }
}

