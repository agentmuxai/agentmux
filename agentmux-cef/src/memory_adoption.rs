// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The human's confirmation for adopting an agent's memory from its earlier
//! accounts (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.4).
//!
//! The Armory lists the candidates (`agent:memory:adoption_list`) and asks
//! for an adoption here (`memory_adoption_request`). That opens a separate
//! approval subwindow — never a modal in the main window: the same reasoning
//! as `credential_broker` (see `CredentialApprovalWindow.tsx`), since
//! anything in the main window outside a pane is reachable by every agent's
//! `UIQuery`/`UIClick`. Only the subwindow knows the `approval_id`; its
//! `memory_adoption_decide` makes the host call srv's host-only
//! `memoryadopt.Adopt`, and the outcome goes back to the Armory's window as
//! `memory-adoption-result`.
//!
//! An agent can click the Armory's button and open the subwindow; it can't
//! approve in it.
//!
//! The same window confirms releasing an agent's claim on a memory folder
//! (`memory_release_request`, srv `memoryadopt.Release`), which can make the
//! folder another agent's to write in.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::state::AppState;

/// An approval left open this long is dropped; the subwindow's decide then
/// finds nothing and the human asks again.
const APPROVAL_TTL: Duration = Duration::from_secs(10 * 60);
/// Adopting reads the chosen folders and writes the record under the
/// agent's reconcile lease.
const ADOPT_TIMEOUT: Duration = Duration::from_secs(30);
pub const RESULT_EVENT: &str = "memory-adoption-result";

/// What the human is asked to confirm.
enum Action {
    /// Adopt these `(index, dir_hash)` choices of an adoption list.
    Adopt { choices: Vec<serde_json::Value> },
    /// Release the claim at `index` of a claims list.
    Release { index: u64 },
}

struct Pending {
    agent_id: String,
    list_id: String,
    action: Action,
    parent_label: String,
    window_id: Option<String>,
    at: Instant,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Open the approval subwindow for adopting `choices` of list `list_id`
/// into `agent_id`, over the window `parent_label` (the Armory's). `summary`
/// is only what the human is shown; srv decides from the list it issued.
pub fn request(
    state: &Arc<AppState>,
    parent_label: &str,
    agent_id: &str,
    list_id: &str,
    choices: Vec<serde_json::Value>,
    summary: serde_json::Value,
) -> Result<String, String> {
    if choices.is_empty() {
        return Err("memory_adoption_request: nothing chosen".into());
    }
    open(state, parent_label, agent_id, list_id, Action::Adopt { choices }, summary)
}

/// Open the approval subwindow for releasing `agent_id`'s claim at `index`
/// of claims list `list_id`.
pub fn request_release(
    state: &Arc<AppState>,
    parent_label: &str,
    agent_id: &str,
    list_id: &str,
    index: u64,
    summary: serde_json::Value,
) -> Result<String, String> {
    open(state, parent_label, agent_id, list_id, Action::Release { index }, summary)
}

fn open(
    state: &Arc<AppState>,
    parent_label: &str,
    agent_id: &str,
    list_id: &str,
    action: Action,
    summary: serde_json::Value,
) -> Result<String, String> {
    let kind = match action {
        Action::Adopt { .. } => "adopt",
        Action::Release { .. } => "release",
    };
    let approval_id = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut map = pending().lock();
        map.retain(|_, p| p.at.elapsed() < APPROVAL_TTL);
        map.insert(
            approval_id.clone(),
            Pending {
                agent_id: agent_id.to_string(),
                list_id: list_id.to_string(),
                action,
                parent_label: parent_label.to_string(),
                window_id: None,
                at: Instant::now(),
            },
        );
    }
    let meta = serde_json::json!({ "approval_id": approval_id, "kind": kind, "summary": summary }).to_string();
    let opened = crate::commands::window::open_subwindow(
        state,
        parent_label.to_string(),
        Some(crate::commands::window::MEMORY_ADOPTION_APPROVAL_VIEW),
        Some(&meta),
    )
    .and_then(|v| v.as_str().map(str::to_string).ok_or_else(|| "open_subwindow returned no label".to_string()));
    match opened {
        Ok(window_id) => {
            if let Some(p) = pending().lock().get_mut(&approval_id) {
                p.window_id = Some(window_id);
            }
            Ok(approval_id)
        }
        Err(e) => {
            pending().lock().remove(&approval_id);
            Err(format!("memory_adoption_request: {e}"))
        }
    }
}

/// The human decided in the subwindow. On approve, the host adopts through
/// srv; either way the outcome goes to the Armory's window and the
/// subwindow closes.
pub async fn decide(state: &Arc<AppState>, approval_id: &str, approve: bool) -> Result<serde_json::Value, String> {
    let Some(p) = pending().lock().remove(approval_id).filter(|p| p.at.elapsed() < APPROVAL_TTL) else {
        tracing::warn!("[memory-adoption] decide for unknown or expired approval {approval_id}");
        return Ok(serde_json::json!(false));
    };
    let (kind, method, args) = match &p.action {
        Action::Adopt { choices } => (
            "adopt",
            "Adopt",
            serde_json::json!({ "agent_id": p.agent_id, "list_id": p.list_id, "choices": choices }),
        ),
        Action::Release { index } => (
            "release",
            "Release",
            serde_json::json!({ "agent_id": p.agent_id, "list_id": p.list_id, "index": index }),
        ),
    };
    let outcome = if approve {
        match crate::credential_broker::call_host_only_service(state, "memoryadopt", method, args, ADOPT_TIMEOUT).await {
            Ok(report) => serde_json::json!({ "agent_id": p.agent_id, "kind": kind, "status": "done", "report": report }),
            Err(e) => {
                tracing::warn!("[memory-adoption] {method} failed: {e}");
                serde_json::json!({ "agent_id": p.agent_id, "kind": kind, "status": "failed", "error": e })
            }
        }
    } else {
        serde_json::json!({ "agent_id": p.agent_id, "kind": kind, "status": "declined" })
    };
    crate::events::emit_event_to_window(state, &p.parent_label, RESULT_EVENT, &outcome);
    if let Some(window_id) = p.window_id {
        if let Err(e) = crate::commands::window::close_window_by_label(state, &serde_json::json!({ "label": window_id })) {
            tracing::warn!("[memory-adoption] failed to close approval window: {e}");
        }
    }
    Ok(serde_json::json!(true))
}

/// The approval subwindow `label` closed before the human decided (or its
/// parent window closed): drop the request and tell the Armory, so it isn't
/// left waiting.
pub fn cancel_for_window(state: &AppState, label: &str) {
    let dropped: Vec<Pending> = {
        let mut map = pending().lock();
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, p)| p.window_id.as_deref() == Some(label))
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    };
    for p in dropped {
        let kind = match p.action {
            Action::Adopt { .. } => "adopt",
            Action::Release { .. } => "release",
        };
        let outcome = serde_json::json!({ "agent_id": p.agent_id, "kind": kind, "status": "declined" });
        crate::events::emit_event_to_window(state, &p.parent_label, RESULT_EVENT, &outcome);
    }
}
