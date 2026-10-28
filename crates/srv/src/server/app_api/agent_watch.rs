// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents watch` (`POST /api/v1/agent/watch`): replay the recent transcript of an agent, then tail it,
//! for a leader that supervises a worker without opening a pane
//! (docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md §5.1). It reads the same output file a pane
//! reads, by the agent's block, so an agent with no pane (parked, or stopped) can be watched too.
//!
//! A call returns whole lines and `next`, a byte position. Pass `next` back as `after` to get what was
//! written since; with `wait_seconds` the call waits for a new line instead of answering empty. Without
//! `after` it is the replay: the last `max_lines` lines. The position is a byte offset into the file, so
//! a line being written is not returned until it is complete, and a read costs the new bytes, not the
//! whole transcript. If the file shrank (a fresh session) the answer says `reset` and replays.
//!
//! Who may watch: the same as `transcript` for an agent on this host, any agent on this machine; the
//! call is not an action on the watched agent and is not audited.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::agent_lifecycle::{actor_of, refuse, target_of, Refusal};
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The zone's file the transcript is.
const OUTPUT_FILE: &str = "output";
/// The most a replay reads back from the end of the file.
const REPLAY_BYTES: i64 = 256 * 1024;
/// The most one tail read takes.
const CHUNK_BYTES: i64 = 512 * 1024;
const DEFAULT_LINES: usize = 100;
const MAX_LINES: usize = 500;
/// The longest a line is shown; a tool result can be much larger.
const MAX_LINE_CHARS: usize = 8 * 1024;
/// The longest a call waits for a new line.
const MAX_WAIT_SECONDS: u64 = 25;
const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug, Deserialize)]
pub(crate) struct WatchRequest {
    agent: String,
    #[serde(default)]
    max_lines: Option<usize>,
    /// The `next` of the last answer: only lines after it.
    #[serde(default)]
    after: Option<u64>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

/// Whole lines read from a window of the file, and where the next read starts.
#[derive(Debug, PartialEq)]
struct Lines {
    lines: Vec<String>,
    /// Absolute byte position after the last line returned.
    next: u64,
    /// Older lines were left out (a replay) or newer ones are still to read (a tail).
    more: bool,
}

fn shown(line: &str) -> String {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((cut, _)) => format!("{}…[{} more characters]", &line[..cut], line[cut..].chars().count()),
        None => line.to_string(),
    }
}

/// The whole lines in `data`, which starts at byte `from` of the file. `mid_line`: `from` may fall inside
/// a line, so the first piece is dropped. `from_end`: keep the last `max` lines (a replay) rather than the
/// first (a tail). A last line with no newline yet is not returned and `next` stays before it.
fn lines_of(data: &[u8], from: u64, mid_line: bool, from_end: bool, max: usize) -> Lines {
    let mut start = 0;
    if mid_line {
        match data.iter().position(|&b| b == b'\n') {
            Some(i) => start = i + 1,
            None => return Lines { lines: Vec::new(), next: from, more: false },
        }
    }
    let Some(end) = data.iter().rposition(|&b| b == b'\n').map(|i| i + 1).filter(|&e| e > start) else {
        return Lines { lines: Vec::new(), next: from + start as u64, more: false };
    };
    // (end offset in `data`, text) of each non-blank line.
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut at = start;
    for piece in data[start..end].split_inclusive(|&b| b == b'\n') {
        at += piece.len();
        let text = String::from_utf8_lossy(piece);
        let text = text.trim();
        if !text.is_empty() {
            found.push((at, shown(text)));
        }
    }
    let more_than_max = found.len() > max;
    let (kept, next_at): (Vec<(usize, String)>, usize) = if !more_than_max {
        (found, end)
    } else if from_end {
        let skip = found.len() - max;
        (found.split_off(skip), end)
    } else {
        found.truncate(max);
        let next_at = found.last().map_or(start, |(e, _)| *e);
        (found, next_at)
    };
    Lines { lines: kept.into_iter().map(|(_, t)| t).collect(), next: from + next_at as u64, more: more_than_max }
}

/// What one read of the file gave.
struct Window {
    /// The file's size now.
    size: u64,
    /// Where `data` starts.
    from: u64,
    data: Vec<u8>,
    /// The cursor asked for bytes the file no longer holds, or is past its end.
    reset: bool,
}

/// Read the new bytes after `after`, or the last [`REPLAY_BYTES`] of the file.
fn read_window(state: &AppState, block_id: &str, after: Option<u64>) -> Result<Window, String> {
    let archived = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .and_then(|b| b.meta.get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT).and_then(|v| v.as_i64()))
        .is_some_and(|at| at > 0);
    let (size, whole): (i64, Option<Vec<u8>>) = if archived {
        let (bytes, _) = crate::backend::session_archive::read_session_output(&state.mstore, &state.filestore, block_id)?;
        (bytes.len() as i64, Some(bytes))
    } else {
        match state.filestore.stat(block_id, OUTPUT_FILE).map_err(|e| format!("filestore stat: {e}"))? {
            Some(f) => (f.size, None),
            None => return Ok(Window { size: 0, from: 0, data: Vec::new(), reset: after.is_some_and(|a| a > 0) }),
        }
    };
    let (want_from, reset) = match after {
        Some(a) if (a as i64) <= size => (a as i64, false),
        Some(_) => ((size - REPLAY_BYTES).max(0), true),
        None => ((size - REPLAY_BYTES).max(0), false),
    };
    let want_len = if after.is_some() && !reset { (size - want_from).min(CHUNK_BYTES) } else { size - want_from };
    let (from, data) = match whole {
        Some(bytes) => {
            let end = (want_from + want_len) as usize;
            (want_from, bytes[want_from as usize..end.min(bytes.len())].to_vec())
        }
        None if want_len <= 0 => (want_from, Vec::new()),
        None => state.filestore.read_at(block_id, OUTPUT_FILE, want_from, want_len).map_err(|e| format!("filestore read: {e}"))?,
    };
    // The window start moved up: a circular file dropped what the cursor pointed at.
    let reset = reset || after.is_some_and(|a| from > a as i64);
    Ok(Window { size: size as u64, from: from as u64, data, reset })
}

async fn watch(state: &AppState, req: &WatchRequest) -> Result<Value, Refusal> {
    let blocking = state.clone();
    let named = req.agent.clone();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents watch: {e}")))??;
    let Some(block_id) = target.block_id.clone() else {
        return Ok(json!({
            "agent": target.name, "agent_id": target.agent_id, "lines": [], "next": 0, "more": false,
            "runtime": { "state": target.state, "desired": target.desired },
            "note": "it has never run: there is no transcript yet; Agents start runs it",
        }));
    };
    let max = req.max_lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(req.wait_seconds.unwrap_or(0).min(MAX_WAIT_SECONDS));
    loop {
        let (reader, block) = (state.clone(), block_id.clone());
        let after = req.after;
        let window = tokio::task::spawn_blocking(move || read_window(&reader, &block, after))
            .await
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents watch: {e}")))?
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents watch {}: {e}", target.name)))?;
        // A replay starts mid-line unless it starts the file; a tail starts where a line ended.
        let replay = req.after.is_none() || window.reset;
        let read = lines_of(&window.data, window.from, replay && window.from > 0, replay, max);
        if !read.lines.is_empty() || read.more || window.reset || tokio::time::Instant::now() >= deadline || req.after.is_none() {
            let turn_active = crate::backend::blockcontroller::get_block_controller_status(&block_id).is_some_and(|s| s.turn_active);
            let now = agentmux_common::time::now_ms_u64();
            let runtime = crate::server::agent_runtime::list(state, now).into_iter().find(|r| r.agent_id == target.agent_id);
            return Ok(json!({
                "agent": target.name, "agent_id": target.agent_id, "block_id": block_id,
                "lines": read.lines, "next": read.next, "more": read.more, "reset": window.reset,
                "turn_active": turn_active,
                "runtime": runtime.map(|r| json!({ "state": r.state, "desired": r.desired })),
            }));
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

pub(crate) async fn handle_agent_watch(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<WatchRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let result = async {
        actor_of(&state, caller.as_ref(), "watch")?;
        watch(&state, &req).await
    }
    .await;
    match result {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

#[cfg(test)]
#[path = "tests/agent_watch_tests.rs"]
mod tests;
