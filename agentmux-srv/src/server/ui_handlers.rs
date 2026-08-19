// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-facing UI automation (`UIScreenshot` / `UIClick` / `UIQuery` MCP
//! tools) — proxies to the paired CEF host's `/agentmux/browser/*` CDP
//! routes (`agentmux-cef/src/browser_api/`). `block_id` on every request is
//! stamped by agentmux-mcp from its own trusted `AGENTMUX_BLOCKID` env when
//! called through the MCP tool schema (which never exposes `block_id` as
//! an agent-settable argument), so a well-behaved caller is scoped to its
//! own pane by construction.
//!
//! **This is not a server-side enforcement boundary.** `/api/v1/ui/*`
//! shares the same instance-wide `X-AuthKey` every other App-API route
//! uses, and that key is present in an agent's own process environment —
//! an agent could bypass the MCP wrapper entirely and POST a DIFFERENT
//! (real) pane's block_id directly, and this endpoint has no way to tell
//! that block_id doesn't actually belong to the caller (Codex review,
//! PR #2662, 2026-08-19). `query.js`'s `__amq_allowed_for` scoping (fixed
//! in the same PR to also block containment, not just direct nesting) only
//! protects a caller from reaching outside WHICHEVER block_id it supplied
//! — it can't detect that the supplied block_id was never the caller's own
//! in the first place. Closing that fully requires a per-agent credential
//! distinguishing "this specific agent" at the HTTP layer, which doesn't
//! exist anywhere in the App-API surface today (this endpoint doesn't
//! introduce the gap, it inherits an existing one) — tracked as follow-up,
//! not fixed in this PR.
//!
//! See `agentmux_common::api_types`'s "UI automation" section and
//! `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use serde_json::json;

use agentmux_common::api_types::{UiClickRequest, UiQueryRequest, UiScreenshotRequest, UiScreenshotResponse};

use super::{AppState, HostIpc};

async fn get_host_ipc(state: &AppState) -> Result<HostIpc, String> {
    state.host_ipc.lock().await.clone().ok_or_else(|| {
        "this AgentMux instance's CEF host has not registered its UI-automation \
         credentials yet (host_ipc.Register) — try again in a moment"
            .to_string()
    })
}

/// POST `body` to `/agentmux/browser/{route}` on the paired host and return
/// its parsed JSON response (the host's own `{ok, data}` / `{ok, error}`
/// envelope — see `browser_api::types::ApiResponse`).
async fn proxy_to_host(
    state: &AppState,
    host: &HostIpc,
    route: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let url = format!("http://127.0.0.1:{}/agentmux/browser/{route}", host.port);
    let resp = state
        .http_client
        .post(&url)
        .header("Authorization", format!("Bearer {}", host.token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("proxy to host {route}: {e}"))?;
    if resp.status() == StatusCode::UNAUTHORIZED {
        return Err(
            "host rejected our ipc_token — stale registration after a host restart? \
             will self-heal on the host's next host_ipc.Register call"
                .to_string(),
        );
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("parse host {route} response: {e}"))
}

fn err_response(status: StatusCode, e: String) -> Response {
    (status, Json(json!({ "ok": false, "error": e }))).into_response()
}

/// Delete `*.png` files in `dir` older than [`SCREENSHOT_RETENTION`]. Called
/// on every `UIScreenshot` write since this directory only ever grows
/// otherwise (reagent P2, PR #2662) — no background scheduler needed for a
/// directory nothing else writes to. Best-effort: any I/O error for an
/// individual entry (permissions, a concurrent delete, a non-UTF8 name) is
/// skipped rather than failing the screenshot request that triggered it.
const SCREENSHOT_RETENTION: std::time::Duration = std::time::Duration::from_secs(60 * 60);

fn prune_old_screenshots(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(modified) = metadata.modified() else { continue };
        let Ok(age) = now.duration_since(modified) else { continue };
        if age > SCREENSHOT_RETENTION {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// `POST /api/v1/ui/screenshot` — backs the `UIScreenshot` MCP tool.
/// Captures a PNG clipped to the caller's own pane, writes it to
/// `<wave_data_dir>/tmp/ui-screenshots/<uuid>.png`, and returns both the
/// path (openable via `OpenMedia`) and the base64 bytes inline.
pub(crate) async fn handle_ui_screenshot(
    State(state): State<AppState>,
    Json(req): Json<UiScreenshotRequest>,
) -> impl IntoResponse {
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };

    let host_resp = match proxy_to_host(
        &state,
        &host,
        "screenshot",
        json!({ "block_id": req.block_id }),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    if host_resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = host_resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown host error")
            .to_string();
        return err_response(StatusCode::BAD_REQUEST, err);
    }
    let png_base64 = host_resp
        .get("data")
        .and_then(|d| d.get("png_base64"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if png_base64.is_empty() {
        return err_response(
            StatusCode::BAD_GATEWAY,
            "host returned no png_base64".to_string(),
        );
    }

    let bytes = match base64::engine::general_purpose::STANDARD.decode(&png_base64) {
        Ok(b) => b,
        Err(e) => {
            return err_response(StatusCode::BAD_GATEWAY, format!("decode png_base64: {e}"))
        }
    };

    let dir = crate::backend::base::get_wave_data_dir().join("tmp/ui-screenshots");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create screenshots dir: {e}"),
        );
    }
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&path, &bytes) {
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write screenshot: {e}"),
        );
    }
    // Unbounded growth over repeated/looped verification calls otherwise
    // (reagent P2, PR #2662, 2026-08-19) — no expiry mechanism existed at
    // all. Best-effort, on the write path rather than a background task:
    // simple and sufficient for a directory that's only ever written here.
    prune_old_screenshots(&dir);

    tracing::info!(
        block_id = %req.block_id,
        path = %path.display(),
        "[ui-automation] screenshot"
    );

    (
        StatusCode::OK,
        Json(UiScreenshotResponse {
            path: path.to_string_lossy().to_string(),
            png_base64,
        }),
    )
        .into_response()
}

/// `POST /api/v1/ui/click` — backs the `UIClick` MCP tool. Synthesizes a
/// real mouse click at the first `selector` match within the caller's own
/// pane subtree.
pub(crate) async fn handle_ui_click(
    State(state): State<AppState>,
    Json(req): Json<UiClickRequest>,
) -> impl IntoResponse {
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };

    let host_resp = match proxy_to_host(
        &state,
        &host,
        "click_element",
        json!({ "block_id": req.block_id, "selector": req.selector }),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    if host_resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = host_resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown host error")
            .to_string();
        return err_response(StatusCode::BAD_REQUEST, err);
    }

    tracing::info!(
        block_id = %req.block_id,
        selector = %req.selector,
        "[ui-automation] click"
    );
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/query` — backs the `UIQuery` MCP tool. Returns matched
/// elements (tag/text/attrs/rect/focused) within the caller's own pane
/// subtree — the same shape `browser_api::types::QueryData` returns.
pub(crate) async fn handle_ui_query(
    State(state): State<AppState>,
    Json(req): Json<UiQueryRequest>,
) -> impl IntoResponse {
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };

    let host_resp = match proxy_to_host(
        &state,
        &host,
        "query",
        json!({ "block_id": req.block_id, "selector": req.selector, "limit": req.limit }),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    if host_resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = host_resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown host error")
            .to_string();
        return err_response(StatusCode::BAD_REQUEST, err);
    }

    tracing::info!(block_id = %req.block_id, selector = %req.selector, "[ui-automation] query");
    let data = host_resp.get("data").cloned().unwrap_or(json!({ "matches": [] }));
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::{prune_old_screenshots, SCREENSHOT_RETENTION};

    #[test]
    fn prune_old_screenshots_deletes_only_stale_pngs() {
        let dir = tempfile::tempdir().unwrap();

        let fresh = dir.path().join("fresh.png");
        std::fs::write(&fresh, b"png").unwrap();

        let stale = dir.path().join("stale.png");
        std::fs::write(&stale, b"png").unwrap();
        let old_time = std::time::SystemTime::now() - (SCREENSHOT_RETENTION * 2);
        let file = std::fs::File::options().write(true).open(&stale).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old_time))
            .unwrap();

        // Non-PNG files must never be touched, however old.
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, b"keep me").unwrap();
        let file = std::fs::File::options().write(true).open(&other).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old_time))
            .unwrap();

        prune_old_screenshots(dir.path());

        assert!(fresh.exists(), "fresh screenshot must survive pruning");
        assert!(!stale.exists(), "stale screenshot must be pruned");
        assert!(other.exists(), "non-png files must never be pruned");
    }
}
