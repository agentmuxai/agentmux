// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `host_ipc` service — the paired CEF host pushes its own CDP-automation
//! credentials (`ipc_port` + `ipc_token`, gating `/agentmux/browser/*` on
//! the host's own IPC server) here once, at the host's own startup, right
//! after it learns this srv instance's address. srv has no other way to
//! learn these values — the host generates `ipc_token` for itself and is
//! the sole source of truth.
//!
//! Backs the `/api/v1/ui/{screenshot,click,query}` proxy routes (agent-
//! facing UI automation). See `AppState::host_ipc`,
//! `agentmux-cef/src/client/helpers.rs::register_ipc_with_backend`, and
//! `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`.
//!
//! `Register` requires a third argument, a shared secret
//! (`AppState::host_reg_secret` / `AGENTMUX_HOST_REG_SECRET`) known only to
//! srv and the paired host — never to any agent — proving the caller is
//! really the host and not an agent process riding the same instance-wide
//! `X-AuthKey`. See `handle_register`'s doc comments for the full threat
//! model (reagent P0, PR #2662, 2026-08-19).

use crate::backend::service::{WebCallType, WebReturnType};

use super::super::{AppState, HostIpc};

pub(super) async fn handle_host_ipc_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    match call.method.as_str() {
        "Register" => handle_register(state, call).await,
        _ => WebReturnType::error(format!("unknown host_ipc method: {}", call.method)),
    }
}

/// Constant-time comparison of the caller-supplied secret against
/// `state.host_reg_secret`, via the same "HMAC then `Mac::verify_slice`"
/// idiom already used for the WhatsApp webhook signature check
/// (`messaging/whatsapp/webhook.rs`) rather than a manual `==`, so a
/// mismatch can't leak timing information about how many leading bytes
/// matched. Not signing anything meaningful — both sides already hold the
/// same static secret out-of-band (spawn-time env), so this is just a
/// constant-time equality check dressed as a one-shot MAC.
/// `pub(super)` so the `credential` service can reuse this exact check
/// rather than growing a second constant-time comparison — it gates on the
/// same `AGENTMUX_HOST_REG_SECRET` for the same reason (proving the caller
/// is the paired host, not an agent riding the shared `X-AuthKey`). The
/// NONCE below names host_ipc only because that was the first caller; it's
/// a locally-computed comparison salt on both sides of a single `==`, never
/// a wire format, so sharing it across callers is safe.
pub(super) fn secret_matches(known: &str, supplied: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    const NONCE: &[u8] = b"agentmux-host-ipc-register-secret-compare-v1";
    let Ok(mut expected_mac) = Hmac::<Sha256>::new_from_slice(known.as_bytes()) else {
        return false;
    };
    expected_mac.update(NONCE);
    let expected = expected_mac.finalize().into_bytes();

    let Ok(mut supplied_mac) = Hmac::<Sha256>::new_from_slice(supplied.as_bytes()) else {
        return false;
    };
    supplied_mac.update(NONCE);
    supplied_mac.verify_slice(&expected).is_ok()
}

async fn handle_register(state: &AppState, call: &WebCallType) -> WebReturnType {
    let port = match call.args.first().and_then(|v| v.as_u64()) {
        Some(p) if p <= u16::MAX as u64 => p as u16,
        _ => return WebReturnType::error("host_ipc.Register: args[0] must be a valid port number"),
    };
    let token = match call.args.get(1).and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return WebReturnType::error("host_ipc.Register: args[1] must be a non-empty token string"),
    };

    // `/agentmux/service` shares ONE instance-wide `X-AuthKey` across every
    // caller, including agent-spawned processes (an agent can read that key
    // from its own environment via a shell command — it's injected there for
    // other legitimate purposes). host_ipc.Register is different from every
    // other method on this route: it's meant to be a host-only bootstrap
    // call, and a spoofed re-registration would silently redirect every
    // agent's UIScreenshot/UIClick/UIQuery to an attacker-controlled
    // endpoint for the rest of the session (reagent P0, PR #2662,
    // 2026-08-19).
    //
    // First line of defense: `AGENTMUX_HOST_REG_SECRET`, a credential known
    // only to srv and the paired host (launcher-mediated in launcher-managed
    // mode, self-generated-and-passed-at-spawn in host-owned-spawn/dev mode
    // — see `Config::host_reg_secret`'s doc comment) — never given to any
    // agent. This is checked BEFORE the `state.host_ipc` None/Some branching
    // below, so it closes the race reagent found in the same P0 re-review:
    // when `state.host_ipc` is `None` (every srv startup, and the window
    // after any `restart_backend` before the host re-registers), there is no
    // existing registration to probe liveness against, so ANY caller used to
    // win outright. An agent has no way to learn this secret, so it can never
    // win that race regardless of `state.host_ipc`'s current value.
    let supplied_secret = call.args.get(2).and_then(|v| v.as_str()).unwrap_or("");
    match state.host_reg_secret.as_deref() {
        None => {
            tracing::error!(
                "[host_ipc] REJECTED a Register call — this srv instance has no \
                 AGENTMUX_HOST_REG_SECRET configured, so it cannot verify the caller \
                 is really the paired host. This should never happen in a normal \
                 launcher-managed or host-owned-spawn boot; check that whichever \
                 process spawned this srv instance set that env var."
            );
            return WebReturnType::error(
                "host_ipc.Register: srv has no host-registration secret configured — \
                 refusing to accept any registration",
            );
        }
        Some(known) if secret_matches(known, supplied_secret) => {}
        Some(_) => {
            tracing::error!(
                "[host_ipc] REJECTED a Register call — args[2] did not match this srv \
                 instance's AGENTMUX_HOST_REG_SECRET. Either a caller without this \
                 credential (never an agent — always never given one) is attempting to \
                 spoof the host, or the caller is a genuine host that's out of sync with \
                 srv's current secret (e.g. after a srv recycle, which mints a fresh one)."
            );
            return WebReturnType::error(
                "host_ipc.Register: args[2] does not match this srv instance's \
                 host-registration secret",
            );
        }
    }

    // A conflicting re-registration (different port/token than what's
    // stored) is NOT rejected outright, though — an earlier version of this
    // fix did that unconditionally, which broke a real, already-supported
    // recovery path (reagent P1, same PR, re-review): srv survives a CEF
    // host crash as a Job Object sibling, and the launcher's crash-budget
    // relaunch restarts just the host — which legitimately gets a fresh
    // ipc_port/ipc_token and needs to overwrite the dead registration.
    // Instead: probe whether the CURRENTLY-registered host is still alive
    // (its own `/health` route, unauthenticated, same as `/agentmux/browser/*`'s
    // sibling routes). Still alive → reject (a live legitimate host is not
    // something a conflicting claim should ever override — this is the
    // actual hijack case). Unreachable → accept the new registration (the
    // old host is provably gone; this is crash-restart recovery, not an
    // attack). An IDENTICAL re-registration (same port/token) is always
    // accepted as a harmless no-op without needing a liveness probe.
    let mut guard = state.host_ipc.lock().await;
    if let Some(existing) = guard.as_ref() {
        if existing.port == port && existing.token == token {
            return WebReturnType::success_empty();
        }

        let still_alive = state
            .http_client
            .get(format!("http://127.0.0.1:{}/health", existing.port))
            .timeout(std::time::Duration::from_millis(750))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        if still_alive {
            tracing::error!(
                existing_port = existing.port,
                attempted_port = port,
                "[host_ipc] REJECTED a conflicting Register call — the currently \
                 registered host is still alive and responding to /health. This is \
                 expected at most once as a benign startup race; if it recurs against a \
                 live host, something is attempting to hijack UI-automation credentials."
            );
            return WebReturnType::error(
                "host_ipc.Register: credentials already registered by a different, \
                 currently-live host",
            );
        }
        tracing::warn!(
            old_port = existing.port,
            new_port = port,
            "[host_ipc] previous registration's host is unreachable — accepting the new \
             registration (host crash-restart recovery)"
        );
    }

    *guard = Some(HostIpc { port, token });
    tracing::info!(port, "[host_ipc] registered CEF host CDP-automation credentials");
    WebReturnType::success_empty()
}
