// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `credential` service — the paired CEF host's browser-pane Basic-Auth
//! credential broker calls this, host→srv, to resolve an identity for a
//! pane and to look up / save / delete / fill a stored site credential.
//!
//! **`Fill` is the only method here that ever returns a plaintext
//! password**, and it's called exactly once per approved use, at the
//! moment the host is about to hand the credential to CEF's
//! `AuthCallback::cont()`. `Lookup` deliberately returns only a masked
//! username — the credential-approval window (agentmux-cef,
//! `open_subwindow`-based, see `credential_broker` there) uses `Lookup`
//! to decide what to *display*, and must never receive the real password
//! even transiently.
//!
//! Storage itself lives in [`agentmux_srv::identity::browser_credential_store`]
//! (OS keychain via the existing `secret_store.rs` wrapper) — this module
//! is just the `/agentmux/service` RPC surface over it, following the same
//! shape as [`super::host_ipc`] (the other host→srv service module).
//!
//! Auth: same instance-wide `X-AuthKey` every `/agentmux/service` caller
//! already proves (this route has no per-service auth layer beyond that —
//! see `super::host_ipc`'s own doc comment for why `host_ipc.Register`
//! specifically needed a stronger, second secret; that reasoning doesn't
//! apply here, since every method on this service only ever reads/writes
//! data scoped to a caller-supplied `identity_id`, not a global
//! session-hijack-shaped credential like `host_ipc`'s port+token).

use serde::Deserialize;

use crate::backend::service::{get_arg, WebCallType, WebReturnType};
use crate::identity::browser_credential_store;

use super::super::AppState;

pub(super) async fn handle_credential_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    match call.method.as_str() {
        "ResolveIdentity" => handle_resolve_identity(state, call),
        "Lookup" => handle_lookup(call),
        "Save" => handle_save(call),
        "Delete" => handle_delete(call),
        "Fill" => handle_fill(call),
        _ => WebReturnType::error(format!("unknown credential method: {}", call.method)),
    }
}

#[derive(Deserialize)]
struct ResolveIdentityArgs {
    block_id: String,
}

/// `block_id` → `identity_id`, via the same instance-lookup the identity
/// resolver already uses for OAuth env-var injection at spawn time
/// (`identity::resolver::inject::…`) — reusing
/// `Store::instance_get_active_for_block` here keeps "which identity owns
/// this pane" resolved in exactly one place in the codebase.
///
/// Returns `identity_id: ""` (not an error) when the block has no active
/// instance or the instance has no identity bound — the caller (the CEF
/// host's credential broker) treats an empty identity_id the same as "no
/// stored credential" and falls through to the normal prompt, matching
/// how identity injection itself already treats an unbound instance as a
/// no-op rather than a hard failure.
fn handle_resolve_identity(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: ResolveIdentityArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.ResolveIdentity: {e}")),
    };
    let identity_id = state
        .wstore
        .instance_get_active_for_block(&args.block_id)
        .ok()
        .flatten()
        .map(|inst| inst.identity_id)
        .unwrap_or_default();
    WebReturnType::success(serde_json::json!({ "identity_id": identity_id }))
}

#[derive(Deserialize)]
struct ProtectionSpaceArgs {
    identity_id: String,
    origin: String,
    realm: String,
    is_proxy: bool,
}

fn handle_lookup(call: &WebCallType) -> WebReturnType {
    let args: ProtectionSpaceArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Lookup: {e}")),
    };
    match browser_credential_store::load(&args.identity_id, &args.origin, &args.realm, args.is_proxy) {
        Ok(Some(cred)) => WebReturnType::success(serde_json::json!({
            "found": true,
            "username": mask_username(&cred.username),
        })),
        Ok(None) => WebReturnType::success(serde_json::json!({ "found": false })),
        Err(e) => {
            // Never surface the raw keychain error to the caller beyond a
            // generic message — the exact failure mode (locked keychain,
            // no Secret Service daemon, permission denied) is diagnostic
            // detail for the host's own logs, not something to round-trip
            // back over HTTP. The host's credential_broker treats any
            // error here identically to "not found" (fall through to the
            // normal prompt), so the distinction wouldn't be actionable
            // on that side anyway.
            tracing::warn!("[credential] Lookup failed: {e}");
            WebReturnType::error("lookup failed")
        }
    }
}

/// `us***@example.com` / `ab***` style masking for the approval window's
/// display — enough for a human to recognize which saved login is being
/// offered, not enough to be useful if somehow logged or screenshotted
/// wrong. Keeps at most the first 2 characters before any masking.
fn mask_username(username: &str) -> String {
    let visible: String = username.chars().take(2).collect();
    if username.chars().count() <= 2 {
        return "*".repeat(username.chars().count().max(1));
    }
    format!("{visible}***")
}

#[derive(Deserialize)]
struct SaveArgs {
    identity_id: String,
    origin: String,
    realm: String,
    is_proxy: bool,
    username: String,
    password: String,
}

fn handle_save(call: &WebCallType) -> WebReturnType {
    let args: SaveArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Save: {e}")),
    };
    match browser_credential_store::save(
        &args.identity_id,
        &args.origin,
        &args.realm,
        args.is_proxy,
        &args.username,
        &args.password,
    ) {
        Ok(()) => WebReturnType::success_empty(),
        Err(e) => {
            tracing::warn!("[credential] Save failed: {e}");
            WebReturnType::error("save failed")
        }
    }
}

fn handle_delete(call: &WebCallType) -> WebReturnType {
    let args: ProtectionSpaceArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Delete: {e}")),
    };
    match browser_credential_store::delete(&args.identity_id, &args.origin, &args.realm, args.is_proxy) {
        Ok(()) => WebReturnType::success_empty(),
        Err(e) => {
            tracing::warn!("[credential] Delete failed: {e}");
            WebReturnType::error("delete failed")
        }
    }
}

/// The one method that returns a real, unmasked password. Called exactly
/// once per approved credential use — see this module's own doc comment.
fn handle_fill(call: &WebCallType) -> WebReturnType {
    let args: ProtectionSpaceArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Fill: {e}")),
    };
    match browser_credential_store::load(&args.identity_id, &args.origin, &args.realm, args.is_proxy) {
        Ok(Some(cred)) => WebReturnType::success(serde_json::json!({
            "username": cred.username,
            "password": cred.password,
        })),
        Ok(None) => WebReturnType::error("no stored credential for this identity/origin/realm"),
        Err(e) => {
            tracing::warn!("[credential] Fill failed: {e}");
            WebReturnType::error("fill failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_username_keeps_at_most_two_leading_chars() {
        assert_eq!(mask_username("alice"), "al***");
        assert_eq!(mask_username("bob"), "bo***");
    }

    #[test]
    fn mask_username_handles_very_short_names() {
        assert_eq!(mask_username("a"), "*");
        assert_eq!(mask_username("ab"), "**");
    }

    #[test]
    fn mask_username_handles_empty() {
        assert_eq!(mask_username(""), "*");
    }
}
