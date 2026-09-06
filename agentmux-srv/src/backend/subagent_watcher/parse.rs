// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Free-standing parsing/path utilities used by the rest of `subagent_watcher`
//! — none of these need `&self`. Covers: JSONL line parsing, journal-count
//! reading, workflow/session id derivation from a path, the block-delete
//! cascade subscriber, and Claude config dir resolution.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::types::{SubagentEvent, SubagentEventType};
use super::SubagentWatcher;

// ── Tool-call display summaries ─────────────────────────────────────────
//
// Both limits are in CHARS. They used to be bare `200`/`500` literals that
// tested a BYTE length (`s.len()`) and then cut by CHAR index — agreeing for
// ASCII and disagreeing for anything else (a string over 200 bytes but under
// 200 chars took the truncation branch, found no 200th char, and fell through
// to no truncation at all). See
// docs/reports/REPORT_TOOL_SUMMARY_FORMATTING_ARCHITECTURE_2026_09_06.md §4.1.

/// Chars of a tool call's input kept for the Swarm feed's collapsed row.
const MAX_INPUT_SUMMARY_CHARS: usize = 200;

/// Chars of a tool result kept for the same feed. Larger than the input
/// budget because a result is the thing a reader is usually scanning.
const MAX_RESULT_PREVIEW_CHARS: usize = 500;

/// Cut to `max` CHARS (never mid-codepoint), appending an ellipsis when it
/// actually cut something.
fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((byte_idx, _)) => format!("{}...", &s[..byte_idx]),
        None => s.to_string(),
    }
}

/// The human-meaningful field of a tool call's `input`, by tool name.
///
/// Mirrors the frontend's `extractToolDetail`
/// (`frontend/app/view/agent/stream-parser.ts`), which is what the Agent pane,
/// the Activity Dock and the tool blocks have always used. The Swarm feed
/// instead showed `serde_json::Value::to_string()` — the whole input object as
/// compact JSON, cut mid-token — because summarising happens here, at parse
/// time, and the structured input is dropped before it ever reaches a view
/// that could format it properly.
///
/// **This duplicates the mapping in a second language, deliberately and
/// temporarily.** It buys the readable output now; the real fix is to carry
/// the structured input across the boundary and let the frontend's existing
/// extractor (and its renderer registry) handle it, which deletes this
/// function. Phases 1 and 2 of the report above.
///
/// Unknown tools fall back to compact JSON rather than an empty string: for a
/// tool this doesn't know, the raw object is genuinely more useful than
/// nothing, and `mcp__*`/provider-specific names are open-ended by design.
/// (The frontend's version returns `""` there because its callers have the
/// full node to fall back on; here there is nothing else to show.)
fn tool_input_detail(tool: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input.get(key).and_then(|v| v.as_str());
    let pick = match tool {
        "Read" | "Edit" | "Write" | "NotebookEdit" => field("file_path"),
        "Bash" | "BashOutput" => field("command"),
        "Grep" | "Glob" => field("pattern"),
        "Agent" | "Task" => field("description").or_else(|| field("prompt")),
        "Workflow" => field("title").or_else(|| field("description")),
        "WebSearch" | "web_search" => field("query"),
        "WebFetch" | "web_fetch" => field("url"),
        _ => None,
    };
    match pick {
        Some(s) if !s.is_empty() => s.to_string(),
        // Either an unknown tool, or a known one whose expected field is
        // missing/empty/non-string — in both cases the object beats nothing.
        _ => input.to_string(),
    }
}

/// A tool result's text. A plain string is used as-is; anything else (the
/// common `[{"type":"text","text":…}]` block array, or a bare object) falls
/// back to compact JSON, same as before.
fn result_content_text(content: &serde_json::Value) -> String {
    match content.as_str() {
        Some(s) => s.to_string(),
        None => content.to_string(),
    }
}

// ── Block-delete cascade subscriber ─────────────────────────────────────

/// Subscribe to the reducer's `srv_events_tx` broadcast and prune `watcher`'s
/// per-block state on `Event::BlockDeleted`/`TabDeleted`/`WorkspaceDeleted` —
/// the robust backstop described on `SubagentWatcher::prune_block`'s doc
/// comment. Mirrors `agentmux-cef/src/srv_ipc.rs`'s cascaded-block-id
/// extraction (same three-arm match, same rationale: `TabDeleted`/
/// `WorkspaceDeleted` never emit a per-block event of their own — see
/// `reducer/tab.rs::handle_delete_tab`'s doc comment — so `block_ids` is the
/// only signal for a block that cascaded out via its tab/workspace) and
/// `server/wave_obj_bridge.rs::run_wave_obj_bridge`'s subscribe-loop
/// plumbing (lag/close handling).
pub fn spawn_block_prune_subscriber(
    watcher: Arc<SubagentWatcher>,
    mut events_rx: tokio::sync::broadcast::Receiver<agentmux_common::ipc::Event>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match events_rx.recv().await {
                Ok(event) => {
                    let cascaded_block_ids: &[String] = match &event {
                        agentmux_common::ipc::Event::BlockDeleted { block_id, .. } => {
                            std::slice::from_ref(block_id)
                        }
                        agentmux_common::ipc::Event::TabDeleted { block_ids, .. } => block_ids.as_slice(),
                        agentmux_common::ipc::Event::WorkspaceDeleted { block_ids, .. } => block_ids.as_slice(),
                        _ => &[],
                    };
                    for block_id in cascaded_block_ids {
                        watcher.prune_block_and_notify(block_id);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Same non-fatal, no-automatic-recovery handling as
                    // wave_obj_bridge.rs's identical arm: a lag here means a
                    // stale swarm-pane row could persist until its block's
                    // next delete-adjacent event, not silent data loss.
                    tracing::warn!(
                        skipped = n,
                        "subagent block-prune subscriber lagged; some BlockDeleted/TabDeleted/WorkspaceDeleted events were dropped"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("subagent block-prune subscriber exiting: events channel closed");
                    return;
                }
            }
        }
    })
}

// ── JSONL parsing ─────────────────────────────────────────────────────────

/// Metadata extracted from the first JSONL line (the subagent init record).
pub(super) struct JsonlMeta {
    pub(super) slug: String,
    pub(super) model: Option<String>,
    /// The line's own `"parentUuid"` field, verbatim (SPEC §9.2 — `SubAgent.
    /// spawned_from_agent_id`). `None` in every real transcript checked so
    /// far; captured defensively since this line is already parsed.
    pub(super) parent_uuid: Option<String>,
}

/// Read a JSONL file from a byte offset, parsing new subagent events.
/// Returns (events, new_offset, optional_meta).
pub(super) fn read_jsonl_from_offset(
    path: &Path,
    offset: u64,
) -> Result<(Vec<SubagentEvent>, u64, Option<JsonlMeta>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let file_len = file.metadata().map_err(|e| format!("metadata: {e}"))?.len();

    if file_len <= offset {
        return Ok((Vec::new(), offset, None));
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek: {e}"))?;

    let mut events = Vec::new();
    let mut meta = None;
    let mut current_offset = offset;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        current_offset += line.len() as u64 + 1; // +1 for newline

        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract metadata from init/config lines
        if offset == 0 && meta.is_none() {
            let parent_uuid = value
                .get("parentUuid")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let Some(slug) = value.get("slug").and_then(|v| v.as_str()) {
                meta = Some(JsonlMeta {
                    slug: slug.to_string(),
                    model: value
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    parent_uuid: parent_uuid.clone(),
                });
            }
            if meta.is_none() {
                if let Some(agent_id) = value.get("agentId").and_then(|v| v.as_str()) {
                    meta = Some(JsonlMeta {
                        slug: value
                            .get("slug")
                            .and_then(|v| v.as_str())
                            .unwrap_or(agent_id)
                            .to_string(),
                        model: value
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        parent_uuid,
                    });
                }
            }
        }

        let timestamp = value
            .get("timestamp")
            .and_then(parse_event_timestamp)
            .unwrap_or_else(now_millis);

        let event_type = parse_event_type(&value);
        if let Some(et) = event_type {
            let line_agent_id = value
                .get("agentId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            events.push(SubagentEvent {
                agent_id: line_agent_id,
                event_type: et,
                timestamp,
            });
        }
    }

    Ok((events, current_offset, meta))
}

/// Read just the subagent's own initial task prompt from the first JSONL
/// line (a `"type":"user"` init record) — used by `subagent.GenerateName` as
/// the source text for the Haiku naming call. Deliberately bypasses the
/// events cache/offset machinery `read_jsonl_from_offset` uses: naming only
/// ever needs the first line, is called at most once per subagent (cached
/// via `display_name` thereafter), and must work even for a subagent whose
/// events haven't been scanned into `SubagentState` yet.
///
/// `message.content` is either a plain string or an array of content blocks
/// (mirrors the two shapes `parse_event_type`'s "assistant" arm already
/// handles) — both are accepted here.
pub(crate) fn read_task_prompt(jsonl_path: &str) -> Option<String> {
    let file = std::fs::File::open(jsonl_path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;

    let value: serde_json::Value = serde_json::from_str(first_line.trim()).ok()?;
    if value.get("type").and_then(|v| v.as_str()) != Some("user") {
        return None;
    }
    let content = value.get("message")?.get("content")?;

    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }

    if let Some(arr) = content.as_array() {
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
        let joined = texts.join("\n").trim().to_string();
        return (!joined.is_empty()).then_some(joined);
    }

    None
}

/// Parse a JSONL line into a SubagentEventType based on the `type` field.
pub(super) fn parse_event_type(value: &serde_json::Value) -> Option<SubagentEventType> {
    let event_type = value.get("type").and_then(|v| v.as_str())?;

    match event_type {
        "assistant" => {
            let content = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| {
                    if let Some(arr) = c.as_array() {
                        let texts: Vec<&str> = arr
                            .iter()
                            .filter_map(|block| {
                                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    block.get("text").and_then(|t| t.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if texts.is_empty() {
                            None
                        } else {
                            Some(texts.join("\n"))
                        }
                    } else {
                        c.as_str().map(|s| s.to_string())
                    }
                })
                .unwrap_or_default();
            Some(SubagentEventType::Text { content })
        }
        "tool_use" => {
            let name = value
                .get("name")
                .or_else(|| value.get("tool_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let input_summary = value
                .get("input")
                .map(|v| truncate_chars(&tool_input_detail(&name, v), MAX_INPUT_SUMMARY_CHARS))
                .unwrap_or_default();
            Some(SubagentEventType::ToolUse {
                name,
                input_summary,
            })
        }
        "tool_result" => {
            let is_error = value
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let preview = value
                .get("content")
                .or_else(|| value.get("output"))
                .map(|v| truncate_chars(&result_content_text(v), MAX_RESULT_PREVIEW_CHARS))
                .unwrap_or_default();
            Some(SubagentEventType::ToolResult { is_error, preview })
        }
        "progress" => {
            let output = value
                .get("output")
                .or_else(|| value.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(SubagentEventType::Progress { output })
        }
        "result" => {
            let content = value
                .get("result")
                .or_else(|| value.get("content"))
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_else(|| "Subagent completed".to_string());
            Some(SubagentEventType::Result { content })
        }
        _ => None,
    }
}

/// Epoch-millis for a subagent JSONL line's `timestamp` field, tolerant of
/// BOTH shapes that occur in real transcripts.
///
/// Claude Code writes an ISO-8601 STRING (`"2026-07-03T08:09:20.743Z"`), but
/// this was read with a bare `as_u64()`, which returns `None` for a string
/// and silently fell through to `now_millis()`. Every replayed historical
/// event therefore claimed to have happened *at replay time*.
///
/// That is not a cosmetic timestamp bug: `last_event_at` becomes
/// `subagentToActivity`'s `endedAt` (activity/subagent-adapter.ts), and the
/// Activity Dock keeps a terminal row only while
/// `now - endedAt < RETENTION_MS[status]`. With `endedAt ≈ now`, every
/// long-dead subagent replayed on pane (re)open passed that window and
/// rendered as a live-looking dock row, then vanished a few seconds later
/// when it aged out — the "dock flashes on load, then disappears" report.
/// With real historical timestamps these rows are filtered out immediately
/// and never paint at all.
///
/// Numeric epoch-millis are still accepted unchanged (other providers, and
/// any future writer that emits a number). An unparseable value returns
/// `None` so the caller keeps its existing `now_millis()` fallback — a
/// genuinely-live event with a malformed timestamp is better dated "now"
/// than dropped to the epoch.
///
/// See docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md.
pub(super) fn parse_event_timestamp(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    let s = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
        .filter(|ms| *ms >= 0)
        .map(|ms| ms as u64)
}

/// Modification time of `path`, or `UNIX_EPOCH` if it can't be read (a
/// vanished/permission-denied file sorts oldest — excluded first by
/// `scan_subagents_dir`'s recency cap rather than crashing the scan).
pub(super) fn file_mtime(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Walk up `path`'s ancestors (parent, grandparent, ...) and return the
/// first one that already exists on disk. Used by `watch_agent` when the
/// target directory doesn't exist yet — `notify::Watcher::watch` fails
/// outright on a nonexistent path, so this finds the closest directory that
/// can actually be watched right now (a later-created descendant is still
/// picked up, since the watch is recursive).
///
/// Never walks above the user's home directory. Without a floor, a fresh
/// environment where even `~/.config` doesn't exist yet (ephemeral
/// dev/CI/container — this project's own docs describe several such setups)
/// would walk all the way to `$HOME` or further, and
/// `watcher.watch(&watched_dir, RecursiveMode::Recursive)` performs a
/// synchronous, blocking directory walk of whatever it's handed — recursing
/// the entire home directory (or beyond) from inside the async
/// `handle_reactive_register` request handler risks a long stall and, on
/// Linux, exhausting the OS-wide inotify watch-count limit. Returns `None`
/// (giving up on watching rather than risking an unbounded walk) if `path`
/// isn't under the home directory at all, or if no existing ancestor is
/// found within that bound.
pub(super) fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let floor = dirs::home_dir()?;
    path.ancestors()
        .skip(1)
        .take_while(|p| p.starts_with(&floor))
        .find(|p| p.exists())
        .map(|p| p.to_path_buf())
}

/// Extract the workflow id from a path under `.../subagents/workflows/<id>/...`.
/// Returns None for direct (non-workflow) subagent files.
pub(super) fn parse_workflow_id(path: &Path) -> Option<String> {
    let comps: Vec<&str> = path.iter().filter_map(|c| c.to_str()).collect();
    comps.windows(3).find_map(|w| {
        (w[0] == "subagents" && w[1] == "workflows" && !w[2].ends_with(".jsonl"))
            .then(|| w[2].to_string())
    })
}

/// Session id = the name of the directory containing `subagents/`. Workflow
/// member files are nested (`subagents/workflows/<wf>/agent-*.jsonl`), so walk
/// ancestors instead of assuming a fixed depth.
pub(super) fn derive_session_id(path: &Path) -> String {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.file_name().and_then(|n| n.to_str()) == Some("subagents") {
            return d
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }
        dir = d.parent();
    }
    "unknown".to_string()
}

/// Count new `started` / `result` records in a workflow journal from `offset`.
/// Returns (started, results, new_offset).
///
/// Reads line-by-line via `read_until(b'\n', ..)` rather than `BufRead::lines()`
/// so a trailing line with no `\n` yet (the writer racing a partial append) is
/// never counted or consumed: `new_offset` only ever advances past complete,
/// newline-terminated lines, so the next call re-reads that partial line whole
/// once the rest of it lands, instead of seeking mid-line and silently losing
/// the record's leading bytes (and the record itself, since the resulting
/// truncated JSON fails to parse).
pub(super) fn read_journal_counts(path: &Path, offset: u64) -> Result<(usize, usize, u64), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let file_len = file.metadata().map_err(|e| format!("metadata: {e}"))?.len();
    if file_len <= offset {
        return Ok((0, 0, offset));
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek: {e}"))?;

    let (mut started, mut results) = (0usize, 0usize);
    let mut current_offset = offset;
    loop {
        let mut buf = Vec::new();
        let bytes_read = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("read_until: {e}"))?;
        if bytes_read == 0 {
            break; // EOF
        }
        if !buf.ends_with(b"\n") {
            break; // trailing partial line — leave it unconsumed for next time
        }
        current_offset += bytes_read as u64;

        let line = String::from_utf8_lossy(&buf);
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match value.get("type").and_then(|v| v.as_str()) {
            Some("started") => started += 1,
            Some("result") => results += 1,
            _ => {}
        }
    }
    Ok((started, results, current_offset))
}

pub(super) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Utility: encode workspace path like Claude Code does ──────────────────

/// Encode a workspace path the same way Claude Code does for its projects dir.
#[allow(dead_code)]
pub fn encode_workspace_path(workspace_path: &str) -> String {
    workspace_path
        .replace('\\', "-")
        .replace('/', "-")
        .replace(':', "")
}

/// Derive the Claude Code config directory for a host agent. Only matches
/// reality for an agent with an explicit per-identity bundle override —
/// prefer `resolve_claude_config_dir` when the block's meta is available.
pub fn derive_claude_config_dir(agent_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let config_dir = home
        .join(".config")
        .join(format!("claude-{}", agent_id.to_lowercase()));
    Some(config_dir)
}

/// The authoritative Claude Code config directory for a block.
///
/// `bound_dir` — pass `identity::resolver::resolve_bound_oauth_config_dir`'s
/// result here. It wins when present: for an agent bound to an explicit
/// Armory identity, it's the ONLY correct answer — `cmd:env.CLAUDE_CONFIG_DIR`
/// below is a write-once launch-time snapshot of the generic shared-provider
/// dir, while the identity-bound agent's CLI actually runs (and writes
/// subagent files) under its own identity dir, re-resolved fresh on every
/// turn (`inject_identity_env_with_broker`,
/// `SPEC_PROVIDER_ISOLATION_2026_06_20.md` §4.3) — `cmd:env` is never
/// updated to match. Confirmed live: a real dispatch's transcript landed
/// under the identity dir while the watcher, following `cmd:env` alone, was
/// watching the generic dir and never saw it — see
/// `SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md`.
///
/// Falls back to `cmd:env.CLAUDE_CONFIG_DIR` (written by the launch flow
/// before spawn — see `agentmux-cef`'s `ensure_auth_dir`) when `bound_dir`
/// is `None` (an ambient/unbound agent, where `cmd:env` IS correct and
/// stays correct — nothing to re-resolve). Falls back further to
/// `derive_claude_config_dir`'s legacy `~/.config/claude-<agent_id>` guess
/// only when `cmd:env` isn't set yet either.
///
/// This distinction matters: `derive_claude_config_dir`'s guess only holds
/// for an agent with an explicit per-identity bundle override. Any agent
/// without one launches under the shared default at
/// `~/.agentmux/shared/providers/claude/`, a completely different path that
/// the guess never matches — silently disabling subagent tracking for that
/// agent forever (confirmed live: repeated "config dir does not exist yet"
/// over 38+ minutes and multiple re-registrations, for an agent that had, in
/// fact, already spawned subagents — just under the real shared path). See
/// docs/specs/archive/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md.
pub fn resolve_claude_config_dir(
    meta: &crate::backend::obj::MetaMapType,
    agent_id: &str,
    bound_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    bound_dir
        .or_else(|| {
            meta.get("cmd:env")
                .and_then(|v| v.get("CLAUDE_CONFIG_DIR"))
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
        })
        .or_else(|| derive_claude_config_dir(agent_id))
}

#[cfg(test)]
mod summary_tests {
    use super::*;
    use serde_json::json;

    // The flagship case, taken verbatim from the transcript that prompted
    // docs/reports/REPORT_TOOL_SUMMARY_FORMATTING_ARCHITECTURE_2026_09_06.md —
    // the Swarm feed rendered the whole input object as escaped JSON.
    #[test]
    fn a_bash_call_shows_its_command_not_the_json_object() {
        let input = json!({
            "command": "ls /c/Users/area54/ 2>/dev/null | head -50",
            "description": "List home directory contents",
        });
        let got = tool_input_detail("Bash", &input);
        assert_eq!(got, "ls /c/Users/area54/ 2>/dev/null | head -50");
        assert!(!got.contains('{'), "must not be a JSON dump: {got}");
        assert!(!got.contains('"'), "must not carry JSON quoting: {got}");
    }

    #[test]
    fn file_tools_show_the_path() {
        for tool in ["Read", "Edit", "Write"] {
            let input = json!({ "file_path": "/x/y.rs", "old_string": "…" });
            assert_eq!(tool_input_detail(tool, &input), "/x/y.rs", "tool {tool}");
        }
    }

    #[test]
    fn search_tools_show_the_pattern() {
        let input = json!({ "pattern": "fn main", "path": "/repo" });
        assert_eq!(tool_input_detail("Grep", &input), "fn main");
        assert_eq!(tool_input_detail("Glob", &input), "fn main");
    }

    #[test]
    fn a_dispatch_prefers_its_description_over_the_whole_prompt() {
        // An Agent prompt runs to tens of KB; the description is the one-line
        // human summary the caller already wrote.
        let input = json!({ "description": "Research the thing", "prompt": "x".repeat(5000) });
        assert_eq!(tool_input_detail("Agent", &input), "Research the thing");
    }

    #[test]
    fn a_dispatch_falls_back_to_the_prompt_when_undescribed() {
        let input = json!({ "prompt": "do the thing" });
        assert_eq!(tool_input_detail("Agent", &input), "do the thing");
    }

    #[test]
    fn an_unknown_tool_keeps_the_raw_object_rather_than_showing_nothing() {
        // Open-ended by design (`mcp__*`, provider-specific). The object is
        // worse than a real field but strictly better than blank.
        let input = json!({ "anything": 1 });
        assert_eq!(tool_input_detail("mcp__whatever__do", &input), input.to_string());
    }

    #[test]
    fn a_known_tool_missing_its_field_also_falls_back() {
        // Guards the `Some(s) if !s.is_empty()` arm: a Bash call with no
        // command must not render as an empty row.
        assert_eq!(
            tool_input_detail("Bash", &json!({ "description": "no command here" })),
            json!({ "description": "no command here" }).to_string()
        );
        assert_eq!(
            tool_input_detail("Bash", &json!({ "command": "" })),
            json!({ "command": "" }).to_string()
        );
    }

    #[test]
    fn a_string_result_is_used_verbatim() {
        assert_eq!(result_content_text(&json!("plain output")), "plain output");
    }

    #[test]
    fn a_block_array_result_falls_back_to_json() {
        let content = json!([{ "type": "text", "text": "hi" }]);
        assert_eq!(result_content_text(&content), content.to_string());
    }

    #[test]
    fn truncation_counts_chars_and_marks_that_it_cut() {
        assert_eq!(truncate_chars("abcdef", 3), "abc...");
        assert_eq!(truncate_chars("abc", 3), "abc", "exact length must not gain an ellipsis");
        assert_eq!(truncate_chars("ab", 3), "ab");
    }

    #[test]
    fn truncation_never_splits_a_multibyte_char() {
        // The old code tested byte length and cut by char index, so a string
        // over the byte budget but under the char budget silently escaped
        // truncation entirely (§4.1 of the report).
        let s = "é".repeat(150); // 300 bytes, 150 chars
        assert_eq!(truncate_chars(&s, 200), s, "under the CHAR budget — unchanged");
        let long = "é".repeat(250);
        let got = truncate_chars(&long, 200);
        assert_eq!(got.chars().count(), 203, "200 chars + the three-dot marker");
    }
}
