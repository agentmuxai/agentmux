// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What AgentMux's own record says about an agent's conversations.
//!
//! Every agent's provider output is mirrored into the global transcript store
//! (`~/.agentmux/shared/agents/transcripts/filestore.db`): zone
//! `agent:<id>:current`, moved to `agent:<id>:archive:<ms>` by "New
//! conversation". For a user agent `<id>` is its UID (`db_agents.id`), the same
//! in every channel and version and across account switches, so this record is
//! the one place that holds every session the agent ran — including sessions
//! whose provider transcript is gone (75 of 190 sessions across the 15 agents
//! on one host, measured 2026-09-24). A template launch writes into its
//! template's shared zone; that zone is not the agent's alone and is never read
//! here.
//!
//! Sessions come from the record, not from guessing by folder or account:
//! every line the provider writes carries its `session_id`. A line without one
//! — a message a person typed, echoed by AgentMux — belongs to the session of
//! the next line that has one, the process that received it. The record only
//! grows until it's archived, so each zone remembers how far it has been read
//! and a later search reads only what was appended.
//!
//! Claude's stream-json only: a line from a provider whose output carries no
//! `session_id` never starts a session here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::adapter::{HistoryError, HistoryMessage};
use crate::backend::agent_session::{OUTPUT_FILE, TSIDX_FILE};
use crate::backend::storage::filestore::FileStore;

/// How much of a zone is read per step.
const READ_CHUNK: i64 = 4 * 1024 * 1024;

/// A session, as the agent's record holds it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordSession {
    pub session_id: String,
    /// The working directory its `init` line reported.
    pub cwd: Option<String>,
    /// `(zone, start, end)` byte ranges of the zone's `output`, in record
    /// order. Empty for a session only the segment log knows.
    pub ranges: Vec<(String, i64, i64)>,
    /// Receive times of its first and last lines (unix ms); 0 = unknown.
    pub first_ms: i64,
    pub last_ms: i64,
}

/// Consecutive lines of one session.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    session_id: String,
    start: i64,
    end: i64,
}

/// How far a zone has been read, and what was found.
#[derive(Debug, Default, Clone)]
struct ZoneScan {
    /// Bytes of complete lines already read.
    read_to: i64,
    runs: Vec<Run>,
    /// Start of trailing lines no session owns yet (a typed message whose
    /// answer hasn't arrived).
    unowned_from: Option<i64>,
    cwd: HashMap<String, String>,
}

/// Reads agents' records from the global transcript store. No store (a
/// test, or a srv that couldn't attach it) reads nothing.
pub(crate) struct AgentRecords {
    store: StoreSource,
    scans: Mutex<HashMap<String, ZoneScan>>,
}

enum StoreSource {
    /// The global transcript store, looked up when used: bootstrap attaches it.
    Global,
    Fixed(Option<Arc<FileStore>>),
}

impl AgentRecords {
    pub(crate) fn global() -> Self {
        AgentRecords { store: StoreSource::Global, scans: Mutex::new(HashMap::new()) }
    }

    pub(crate) fn new(store: Option<Arc<FileStore>>) -> Self {
        AgentRecords { store: StoreSource::Fixed(store), scans: Mutex::new(HashMap::new()) }
    }

    fn store(&self) -> Option<Arc<FileStore>> {
        match &self.store {
            StoreSource::Global => crate::backend::agent_session::global_transcript_store().cloned(),
            StoreSource::Fixed(store) => store.clone(),
        }
    }

    /// Every session in `uid`'s record — its current conversation and every
    /// archived one — plus the segment log's sessions the record doesn't
    /// have, and what couldn't be read. Newest last activity first.
    pub(crate) fn sessions_of(&self, uid: &str) -> (Vec<RecordSession>, Vec<String>) {
        let Some(store) = self.store() else {
            return (Vec::new(), Vec::new());
        };
        let store = &*store;
        if !crate::backend::agent_session::is_valid_definition_id(uid) {
            return (Vec::new(), Vec::new());
        }
        let mut problems = Vec::new();
        let zones = match store.get_all_zone_ids() {
            Ok(all) => all.into_iter().filter(|z| is_agents_own_zone(z, uid)).collect::<Vec<_>>(),
            Err(e) => return (Vec::new(), vec![format!("agent record: listing zones failed: {e}")]),
        };

        let mut by_id: HashMap<String, RecordSession> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for zone in &zones {
            let scan = {
                let mut scans = self.scans.lock().unwrap_or_else(|p| p.into_inner());
                let scan = scans.entry(zone.clone()).or_default();
                if let Err(e) = scan_zone(store, zone, scan) {
                    problems.push(format!("agent record {zone}: {e}"));
                }
                scan.clone()
            };
            // Trailing unowned lines stay unassigned until their answer
            // arrives, except in a finished (archived) zone, where nothing
            // will: they belong to its last session.
            let mut runs = scan.runs.clone();
            if let (Some(from), Some(last)) = (scan.unowned_from, runs.last_mut()) {
                if zone.contains(":archive:") {
                    last.end = last.end.max(scan.read_to.max(from));
                }
            }
            let stamps = zone_stamps(store, zone, &runs);
            for (i, run) in runs.iter().enumerate() {
                let (first, last) = stamps.get(i).copied().unwrap_or((0, 0));
                let session = by_id.entry(run.session_id.clone()).or_insert_with(|| {
                    order.push(run.session_id.clone());
                    RecordSession {
                        session_id: run.session_id.clone(),
                        cwd: scan.cwd.get(&run.session_id).cloned(),
                        ranges: Vec::new(),
                        first_ms: 0,
                        last_ms: 0,
                    }
                });
                session.ranges.push((zone.clone(), run.start, run.end));
                if first != 0 && (session.first_ms == 0 || first < session.first_ms) {
                    session.first_ms = first;
                }
                session.last_ms = session.last_ms.max(last);
                if session.cwd.is_none() {
                    session.cwd = scan.cwd.get(&run.session_id).cloned();
                }
            }
        }

        // The segment log (#3676) names every session a spawn ran since it
        // shipped, even one whose output never reached the record.
        for segment in crate::backend::continuity_segments::segments(store, uid) {
            let Some(sid) = segment.start.provider_session_id else { continue };
            by_id.entry(sid.clone()).or_insert_with(|| {
                order.push(sid.clone());
                RecordSession {
                    session_id: sid,
                    cwd: Some(segment.start.cwd).filter(|c| !c.is_empty()),
                    ranges: Vec::new(),
                    first_ms: segment.start.started_at_ms,
                    last_ms: segment.ended_at_ms.unwrap_or(segment.start.started_at_ms),
                }
            });
        }

        let position: HashMap<&str, usize> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        let mut sessions: Vec<RecordSession> = by_id.into_values().collect();
        sessions.sort_by(|a, b| {
            b.last_ms
                .cmp(&a.last_ms)
                .then(position[b.session_id.as_str()].cmp(&position[a.session_id.as_str()]))
        });
        (sessions, problems)
    }

    /// A session's messages, from the record, with the receive time of each
    /// line; and how many of its records couldn't be read.
    pub(crate) fn read(&self, session: &RecordSession) -> Result<(Vec<HistoryMessage>, u32), HistoryError> {
        let Some(store) = self.store() else {
            return Err(HistoryError::Other("AgentMux's transcript store isn't available".into()));
        };
        let store = &*store;
        let mut messages = Vec::new();
        let mut skipped = 0u32;
        for (zone, start, end) in &session.ranges {
            let mut offsets: Vec<u64> = Vec::new();
            let mut zone_messages: Vec<HistoryMessage> = Vec::new();
            for_each_line(store, zone, *start, *end, |line, offset| {
                if line.iter().all(u8::is_ascii_whitespace) || is_stream_event(line) {
                    return;
                }
                let Ok(entry) = serde_json::from_slice::<serde_json::Value>(line) else {
                    skipped += 1;
                    return;
                };
                if entry.get("session_id").and_then(|v| v.as_str()).is_some_and(|sid| sid != session.session_id) {
                    return;
                }
                if let Some(message) = super::claude_adapter::message_from_entry(&entry, 0) {
                    offsets.push(offset as u64);
                    zone_messages.push(message);
                }
            })
            .map_err(|e| HistoryError::Other(format!("agent record {zone}: {e}")))?;
            if let Some(stamps) = line_stamps(store, zone, &offsets) {
                for (message, ms) in zone_messages.iter_mut().zip(stamps) {
                    message.timestamp = ms;
                }
            }
            messages.extend(zone_messages);
        }
        Ok((messages, skipped))
    }
}

/// `agent:<uid>:current` and `agent:<uid>:archive:<ms>` — never another id's
/// zone, and never a template's shared one (its key isn't this UID).
fn is_agents_own_zone(zone: &str, uid: &str) -> bool {
    let Some(rest) = zone.strip_prefix("agent:").and_then(|z| z.strip_prefix(uid)) else {
        return false;
    };
    rest == ":current" || rest.strip_prefix(":archive:").is_some_and(|ms| !ms.is_empty() && ms.bytes().all(|b| b.is_ascii_digit()))
}

/// Streaming deltas: most of the record's bytes, and never a message.
fn is_stream_event(line: &[u8]) -> bool {
    memchr::memmem::find(line.get(..48).unwrap_or(line), b"\"stream_event\"").is_some()
}

/// The session a line belongs to, if it names one. A streaming delta's shape
/// is fixed and its payload holds only escaped strings, so its last
/// `"session_id":"` key is the line's own; anything else is parsed.
fn line_session_id(line: &[u8]) -> Option<String> {
    if is_stream_event(line) {
        const KEY: &[u8] = b"\"session_id\":\"";
        let at = memchr::memmem::rfind(line, KEY)? + KEY.len();
        let len = memchr::memchr(b'"', &line[at..])?;
        return String::from_utf8(line[at..at + len].to_vec()).ok().filter(|s| !s.is_empty());
    }
    // Only the top-level key: the rest of the line is skipped, not built.
    #[derive(serde::Deserialize)]
    struct Top {
        #[serde(default)]
        session_id: Option<String>,
    }
    serde_json::from_slice::<Top>(line).ok()?.session_id.filter(|s| !s.is_empty())
}

/// The working directory an `init` line reports.
fn init_cwd(line: &[u8]) -> Option<String> {
    // `init` is a small system line whose subtype comes first.
    if memchr::memmem::find(line.get(..96).unwrap_or(line), b"\"subtype\":\"init\"").is_none() {
        return None;
    }
    let entry: serde_json::Value = serde_json::from_slice(line).ok()?;
    (entry.get("subtype").and_then(|v| v.as_str()) == Some("init"))
        .then(|| entry.get("cwd").and_then(|v| v.as_str()).map(str::to_string))
        .flatten()
}

fn scan_line(scan: &mut ZoneScan, line: &[u8], start: i64, end: i64) {
    if line.iter().all(u8::is_ascii_whitespace) {
        return;
    }
    let Some(sid) = line_session_id(line) else {
        scan.unowned_from.get_or_insert(start);
        return;
    };
    let from = scan.unowned_from.take().unwrap_or(start);
    match scan.runs.last_mut() {
        Some(run) if run.session_id == sid && run.end == from => run.end = end,
        _ => scan.runs.push(Run { session_id: sid.clone(), start: from, end }),
    }
    if let Some(cwd) = init_cwd(line) {
        scan.cwd.entry(sid).or_insert(cwd);
    }
}

/// Read what was appended to `zone` since the last scan. A zone smaller than
/// what was read (archived and started over) is read again from the start.
fn scan_zone(store: &FileStore, zone: &str, scan: &mut ZoneScan) -> Result<(), String> {
    let size = match store.stat(zone, OUTPUT_FILE).map_err(|e| e.to_string())? {
        Some(file) => file.size,
        None => {
            *scan = ZoneScan::default();
            return Ok(());
        }
    };
    if size < scan.read_to {
        *scan = ZoneScan::default();
    }
    let from = scan.read_to;
    let mut last_complete = from;
    for_each_line(store, zone, from, size, |line, offset| {
        let end = offset + line.len() as i64 + 1;
        scan_line(scan, line, offset, end);
        last_complete = end;
    })?;
    scan.read_to = last_complete;
    Ok(())
}

/// Call `f(line, offset)` for every complete (newline-terminated) line of the
/// zone's `output` in `[start, end)`. A trailing unterminated line is left for
/// the next read — it's an append in progress.
fn for_each_line(
    store: &FileStore,
    zone: &str,
    start: i64,
    end: i64,
    mut f: impl FnMut(&[u8], i64),
) -> Result<(), String> {
    let mut pos = start;
    let mut carry: Vec<u8> = Vec::new();
    let mut carry_at = start;
    while pos < end {
        let (_, data) = store
            .read_at(zone, OUTPUT_FILE, pos, READ_CHUNK.min(end - pos))
            .map_err(|e| e.to_string())?;
        if data.is_empty() {
            break;
        }
        pos += data.len() as i64;
        carry.extend_from_slice(&data);
        let mut line_start = 0usize;
        while let Some(nl) = memchr::memchr(b'\n', &carry[line_start..]) {
            f(&carry[line_start..line_start + nl], carry_at + line_start as i64);
            line_start += nl + 1;
        }
        carry.drain(..line_start);
        carry_at += line_start as i64;
    }
    Ok(())
}

/// Receive times for line offsets from the zone's `output.tsidx`; `None` if
/// the zone has none.
fn line_stamps(store: &FileStore, zone: &str, offsets: &[u64]) -> Option<Vec<i64>> {
    let size = store.stat(zone, TSIDX_FILE).ok()??.size;
    let read = |off: i64, len: i64| store.read_at(zone, TSIDX_FILE, off, len).map(|(_, data)| Some(data));
    crate::backend::blockcontroller::shell::stamps::stamps_for(&read, size, offsets).ok()?
}

/// `(first, last)` receive time for each run: its first line and the line
/// holding its last byte.
fn zone_stamps(store: &FileStore, zone: &str, runs: &[Run]) -> Vec<(i64, i64)> {
    let mut points: Vec<u64> = runs
        .iter()
        .flat_map(|r| [r.start as u64, (r.end - 1).max(r.start) as u64])
        .collect();
    let original = points.clone();
    points.sort_unstable();
    points.dedup();
    let Some(stamps) = line_stamps(store, zone, &points) else {
        return vec![(0, 0); runs.len()];
    };
    let at: HashMap<u64, i64> = points.into_iter().zip(stamps).collect();
    original.chunks(2).map(|p| (at[&p[0]], at[&p[1]])).collect()
}

/// Record lines and zones for tests here and in the search tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    pub(crate) const UID: &str = "4d2a2b8c-1111-4222-8333-944455556666";

    pub(crate) fn store() -> Arc<FileStore> {
        Arc::new(FileStore::open_in_memory().unwrap())
    }

    pub(crate) fn append(fs: &FileStore, zone: &str, lines: &[String]) {
        if fs.stat(zone, OUTPUT_FILE).unwrap().is_none() {
            fs.make_file(zone, OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
        }
        let mut data = lines.join("\n");
        data.push('\n');
        fs.append_data(zone, OUTPUT_FILE, data.as_bytes()).unwrap();
    }

    pub(crate) fn stamp(fs: &FileStore, zone: &str, off: u64, ms: i64) {
        if fs.stat(zone, TSIDX_FILE).unwrap().is_none() {
            fs.make_file(zone, TSIDX_FILE, FileMeta::default(), FileOpts::default()).unwrap();
        }
        fs.append_data(zone, TSIDX_FILE, format!("{{\"off\":{off},\"ms\":{ms}}}\n").as_bytes()).unwrap();
    }

    pub(crate) fn init(sid: &str, cwd: &str) -> String {
        format!(r#"{{"type":"system","subtype":"init","cwd":"{cwd}","session_id":"{sid}","tools":[]}}"#)
    }
    pub(crate) fn assistant(sid: &str, text: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}},"parent_tool_use_id":null,"session_id":"{sid}"}}"#
        )
    }
    pub(crate) fn typed(text: &str) -> String {
        format!(r#"{{"type":"user","message":{{"role":"user","content":"{text}"}}}}"#)
    }
    pub(crate) fn delta(sid: &str) -> String {
        format!(
            r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"text":"has \"session_id\":\"not-me\" inside"}}}},"session_id":"{sid}","parent_tool_use_id":null}}"#
        )
    }

    pub(crate) fn zone() -> String {
        format!("agent:{UID}:current")
    }

}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    /// Sessions come from the `session_id` each line carries; the message a
    /// person typed (no id) belongs to the session that answered it.
    #[test]
    fn sessions_are_read_from_the_record_not_guessed() {
        let fs = store();
        append(&fs, &zone(), &[init("s1", "/work/a"), assistant("s1", "first answer"), delta("s1")]);
        append(&fs, &zone(), &[typed("second question"), init("s2", "/work/b"), assistant("s2", "second answer")]);
        let records = AgentRecords::new(Some(fs));

        let (sessions, problems) = records.sessions_of(UID);
        assert!(problems.is_empty(), "{problems:?}");
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, vec!["s2", "s1"], "newest first");
        assert_eq!(sessions[0].cwd.as_deref(), Some("/work/b"));
        assert_eq!(sessions[1].cwd.as_deref(), Some("/work/a"));

        let (s2, skipped) = records.read(&sessions[0]).unwrap();
        let texts: Vec<&str> = s2.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(texts, vec!["second question", "second answer"], "the typed message is s2's");
        assert_eq!(skipped, 0);
        let (s1, _) = records.read(&sessions[1]).unwrap();
        assert_eq!(s1.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), vec!["first answer"]);
    }

    /// A later search reads only what was appended; a session that continues
    /// keeps one range.
    #[test]
    fn a_rescan_picks_up_what_was_appended() {
        let fs = store();
        append(&fs, &zone(), &[init("s1", "/w"), assistant("s1", "one")]);
        let records = AgentRecords::new(Some(fs.clone()));
        assert_eq!(records.sessions_of(UID).0.len(), 1);

        append(&fs, &zone(), &[typed("more"), assistant("s1", "two"), init("s3", "/w"), assistant("s3", "three")]);
        let (sessions, _) = records.sessions_of(UID);
        let s1 = sessions.iter().find(|s| s.session_id == "s1").unwrap();
        assert_eq!(s1.ranges.len(), 1, "s1's lines are one run");
        let texts: Vec<String> = records.read(s1).unwrap().0.into_iter().map(|m| m.content).collect();
        assert_eq!(texts, vec!["one", "more", "two"]);
        assert!(sessions.iter().any(|s| s.session_id == "s3"));
    }

    /// "New conversation" archives the zone and starts it over, smaller: the
    /// next scan reads it from the start, and the archive is still the
    /// agent's history.
    #[test]
    fn an_archived_conversation_stays_and_the_new_one_is_read_afresh() {
        let fs = store();
        append(&fs, &zone(), &[init("old", "/w"), assistant("old", "before the archive, a long answer")]);
        let records = AgentRecords::new(Some(fs.clone()));
        assert_eq!(records.sessions_of(UID).0.len(), 1);

        let archive = format!("agent:{UID}:archive:1790000000000");
        let bytes = fs.read_file(&zone(), OUTPUT_FILE).unwrap().unwrap();
        fs.make_file(&archive, OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
        fs.append_data(&archive, OUTPUT_FILE, &bytes).unwrap();
        fs.delete_file(&zone(), OUTPUT_FILE).unwrap();
        append(&fs, &zone(), &[init("new", "/w")]);

        let (sessions, _) = records.sessions_of(UID);
        let mut ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["new", "old"]);
        let old = sessions.iter().find(|s| s.session_id == "old").unwrap();
        assert_eq!(old.ranges[0].0, archive);
    }

    /// Only the agent's own zones: never another agent's, never a template's
    /// shared one, whatever their names start with.
    #[test]
    fn only_the_agents_own_zones_are_read() {
        assert!(is_agents_own_zone(&format!("agent:{UID}:current"), UID));
        assert!(is_agents_own_zone(&format!("agent:{UID}:archive:1790000000000"), UID));
        assert!(!is_agents_own_zone("agent:claude:current", UID));
        assert!(!is_agents_own_zone(&format!("agent:{UID}0:current"), UID), "a longer id is another agent");
        assert!(!is_agents_own_zone(&format!("agent:{UID}:archive:"), UID));
        assert!(!is_agents_own_zone(&format!("agent-uid:{UID}:segments"), UID));
    }

    /// A line still being written stays unread until it's finished.
    #[test]
    fn a_half_written_line_waits_for_the_next_scan() {
        let fs = store();
        append(&fs, &zone(), &[init("s1", "/w")]);
        fs.append_data(&zone(), OUTPUT_FILE, br#"{"type":"assistant","message":{"con"#).unwrap();
        let records = AgentRecords::new(Some(fs.clone()));
        let (sessions, _) = records.sessions_of(UID);
        assert!(records.read(&sessions[0]).unwrap().0.is_empty());

        fs.append_data(
            &zone(),
            OUTPUT_FILE,
            b"tent\":[{\"type\":\"text\",\"text\":\"done\"}]},\"session_id\":\"s1\"}\n",
        )
        .unwrap();
        let (sessions, _) = records.sessions_of(UID);
        let texts: Vec<String> = records.read(&sessions[0]).unwrap().0.into_iter().map(|m| m.content).collect();
        assert_eq!(texts, vec!["done"]);
    }

    /// Messages carry the time AgentMux received their line.
    #[test]
    fn messages_are_dated_from_the_timestamp_index() {
        let fs = store();
        let first = init("s1", "/w");
        append(&fs, &zone(), &[first.clone()]);
        stamp(&fs, &zone(), 0, 1_000);
        let second_at = first.len() as u64 + 1;
        append(&fs, &zone(), &[assistant("s1", "answer")]);
        stamp(&fs, &zone(), second_at, 5_000);
        let records = AgentRecords::new(Some(fs));
        let (sessions, _) = records.sessions_of(UID);
        assert_eq!((sessions[0].first_ms, sessions[0].last_ms), (1_000, 5_000));
        let (messages, _) = records.read(&sessions[0]).unwrap();
        assert_eq!(messages[0].timestamp, 5_000);
    }

    /// A line that can't be parsed is counted, not skipped in silence.
    #[test]
    fn an_unreadable_record_line_is_counted() {
        let fs = store();
        append(&fs, &zone(), &[init("s1", "/w"), "{\"type\":\"assistant\",\"mess".into(), assistant("s1", "ok")]);
        let records = AgentRecords::new(Some(fs));
        let (sessions, _) = records.sessions_of(UID);
        let (messages, skipped) = records.read(&sessions[0]).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn no_store_reads_nothing() {
        let records = AgentRecords::new(None);
        assert_eq!(records.sessions_of(UID).0, Vec::new());
    }
}
