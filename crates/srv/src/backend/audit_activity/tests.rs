// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::line::parse;
use super::scrub::{looks_secret, summarize};
use super::store::{day_files, day_of, prune, spawn_writer};
use super::*;

// ── the summaries

#[test]
fn a_command_is_cut_to_its_start_and_a_path_is_the_path() {
    assert_eq!(summarize("Bash", &json!({"command": "cargo test -p agentmux-srv"})), "Bash cargo test -p agentmux-srv");
    let long = format!("echo {}", "word ".repeat(40));
    let got = summarize("Bash", &json!({"command": long}));
    assert!(got.ends_with('…') && got.chars().count() <= "Bash ".len() + 81, "{got}");
    assert_eq!(summarize("Bash", &json!({"command": "ls\nrm -rf x"})), "Bash ls", "only the first line");
    assert_eq!(summarize("Read", &json!({"file_path": "/work/a/b.rs", "offset": 10})), "Read /work/a/b.rs");
    assert_eq!(summarize("Edit", &json!({"file_path": "/work/a/b.rs", "old_string": "secret text", "new_string": "x"})), "Edit /work/a/b.rs");
}

#[test]
fn a_command_that_holds_a_secret_is_cut_to_its_tool_name() {
    for command in [
        "curl -H 'Authorization: Bearer abc' https://x.test",
        "export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "aws configure set aws_secret_access_key wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "psql postgres://user:hunter2@host/db --password hunter2",
        "echo AKIAIOSFODNN7EXAMPLE",
        "deploy --token sk-live-1234567890abcdef",
        "echo 3f786850e387550fdab836ed7e6dc881de23001b0a1f9cd0",
    ] {
        assert_eq!(summarize("Bash", &json!({ "command": command })), "Bash", "{command}");
    }
}

#[test]
fn ordinary_commands_and_ids_are_not_taken_for_secrets() {
    for ok in [
        "git status",
        "cargo test --lib backend::audit_query",
        "ls /home/dev/.claude/projects/bb87e687-6ea3-430a-b9fa-7f83435a2926/memory",
        "grep -rn needle crates",
        "npx vitest run frontend/app/store/domain-store.test.ts",
    ] {
        assert!(!looks_secret(ok), "{ok}");
    }
}

#[test]
fn muxapi_tools_give_their_action_and_a_messages_target_never_its_text() {
    let send = json!({"action": "send", "to": "agenty", "message": "the password is hunter2"});
    assert_eq!(summarize("mcp__agentmux__Message", &send), "Message send to agenty");
    assert_eq!(summarize("mcp__agentmux__Vault", &json!({"action": "reveal", "id": "abc", "part": "value"})), "Vault reveal");
    assert_eq!(summarize("mcp__agentmux__Tasks", &json!({"action": "claim", "id": "12"})), "Tasks claim");
    assert_eq!(summarize("SomethingNew", &json!({"anything": "x"})), "SomethingNew");
    assert_eq!(summarize("WebFetch", &json!({"url": "https://x.test/a?token=abc#frag"})), "WebFetch https://x.test/a");
}

// ── the lines

#[test]
fn an_assistant_line_gives_a_row_per_tool_call_and_thought_and_none_for_text() {
    let line = json!({"type": "assistant", "message": {"content": [
        {"type": "thinking", "thinking": "I should delete the production database"},
        {"type": "text", "text": "Running the tests"},
        {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cargo test"}},
        {"type": "tool_use", "id": "t2", "name": "Read", "input": {"file_path": "/a.rs"}},
    ]}})
    .to_string();
    let rows = parse(&line);
    assert_eq!(rows.iter().map(|r| (r.kind, r.summary.as_str())).collect::<Vec<_>>(), vec![("thought", "thinking"), ("tool", "Bash cargo test"), ("tool", "Read /a.rs")]);
    assert!(rows.iter().all(|r| !r.summary.contains("production")), "a thought's text is never kept");
    assert_eq!(rows[1].tool.as_deref(), Some("Bash"));
}

#[test]
fn a_result_line_ends_a_turn_with_its_outcome_length_and_measured_cost() {
    let line = json!({"type": "result", "subtype": "success", "is_error": false, "duration_ms": 12_345, "total_cost_usd": 0.25,
        "usage": {"input_tokens": 100, "cache_creation_input_tokens": 20, "cache_read_input_tokens": 5000, "output_tokens": 300}})
    .to_string();
    let row = parse(&line).remove(0);
    assert_eq!((row.kind, row.outcome.as_deref(), row.ms), ("turn", Some("success"), Some(12_345)));
    assert_eq!(row.summary, "turn ended (success) in 12.3 s");
    let cost = row.cost.unwrap();
    assert_eq!((cost.input, cost.cache_read, cost.output, cost.usd, cost.basis.as_str()), (120, 5000, 300, Some(0.25), "measured"));

    let failed = parse(&json!({"type": "result", "subtype": "error_max_turns", "is_error": true}).to_string()).remove(0);
    assert_eq!(failed.outcome.as_deref(), Some("error_max_turns"));
}

#[test]
fn other_lines_give_nothing() {
    for line in [
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "tool_use output"}]}}).to_string(),
        json!({"type": "system", "subtype": "init"}).to_string(),
        json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "hello"}]}}).to_string(),
        "not json at all tool_use".to_string(),
        String::new(),
    ] {
        assert!(parse(&line).is_empty(), "{line}");
    }
}

#[test]
fn rows_carry_the_byte_each_line_landed_at_and_only_a_defined_agents_zone_is_indexed() {
    let a = json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "ls"}}]}}).to_string();
    let b = json!({"type": "result", "subtype": "success"}).to_string();
    let records = format!("{{\"type\":\"system\"}}\n{a}\n{b}\n");
    let rows = rows_of("block-1", "agent:uid-1:current", records.as_bytes(), Some(1000), 77);
    assert_eq!(rows.len(), 2);
    let first_line = "{\"type\":\"system\"}\n".len() as u64;
    assert_eq!(rows[0].byte, Some(1000 + first_line));
    assert_eq!(rows[1].byte, Some(1000 + first_line + a.len() as u64 + 1));
    assert_eq!((rows[0].agent.as_str(), rows[0].block.as_str(), rows[0].zone.as_str(), rows[0].ts), ("uid-1", "block-1", "agent:uid-1:current", 77));
    // A quick-launch provider's zone is shared by every pane of the provider: no one agent made what is in it.
    assert!(rows_of("block-1", "agent:claude:current", records.as_bytes(), Some(0), 77).is_empty());
    assert!(rows_of("block-1", "block-1", records.as_bytes(), Some(0), 77).is_empty());
}

// ── the day files

fn row(ts: u64) -> ActivityRow {
    ActivityRow { ts, kind: "tool".into(), agent: "uid-1".into(), block: "b".into(), zone: "z".into(), byte: Some(0), summary: "Bash ls".into(), tool: Some("Bash".into()), outcome: None, ms: None, cost: None, channel: None }
}

#[test]
fn each_row_goes_to_the_file_of_its_own_day_stamped_with_its_channel() {
    let dir = tempfile::tempdir().unwrap();
    // Recent, or the writer's own retention would delete the files as it opens them.
    let base = agentmux_common::time::now_ms_u64() - 5 * 86_400_000;
    let day = |d: u64| base + d * 86_400_000;
    let (tx, writer) = spawn_writer(dir.path().to_path_buf(), "dev-a".into()).unwrap();
    for ts in [day(0), day(0) + 5, day(1), day(0) + 9] {
        tx.send(row(ts)).unwrap();
    }
    drop(tx);
    writer.join().unwrap();
    let files = day_files(dir.path());
    assert_eq!(files.iter().map(|f| f.day.clone()).collect::<Vec<_>>(), vec![day_of(day(1)), day_of(day(0))], "newest day first");
    let text = std::fs::read_to_string(&files[1].path).unwrap();
    let rows: Vec<ActivityRow> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.channel.as_deref() == Some("dev-a") && day_of(r.ts) == files[1].day));
}

#[test]
fn a_day_older_than_the_retention_is_deleted_and_the_rest_kept() {
    let dir = tempfile::tempdir().unwrap();
    for day in ["2026-09-01", "2026-10-04", "2026-10-06", "2026-11-04"] {
        std::fs::write(dir.path().join(format!("activity-{day}.jsonl")), "{}\n").unwrap();
    }
    std::fs::write(dir.path().join("activity-notes.jsonl"), "x").unwrap();
    std::fs::write(dir.path().join("jekt-audit.jsonl"), "x").unwrap();
    // Thirty days before 2026-11-05 is 2026-10-06: that day is kept, the one before it is not.
    assert_eq!(prune(dir.path(), "2026-11-05"), 2);
    let left: Vec<String> = day_files(dir.path()).into_iter().map(|f| f.day).collect();
    assert_eq!(left, vec!["2026-11-04", "2026-10-06"]);
    assert!(dir.path().join("jekt-audit.jsonl").exists() && dir.path().join("activity-notes.jsonl").exists(), "only day files go");
}
