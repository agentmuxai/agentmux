// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::pr_flow::PrFlowSnapshot;

fn card(num: i64, agent: &str, repo: &str, number: u64) -> CardIn {
    CardIn {
        task: ReviewsTask { task_ref: format!("#{num}"), num, title: String::new(), state: "claimed".into() },
        agent: agent.into(),
        repo: repo.into(),
        number,
        branch: String::new(),
        merged_at_ms: None,
        released_in: String::new(),
        updated_at_ms: num,
    }
}

fn list() -> ReviewsList {
    reviews::build(
        &[card(1, "lazo", "agentmuxai/agentmux", 10), card(2, "camper", "agentmuxai/agentmux", 11), card(3, "lazo", "agentmuxai/muxcode", 12)],
        &PrFlowSnapshot::default(),
        100,
    )
}

fn numbers(l: &ReviewsList) -> Vec<u64> {
    let mut n: Vec<u64> = l.rows.iter().map(|r| r.number).collect();
    n.sort();
    n
}

fn q(agent: Option<&str>, waits_on: Option<&str>, task: Option<&str>, repo: Option<&str>) -> ReviewsQuery {
    let own = |s: Option<&str>| s.map(str::to_string);
    ReviewsQuery { agent: own(agent), waits_on: own(waits_on), task: own(task), repo: own(repo) }
}

#[test]
fn no_query_keeps_every_row() {
    assert_eq!(numbers(&narrow(list(), &q(None, None, None, None))), vec![10, 11, 12]);
}

#[test]
fn the_agent_filter_is_the_holder_and_ignores_case() {
    assert_eq!(numbers(&narrow(list(), &q(Some("Lazo"), None, None, None))), vec![10, 12]);
}

#[test]
fn the_repo_and_task_filters_narrow_together() {
    assert_eq!(numbers(&narrow(list(), &q(None, None, None, Some("AgentMuxAI/agentmux")))), vec![10, 11]);
    for t in ["2", "#2"] {
        assert_eq!(numbers(&narrow(list(), &q(None, None, Some(t), None))), vec![11], "task {t}");
    }
    assert_eq!(numbers(&narrow(list(), &q(Some("lazo"), None, None, Some("agentmuxai/agentmux")))), vec![10]);
}

#[test]
fn the_waits_on_filter_is_the_wire_word() {
    assert_eq!(numbers(&narrow(list(), &q(None, Some("reviewer"), None, None))), vec![10, 11, 12]);
    assert!(narrow(list(), &q(None, Some("person"), None, None)).rows.is_empty());
    assert!(narrow(list(), &q(None, Some("nonsense"), None, None)).rows.is_empty());
}

#[test]
fn a_blank_filter_is_no_filter() {
    assert_eq!(numbers(&narrow(list(), &q(Some(" "), Some(""), Some(" "), Some("")))), vec![10, 11, 12]);
}
