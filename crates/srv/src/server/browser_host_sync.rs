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

/// Send the host the agent-owned panes, each limited pane's site list, and
/// the panes asking the person about a navigation (allowed-origins spec §6).
/// The sync loop does this on every change; a caller that needs the host to
/// have it before the next navigation awaits it directly. The host replaces
/// its copy with each push, so pushes go one at a time, each taking its
/// snapshot inside the lock: an older one can't arrive after a newer one.
pub(crate) async fn push(state: &AppState) {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = ONE_AT_A_TIME.lock().await;
    let Some(host) = state.host_ipc.lock().await.clone() else {
        return;
    };
    let body = serde_json::json!({
        "panes": crate::server::browser_owner::owned_panes(),
        "allowed": crate::server::browser_allowlist::snapshot(),
        "asking": crate::server::browser_attention::asking(crate::server::browser_attention::Kind::Navigation),
        "jars": crate::server::browser_identity::live_jars(state),
    });
    if let Err(e) = crate::server::ui_handlers::proxy_to_host_timeout(state, &host, "owned_panes", body, Some(std::time::Duration::from_secs(5))).await {
        tracing::debug!(error = %e, "[browser-popup] couldn't send the owned panes to the host");
    }
}
