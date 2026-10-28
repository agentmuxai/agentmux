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
/// The most searched for the end of one line before it is given up on.
const MAX_SCAN_BYTES: i64 = if cfg!(test) { 1024 * 1024 } else { 16 * 1024 * 1024 };
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

/// The transcript, whole in memory (an archived session) or read in spans from the file store.
struct Source<'a> {
    state: &'a AppState,
    block_id: &'a str,
    whole: Option<Vec<u8>>,
    size: i64,
}

impl Source<'_> {
    fn open<'a>(state: &'a AppState, block_id: &'a str) -> Result<Option<Source<'a>>, String> {
        let archived = state
            .mstore
            .get::<crate::backend::obj::Block>(block_id)
            .ok()
            .flatten()
            .and_then(|b| b.meta.get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT).and_then(|v| v.as_i64()))
            .is_some_and(|at| at > 0);
        if archived {
            let (bytes, _) = crate::backend::session_archive::read_session_output(&state.mstore, &state.filestore, block_id)?;
            return Ok(Some(Source { state, block_id, size: bytes.len() as i64, whole: Some(bytes) }));
        }
        let stat = state.filestore.stat(block_id, OUTPUT_FILE).map_err(|e| format!("filestore stat: {e}"))?;
        Ok(stat.map(|f| Source { state, block_id, whole: None, size: f.size }))
    }

    /// `len` bytes from `from`, and where they really start (a circular file may have dropped the front).
    fn span(&self, from: i64, len: i64) -> Result<(i64, Vec<u8>), String> {
        match &self.whole {
            Some(bytes) => {
                let end = ((from + len) as usize).min(bytes.len());
                Ok((from, bytes[(from as usize).min(end)..end].to_vec()))
            }
            None if len <= 0 => Ok((from, Vec::new())),
            None => self.state.filestore.read_at(self.block_id, OUTPUT_FILE, from, len).map_err(|e| format!("filestore read: {e}")),
        }
    }

    /// The position of the first line break at or after `from`, reading on in spans.
    fn find_newline(&self, mut from: i64) -> Result<Option<i64>, String> {
        while from < self.size {
            let (at, data) = self.span(from, CHUNK_BYTES)?;
            if let Some(i) = data.iter().position(|&b| b == b'\n') {
                return Ok(Some(at + i as i64));
            }
            if data.is_empty() {
                break;
            }
            from = at + data.len() as i64;
        }
        Ok(None)
    }
}

/// What one read of the transcript gave.
struct Read {
    lines: Lines,
    /// The cursor asked for bytes the file no longer holds, or is past its end: this is a replay.
    reset: bool,
    /// A session that ended: its file never grows, so waiting for a line is pointless.
    archived: bool,
}

/// The lines after `after`, or the tail of the transcript when `after` is `None` or no longer valid.
/// A line longer than a read is searched for its end, up to [`MAX_SCAN_BYTES`], and a longer one is
/// skipped with a marker line, so a cursor never stops in front of a line or lands inside one.
fn read_lines(state: &AppState, block_id: &str, after: Option<u64>, max: usize) -> Result<Read, String> {
    let Some(src) = Source::open(state, block_id)? else {
        let lines = Lines { lines: Vec::new(), next: 0, more: false };
        return Ok(Read { lines, reset: after.is_some_and(|a| a > 0), archived: false });
    };
    let archived = src.whole.is_some();
    let size = src.size;
    let mut reset = false;
    if let Some(after) = after.map(|a| a as i64) {
        if after <= size {
            let mut len = (size - after).min(CHUNK_BYTES);
            loop {
                let (at, data) = src.span(after, len)?;
                if at > after {
                    // A circular file dropped what the cursor pointed at.
                    reset = true;
                    break;
                }
                let lines = lines_of(&data, at as u64, false, false, max);
                if !lines.lines.is_empty() || lines.next > after as u64 || after + len >= size {
                    return Ok(Read { lines, reset: false, archived });
                }
                if len >= MAX_SCAN_BYTES {
                    let lines = match src.find_newline(after + len)? {
                        Some(end) => Lines {
                            lines: vec![format!("[a line of {} bytes is too long to show and was skipped]", end + 1 - after)],
                            next: (end + 1) as u64,
                            more: true,
                        },
                        // Still being written: nothing to move past yet.
                        None => Lines { lines: Vec::new(), next: after as u64, more: false },
                    };
                    return Ok(Read { lines, reset: false, archived });
                }
                len = (len * 2).min(size - after);
            }
        } else {
            reset = true;
        }
    }
    // The replay: the last lines, from a window that grows until it holds a whole line.
    let mut window = REPLAY_BYTES;
    loop {
        let start = (size - window).max(0);
        let (at, data) = src.span(start, size - start)?;
        let lines = lines_of(&data, at as u64, at > 0, true, max);
        if !lines.lines.is_empty() || at == 0 {
            return Ok(Read { lines, reset, archived });
        }
        if window >= MAX_SCAN_BYTES {
            let skipped = Lines {
                lines: vec![format!("[no line break in the last {} bytes: older output was skipped]", size - at)],
                next: size as u64,
                more: true,
            };
            return Ok(Read { lines: skipped, reset, archived });
        }
        window *= 2;
    }
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
        let (after, max_lines) = (req.after, max);
        let read = tokio::task::spawn_blocking(move || read_lines(&reader, &block, after, max_lines))
            .await
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents watch: {e}")))?
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents watch {}: {e}", target.name)))?;
        if !read.lines.lines.is_empty()
            || read.lines.more
            || read.reset
            || read.archived
            || tokio::time::Instant::now() >= deadline
            || req.after.is_none()
        {
            let turn_active = crate::backend::blockcontroller::get_block_controller_status(&block_id).is_some_and(|s| s.turn_active);
            let now = agentmux_common::time::now_ms_u64();
            let runtime = crate::server::agent_runtime::list(state, now).into_iter().find(|r| r.agent_id == target.agent_id);
            return Ok(json!({
                "agent": target.name, "agent_id": target.agent_id, "block_id": block_id,
                "lines": read.lines.lines, "next": read.lines.next, "more": read.lines.more, "reset": read.reset,
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
