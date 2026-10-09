// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host's copy of srv's browser-pane records: which panes an agent owns
//! (native-popups spec §3) and the sites each limited pane may go to
//! (SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §6). The host reads both
//! inside CEF callbacks that can't wait for srv.

use crate::server::AppState;

/// Keep the host's copy of the agent-owned panes current: push the whole set
/// whenever it changes or the host registers (`browser_owner::changed`), and
/// every minute regardless, so a push the host missed is made up for.
pub(crate) fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                crate::server::browser_owner::changed().notified(),
            )
            .await;
            push(&state).await;
        }
    });
}

/// Send the host the agent-owned panes and each limited pane's site list
/// (allowed-origins spec §6). The sync loop does this on every change; a
/// caller that needs the host to have it before the next navigation awaits
/// it directly.
pub(crate) async fn push(state: &AppState) {
    let Some(host) = state.host_ipc.lock().await.clone() else {
        return;
    };
    let body = serde_json::json!({
        "panes": crate::server::browser_owner::owned_panes(),
        "allowed": crate::server::browser_allowlist::snapshot(),
    });
    if let Err(e) = crate::server::ui_handlers::proxy_to_host_timeout(state, &host, "owned_panes", body, Some(std::time::Duration::from_secs(5))).await {
        tracing::debug!(error = %e, "[browser-popup] couldn't send the owned panes to the host");
    }
}
