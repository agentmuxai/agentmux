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
    // A deleted pane's browser ownership, waits and limits go with it.
    super::browser_pane_gone::install();
    // The owners and site limits the last run kept, before anything can change them.
    super::browser_pane_records::restore(&state);
    tokio::spawn(async move {
        loop {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                crate::server::browser_owner::changed().notified(),
            )
            .await;
            super::browser_pane_records::save();
            push(&state).await;
        }
    });
}

/// Send the host the agent-owned panes, each limited pane's site list, the
/// panes asking the person about a navigation (allowed-origins spec §6), and
/// the panes handed to the person, whose page dialogs are theirs to answer
/// (SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md §4.3).
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
    let owned = crate::server::browser_owner::owned_panes();
    let handed = crate::server::browser_attention::asking(crate::server::browser_attention::Kind::Handoff);
    let mut body = serde_json::json!({
        "download_workdirs": crate::server::browser_agent_downloads::workdirs(state, &owned, &handed),
        "panes": owned,
        "allowed": crate::server::browser_allowlist::snapshot(),
        "asking": crate::server::browser_attention::asking(crate::server::browser_attention::Kind::Navigation),
        "unlimited": crate::server::browser_allowlist::confirmed_unlimited(),
        "handoff": handed,
    });
    // Left out when the store can't be read: the host then keeps its jars.
    if let Some(jars) = crate::server::browser_identity::live_jars(state) {
        body["jars"] = serde_json::json!(jars);
    }
    // The named profiles there are: the host drops the jar and folder of one
    // deleted. Left out when they can't be read, so it keeps them all.
    if let Some(ids) = crate::server::browser_identity::profile_ids() {
        body["profiles"] = serde_json::json!(ids);
    }
    if let Err(e) = crate::server::ui_handlers::proxy_to_host_timeout(state, &host, "owned_panes", body, Some(std::time::Duration::from_secs(5))).await {
        tracing::debug!(error = %e, "[browser-popup] couldn't send the owned panes to the host");
    }
    // A zoom of a pane that is no longer the zoomer's: the host drops the unowned ones itself, this the rest.
    crate::server::browser_zoom::reconcile(state, &host, &handed).await;
}
