// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent was last asked, read from its GLOBAL transcript zone.
//!
//! "My Agents" needs a preview, a last-activity time and an honest empty state
//! for every agent, whether or not it was launched in this channel and whether
//! or not a pane is open. The per-block `output.state.json` it used to read is
//! no longer written; the history lives under `agent:<defId>:current` (or, once
//! archived, `agent:<defId>:archive:<ms>`) in the shared transcript store, keyed
//! by agent id and written by every channel.
//!
//! The `output` file there is the provider's raw NDJSON stream and can be
//! gigabytes (most of it `stream_event` deltas), so it is never loaded whole:
//! the newest user prompt is found by reading backwards in fixed chunks, up to
//! a cap, and the result is remembered until the file changes.
//! Spec: SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md §5.3.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::backend::storage::filestore::FileStore;

use super::archive::collapse_preview;
use super::session_io::OUTPUT_FILE;
use super::zone_naming::{agent_current_zone, is_valid_definition_id};

/// Bytes read per backwards step.
const CHUNK: i64 = 1024 * 1024;
/// The most of a transcript's tail one lookup reads. Real transcripts put the
/// newest user prompt well inside the last two chunks; past this the answer is
/// "none recent", not a longer read.
const MAX_SCAN: i64 = 4 * CHUNK;
/// A single line longer than this is skipped rather than carried across chunks.
const MAX_LINE: usize = 2 * 1024 * 1024;
/// Remembered results kept before the memo is reset.
const MEMO_CAP: usize = 512;

/// What the transcript store knows about one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryRead {
    /// No transcript for this agent: it has never run, or left nothing behind.
    NoTranscript,
    /// A transcript exists.
    Transcript(AgentHistory),
    /// The store could not be read; the real state is unknown.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHistory {
    /// The newest real user prompt in the scanned tail, collapsed to one line.
    /// Empty when the tail holds none (a long run of tool work).
    pub last_user_message: String,
    /// When the transcript last changed, in the store's own unit (ms).
    pub last_activity_ms: i64,
}

/// The store to read: the process-global transcript store, or in tests the one
/// the current thread installed.
fn history_store() -> Option<std::sync::Arc<FileStore>> {
    #[cfg(test)]
    {
        if let Some(s) = TEST_STORE.with(|t| t.borrow().clone()) {
            return Some(s);
        }
    }
    super::global_store::global_transcript_store().cloned()
}

#[cfg(test)]
thread_local! {
    static TEST_STORE: std::cell::RefCell<Option<std::sync::Arc<FileStore>>> =
        const { std::cell::RefCell::new(None) };
}

/// Install a transcript store for the current thread only (tests).
#[cfg(test)]
pub(crate) fn set_test_store(store: Option<std::sync::Arc<FileStore>>) {
    TEST_STORE.with(|t| *t.borrow_mut() = store);
}

/// History for each distinct agent id, read on the blocking pool. Empty when no
/// transcript store is installed, which callers treat as "use the old source".
pub async fn read_histories(definition_ids: Vec<String>) -> HashMap<String, HistoryRead> {
    let Some(store) = history_store() else {
        return HashMap::new();
    };
    let mut ids = definition_ids;
    ids.sort();
    ids.dedup();
    tokio::task::spawn_blocking(move || {
        ids.into_iter()
            .map(|id| {
                let r = read_agent_history(&store, &id);
                (id, r)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

type MemoKey = (usize, String);
type MemoVal = (i64, i64, AgentHistory);

fn memo() -> &'static Mutex<HashMap<MemoKey, MemoVal>> {
    static MEMO: std::sync::OnceLock<Mutex<HashMap<MemoKey, MemoVal>>> = std::sync::OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read what `definition_id`'s transcript says, falling back to its newest
/// archive when the current zone is empty.
pub fn read_agent_history(store: &FileStore, definition_id: &str) -> HistoryRead {
    if !is_valid_definition_id(definition_id) {
        return HistoryRead::NoTranscript;
    }
    let zone = match newest_zone_with_output(store, definition_id) {
        Ok(Some(z)) => z,
        Ok(None) => return HistoryRead::NoTranscript,
        Err(e) => {
            tracing::warn!(error = %e, definition_id, "agent history: transcript lookup failed");
            return HistoryRead::Failed;
        }
    };
    let (zone, size, modts) = zone;
    let key = (store as *const FileStore as usize, zone.clone());
    if let Some((s, m, h)) = memo().lock().unwrap().get(&key) {
        if *s == size && *m == modts {
            return HistoryRead::Transcript(h.clone());
        }
    }
    let last_user_message = match scan_tail_for_prompt(store, &zone, size) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, zone = %zone, "agent history: transcript tail read failed");
            return HistoryRead::Failed;
        }
    };
    let history = AgentHistory { last_user_message, last_activity_ms: modts };
    let mut memo = memo().lock().unwrap();
    if memo.len() >= MEMO_CAP {
        memo.clear();
    }
    memo.insert(key, (size, modts, history.clone()));
    HistoryRead::Transcript(history)
}

/// `(zone, size, modts)` of the agent's current zone if it has output, else of
/// its newest archive that does.
fn newest_zone_with_output(
    store: &FileStore,
    definition_id: &str,
) -> Result<Option<(String, i64, i64)>, String> {
    let current = agent_current_zone(definition_id);
    if let Some(f) = store.stat(&current, OUTPUT_FILE).map_err(|e| e.to_string())? {
        if f.size > 0 {
            return Ok(Some((current, f.size, f.modts)));
        }
    }
    let prefix = format!("agent:{definition_id}:archive:");
    let mut archives: Vec<(u64, String)> = store
        .zone_ids_with_prefix(&prefix)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|z| z.strip_prefix(&prefix).and_then(|t| t.parse::<u64>().ok()).map(|ts| (ts, z)))
        .collect();
    archives.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, zone) in archives {
        if let Some(f) = store.stat(&zone, OUTPUT_FILE).map_err(|e| e.to_string())? {
            if f.size > 0 {
                return Ok(Some((zone, f.size, f.modts)));
            }
        }
    }
    Ok(None)
}

/// The newest real user prompt within the last [`MAX_SCAN`] bytes of `output`,
/// or an empty string if there is none.
fn scan_tail_for_prompt(store: &FileStore, zone: &str, size: i64) -> Result<String, String> {
    let floor = (size - MAX_SCAN).max(0);
    let mut end = size;
    // The start of the line that straddles the boundary with the chunk after
    // this one; prepended to nothing, appended to this chunk's bytes.
    let mut carry: Vec<u8> = Vec::new();
    while end > floor {
        let start = (end - CHUNK).max(floor);
        let (_, mut buf) = store
            .read_at(zone, OUTPUT_FILE, start, end - start)
            .map_err(|e| e.to_string())?;
        buf.extend_from_slice(&carry);
        carry.clear();
        // Unless this chunk begins the file, its first line may be a fragment
        // of one that began in the chunk before; hold it back.
        let body_from = if start > 0 {
            match buf.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    if i <= MAX_LINE {
                        carry = buf[..i].to_vec();
                    }
                    i + 1
                }
                None => {
                    if buf.len() <= MAX_LINE {
                        carry = std::mem::take(&mut buf);
                    }
                    buf.len()
                }
            }
        } else {
            0
        };
        for line in buf[body_from.min(buf.len())..].split(|b| *b == b'\n').rev() {
            if let Some(msg) = user_prompt_in_line(line) {
                return Ok(collapse_preview(&msg));
            }
        }
        end = start;
    }
    Ok(String::new())
}

/// The text of a real user prompt if `line` is one: a `user` record that is not
/// a tool result, not from a sub-agent, and not startup or injected scaffolding.
fn user_prompt_in_line(line: &[u8]) -> Option<String> {
    // Cheap reject before parsing: nearly every line is a stream_event.
    if !contains(line, br#""type":"user""#) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(line).ok()?;
    if v.get("type")?.as_str()? != "user" {
        return None;
    }
    if v.get("tool_use_result").is_some() {
        return None;
    }
    if v.get("parent_tool_use_id").map_or(false, |p| !p.is_null()) {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => {
            if blocks.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")) {
                return None;
            }
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() || is_scaffolding(text) {
        return None;
    }
    Some(text.to_string())
}

/// Text the harness puts in the user turn that the user did not type.
fn is_scaffolding(text: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "# Session Context",
        "[JEKT:",
        "[BROADCAST:",
        "<system-reminder>",
        "<command-name>",
        "<command-message>",
        "<local-command",
        "Caveat:",
        "[Request interrupted",
    ];
    PREFIXES.iter().any(|p| text.starts_with(p))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};
    use std::collections::HashMap as Map;

    fn put(fs: &FileStore, zone: &str, body: &[u8]) {
        let meta: FileMeta = Map::new();
        fs.make_file(zone, OUTPUT_FILE, meta, FileOpts::default()).unwrap();
        fs.write_file(zone, OUTPUT_FILE, body).unwrap();
    }

    fn user(text: &str) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":text}}).to_string()
    }

    fn delta() -> String {
        r#"{"type":"stream_event","event":{"type":"content_block_delta"}}"#.to_string()
    }

    fn read(fs: &FileStore, id: &str) -> HistoryRead {
        read_agent_history(fs, id)
    }

    #[test]
    fn no_zone_is_no_transcript_and_a_bad_id_is_too() {
        let fs = FileStore::open_in_memory().unwrap();
        assert_eq!(read(&fs, "agent-1"), HistoryRead::NoTranscript);
        assert_eq!(read(&fs, "../evil"), HistoryRead::NoTranscript);
    }

    #[test]
    fn finds_the_newest_real_prompt_and_skips_tool_results_and_scaffolding() {
        let fs = FileStore::open_in_memory().unwrap();
        let body = [
            user("# Session Context\nboilerplate"),
            user("first real question"),
            delta(),
            user("fix the login race"),
            r#"{"type":"user","tool_use_result":{},"message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}"#.to_string(),
            r#"{"type":"user","parent_tool_use_id":"toolu_1","message":{"role":"user","content":"sub-agent prompt"}}"#.to_string(),
            user("[JEKT:FROM=someone TIER=coord]\nbody"),
            delta(),
        ]
        .join("\n");
        put(&fs, "agent:agent-1:current", body.as_bytes());
        match read(&fs, "agent-1") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "fix the login race"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    #[test]
    fn reads_a_prompt_given_as_text_blocks() {
        let fs = FileStore::open_in_memory().unwrap();
        let body = serde_json::json!({"type":"user","message":{"role":"user","content":[
            {"type":"text","text":"line one"},{"type":"text","text":"line two"}]}})
        .to_string();
        put(&fs, "agent:agent-2:current", body.as_bytes());
        match read(&fs, "agent-2") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "line one line two"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    #[test]
    fn a_transcript_with_no_prompt_is_still_a_transcript() {
        let fs = FileStore::open_in_memory().unwrap();
        put(&fs, "agent:agent-3:current", delta().as_bytes());
        match read(&fs, "agent-3") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, ""),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    #[test]
    fn an_archived_only_agent_reads_its_newest_archive() {
        let fs = FileStore::open_in_memory().unwrap();
        put(&fs, "agent:agent-4:archive:1000", user("old question").as_bytes());
        put(&fs, "agent:agent-4:archive:2000", user("newer question").as_bytes());
        match read(&fs, "agent-4") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "newer question"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_current_zone_falls_through_to_the_archive() {
        let fs = FileStore::open_in_memory().unwrap();
        put(&fs, "agent:agent-5:current", b"");
        put(&fs, "agent:agent-5:archive:5", user("before the archive").as_bytes());
        match read(&fs, "agent-5") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "before the archive"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    /// The point of the tail read: a transcript far larger than the scan cap is
    /// answered from its end without reading the rest, and a prompt buried
    /// deeper than the cap is reported as "none recent", not searched for.
    #[test]
    fn only_the_tail_is_read_and_a_prompt_beyond_the_cap_is_not_found() {
        let fs = FileStore::open_in_memory_with_cap(64 * 1024 * 1024).unwrap();
        let filler = format!("{}\n", delta());
        let mut body = Vec::new();
        body.extend_from_slice(user("buried far back").as_bytes());
        body.push(b'\n');
        while (body.len() as i64) < MAX_SCAN + 3 * CHUNK {
            body.extend_from_slice(filler.as_bytes());
        }
        put(&fs, "agent:agent-6:current", &body);
        match read(&fs, "agent-6") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, ""),
            other => panic!("expected a transcript, got {other:?}"),
        }
        // A recent prompt at the very end of the same file is found.
        let mut body2 = body.clone();
        body2.extend_from_slice(user("asked just now").as_bytes());
        put(&fs, "agent:agent-7:current", &body2);
        match read(&fs, "agent-7") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "asked just now"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    /// One filler line of exactly `n` bytes, newline included.
    fn filler_of(n: usize) -> Vec<u8> {
        let head = br#"{"type":"stream_event","p":""#;
        let tail = b"\"}\n";
        let mut v = head.to_vec();
        v.extend(std::iter::repeat(b'x').take(n - head.len() - tail.len()));
        v.extend_from_slice(tail);
        v
    }

    /// A prompt line that straddles a chunk boundary is reassembled, not lost.
    #[test]
    fn a_prompt_straddling_a_chunk_boundary_is_found() {
        let fs = FileStore::open_in_memory_with_cap(64 * 1024 * 1024).unwrap();
        let prompt = user("split across two chunks");
        let mut body = Vec::new();
        body.extend(filler_of(CHUNK as usize));
        body.extend_from_slice(prompt.as_bytes());
        body.push(b'\n');
        // The last chunk starts CHUNK bytes from the end, so this much after the
        // prompt puts that boundary 20 bytes into the prompt's own line.
        let after = CHUNK as usize - 20;
        body.extend(filler_of(after));
        put(&fs, "agent:agent-8:current", &body);
        match read(&fs, "agent-8") {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "split across two chunks"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }

    /// Manual check against a real transcript store (read-only): times a cold
    /// read of every agent in it.
    /// `AGENTMUX_HISTORY_REAL_STORE=<path to filestore.db> cargo test -p agentmux-srv --bins -- --ignored --nocapture history_real_store`
    #[test]
    #[ignore]
    fn history_real_store_timing() {
        let Ok(path) = std::env::var("AGENTMUX_HISTORY_REAL_STORE") else { return };
        let fs = FileStore::open_read_only(std::path::Path::new(&path)).unwrap();
        let ids: Vec<String> = fs
            .zone_ids_with_prefix("agent:")
            .unwrap()
            .into_iter()
            .filter_map(|z| z.strip_suffix(":current").and_then(|z| z.strip_prefix("agent:")).map(String::from))
            .collect();
        let t = std::time::Instant::now();
        let (mut found, mut empty, mut none, mut failed) = (0, 0, 0, 0);
        for id in &ids {
            match read_agent_history(&fs, id) {
                HistoryRead::Transcript(h) if h.last_user_message.is_empty() => empty += 1,
                HistoryRead::Transcript(_) => found += 1,
                HistoryRead::NoTranscript => none += 1,
                HistoryRead::Failed => failed += 1,
            }
        }
        println!("{} agents: {found} with a prompt, {empty} without, {none} no transcript, {failed} failed, cold in {:?}", ids.len(), t.elapsed());
        let t = std::time::Instant::now();
        for id in &ids {
            let _ = read_agent_history(&fs, id);
        }
        println!("warm in {:?}", t.elapsed());
    }

    #[test]
    fn a_changed_transcript_is_read_again() {
        let fs = FileStore::open_in_memory().unwrap();
        put(&fs, "agent:agent-9:current", user("one").as_bytes());
        let first = read(&fs, "agent-9");
        fs.append_data("agent:agent-9:current", OUTPUT_FILE, format!("\n{}", user("two")).as_bytes())
            .unwrap();
        let second = read(&fs, "agent-9");
        assert_ne!(first, second);
        match second {
            HistoryRead::Transcript(h) => assert_eq!(h.last_user_message, "two"),
            other => panic!("expected a transcript, got {other:?}"),
        }
    }
}
