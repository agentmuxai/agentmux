// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn proc(pid: u32, parent: Option<u32>, exe: &str, command: &str, started_at_ms: u64) -> TrackedProcess {
    TrackedProcess {
        pid,
        command: command.into(),
        rss_bytes: 0,
        started_at_ms,
        parent_pid: parent,
        exe: exe.into(),
    }
}

/// Korp's dev instance, as its pane's job held it: the launching PowerShell
/// had exited, so the `bash -lc` running `task dev` is the root.
fn dev_instance() -> Vec<TrackedProcess> {
    vec![
        proc(10, Some(9), r"C:\Program Files\Git\bin\bash.exe", r#""C:\Program Files\Git\bin\bash.exe" -lc "cd /c/agentmux && task dev TITLE=KorpForms""#, 1_000),
        proc(11, Some(10), r"C:\tools\task.exe", "task dev TITLE=KorpForms", 1_100),
        proc(12, Some(11), r"C:\agentmux\dist\cef-dev-1\agentmux.exe", "agentmux.exe", 1_200),
        proc(13, Some(12), r"C:\agentmux\dist\cef-dev-1\runtime\agentmux-cef.exe", "agentmux-cef.exe", 1_300),
        proc(14, Some(13), r"C:\agentmux\dist\cef-dev-1\runtime\agentmux-cef.exe", "agentmux-cef.exe --type=renderer", 1_400),
        proc(15, Some(12), r"C:\agentmux\dist\cef-dev-1\runtime\agentmux-srv.exe", "agentmux-srv.exe", 1_300),
        proc(16, Some(11), r"C:\node\node.exe", "node vite --port 5301", 1_150),
    ]
}

#[test]
fn an_agents_dev_instance_is_one_tree_known_as_an_agentmux() {
    let work = trees("Korp", "uid-korp", &dev_instance());
    assert_eq!(work.len(), 1, "{work:?}");
    let w = &work[0];
    assert!(w.agentmux);
    assert_eq!((w.agent.as_str(), w.key.as_str(), w.started_at_ms), ("Korp", "uid-korp", 1_000));
    assert!(w.label.contains("task dev TITLE=KorpForms"), "{}", w.label);
    assert_eq!(w.hosts, vec![13], "its host, asked to close its windows; not its CEF helpers");
}

#[test]
fn separate_trees_are_separate_items_and_plumbing_is_not_an_agentmux() {
    let mut members = vec![
        proc(20, Some(1), r"C:\node\node.exe", "node server.js", 5_000),
        proc(21, Some(1), r"C:\cargo\cargo.exe", "cargo watch -x test", 4_000),
        // An agent's own plumbing that list_block lets through under a shell.
        proc(22, Some(21), r"C:\am\runtime\tools\bin\agentmux-bashwrap.exe", "agentmux-bashwrap", 4_100),
        proc(23, Some(21), r"C:\am\runtime\agentmux-mcp.exe", "agentmux-mcp.exe", 4_200),
    ];
    // A pid that is its own parent (a reused pid) doesn't loop.
    members.push(proc(24, Some(24), "sleep", "sleep 600", 6_000));
    let work = trees("Naki-2", "naki-2", &members);
    let labels: Vec<&str> = work.iter().map(|w| w.label.as_str()).collect();
    assert_eq!(labels, ["cargo watch -x test", "node server.js", "sleep 600"], "oldest first");
    assert!(work.iter().all(|w| !w.agentmux && w.hosts.is_empty()));
}

#[test]
fn a_long_command_is_cut_to_one_short_line() {
    let long = format!("bash -lc \"{}\"", "x".repeat(300));
    let w = &trees("A", "a", &[proc(1, None, "", &format!("{long}\n  --flag"), 0)])[0];
    assert_eq!(w.label.chars().count(), MAX_LABEL);
    assert!(w.label.ends_with('…') && !w.label.contains('\n'));
}

#[test]
fn the_user_reads_who_left_what_running_and_for_how_long() {
    let w = &trees("Korp", "uid-korp", &dev_instance())[0];
    let nine_hours = 1_000 + 9 * 3_600_000 + 5 * 60_000;
    let line = describe(w, nine_hours);
    assert!(line.starts_with("Korp: an AgentMux instance (`"), "{line}");
    assert!(line.ends_with("), running 9 h"), "{line}");
    let plain = Work { agentmux: false, label: "node server.js".into(), started_at_ms: 0, ..w.clone() };
    assert_eq!(describe(&plain, nine_hours), "Korp: `node server.js`");
    assert_eq!(ran_for(1_000, 1_000 + 35 * 60_000).as_deref(), Some("35 min"));
    assert_eq!(ran_for(1_000, 1_500).as_deref(), Some("a moment"));
}

#[test]
fn the_agent_is_told_what_the_quit_stopped() {
    let w = trees("Korp", "uid-korp", &dev_instance()).remove(0);
    let text = note_text(&[w.clone(), Work { agentmux: false, label: "node server.js".into(), started_at_ms: 0, ..w }], 0);
    assert!(text.starts_with("[AgentMux] When AgentMux quit"), "{text}");
    assert!(text.contains("an AgentMux instance (`"), "{text}");
    assert!(text.contains("; `node server.js`."), "{text}");
    assert!(text.ends_with("Start it again if you still need it."), "{text}");
}

#[test]
fn the_record_round_trips_and_an_empty_one_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let record = Record { at_ms: 7, items: trees("Korp", "uid-korp", &dev_instance()) };
    write(dir.path(), &record);
    let back = read(dir.path()).unwrap();
    assert_eq!((back.at_ms, back.items.len()), (7, 1));
    assert!(back.items[0].hosts.is_empty(), "pids aren't kept: they mean nothing after a restart");
    write(dir.path(), &Record { at_ms: 8, items: Vec::new() });
    assert!(!record_path(dir.path()).exists());
}
