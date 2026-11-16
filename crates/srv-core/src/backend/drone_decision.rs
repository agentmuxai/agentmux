// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The answer a Drone run waits for when it needs attention
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "the step that was in flight").
//!
//! After a restart, a step with effects that was running may already have done
//! its work, so the run does not repeat it: it waits, `needs_attention`, for a
//! person or an agent to say what to do with that step. It lives in the core so
//! the RPC handler can answer without reaching up into `drone/`; the executor is
//! the one that waits.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use tokio::sync::oneshot;

/// What to do with the step that was running. Cancelling the run is not here:
/// it is `cancel_run`, which ends a waiting run as it ends any other.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Run the step again. It may repeat what it did before the restart.
    RunAgain,
    /// Do not run it: carry on as if it had finished with `output`. The output is
    /// outside text (tainted) whoever supplied it, since the run cannot tell a
    /// person's typing from an agent's.
    Skip { output: Value },
}

/// The runs waiting for an answer, by run id.
static WAITING: LazyLock<Mutex<HashMap<String, oneshot::Sender<Decision>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The run `run_id` is waiting: the receiver gets the answer. A run that waits
/// again replaces its earlier wait.
pub fn wait_for_decision(run_id: &str) -> oneshot::Receiver<Decision> {
    let (tx, rx) = oneshot::channel();
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).insert(run_id.to_string(), tx);
    rx
}

/// The run `run_id` is no longer waiting (it was answered, or it ended).
pub fn stop_waiting(run_id: &str) {
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(run_id);
}

/// Whether the run `run_id` is waiting for an answer.
pub fn is_waiting(run_id: &str) -> bool {
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).contains_key(run_id)
}

/// Answer the run `run_id`. `Err` when it is not waiting (it was answered
/// already, it ended, or it never needed attention).
pub fn decide(run_id: &str, decision: Decision) -> Result<(), String> {
    let waiting = WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(run_id);
    let Some(tx) = waiting else { return Err(format!("run {run_id} is not waiting for an answer")) };
    tx.send(decision).map_err(|_| format!("run {run_id} stopped waiting"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_waiting_run_gets_its_answer_once() {
        let rx = wait_for_decision("decision-test-1");
        assert!(is_waiting("decision-test-1"));
        decide("decision-test-1", Decision::Skip { output: json!({ "value": 1 }) }).unwrap();
        assert_eq!(rx.await.unwrap(), Decision::Skip { output: json!({ "value": 1 }) });
        assert!(!is_waiting("decision-test-1"));
        assert!(decide("decision-test-1", Decision::RunAgain).is_err(), "a second answer finds nothing waiting");
    }

    #[test]
    fn a_run_that_is_not_waiting_cannot_be_answered() {
        assert!(decide("decision-test-none", Decision::RunAgain).is_err());
    }

    #[tokio::test]
    async fn a_run_that_stopped_waiting_is_not_answered() {
        let rx = wait_for_decision("decision-test-2");
        drop(rx);
        // The receiver is gone: the answer has nowhere to go, and says so.
        assert!(decide("decision-test-2", Decision::RunAgain).is_err());
        let _rx = wait_for_decision("decision-test-3");
        stop_waiting("decision-test-3");
        assert!(!is_waiting("decision-test-3"));
    }
}
