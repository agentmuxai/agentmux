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
//! The sign-ins are Vault `login` entries kept per browser profile
//! ([`crate::backend::vault::signins`], spec
//! SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.4, §9.3).
//! The host still calls the pane's key `identity_id`; it is now the tab's
//! profile, `personal` or `profile:<id>`, and the host passes it back unread.
//! This module is the `/agentmux/service` RPC surface over them, in the same
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

use crate::backend::obj::Block;
use crate::backend::service::{get_arg, WebCallType, WebReturnType};
use crate::backend::vault::signins::{self, Space};

use super::super::AppState;
use super::host_ipc::secret_matches;

pub(super) async fn handle_credential_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    if let Some(rejection) = reject_unless_host_caller(state, call) {
        return rejection;
    }
    match call.method.as_str() {
        "ResolveIdentity" => handle_resolve_identity(state, call),
        "Lookup" => handle_lookup(state, call),
        "Save" => handle_save(state, call),
        "Delete" => handle_delete(state, call),
        "Fill" => handle_fill(state, call),
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
/// already `.filter(|s| !s.is_empty())`s this value (`host_reg_secret` in
/// `config.rs`'s `Config::from_env_and_args`), so
/// today the case is unreachable; the check is here so the gate stays
/// correct on its own terms rather than depending on a filter two modules
/// away that a future refactor could drop.
pub(crate) fn host_caller_allowed(known: Option<&str>, supplied: &str) -> bool {
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

/// `block_id` → the profile its sign-ins are kept under: `personal` for a tab
/// with no `browser:identity`, `profile:<id>` for a named profile's tab.
///
/// Returns `identity_id: ""` (not an error) for an Incognito tab, which never
/// saves a sign-in (spec §9.1), and for a block that is gone. The host's
/// credential broker treats an empty one as "no stored credential" and falls
/// through to the normal prompt, saving nothing.
fn handle_resolve_identity(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: ResolveIdentityArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.ResolveIdentity: {e}")),
    };
    let profile = state
        .mstore
        .get::<Block>(&args.block_id)
        .ok()
        .flatten()
        .and_then(|b| signins::profile_for_tab(b.meta.get(crate::server::browser_identity::IDENTITY_META_KEY).and_then(|v| v.as_str())))
        .unwrap_or_default();
    WebReturnType::success(serde_json::json!({ "identity_id": profile }))
}

#[derive(Deserialize)]
struct ProtectionSpaceArgs {
    identity_id: String,
    origin: String,
    realm: String,
    is_proxy: bool,
}

impl ProtectionSpaceArgs {
    /// Where the sign-in is used; `None` for a pane with no profile, which keeps none.
    fn space(self) -> Option<Space> {
        signins::is_profile(&self.identity_id).then(|| Space { profile: self.identity_id, origin: self.origin, realm: self.realm, proxy: self.is_proxy })
    }
}

fn handle_lookup(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: ProtectionSpaceArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Lookup: {e}")),
    };
    let Some(space) = args.space() else { return WebReturnType::success(serde_json::json!({ "found": false })) };
    match signins::find(&state.identity_store, &space) {
        Ok(Some((_, username))) => WebReturnType::success(serde_json::json!({
            "found": true,
            "username": mask_username(&username),
        })),
        Ok(None) => WebReturnType::success(serde_json::json!({ "found": false })),
        Err(e) => {
            // The exact failure is for srv's log, not the caller: the host's
            // credential broker treats any error here as "not found" and falls
            // through to the normal prompt.
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

fn handle_save(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: SaveArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Save: {e}")),
    };
    let password = zeroize::Zeroizing::new(args.password);
    if !signins::is_profile(&args.identity_id) {
        return WebReturnType::error("save failed: this pane keeps no sign-ins");
    }
    let space = Space { profile: args.identity_id, origin: args.origin, realm: args.realm, proxy: args.is_proxy };
    match signins::save(&state.identity_store, &space, &args.username, &password) {
        Ok(_) => {
            crate::server::vault_handlers::publish(state);
            WebReturnType::success_empty()
        }
        Err(e) => {
            tracing::warn!("[credential] Save failed: {e}");
            WebReturnType::error("save failed")
        }
    }
}

/// The site turned down a sign-in just filled: the Vault keeps it, marked, and
/// stops offering it (see [`signins::refused`]); nothing is deleted.
fn handle_delete(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: ProtectionSpaceArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Delete: {e}")),
    };
    let Some(space) = args.space() else { return WebReturnType::success_empty() };
    match signins::refused(&state.identity_store, &space) {
        Ok(()) => {
            crate::server::vault_handlers::publish(state);
            WebReturnType::success_empty()
        }
        Err(e) => {
            tracing::warn!("[credential] Delete failed: {e}");
            WebReturnType::error("delete failed")
        }
    }
}

/// The one method that returns a real, unmasked password. Called exactly
/// once per approved credential use — see this module's own doc comment.
fn handle_fill(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: ProtectionSpaceArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("credential.Fill: {e}")),
    };
    let Some(space) = args.space() else { return WebReturnType::error("no stored credential for this pane") };
    match signins::fill(&state.identity_store, &space) {
        Ok(Some((username, password))) => {
            crate::server::vault_handlers::publish(state);
            WebReturnType::success(serde_json::json!({
                "username": username,
                "password": password.as_str(),
            }))
        }
        Ok(None) => WebReturnType::error("no stored credential for this profile/origin/realm"),
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
