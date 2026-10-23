// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How full a Codex thread's context is, for the pane's context meter.
//!
//! `codex exec --json` reports only the thread's running totals: three
//! one-word turns read `input_tokens` 14,295, 28,608 and 42,939 (codex-cli
//! 0.154.0), so its `turn.completed` is never a context size. Codex's own
//! session file (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<thread>.jsonl`,
//! the local date the thread started) records each model call's usage:
//!
//! ```text
//! {"type":"event_msg","payload":{"type":"token_count","info":{
//!   "total_token_usage":{"input_tokens":42939,…},
//!   "last_token_usage":{"input_tokens":14331,"cached_input_tokens":14080,…},
//!   "model_context_window":258400}}}
//! {"type":"turn_context","payload":{…,"model":"gpt-6-astra",…}}
//! ```
//!
//! `last_token_usage.input_tokens` is the last call's whole prompt (cached
//! included), and `model_context_window` the window Codex compacts against.
//! After each host turn srv reads them and writes an `agentmux_context` frame
//! into the pane's stream, which the frontend reads for any provider
//! (frontend/app/view/agent/context-reader.ts). A container pane's session file
//! is inside its container, so it gets none.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{Duration, Local, TimeZone};
use serde_json::{json, Value};

/// How much of a session file's end is read: its last turn's `token_count`
/// and `turn_context`. A long thread's file runs to megabytes.
const TAIL_BYTES: u64 = 1024 * 1024;

/// The last model call's context, from a Codex session file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodexContext {
    pub tokens: u64,
    pub window: Option<u64>,
    pub model: Option<String>,
}

/// The `CODEX_HOME` a turn runs with: the pane's own (its account), else
/// srv's, else Codex's default `~/.codex`.
pub(super) fn codex_home(env_vars: &std::collections::HashMap<String, String>) -> Option<PathBuf> {
    env_vars
        .get("CODEX_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()).map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

/// The thread's context after its last turn, or None when its session file
/// can't be found or records none.
pub(super) fn read(codex_home: &Path, thread_id: &str) -> Option<CodexContext> {
    last_context(&session_file(codex_home, thread_id)?)
}

/// The `agentmux_context` frame for `ctx`.
pub(super) fn frame(ctx: &CodexContext) -> Value {
    json!({ "type": "agentmux_context", "tokens": ctx.tokens, "window": ctx.window, "model": ctx.model })
}

/// The thread's session file. A thread id is a UUIDv7, whose first 48 bits
/// are when it started (unix ms), and Codex files it under that local date;
/// the days either side are tried too, for a clock or time zone that moved.
fn session_file(codex_home: &Path, thread_id: &str) -> Option<PathBuf> {
    let hex: String = thread_id.chars().filter(|c| *c != '-').take(12).collect();
    let started_ms = i64::from_str_radix(&hex, 16).ok().filter(|_| hex.len() == 12)?;
    let started = Local.timestamp_millis_opt(started_ms).single()?;
    let suffix = format!("-{thread_id}.jsonl");
    [0, -1, 1].into_iter().find_map(|days| {
        let day = started + Duration::days(days);
        let dir = codex_home.join("sessions").join(day.format("%Y").to_string()).join(day.format("%m").to_string()).join(day.format("%d").to_string());
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
        })
    })
}

/// The last `token_count` in the file's tail, with the model of the last
/// `turn_context` there (None when the tail holds none).
fn last_context(path: &Path) -> Option<CodexContext> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);
    // The first line may be cut off by the seek; it fails to parse and is skipped.
    let mut usage: Option<(u64, Option<u64>)> = None;
    let mut model: Option<String> = None;
    for line in text.lines().rev() {
        if usage.is_some() && model.is_some() {
            break;
        }
        // Cheap checks first: most lines are messages and tool output.
        let is_count = usage.is_none() && line.contains("\"token_count\"");
        let is_context = model.is_none() && line.contains("\"turn_context\"");
        if !is_count && !is_context {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if is_count && v.pointer("/payload/type").and_then(Value::as_str) == Some("token_count") {
            let info = v.pointer("/payload/info");
            let tokens = info.and_then(|i| i.pointer("/last_token_usage/input_tokens")).and_then(Value::as_u64).filter(|t| *t > 0);
            if let Some(tokens) = tokens {
                let window = info.and_then(|i| i.get("model_context_window")).and_then(Value::as_u64).filter(|w| *w > 0);
                usage = Some((tokens, window));
            }
        }
        if is_context && v.get("type").and_then(Value::as_str) == Some("turn_context") {
            model = v.pointer("/payload/model").and_then(Value::as_str).filter(|m| !m.is_empty()).map(str::to_string);
        }
    }
    let (tokens, window) = usage?;
    Some(CodexContext { tokens, window, model })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines shaped like codex-cli 0.154.0's (measured 2026-10-22): a thread
    /// of three one-word turns, the last call 14,331 tokens of 258,400.
    fn session_lines() -> Vec<String> {
        let count = |total: u64, last: u64| {
            json!({ "timestamp": "2026-10-23T01:15:23.217Z", "type": "event_msg", "payload": { "type": "token_count", "info": {
                "total_token_usage": { "input_tokens": total, "cached_input_tokens": total - 2_000, "output_tokens": 5 },
                "last_token_usage": { "input_tokens": last, "cached_input_tokens": last - 251, "output_tokens": 5 },
                "model_context_window": 258_400 } } })
            .to_string()
        };
        let context = json!({ "type": "turn_context", "payload": { "cwd": "/w", "model": "gpt-6-astra", "approval_policy": "never" } }).to_string();
        let message = json!({ "type": "response_item", "payload": { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "token_count is in this reply" }] } }).to_string();
        vec![context.clone(), message.clone(), count(14_295, 14_295), context, message.clone(), count(28_608, 14_313), count(42_939, 14_331), message]
    }

    fn write_session(home: &Path, thread_id: &str, lines: &[String]) -> PathBuf {
        let hex: String = thread_id.chars().filter(|c| *c != '-').take(12).collect();
        let day = Local.timestamp_millis_opt(i64::from_str_radix(&hex, 16).unwrap()).single().unwrap();
        let dir = home.join("sessions").join(day.format("%Y/%m/%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{}-{thread_id}.jsonl", day.format("%Y-%m-%dT%H-%M-%S")));
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    const THREAD: &str = "01a16653-6ccb-7842-8746-7619c0f55215";

    #[test]
    fn reads_the_last_calls_context_never_the_threads_total() {
        let home = tempfile::tempdir().unwrap();
        write_session(home.path(), THREAD, &session_lines());
        let ctx = read(home.path(), THREAD).expect("a reading");
        assert_eq!(ctx, CodexContext { tokens: 14_331, window: Some(258_400), model: Some("gpt-6-astra".into()) });
        assert_eq!(frame(&ctx), json!({ "type": "agentmux_context", "tokens": 14_331, "window": 258_400, "model": "gpt-6-astra" }));
    }

    #[test]
    fn finds_the_file_from_the_date_in_the_thread_id() {
        let home = tempfile::tempdir().unwrap();
        let path = write_session(home.path(), THREAD, &session_lines());
        assert_eq!(session_file(home.path(), THREAD), Some(path));
        assert_eq!(session_file(home.path(), "01a16653-6ccb-7842-8746-000000000000"), None, "another thread of the same day");
        assert_eq!(session_file(home.path(), "not-a-uuid"), None);
    }

    #[test]
    fn a_tail_past_the_turn_context_still_has_the_tokens() {
        let home = tempfile::tempdir().unwrap();
        let mut lines = session_lines();
        lines.retain(|l| !l.contains("turn_context"));
        write_session(home.path(), THREAD, &lines);
        assert_eq!(read(home.path(), THREAD).map(|c| (c.tokens, c.model)), Some((14_331, None)));
    }

    #[test]
    fn none_without_a_session_file_or_a_count() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(read(home.path(), THREAD), None);
        let lines: Vec<String> = session_lines().into_iter().filter(|l| !l.contains("\"token_count\"")).collect();
        write_session(home.path(), THREAD, &lines);
        assert_eq!(read(home.path(), THREAD), None);
    }

    #[test]
    fn the_panes_own_codex_home_comes_first() {
        let env = std::collections::HashMap::from([("CODEX_HOME".to_string(), "/accounts/a/codex".to_string())]);
        assert_eq!(codex_home(&env), Some(PathBuf::from("/accounts/a/codex")));
    }
}
