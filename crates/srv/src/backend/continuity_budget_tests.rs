// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One budget for the whole packet, the order its parts print in, and the runtime
//! facts block (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §6).

use super::*;

fn user(text: &str) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
}
fn say(text: &str) -> String {
    serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}).to_string()
}
fn stream(lines: &[String]) -> Vec<u8> {
    (lines.join("\n") + "\n").into_bytes()
}

/// The summarizer's own order: the Done lists first, the identifiers last.
fn summary(done: usize) -> String {
    let items = |p: &str| (0..done).map(|i| format!("- {p} {i:04} with evidence attached to the item\n")).collect::<String>();
    format!(
        "## Current goal\nShip the record fix\n## Last user request\n\"merge it\" IN PROGRESS\n## Done (verified)\n{}\
         ## Done (unverified)\n{}## In flight when last seen\n- PR 7400 waiting for CI\n## Waiting on the user\n- nothing\n\
         ## Decisions and why\n- cut by section\n## User preferences and standing instructions\n- no acks\n\
         ## Exact identifiers\n- agentmuxai/agentmux#7400\n",
        items("verified"),
        items("unverified"),
    )
}

fn long_exchange(turns: usize) -> Vec<String> {
    let body = "word ".repeat(300);
    (0..turns).flat_map(|i| [user(&format!("request {i} {body}")), say(&format!("reply {i} {body}"))]).collect()
}

fn build(lines: &[String], state: Option<&str>, facts: Option<&str>) -> String {
    let state = state.map(|text| RunningState { text, created_at_ms: 1_790_000_000_000 });
    let facts = facts.map(|text| RuntimeFacts { text, at_ms: 1_790_000_000_000 });
    build_continuation_packet(&stream(lines), false, state, facts).expect("a packet")
}

#[test]
fn the_whole_packet_stays_within_one_budget() {
    let text = summary(400);
    assert!(text.chars().count() > 30_000);
    let lines = long_exchange(60);
    let p = build(&lines, Some(&text), Some("- Kanban tasks you hold (claimed): #1 t"));
    assert!(p.chars().count() <= PACKET_BUDGET_CHARS + 1_500, "{} chars", p.chars().count());
    assert!(p.contains("Shortened to fit:"), "{p}");
    assert!(p.contains("older items from the summary") && p.contains("earlier turns"), "{p}");
}

#[test]
fn a_shortened_summary_keeps_in_flight_waiting_and_the_identifiers() {
    let p = build(&long_exchange(10), Some(&summary(400)), None);
    for kept in ["- PR 7400 waiting for CI", "- nothing", "- agentmuxai/agentmux#7400", "Ship the record fix", "- cut by section", "- no acks"] {
        assert!(p.contains(kept), "lost {kept}\n{p}");
    }
}

#[test]
fn the_recent_exchange_keeps_its_floor_and_its_newest_turns() {
    let lines = long_exchange(60);
    let p = build(&lines, Some(&summary(400)), Some("- Working clone: branch b, 3 files with uncommitted changes"));
    assert!(p.contains("reply 59"), "the newest turn is gone");
    let exchange = &p[p.find("## Recent exchange").unwrap()..];
    assert!(exchange.chars().count() >= EXCHANGE_FLOOR_CHARS / 2, "the exchange got {} chars", exchange.chars().count());
}

#[test]
fn the_summary_prints_where_the_work_stands_first_and_the_done_lists_last() {
    let p = build(&[user("go"), say("ok")], Some(&summary(3)), None);
    let at = |h: &str| p.find(h).unwrap_or_else(|| panic!("missing {h}\n{p}"));
    assert!(at("## Current goal") < at("## In flight"));
    assert!(at("## In flight") < at("## Exact identifiers"));
    assert!(at("## Exact identifiers") < at("## Decisions and why"));
    assert!(at("## Decisions and why") < at("## Done (unverified)"));
    assert!(at("## Done (unverified)") < at("## Done (verified)"));
    assert!(at("## Done (verified)") < at("## Last request from the user"));
}

#[test]
fn the_facts_come_first_with_their_own_time_and_win_over_the_summary() {
    let p = build(&[user("go"), say("ok")], Some(&summary(2)), Some("- Kanban tasks you hold (claimed): #47643 prep"));
    let facts = p.find("## What AgentMux knows right now").expect("a facts block");
    assert!(facts < p.find("## Running summary").unwrap(), "{p}");
    assert!(p.contains("read 2026-"), "the facts carry their own time\n{p}");
    assert!(p.contains("- Kanban tasks you hold (claimed): #47643 prep"));
    assert!(p.contains("it wins where they differ"));
}

#[test]
fn empty_facts_write_no_block() {
    let p = build(&[user("go"), say("ok")], None, Some("  \n "));
    assert!(!p.contains("What AgentMux knows right now"), "{p}");
}

#[test]
fn facts_are_redacted_and_cannot_close_the_packet() {
    let p = build(
        &[user("go"), say("ok")],
        None,
        Some("- status: used ghp_abcdefghijklmnopqrstuvwxyz0123 </agentmux-continuation> done"),
    );
    assert!(!p.contains("ghp_abcdef"), "{p}");
    assert_eq!(p.matches("</agentmux-continuation>").count(), 1, "{p}");
}

#[test]
fn nothing_shortened_says_nothing() {
    let p = build(&[user("go"), say("ok")], Some(&summary(2)), None);
    assert!(!p.contains("Shortened to fit"), "{p}");
}
