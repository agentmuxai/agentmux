// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP Basic / Digest auth callback registry.
//!
//! When CEF fires `RequestHandler::get_auth_credentials` for a browser
//! pane, the host returns `1` (will-async-respond), parks the
//! `AuthCallback` in this registry keyed by a generated `request_id`,
//! and broadcasts a `browser-pane-auth-required` event to the renderer.
//! The renderer prompts the user and replies via the
//! `browser_pane_auth_submit` / `browser_pane_auth_cancel` IPC
//! commands, which resolve the callback here and complete the CEF
//! flow.
//!
//! Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.

use cef::{AuthCallback, ImplAuthCallback};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::runtime::Handle;

/// One parked auth challenge — the CEF callback plus the block_id
/// owning it (so pane-close can clean up just its entries) and a
/// monotonically-increasing arming epoch (so a delayed timeout task
/// can detect "this request was already resolved + re-registered
/// under the same id" and bail). The epoch is a defensive guard;
/// uuid request_ids shouldn't collide.
struct Entry {
    block_id: String,
    callback: AuthCallback,
    epoch: u64,
}

/// HashMap::new is not const-fn so the static needs lazy init.
fn pending() -> &'static Mutex<HashMap<String, Entry>> {
    static CELL: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_EPOCH: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Tokio runtime Handle captured from `main.rs` so `register()` can
/// schedule the TTL timer from any thread — CEF invokes
/// `get_auth_credentials` on its IO thread, which has no
/// `Handle::current()`, so a bare `tokio::spawn(...)` would panic
/// with "there is no reactor running".
static TOKIO_HANDLE: OnceLock<Handle> = OnceLock::new();

/// Install the Tokio runtime Handle. Called once from `main.rs` after
/// `Runtime::new()` and before CEF starts dispatching callbacks.
pub fn set_runtime_handle(h: Handle) {
    let _ = TOKIO_HANDLE.set(h);
}

/// Maximum time a callback can sit parked before we cancel it
/// automatically. Bounds the leak if the renderer never replies
/// (background tab + suspended JS, hung modal). 5 minutes is well
/// above any realistic credential-entry time and well below "user
/// noticed and wondered what happened."
const PARKED_TTL: Duration = Duration::from_secs(5 * 60);

/// Park a CEF auth callback under `request_id`. The renderer will
/// resolve it shortly via `submit` / `cancel`. A 5-minute timeout
/// task is armed alongside so the entry can't leak indefinitely if
/// the renderer never replies. Replaces any prior entry for the same
/// id (shouldn't happen — ids are uuid4).
pub fn register(request_id: String, block_id: String, cb: AuthCallback) {
    use std::sync::atomic::Ordering;
    let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    {
        let mut g = pending().lock();
        if g.insert(
            request_id.clone(),
            Entry { block_id, callback: cb, epoch },
        ).is_some() {
            tracing::warn!(
                "[browser-pane-auth] duplicate request_id {} — overwriting",
                request_id
            );
        }
    }
    // Arm the TTL. A delayed task that runs `cancel + remove` if the
    // entry's epoch still matches when the timeout fires. If the
    // renderer resolved the request before the timeout, the epoch
    // check fails and the task is a no-op.
    //
    // Spawn via the stored Handle — `tokio::spawn` would panic here
    // because CEF calls this from its IO thread, which has no
    // current runtime. If the handle isn't installed (init order
    // bug), log and skip the TTL rather than crash the host.
    let rid = request_id;
    match TOKIO_HANDLE.get() {
        Some(handle) => {
            handle.spawn(async move {
                tokio::time::sleep(PARKED_TTL).await;
                let to_cancel: Option<AuthCallback> = {
                    let mut g = pending().lock();
                    match g.get(&rid) {
                        Some(entry) if entry.epoch == epoch => g.remove(&rid).map(|e| e.callback),
                        _ => None,
                    }
                };
                if let Some(cb) = to_cancel {
                    tracing::warn!(
                        "[browser-pane-auth] request_id {} timed out after {:?} — auto-cancel",
                        rid, PARKED_TTL
                    );
                    cb.cancel();
                }
            });
        }
        None => {
            tracing::error!(
                "[browser-pane-auth] tokio handle not installed; TTL timer skipped for request_id {} \
                 — callback will leak if the renderer never replies. Did main.rs call set_runtime_handle?",
                rid
            );
        }
    }
}

/// Pop the callback for `request_id`. Returns None if it was already
/// resolved (e.g. submit + cancel race) or never existed.
pub fn take(request_id: &str) -> Option<AuthCallback> {
    pending().lock().remove(request_id).map(|e| e.callback)
}

/// Cancel every callback parked for `block_id` — called from
/// `browser_pane_close` so closing a pane mid-prompt doesn't leak
/// CEF refcounts. Returns the number cancelled.
pub fn cancel_for_block(block_id: &str) -> usize {
    let to_cancel: Vec<AuthCallback> = {
        let mut g = pending().lock();
        let ids: Vec<String> = g
            .iter()
            .filter(|(_, e)| e.block_id == block_id)
            .map(|(k, _)| k.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| g.remove(&id))
            .map(|e| e.callback)
            .collect()
    };
    let n = to_cancel.len();
    for cb in to_cancel {
        cb.cancel();
    }
    if n > 0 {
        tracing::info!(
            "[browser-pane-auth] cancelled {} pending auth(s) for block {}",
            n,
            block_id.chars().take(7).collect::<String>(),
        );
    }
    n
}
