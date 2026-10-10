// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-facing UI automation (`UIScreenshot` / `UIClick` / `UIQuery`, and
//! the browser-pane deep-control tools `BrowserNavigate` / `BrowserBack` /
//! `BrowserForward` / `BrowserReload` / `BrowserEval` / `BrowserDispatchKey`
//! / `BrowserFocusElement` / `BrowserFocusInfo`) — proxies to the paired CEF
//! host's `/agentmux/browser/*` CDP routes (`crates/cef/src/browser_api/`).
//! See docs/specs/SPEC_AGENT_BROWSER_PANE_DEEP_CONTROL_2026_09_20.md for the
//! browser-pane tools' own design (they additionally require the caller's
//! own pane to be a dedicated browser pane — enforced host-side, not here).
//!
//! **`block_id` is never a client-supplied field on any request here**
//! (2026-08-19, reagent + Codex review, PR #2662 — a bare client-supplied
//! `block_id` was a real cross-agent content-disclosure vulnerability:
//! `/api/v1/ui/*` shares the same instance-wide `X-AuthKey` every App-API
//! route trusts, and any agent can read that key from its own environment,
//! so no amount of "the MCP schema never exposes it" convention actually
//! stopped a bypassing agent from supplying a different pane's real
//! `block_id`). Every request instead carries `UiAutomationAuth` — an
//! HMAC-SHA256 signature over the caller's own agent_id, using that
//! agent's own `AGENTMUX_JEKT_KEY` (the same per-agent key already used
//! for jekt sender authentication, see `agentmux_common::jekt_sign`).
//! [`verified_block_id`] verifies that signature against the claimed
//! agent's key on file, and — only once verification succeeds — looks up
//! that agent's actual current block_id server-side via the global
//! `ReactiveHandler` registry. The block_id a call actually operates on is
//! never taken from the client at all, so there's nothing left to spoof:
//! an agent without a valid signature for identity X cannot act as X, full
//! stop, regardless of what it claims in the request body.
//!
//! See `agentmux_common::api_types`'s "UI automation" section and
//! `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use serde_json::json;

use agentmux_common::api_types::{
    UiAutomationAuth, UiBrowserDispatchKeyRequest, UiBrowserEvalRequest, UiBrowserFocusElementRequest,
    UiBrowserFocusInfoRequest, UiBrowserHistoryRequest, UiBrowserNavigateRequest, UiBrowserOpenRequest, UiClickRequest,
    UiQueryRequest, UiScreenshotRequest, UiScreenshotResponse,
};

use super::{AppState, HostIpc};

/// Signatures older than this are rejected — bounds replay of a leaked/
/// logged signature to a short window. Mirrors `server/reactive.rs`'s
/// `JEKT_SIG_MAX_AGE_SECS` (same threat model: a host-tier HMAC signature
/// verified locally, not a one-time nonce scheme).
const UI_AUTOMATION_SIG_MAX_AGE_SECS: i64 = 300;

use agentmux_common::time::now_secs as now_unix_secs;

/// Verify `auth` proves the caller genuinely is `auth.agent_id` (via that
/// agent's own jekt key — an attacker without agent_id's key cannot
/// produce a valid signature no matter what it claims), then derive that
/// agent's ACTUAL current block_id server-side. Never trusts a block_id
/// from the client — there isn't one to trust. See this module's own doc
/// comment for the full rationale.
/// `pub(crate)`: also reused by `app_api::pane::handle_close_pane` (and any
/// future own-pane-resolving handler) — the identity-verification mechanism
/// isn't UI-automation-specific, just first built for it. See
/// docs/specs/SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md §5.0.
///
/// Identity M4a-2: `caller` is the request's [`Caller`](crate::server::caller::Caller);
/// `auth.agent_id` is counted against it (`m4.actor_*.ui_auth`), never refused
/// on it — M4d moves this check onto `Caller` (spec §6.5.4).
pub(crate) fn verified_block_id(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    auth: &UiAutomationAuth,
) -> Result<String, String> {
    crate::server::actor::check_actor(
        state,
        caller,
        crate::server::actor::ActorSite::UiAuth,
        Some(&auth.agent_id),
    );
    if auth.ts_secs <= 0 || (now_unix_secs() - auth.ts_secs).abs() > UI_AUTOMATION_SIG_MAX_AGE_SECS
    {
        return Err("signature timestamp missing or outside the freshness window".to_string());
    }
    let key = match state.mstore.agent_jekt_key_load(&auth.agent_id) {
        Ok(Some(k)) => k,
        Ok(None) => {
            return Err(format!(
                "no signing key on file for agent_id={:?} — respawn this agent to get one",
                auth.agent_id
            ))
        }
        Err(e) => return Err(format!("load signing key: {e}")),
    };
    let ok = agentmux_common::jekt_sign::verify_jekt(
        &key,
        "ui-automation-identity",
        &auth.agent_id,
        "__srv__",
        auth.ts_secs,
        "",
        &auth.sig,
    );
    if !ok {
        tracing::warn!(
            agent_id = %auth.agent_id,
            "[ui-automation] REJECTED an invalid identity signature — either a forged \
             request or a stale/corrupted key"
        );
        return Err("identity signature verification failed".to_string());
    }

    crate::backend::reactive::handler::get_global_handler()
        .get_agent(&auth.agent_id)
        .map(|reg| reg.block_id)
        .ok_or_else(|| {
            format!(
                "agent_id={:?} verified but is not currently registered with a pane \
                 (not spawned via AgentMux, or not yet registered)",
                auth.agent_id
            )
        })
}

/// The block a UI-automation request acts on: the caller's own pane, or,
/// when `pane` is given, a browser pane the caller opened with `OpenBrowser`
/// (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §3). The caller's
/// identity is verified first either way; `pane` is then checked against
/// `browser_owner`'s map, never trusted on its own.
pub(crate) fn target_block_id(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    auth: &UiAutomationAuth,
    pane: Option<&str>,
) -> Result<String, (StatusCode, String)> {
    use crate::server::browser_owner::{self, Denied};
    let own = verified_block_id(state, caller, auth).map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    let Some(pane) = pane.map(str::trim).filter(|p| !p.is_empty()) else {
        return not_waiting_on_user(&own).map(|()| own);
    };
    if crate::server::browser_popup::is_window_id(pane) {
        return popup_window_target(state, pane, &auth.agent_id);
    }
    let block = state
        .mstore
        .get::<crate::backend::obj::Block>(pane)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("load pane {pane:?}: {e}")))?;
    match browser_owner::check(block.as_ref(), browser_owner::owner_of(pane).as_deref(), &auth.agent_id, pane) {
        Ok(()) => {
            opener_allows(state, block.as_ref(), &auth.agent_id, pane)?;
            not_waiting_on_user(pane).map(|()| pane.to_string())
        }
        Err(Denied::NotFound(m)) => {
            // The pane is gone: drop any owner entry it left behind.
            browser_owner::forget(pane);
            Err((StatusCode::NOT_FOUND, m))
        }
        Err(Denied::Forbidden(m)) => Err((StatusCode::FORBIDDEN, m)),
    }
}

pub(crate) async fn get_host_ipc(state: &AppState) -> Result<HostIpc, String> {
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
    proxy_to_host_timeout(state, host, route, body, None).await
}

/// `proxy_to_host` with a request timeout of its own, for host routes that
/// legitimately take longer than the shared client's default (a snapshot of
/// a large page, `wait_for`).
pub(crate) async fn proxy_to_host_timeout(
    state: &AppState,
    host: &HostIpc,
    route: &str,
    body: serde_json::Value,
    timeout: Option<std::time::Duration>,
) -> Result<serde_json::Value, String> {
    let url = format!("http://127.0.0.1:{}/agentmux/browser/{route}", host.port);
    let mut request = state
        .http_client
        .post(&url)
        .header("Authorization", format!("Bearer {}", host.token))
        .json(&body);
    if let Some(t) = timeout {
        request = request.timeout(t);
    }
    let resp = request
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

pub(crate) fn err_response(status: StatusCode, e: String) -> Response {
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
/// `<mux_data_dir>/tmp/ui-screenshots/<uuid>.png`, and returns both the
/// path (openable via `OpenMedia`) and the base64 bytes inline.
pub(crate) async fn handle_ui_screenshot(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiScreenshotRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };

    let host_resp = match proxy_to_host(
        &state,
        &host,
        "screenshot",
        json!({ "block_id": block_id }),
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

    let dir = crate::backend::base::get_mux_data_dir().join("tmp/ui-screenshots");
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
        agent_id = %req.auth.agent_id,
        block_id = %block_id,
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
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiClickRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    if let Err(resp) = refuse_on_driven_pane(&block_id, "UIClick", "BrowserSnapshot, then BrowserClick on a reference") {
        return resp;
    }
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };

    let host_resp = match proxy_to_host(
        &state,
        &host,
        "click_element",
        json!({ "block_id": block_id, "selector": req.selector }),
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
        agent_id = %req.auth.agent_id,
        block_id = %block_id,
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
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiQueryRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };

    let host_resp = match proxy_to_host(
        &state,
        &host,
        "query",
        json!({ "block_id": block_id, "selector": req.selector, "limit": req.limit }),
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
        agent_id = %req.auth.agent_id,
        block_id = %block_id,
        selector = %req.selector,
        "[ui-automation] query"
    );
    let data = host_resp.get("data").cloned().unwrap_or(json!({ "matches": [] }));
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// Proxies `route` with `body` and checks the host's `{ok, ...}` envelope.
/// Shared by the write-only browser-pane tools (navigate/back/forward/
/// reload/focus_element/dispatch_key) — they differ only in which
/// route/body they send, not in how they interpret an ack-only reply.
pub(crate) async fn proxy_ack(
    state: &AppState,
    host: &HostIpc,
    route: &str,
    body: serde_json::Value,
) -> Result<(), Response> {
    let host_resp = proxy_to_host(state, host, route, body)
        .await
        .map_err(|e| err_response(StatusCode::BAD_GATEWAY, e))?;
    if host_resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = host_resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown host error")
            .to_string();
        return Err(err_response(StatusCode::BAD_REQUEST, err));
    }
    Ok(())
}

/// `POST /api/v1/ui/browser/navigate` — backs `BrowserNavigate`. Own pane
/// only, and only when it's a dedicated browser pane (checked host-side).
pub(crate) async fn handle_ui_browser_navigate(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserNavigateRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    // Off the pane's site list: refused outright, without asking the person.
    if let Some(e) = crate::server::browser_allowlist::refuse_agent_navigation(&banner_for(&block_id).0, &req.url) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    if let Err(resp) = proxy_ack(
        &state,
        &host,
        "navigate",
        json!({ "block_id": block_id, "url": req.url }),
    )
    .await
    {
        return resp;
    }
    tracing::info!(
        agent_id = %req.auth.agent_id, block_id = %block_id, url = %req.url,
        "[ui-automation] browser navigate"
    );
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/browser/back` — backs `BrowserBack`.
pub(crate) async fn handle_ui_browser_back(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserHistoryRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    if let Err(resp) = proxy_ack(&state, &host, "back", json!({ "block_id": block_id })).await {
        return resp;
    }
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser back");
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/browser/forward` — backs `BrowserForward`.
pub(crate) async fn handle_ui_browser_forward(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserHistoryRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    if let Err(resp) = proxy_ack(&state, &host, "forward", json!({ "block_id": block_id })).await {
        return resp;
    }
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser forward");
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/browser/reload` — backs `BrowserReload`.
pub(crate) async fn handle_ui_browser_reload(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserHistoryRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    if let Err(resp) = proxy_ack(
        &state,
        &host,
        "reload",
        json!({ "block_id": block_id, "ignore_cache": req.ignore_cache.unwrap_or(false) }),
    )
    .await
    {
        return resp;
    }
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser reload");
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/browser/eval` — backs `BrowserEval`. Returns the
/// host's `{result, type, exception}` shape verbatim under `data`, same
/// passthrough pattern `handle_ui_query` uses for `matches`.
pub(crate) async fn handle_ui_browser_eval(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserEvalRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    if let Err(resp) = refuse_on_driven_pane(
        &block_id,
        "BrowserEval",
        "BrowserSnapshot to read the page, and BrowserClick/Fill/Select/Check to act on it",
    ) {
        return resp;
    }
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let host_resp = match proxy_to_host(
        &state,
        &host,
        "eval",
        json!({
            "block_id": block_id,
            "script": req.script,
            "await_promise": req.await_promise.unwrap_or(false),
        }),
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
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser eval");
    let data = host_resp
        .get("data")
        .cloned()
        .unwrap_or(json!({ "result": null, "type": "undefined", "exception": null }));
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// `POST /api/v1/ui/browser/dispatch_key` — backs `BrowserDispatchKey`.
pub(crate) async fn handle_ui_browser_dispatch_key(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserDispatchKeyRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    // Enter submits a form and Space presses the focused button: neither
    // would pass the approval banner, so on a driven pane they're refused.
    let presses = matches!(req.key.as_deref(), Some("Enter") | Some("NumpadEnter") | Some("Space"))
        || req.text.as_deref().is_some_and(|t| t.contains('\n') || t.contains('\r'));
    if presses {
        if let Err(resp) = refuse_on_driven_pane(
            &block_id,
            "BrowserDispatchKey with Enter or Space",
            "BrowserClick on the button (it asks the user when the click submits something)",
        ) {
            return resp;
        }
    }
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    if let Err(resp) = proxy_ack(
        &state,
        &host,
        "dispatch_key",
        json!({
            "block_id": block_id,
            "selector": req.selector,
            "text": req.text,
            "key": req.key,
        }),
    )
    .await
    {
        return resp;
    }
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser dispatch_key");
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/browser/focus_element` — backs `BrowserFocusElement`.
pub(crate) async fn handle_ui_browser_focus_element(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserFocusElementRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    if let Err(resp) = proxy_ack(
        &state,
        &host,
        "focus_element",
        json!({ "block_id": block_id, "selector": req.selector }),
    )
    .await
    {
        return resp;
    }
    tracing::info!(
        agent_id = %req.auth.agent_id, block_id = %block_id, selector = %req.selector,
        "[ui-automation] browser focus_element"
    );
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// `POST /api/v1/ui/browser/focus_info` — backs `BrowserFocusInfo`. Returns
/// the host's `{focused}` shape verbatim under `data`.
pub(crate) async fn handle_ui_browser_focus_info(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserFocusInfoRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let host_resp = match proxy_to_host(&state, &host, "focus_info", json!({ "block_id": block_id })).await {
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
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser focus_info");
    let data = host_resp.get("data").cloned().unwrap_or(json!({ "focused": null }));
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// `POST /api/v1/ui/browser/open` — backs `OpenBrowser`. Opens a browser pane
/// next to the caller's own pane and records the caller as its owner, so the
/// `Browser*`/`UI*` tools accept it as `pane`
/// (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §3).
pub(crate) async fn handle_ui_browser_open(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiBrowserOpenRequest>,
) -> impl IntoResponse {
    let own = match verified_block_id(&state, caller.as_deref(), &req.auth) {
        Ok(b) => b,
        Err(e) => return err_response(StatusCode::UNAUTHORIZED, e),
    };
    let url = req.url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return err_response(
            StatusCode::BAD_REQUEST,
            format!("OpenBrowser takes an http:// or https:// URL, not {url:?}"),
        );
    }
    let split = req.split.as_deref().unwrap_or("right");
    if !matches!(split, "right" | "left" | "up" | "down") {
        return err_response(
            StatusCode::BAD_REQUEST,
            format!("split must be right, left, up or down, not {split:?}"),
        );
    }
    let allowed = match crate::server::browser_allowlist::list_for_open(req.allowed_origins.as_deref(), url) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let mut cmd = crate::backend::rpc_types::CommandPaneOpenData {
        view: "browser".to_string(),
        file: None,
        url: Some(url.to_string()),
        cwd: None,
        title: req.title.clone(),
        tab_id: None,
        split_direction: Some(split.to_string()),
        split_reference_block_id: Some(own.clone()),
        focus: Some(false),
        tree_expanded: None,
        floating: None,
        meta: None,
        skip_placement: None,
        stack_onto_block_id: None,
        connection: None,
        auth: None,
        reuse_editor_pane: None,
        select: None,
        line: None,
    };
    let mut meta = match crate::server::app_api::pane::build_pane_meta(&cmd) {
        Ok(m) => m,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    meta.insert(
        crate::server::browser_owner::OWNER_META_KEY.to_string(),
        json!(req.auth.agent_id),
    );
    crate::server::browser_allowlist::mirror(&mut meta, allowed.as_deref());
    cmd.meta = Some(meta);
    let result = match crate::server::app_api::open_pane(&state, cmd).await {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    crate::server::browser_allowlist::limit_new_pane(&state, &result.block_id, allowed).await;
    crate::server::browser_owner::record(&result.block_id, &req.auth.agent_id);
    tracing::info!(
        agent_id = %req.auth.agent_id, own_block = %own, pane = %result.block_id, url = %url,
        "[ui-automation] browser open (agent-owned)"
    );
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "data": { "pane": result.block_id } })),
    )
        .into_response()
}

/// Proxy a browser-pane request to the host and pass its `data` through:
/// the shared tail of the snapshot and act handlers.
async fn proxy_value(
    state: &AppState,
    route: &str,
    body: serde_json::Value,
    timeout: std::time::Duration,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let host = get_host_ipc(state).await.map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    let host_resp = proxy_to_host_timeout(state, &host, route, body, Some(timeout))
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    if host_resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = host_resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown host error")
            .to_string();
        return Err((StatusCode::BAD_REQUEST, err));
    }
    Ok(host_resp.get("data").cloned().unwrap_or(serde_json::Value::Null))
}

async fn proxy_data(
    state: &AppState,
    route: &str,
    body: serde_json::Value,
    timeout: std::time::Duration,
) -> axum::response::Response {
    match proxy_value(state, route, body, timeout).await {
        Ok(data) => (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response(),
        Err((code, e)) => err_response(code, e),
    }
}

/// `POST /api/v1/ui/browser/snapshot` — backs `BrowserSnapshot`
/// (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §4.1).
pub(crate) async fn handle_ui_browser_snapshot(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserSnapshotRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser snapshot");
    let mut data = match proxy_value(
        &state,
        "snapshot",
        json!({ "block_id": block_id, "scope": req.scope }),
        std::time::Duration::from_secs(30),
    )
    .await
    {
        Ok(d) => d,
        Err((code, e)) => return err_response(code, e),
    };
    // The popups this page opened, which an agent finds no other way
    // (SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md §3.4).
    let popups = popup_listing(&state, &block_id, &req.auth.agent_id);
    if let Some(obj) = data.as_object_mut() {
        obj.insert("popups".to_string(), json!(popups));
    }
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// `opener`'s open popups for its snapshot: each one's pane id, address, and
/// whether `agent_id` may drive it (not if the person took it over).
pub(crate) fn popup_listing(state: &AppState, opener: &str, agent_id: &str) -> Vec<serde_json::Value> {
    use crate::backend::obj::Block;
    let load = |id: &str| state.mstore.get::<Block>(id).ok().flatten();
    crate::server::browser_popup::open_popups(opener, |id| load(id).is_some())
        .into_iter()
        .filter_map(|pane| {
            let block = load(&pane)?;
            let url = block.meta.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let yours = owning_agent(&block, &pane).is_some_and(|a| a.eq_ignore_ascii_case(agent_id));
            Some(json!({ "pane": pane, "url": url, "yours": yours, "kind": "pane" }))
        })
        .chain(crate::server::browser_popup::windows_of(opener).into_iter().map(|(id, w)| {
            let yours = popup_window_target(state, &id, agent_id).is_ok();
            json!({ "pane": id, "url": w.url, "yours": yours, "kind": "window" })
        }))
        .collect()
}

/// `POST /api/v1/ui/browser/set_files` — backs `BrowserSetFiles`. Only files
/// inside the calling agent's own workspace (its block's launch directory,
/// host agents only) are allowed (spec §5.3).
pub(crate) async fn handle_ui_browser_set_files(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserSetFilesRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let own = match verified_block_id(&state, caller.as_deref(), &req.auth) {
        Ok(b) => b,
        Err(e) => return err_response(StatusCode::UNAUTHORIZED, e),
    };
    // Where srv launched the agent, not its block's `cmd:cwd`, which any
    // client can rewrite (spec §5.3).
    let workspace = crate::server::browser_uploads::agent_workspace(&own);
    let Some(workspace) = workspace else {
        return err_response(
            StatusCode::FORBIDDEN,
            "uploads come only from an agent's own workspace, and this agent has none \
             (a container agent, or no launch directory)"
                .to_string(),
        );
    };
    // The page gets the bytes srv read through each file's checked handle,
    // never a path the browser would open later (spec §5.3). Up to 25 MB of
    // file I/O and base64: off the async workers.
    let requested = req.paths.clone();
    let read = tokio::task::spawn_blocking(move || {
        use base64::Engine as _;
        let paths = crate::server::browser_uploads::check_upload_paths(&workspace, &requested)?;
        let files = crate::server::browser_uploads::read_upload_files(&workspace, &paths)?;
        Ok::<_, String>(
            files
                .iter()
                .map(|f| {
                    json!({
                        "name": f.name,
                        "type": f.mime,
                        "data": base64::engine::general_purpose::STANDARD.encode(&f.bytes),
                    })
                })
                .collect::<Vec<serde_json::Value>>(),
        )
    })
    .await;
    let files = match read {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return err_response(StatusCode::FORBIDDEN, e),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("reading the files failed: {e}")),
    };
    let names: Vec<&str> = files.iter().filter_map(|f| f["name"].as_str()).collect();
    tracing::info!(
        agent_id = %req.auth.agent_id, block_id = %block_id, r#ref = %req.ref_, files = ?names,
        "[ui-automation] browser upload"
    );
    proxy_data(
        &state,
        "set_files",
        json!({ "block_id": block_id, "ref": req.ref_, "files": files }),
        std::time::Duration::from_secs(60),
    )
    .await
}

/// `POST /api/v1/ui/browser/wait_for` — backs `BrowserWaitFor`.
pub(crate) async fn handle_ui_browser_wait_for(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserWaitForRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let timeout_ms = req.timeout_ms.unwrap_or(10_000).clamp(250, 60_000);
    proxy_data(
        &state,
        "wait_for",
        json!({
            "block_id": block_id,
            "text": req.text,
            "url_contains": req.url_contains,
            "gone": req.gone,
            "timeout_ms": timeout_ms,
        }),
        std::time::Duration::from_millis(timeout_ms + 10_000),
    )
    .await
}

/// On a browser pane an agent drives (opened with `OpenBrowser`), refuse the
/// general-purpose routes that could submit, send or delete without the
/// approval banner (spec §5.4): arbitrary script, clicks by CSS selector,
/// and the keys that press buttons. The reference tools do the same work
/// with the approval check in front of committing clicks.
#[allow(clippy::result_large_err)]
fn refuse_on_driven_pane(block_id: &str, what: &str, instead: &str) -> Result<(), axum::response::Response> {
    // A popup window has no owner record of its own (it is driven through the
    // pane that opened it), and it is always driven: the same refusal holds.
    if crate::server::browser_popup::is_window_id(block_id) {
        return Err(err_response(
            StatusCode::FORBIDDEN,
            format!(
                "{what} isn't allowed on a popup window: it could submit a form without the \
                 user's approval. Use {instead}."
            ),
        ));
    }
    if crate::server::browser_owner::owner_of(block_id).is_none() {
        return Ok(());
    }
    Err(err_response(
        StatusCode::FORBIDDEN,
        format!(
            "{what} isn't allowed on a browser pane you opened with OpenBrowser: it could submit \
             a form without the user's approval. Use {instead}."
        ),
    ))
}

/// A popup pane (`browser:popup_of`) is driven as part of its opener: while
/// the opener exists, the agent must still own it (the person's Take over on
/// the opener ends its hold on the popups it opened too), and the opener
/// must not be waiting on the person (a hand-off there pauses its popups).
/// A popup whose opener was closed stands on its own.
fn opener_allows(
    state: &AppState,
    block: Option<&crate::backend::obj::Block>,
    agent_id: &str,
    pane: &str,
) -> Result<(), (StatusCode, String)> {
    // Every pane up the chain, not just the one that opened this popup: a
    // popup opened by a popup is still driven as part of the pane the chain
    // started in, so Take over or a hand-off anywhere above stops it.
    let mut next = block.and_then(popup_of);
    for _ in 0..POPUP_CHAIN_LIMIT {
        let Some(opener) = next else {
            return Ok(());
        };
        let Ok(Some(opener_block)) = state.mstore.get::<crate::backend::obj::Block>(&opener) else {
            // The rest of the chain was closed: from here the popup stands on its own.
            return Ok(());
        };
        if !owning_agent(&opener_block, &opener).is_some_and(|a| a.eq_ignore_ascii_case(agent_id)) {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "the user took over pane {opener:?}, which opened popup {pane:?} (directly or \
                     through other popups); open a new pane with OpenBrowser if you still need a browser"
                ),
            ));
        }
        not_waiting_on_user(&opener)?;
        next = popup_of(&opener_block);
    }
    if next.is_none() {
        return Ok(());
    }
    Err((StatusCode::FORBIDDEN, format!("popup {pane:?} is too many popups deep to drive")))
}

/// How many openers up a popup's chain the checks follow. A chain this deep
/// is a page opening popups from popups on purpose; driving stops there.
const POPUP_CHAIN_LIMIT: usize = 8;

/// The pane that opened `block` as a popup pane, if it is one.
fn popup_of(block: &crate::backend::obj::Block) -> Option<String> {
    block
        .meta
        .get(crate::server::browser_popup::POPUP_OF_META_KEY)
        .and_then(|v| v.as_str())
        .filter(|o| !o.is_empty())
        .map(str::to_string)
}

/// The pane a chain of popups started in: `pane` itself unless it is a popup
/// pane, else its opener's root, as far up as the panes still exist. Popups
/// are counted over its whole tree, so a popup opening popups shares its
/// root's cap rather than getting its own.
fn chain_root(state: &AppState, pane: &str) -> String {
    let mut root = pane.to_string();
    for _ in 0..POPUP_CHAIN_LIMIT {
        let Some(up) = state.mstore.get::<crate::backend::obj::Block>(&root).ok().flatten().as_ref().and_then(popup_of) else {
            break;
        };
        if !matches!(state.mstore.get::<crate::backend::obj::Block>(&up), Ok(Some(_))) {
            break;
        }
        root = up;
    }
    root
}

/// A popup window (a native window a pane's page opened, id `popup-…`) may be
/// driven by the agent recorded when it opened, while that agent still owns
/// the pane that opened it and that pane isn't waiting on the person
/// (SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §5).
fn popup_window_target(state: &AppState, id: &str, agent_id: &str) -> Result<String, (StatusCode, String)> {
    let Some(w) = crate::server::browser_popup::window(id) else {
        return Err((StatusCode::NOT_FOUND, format!("no popup window {id:?} (it may have been closed)")));
    };
    if !w.owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(agent_id)) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "popup window {id:?} isn't yours: a popup window belongs to the agent that owned \
                 the pane that opened it"
            ),
        ));
    }
    let Ok(Some(opener)) = state.mstore.get::<crate::backend::obj::Block>(&w.opener) else {
        return Err((StatusCode::NOT_FOUND, format!("the pane that opened popup window {id:?} is gone")));
    };
    if !owning_agent(&opener, &w.opener).is_some_and(|a| a.eq_ignore_ascii_case(agent_id)) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "the user took over pane {:?}, which opened popup window {id:?}; open a new pane \
                 with OpenBrowser if you still need a browser",
                w.opener
            ),
        ));
    }
    not_waiting_on_user(&w.opener)?;
    // The opener may itself be a popup pane: the whole chain above it counts.
    opener_allows(state, Some(&opener), agent_id, id)?;
    Ok(id.to_string())
}

/// Where a request to the person about `target` shows: in its own banner,
/// or, for a popup window (which has no AgentMux header), in its opener's,
/// with the window's address so the banner can say which window it means.
pub(crate) fn banner_for(target: &str) -> (String, Option<String>) {
    if crate::server::browser_popup::is_window_id(target) {
        if let Some(w) = crate::server::browser_popup::window(target) {
            return (w.opener, Some(w.url));
        }
    }
    (target.to_string(), None)
}

/// Refuse a tool call on a pane that is waiting for the user: a hand-off
/// (they're working in it) or an approval (they're reading the form the
/// approved click will send, which mustn't change under them).
fn not_waiting_on_user(pane: &str) -> Result<(), (StatusCode, String)> {
    use crate::server::browser_attention::{waiting_on_user, Kind};
    match waiting_on_user(pane) {
        None => Ok(()),
        Some(Kind::Handoff) => Err((
            StatusCode::CONFLICT,
            format!(
                "the user is working in pane {pane:?} (your BrowserHandoff); your tools on it are \
                 paused until they click Done or Cancel"
            ),
        )),
        Some(Kind::Approval) => Err((
            StatusCode::CONFLICT,
            format!(
                "pane {pane:?} is waiting for the user to approve your click; your tools on it are \
                 paused until they answer"
            ),
        )),
        Some(Kind::Navigation) => Err((
            StatusCode::CONFLICT,
            format!("the page in pane {pane:?} tried to leave its allowed_origins and the user is being asked whether to allow it; your tools on it are paused until they answer"),
        )),
    }
}

/// `POST /api/v1/ui/browser/handoff` — backs `BrowserHandoff` (spec §5.2):
/// a banner in the pane asks the user to do something the agent mustn't
/// (sign in, a CAPTCHA), and the call waits for their Done or Cancel. The
/// agent's tools on that pane are refused meanwhile (`target_block_id`).
pub(crate) async fn handle_ui_browser_handoff(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserHandoffRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    // A popup window's request shows on its opener's banner.
    let (banner_at, in_window) = banner_for(&block_id);
    // Only a browser pane shows the banner; anywhere else the request would
    // lock the agent's own tools with nothing for the user to click.
    let is_browser = state
        .mstore
        .get::<crate::backend::obj::Block>(&banner_at)
        .ok()
        .flatten()
        .is_some_and(|b| b.meta.get("view").and_then(|v| v.as_str()) == Some("browser"));
    if !is_browser {
        return err_response(
            StatusCode::BAD_REQUEST,
            "BrowserHandoff needs a browser pane: pass the `pane` OpenBrowser returned".to_string(),
        );
    }
    let reason: String = req.reason.trim().chars().take(300).collect();
    if reason.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "say what the user should do (`reason`)".to_string());
    }
    // Only the host relays the user's Done or Cancel (`/api/v1/host/browser_attention`).
    // With none connected nobody can answer: refuse now rather than hold the
    // agent's tools on this pane until the timeout.
    if let Err(e) = crate::server::app_api::connections::host_to_ask_user(&state).await {
        return err_response(StatusCode::SERVICE_UNAVAILABLE, e);
    }
    let minutes = req.timeout_minutes.unwrap_or(15).clamp(1, 60);
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser hand-off");
    let answer = match crate::server::browser_attention::ask(
        &state,
        &banner_at,
        &req.auth.agent_id,
        crate::server::browser_attention::Kind::Handoff,
        json!({ "reason": reason, "window": in_window }),
        std::time::Duration::from_secs(minutes * 60),
    )
    .await
    {
        Ok((a, _waiting)) => a,
        Err(e) => return err_response(StatusCode::CONFLICT, e),
    };
    let answer = match answer {
        crate::server::browser_attention::Answer::Yes => "done",
        crate::server::browser_attention::Answer::Cancelled => "cancelled",
        crate::server::browser_attention::Answer::TimedOut => "timed_out",
    };
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "answer": answer } }))).into_response()
}

/// `POST /api/v1/host/browser_attention` — the user's answer to a hand-off
/// or approval banner, relayed by the CEF host. Only the host can call it:
/// `X-Host-Token` must be the IPC token the host registered with
/// (`host_ipc.Register`), which agents never see; the instance auth key
/// alone, which every agent has, isn't enough.
pub(crate) async fn handle_host_browser_attention(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can answer a pane's request".to_string());
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (block_id, id, decision) = (s("block_id"), s("id"), s("decision"));
    let Some(answer) = crate::server::browser_attention::Answer::parse(&decision) else {
        return err_response(StatusCode::BAD_REQUEST, format!("unknown decision {decision:?}"));
    };
    match crate::server::browser_attention::resolve(&id, &block_id, answer) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            // Nothing waits on that banner any more: take it down.
            crate::server::browser_attention::clear_stale(&state, &block_id, &id);
            err_response(StatusCode::NOT_FOUND, e)
        }
    }
}

/// Is this request from the CEF host? `X-Host-Token` must be the IPC token
/// the host registered with (`host_ipc.Register`), which agents never see;
/// the instance auth key alone, which every agent has, isn't enough.
pub(crate) async fn from_host(state: &AppState, headers: &axum::http::HeaderMap) -> bool {
    let presented = headers.get("x-host-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    let registered = state.host_ipc.lock().await.clone().map(|h| h.token).unwrap_or_default();
    !registered.is_empty()
        && !presented.is_empty()
        && agentmux_common::secret_eq::secret_eq(presented.as_bytes(), registered.as_bytes())
}

/// The agent that owns `block_id`, if one does and the person hasn't taken
/// the pane over: the same test every `pane` argument passes.
fn owning_agent(block: &crate::backend::obj::Block, block_id: &str) -> Option<String> {
    let agent = crate::server::browser_owner::owner_of(block_id)?;
    crate::server::browser_owner::check(Some(block), Some(agent.as_str()), &agent, block_id)
        .ok()
        .map(|()| agent)
}

/// `POST /api/v1/host/browser_popup` — a browser pane's page opened a popup
/// (docs/specs/SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md). Only the host
/// calls it, from `on_before_popup`. Answers `{admitted: true, pane}` when
/// srv opened the popup as a pane beside its opener, or `{admitted: false,
/// reason}`, and the host opens it in the system browser as it always did.
pub(crate) async fn handle_host_browser_popup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use crate::server::browser_popup as popup;
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can report a popup".to_string());
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let (opener, url, opener_url) = (s("opener"), s("url"), s("opener_url"));
    let user_gesture = body.get("user_gesture").and_then(|v| v.as_bool()).unwrap_or(false);
    let refused = |reason: &str| {
        (StatusCode::OK, Json(json!({ "ok": true, "data": { "admitted": false, "reason": reason } }))).into_response()
    };
    let block = match state.mstore.get::<crate::backend::obj::Block>(&opener) {
        Ok(Some(b)) => b,
        Ok(None) => return refused("the pane that opened it is gone"),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("load pane {opener:?}: {e}")),
    };
    if block.meta.get("view").and_then(|v| v.as_str()) != Some("browser") {
        return refused("it was not opened by a browser pane");
    }
    // The opener's owner, if the whole chain above it (when the opener is a
    // popup pane itself) is still that agent's: a popup from a popup of a pane
    // the person took over belongs to nobody.
    let owner = owning_agent(&block, &opener)
        .filter(|agent| opener_allows(&state, Some(&block), agent, &opener).is_ok());
    if let Some(why) = crate::server::browser_allowlist::refuse_popup(&opener, &url) { return refused(why); }
    // The count and the slot are taken together: two popups reported at once
    // can't both fit under the cap. The slot is given back if the pane
    // doesn't open.
    let root = chain_root(&state, &opener);
    let reservation = match popup::reserve(
        &opener,
        &root,
        |id| matches!(state.mstore.get::<crate::backend::obj::Block>(id), Ok(Some(_))),
        |taken| popup::decide(&url, &opener_url, user_gesture, owner.is_some(), taken),
    ) {
        Ok(r) => r,
        Err(r) => {
            tracing::info!(opener = %opener, url = %url, reason = r.reason(), "[browser-popup] not opened as a pane");
            return refused(r.reason());
        }
    };

    let mut cmd = crate::backend::rpc_types::CommandPaneOpenData {
        view: "browser".to_string(),
        file: None,
        url: Some(url.clone()),
        cwd: None,
        title: None,
        tab_id: None,
        split_direction: Some("right".to_string()),
        split_reference_block_id: Some(opener.clone()),
        // The person clicked: show them the popup. An agent's popup opens
        // beside its pane without taking the person's focus, like OpenBrowser.
        focus: Some(owner.is_none()),
        tree_expanded: None,
        floating: None,
        meta: None,
        skip_placement: None,
        stack_onto_block_id: None,
        connection: None,
        auth: None,
        reuse_editor_pane: None,
        select: None,
        line: None,
    };
    let mut meta = match crate::server::app_api::pane::build_pane_meta(&cmd) {
        Ok(m) => m,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    meta.insert(popup::POPUP_OF_META_KEY.to_string(), json!(opener));
    meta.insert(popup::POPUP_FROM_META_KEY.to_string(), json!(popup::origin_of(&opener_url)));
    // On the new pane from the start, so the host has it before the first load.
    crate::server::browser_allowlist::mirror(&mut meta, crate::server::browser_allowlist::list_for(&opener).as_deref());
    if let Some(agent) = &owner {
        meta.insert(crate::server::browser_owner::OWNER_META_KEY.to_string(), json!(agent));
    }
    cmd.meta = Some(meta);
    let result = match crate::server::app_api::open_pane(&state, cmd).await {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    crate::server::browser_allowlist::join_popup_pane(&state, &result.block_id, &opener).await;
    // The opener's owner owns its popup: written here, from srv's own record,
    // never from anything the page or a client said. Checked again now the
    // pane is open: the person may have taken the opener over meanwhile, and
    // then the agent gets neither.
    let mut owner = owner;
    if let Some(agent) = owner.clone() {
        let still = state
            .mstore
            .get::<crate::backend::obj::Block>(&opener)
            .ok()
            .flatten()
            .and_then(|b| owning_agent(&b, &opener))
            .is_some_and(|a| a.eq_ignore_ascii_case(&agent));
        if still {
            crate::server::browser_owner::record(&result.block_id, &agent);
        } else {
            owner = None;
            let mut clear = crate::backend::obj::MetaMapType::new();
            clear.insert(crate::server::browser_owner::OWNER_META_KEY.to_string(), serde_json::Value::Null);
            if let Err(e) = crate::server::http_shell::broadcast_meta_update(&state, &result.block_id, &clear) {
                tracing::warn!(pane = %result.block_id, error = %e, "[browser-popup] couldn't clear the popup's owner");
            }
        }
    }
    reservation.commit(&result.block_id);
    tracing::info!(
        opener = %opener, pane = %result.block_id, url = %url, owner = ?owner,
        "[browser-popup] popup opened as a pane"
    );
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "data": { "admitted": true, "pane": result.block_id } })),
    )
        .into_response()
}

/// `POST /api/v1/host/browser_popup_window` — the host created, navigated
/// or closed a popup window (a native window a browser pane's page opened;
/// SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §4). Body
/// `{event: "opened"|"navigated"|"closed", popup, opener, url}`. Only the
/// host can call it. The window's owner is the opener's, from srv's own
/// record; the opener's popup-windows strip is kept current.
pub(crate) async fn handle_host_browser_popup_window(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use crate::server::browser_popup as popup;
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can report a popup window".to_string());
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let (event, id, opener, url) = (s("event"), s("popup"), s("opener"), s("url"));
    if !popup::is_window_id(&id) {
        return err_response(StatusCode::BAD_REQUEST, format!("not a popup window id: {id:?}"));
    }
    let touched = match event.as_str() {
        "opened" => {
            let block = match state.mstore.get::<crate::backend::obj::Block>(&opener) {
                Ok(Some(b)) if b.meta.get("view").and_then(|v| v.as_str()) == Some("browser") => b,
                // Not from a browser pane srv knows: nobody drives it.
                _ => return (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
            };
            let owner = owning_agent(&block, &opener)
                .filter(|agent| opener_allows(&state, Some(&block), agent, &opener).is_ok());
            tracing::info!(popup = %id, opener = %opener, url = %url, owner = ?owner, "[browser-popup] popup window opened");
            popup::window_opened(&id, &opener, &url, owner);
            Some(opener)
        }
        "navigated" => popup::window_navigated(&id, &url),
        "closed" => {
            tracing::info!(popup = %id, "[browser-popup] popup window closed");
            popup::window_closed(&id)
        }
        other => return err_response(StatusCode::BAD_REQUEST, format!("unknown event {other:?}")),
    };
    if let Some(opener) = touched {
        update_popup_windows_strip(&state, &opener);
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// Forget every popup window and clear every opener's strip. Called when the
/// host registers: after either side restarts, the windows srv knew of are
/// gone or no longer known, and the strip, which is saved with the pane, must
/// not keep listing them.
pub(crate) fn reset_popup_windows(state: &AppState) {
    use crate::server::browser_popup as popup;
    // The windows opened from this srv's panes (in a test, other srvs share
    // the record).
    popup::clear_windows(|opener| matches!(state.mstore.get::<crate::backend::obj::Block>(opener), Ok(Some(_))));
    let blocks = match state.mstore.get_all::<crate::backend::obj::Block>() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "[browser-popup] couldn't list panes to clear popup-window strips");
            return;
        }
    };
    for b in blocks.into_iter().filter(|b| b.meta.get(popup::POPUP_WINDOWS_META_KEY).is_some_and(|v| !v.is_null())) {
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert(popup::POPUP_WINDOWS_META_KEY.to_string(), serde_json::Value::Null);
        if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, &b.oid, &meta) {
            tracing::debug!(block = %b.oid, error = %e, "[browser-popup] stale popup-window strip not cleared");
        }
    }
}

/// Rewrite `opener`'s popup-windows strip (`browser:popup_windows`). The
/// opener may be closing; then there's nothing to update.
fn update_popup_windows_strip(state: &AppState, opener: &str) {
    use crate::server::browser_popup as popup;
    let value = popup::strip_value(opener);
    let value = if value.as_array().is_some_and(|a| a.is_empty()) { serde_json::Value::Null } else { value };
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(popup::POPUP_WINDOWS_META_KEY.to_string(), value);
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, opener, &meta) {
        tracing::debug!(opener = %opener, error = %e, "[browser-popup] popup-windows strip not updated");
    }
}

/// `POST /api/v1/ui/browser/act` — backs `BrowserClick`, `BrowserFill`,
/// `BrowserSelect` and `BrowserCheck` (spec §4.2).
pub(crate) async fn handle_ui_browser_act(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserActRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    // The typed text isn't logged: it can be anything the agent was given.
    tracing::info!(
        agent_id = %req.auth.agent_id, block_id = %block_id, action = %req.action, r#ref = %req.ref_,
        "[ui-automation] browser act"
    );
    // `approved` carries what the user was shown: the host checks the form
    // still matches it before clicking (the page's own script could have
    // changed it while they read the banner).
    let body = |approved: Option<&serde_json::Value>| {
        json!({
            "block_id": block_id,
            "ref": req.ref_,
            "action": req.action,
            "text": req.text,
            "option": req.option,
            "checked": req.checked,
            "approved": approved.is_some(),
            "approved_as": approved,
        })
    };
    let first = match proxy_value(&state, "act", body(None), std::time::Duration::from_secs(20)).await {
        Ok(d) => d,
        Err((code, e)) => return err_response(code, e),
    };
    // A committing click (submit, send, pay, delete, ...) waits for the
    // user's approval in the pane; only their click there lets it through
    // (spec §5.4).
    let Some(ask) = first.pointer("/after/needs_approval").cloned() else {
        return (StatusCode::OK, Json(json!({ "ok": true, "data": first }))).into_response();
    };
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser act waits for approval");
    // `_waiting` keeps the pane locked until the approved click is done, so
    // the form can't change between what the user approved and what's sent.
    // The banner shows the summary, not the fingerprint of everything the
    // click sends (hidden controls included): that stays here, for the check
    // the host makes before the approved click.
    let (banner_at, in_window) = banner_for(&block_id);
    let mut banner = ask.clone();
    if let Some(b) = banner.as_object_mut() {
        b.remove("fingerprint");
        b.insert("window".to_string(), json!(in_window));
    }
    let owner_before = crate::server::browser_owner::owner_of(&banner_at);
    let (answer, _waiting) = match crate::server::browser_attention::ask(
        &state,
        &banner_at,
        &req.auth.agent_id,
        crate::server::browser_attention::Kind::Approval,
        banner,
        std::time::Duration::from_secs(10 * 60),
    )
    .await
    {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::CONFLICT, e),
    };
    match answer {
        // The user may have taken the pane over while the banner was up:
        // their Take over wins over an Approve that comes after it.
        crate::server::browser_attention::Answer::Yes
            if crate::server::browser_owner::owner_of(&banner_at) != owner_before =>
        {
            err_response(
                StatusCode::FORBIDDEN,
                "the user took over this pane, so the approved click wasn't made".to_string(),
            )
        }
        crate::server::browser_attention::Answer::Yes => {
            proxy_data(&state, "act", body(Some(&ask)), std::time::Duration::from_secs(20)).await
        }
        crate::server::browser_attention::Answer::Cancelled => err_response(
            StatusCode::FORBIDDEN,
            "the user didn't approve this action: don't retry it; ask them what to do".to_string(),
        ),
        crate::server::browser_attention::Answer::TimedOut => err_response(
            StatusCode::REQUEST_TIMEOUT,
            "the user didn't answer the approval within 10 minutes; ask them in chat before trying again".to_string(),
        ),
    }
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
