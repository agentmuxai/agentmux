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
    End {
        segment_id: String,
        ended_at_ms: i64,
        end_reason: EndReason,
        byte_end: Option<i64>,
        /// Size of the provider's session file when the process went away
        /// (spec §4.4). `None`: unknown, or a record from before the field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_bytes_end: Option<i64>,
    },
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
    /// The Anthropic identity `config_dir` was signed in as
    /// (`account_email::identity_key_from_oauth_dir`,
    /// SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.1).
    /// `None`: unknown, or a record written before this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_key: Option<String>,
    /// The session this spawn resumed with `--fork-session` after
    /// relocating it from another login of the same identity (spec §4.3).
    /// Its own id arrives in the segment's `session` event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
}

/// One segment, folded from its events.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    pub start: Start,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<EndReason>,
    pub byte_end: Option<i64>,
    pub provider_bytes_end: Option<i64>,
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
    record_end_with_provider_bytes(fs, agent_uid, segment_id, end_reason, byte_end, None, ended_at_ms)
}

/// [`record_end`], also recording the provider session file's size then
/// (spec §4.4), so a later spawn can tell whether something outside
/// AgentMux wrote to it since.
pub(crate) fn record_end_with_provider_bytes(
    fs: &FileStore,
    agent_uid: &str,
    segment_id: &str,
    end_reason: EndReason,
    byte_end: Option<i64>,
    provider_bytes_end: Option<i64>,
    ended_at_ms: i64,
) -> Result<(), String> {
    let zone = zone_for(agent_uid).ok_or("agent uid is not a valid zone key")?;
    append(fs, &zone, &Event::End { segment_id: segment_id.to_string(), ended_at_ms, end_reason, byte_end, provider_bytes_end })
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
                out.push(Segment { start, ended_at_ms: None, end_reason: None, byte_end: None, provider_bytes_end: None });
            }
            Event::Session { segment_id, provider_session_id, .. } => {
                if let Some(s) = out.iter_mut().rev().find(|s| s.start.segment_id == segment_id) {
                    s.start.provider_session_id = Some(provider_session_id);
                }
            }
            Event::End { segment_id, ended_at_ms, end_reason, byte_end, provider_bytes_end } => {
                // Closed exactly once: the first `end` wins.
                if let Some(s) = out.iter_mut().rev().find(|s| s.start.segment_id == segment_id && s.ended_at_ms.is_none()) {
                    s.ended_at_ms = Some(ended_at_ms);
                    s.end_reason = Some(end_reason);
                    s.byte_end = byte_end;
                    s.provider_bytes_end = provider_bytes_end;
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

/// What the resume gate decided for a session a spawn is about to `--resume`
/// (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResumeGate {
    /// Resume the candidate: it is the chain head, or the chain can't say
    /// otherwise.
    Allow,
    /// The conversation moved on to `head`, reachable here: resume that.
    Redirect { head: String },
    /// The conversation moved on to `head`, which this spawn can't reach:
    /// resume nothing, so the spawn starts fresh and carries the record.
    Refuse { head: String },
    /// `head` lives under another login of the same identity, readable at
    /// `from_config_dir`: copy it in and resume it with `--fork-session`
    /// (spec §4.3).
    Relocate { head: String, from_config_dir: String },
    /// `head` is reachable here, but its file grew after AgentMux last saw
    /// it: the conversation continued outside AgentMux. Resume it with
    /// `--fork-session`, so that branch is never appended to by a second
    /// writer (spec §4.4, H4).
    Fork { head: String },
}

/// What the gate weighs about the spawn.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GateInput<'a> {
    /// The session the spawn would `--resume`.
    pub candidate: &'a str,
    pub head: Option<&'a Head>,
    /// The spawn's identity key.
    pub identity: Option<&'a str>,
    /// An id this controller already saw the CLI reject.
    pub poisoned: Option<&'a str>,
    /// The spawn's provider config dir and working dir.
    pub config_dir: &'a str,
    pub cwd: &'a str,
}

/// Where the agent's conversation was last: the newest segment that has a
/// provider session, and what that segment recorded about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Head {
    pub session_id: String,
    pub identity_key: Option<String>,
    pub config_dir: Option<String>,
    pub provider: String,
    pub cwd: String,
    /// The provider file's size when the head's segment ended, when known.
    pub provider_bytes_end: Option<i64>,
}

/// Only the chain head is resumed natively (spec §3 I1), and never across
/// identities (I2). `input.head` is the agent's [`chain_head`]; `None` (an
/// agent from before the segment index, or no readable chain) allows the
/// candidate, as before the gate existed. A head equal to the poisoned id
/// allows too: that head is known dead, so the chain has nothing better to
/// offer than the candidate. The identity check refuses only when both
/// sides are known and differ.
///
/// `size_here(sid)`: the size of the session's file under the spawn's
/// config dir and cwd, `None` when it isn't there. `resumable_at(config_dir,
/// sid)`: a complete session file for the spawn's cwd exists under that
/// other config dir (§4.3).
pub(crate) fn resume_gate(
    input: &GateInput<'_>,
    size_here: impl Fn(&str) -> Option<u64>,
    resumable_at: impl Fn(&str, &str) -> bool,
) -> ResumeGate {
    let Some(head) = input.head else { return ResumeGate::Allow };
    if Some(head.session_id.as_str()) == input.poisoned {
        return ResumeGate::Allow;
    }
    let refuse = || ResumeGate::Refuse { head: head.session_id.clone() };
    if matches!((head.identity_key.as_deref(), input.identity), (Some(h), Some(s)) if h != s) {
        return refuse();
    }
    let is_candidate = head.session_id == input.candidate;
    if let Some(size) = size_here(&head.session_id) {
        if head.provider_bytes_end.is_some_and(|end| i64::try_from(size).unwrap_or(i64::MAX) > end) {
            return ResumeGate::Fork { head: head.session_id.clone() };
        }
        return if is_candidate { ResumeGate::Allow } else { ResumeGate::Redirect { head: head.session_id.clone() } };
    }
    if let Some(from) = relocation_source(head, input) {
        if resumable_at(from, &head.session_id) {
            return ResumeGate::Relocate { head: head.session_id.clone(), from_config_dir: from.to_string() };
        }
    }
    // The head is out of reach and can't be relocated. As the candidate, it
    // goes on to `--resume` as before the gate: the CLI's rejection and the
    // retry path already handle it. As anything else, nothing is resumed.
    if is_candidate {
        ResumeGate::Allow
    } else {
        refuse()
    }
}

/// The head's config dir when §4.3's metadata preconditions hold: a Claude
/// session, under another config dir than the spawn's, recorded under the
/// spawn's own identity (both known, I2), for the same project dir.
fn relocation_source<'h>(head: &'h Head, input: &GateInput<'_>) -> Option<&'h str> {
    if !matches!(head.provider.as_str(), "claude" | "claude-code") {
        return None;
    }
    let from = head.config_dir.as_deref()?;
    if same_dir(from, input.config_dir) {
        return None;
    }
    if head.identity_key.as_deref()? != input.identity? {
        return None;
    }
    let slug = crate::backend::claude_layout::project_dir_name;
    (slug(&head.cwd) == slug(input.cwd)).then_some(from)
}

fn same_dir(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

/// A head whose segment predates `identity_key` (recorded by a build
/// before #3839) takes the identity from its own config dir's
/// `.claude.json`, so an agent's first upgrade from such a build can still
/// relocate (spec §4.1). That file names whoever is signed in there now,
/// which for an account's own dir is the login the session ran under unless
/// the dir was since re-logged-into as someone else; a recorded key always
/// wins. Unreadable (the dir is gone) stays unknown, which never relocates.
pub(crate) fn with_legacy_identity(mut head: Head) -> Head {
    if head.identity_key.is_none() {
        if let Some(dir) = head.config_dir.as_deref() {
            head.identity_key = crate::identity::account_email::identity_key_from_oauth_dir(&head.provider, dir);
        }
    }
    head
}

/// Where the agent's conversation was last, read from its chain in `fs`.
/// `None` when the UID has no chain.
pub(crate) fn chain_head(fs: &FileStore, agent_uid: &str) -> Option<Head> {
    segments(fs, agent_uid).into_iter().rev().find_map(|s| {
        let session_id = s.start.provider_session_id?;
        Some(Head {
            session_id,
            identity_key: s.start.identity_key,
            config_dir: s.start.config_dir,
            provider: s.start.provider,
            cwd: s.start.cwd,
            provider_bytes_end: s.provider_bytes_end,
        })
    })
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
            identity_key: None,
            forked_from: None,
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

    // ── resume_gate (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION §4.2) ──

    fn head(sid: &str, identity: Option<&str>) -> Head {
        Head { session_id: sid.into(), identity_key: identity.map(str::to_string), ..Default::default() }
    }

    /// The gate over a spawn in `C:/cfg/new` for `C:/work`, where `here` are
    /// the sessions under the spawn's config dir and `elsewhere` the ones
    /// complete under any other.
    fn gate(candidate: &str, head: Option<&Head>, identity: Option<&str>, poisoned: Option<&str>, here: &[&str], elsewhere: &[&str]) -> ResumeGate {
        let input = GateInput { candidate, head, identity, poisoned, config_dir: "C:/cfg/new", cwd: "C:/work" };
        resume_gate(&input, |sid| here.contains(&sid).then_some(10), |_, sid| elsewhere.contains(&sid))
    }

    #[test]
    fn the_chain_head_itself_is_resumed() {
        let h = head("s2", None);
        assert_eq!(gate("s2", Some(&h), None, None, &["s2"], &[]), ResumeGate::Allow);
    }

    #[test]
    fn without_a_chain_the_candidate_is_resumed_as_before() {
        assert_eq!(gate("s1", None, Some("k1"), None, &[], &[]), ResumeGate::Allow);
    }

    /// The operator's case: the pane holds S1, the conversation moved on to S2.
    #[test]
    fn a_stale_candidate_is_redirected_to_a_reachable_head() {
        let h = head("s2", None);
        assert_eq!(gate("s1", Some(&h), None, None, &["s1", "s2"], &[]), ResumeGate::Redirect { head: "s2".into() });
    }

    #[test]
    fn a_stale_candidate_is_refused_when_the_head_is_out_of_reach() {
        let h = head("s2", None);
        assert_eq!(gate("s1", Some(&h), None, None, &["s1"], &[]), ResumeGate::Refuse { head: "s2".into() });
    }

    /// The head as the candidate but out of reach, and not relocatable: it
    /// goes to `--resume` as before the gate; the CLI's rejection and the
    /// retry path handle it.
    #[test]
    fn an_unreachable_head_as_the_candidate_is_left_to_the_retry_path() {
        let h = head("s2", None);
        assert_eq!(gate("s2", Some(&h), None, None, &[], &[]), ResumeGate::Allow);
    }

    #[test]
    fn a_poisoned_head_never_displaces_the_candidate() {
        let h = head("s2", None);
        assert_eq!(gate("s1", Some(&h), None, Some("s2"), &["s2"], &[]), ResumeGate::Allow);
    }

    // Identity (spec §3 I2, H3): never resume across identities.

    /// The config dir was re-logged-into as someone else: the head's file is
    /// here, but it belongs to another identity.
    #[test]
    fn the_head_under_another_identity_is_not_resumed() {
        let h = head("s2", Some("k-old"));
        assert_eq!(gate("s2", Some(&h), Some("k-new"), None, &["s2"], &[]), ResumeGate::Refuse { head: "s2".into() });
        assert_eq!(gate("s1", Some(&h), Some("k-new"), None, &["s2"], &[]), ResumeGate::Refuse { head: "s2".into() });
    }

    #[test]
    fn the_same_identity_resumes_and_redirects() {
        let h = head("s2", Some("k1"));
        assert_eq!(gate("s2", Some(&h), Some("k1"), None, &["s2"], &[]), ResumeGate::Allow);
        assert_eq!(gate("s1", Some(&h), Some("k1"), None, &["s2"], &[]), ResumeGate::Redirect { head: "s2".into() });
    }

    /// Either side unknown (an older record, an unreadable `.claude.json`):
    /// no information, so the gate behaves as it did before identities.
    #[test]
    fn an_unknown_identity_on_either_side_changes_nothing() {
        let known = head("s2", Some("k1"));
        let unknown = head("s2", None);
        assert_eq!(gate("s2", Some(&known), None, None, &["s2"], &[]), ResumeGate::Allow);
        assert_eq!(gate("s2", Some(&unknown), Some("k1"), None, &["s2"], &[]), ResumeGate::Allow);
        assert_eq!(gate("s1", Some(&known), None, None, &["s2"], &[]), ResumeGate::Redirect { head: "s2".into() });
    }

    // Relocation (spec §4.3): the head under another login of the same identity.

    fn elsewhere(sid: &str, identity: Option<&str>) -> Head {
        Head {
            session_id: sid.into(),
            identity_key: identity.map(str::to_string),
            config_dir: Some("C:/cfg/old".into()),
            provider: "claude".into(),
            cwd: "C:/work".into(),
            provider_bytes_end: None,
        }
    }

    /// A rebuild: new login, same person. The pane holds the head, whose file
    /// is only under the old login.
    #[test]
    fn the_head_under_another_login_of_the_same_identity_is_relocated() {
        let h = elsewhere("s2", Some("k1"));
        let relocate = ResumeGate::Relocate { head: "s2".into(), from_config_dir: "C:/cfg/old".into() };
        assert_eq!(gate("s2", Some(&h), Some("k1"), None, &[], &["s2"]), relocate);
        assert_eq!(gate("s1", Some(&h), Some("k1"), None, &["s1"], &["s2"]), relocate, "a stale candidate too");
    }

    #[test]
    fn relocation_needs_every_precondition() {
        let base = elsewhere("s2", Some("k1"));
        let not_relocated = |h: &Head, identity: Option<&str>, elsewhere: &[&str]| {
            !matches!(gate("s1", Some(h), identity, None, &[], elsewhere), ResumeGate::Relocate { .. })
        };
        assert!(not_relocated(&base, Some("k1"), &[]), "no complete source file");
        assert!(not_relocated(&base, None, &["s2"]), "spawn identity unknown");
        assert!(not_relocated(&Head { identity_key: None, ..base.clone() }, Some("k1"), &["s2"]), "head identity unknown");
        assert!(not_relocated(&Head { provider: "codex".into(), ..base.clone() }, Some("k1"), &["s2"]), "not Claude");
        assert!(not_relocated(&Head { config_dir: None, ..base.clone() }, Some("k1"), &["s2"]), "no recorded dir");
        assert!(not_relocated(&Head { config_dir: Some("c:\\cfg\\new\\".into()), ..base.clone() }, Some("k1"), &["s2"]), "the same dir");
        assert!(not_relocated(&Head { cwd: "C:/elsewhere".into(), ..base.clone() }, Some("k1"), &["s2"]), "another project");
        assert_eq!(
            gate("s1", Some(&Head { identity_key: Some("k2".into()), ..base }), Some("k1"), None, &[], &["s2"]),
            ResumeGate::Refuse { head: "s2".into() },
            "another identity is refused outright"
        );
    }

    // Continued outside AgentMux (spec §4.4, H4).

    /// The head's file is larger than when its segment ended: something
    /// outside AgentMux (a terminal `claude --resume`) continued it.
    #[test]
    fn a_head_that_grew_outside_agentmux_is_forked() {
        let h = Head { provider_bytes_end: Some(9), ..head("s2", None) };
        assert_eq!(gate("s2", Some(&h), None, None, &["s2"], &[]), ResumeGate::Fork { head: "s2".into() });
        assert_eq!(gate("s1", Some(&h), None, None, &["s2"], &[]), ResumeGate::Fork { head: "s2".into() });
    }

    #[test]
    fn a_head_as_agentmux_left_it_resumes_in_place() {
        let same = Head { provider_bytes_end: Some(10), ..head("s2", None) };
        let unknown = Head { provider_bytes_end: None, ..head("s2", None) };
        assert_eq!(gate("s2", Some(&same), None, None, &["s2"], &[]), ResumeGate::Allow);
        assert_eq!(gate("s2", Some(&unknown), None, None, &["s2"], &[]), ResumeGate::Allow);
    }

    #[test]
    fn a_segment_end_records_the_provider_file_size_and_older_ends_still_read() {
        let fs = FileStore::open_in_memory().unwrap();
        let a = record_start(&fs, start(1_000, Rung::Fresh)).unwrap();
        record_session(&fs, UID, &a, "sid-a", 1_001).unwrap();
        record_end_with_provider_bytes(&fs, UID, &a, EndReason::Exited, None, Some(4_096), 2_000).unwrap();
        assert_eq!(chain_head(&fs, UID).unwrap().provider_bytes_end, Some(4_096));

        let old = serde_json::json!({"event": "end", "segment_id": "s", "ended_at_ms": 5, "end_reason": "exited", "byte_end": null});
        match serde_json::from_value::<Event>(old).unwrap() {
            Event::End { provider_bytes_end, .. } => assert_eq!(provider_bytes_end, None),
            other => panic!("expected an end event, got {other:?}"),
        }
    }

    // A head recorded before identity keys existed (spec §4.1).

    fn login_dir(account: &str, org: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(".claude.json"),
            format!(r#"{{"oauthAccount":{{"accountUuid":"{account}","organizationUuid":"{org}"}}}}"#),
        )
        .unwrap();
        d
    }

    #[test]
    fn a_legacy_head_takes_its_identity_from_its_own_config_dir() {
        let dir = login_dir("acc-1", "org-1");
        let path = dir.path().to_string_lossy().to_string();
        let legacy = Head { config_dir: Some(path.clone()), provider: "claude".into(), ..head("s2", None) };
        let expected = crate::identity::account_email::identity_key_from_oauth_dir("claude", &path);
        assert!(expected.is_some());
        assert_eq!(with_legacy_identity(legacy).identity_key, expected);
    }

    #[test]
    fn a_recorded_identity_always_wins_and_an_unreadable_dir_stays_unknown() {
        let dir = login_dir("acc-1", "org-1");
        let path = dir.path().to_string_lossy().to_string();
        let recorded = Head { config_dir: Some(path), provider: "claude".into(), ..head("s2", Some("k-recorded")) };
        assert_eq!(with_legacy_identity(recorded).identity_key.as_deref(), Some("k-recorded"));
        let gone = Head { config_dir: Some("/no/such/dir".into()), provider: "claude".into(), ..head("s2", None) };
        assert_eq!(with_legacy_identity(gone).identity_key, None);
        let no_dir = Head { provider: "claude".into(), ..head("s2", None) };
        assert_eq!(with_legacy_identity(no_dir).identity_key, None);
    }

    #[test]
    fn the_chain_head_is_the_newest_segment_with_a_session() {
        let fs = FileStore::open_in_memory().unwrap();
        assert_eq!(chain_head(&fs, UID), None);
        let a = record_start(&fs, start(1_000, Rung::Fresh)).unwrap();
        record_session(&fs, UID, &a, "sid-a", 1_001).unwrap();
        let b = record_start(&fs, Start { identity_key: Some("k1".into()), ..start(2_000, Rung::Fresh) }).unwrap();
        record_session(&fs, UID, &b, "sid-b", 2_001).unwrap();
        // A spawn that died before reporting a session doesn't move the head.
        record_start(&fs, start(3_000, Rung::Fresh)).unwrap();
        let h = chain_head(&fs, UID).expect("a head");
        assert_eq!(h.session_id, "sid-b");
        assert_eq!(h.identity_key.as_deref(), Some("k1"));
        assert_eq!(h.config_dir.as_deref(), Some("C:/cfg/a"));
        assert_eq!(h.provider, "claude");
    }

    /// A record written before `identity_key` existed still reads, as `None`.
    #[test]
    fn older_records_without_an_identity_still_read() {
        let old = serde_json::json!({
            "event": "start", "segment_id": "s", "agent_uid": UID, "definition_id": null, "provider": "claude",
            "config_dir": null, "provider_session_id": "sid", "cwd": "", "channel": "stable",
            "agentmux_version": "0.57.5", "block_id": "b", "zone": null, "byte_start": null,
            "started_at_ms": 1, "continuity_rung": "native", "predecessor_segment_id": null, "lease_epoch": 2,
        });
        match serde_json::from_value::<Event>(old).unwrap() {
            Event::Start(s) => assert_eq!(s.identity_key, None),
            other => panic!("expected a start event, got {other:?}"),
        }
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
        let e = Event::End { segment_id: "s".into(), ended_at_ms: 5, end_reason: EndReason::Restarted, byte_end: None, provider_bytes_end: None };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"end\"") && json.contains("\"end_reason\":\"restarted\""), "{json}");
    }

    fn seg(id: &str, sid: Option<&str>) -> Segment {
        let mut s = start(0, Rung::Fresh);
        s.segment_id = id.into();
        s.provider_session_id = sid.map(str::to_string);
        Segment { start: s, ended_at_ms: None, end_reason: None, byte_end: None, provider_bytes_end: None }
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
