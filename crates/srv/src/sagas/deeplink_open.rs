// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's deep-link open, carried to the window and its outcome carried back
//! (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §4.2, R3, G9).
//!
//! srv's guard (`server::deeplink_guard`) has already let the open through when it reaches here. srv publishes
//! `deeplink:open`; the window that holds the tab carries it out with the app's executor, which applies the rules only
//! the window can (R5, the caps and the typing queue of G3, G6), and answers `deeplink.open.done`. The answer is
//! accepted only from the proven AgentMux window (`operator_only`), so an agent can't report an outcome for itself.
//!
//! An open the window has not answered within [`WAIT_FOR`] is `queued` for the caller (the person is typing, the
//! app is starting); its outcome is recorded when it comes, until [`KEEP_FOR`].

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::mps_events::DeepLinkOpenEvent;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::AppState;
use agentmux_common::time::now_ms;

pub const COMMAND_DEEPLINK_OPEN_DONE: &str = "deeplink.open.done";
/// How long the caller waits for the window's answer before it is told `queued`. Longer than the typing queue's
/// own wait (two seconds after the last keystroke, ten at most), so an open that waited for the person still answers.
pub const WAIT_FOR: Duration = Duration::from_secs(13);
/// How long a window may still carry an open out, and srv still records its outcome.
pub const KEEP_FOR: Duration = Duration::from_secs(60);
/// How many opens a window that connects late is given (§8: the app is still starting; a queue of ten).
const REPLAY_EVENTS: usize = 10;

/// What the window answers: the outcome of R3, as the executor gave it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DeepLinkOutcome {
    #[ts(type = r#""opened" | "reused" | "queued" | "refused""#)]
    pub outcome: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub block_id: Option<String>,
    /// `missing` when the view opened but the place inside it is gone (§8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remedy: Option<String>,
}

impl DeepLinkOutcome {
    pub fn refused(label: &str, code: &str, reason: &str, remedy: Option<String>) -> Self {
        Self {
            outcome: "refused".into(),
            label: label.into(),
            reason_code: Some(code.into()),
            reason: Some(reason.into()),
            remedy,
            ..Default::default()
        }
    }

    pub fn queued(label: &str, code: &str, reason: &str) -> Self {
        Self { outcome: "queued".into(), label: label.into(), reason_code: Some(code.into()), reason: Some(reason.into()), ..Default::default() }
    }

    /// Whether a window's answer is one srv can record: a known outcome, and text of a sane length.
    fn well_formed(&self) -> bool {
        let short = |s: &Option<String>| s.as_ref().is_none_or(|t| t.len() <= 500);
        matches!(self.outcome.as_str(), "opened" | "reused" | "queued" | "refused")
            && self.label.len() <= 200
            && short(&self.block_id)
            && short(&self.place)
            && short(&self.reason_code)
            && short(&self.reason)
            && short(&self.remedy)
    }
}

/// Data for `deeplink.open.done`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandDeepLinkOpenDoneData {
    pub request_id: String,
    pub outcome: DeepLinkOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DeepLinkOpenDoneResult {
    #[ts(type = r#""recorded" | "too_late""#)]
    pub status: String,
}

/// What to do with an outcome that comes after the caller stopped waiting: record it.
pub type Late = Box<dyn FnOnce(&DeepLinkOutcome) + Send>;

struct Entry {
    answer: Option<DeepLinkOutcome>,
    wake: Arc<tokio::sync::Notify>,
    expires_at_ms: i64,
    /// Set once the caller has stopped waiting.
    late: Option<Late>,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn entries() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Publish `event` and wait up to `window` for the window's outcome. `None` when none came in time: the caller says
/// `queued`, and `late` records the outcome if it comes before the event expires.
pub async fn carry_out(state: &AppState, event: DeepLinkOpenEvent, window: Duration, late: Late) -> Option<DeepLinkOutcome> {
    let id = event.request_id.clone();
    let wake = Arc::new(tokio::sync::Notify::new());
    {
        let mut map = entries();
        let now = now_ms();
        map.retain(|_, e| e.expires_at_ms > now);
        map.insert(id.clone(), Entry { answer: None, wake: wake.clone(), expires_at_ms: event.expires_at_ms, late: None });
    }
    state.broker.publish_event::<crate::backend::mps_events::DeepLinkOpen>(&event, vec![], REPLAY_EVENTS);
    let _ = tokio::time::timeout(window, wake.notified()).await;
    let mut map = entries();
    let Some(entry) = map.get_mut(&id) else { return None };
    match entry.answer.take() {
        Some(answer) => {
            map.remove(&id);
            Some(answer)
        }
        None => {
            entry.late = Some(late);
            None
        }
    }
}

/// The window's outcome for open `id`.
pub fn done(id: &str, outcome: DeepLinkOutcome) -> &'static str {
    if !outcome.well_formed() {
        return "too_late";
    }
    let mut map = entries();
    let Some(entry) = map.get_mut(id).filter(|e| e.expires_at_ms > now_ms() && e.answer.is_none()) else {
        return "too_late";
    };
    if let Some(late) = entry.late.take() {
        late(&outcome);
        map.remove(id);
        return "recorded";
    }
    entry.answer = Some(outcome);
    entry.wake.notify_one();
    "recorded"
}

/// `deeplink.open.done`, accepted only from the proven AgentMux window (`operator_only`), never from an agent.
pub fn register(engine: &Arc<WshRpcEngine>) {
    engine.register_typed(COMMAND_DEEPLINK_OPEN_DONE, move |cmd: CommandDeepLinkOpenDoneData, ctx| async move {
        ctx.operator_only("only the AgentMux window reports what an open did")?;
        Ok(DeepLinkOpenDoneResult { status: done(&cmd.request_id, cmd.outcome).to_string() })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    fn event(id: &str) -> DeepLinkOpenEvent {
        DeepLinkOpenEvent {
            request_id: id.into(),
            link: "agentmux://settings/recording".into(),
            agent_id: "a1".into(),
            caller_block_id: None,
            tab_id: Some("t1".into()),
            window_id: None,
            target: "beside".into(),
            expires_at_ms: now_ms() + KEEP_FOR.as_millis() as i64,
        }
    }

    fn opened() -> DeepLinkOutcome {
        DeepLinkOutcome { outcome: "opened".into(), label: "Settings › Recording".into(), block_id: Some("b9".into()), ..Default::default() }
    }

    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[tokio::test]
    async fn the_windows_outcome_is_the_callers_answer() {
        let state = test_state();
        let rid = id();
        let task = {
            let (state, rid) = (state.clone(), rid.clone());
            tokio::spawn(async move { carry_out(&state, event(&rid), Duration::from_secs(5), Box::new(|_| {})).await })
        };
        for _ in 0..200 {
            if done(&rid, opened()) == "recorded" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(task.await.unwrap(), Some(opened()));
        assert_eq!(done(&rid, opened()), "too_late", "an open is answered once");
    }

    #[tokio::test]
    async fn no_answer_in_time_is_queued_and_a_late_answer_is_still_recorded() {
        let state = test_state();
        let rid = id();
        let seen = Arc::new(Mutex::new(None));
        let rec = seen.clone();
        let got = carry_out(&state, event(&rid), Duration::from_millis(30), Box::new(move |o| *rec.lock().unwrap() = Some(o.clone()))).await;
        assert_eq!(got, None);
        assert_eq!(done(&rid, opened()), "recorded");
        assert_eq!(seen.lock().unwrap().clone(), Some(opened()));
    }

    #[test]
    fn an_answer_for_no_open_or_a_malformed_one_is_too_late() {
        assert_eq!(done("nothing-asked", opened()), "too_late");
        let mut odd = opened();
        odd.outcome = "approved".into();
        assert_eq!(done("nothing-asked", odd), "too_late");
        let mut long = opened();
        long.label = "x".repeat(201);
        assert!(!long.well_formed());
    }

    #[tokio::test]
    async fn an_expired_open_takes_no_answer() {
        let state = test_state();
        let rid = id();
        let mut ev = event(&rid);
        ev.expires_at_ms = now_ms() - 1;
        let _ = carry_out(&state, ev, Duration::from_millis(10), Box::new(|_| panic!("an expired open records nothing late"))).await;
        assert_eq!(done(&rid, opened()), "too_late");
    }
}
