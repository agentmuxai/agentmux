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

use crate::backend::service::{WebCallType, WebReturnType};

use super::super::{AppState, HostIpc};

pub(super) async fn handle_host_ipc_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    match call.method.as_str() {
        "Register" => handle_register(state, call).await,
        _ => WebReturnType::error(format!("unknown host_ipc method: {}", call.method)),
    }
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
