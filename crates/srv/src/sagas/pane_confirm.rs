// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A question srv asks the person in the pane it is about, as a modal that
//! covers only that pane (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md
//! §6 rows 3 and 6): add a host to the ssh config, remove or install AgentMux's
//! helper, end a durable session, run a system tool's install command. Each is
//! the confirmation of the person's own click in a pane (Remotes, Toolchain, a
//! terminal), so the question belongs to that pane. It used to open an approval
//! window over the whole AgentMux window.
//!
//! Unlike an agent's notice (`agent_notice`), nothing goes ahead by itself: only
//! an answered yes does. No answer in [`ASK_FOR`], a closed pane, or no window
//! connected all leave the thing undone.
//!
//! The person's answer comes only through `pane.confirm.decide`, accepted only
//! from the proven AgentMux window (`operator_only`), and the modal is drawn
//! where an agent's UI tool can neither read nor press it. So an agent that
//! calls one of these commands with srv's key still can't answer its own
//! question, which is what the approval window was for.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use crate::backend::prompt_events::{PaneConfirmView, EVENT_PANE_CONFIRM};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// The person's answer to a question.
pub const COMMAND_PANE_CONFIRM_DECIDE: &str = "pane.confirm.decide";
/// How long the person has to answer.
pub const ASK_FOR: Duration = Duration::from_secs(120);
/// A settled question stays answerable (as "too late") this long.
const KEEP_FINISHED_MS: i64 = 60_000;
/// How many events are kept for a window that opens while a question waits.
const REPLAY_EVENTS: usize = 32;

/// What the person is asked. Built by srv from its own record, never from an agent's words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Question {
    pub title: String,
    /// Plain text; line breaks are kept (a command, a config block).
    pub message: String,
    pub ok_label: String,
    pub cancel_label: String,
    /// The yes can't be undone: drawn in the danger tone, with No in focus.
    pub destructive: bool,
    /// A box to tick with the answer ("Don't ask again for db1"); empty for none.
    pub checkbox: String,
}

impl Question {
    /// From the JSON the approval window takes (`title`, `message`, `ok_label`,
    /// `cancel_label`, `destructive`, `checkbox`), so one question can go to either.
    pub fn from_json(q: &serde_json::Value) -> Self {
        let text = |k: &str, d: &str| q.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(d).to_string();
        Self {
            title: text("title", "AgentMux needs your answer"),
            message: text("message", ""),
            ok_label: text("ok_label", "OK"),
            cancel_label: text("cancel_label", "Cancel"),
            destructive: q.get("destructive").and_then(|v| v.as_bool()).unwrap_or(false),
            checkbox: text("checkbox", ""),
        }
    }
}

/// The person's answer. `answered` is false when the time ran out, the pane was closed, or the
/// caller went away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Answer {
    pub answered: bool,
    pub approve: bool,
    pub checkbox: bool,
}

/// Data for `pane.confirm.decide`: the question's `id` (from `pane:confirm`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandPaneConfirmDecideData {
    pub request_id: String,
    pub approve: bool,
    /// The tick box, when the question had one.
    #[serde(default)]
    pub checkbox: bool,
}

/// How the answer came out: `too_late` when srv had stopped waiting.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PaneConfirmDecideResult {
    #[ts(type = r#""answered" | "too_late""#)]
    pub outcome: String,
}

struct Entry {
    view: PaneConfirmView,
    answer: Option<Answer>,
    wake: Arc<tokio::sync::Notify>,
    finished_at_ms: Option<i64>,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn entries() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

/// The questions still waiting.
pub fn pending() -> Vec<PaneConfirmView> {
    entries().values().filter(|e| e.view.status == "pending").map(|e| e.view.clone()).collect()
}

/// Ask `question` in pane `block_id` and wait for the person's answer. `Err` when there is no
/// window connected to ask in: whatever needed the answer must not happen, nor wait.
pub async fn ask(state: &AppState, block_id: &str, question: Question) -> Result<Answer, String> {
    ask_for(state, block_id, question, ASK_FOR).await
}

/// [`ask`] that waits `window` for the answer.
pub async fn ask_for(state: &AppState, block_id: &str, question: Question, window: Duration) -> Result<Answer, String> {
    if block_id.is_empty() {
        return Err("no pane was named to ask the user in".to_string());
    }
    crate::server::app_api::connections::host_to_ask_user(state).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let opened = now_ms();
    let view = PaneConfirmView {
        id: id.clone(),
        block_id: block_id.to_string(),
        title: question.title,
        message: question.message,
        ok_label: question.ok_label,
        cancel_label: question.cancel_label,
        destructive: question.destructive,
        checkbox: question.checkbox,
        deadline_ms: opened + window.as_millis() as i64,
        status: "pending",
    };
    let wake = Arc::new(tokio::sync::Notify::new());
    {
        let mut map = entries();
        map.retain(|_, e| e.finished_at_ms.is_none_or(|t| opened - t < KEEP_FINISHED_MS));
        map.insert(id.clone(), Entry { view: view.clone(), answer: None, wake: wake.clone(), finished_at_ms: None });
    }
    state.broker.publish_event::<crate::backend::mps_events::PaneConfirm>(&view, vec![], REPLAY_EVENTS);
    // The pane's waiting tone, tab badge and notification, while it waits.
    let _asking = crate::backend::user_attention::Asking::start(&state.broker, &state.mstore, block_id, "confirm", &view.title);
    tracing::info!(request_id = %id, block_id, title = %view.title, "pane confirm: asked");

    // Ends the question however this ends: the caller can be dropped while it waits (its request
    // closed), and the modal must still go.
    let mut ending = Ending { broker: state.broker.clone(), id: id.clone(), view, done: false };
    let _ = tokio::time::timeout(window, wake.notified()).await;
    Ok(ending.finish())
}

struct Ending {
    broker: Arc<crate::backend::mps::Broker>,
    id: String,
    view: PaneConfirmView,
    done: bool,
}

impl Ending {
    fn finish(&mut self) -> Answer {
        let answer = settle(&self.id);
        self.done = true;
        let mut done = self.view.clone();
        done.status = "cleared";
        self.broker.publish_event::<crate::backend::mps_events::PaneConfirm>(&done, vec![], REPLAY_EVENTS);
        tracing::info!(request_id = %self.id, answered = answer.answered, approve = answer.approve, "pane confirm: ended");
        answer
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        if !self.done {
            self.finish();
        }
    }
}

/// End question `id` with the person's answer if they gave one, else unanswered. After this it
/// answers "too late".
fn settle(id: &str) -> Answer {
    let mut map = entries();
    let Some(e) = map.get_mut(id) else { return Answer::default() };
    e.view.status = "cleared";
    e.finished_at_ms = Some(now_ms());
    e.answer.unwrap_or_default()
}

/// The person's answer to question `id`.
pub fn decide(id: &str, approve: bool, checkbox: bool) -> &'static str {
    let mut map = entries();
    let Some(e) = map.get_mut(id).filter(|e| e.view.status == "pending" && e.answer.is_none()) else {
        return "too_late";
    };
    e.answer = Some(Answer { answered: true, approve, checkbox: checkbox && !e.view.checkbox.is_empty() });
    e.wake.notify_one();
    "answered"
}

/// `pane.confirm.decide`, accepted only from the proven AgentMux window (`operator_only`), never
/// from an agent: the answer is the person's own.
pub fn register(engine: &Arc<WshRpcEngine>) {
    engine.register_typed(COMMAND_PANE_CONFIRM_DECIDE, move |cmd: CommandPaneConfirmDecideData, ctx| async move {
        ctx.operator_only("only the person can answer a question AgentMux asks them")?;
        Ok(PaneConfirmDecideResult { outcome: decide(&cmd.request_id, cmd.approve, cmd.checkbox).to_string() })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use crate::server::HostIpc;

    fn question() -> Question {
        Question {
            title: "End a session on db1".to_string(),
            message: "End the durable session s1 on db1?".to_string(),
            ok_label: "End Session".to_string(),
            cancel_label: "Keep It".to_string(),
            destructive: true,
            checkbox: String::new(),
        }
    }

    fn block() -> String {
        format!("block-{}", uuid::Uuid::new_v4().simple())
    }

    async fn with_window() -> AppState {
        let state = test_state();
        *state.host_ipc.lock().await = Some(HostIpc { port: 1, token: "t".to_string() });
        state
    }

    async fn asked_in(block_id: &str) -> PaneConfirmView {
        for _ in 0..400 {
            if let Some(v) = pending().into_iter().find(|v| v.block_id == block_id) {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("nothing asked in {block_id}");
    }

    fn spawn_ask(state: &AppState, block_id: &str, q: Question, window: Duration) -> tokio::task::JoinHandle<Result<Answer, String>> {
        let (state, block_id) = (state.clone(), block_id.to_string());
        tokio::spawn(async move { ask_for(&state, &block_id, q, window).await })
    }

    #[tokio::test]
    async fn with_no_window_connected_nothing_is_asked_and_nothing_is_approved() {
        let state = test_state();
        let b = block();
        let err = ask_for(&state, &b, question(), Duration::from_secs(60)).await.unwrap_err();
        assert!(err.contains("no AgentMux window is connected"), "{err}");
        assert!(pending().iter().all(|v| v.block_id != b));
    }

    #[tokio::test]
    async fn a_question_with_no_pane_is_refused() {
        let state = with_window().await;
        assert!(ask_for(&state, "", question(), Duration::from_secs(60)).await.unwrap_err().contains("no pane"));
    }

    #[tokio::test]
    async fn the_persons_yes_is_the_answer_and_the_question_goes() {
        let state = with_window().await;
        let b = block();
        let task = spawn_ask(&state, &b, question(), Duration::from_secs(60));
        let v = asked_in(&b).await;
        assert_eq!((v.title.as_str(), v.ok_label.as_str(), v.cancel_label.as_str(), v.destructive), ("End a session on db1", "End Session", "Keep It", true));
        assert_eq!(decide(&v.id, true, false), "answered");
        assert_eq!(task.await.unwrap().unwrap(), Answer { answered: true, approve: true, checkbox: false });
        assert!(pending().iter().all(|p| p.block_id != b), "the question goes with its answer");
        assert_eq!(decide(&v.id, true, false), "too_late");
    }

    /// The toast and the "waiting on you" list word a `confirm` for a pane the person
    /// clicked in; `consent` is worded for an agent that asks.
    #[tokio::test]
    async fn the_attention_it_raises_is_a_panes_question_not_an_agents_consent() {
        let state = with_window().await;
        let b = block();
        let task = spawn_ask(&state, &b, question(), Duration::from_secs(60));
        let v = asked_in(&b).await;
        let raised = crate::backend::user_attention::open_requests().into_iter().find(|a| a.block_id == b);
        assert_eq!(raised.map(|a| a.kind).as_deref(), Some("confirm"));
        decide(&v.id, false, false);
        task.await.unwrap().unwrap();
        assert!(crate::backend::user_attention::open_requests().iter().all(|a| a.block_id != b));
    }

    #[tokio::test]
    async fn a_no_is_answered_and_not_approved() {
        let state = with_window().await;
        let b = block();
        let task = spawn_ask(&state, &b, question(), Duration::from_secs(60));
        let v = asked_in(&b).await;
        assert_eq!(decide(&v.id, false, false), "answered");
        assert_eq!(task.await.unwrap().unwrap(), Answer { answered: true, approve: false, checkbox: false });
    }

    #[tokio::test]
    async fn nobody_answering_leaves_it_unanswered_never_approved() {
        let state = with_window().await;
        let b = block();
        let answer = ask_for(&state, &b, question(), Duration::from_millis(60)).await.unwrap();
        assert_eq!(answer, Answer::default());
        assert!(pending().iter().all(|p| p.block_id != b));
    }

    #[tokio::test]
    async fn the_tick_box_counts_only_when_the_question_has_one() {
        let state = with_window().await;
        let b = block();
        let mut q = question();
        q.checkbox = "Don't ask again for db1".to_string();
        let task = spawn_ask(&state, &b, q, Duration::from_secs(60));
        let v = asked_in(&b).await;
        assert_eq!(v.checkbox, "Don't ask again for db1");
        decide(&v.id, true, true);
        assert!(task.await.unwrap().unwrap().checkbox);

        let b2 = block();
        let task = spawn_ask(&state, &b2, question(), Duration::from_secs(60));
        let v = asked_in(&b2).await;
        decide(&v.id, true, true);
        assert!(!task.await.unwrap().unwrap().checkbox, "no box was shown, so none was ticked");
    }

    #[tokio::test]
    async fn a_caller_dropped_while_it_waits_takes_its_question_with_it() {
        let state = with_window().await;
        let b = block();
        let task = spawn_ask(&state, &b, question(), Duration::from_secs(60));
        let v = asked_in(&b).await;
        task.abort();
        let _ = task.await;
        assert!(pending().iter().all(|p| p.block_id != b));
        assert_eq!(decide(&v.id, true, false), "too_late");
    }

    #[test]
    fn a_question_reads_the_approval_windows_json() {
        let q = Question::from_json(&serde_json::json!({
            "kind": "consent", "title": "Install git", "message": "Run:\n\nwinget install git", "ok_label": "Install",
            "cancel_label": "Cancel", "destructive": true, "checkbox": "Don't ask again",
        }));
        assert_eq!(q.title, "Install git");
        assert!(q.message.contains('\n'));
        assert_eq!((q.ok_label.as_str(), q.cancel_label.as_str(), q.destructive, q.checkbox.as_str()), ("Install", "Cancel", true, "Don't ask again"));
        let bare = Question::from_json(&serde_json::json!({}));
        assert_eq!((bare.ok_label.as_str(), bare.cancel_label.as_str(), bare.destructive), ("OK", "Cancel", false));
        assert!(!bare.title.is_empty());
    }
}
