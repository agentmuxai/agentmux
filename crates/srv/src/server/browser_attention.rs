// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's browser pane asking the human for something: a hand-off
//! (sign in, a CAPTCHA, anything the agent mustn't do) or approval of a
//! committing action (submit, send, pay, delete)
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.2, §5.4).
//!
//! The request is shown as a banner in the pane through the block's
//! `browser:attention` meta key. The human's answer comes back only through
//! the CEF host (`/api/v1/host/browser_attention`, authenticated with the
//! host's IPC token, which agents never see): an agent can't answer its own
//! request, even though it holds the instance auth key every other route
//! trusts.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::server::AppState;

/// Block meta key the browser pane's banner reads.
pub(crate) const ATTENTION_META_KEY: &str = "browser:attention";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Handoff,
    Approval,
    /// The page tried to leave the sites the pane is limited to
    /// (SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §4).
    Navigation,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Handoff => "handoff",
            Kind::Approval => "approval",
            Kind::Navigation => "navigation",
        }
    }
}

/// The human's answer, or the lack of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Done (hand-off) or Approve (approval).
    Yes,
    Cancelled,
    TimedOut,
}

impl Answer {
    pub(crate) fn parse(decision: &str) -> Option<Answer> {
        match decision {
            "done" | "approve" => Some(Answer::Yes),
            "cancel" => Some(Answer::Cancelled),
            _ => None,
        }
    }
}

struct Pending {
    block_id: String,
    kind: Kind,
    /// Taken by the first answer. The entry itself stays until the asker's
    /// [`Waiting`] is dropped, so the pane stays locked until it's done.
    tx: Option<oneshot::Sender<Answer>>,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static P: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// What `block_id` is waiting on the human for, if anything. While it
/// waits, the agent's tools on that pane are refused: during a hand-off so
/// it can't watch what the human types (§5.2), during an approval so it
/// can't change the form the human is reading (§5.4).
pub(crate) fn waiting_on_user(block_id: &str) -> Option<Kind> {
    pending()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .find(|p| p.block_id == block_id)
        .map(|p| p.kind)
}

/// Resolve request `id` on `block_id` with the human's answer. Called only
/// by the host-authenticated route.
pub(crate) fn resolve(id: &str, block_id: &str, answer: Answer) -> Result<(), String> {
    let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
    let tx = match map.get_mut(id) {
        Some(p) if p.block_id == block_id => p.tx.take(),
        Some(_) => return Err("that request belongs to another pane".to_string()),
        None => return Err("no such request (it may have timed out)".to_string()),
    };
    let Some(tx) = tx else {
        return Err("that request was already answered".to_string());
    };
    let _ = tx.send(answer);
    Ok(())
}

/// Answer every request waiting on `block_id` with Cancelled: the user took
/// the pane over, which ends whatever the agent was waiting for there.
pub(crate) fn cancel_for(block_id: &str) {
    let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
    for p in map.values_mut().filter(|p| p.block_id == block_id) {
        if let Some(tx) = p.tx.take() {
            let _ = tx.send(Answer::Cancelled);
        }
    }
}

/// A request that is still up. Dropping it takes the request down: it
/// leaves the pending map, which unlocks the pane, and the banner is
/// cleared. That happens on every path, including when the waiting HTTP
/// handler is cancelled because its caller went away.
pub(crate) struct Waiting {
    state: AppState,
    id: String,
    block_id: String,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        // Removing the entry and clearing the banner happen under one hold of
        // the lock, so a new request on the pane (which needs the lock, and
        // this entry gone) can't publish its banner in between and lose it.
        let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&self.id);
        clear_banner_if(&self.state, &self.block_id, &self.id);
    }
}

/// Clear `block_id`'s banner if it is still request `id`'s.
fn clear_banner_if(state: &AppState, block_id: &str, id: &str) {
    let shown = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .and_then(|b| b.meta.get(ATTENTION_META_KEY).cloned())
        .unwrap_or(Value::Null);
    if shown.get("id").and_then(|v| v.as_str()) != Some(id) {
        return;
    }
    let mut clear = crate::backend::obj::MetaMapType::new();
    clear.insert(ATTENTION_META_KEY.to_string(), Value::Null);
    let _ = crate::server::http_shell::broadcast_meta_update(state, block_id, &clear);
}

/// The human answered a banner srv has no request for (srv restarted, so its
/// requests are gone but the block meta persisted): take that banner down,
/// so its buttons don't sit there doing nothing. Host-authenticated callers only.
pub(crate) fn clear_stale(state: &AppState, block_id: &str, id: &str) {
    let map = pending().lock().unwrap_or_else(|p| p.into_inner());
    if map.contains_key(id) {
        return;
    }
    clear_banner_if(state, block_id, id);
}

/// Show `payload` in `block_id`'s banner and wait for the human, up to
/// `timeout`. One request per pane at a time. The pane stays locked until
/// the returned [`Waiting`] is dropped, so a caller that acts on a Yes
/// holds it until that action is done.
pub(crate) async fn ask(
    state: &AppState,
    block_id: &str,
    agent_id: &str,
    kind: Kind,
    payload: Value,
    timeout: Duration,
) -> Result<(Answer, Waiting), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    {
        let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
        if map.values().any(|p| p.block_id == block_id) {
            return Err("this pane is already waiting for the user".to_string());
        }
        map.insert(id.clone(), Pending { block_id: block_id.to_string(), kind, tx: Some(tx) });
    }
    let waiting = Waiting { state: state.clone(), id: id.clone(), block_id: block_id.to_string() };
    let mut banner = json!({ "id": id, "kind": kind.as_str(), "agent": agent_id });
    if let (Some(b), Some(p)) = (banner.as_object_mut(), payload.as_object()) {
        for (k, v) in p {
            b.insert(k.clone(), v.clone());
        }
    }
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(ATTENTION_META_KEY.to_string(), banner);
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, block_id, &meta) {
        return Err(format!("couldn't show the request in the pane: {e}"));
    }
    let answer = match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(a)) => a,
        Ok(Err(_)) => Answer::Cancelled,
        Err(_) => Answer::TimedOut,
    };
    Ok((answer, waiting))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn park(id: &str, block: &str, kind: Kind) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        pending()
            .lock()
            .unwrap()
            .insert(id.to_string(), Pending { block_id: block.to_string(), kind, tx: Some(tx) });
        rx
    }

    #[test]
    fn decisions_parse() {
        assert_eq!(Answer::parse("done"), Some(Answer::Yes));
        assert_eq!(Answer::parse("approve"), Some(Answer::Yes));
        assert_eq!(Answer::parse("cancel"), Some(Answer::Cancelled));
        assert_eq!(Answer::parse("yes please"), None);
    }

    #[tokio::test]
    async fn resolve_answers_the_waiting_request_once() {
        let rx = park("attn-test-1", "pane-a", Kind::Approval);
        resolve("attn-test-1", "pane-a", Answer::Yes).unwrap();
        assert_eq!(rx.await.unwrap(), Answer::Yes);
        assert!(resolve("attn-test-1", "pane-a", Answer::Yes).is_err(), "single use");
        // Answered, but the asker hasn't finished acting on it: still locked.
        assert_eq!(waiting_on_user("pane-a"), Some(Kind::Approval));
        pending().lock().unwrap().remove("attn-test-1");
    }

    #[tokio::test]
    async fn take_over_cancels_what_the_pane_waits_for() {
        let rx = park("attn-test-5", "pane-f", Kind::Approval);
        let _other = park("attn-test-6", "pane-g", Kind::Approval);
        cancel_for("pane-f");
        assert_eq!(rx.await.unwrap(), Answer::Cancelled);
        // A late Approve finds the request already answered.
        assert!(resolve("attn-test-5", "pane-f", Answer::Yes).is_err());
        // Another pane's request is untouched.
        assert!(resolve("attn-test-6", "pane-g", Answer::Yes).is_ok());
        pending().lock().unwrap().remove("attn-test-5");
        pending().lock().unwrap().remove("attn-test-6");
    }

    #[test]
    fn a_request_is_answered_only_for_its_own_pane() {
        let _rx = park("attn-test-2", "pane-b", Kind::Approval);
        assert!(resolve("attn-test-2", "pane-other", Answer::Yes).is_err());
        pending().lock().unwrap().remove("attn-test-2");
    }

    #[test]
    fn a_pending_request_of_either_kind_locks_the_pane() {
        let _rx = park("attn-test-3", "pane-c", Kind::Handoff);
        assert_eq!(waiting_on_user("pane-c"), Some(Kind::Handoff));
        assert_eq!(waiting_on_user("pane-d"), None);
        pending().lock().unwrap().remove("attn-test-3");
        assert_eq!(waiting_on_user("pane-c"), None);
        let _rx = park("attn-test-4", "pane-e", Kind::Approval);
        assert_eq!(waiting_on_user("pane-e"), Some(Kind::Approval));
        pending().lock().unwrap().remove("attn-test-4");
    }
}
