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
    // other method on this route: it's meant to be a one-time, host-only
    // bootstrap call, and a spoofed re-registration would silently redirect
    // every agent's UIScreenshot/UIClick/UIQuery to an attacker-controlled
    // endpoint for the rest of the session (reagent P0, PR #2662,
    // 2026-08-19). Reject any re-registration whose port/token don't match
    // what's already stored — this can't fully distinguish the real host
    // from a maximally-fast attacker racing it at the very first instant of
    // srv's boot (before any agent process can even exist yet, in practice),
    // but it closes the "hijack persists silently for the rest of the
    // session" failure mode: at most one set of credentials is ever
    // accepted, and every rejected attempt is logged loudly. An IDENTICAL
    // re-registration (a legitimate retry with the same values) is still
    // accepted as a harmless no-op.
    let mut guard = state.host_ipc.lock().await;
    if let Some(existing) = guard.as_ref() {
        if existing.port == port && existing.token == token {
            return WebReturnType::success_empty();
        }
        tracing::error!(
            existing_port = existing.port,
            attempted_port = port,
            "[host_ipc] REJECTED a conflicting Register call — host_ipc credentials are \
             already set and this request supplied different ones. This is expected at \
             most once (a benign race at startup); if it recurs, something is attempting \
             to hijack UI-automation credentials."
        );
        return WebReturnType::error(
            "host_ipc.Register: credentials already registered by a different caller",
        );
    }

    *guard = Some(HostIpc { port, token });
    tracing::info!(port, "[host_ipc] registered CEF host CDP-automation credentials");
    WebReturnType::success_empty()
}
