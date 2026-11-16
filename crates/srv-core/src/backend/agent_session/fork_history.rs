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

/// The most of a parent's transcript a fork copies: its newest part, cut at a turn start. A parent's
/// transcript is its whole life (gigabytes for a long-lived agent); copying all of it holds the store's
/// write lock for seconds and outlasts the pane's call. The fork's CLI continues the parent's
/// conversation whatever the pane shows, so only the pane's history is cut.
pub const FORK_COPY_MAX_BYTES: i64 = 64 << 20;

/// Copy the parent's transcript into the fork's zone in `gfs`, when the
/// fork continues `session_id`, the session the parent's transcript ends in.
/// `session_id_field` is the provider's session field in its stream events.
/// A transcript over [`FORK_COPY_MAX_BYTES`] is copied only from a turn start inside its newest part.
pub fn copy_parent_history(
    gfs: &FileStore,
    parent_id: &str,
    fork_id: &str,
    session_id: &str,
    session_id_field: &str,
) -> Result<ForkHistory, String> {
    copy_parent_history_capped(gfs, parent_id, fork_id, session_id, session_id_field, FORK_COPY_MAX_BYTES)
}

fn copy_parent_history_capped(
    gfs: &FileStore,
    parent_id: &str,
    fork_id: &str,
    session_id: &str,
    session_id_field: &str,
    cap: i64,
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
    // The pane state is written through the cache, so the cache is flushed to the database first.
    gfs.flush_cache().map_err(|e| format!("flush before copy: {e}"))?;
    let size = gfs.stat(&parent, OUTPUT_FILE).map_err(|e| format!("stat transcript: {e}"))?.map_or(0, |f| f.size);
    if size > cap {
        return copy_window(gfs, &parent, &fork, (parent_id, fork_id), (session_id, session_id_field), size, cap, true);
    }
    // One copy of all four files, in one transaction, so the pane state can't
    // come from a different moment than the transcript.
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
    sign_for_fork(gfs, &parent, &fork);
    tracing::info!(parent = %parent_id, fork = %fork_id, session_id = %session_id, bytes, "fork: copied its parent's history");
    Ok(ForkHistory::Copied { bytes })
}

/// The copy's user lines that were signed for the parent are signed for the fork, so the human's own
/// earlier messages still show as theirs (`transcript::origin_resign`). A failure leaves them
/// unsigned for the fork, which shows them in the container, never as the human. The signing
/// rewrites bytes in the file, which drops its line epoch, so the file is counted again here (a copy
/// is at most [`FORK_COPY_MAX_BYTES`]): the fork's pane opens from its line positions at once.
fn sign_for_fork(gfs: &FileStore, from_zone: &str, fork_zone: &str) {
    use crate::backend::transcript::origin_sig::owner_of_zone;
    let (from, to) = (owner_of_zone(from_zone), owner_of_zone(fork_zone));
    if let Err(e) = crate::backend::transcript::origin_resign::resign_zone(gfs, fork_zone, OUTPUT_FILE, from, to) {
        tracing::warn!(parent = %from, fork = %to, error = %e, "fork: couldn't sign the copied messages for the fork");
    }
    if let Err(e) = gfs.init_line_counter(fork_zone, OUTPUT_FILE) {
        tracing::warn!(fork = %to, error = %e, "fork: couldn't count the copied transcript; the pane counts it when it opens");
    }
}

/// A transcript over the cap: the fork's pane gets its newest part. The window is `[end - cap, end)`
/// of `src_zone`'s `output`, started at the first turn start in it (a pane can't open on the middle
/// of a turn), signed for the fork in memory and written once, so the file is counted from its first
/// byte and nothing rewrites it. Its sidecar is the parent's records for that range, moved to start
/// at 0. The parent's snapshot describes line numbers of the whole transcript, so it is not copied;
/// the running summary is, for a fork of the session the parent is in (`with_state`).
#[allow(clippy::too_many_arguments)]
fn copy_window(
    gfs: &FileStore,
    src_zone: &str,
    fork: &str,
    (parent_id, fork_id): (&str, &str),
    (session_id, field): (&str, &str),
    end: i64,
    cap: i64,
    with_state: bool,
) -> Result<ForkHistory, String> {
    use crate::backend::storage::error::StoreError;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};
    use crate::backend::transcript::origin_sig::owner_of_zone;
    let (at, mut data) = gfs.read_at(src_zone, OUTPUT_FILE, (end - cap).max(0), end.min(cap)).map_err(|e| format!("read transcript: {e}"))?;
    let start = window_start(&data, at);
    if start >= data.len() {
        return Ok(ForkHistory::NoParentHistory);
    }
    data.drain(..start);
    let cut = at + start as i64;
    if gfs.stat(fork, OUTPUT_FILE).map_err(|e| format!("stat fork transcript: {e}"))?.is_some() {
        return Ok(ForkHistory::ForkHasHistory);
    }
    crate::backend::transcript::origin_resign::resign_buffer(&mut data, owner_of_zone(src_zone), owner_of_zone(fork));
    // The sidecar before the transcript: a fork never has a transcript whose times are unread.
    let side = match gfs.stat(src_zone, TSIDX_FILE).map_err(|e| format!("stat sidecar: {e}"))? {
        Some(side) if side.size > 0 => {
            let read = |off: i64, len: i64| -> Result<Option<Vec<u8>>, StoreError> {
                let (_, bytes) = gfs.read_at(src_zone, TSIDX_FILE, off, len)?;
                Ok((bytes.len() as i64 == len).then_some(bytes))
            };
            crate::backend::blockcontroller::shell::stamps::rebase_range(&read, side.size, cut as u64, end as u64)
                .map_err(|e| format!("cut sidecar: {e}"))?
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    let put = |name: &str, bytes: &[u8]| -> Result<bool, String> {
        match gfs.make_file(fork, name, FileMeta::new(), FileOpts::default()) {
            Ok(()) => gfs.write_file(fork, name, bytes).map(|()| true).map_err(|e| format!("write {name}: {e}")),
            Err(StoreError::AlreadyExists) => Ok(false),
            Err(e) => Err(format!("make {name}: {e}")),
        }
    };
    if !put(OUTPUT_FILE, &data)? {
        return Ok(ForkHistory::ForkHasHistory);
    }
    if !side.is_empty() {
        put(TSIDX_FILE, &side)?;
    }
    let mut bytes = (data.len() + side.len()) as u64;
    if with_state {
        // Each version says how many bytes of the parent's transcript it covers: moved to the copy's
        // start, or `latest_state` drops it as describing more than the fork holds.
        if let Some(state) = rebased_state(gfs, src_zone, cut, data.len() as i64)? {
            if put(crate::backend::continuity_state::STATE_FILE, &state)? {
                bytes += state.len() as u64;
            }
        }
    }
    if last_session(gfs, fork, field)?.as_deref() != Some(session_id) {
        gfs.delete_files(fork, &[OUTPUT_FILE, TSIDX_FILE, crate::backend::continuity_state::STATE_FILE]).map_err(|e| format!("undo copy: {e}"))?;
        return Ok(ForkHistory::NotTheLastSession);
    }
    tracing::info!(parent = %parent_id, fork = %fork_id, session_id = %session_id, zone = %src_zone, bytes, from_byte = cut, "fork: copied the newest part of its parent's history");
    Ok(ForkHistory::Copied { bytes })
}

/// The newest versions of `zone`'s running summary with their coverage moved to a copy that starts
/// `cut` bytes into the transcript and is `size` bytes long; `None` when there is no summary.
fn rebased_state(gfs: &FileStore, zone: &str, cut: i64, size: i64) -> Result<Option<Vec<u8>>, String> {
    use crate::backend::continuity_state::{rebase_versions, STATE_FILE};
    /// A summary file is a few KB a version; past this the newest part is not read.
    const READ_MAX: i64 = 16 << 20;
    let Some(file) = gfs.stat(zone, STATE_FILE).map_err(|e| format!("stat summary: {e}"))?.filter(|f| f.size > 0) else {
        return Ok(None);
    };
    let from = (file.size - READ_MAX).max(0);
    let (_, raw) = gfs.read_at(zone, STATE_FILE, from, file.size - from).map_err(|e| format!("read summary: {e}"))?;
    // A read that began mid-line starts at the next whole one.
    let raw = if from > 0 { raw.iter().position(|b| *b == b'\n').map_or(&raw[..0], |i| &raw[i + 1..]) } else { &raw[..] };
    let rebased = rebase_versions(raw, cut, size);
    Ok((!rebased.is_empty()).then_some(rebased))
}

/// Where in `window` (read from `at` in its file) the copy starts: the first turn start, after the
/// line the window began inside of; the first whole line when there is no turn start. One past the
/// end when the window holds no whole line.
fn window_start(window: &[u8], at: i64) -> usize {
    use crate::backend::transcript::turns::is_claude_turn_start;
    let first = if at == 0 { 0 } else { window.iter().position(|b| *b == b'\n').map_or(window.len(), |i| i + 1) };
    let mut pos = first;
    while pos < window.len() {
        let end = window[pos..].iter().position(|b| *b == b'\n').map_or(window.len(), |i| pos + i + 1);
        if is_claude_turn_start(&String::from_utf8_lossy(&window[pos..end])) {
            return pos;
        }
        pos = end;
    }
    first
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
    copy_older_session_history_capped(gfs, parent_id, fork_id, session_id, session_id_field, src_zone, end, FORK_COPY_MAX_BYTES)
}

/// [`copy_older_session_history`], with the cap on what is copied: a history longer than `cap` is
/// copied from a turn start inside its newest `cap` bytes (see [`copy_window`]).
#[allow(clippy::too_many_arguments)]
fn copy_older_session_history_capped(
    gfs: &FileStore,
    parent_id: &str,
    fork_id: &str,
    session_id: &str,
    session_id_field: &str,
    src_zone: &str,
    end: i64,
    cap: i64,
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
    if end > cap {
        return copy_window(gfs, src_zone, &fork, (parent_id, fork_id), (session_id, session_id_field), end, cap, false);
    }
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
    sign_for_fork(gfs, src_zone, &fork);
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
    if file.size == 0 {
        return Ok(None);
    }
    let mut window = LAST_SESSION_TAIL_BYTES;
    loop {
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
