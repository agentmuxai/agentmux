// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Credential-isolated browser-pane auto-fill.
//!
//! Orchestrates `on_auth_credentials` (`client::crash_recovery`) against
//! the srv-side `credential` service (identity resolve → keychain lookup)
//! and, when a stored credential exists, a human-approval subwindow
//! (never a `<Modal>` — see [`approval`]'s doc comment and the plan's
//! "Why this shape" for why a `<Modal>` would leak the Approve control to
//! any agent's `UIQuery`/`UIClick`, not just the one whose credential is
//! being approved). See `docs/status/majestic-painting-minsky` plan.
//!
//! Every failure mode here — srv unreachable, keychain error, no stored
//! credential, window-creation failure — falls through to the existing,
//! unmodified `browser-pane-auth-required` prompt. This module can only
//! ever make auth *more* automatic, never break the baseline flow.

pub mod approval;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;
use tokio::runtime::Handle;

use crate::state::AppState;

/// Installed once from `main.rs`, alongside `browser_pane::auth`'s own
/// handle — CEF invokes `on_auth_credentials` on its IO thread, which has
/// no `Handle::current()`, so a bare `tokio::spawn` there would panic.
/// [`approval`]'s TTL timer shares this same handle via [`runtime_handle`]
/// rather than keeping a second copy.
static TOKIO_HANDLE: OnceLock<Handle> = OnceLock::new();

pub fn set_runtime_handle(h: Handle) {
    let _ = TOKIO_HANDLE.set(h);
}

pub(crate) fn runtime_handle() -> Option<Handle> {
    TOKIO_HANDLE.get().cloned()
}

/// Closes an approval subwindow by label. Installed once from `lib.rs`
/// alongside [`set_runtime_handle`], for the same reason a handle is: the
/// paths that need to close a window (notably [`approval`]'s TTL timer)
/// run on tasks that hold neither `AppState` nor a window handle.
///
/// A hook rather than a direct `commands::window` call from [`approval`] so
/// that module keeps its stated design property — it holds `window_id`
/// strings and nothing else, with no dependency on window management.
type WindowCloser = Box<dyn Fn(&str) + Send + Sync + 'static>;
static WINDOW_CLOSER: OnceLock<WindowCloser> = OnceLock::new();

pub fn set_window_closer(f: impl Fn(&str) + Send + Sync + 'static) {
    let _ = WINDOW_CLOSER.set(Box::new(f));
}

/// Best-effort — a failure (or a closer that was never installed) leaves a
/// dead window for the human to close, which is strictly better than the
/// previous behaviour of always leaving it. Never propagates: every caller
/// is on a cleanup path where there is nothing useful to do with an error.
pub(crate) fn close_approval_window(window_id: &str) {
    match WINDOW_CLOSER.get() {
        Some(f) => f(window_id),
        None => tracing::error!(
            "[credential-broker] no window closer installed; approval subwindow {} will \
             be left open. Did lib.rs call credential_broker::set_window_closer?",
            window_id
        ),
    }
}

/// srv is on the same machine (`127.0.0.1`) — a slow/unreachable srv must
/// never add a long stall to auth challenges that have no stored
/// credential at all (the overwhelming common case, now sitting in this
/// module's critical path for every challenge, not just ones this feature
/// actually helps). 3s comfortably covers a legitimately slow keychain
/// prompt (e.g. an OS consent dialog) without making a genuinely dead srv
/// feel like a hang.
const SRV_CALL_TIMEOUT: Duration = Duration::from_secs(3);

/// Entry point called from `on_auth_credentials` right after it parks the
/// CEF `AuthCallback` in `browser_pane::auth` (unchanged). Spawns the
/// async identity-resolve → keychain-lookup → (approval-window | fall
/// through) sequence; `on_auth_credentials` itself still returns
/// synchronously to CEF regardless of what this decides.
pub fn on_auth_challenge(
    state: Arc<AppState>,
    request_id: String,
    block_id: String,
    origin: String,
    host: String,
    port: i32,
    realm: String,
    is_proxy: bool,
) {
    match runtime_handle() {
        Some(handle) => {
            handle.spawn(run_challenge(state, request_id, block_id, origin, host, port, realm, is_proxy));
        }
        None => {
            tracing::error!(
                "[credential-broker] tokio handle not installed — falling through to the \
                 manual prompt for request_id {}. Did main.rs call \
                 credential_broker::set_runtime_handle?",
                request_id
            );
            fall_through_to(&state, &block_id, &request_id, &origin, &host, port, &realm, is_proxy);
        }
    }
}

async fn run_challenge(
    state: Arc<AppState>,
    request_id: String,
    block_id: String,
    origin: String,
    host: String,
    port: i32,
    realm: String,
    is_proxy: bool,
) {
    let fall_through = || fall_through_to(&state, &block_id, &request_id, &origin, &host, port, &realm, is_proxy);

    let identity_id = match call_credential_service(
        &state,
        "ResolveIdentity",
        serde_json::json!({ "block_id": block_id }),
    )
    .await
    {
        Ok(data) => data
            .get("identity_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        Err(e) => {
            tracing::warn!("[credential-broker] ResolveIdentity failed: {e} — falling through");
            fall_through();
            return;
        }
    };
    if identity_id.is_empty() {
        // No identity bound to this pane's instance — nothing to look up.
        fall_through();
        return;
    }

    // A re-challenge for the exact same protection space shortly after we
    // auto-filled it means the stored credential was wrong (CEF gives no
    // "auth succeeded" signal to hook — this is the only observable
    // proxy). Delete it and fall through rather than looping the same bad
    // credential forever.
    let recent_key = recent_fill::key(&identity_id, &origin, &realm, is_proxy);
    if recent_fill::take_if_recent(&recent_key) {
        tracing::info!(
            "[credential-broker] re-challenge within {:?} of an auto-fill for origin={} — \
             treating the stored credential as wrong; deleting and falling through",
            recent_fill::WINDOW,
            origin,
        );
        let _ = call_credential_service(
            &state,
            "Delete",
            serde_json::json!({
                "identity_id": identity_id, "origin": origin, "realm": realm, "is_proxy": is_proxy,
            }),
        )
        .await;
        fall_through();
        return;
    }

    let lookup = match call_credential_service(
        &state,
        "Lookup",
        serde_json::json!({
            "identity_id": identity_id, "origin": origin, "realm": realm, "is_proxy": is_proxy,
        }),
    )
    .await
    {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("[credential-broker] Lookup failed: {e} — falling through");
            fall_through();
            return;
        }
    };
    let found = lookup.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
    if !found {
        fall_through();
        return;
    }
    let masked_username = lookup
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match approval::reserve_or_join(
        &identity_id,
        &origin,
        &host,
        port,
        &realm,
        is_proxy,
        request_id.clone(),
        block_id.clone(),
    ) {
        approval::Reservation::Joined => {
            tracing::info!(
                "[credential-broker] request_id {} joined an already-pending approval for \
                 origin={}",
                request_id,
                origin,
            );
        }
        approval::Reservation::New { approval_id } => {
            match open_approval_window(&state, &block_id, &approval_id, &origin, &realm, is_proxy, &masked_username)
            {
                Ok(window_id) => {
                    // The reservation can be cancelled (pane closed) or time
                    // out during `open_approval_window`'s synchronous gap —
                    // no lock is held across it. If that happened, the window
                    // now belongs to nothing: `cancel_for_window` can't match
                    // it, the decide-handler's `take` returns `None`, and the
                    // TTL entry is already gone — so nothing else will ever
                    // close it (reagent P1 on PR #2824). Close it here, and
                    // fall this request through to the manual prompt rather
                    // than leaving it parked for a decision that can no
                    // longer be made.
                    if !approval::finalize_window(&approval_id, window_id.clone()) {
                        close_approval_window(&window_id);
                        fall_through();
                    }
                }
                Err(e) => {
                    // Fall through EVERY request riding this reservation, not
                    // just our own. Another same-protection-space challenge can
                    // join on a different Tokio worker while
                    // `open_approval_window` runs (no lock is held across it),
                    // and it may belong to a different pane — so each needs a
                    // prompt addressed to its own `block_id`. Discarding
                    // `abandon`'s return value left those joined requests
                    // parked with no prompt until `browser_pane::auth`'s TTL
                    // fired (codex P2 / reagent P2 on PR #2824).
                    tracing::warn!(
                        "[credential-broker] failed to open approval window for origin={}: {e} \
                         — falling through",
                        origin,
                    );
                    let mut parked = approval::abandon(&approval_id);
                    // `abandon` returns nothing if the entry was already gone
                    // (pane closed / TTL) during the window-creation gap. Our
                    // own request still needs its prompt either way, and must
                    // not be prompted twice if it is in the list.
                    if !parked.iter().any(|p| p.request_id == request_id) {
                        parked.push(approval::ParkedRequest {
                            request_id: request_id.clone(),
                            block_id: block_id.clone(),
                        });
                    }
                    for p in &parked {
                        fall_through_to(&state, &p.block_id, &p.request_id, &origin, &host, port, &realm, is_proxy);
                    }
                }
            }
        }
    }
}

/// Emit the same `browser-pane-auth-required` event `on_auth_credentials`
/// emitted unconditionally before this feature existed — the unmodified
/// baseline path, reached from every early-return above.
pub(crate) fn fall_through_to(
    state: &Arc<AppState>,
    block_id: &str,
    request_id: &str,
    origin: &str,
    host: &str,
    port: i32,
    realm: &str,
    is_proxy: bool,
) {
    crate::events::emit_event_to_top_level_windows(
        state,
        "browser-pane-auth-required",
        &serde_json::json!({
            "block_id": block_id,
            "request_id": request_id,
            "origin": origin,
            "host": host,
            "port": port,
            "realm": realm,
            "is_proxy": is_proxy,
        }),
    );
}

/// Open the credential-approval subwindow, a genuinely separate top-level
/// CEF window/DOM (never a `<Modal>`) — see this module's own doc comment
/// for why. Parented to whichever top-level window currently owns
/// `block_id`, so it closes along with that window
/// (`client::lifecycle::on_before_close` → `approval::cancel_for_window`,
/// wired in a follow-up commit) if the human never decides.
fn open_approval_window(
    state: &Arc<AppState>,
    block_id: &str,
    approval_id: &str,
    origin: &str,
    realm: &str,
    is_proxy: bool,
    masked_username: &str,
) -> Result<String, String> {
    let parent_label = state
        .browser_pane_window_label(block_id)
        .ok_or_else(|| format!("no owning window found for block {block_id}"))?;

    let meta = serde_json::json!({
        "approval_id": approval_id,
        "origin": origin,
        "realm": realm,
        "is_proxy": is_proxy,
        "masked_username": masked_username,
    })
    .to_string();

    let result = crate::commands::window::open_subwindow(
        state,
        parent_label,
        Some("credential-approval"),
        Some(&meta),
    )?;
    result
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "open_subwindow returned a non-string window label".to_string())
}

/// Called once a human approves in the subwindow, right before `cb.cont()`
/// — the ONE place a plaintext password crosses into this process. Marks
/// the protection space as "recently filled" so a fast re-challenge (a
/// wrong saved credential) is recognized by [`run_challenge`] above.
pub async fn fill_credential(
    state: &Arc<AppState>,
    identity_id: &str,
    origin: &str,
    realm: &str,
    is_proxy: bool,
) -> Result<(String, String), String> {
    let data = call_credential_service(
        state,
        "Fill",
        serde_json::json!({
            "identity_id": identity_id, "origin": origin, "realm": realm, "is_proxy": is_proxy,
        }),
    )
    .await?;
    let username = data
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or("credential.Fill: response missing username")?
        .to_string();
    let password = data
        .get("password")
        .and_then(|v| v.as_str())
        .ok_or("credential.Fill: response missing password")?
        .to_string();
    recent_fill::mark(recent_fill::key(identity_id, origin, realm, is_proxy));
    Ok((username, password))
}

/// Called from the `browser_pane_auth_save` IPC handler after a human
/// manually submits credentials with the "save this credential" checkbox
/// checked (`frontend/app/view/browser/use-browser-auth.ts`). Resolves
/// `block_id` → `identity_id` itself rather than requiring the caller to
/// — the renderer never learns `identity_id`, only `block_id`.
pub async fn save_credential(
    state: &Arc<AppState>,
    block_id: &str,
    origin: &str,
    realm: &str,
    is_proxy: bool,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let identity_id = call_credential_service(state, "ResolveIdentity", serde_json::json!({ "block_id": block_id }))
        .await?
        .get("identity_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if identity_id.is_empty() {
        return Err("no identity bound to this pane — nothing to save the credential under".to_string());
    }
    call_credential_service(
        state,
        "Save",
        serde_json::json!({
            "identity_id": identity_id, "origin": origin, "realm": realm, "is_proxy": is_proxy,
            "username": username, "password": password,
        }),
    )
    .await?;
    Ok(())
}

#[derive(Deserialize)]
struct WebReturn {
    success: bool,
    error: Option<String>,
    data: Option<serde_json::Value>,
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// POST `{"service": "credential", "method": ..., "args": [args0, secret]}`
/// to `/agentmux/service`, same endpoint + `X-AuthKey` auth
/// `client::helpers::backend_close_window` already uses for this
/// direction — async via `reqwest` here rather than that raw-TCP style,
/// since this call site is already fully async (unlike
/// `backend_close_window`'s non-async caller).
///
/// `args[1]` is `AGENTMUX_HOST_REG_SECRET` — srv rejects every method on
/// this service without it. `X-AuthKey` alone is NOT sufficient and must
/// never be treated as such here: it's injected into every agent's own
/// environment, so gating on it alone let any agent `curl` the plaintext
/// password straight out of `credential.Fill` (reagent P0 on PR #2824).
/// See `agentmux-srv/src/server/service/credential.rs`'s module doc.
///
/// Sent as a positional arg rather than a header to match
/// `host_ipc.Register`'s existing shape (`args[2]` there), keeping both
/// host→srv host-proving calls in one idiom.
async fn call_credential_service(
    state: &Arc<AppState>,
    method: &str,
    args0: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    if web_endpoint.is_empty() {
        return Err("backend web_endpoint not yet configured".to_string());
    }
    let auth_key = state.auth_key.lock().clone();
    let host_reg_secret = state.host_reg_secret.lock().clone();
    if host_reg_secret.is_empty() {
        // Every caller of this function already falls through to the manual
        // prompt on Err, so failing here degrades to "no auto-fill" rather
        // than breaking auth — the same posture as every other failure mode
        // in this feature.
        return Err(format!(
            "credential.{method}: host has no AGENTMUX_HOST_REG_SECRET — cannot prove \
             host identity to srv, falling through to the manual prompt"
        ));
    }
    let url = format!("{}/agentmux/service", web_endpoint.trim_end_matches('/'));
    let body = serde_json::json!({
        "service": "credential",
        "method": method,
        "args": [args0, host_reg_secret],
        "uicontext": serde_json::Value::Null,
    });

    let resp = http_client()
        .post(&url)
        .header("X-AuthKey", auth_key)
        .json(&body)
        .timeout(SRV_CALL_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("credential.{method}: request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("credential.{method}: HTTP {}", resp.status()));
    }

    let parsed: WebReturn = resp
        .json()
        .await
        .map_err(|e| format!("credential.{method}: bad response body: {e}"))?;
    if !parsed.success {
        return Err(parsed.error.unwrap_or_else(|| format!("credential.{method}: unknown error")));
    }
    Ok(parsed.data.unwrap_or(serde_json::Value::Null))
}

/// Short-TTL "this protection space was just auto-filled" marker, used
/// only to detect a wrong stored credential (see [`run_challenge`]'s use
/// of `take_if_recent`). Deliberately separate from [`approval`]'s
/// registry — different lifetime (survives past the approval itself,
/// which is already resolved and gone by the time this matters) and
/// different key shape (no request/window association at all, just a
/// timestamp per protection space).
mod recent_fill {
    use super::*;

    pub const WINDOW: Duration = Duration::from_secs(15);

    fn marks() -> &'static Mutex<HashMap<String, Instant>> {
        static CELL: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
        CELL.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Not a cryptographic key — just a namespaced join of fields already
    /// scoped by identity_id (unlike `browser_credential_store::account_key`,
    /// this key never leaves the process, so delimiter-collision hardening
    /// isn't worth the extra complexity here; a false-positive collision
    /// only ever causes an extra fall-through prompt, never a credential
    /// leak).
    pub fn key(identity_id: &str, origin: &str, realm: &str, is_proxy: bool) -> String {
        format!("{identity_id}\u{0}{origin}\u{0}{realm}\u{0}{is_proxy}")
    }

    pub fn mark(key: String) {
        marks().lock().insert(key, Instant::now());
    }

    /// If `key` was marked within the last [`WINDOW`], consume the mark
    /// (so a THIRD challenge doesn't also read as "recent") and return
    /// true. Also opportunistically drops any other stale entries so this
    /// map can't grow unbounded across a long session.
    pub fn take_if_recent(key: &str) -> bool {
        let mut g = marks().lock();
        let now = Instant::now();
        g.retain(|_, t| now.duration_since(*t) < WINDOW);
        g.remove(key).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::recent_fill;

    #[test]
    fn recent_fill_marks_and_consumes_once() {
        let key = recent_fill::key("id", "https://a", "realm", false);
        assert!(!recent_fill::take_if_recent(&key), "unmarked key must not read as recent");
        recent_fill::mark(key.clone());
        assert!(recent_fill::take_if_recent(&key), "marked key must read as recent");
        assert!(!recent_fill::take_if_recent(&key), "take_if_recent must consume the mark");
    }

    #[test]
    fn recent_fill_key_distinguishes_protection_spaces() {
        let a = recent_fill::key("id", "https://a", "realm", false);
        let b = recent_fill::key("id", "https://b", "realm", false);
        assert_ne!(a, b);
    }
}
