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

    *state.host_ipc.lock().await = Some(HostIpc { port, token });
    tracing::info!(port, "[host_ipc] registered CEF host CDP-automation credentials");
    WebReturnType::success_empty()
}
