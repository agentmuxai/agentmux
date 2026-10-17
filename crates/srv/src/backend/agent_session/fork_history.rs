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
    /// showing the parent's later history would be wrong. Nothing copied.
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
    // The mark first: a crash after it leaves a fork without history, never
    // history without the mark that keeps its resume a fork.
    if gfs.stat(&fork, FORK_MARK_FILE).map_err(|e| format!("stat fork mark: {e}"))?.is_none() {
        let mark = serde_json::json!({ "from": parent_id, "session_id": session_id, "at_ms": now_ms() });
        write_zone_file(gfs, &fork, FORK_MARK_FILE, mark.to_string().as_bytes())?;
    }
    let state = crate::backend::continuity_state::STATE_FILE;
    let Some(bytes) = gfs
        .copy_files_to_zone(&parent, &fork, &[OUTPUT_FILE, TSIDX_FILE, state])
        .map_err(|e| format!("copy transcript: {e}"))?
    else {
        return Ok(ForkHistory::ForkHasHistory);
    };
    // The pane state is written through the cache, so it's read through it.
    if let Some(snapshot) = gfs.read_file(&parent, SNAPSHOT_FILE).map_err(|e| format!("read pane state: {e}"))? {
        if !snapshot.is_empty() {
            write_zone_file(gfs, &fork, SNAPSHOT_FILE, &snapshot)?;
        }
    }
    tracing::info!(parent = %parent_id, fork = %fork_id, session_id = %session_id, bytes, "fork: copied its parent's history");
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

/// The session `zone`'s transcript ends in, or `None` when it has none.
fn last_session(gfs: &FileStore, zone: &str, field: &str) -> Result<Option<String>, String> {
    let Some(file) = gfs.stat(zone, OUTPUT_FILE).map_err(|e| format!("stat transcript: {e}"))? else {
        return Ok(None);
    };
    if file.size <= 0 {
        return Ok(None);
    }
    let offset = (file.size - LAST_SESSION_TAIL_BYTES).max(0);
    let (at, tail) = gfs
        .read_at(zone, OUTPUT_FILE, offset, file.size - offset)
        .map_err(|e| format!("read transcript tail: {e}"))?;
    Ok(crate::backend::blockcontroller::persistent::last_session_id_in_stream(&tail, at > 0, field))
}

#[cfg(test)]
#[path = "fork_history_tests.rs"]
mod tests;
