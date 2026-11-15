// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const NOW: i64 = 1_790_000_000_000;

fn full() -> Facts {
    Facts {
        tasks: vec![
            HeldTask { reference: "#47659".into(), title: "Agent pane with a huge history draws blank".into() },
            HeldTask { reference: "#47643".into(), title: "Build: prep before a session restart".into() },
        ],
        background: vec![
            Background { label: "cargo test -p agentmux-srv".into(), started_at_ms: NOW - 12 * 60_000, alive: Some(true) },
            Background { label: "vite".into(), started_at_ms: NOW - 40_000, alive: Some(false) },
        ],
        line: Some(StatusLine { text: "Waiting: agenty-2 on the shared prefix".into(), state: Some("waiting".into()), set_at_ms: (NOW - 5 * 60_000) as u64 }),
        repo: Some(WorkingClone { branch: Some("agent5/continuity-record-sections".into()), dirty_files: 7, checked_ms: Some((NOW - 15_000) as u64) }),
    }
}

#[test]
fn every_fact_is_written_with_its_age() {
    let out = render(&full(), NOW);
    assert!(out.contains("Kanban tasks you hold (claimed): #47659 Agent pane with a huge history draws blank; #47643 Build"), "{out}");
    assert!(out.contains("cargo test -p agentmux-srv (running 12 min, process alive)"), "{out}");
    assert!(out.contains("vite (running 40 s, process gone)"), "{out}");
    assert!(out.contains("Your status line [waiting], set 5 min ago: Waiting: agenty-2 on the shared prefix"), "{out}");
    assert!(out.contains("Working clone: branch agent5/continuity-record-sections, 7 files with uncommitted changes (checked 15 s ago)"), "{out}");
}

#[test]
fn nothing_known_writes_nothing() {
    assert_eq!(render(&Facts::default(), NOW), "");
}

#[test]
fn only_what_is_known_is_written() {
    let facts = Facts { tasks: vec![HeldTask { reference: "#1".into(), title: "t".into() }], ..Facts::default() };
    let out = render(&facts, NOW);
    assert_eq!(out, "- Kanban tasks you hold (claimed): #1 t");
}

#[test]
fn a_long_list_is_counted_not_listed() {
    let tasks = (0..8).map(|i| HeldTask { reference: format!("#{i}"), title: "t".into() }).collect();
    let out = render(&Facts { tasks, ..Facts::default() }, NOW);
    assert!(out.contains("#4 t; and 3 more"), "{out}");
    assert!(!out.contains("#5 t"));
}

#[test]
fn a_long_or_multiline_text_is_cut_to_one_line() {
    let facts = Facts {
        line: Some(StatusLine { text: format!("first\nsecond {}", "x".repeat(400)), state: None, set_at_ms: 0 }),
        ..Facts::default()
    };
    let out = render(&facts, NOW);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.starts_with("- Your status line: first second "));
    assert!(out.ends_with('…'));
    assert!(out.chars().count() < 200);
}

#[test]
fn a_process_with_no_pid_says_so() {
    let facts = Facts { background: vec![Background { label: "job".into(), started_at_ms: NOW - 3 * 3_600_000, alive: None }], ..Facts::default() };
    assert!(render(&facts, NOW).contains("job (running 3 h, no pid)"));
}

#[test]
fn an_agent_with_no_registration_has_no_facts() {
    let store = Store::open_in_memory().expect("an in-memory store");
    let facts = collect(&store, None, "no-such-block");
    assert!(facts.tasks.is_empty() && facts.background.is_empty() && facts.line.is_none() && facts.repo.is_none());
}
