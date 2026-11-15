// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The wire shape of the events that put a countdown or a question in front of the person, and of a drone run's
//! stream, plus the names they go out under. The frontend reads these field names and event names, so they are
//! pinned here by the old `sagas::` and `drone::` paths, which must keep resolving whichever module holds the types.

use serde_json::json;

use super::agent_notice::{GrantsEvent, NoticeView, EVENT_AGENT_GRANTS, EVENT_AGENT_NOTICE};
use super::pane_confirm::{PaneConfirmView, EVENT_PANE_CONFIRM};
use super::sensitive_window::{SensitiveClearedEvent, SensitiveView, EVENT_SENSITIVE_CLEARED, EVENT_SENSITIVE_PENDING};
use crate::drone::executor::RunEvent;

#[test]
fn the_event_names_are_what_the_frontend_listens_for() {
    assert_eq!(EVENT_SENSITIVE_PENDING, "sensitive:pending");
    assert_eq!(EVENT_SENSITIVE_CLEARED, "sensitive:cleared");
    assert_eq!(EVENT_AGENT_NOTICE, "agent:notice");
    assert_eq!(EVENT_AGENT_GRANTS, "agent:grants");
    assert_eq!(EVENT_PANE_CONFIRM, "pane:confirm");
}

fn sensitive() -> SensitiveView {
    SensitiveView {
        request_id: "req-1".into(),
        op: "vault.use".into(),
        summary: "Use the card".into(),
        cancel_label: "Cancel".into(),
        now_label: "Go now".into(),
        requester: "korp".into(),
        approver: None,
        deadline_ms: 2000,
        opened_ms: 1000,
        status: "pending",
        error: None,
    }
}

#[test]
fn a_sensitive_view_omits_what_is_absent_and_carries_what_is_set() {
    assert_eq!(
        serde_json::to_value(sensitive()).unwrap(),
        json!({
            "request_id": "req-1", "op": "vault.use", "summary": "Use the card", "cancel_label": "Cancel",
            "now_label": "Go now", "requester": "korp", "deadline_ms": 2000, "opened_ms": 1000, "status": "pending",
        })
    );
    let mut v = sensitive();
    v.approver = Some("super".into());
    v.error = Some("boom".into());
    v.status = "failed";
    let got = serde_json::to_value(v).unwrap();
    assert_eq!((got["approver"].as_str(), got["error"].as_str(), got["status"].as_str()), (Some("super"), Some("boom"), Some("failed")));

    assert_eq!(
        serde_json::to_value(SensitiveClearedEvent { request_id: "req-1".into(), outcome: "vetoed".into() }).unwrap(),
        json!({ "request_id": "req-1", "outcome": "vetoed" })
    );
}

#[test]
fn a_notice_a_grants_event_and_a_pane_question_keep_their_wire_shape() {
    let notice = NoticeView {
        id: "n1".into(),
        kind: "spawn",
        block_id: "blk".into(),
        agent: "posa".into(),
        headline: "Posa is starting Korp-2".into(),
        body: "Sonnet via Claude".into(),
        quote: "build it".into(),
        always_label: "Always".into(),
        deadline_ms: 90,
        opened_ms: 60,
        status: "pending",
        outcome: String::new(),
    };
    assert_eq!(
        serde_json::to_value(notice).unwrap(),
        json!({
            "id": "n1", "kind": "spawn", "block_id": "blk", "agent": "posa", "headline": "Posa is starting Korp-2",
            "body": "Sonnet via Claude", "quote": "build it", "always_label": "Always", "deadline_ms": 90,
            "opened_ms": 60, "status": "pending", "outcome": "",
        })
    );
    assert_eq!(
        serde_json::to_value(GrantsEvent { topic: "memory", agents: vec!["a".into(), "b".into()] }).unwrap(),
        json!({ "topic": "memory", "agents": ["a", "b"] })
    );
    let q = PaneConfirmView {
        id: "q1".into(),
        block_id: "blk".into(),
        title: "Delete?".into(),
        message: "rm -rf".into(),
        ok_label: "Yes".into(),
        cancel_label: "No".into(),
        destructive: true,
        checkbox: "Don't ask again".into(),
        deadline_ms: 120,
        status: "cleared",
    };
    assert_eq!(
        serde_json::to_value(q).unwrap(),
        json!({
            "id": "q1", "block_id": "blk", "title": "Delete?", "message": "rm -rf", "ok_label": "Yes",
            "cancel_label": "No", "destructive": true, "checkbox": "Don't ask again", "deadline_ms": 120,
            "status": "cleared",
        })
    );
}

#[test]
fn a_drone_run_event_is_tagged_by_its_kind() {
    let run = |e: RunEvent| serde_json::to_value(e).unwrap();
    assert_eq!(run(RunEvent::RunStarted { run_id: "r".into(), drone_id: "d".into() }), json!({ "kind": "run_started", "run_id": "r", "drone_id": "d" }));
    assert_eq!(run(RunEvent::BlockStarted { run_id: "r".into(), block_id: "b".into() }), json!({ "kind": "block_started", "run_id": "r", "block_id": "b" }));
    assert_eq!(
        run(RunEvent::BlockDone { run_id: "r".into(), block_id: "b".into(), output: json!(1) }),
        json!({ "kind": "block_done", "run_id": "r", "block_id": "b", "output": 1 })
    );
    assert_eq!(
        run(RunEvent::BlockError { run_id: "r".into(), block_id: "b".into(), error: "e".into() }),
        json!({ "kind": "block_error", "run_id": "r", "block_id": "b", "error": "e" })
    );
    assert_eq!(run(RunEvent::RunDone { run_id: "r".into(), output: json!({"ok": true}) }), json!({ "kind": "run_done", "run_id": "r", "output": {"ok": true} }));
    assert_eq!(run(RunEvent::RunFailed { run_id: "r".into(), error: "e".into() }), json!({ "kind": "run_failed", "run_id": "r", "error": "e" }));
    assert_eq!(run(RunEvent::RunCancelled { run_id: "r".into(), reason: "why".into() }), json!({ "kind": "run_cancelled", "run_id": "r", "reason": "why" }));
    assert_eq!(
        run(RunEvent::NeedsAttention { run_id: "r".into(), block_id: "b".into(), reason: "why".into() }),
        json!({ "kind": "needs_attention", "run_id": "r", "block_id": "b", "reason": "why" })
    );
    assert_eq!(
        run(RunEvent::AttentionAnswered { run_id: "r".into(), block_id: "b".into(), choice: "skip".into() }),
        json!({ "kind": "attention_answered", "run_id": "r", "block_id": "b", "choice": "skip" })
    );
    assert_eq!(
        run(RunEvent::Waiting { run_id: "r".into(), block_id: "w".into(), resume_at: Some(9), on: None, key: None }),
        json!({ "kind": "waiting", "run_id": "r", "block_id": "w", "resume_at": 9 })
    );
    assert_eq!(
        run(RunEvent::Waiting { run_id: "r".into(), block_id: "w".into(), resume_at: None, on: Some("e".into()), key: Some("k".into()) }),
        json!({ "kind": "waiting", "run_id": "r", "block_id": "w", "on": "e", "key": "k" })
    );
    assert_eq!(run(RunEvent::WaitEnded { run_id: "r".into(), block_id: "w".into(), late: true }), json!({ "kind": "wait_ended", "run_id": "r", "block_id": "w", "late": true }));
    assert_eq!(run(RunEvent::Resumed { run_id: "r".into(), step: 4 }), json!({ "kind": "resumed", "run_id": "r", "step": 4 }));
    assert_eq!(
        run(RunEvent::StepCheckpointed { run_id: "r".into(), step: 3, block_id: Some("b".into()) }),
        json!({ "kind": "step_checkpointed", "run_id": "r", "step": 3, "block_id": "b" })
    );
    assert_eq!(
        run(RunEvent::StepCheckpointed { run_id: "r".into(), step: 0, block_id: None }),
        json!({ "kind": "step_checkpointed", "run_id": "r", "step": 0 })
    );
    assert_eq!(run(RunEvent::LoopIteration { run_id: "r".into(), block_id: "g".into(), iteration: 2 }), json!({ "kind": "loop_iteration", "run_id": "r", "block_id": "g", "iteration": 2 }));
    assert_eq!(run(RunEvent::CapReached { run_id: "r".into(), block_id: "g".into(), reason: "why".into() }), json!({ "kind": "cap_reached", "run_id": "r", "block_id": "g", "reason": "why" }));
}
