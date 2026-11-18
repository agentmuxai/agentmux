// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What an agent reads of its workspace's stop.

use serde_json::json;

use super::*;
use crate::server::host_control::Reason;

#[test]
fn no_stop_pending_reads_as_not_stopping_with_nothing_asked() {
    assert_eq!(
        workspace_view(None, None),
        json!({ "stopping": false, "stop_at_ms": null, "reason_id": null, "reason": null, "keep_running": null })
    );
}

#[test]
fn a_pending_stop_reads_with_its_time_and_the_fixed_reason() {
    let stop = Stop { at_ms: 1_800_000_600_000, reason: Reason::Timer { idle_minutes: 50 } };
    let v = workspace_view(Some(stop), None);
    assert_eq!(v["stopping"], true);
    assert_eq!(v["stop_at_ms"], 1_800_000_600_000u64);
    assert_eq!(v["reason_id"], "timer");
    assert_eq!(v["reason"], "nothing has happened for 50 minutes");
    assert_eq!(v["keep_running"], json!(null));
}

#[test]
fn what_an_agent_asked_for_reads_as_that_agent() {
    let stop = Stop { at_ms: 1_800_000_600_000, reason: Reason::Person };
    let keep = KeepRunning { minutes: 30, by: "agent:agenta".to_string(), at_ms: 1_800_000_000_000 };
    let v = workspace_view(Some(stop), Some(keep));
    assert_eq!(v["keep_running"], json!({ "minutes": 30, "by": "agent:agenta", "at_ms": 1_800_000_000_000u64 }));
}
