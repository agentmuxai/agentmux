// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The approvals a Drone run is waiting for (`SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §4.2 and §6.4).
//!
//! An Approval block stops its run and shows a person the proposed action and the inputs it was made from, the
//! ones that came from outside text marked as such. Each open approval is kept here, with the key of the wait the
//! run holds. The key never leaves AgentMux: an answer names the run and the block, and this finds the key. That
//! is what keeps an agent, which can read a run's events, from answering for the person: the answering route
//! accepts only the person's own client (`server::drone_approval_handlers`).
//!
//! It lives in the core, beside `drone_wait`, so the handlers can answer without reaching up into `drone/`.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;

/// One thing the card shows: a label, the text, and whether it came from outside text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ApprovalItem {
    pub label: String,
    pub text: String,
    /// Outside text (an event, an API body, an agent's reply): marked on the card.
    pub tainted: bool,
}

/// An approval a run waits on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DroneApproval {
    pub run_id: String,
    pub block_id: String,
    pub drone_id: String,
    /// The proposed action, in the graph's own words.
    pub title: String,
    pub items: Vec<ApprovalItem>,
    /// When silence rejects it, in milliseconds since 1970.
    #[ts(type = "number")]
    pub deadline_ms: i64,
}

struct Open {
    approval: DroneApproval,
    key: String,
}

static OPEN: LazyLock<Mutex<HashMap<(String, String), Open>>> = LazyLock::new(Mutex::default);

fn open_map() -> std::sync::MutexGuard<'static, HashMap<(String, String), Open>> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// The run holds this approval, waiting on `key`. A block run again (a restart took the wait up) replaces its entry.
pub fn register(approval: DroneApproval, key: &str) {
    let id = (approval.run_id.clone(), approval.block_id.clone());
    open_map().insert(id, Open { approval, key: key.to_string() });
}

/// The approval is over, however it ended.
pub fn forget(run_id: &str, block_id: &str) {
    open_map().remove(&(run_id.to_string(), block_id.to_string()));
}

/// The approvals open now, of `drone_id` when it is named, newest deadline last.
pub fn list(drone_id: &str) -> Vec<DroneApproval> {
    let mut out: Vec<DroneApproval> = open_map().values().map(|o| o.approval.clone()).filter(|a| drone_id.is_empty() || a.drone_id == drone_id).collect();
    out.sort_by(|a, b| (a.deadline_ms, &a.run_id, &a.block_id).cmp(&(b.deadline_ms, &b.run_id, &b.block_id)));
    out
}

/// Answer the approval of `block_id` in `run_id`: yes or no, by `by` (who the record says answered). `Err` when none
/// is open (it was answered, it timed out, or the run ended).
pub fn answer(run_id: &str, block_id: &str, approve: bool, by: &str) -> Result<(), String> {
    let key = {
        let all = open_map();
        match all.get(&(run_id.to_string(), block_id.to_string())) {
            Some(o) => o.key.clone(),
            None => return Err("nothing is waiting for an answer there: it was answered, it timed out, or the run ended".to_string()),
        }
    };
    crate::backend::drone_wait::signal(&key, json!({ "approve": approve, "by": by }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(run: &str, block: &str, drone: &str, deadline: i64) -> DroneApproval {
        DroneApproval { run_id: run.into(), block_id: block.into(), drone_id: drone.into(), title: "Merge it".into(), items: vec![], deadline_ms: deadline }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_wait_the_run_holds_and_only_once() {
        let key = crate::backend::drone_wait::new_key();
        let rx = crate::backend::drone_wait::open(&key);
        register(card("r1", "b", "d1", 5), &key);
        answer("r1", "b", true, "operator").unwrap();
        assert_eq!(rx.await.unwrap(), json!({ "approve": true, "by": "operator" }));
        assert!(answer("r1", "b", false, "operator").is_err(), "a second answer finds nothing waiting");
        forget("r1", "b");
        assert!(answer("r1", "b", true, "operator").unwrap_err().contains("nothing is waiting"));
    }

    #[test]
    fn the_list_is_by_drone_and_oldest_deadline_first_and_forgetting_removes() {
        register(card("r-a", "b", "list-d1", 20), "k1");
        register(card("r-b", "b", "list-d1", 10), "k2");
        register(card("r-c", "b", "list-d2", 1), "k3");
        let mine: Vec<String> = list("list-d1").into_iter().map(|a| a.run_id).collect();
        assert_eq!(mine, ["r-b", "r-a"]);
        assert!(list("").iter().any(|a| a.run_id == "r-c"), "no drone named lists every one");
        forget("r-b", "b");
        assert_eq!(list("list-d1").len(), 1);
        for r in ["r-a", "r-c"] {
            forget(r, "b");
        }
    }
}
