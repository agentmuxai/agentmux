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
//! ## Auth: host-only, NOT the shared `X-AuthKey`
//!
//! **Every method on this service requires `args[1]` to be this srv
//! instance's `AGENTMUX_HOST_REG_SECRET`**, the same host-only credential
//! `host_ipc.Register` proves (see that module's doc comment and
//! `Config::host_reg_secret`).
//!
//! An earlier cut of this module gated on the instance-wide `X-AuthKey`
//! alone, reasoning that — unlike `host_ipc.Register`'s
//! "session-hijack-shaped" port+token — every method here only touches data
//! scoped to a caller-supplied `identity_id`. **That reasoning was wrong,
//! and it defeated this feature's entire stated security goal** (reagent P0
//! on PR #2824). `X-AuthKey` is deliberately re-injected into every spawned
//! agent's own environment as `AGENTMUX_AUTH_KEY`
//! (`server/agent_handlers/input.rs`), and `AGENTMUX_BLOCKID` is injected
//! alongside it. So any agent could, with a plain `curl` from its own Shell
//! tool:
//!
//!   1. `credential.ResolveIdentity` with its own known `block_id` →
//!      `identity_id`
//!   2. `credential.Fill` with that `identity_id` plus the origin/realm it
//!      already knows (it is, after all, the thing browsing that site) →
//!      **the plaintext password**
//!
//! …with no human approval window anywhere in the path. `Fill` uniquely
//! returns a raw password, which is exactly the class of secret that needed
//! `host_ipc.Register`'s treatment, not a weaker one.
//!
//! The gate is applied to the whole service rather than to `Fill` alone:
//! `ResolveIdentity` leaks identity_ids, `Lookup` confirms which sites have
//! saved credentials, and `Save`/`Delete` let an agent plant or destroy
//! them. None of those are things an agent should reach either, and a
//! uniform gate cannot be defeated by finding the one method someone forgot
//! to cover.
//!
//! Note this proves *caller is the host*, not *a human approved this exact
//! use*. That is the boundary the threat model needs — the host is the
//! process that runs the approval window, so a capability token it mints
//! for itself would add no defence against the attacker in question (an
//! agent), only an internal consistency check within the host.

use serde::Deserialize;

use crate::backend::service::{get_arg, WebCallType, WebReturnType};
use crate::identity::browser_credential_store;

use super::super::AppState;
use super::host_ipc::secret_matches;

pub(super) async fn handle_credential_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    if let Some(rejection) = reject_unless_host_caller(state, call) {
        return rejection;
    }
    match call.method.as_str() {
        "ResolveIdentity" => handle_resolve_identity(state, call),
        "Lookup" => handle_lookup(call),
        "Save" => handle_save(call),
        "Delete" => handle_delete(call),
        "Fill" => handle_fill(call),
        _ => WebReturnType::error(format!("unknown credential method: {}", call.method)),
    }
}

/// `Some(error)` when the caller failed to prove it's the paired host —
/// checked for EVERY method before dispatch, so a new method added later is
/// gated by construction rather than by remembering to gate it.
///
/// Fails closed when srv has no secret configured: with nothing to verify
/// against, "allow" would silently restore the exact bypass this exists to
/// close. Mirrors `host_ipc::handle_register`'s `None` arm.
/// The whole security decision, as a pure function of the two secrets —
/// extracted so it can be tested directly (constructing an `AppState` in a
/// unit test isn't practical, and this gate is far too load-bearing to
/// leave to integration coverage alone).
///
/// `known == None` → always `false`: fail closed. See the caller.
///
/// An EMPTY configured secret is treated the same as an absent one. Without
/// that, `secret_matches("", "")` compares equal and a caller supplying
/// nothing — precisely what an agent without the secret sends — would be
/// admitted, turning the misconfiguration into a skeleton key. `Config`
/// already `.filter(|s| !s.is_empty())`s this value (`config.rs:130`), so
/// today the case is unreachable; the check is here so the gate stays
/// correct on its own terms rather than depending on a filter two modules
/// away that a future refactor could drop.
fn host_caller_allowed(known: Option<&str>, supplied: &str) -> bool {
    match known {
        None => false,
        Some(known) if known.is_empty() => false,
        Some(known) => secret_matches(known, supplied),
    }
}

fn reject_unless_host_caller(state: &AppState, call: &WebCallType) -> Option<WebReturnType> {
    let supplied = call.args.get(1).and_then(|v| v.as_str()).unwrap_or("");
    if host_caller_allowed(state.host_reg_secret.as_deref(), supplied) {
        return None;
    }
    match state.host_reg_secret.as_deref() {
        None => {
            tracing::error!(
                "[credential] REJECTED a {} call — this srv instance has no \
                 AGENTMUX_HOST_REG_SECRET configured, so it cannot verify the caller is \
                 really the paired host. Refusing rather than falling back to the shared \
                 X-AuthKey, which every agent can read from its own environment.",
                call.method
            );
            Some(WebReturnType::error(
                "credential: srv has no host-registration secret configured — refusing",
            ))
        }
        Some(_) => {
            tracing::error!(
                "[credential] REJECTED a {} call — args[1] did not match this srv \
                 instance's AGENTMUX_HOST_REG_SECRET. Either a caller without that \
                 credential (an agent riding the shared X-AuthKey never has it) is \
                 attempting to read stored browser passwords, or a genuine host is out \
                 of sync with srv's current secret (e.g. after a srv recycle).",
                call.method
            );
            Some(WebReturnType::error(
                "credential: args[1] does not match this srv instance's host-registration secret",
            ))
        }
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

    // ── Host-caller gate (reagent P0 on PR #2824) ────────────────────────
    //
    // The attack these close: an agent reads AGENTMUX_AUTH_KEY and
    // AGENTMUX_BLOCKID from its own environment (both injected at spawn) and
    // curls credential.Fill to get a stored plaintext password with no human
    // approval anywhere in the path. The only thing standing between an agent
    // and that password is this gate, so it gets tested directly.

    const KNOWN: &str = "host-registration-secret-value";

    #[test]
    fn the_real_host_secret_is_accepted() {
        assert!(host_caller_allowed(Some(KNOWN), KNOWN));
    }

    /// The exact shape of the bypass: an agent has `X-AuthKey` (which got it
    /// this far) but cannot produce `AGENTMUX_HOST_REG_SECRET`, so it sends
    /// nothing for `args[1]` and `unwrap_or("")` yields an empty string.
    #[test]
    fn an_absent_secret_is_rejected() {
        assert!(!host_caller_allowed(Some(KNOWN), ""));
    }

    #[test]
    fn a_wrong_secret_is_rejected() {
        assert!(!host_caller_allowed(Some(KNOWN), "not-the-secret"));
        assert!(!host_caller_allowed(Some(KNOWN), "host-registration-secret-valu"));
        assert!(!host_caller_allowed(Some(KNOWN), "host-registration-secret-value-extra"));
    }

    /// Fails CLOSED. If srv has no secret configured there is nothing to
    /// verify against, and treating that as "allow" would silently restore
    /// the full bypass on exactly the misconfiguration least likely to be
    /// noticed. Mirrors `host_ipc::handle_register`'s `None` arm.
    #[test]
    fn no_configured_secret_rejects_everything_rather_than_allowing_it() {
        assert!(!host_caller_allowed(None, ""));
        assert!(!host_caller_allowed(None, KNOWN));
        assert!(!host_caller_allowed(None, "anything at all"));
    }

    /// An empty configured secret must not become a skeleton key for callers
    /// that also send nothing — `Some("")` is a misconfiguration, not a
    /// credential, and the empty-vs-empty comparison would otherwise pass.
    #[test]
    fn an_empty_configured_secret_does_not_admit_an_empty_supplied_secret() {
        assert!(!host_caller_allowed(Some(""), ""));
    }
}
