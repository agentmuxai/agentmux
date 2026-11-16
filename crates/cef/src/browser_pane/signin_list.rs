// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The list of saved sign-ins under a login field
//! (SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.3), host side.
//!
//! `signin_notice.js` reports what the person did in a login field: a click or an arrow key
//! (`open`), a key the list should take (`key`), or a press elsewhere (`close`). Here the host
//! hands each report to srv (`credential.FieldOpen`, `FieldKey`, `FieldClose`), which owns the
//! rows, the filter and the selected row and shows the list in the pane's block meta, and then
//! tells the listener's world what srv answered: whether a list is showing and whether one of its
//! rows is selected (`window.__agentmuxList`), so the listener swallows a key only when it
//! belongs to the list. The reports of one pane go one at a time, in the order they were seen.
//!
//! Nothing here runs while an agent's input is reaching the pane (`agent_input`): the caller
//! drops the report first.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::state::AppState;

const SRV_WAIT: Duration = Duration::from_secs(5);

/// One report on its way to srv, and the world its answer goes back to.
struct Report {
    method: &'static str,
    args: Value,
    session: Option<String>,
    context: i64,
}

static SENDERS: LazyLock<Mutex<HashMap<i32, UnboundedSender<Report>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// The browsers whose page asked for a list: a navigation tells srv to close it, even on a page that
/// showed no sign-in form.
static OPENED: LazyLock<Mutex<std::collections::HashSet<i32>>> = LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// Whether `browser_id`'s page asked for a list since the last time this was asked.
pub(crate) fn take_opened(browser_id: i32) -> bool {
    OPENED.lock().remove(&browser_id)
}

/// What the listener's world is told after srv answers: whether a list is showing, and whether one of
/// its rows is selected. Booleans only, so nothing srv says can be run as script.
pub(crate) fn state_script(answer: &Value) -> String {
    let flag = |k: &str| answer.get(k).and_then(Value::as_bool) == Some(true);
    format!("window.__agentmuxList={{shown:{},cursor:{}}}", flag("shown"), flag("cursor"))
}

/// Hand srv one report from the listener of the world `context` (in `session`) of `browser_id`.
pub(crate) fn send(state: Arc<AppState>, browser_id: i32, session: Option<String>, context: i64, method: &'static str, args: Value) {
    let Some(rt) = super::auth::runtime() else { return };
    if method == "FieldOpen" {
        OPENED.lock().insert(browser_id);
    }
    let mut senders = SENDERS.lock();
    let tx = senders.entry(browser_id).or_insert_with(|| {
        let (tx, mut rx) = unbounded_channel::<Report>();
        rt.spawn(async move {
            while let Some(r) = rx.recv().await {
                match crate::credential_broker::call_host_only_service(&state, "credential", r.method, r.args, SRV_WAIT).await {
                    Ok(answer) => super::side_button_cdp::eval_in(browser_id, r.session.as_deref(), r.context, &state_script(&answer)).await,
                    Err(e) => tracing::warn!(error = %e, "[signin-list] couldn't hand srv a {}", r.method),
                }
            }
        });
        tx
    });
    let _ = tx.send(Report { method, args, session, context });
}

/// srv closed the list by itself (a pick made by a click in the pane's own UI, a strong password made
/// there): tell every listener world of `browser_id` that no list shows and nothing is selected, so
/// the next Up, Down, Escape or Enter in the page's field is the page's again. Nothing srv could have
/// answered a report of the listener reaches it otherwise.
pub(crate) async fn clear(browser_id: i32) {
    let script = state_script(&Value::Null);
    for (session, context) in super::signin_notice::worlds_of(browser_id) {
        super::side_button_cdp::eval_in(browser_id, session.as_deref(), context, &script).await;
    }
}

/// The pane's browser closed.
pub(crate) fn forget(browser_id: i32) {
    SENDERS.lock().remove(&browser_id);
    OPENED.lock().remove(&browser_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_state_the_listener_is_told_is_two_booleans() {
        assert_eq!(state_script(&json!({ "shown": true, "cursor": false })), "window.__agentmuxList={shown:true,cursor:false}");
        assert_eq!(state_script(&json!({ "shown": true, "cursor": true })), "window.__agentmuxList={shown:true,cursor:true}");
        assert_eq!(state_script(&json!({})), "window.__agentmuxList={shown:false,cursor:false}");
    }

    #[test]
    fn a_close_srv_made_itself_is_told_to_every_listener_world_as_nothing_showing() {
        use super::super::signin_notice::{forget, note_world, worlds_of};
        let browser = 7_100_001;
        note_world(browser, None, "main", 4, "https://a.example.com");
        note_world(browser, Some("frame-session"), "child", 9, "https://b.example.com");
        assert_eq!(worlds_of(browser), vec![(None, 4), (Some("frame-session".to_string()), 9)]);
        // What `clear` runs in each of them: no list, no selected row.
        assert_eq!(state_script(&Value::Null), "window.__agentmuxList={shown:false,cursor:false}");
        forget(browser);
        assert!(worlds_of(browser).is_empty());
    }

    #[test]
    fn nothing_srv_says_can_be_run_as_script() {
        let hostile = json!({ "shown": "true;alert(1)", "cursor": "1", "extra": "});alert(1);//" });
        assert_eq!(state_script(&hostile), "window.__agentmuxList={shown:false,cursor:false}");
    }
}
