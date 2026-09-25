// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Segment index (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.1, P1):
//! one record per provider session an agent has run, so "this agent's
//! conversation" is a chain AgentMux owns rather than whatever session id a
//! pane happens to hold.
//!
//! Stored as an append-only event log, `segments.jsonl` in the global
//! transcript store's `agent-uid:<uid>:segments` zone, not as a SQL table
//! (§4.1.1): that store is shared by every srv on the machine and takes
//! additions without a schema version bump, where a new table in
//! `identity-store.db` would bump its version and lock older builds out of
//! it (`check_schema_compat`). Events: `start` at spawn, `session` when a
//! fresh process reports its provider session id, `end` when the process
//! goes away. [`segments`] folds them back into one row per segment.
//!
//! The zone is keyed by the agent's UID (`AGENTMUX_AGENT_UID`), not its
//! definition id: a template launch shares the template's transcript zone
//! but is a different agent. It is deliberately not the transcript zone, so
//! "New conversation" (which clears `agent:<defId>:current`) never erases the
//! chain; a new conversation is just a segment whose rung is `fresh`.

use serde::{Deserialize, Serialize};

use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

pub(crate) const SEGMENTS_FILE: &str = "segments.jsonl";
/// Enough of the log's end to find the newest segment for a predecessor link
/// without reading an agent's whole history.
const TAIL_WINDOW_BYTES: i64 = 64 * 1024;

/// How a segment began (§4.2's rungs; `relocated` arrives with P4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Rung {
    /// The provider resumed the previous session.
    Native,
    /// A fresh provider session, given AgentMux's record of the conversation.
    Virtualized,
    /// A fresh provider session with nothing carried over.
    Fresh,
}

/// Why a segment's process went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndReason {
    /// Exited on its own with status 0.
    Exited,
    /// Exited on its own with a non-zero status.
    Crashed,
    /// The pane was closed.
    Closed,
    /// Stopped to apply a runtime-config change; the next spawn resumes it.
    Restarted,
    /// Stopped for any other reason (user stop, controller replaced).
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Start(Start),
    Session { segment_id: String, provider_session_id: String, at_ms: i64 },
    End { segment_id: String, ended_at_ms: i64, end_reason: EndReason, byte_end: Option<i64> },
}

/// What is known about a segment when its process starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Start {
    pub segment_id: String,
    pub agent_uid: String,
    /// The block's `agentId`: the transcript zone's key, which differs from
    /// `agent_uid` for a template launch.
    pub definition_id: Option<String>,
    pub provider: String,
    /// Where native resume would have to look (`CLAUDE_CONFIG_DIR`, …).
    pub config_dir: Option<String>,
    /// Known up front on a resume; otherwise filled in by a `session` event.
    pub provider_session_id: Option<String>,
    pub cwd: String,
    pub channel: String,
    pub agentmux_version: String,
    pub block_id: String,
    /// The global transcript zone this segment appends to, and its size then.
    pub zone: Option<String>,
    pub byte_start: Option<i64>,
    pub started_at_ms: i64,
    pub continuity_rung: Rung,
    pub predecessor_segment_id: Option<String>,
    /// The single-live-instance lease epoch this segment's process held
    /// (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 Phase 3). A change between
    /// consecutive segments is an ownership change — another instance, or a
    /// takeover — visible in the ledger. `None`: no lease (older build, no
    /// UID) or a record written before this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_epoch: Option<u64>,
}

/// One segment, folded from its events.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    pub start: Start,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<EndReason>,
    pub byte_end: Option<i64>,
}

/// `agent-uid:<uid>:segments`, or `None` for a UID outside the safe zone-name
/// character set.
pub(crate) fn zone_for(agent_uid: &str) -> Option<String> {
    crate::backend::agent_session::is_valid_definition_id(agent_uid).then(|| format!("agent-uid:{agent_uid}:segments"))
}

fn append(fs: &FileStore, zone: &str, event: &Event) -> Result<(), String> {
    let mut line = serde_json::to_vec(event).map_err(|e| format!("encode segment event: {e}"))?;
    line.push(b'\n');
    if fs.stat(zone, SEGMENTS_FILE).map_err(|e| format!("stat segments: {e}"))?.is_none() {
        // Two srvs can race to create it; losing that race is fine.
        let _ = fs.make_file(zone, SEGMENTS_FILE, FileMeta::default(), FileOpts::default());
    }
    fs.append_data(zone, SEGMENTS_FILE, &line).map_err(|e| format!("append segment event: {e}"))
}

fn events_in(bytes: &[u8]) -> impl Iterator<Item = Event> + '_ {
    bytes.split(|b| *b == b'\n').filter_map(|line| serde_json::from_slice::<Event>(line).ok())
}

/// The id of the agent's newest segment, from the end of its log.
fn newest_segment_id(fs: &FileStore, zone: &str) -> Option<String> {
    let file = fs.stat(zone, SEGMENTS_FILE).ok()??;
    let offset = (file.size - TAIL_WINDOW_BYTES).max(0);
    let (_, data) = fs.read_at(zone, SEGMENTS_FILE, offset, file.size - offset).ok()?;
    let mut newest: Option<(i64, String)> = None;
    for event in events_in(&data) {
        if let Event::Start(s) = event {
            if newest.as_ref().is_none_or(|(at, _)| s.started_at_ms >= *at) {
                newest = Some((s.started_at_ms, s.segment_id));
            }
        }
    }
    newest.map(|(_, id)| id)
}

/// Records a segment's start and returns its id. `start.segment_id` and
/// `start.predecessor_segment_id` are filled in here.
pub(crate) fn record_start(fs: &FileStore, mut start: Start) -> Result<String, String> {
    let zone = zone_for(&start.agent_uid).ok_or("agent uid is not a valid zone key")?;
    start.segment_id = uuid::Uuid::new_v4().to_string();
    start.predecessor_segment_id = newest_segment_id(fs, &zone);
    let id = start.segment_id.clone();
    append(fs, &zone, &Event::Start(start))?;
    Ok(id)
}

pub(crate) fn record_session(fs: &FileStore, agent_uid: &str, segment_id: &str, provider_session_id: &str, at_ms: i64) -> Result<(), String> {
    let zone = zone_for(agent_uid).ok_or("agent uid is not a valid zone key")?;
    append(
        fs,
        &zone,
        &Event::Session { segment_id: segment_id.to_string(), provider_session_id: provider_session_id.to_string(), at_ms },
    )
}

pub(crate) fn record_end(
    fs: &FileStore,
    agent_uid: &str,
    segment_id: &str,
    end_reason: EndReason,
    byte_end: Option<i64>,
    ended_at_ms: i64,
) -> Result<(), String> {
    let zone = zone_for(agent_uid).ok_or("agent uid is not a valid zone key")?;
    append(fs, &zone, &Event::End { segment_id: segment_id.to_string(), ended_at_ms, end_reason, byte_end })
}

/// Every segment the agent has run, oldest first. A segment with no `end` is
/// running, or its srv died before it could record one.
pub(crate) fn segments(fs: &FileStore, agent_uid: &str) -> Vec<Segment> {
    let Some(zone) = zone_for(agent_uid) else { return Vec::new() };
    let Ok(Some(data)) = fs.read_file(&zone, SEGMENTS_FILE) else { return Vec::new() };
    fold(events_in(&data))
}

fn fold(events: impl Iterator<Item = Event>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for event in events {
        match event {
            Event::Start(start) => {
                out.push(Segment { start, ended_at_ms: None, end_reason: None, byte_end: None });
            }
            Event::Session { segment_id, provider_session_id, .. } => {
                if let Some(s) = out.iter_mut().rev().find(|s| s.start.segment_id == segment_id) {
                    s.start.provider_session_id = Some(provider_session_id);
                }
            }
            Event::End { segment_id, ended_at_ms, end_reason, byte_end } => {
                // Closed exactly once: the first `end` wins.
                if let Some(s) = out.iter_mut().rev().find(|s| s.start.segment_id == segment_id && s.ended_at_ms.is_none()) {
                    s.ended_at_ms = Some(ended_at_ms);
                    s.end_reason = Some(end_reason);
                    s.byte_end = byte_end;
                }
            }
        }
    }
    out.sort_by_key(|s| s.start.started_at_ms);
    out
}

/// The provider session the agent's conversation was last in: the session
/// of the newest segment, other than `current` (the spawn now running or
/// just failed), that has one. `None` when the agent has no such segment.
pub(crate) fn head_session(segments: &[Segment], current: Option<&str>) -> Option<String> {
    segments
        .iter()
        .rev()
        .filter(|s| Some(s.start.segment_id.as_str()) != current)
        .find_map(|s| s.start.provider_session_id.clone())
}

/// Whether a resume-failure recovery may continue `candidate` (§4.2: only
/// the head of the chain is this conversation). With a chain, any other
/// session is older or unrelated history. It's typically the same agent's
/// session from the last time it ran under this account, and resuming it
/// would show "Resumed" on the wrong conversation, so the caller falls
/// through to a fresh session carrying the continuation packet. With no
/// known head (an agent from before the segment index, whose only segment is
/// the current one), nothing better is known and recovery works as before.
pub(crate) fn recovery_allowed(candidate: &str, head: Option<&str>) -> bool {
    head.is_none_or(|h| h == candidate)
}

/// The rung a spawn started on, from what the spawn knows: whether it passed
/// `--resume`, and whether it carries a continuation packet.
pub(crate) fn rung_for_spawn(resumed: bool, carries_packet: bool) -> Rung {
    if resumed {
        Rung::Native
    } else if carries_packet {
        Rung::Virtualized
    } else {
        Rung::Fresh
    }
}

/// Why a killed process went away, from the controller's state at the kill.
pub(crate) fn kill_reason(closing_this_generation: bool, restart_pending: bool) -> EndReason {
    if closing_this_generation {
        EndReason::Closed
    } else if restart_pending {
        EndReason::Restarted
    } else {
        EndReason::Stopped
    }
}

pub(crate) fn exit_reason(exit_code: i32) -> EndReason {
    if exit_code == 0 { EndReason::Exited } else { EndReason::Crashed }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: &str = "uid-1";

    fn start(at: i64, rung: Rung) -> Start {
        Start {
            segment_id: String::new(),
            agent_uid: UID.into(),
            definition_id: Some("def-1".into()),
            provider: "claude".into(),
            config_dir: Some("C:/cfg/a".into()),
            provider_session_id: None,
            cwd: "C:/work".into(),
            channel: "stable".into(),
            agentmux_version: "0.57.2".into(),
            block_id: "block-1".into(),
            zone: Some("agent:def-1:current".into()),
            byte_start: Some(100),
            started_at_ms: at,
            continuity_rung: rung,
            predecessor_segment_id: None,
            lease_epoch: None,
        }
    }

    /// SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 Phase 3: each segment
    /// records the lease epoch its process held, so an ownership change
    /// between segments is visible in the ledger; a record written before
    /// the field existed still reads, as `None`.
    #[test]
    fn segments_record_the_lease_epoch_and_older_records_still_read() {
        let fs = FileStore::open_in_memory().unwrap();
        let first = record_start(&fs, Start { lease_epoch: Some(3), ..start(1_000, Rung::Fresh) }).unwrap();
        record_start(&fs, Start { lease_epoch: Some(4), ..start(2_000, Rung::Native) }).unwrap();
        let all = segments(&fs, UID);
        assert_eq!(all.iter().map(|s| s.start.lease_epoch).collect::<Vec<_>>(), vec![Some(3), Some(4)]);
        assert_eq!(all[0].start.segment_id, first);

        let old = serde_json::json!({
            "event": "start", "segment_id": "s", "agent_uid": UID, "definition_id": null, "provider": "claude",
            "config_dir": null, "provider_session_id": null, "cwd": "", "channel": "stable",
            "agentmux_version": "0.57.0", "block_id": "b", "zone": null, "byte_start": null,
            "started_at_ms": 1, "continuity_rung": "fresh", "predecessor_segment_id": null,
        });
        match serde_json::from_value::<Event>(old).unwrap() {
            Event::Start(s) => assert_eq!(s.lease_epoch, None),
            other => panic!("expected a start event, got {other:?}"),
        }
    }

    #[test]
    fn segments_chain_to_their_predecessor_and_fold_their_events() {
        let fs = FileStore::open_in_memory().unwrap();
        let first = record_start(&fs, start(1_000, Rung::Fresh)).unwrap();
        record_session(&fs, UID, &first, "sid-a", 1_001).unwrap();
        record_end(&fs, UID, &first, EndReason::Crashed, Some(900), 2_000).unwrap();
        let second = record_start(&fs, start(3_000, Rung::Virtualized)).unwrap();

        let all = segments(&fs, UID);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].start.segment_id, first);
        assert_eq!(all[0].start.provider_session_id.as_deref(), Some("sid-a"));
        assert_eq!((all[0].ended_at_ms, all[0].end_reason, all[0].byte_end), (Some(2_000), Some(EndReason::Crashed), Some(900)));
        assert_eq!(all[0].start.predecessor_segment_id, None);
        assert_eq!(all[1].start.predecessor_segment_id.as_deref(), Some(first.as_str()));
        assert_eq!(all[1].start.continuity_rung, Rung::Virtualized);
        assert!(all[1].ended_at_ms.is_none(), "still running");
        assert_ne!(first, second);
    }

    #[test]
    fn a_segment_is_closed_exactly_once() {
        let fs = FileStore::open_in_memory().unwrap();
        let id = record_start(&fs, start(1, Rung::Fresh)).unwrap();
        record_end(&fs, UID, &id, EndReason::Closed, None, 10).unwrap();
        record_end(&fs, UID, &id, EndReason::Crashed, None, 20).unwrap();
        let s = &segments(&fs, UID)[0];
        assert_eq!((s.ended_at_ms, s.end_reason), (Some(10), Some(EndReason::Closed)));
    }

    #[test]
    fn agents_have_separate_chains() {
        let fs = FileStore::open_in_memory().unwrap();
        record_start(&fs, start(1, Rung::Fresh)).unwrap();
        let mut other = start(2, Rung::Fresh);
        other.agent_uid = "uid-2".into();
        let id = record_start(&fs, other).unwrap();
        assert_eq!(segments(&fs, UID).len(), 1);
        let theirs = segments(&fs, "uid-2");
        assert_eq!(theirs[0].start.segment_id, id);
        assert_eq!(theirs[0].start.predecessor_segment_id, None, "another agent's segment is not a predecessor");
    }

    #[test]
    fn a_corrupt_line_is_skipped() {
        let fs = FileStore::open_in_memory().unwrap();
        let id = record_start(&fs, start(1, Rung::Fresh)).unwrap();
        fs.append_data("agent-uid:uid-1:segments", SEGMENTS_FILE, b"{\"event\":\"start\",\"trunc\n").unwrap();
        let next = record_start(&fs, start(2, Rung::Native)).unwrap();
        let all = segments(&fs, UID);
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].start.segment_id, next);
        assert_eq!(all[1].start.predecessor_segment_id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn an_unsafe_uid_is_refused() {
        let fs = FileStore::open_in_memory().unwrap();
        let mut bad = start(1, Rung::Fresh);
        bad.agent_uid = "../x:y".into();
        assert!(record_start(&fs, bad).is_err());
        assert!(segments(&fs, "../x:y").is_empty());
    }

    #[test]
    fn events_are_tagged_json_lines() {
        let e = Event::End { segment_id: "s".into(), ended_at_ms: 5, end_reason: EndReason::Restarted, byte_end: None };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"end\"") && json.contains("\"end_reason\":\"restarted\""), "{json}");
    }

    fn seg(id: &str, sid: Option<&str>) -> Segment {
        let mut s = start(0, Rung::Fresh);
        s.segment_id = id.into();
        s.provider_session_id = sid.map(str::to_string);
        Segment { start: s, ended_at_ms: None, end_reason: None, byte_end: None }
    }

    #[test]
    fn the_head_is_the_newest_earlier_segment_with_a_session() {
        let chain = [seg("a", Some("sid-old")), seg("b", Some("sid-head")), seg("c", None), seg("now", Some("sid-attempted"))];
        assert_eq!(head_session(&chain, Some("now")).as_deref(), Some("sid-head"));
        assert_eq!(head_session(&chain, None).as_deref(), Some("sid-attempted"));
        assert_eq!(head_session(&[], Some("now")), None);
    }

    /// The account-switch case: the head (the old account's session) was
    /// just rejected, and the new account's config dir holds an older
    /// session of the same agent. Recovering it would resume the wrong
    /// conversation.
    #[test]
    fn recovery_only_continues_the_head_of_the_chain() {
        assert!(!recovery_allowed("sid-old", Some("sid-head")));
        assert!(recovery_allowed("sid-head", Some("sid-head")), "a stale pane id whose real session is the head");
        assert!(recovery_allowed("sid-any", None), "no known head (pre-index agent): behave as before the index");
    }

    #[test]
    fn rungs_and_reasons() {
        assert_eq!(rung_for_spawn(true, false), Rung::Native);
        assert_eq!(rung_for_spawn(true, true), Rung::Native, "a resume never carries a packet, but resume wins");
        assert_eq!(rung_for_spawn(false, true), Rung::Virtualized);
        assert_eq!(rung_for_spawn(false, false), Rung::Fresh);
        assert_eq!(kill_reason(true, true), EndReason::Closed);
        assert_eq!(kill_reason(false, true), EndReason::Restarted);
        assert_eq!(kill_reason(false, false), EndReason::Stopped);
        assert_eq!(exit_reason(0), EndReason::Exited);
        assert_eq!(exit_reason(1), EndReason::Crashed);
        assert_eq!(exit_reason(-1), EndReason::Crashed);
    }
}
