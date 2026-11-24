// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The approvals a Drone run waits on, read and answered over the websocket
//! (`SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §6.4 a).
//!
//! * `listdroneapprovals`: the approvals open now, of one drone or of all.
//! * `answerdroneapproval`: yes or no. **Only the person's own client may answer.** An answer given as the person is
//!   the person's voice, and an agent giving it would be putting words in their mouth, the one thing agents may not
//!   do; so a connection that is an agent, or that has not proved it is the person's window, is refused. The answer
//!   is recorded as who gave it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::drone_approval::{self, DroneApproval};
use crate::backend::rpc::actor::actor_of;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, COMMAND_ANSWER_DRONE_APPROVAL, COMMAND_LIST_DRONE_APPROVALS};
use crate::server::AppState;

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ListDroneApprovalsReq {
    /// The drone, or empty for every drone.
    #[serde(default)]
    pub drone_id: String,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AnswerDroneApprovalReq {
    pub run_id: String,
    pub block_id: String,
    pub approve: bool,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AnswerDroneApprovalResp {
    /// Who the record says answered.
    pub answered_by: String,
}

pub(crate) fn list_approvals(cmd: ListDroneApprovalsReq) -> Vec<DroneApproval> {
    drone_approval::list(cmd.drone_id.trim())
}

pub(crate) fn answer_approval(ctx: &RpcContext, cmd: AnswerDroneApprovalReq) -> Result<AnswerDroneApprovalResp, String> {
    let actor = actor_of(ctx);
    if !actor.is_operator() {
        return Err("FORBIDDEN: an approval is answered as the person, and only from the person's own AgentMux window".to_string());
    }
    let by = actor.label();
    drone_approval::answer(&cmd.run_id, &cmd.block_id, cmd.approve, &by).map_err(|e| format!("NOT_FOUND: {e}"))?;
    tracing::info!(run_id = %cmd.run_id, block_id = %cmd.block_id, approve = cmd.approve, by = %by, "answerdroneapproval");
    Ok(AnswerDroneApprovalResp { answered_by: by })
}

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_LIST_DRONE_APPROVALS, move |cmd: Option<ListDroneApprovalsReq>, _ctx| async move {
        Ok::<_, String>(list_approvals(cmd.unwrap_or(ListDroneApprovalsReq { drone_id: String::new() })))
    });
    engine.register_typed(COMMAND_ANSWER_DRONE_APPROVAL, move |cmd: AnswerDroneApprovalReq, ctx| async move { answer_approval(&ctx, cmd) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::drone_approval::register as open;
    use serde_json::json;

    fn card(run: &str) -> DroneApproval {
        DroneApproval { run_id: run.into(), block_id: "ap".into(), drone_id: "h-d".into(), title: "t".into(), items: vec![], deadline_ms: 1 }
    }

    fn ask(approve: bool, run: &str) -> AnswerDroneApprovalReq {
        AnswerDroneApprovalReq { run_id: run.into(), block_id: "ap".into(), approve }
    }

    #[tokio::test]
    async fn the_persons_window_answers_and_the_record_says_who() {
        let key = crate::backend::drone_wait::new_key();
        let rx = crate::backend::drone_wait::open(&key);
        open(card("h-run1"), &key);
        let done = answer_approval(&RpcContext::operator_for_tests(), ask(true, "h-run1")).unwrap();
        assert_eq!(done.answered_by, "operator");
        assert_eq!(rx.await.unwrap(), json!({ "approve": true, "by": "operator" }));
        drone_approval::forget("h-run1", "ap");
    }

    #[tokio::test]
    async fn an_agent_and_a_connection_that_proved_nothing_cannot_answer() {
        let key = crate::backend::drone_wait::new_key();
        let mut rx = crate::backend::drone_wait::open(&key);
        open(card("h-run2"), &key);
        for ctx in [RpcContext { agent_id: "opaz".into(), ..Default::default() }, RpcContext::default()] {
            let err = answer_approval(&ctx, ask(true, "h-run2")).unwrap_err();
            assert!(err.starts_with("FORBIDDEN"), "{err}");
        }
        assert!(rx.try_recv().is_err(), "nothing was sent to the run");
        drone_approval::forget("h-run2", "ap");
        let gone = answer_approval(&RpcContext::operator_for_tests(), ask(true, "h-run2")).unwrap_err();
        assert!(gone.starts_with("NOT_FOUND"), "{gone}");
    }

    #[test]
    fn the_list_names_a_drone_or_all() {
        open(card("h-run3"), "k");
        assert_eq!(list_approvals(ListDroneApprovalsReq { drone_id: "h-d".into() }).iter().filter(|a| a.run_id == "h-run3").count(), 1);
        assert!(list_approvals(ListDroneApprovalsReq { drone_id: "someone-else".into() }).is_empty());
        drone_approval::forget("h-run3", "ap");
    }
}
