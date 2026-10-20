// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fork's history. A fork (the pane's Quick Fork, the picker's Fork) is an
//! exact copy of its agent, conversation included: its CLI continues the
//! parent's conversation with `--fork-session`, and its pane shows the
//! parent's history. A worker (the `Agents` `spawn` action) is not a fork and
//! never comes here. See "Fork, spawn and worker" in
//! `server/agent_handlers/mod.rs`.
//!
//! The fork's transcript zone starts as a copy of the parent's, made in the
//! global transcript store when the fork is launched. Beside it, [`FORK_MARK_FILE`]
//! names the parent's session the copy ends in: a spawn that resumes that
//! session from the fork forks it (`persistent/spawn.rs`), so the fork never
//! writes into its parent's conversation.

use crate::backend::storage::filestore::FileStore;

use super::helpers::{now_ms, write_zone_file};
use super::session_io::{OUTPUT_FILE, SNAPSHOT_FILE, TSIDX_FILE};
use super::zone_naming::{agent_current_zone, is_valid_definition_id};

/// `{"from":<parent definition>,"session_id":<parent session>,"at_ms":..}`
/// in the fork's zone.
pub const FORK_MARK_FILE: &str = "fork.json";

/// How far back from the end of the parent's transcript the session it ends
/// in is looked for. Every stream event carries the session id.
const LAST_SESSION_TAIL_BYTES: i64 = 256 * 1024;

/// What [`copy_parent_history`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkHistory {
    /// The parent's transcript is the fork's, `bytes` long.
    Copied { bytes: u64 },
    /// The parent has no transcript to copy.
    NoParentHistory,
    /// The parent's transcript doesn't end in the session being forked (a
    /// fork of an older conversation): the fork's CLI continues that one, so
    /// showing the parent's later history would be wrong. Nothing copied here;
    /// [`copy_older_session_history`] copies the history up to where that
    /// session ends.
    NotTheLastSession,
    /// The fork already has history of its own. Nothing copied.
    ForkHasHistory,
}

/// Copy the parent's transcript into the fork's zone in `gfs`, when the
/// fork continues `session_id`, the session the parent's transcript ends in.
/// `session_id_field` is the provider's session field in its stream events.
pub fn copy_parent_history(
    gfs: &FileStore,
    parent_id: &str,
    fork_id: &str,
    session_id: &str,
    session_id_field: &str,
) -> Result<ForkHistory, String> {
    if !is_valid_definition_id(parent_id) || !is_valid_definition_id(fork_id) || parent_id == fork_id {
        return Err(format!("copyforkhistory: invalid definition ids {parent_id:?} -> {fork_id:?}"));
    }
    if session_id.trim().is_empty() || session_id_field.is_empty() {
        return Err("copyforkhistory: no session to fork".to_string());
    }
    let (parent, fork) = (agent_current_zone(parent_id), agent_current_zone(fork_id));
    let Some(last) = last_session(gfs, &parent, session_id_field)? else {
        return Ok(ForkHistory::NoParentHistory);
    };
    if last != session_id {
        return Ok(ForkHistory::NotTheLastSession);
    }
    write_fork_mark(gfs, &fork, parent_id, session_id)?;
    // One copy of all four files, in one transaction, so the pane state can't
    // come from a different moment than the transcript. The pane state is
    // written through the cache, so the cache is flushed to the database first.
    gfs.flush_cache().map_err(|e| format!("flush before copy: {e}"))?;
    let files = [OUTPUT_FILE, TSIDX_FILE, crate::backend::continuity_state::STATE_FILE, SNAPSHOT_FILE];
    let Some(bytes) = gfs.copy_files_to_zone(&parent, &fork, &files).map_err(|e| format!("copy transcript: {e}"))? else {
        return Ok(ForkHistory::ForkHasHistory);
    };
    // The check above and the copy are separate reads: the parent may have
    // moved to a new session in between (a New conversation right after the
    // fork). What counts is the copy, so it is checked again, and undone if
    // it doesn't end in the session the fork continues.
    if last_session(gfs, &fork, session_id_field)?.as_deref() != Some(session_id) {
        gfs.delete_files(&fork, &files).map_err(|e| format!("undo copy: {e}"))?;
        return Ok(ForkHistory::NotTheLastSession);
    }
    tracing::info!(parent = %parent_id, fork = %fork_id, session_id = %session_id, bytes, "fork: copied its parent's history");
    Ok(ForkHistory::Copied { bytes })
}

/// The mark first: a crash after it leaves a fork without history, never
/// history without the mark that keeps its resume a fork.
fn write_fork_mark(gfs: &FileStore, fork_zone: &str, parent_id: &str, session_id: &str) -> Result<(), String> {
    if gfs.stat(fork_zone, FORK_MARK_FILE).map_err(|e| format!("stat fork mark: {e}"))?.is_none() {
        let mark = serde_json::json!({ "from": parent_id, "session_id": session_id, "at_ms": now_ms() });
        write_zone_file(gfs, fork_zone, FORK_MARK_FILE, mark.to_string().as_bytes())?;
    }
    Ok(())
}

/// A fork of an OLDER session (`session_id` is not the one the parent's
/// transcript ends in): the fork's transcript is the parent's `output` in
/// `src_zone` up to `end`, the byte where that session's last line ends
/// (from the agent's record: `RecordSession.ranges`), and its `output.tsidx`
/// up to the same point. The parent's snapshot and running summary describe
/// where the parent is now, not this point, so they are not copied.
///
/// `src_zone` is the parent's current zone, or the archived conversation
/// that session ended in. A session that continued across zones has only its
/// last zone's part copied.
pub fn copy_older_session_history(
    gfs: &FileStore,
    parent_id: &str,
    fork_id: &str,
    session_id: &str,
    session_id_field: &str,
    src_zone: &str,
    end: i64,
) -> Result<ForkHistory, String> {
    if !is_valid_definition_id(parent_id) || !is_valid_definition_id(fork_id) || parent_id == fork_id {
        return Err(format!("copyforkhistory: invalid definition ids {parent_id:?} -> {fork_id:?}"));
    }
    if session_id.trim().is_empty() || session_id_field.is_empty() || end <= 0 {
        return Err("copyforkhistory: no session to fork".to_string());
    }
    let fork = agent_current_zone(fork_id);
    let Some(output) = gfs.stat(src_zone, OUTPUT_FILE).map_err(|e| format!("stat transcript: {e}"))? else {
        return Ok(ForkHistory::NoParentHistory);
    };
    let end = end.min(output.size);
    // A session ends at the end of a line: a cut anywhere else is a record that
    // no longer matches the transcript.
    let (_, last_byte) = gfs.read_at(src_zone, OUTPUT_FILE, end - 1, 1).map_err(|e| format!("read transcript: {e}"))?;
    if last_byte != b"
" {
        return Ok(ForkHistory::NotTheLastSession);
    }
    // The sidecar up to the same point. Without one (or when it can't be read)
    // the copy has no receive times, as an old transcript has none.
    let tsidx_len = match gfs.stat(src_zone, TSIDX_FILE).map_err(|e| format!("stat sidecar: {e}"))? {
        Some(side) if side.size > 0 => {
            let read = |off: i64, len: i64| -> Result<Option<Vec<u8>>, crate::backend::storage::error::StoreError> {
                let (_, data) = gfs.read_at(src_zone, TSIDX_FILE, off, len)?;
                Ok((data.len() as i64 == len).then_some(data))
            };
            crate::backend::blockcontroller::shell::stamps::cut_before(&read, side.size, end as u64)
                .map_err(|e| format!("cut sidecar: {e}"))?
                .unwrap_or(0)
        }
        _ => 0,
    };
    write_fork_mark(gfs, &fork, parent_id, session_id)?;
    gfs.flush_cache().map_err(|e| format!("flush before copy: {e}"))?;
    let mut prefixes = vec![(OUTPUT_FILE, end)];
    if tsidx_len > 0 {
        prefixes.push((TSIDX_FILE, tsidx_len));
    }
    let Some(bytes) = gfs.copy_prefixes_to_zone(src_zone, &fork, &prefixes).map_err(|e| format!("copy transcript: {e}"))? else {
        return Ok(ForkHistory::ForkHasHistory);
    };
    // What counts is the copy: it must end in the session the fork continues
    // (a record that is stale, or a cut in the middle of another session).
    if last_session(gfs, &fork, session_id_field)?.as_deref() != Some(session_id) {
        gfs.delete_files(&fork, &[OUTPUT_FILE, TSIDX_FILE]).map_err(|e| format!("undo copy: {e}"))?;
        return Ok(ForkHistory::NotTheLastSession);
    }
    tracing::info!(parent = %parent_id, fork = %fork_id, session_id = %session_id, zone = %src_zone, bytes, "fork: copied its parent's history up to an older session");
    Ok(ForkHistory::Copied { bytes })
}

/// The parent session a fork's copied history ends in, from its mark in
/// `zone`. `None` for anything that isn't a fork with copied history.
pub fn inherited_session(gfs: &FileStore, zone: &str) -> Option<String> {
    let bytes = gfs.read_file(zone, FORK_MARK_FILE).ok()??;
    let mark: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let sid = mark.get("session_id")?.as_str()?.trim();
    (!sid.is_empty()).then(|| sid.to_string())
}

/// The session `zone`'s transcript ends in, or `None` when it has none. The
/// window widens until it holds a complete record that names one: the last
/// event can be larger than any fixed tail.
fn last_session(gfs: &FileStore, zone: &str, field: &str) -> Result<Option<String>, String> {
    let Some(file) = gfs.stat(zone, OUTPUT_FILE).map_err(|e| format!("stat transcript: {e}"))? else {
        return Ok(None);
    };
    let mut window = LAST_SESSION_TAIL_BYTES;
    while file.size > 0 {
        let offset = (file.size - window).max(0);
        let (at, tail) = gfs
            .read_at(zone, OUTPUT_FILE, offset, file.size - offset)
            .map_err(|e| format!("read transcript tail: {e}"))?;
        if let Some(sid) = crate::backend::blockcontroller::persistent::last_session_id_in_stream(&tail, at > 0, field) {
            return Ok(Some(sid));
        }
        if at == 0 {
            break;
        }
        window = window.saturating_mul(4);
    }
    Ok(None)
}

#[cfg(test)]
#[path = "fork_history_tests.rs"]
mod tests;
