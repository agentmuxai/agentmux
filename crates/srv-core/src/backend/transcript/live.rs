// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The live file of a streaming reply (`SPEC_HISTORY_COMPACT_FORM_ONLY_2026_11_23.md` section 4).
//!
//! While a reply streams, its fragments (`stream_event` lines) go to `output.live`, a second file in
//! the agent's zone, and to the pane's feed as events with no position, as the pane takes them today.
//! `output` never receives a fragment unless it cannot be proven redundant (see
//! [`compact::LiveReply`]). When a message has stopped and every block of it is in a complete line,
//! its fragments leave `output.live`. At a turn boundary, what is left of a cut-off reply becomes
//! one interrupted `assistant` line in `output`, before the boundary line.
//!
//! A kill leaves `output.live` behind. The first write to a block after a start looks at it: the
//! leftovers are reassembled against the last complete lines of `output`, appended, and the file is
//! emptied.
//!
//! Readers of `output` see no fragments. A pane opened mid-reply gets the rest of the reply from the
//! feed and the whole of it from `output` once the block completes.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use super::compact::{recover_live, LiveReply};
use crate::backend::agent_session::OUTPUT_FILE;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::{FileOpts, FileStore};

/// The file in an agent's zone that holds the fragments of the reply in progress.
pub const LIVE_FILE: &str = "output.live";

/// How much of the end of `output` a recovery reads to find the complete lines of a cut-off reply.
const RECOVERY_TAIL_BYTES: i64 = 4 << 20;

#[derive(Default)]
struct State {
    reply: LiveReply,
    /// The live file has been looked at since this process started.
    looked: bool,
}

static STATES: LazyLock<Mutex<HashMap<String, State>>> = LazyLock::new(Default::default);

/// One step of an append, in order.
pub(crate) enum Run {
    /// Fragment lines, for the live file and an unpositioned event.
    Fragments(Vec<u8>),
    /// Complete lines, for `output`.
    Lines(Vec<u8>),
    /// The held fragments are redundant now: empty the live file.
    ClearLive,
}

fn push(runs: &mut Vec<Run>, fragment: bool, line: &[u8]) {
    match runs.last_mut() {
        Some(Run::Fragments(b)) if fragment => {
            b.extend_from_slice(line);
            b.push(b'\n');
        }
        Some(Run::Lines(b)) if !fragment => {
            b.extend_from_slice(line);
            b.push(b'\n');
        }
        _ => {
            let mut b = line.to_vec();
            b.push(b'\n');
            runs.push(if fragment { Run::Fragments(b) } else { Run::Lines(b) });
        }
    }
}

/// Split `records` (complete lines, each ending in a newline) into the runs to write, feeding the
/// block's reply tracker in line order. Not UTF-8 is not JSON: such a line is a complete line.
pub(crate) fn plan(block_id: &str, records: &[u8]) -> Vec<Run> {
    let mut states = STATES.lock().unwrap_or_else(|e| e.into_inner());
    let st = states.entry(block_id.to_string()).or_default();
    let mut runs = Vec::new();
    for line in records.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(text) = std::str::from_utf8(line) else {
            push(&mut runs, false, line);
            continue;
        };
        let before = st.reply.before(text);
        for e in &before.emits {
            push(&mut runs, false, e.line().as_bytes());
        }
        push(&mut runs, before.fragment, line);
        if before.ended {
            runs.push(Run::ClearLive);
        } else if st.reply.holds_fragments() && st.reply.settled() {
            st.reply.reset();
            runs.push(Run::ClearLive);
        }
    }
    runs
}

/// Forget what this process knows of a block, as a new process would.
#[cfg(test)]
pub(crate) fn forget(block_id: &str) {
    STATES.lock().unwrap_or_else(|e| e.into_inner()).remove(block_id);
}

/// The files a block's live fragments are written to and cleared from: the same zones its complete
/// lines go to.
pub(crate) fn targets(
    block_id: &str,
    filestore: Option<&Arc<FileStore>>,
    global: Option<(&Arc<FileStore>, &str)>,
    global_only: bool,
) -> Vec<(Arc<FileStore>, String)> {
    let mut out = Vec::new();
    if let Some((gfs, zone)) = global {
        out.push((gfs.clone(), zone.to_string()));
    }
    if let Some(fs) = filestore.filter(|_| !global_only) {
        out.push((fs.clone(), block_id.to_string()));
    }
    out
}

/// Append fragment lines to the live file of every target, creating it on first write.
pub(crate) fn append_fragments(targets: &[(Arc<FileStore>, String)], bytes: &[u8]) {
    for (fs, zone) in targets {
        let mut put = || fs.append_data(zone, LIVE_FILE, bytes);
        let made = match put() {
            Err(StoreError::NotFound) => match fs.make_file(zone, LIVE_FILE, HashMap::new(), FileOpts::default()) {
                Ok(()) | Err(StoreError::AlreadyExists) => put(),
                Err(e) => Err(e),
            },
            other => other,
        };
        if let Err(e) = made {
            tracing::warn!(zone = %zone, error = %e, "transcript live file: append failed");
        }
    }
}

/// Empty the live file of every target.
pub(crate) fn clear(targets: &[(Arc<FileStore>, String)]) {
    for (fs, zone) in targets {
        match fs.delete_file(zone, LIVE_FILE) {
            Ok(()) | Err(StoreError::NotFound) => {}
            Err(e) => tracing::warn!(zone = %zone, error = %e, "transcript live file: clearing failed"),
        }
    }
}

/// The lines of `bytes`, without a first line that may be cut by where the read began.
fn lines_of(bytes: &[u8], cut_head: bool) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut it = text.split('\n');
    if cut_head {
        it.next();
    }
    it.filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// What a leftover live file of `zone` leaves for `output`, as lines ending in newlines; `None` when
/// there is no live file or nothing in it.
fn leftovers_of(fs: &FileStore, zone: &str) -> Option<Vec<u8>> {
    let live = fs.read_file(zone, LIVE_FILE).ok().flatten().filter(|b| !b.is_empty())?;
    let size = fs.stat(zone, OUTPUT_FILE).ok().flatten().map_or(0, |f| f.size);
    let start = (size - RECOVERY_TAIL_BYTES).max(0);
    let tail = if size > start { fs.read_bytes_db(zone, OUTPUT_FILE, start, size - start).unwrap_or_default() } else { Vec::new() };
    let emits = recover_live(&lines_of(&tail, start > 0), &lines_of(&live, false));
    let mut out = Vec::new();
    for e in emits {
        out.extend_from_slice(e.line().as_bytes());
        out.push(b'\n');
    }
    Some(out)
}

/// Settle a zone's live file before its conversation leaves the zone (an archive moves `output` away): what a kill
/// left becomes lines of `output`, and the file is emptied. A leftover left in place would be recovered into the next
/// conversation's `output` as a cut-off reply it never had. A failure to append leaves the live file as it is.
pub(crate) fn settle(fs: &FileStore, zone: &str) {
    let Some(left) = leftovers_of(fs, zone) else { return };
    if !left.is_empty() {
        if let Err(e) = fs.append_data(zone, OUTPUT_FILE, &left) {
            tracing::warn!(zone = %zone, error = %e, "transcript live file: settling before archive failed");
            return;
        }
    }
    if let Err(e) = fs.delete_file(zone, LIVE_FILE) {
        tracing::warn!(zone = %zone, error = %e, "transcript live file: clearing failed");
    }
}

/// The first call for a block in this process: what a kill left in its live file, as lines for
/// `output` (possibly none), after which the live files are emptied. `None` on every later call,
/// and when there is nothing to recover. The first target (the shared zone, when there is one) is the
/// one that is read: a copy left in another target after the first was emptied (an archive settles the
/// shared zone) is stale, and is only emptied.
pub(crate) fn recover_once(block_id: &str, targets: &[(Arc<FileStore>, String)]) -> Option<Vec<u8>> {
    {
        let mut states = STATES.lock().unwrap_or_else(|e| e.into_inner());
        let st = states.entry(block_id.to_string()).or_default();
        if std::mem::replace(&mut st.looked, true) {
            return None;
        }
    }
    let (fs, zone) = targets.first()?;
    let found = leftovers_of(fs, zone);
    clear(targets);
    found
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
