// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-event extraction and translation (Phase 1.5 PR 1). Feeds agent-pane
//! stdout through the Claude stream-json translator and publishes `AgentEvent`s.

use crate::backend::wps;

/// Maximum size of the per-block line buffer used by the agent-event
/// translation path. Past this, the buffer is reset (a producer that
/// never emits a newline can't grow it unboundedly). 1 MiB is far
/// beyond any plausible stream-json frame size.
pub(super) const AGENT_LINE_BUFFER_CAP: usize = 1024 * 1024;

/// Append `chunk` to `line_buf`, drain complete lines, JSON-parse
/// each, and run successful parses through `translator`. Returns the
/// events the translator emitted. Caller is responsible for
/// publishing them.
///
/// Buffers RAW BYTES (not lossy-decoded strings) so a multi-byte
/// UTF-8 character split across two PTY reads decodes cleanly when
/// the complete line arrives. Decoding each chunk lossily would
/// insert U+FFFD into the middle of words for non-ASCII content
/// (CJK, emoji, accented chars), silently corrupting
/// `AssistantText`/`Done.response` while the parallel raw-byte WPS
/// path stays correct — drone consumers would have no way to
/// recover. Reagent P1 + codex P2 on PR #833.
///
/// Pure function — split out from `accumulate_and_translate` so the
/// line-buffering + JSON-fast-reject + translator-call logic is
/// unit-testable without spinning up a broker.
pub(super) fn extract_agent_events(
    line_buf: &mut Vec<u8>,
    chunk: &[u8],
    translator: &mut crate::agents::translator::claude::ClaudeTranslator,
) -> Vec<crate::agents::types::AgentEvent> {
    use crate::agents::translator::Translator as _;
    let mut out: Vec<crate::agents::types::AgentEvent> = Vec::new();
    line_buf.extend_from_slice(chunk);
    if line_buf.len() > AGENT_LINE_BUFFER_CAP {
        // No newline in a megabyte — definitely not stream-json.
        // Reset to keep memory bounded.
        line_buf.clear();
        return out;
    }
    while let Some(nl) = line_buf.iter().position(|&b| b == b'\n') {
        let line_bytes: Vec<u8> = line_buf.drain(..=nl).collect();
        // Decode the COMPLETE line as lossy UTF-8 — split codepoints
        // are now fully present, so lossy-vs-strict only matters for
        // genuinely malformed bytes which would also fail strict.
        let line = String::from_utf8_lossy(&line_bytes);
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if !trimmed.starts_with('{') {
            // Fast-reject: stream-json frames are JSON objects.
            // Skips ANSI escapes, prompts, blank lines, etc.
            continue;
        }
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        out.extend(translator.translate(frame));
    }
    out
}

/// Publish the events `extract_agent_events` produced on the WPS
/// scope `agent_event:<block_id>`. Phase 1.5 PR 1 hook for agent
/// panes. Called only when `is_agent` is true at spawn time (see
/// read-task closure in `start()`).
pub(super) fn accumulate_and_translate(
    broker: &wps::Broker,
    block_id: &str,
    line_buf: &mut Vec<u8>,
    chunk: &[u8],
    translator: &mut crate::agents::translator::claude::ClaudeTranslator,
) {
    for event in extract_agent_events(line_buf, chunk, translator) {
        broker.publish(wps::WaveEvent {
            event: format!("agent_event:{}", block_id),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::to_value(&event).unwrap_or_default()),
        });
    }
}
