// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::rpc_types::{AuditEnd, AuditLinks};
use crate::backend::storage::work_board::{NewCard, WorkEvent};
use crate::backend::storage::work_queue::WorkItem;

fn entry(kind: &str, ts_ms: u64, agent: Option<&str>) -> AuditEntry {
    AuditEntry {
        id: format!("{kind}-{ts_ms}"),
        ts_ms,
        kind: kind.into(),
        agent_id: agent.map(str::to_string),
        from: None,
        to: None,
        summary: String::new(),
        outcome: None,
        reason: None,
        cost: None,
        links: AuditLinks::default(),
        provenance: None,
        prompt: None,
        answer: None,
        level: None,
        standing: None,
    }
}

fn end(agent: &str) -> AuditEnd {
    AuditEnd { agent_id: agent.into(), links: AuditLinks::default() }
}

fn link(num: i64) -> AuditTaskLink {
    AuditTaskLink { num, task_ref: format!("#{num}"), pr_url: None }
}

#[test]
fn a_row_gets_the_task_its_agent_held_and_a_jekt_gets_one_for_each_end() {
    let held = |agent: &str, _at: u64| match agent {
        "loap" => Some(link(12)),
        "agenty" => Some(link(7)),
        _ => None,
    };
    let mut jekt = entry("jekt", 1000, None);
    jekt.from = Some(end("agenty"));
    jekt.to = Some(end("loap"));
    let mut rows = vec![entry("tool", 1000, Some("loap")), entry("tool", 1000, Some("stranger")), jekt];
    attach(&mut rows, &held);
    assert_eq!(rows[0].links.task, Some(link(12)));
    assert_eq!(rows[1].links.task, None, "an agent holding nothing gets no link");
    assert_eq!(rows[2].from.as_ref().unwrap().links.task, Some(link(7)));
    assert_eq!(rows[2].to.as_ref().unwrap().links.task, Some(link(12)));
    assert_eq!(rows[2].links.task, None, "a jekt names no agent of its own");
}

#[test]
fn the_kanban_gives_the_task_and_its_pr_once_per_page() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    let item = WorkItem {
        id: "t1".into(),
        title: "task".into(),
        payload: "do it".into(),
        kind: String::new(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: "tester".into(),
        created_by_uid: String::new(),
        created_at: 1000,
        updated_at: 1000,
        not_before: None,
        result: String::new(),
    };
    s.work_queue_enqueue(&item).unwrap();
    s.work_card_create("t1", &NewCard::default(), 1000).unwrap();
    s.work_card_update("t1", &crate::backend::storage::work_board::CardUpdate { pr_url: Some("https://github.com/o/r/pull/9".into()), ..Default::default() }, 1500).unwrap();
    s.work_event_add(&WorkEvent { work_id: "t1".into(), at: 2000, actor: "loap".into(), kind: "claimed".into(), ..Default::default() }).unwrap();
    let held = from_store(&s);
    let got = held("loap", 3000).expect("the task it held");
    assert_eq!((got.task_ref.as_str(), got.pr_url.as_deref()), ("#1", Some("https://github.com/o/r/pull/9")));
    assert_eq!(held("loap", 1500), None, "nothing before the claim");
    assert_eq!(held("naki", 3000), None);
}
