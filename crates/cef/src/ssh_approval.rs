// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SSH approvals: the user's consent for an agent to use an SSH host, and
//! ssh's own prompts for an agent's ssh (password, passphrase, new host key),
//! asked in an approval subwindow (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md
//! §5.3, §8.2).
//!
//! Not a modal in the main window, and not answered through srv's own
//! services: every agent holds srv's auth key, and anything in the main window
//! outside a pane is reachable by every agent's `UIClick`/`UIQuery` (the same
//! reasoning as `credential_broker` and `memory_adoption`). So srv asks the
//! host over the host's own IPC server (`POST /agentmux/approval/ask`, the
//! host's IPC token, which no agent has) and waits; the host opens an
//! approval subwindow, which the browser API never resolves a pane into; only
//! that subwindow knows the approval id; its `ssh_approval_decide` resolves
//! the waiting request, and the answer goes back to srv in the response to
//! srv's own call. Nothing an agent can call carries or forges it.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::browser_api::types::ApiResponse;
use crate::state::AppState;

pub const ASK_ROUTE: &str = "/agentmux/approval/ask";

/// The longest srv may have the user take; srv asks for less (two minutes).
const MAX_WAIT: Duration = Duration::from_secs(150);

/// What srv asks the user, from `POST /agentmux/approval/ask`.
#[derive(Debug, Deserialize)]
pub struct AskRequest {
    /// The agent's pane: the subwindow opens over the window showing it.
    pub block_id: String,
    /// `consent` (allow an agent on a host), `secret` (a hidden answer),
    /// `yesno`, or `info` (a notice; nothing to answer).
    pub kind: String,
    pub title: String,
    /// Plain text, shown as is.
    pub message: String,
    /// A checkbox label (consent's "Always allow"); empty for none.
    #[serde(default)]
    pub checkbox: String,
    #[serde(default)]
    pub ok_label: String,
    #[serde(default)]
    pub cancel_label: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

/// The user's answer, returned to srv. `answered` is false when the window
/// was closed, timed out, or could not be opened.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Answer {
    pub answered: bool,
    pub approve: bool,
    /// The typed answer, for `secret`: never logged.
    pub text: String,
    pub checkbox: bool,
}

struct Pending {
    tx: oneshot::Sender<Answer>,
    window_label: Option<String>,
}

/// Forgets the request and closes its window when the wait ends, however it
/// ends: answered, timed out, or srv gone (its request dropped mid-wait), so
/// no window is left asking for an answer nothing will read.
struct Cleanup {
    state: Arc<AppState>,
    approval_id: String,
    window_label: Option<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        pending().lock().remove(&self.approval_id);
        if let Some(label) = self.window_label.take() {
            let _ = crate::commands::window::close_window_by_label(
                &self.state,
                &serde_json::json!({ "label": label }),
            );
        }
    }
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register `POST /agentmux/approval/ask` on the host's IPC server.
pub fn register_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route(ASK_ROUTE, post(ask))
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|token| agentmux_common::secret_eq::secret_eq(token.as_bytes(), expected.as_bytes()))
        .unwrap_or(false)
}

async fn ask(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<AskRequest>,
) -> (StatusCode, Json<ApiResponse<Answer>>) {
    if !authorized(&headers, &state.ipc_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::err("unauthorized")),
        );
    }
    let wait = Duration::from_millis(req.timeout_ms.max(1_000)).min(MAX_WAIT);
    let parent = match parent_window(&state, &req.block_id).await {
        Some(p) => p,
        None => return (StatusCode::OK, Json(ApiResponse::ok(Answer::default()))),
    };
    let approval_id = uuid::Uuid::new_v4().simple().to_string();
    let (tx, rx) = oneshot::channel();
    pending().lock().insert(
        approval_id.clone(),
        Pending {
            tx,
            window_label: None,
        },
    );
    let meta = serde_json::json!({
        "approval_id": approval_id,
        "kind": req.kind,
        "title": req.title,
        "message": req.message,
        "checkbox": req.checkbox,
        "ok_label": req.ok_label,
        "cancel_label": req.cancel_label,
        "timeout_ms": wait.as_millis() as u64,
    })
    .to_string();
    let opened = crate::commands::window::open_subwindow(
        &state,
        parent,
        Some(crate::commands::window::SSH_APPROVAL_VIEW),
        Some(&meta),
    )
    .and_then(|v| {
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| "open_subwindow returned no label".to_string())
    });
    let _cleanup = match opened {
        Ok(label) => {
            if let Some(p) = pending().lock().get_mut(&approval_id) {
                p.window_label = Some(label.clone());
            }
            Cleanup {
                state: state.clone(),
                approval_id: approval_id.clone(),
                window_label: Some(label),
            }
        }
        Err(e) => {
            tracing::warn!("[ssh-approval] could not open the approval window: {e}");
            pending().lock().remove(&approval_id);
            return (StatusCode::OK, Json(ApiResponse::ok(Answer::default())));
        }
    };
    // Closed (cancel_for_window) or timed out: unanswered. Whatever happens,
    // `_cleanup` forgets the request and closes the window, srv dropping this
    // request mid-wait included.
    let answer = match tokio::time::timeout(wait, rx).await {
        Ok(Ok(answer)) => answer,
        _ => Answer::default(),
    };
    tracing::info!(
        "[ssh-approval] {} answered={} approve={} checkbox={}",
        req.kind,
        answer.answered,
        answer.approve,
        answer.checkbox
    );
    (StatusCode::OK, Json(ApiResponse::ok(answer)))
}

/// A pre-created window waiting in a pool: hidden, so never a parent the user
/// would see.
fn is_pool_window(label: &str) -> bool {
    label.starts_with("window-pool-") || label.starts_with("floating-pool-")
}

/// The full window showing `block_id`; else the main window; else any other
/// full window the user can see. An approval subwindow needs a full window as
/// its parent, and one over a hidden pool window is never seen.
async fn parent_window(state: &Arc<AppState>, block_id: &str) -> Option<String> {
    let full_windows: Vec<String> = state
        .window_meta
        .lock()
        .iter()
        .filter(|(label, m)| {
            m.kind == crate::state::WindowKind::FullInstance && !is_pool_window(label)
        })
        .map(|(label, _)| label.clone())
        .collect();
    if let Ok(target) = state
        .browser_api
        .target_cache
        .resolve(state, block_id)
        .await
    {
        if full_windows.contains(&target.label) {
            return Some(target.label);
        }
    }
    pick_fallback(&full_windows)
}

fn pick_fallback(full_windows: &[String]) -> Option<String> {
    full_windows
        .iter()
        .find(|l| l.as_str() == "main")
        .or_else(|| full_windows.iter().min())
        .cloned()
}

/// The user decided in the subwindow (`ssh_approval_decide`). Only that
/// subwindow knows `approval_id`. `false` if nothing waits on it any more.
pub fn decide(approval_id: &str, approve: bool, text: String, checkbox: bool) -> bool {
    match pending().lock().remove(approval_id) {
        Some(p) => {
            p.tx.send(Answer {
                answered: true,
                approve,
                text,
                checkbox,
            })
            .is_ok()
        }
        None => false,
    }
}

/// The approval subwindow `label` closed before the user answered: the
/// request is unanswered, at once, rather than when its wait runs out.
pub fn cancel_for_window(label: &str) {
    let mut map = pending().lock();
    let ids: Vec<String> = map
        .iter()
        .filter(|(_, p)| p.window_label.as_deref() == Some(label))
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        if let Some(p) = map.remove(&id) {
            let _ = p.tx.send(Answer::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(id: &str) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        pending().lock().insert(
            id.to_string(),
            Pending {
                tx,
                window_label: Some(format!("win-{id}")),
            },
        );
        rx
    }

    #[test]
    fn only_a_waiting_approval_takes_an_answer_once() {
        let mut rx = wait_for("ssh-a1");
        assert!(decide("ssh-a1", true, "pw".into(), true));
        assert_eq!(
            rx.try_recv().unwrap(),
            Answer {
                answered: true,
                approve: true,
                text: "pw".into(),
                checkbox: true
            }
        );
        assert!(
            !decide("ssh-a1", true, String::new(), false),
            "answered once"
        );
        assert!(!decide("ssh-never", true, String::new(), false));
    }

    #[test]
    fn a_hidden_pool_window_is_never_the_parent() {
        assert!(is_pool_window("floating-pool-f7b0"));
        assert!(is_pool_window("window-pool-1"));
        assert!(!is_pool_window("main") && !is_pool_window("window-731e"));
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            pick_fallback(&s(&["window-b", "main", "window-a"])).as_deref(),
            Some("main")
        );
        assert_eq!(
            pick_fallback(&s(&["window-b", "window-a"])).as_deref(),
            Some("window-a")
        );
        assert_eq!(pick_fallback(&[]), None);
    }

    #[test]
    fn closing_the_window_leaves_the_request_unanswered() {
        let mut rx = wait_for("ssh-a2");
        cancel_for_window("win-ssh-a2");
        assert_eq!(rx.try_recv().unwrap(), Answer::default());
        assert!(!decide("ssh-a2", true, String::new(), false));
    }
}
