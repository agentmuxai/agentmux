// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The size cap on what a terminal pane keeps of its output (the `term` file in
//! the pane's zone). A command that prints without end (`yes`) was stored whole:
//! 2.5 GB and a 4.3 GB write-ahead log in 100 s. The file is now circular: it
//! holds the last `term:storedoutputmb` megabytes (50 by default, 0 for no
//! limit), offsets stay absolute so readers that remember where they were keep
//! working, and the parts below the window are deleted as the file grows
//! (`FileStore::trim_circular_head`). Agent transcripts are a different file and
//! are not touched.

use std::sync::atomic::{AtomicI64, Ordering};

use crate::backend::storage::filestore::{FileOpts, FileStore};

/// The settings.json key, whole megabytes.
pub const SETTING: &str = "term:storedoutputmb";
const DEFAULT_MB: i64 = 50;
/// A typo like 5000000 is a limit nobody meant.
const MAX_MB: i64 = 100_000;
const MIB: i64 = 1 << 20;
/// Log written between two checkpoints made by the writer itself. Counted as the
/// log takes it in (`append_log_cost`), not as output: the PTY's batches run
/// from one 4 KB read to 256 KB, and the smaller they are the more log each
/// byte of output costs.
const WAL_RELIEF_EVERY: i64 = 8 * MIB;
/// The block file the cap applies to.
pub const TERM_FILE: &str = "term";

/// Written into the stream once, when the first output is dropped, so the pane
/// (live and on reload, until it scrolls out of the window) says so.
pub const TRIMMED_NOTICE: &[u8] = b"\r\n\x1b[2m[earlier output trimmed: AgentMux keeps a limited amount of each terminal's output]\x1b[0m\r\n";

static CAP_BYTES: AtomicI64 = AtomicI64::new(DEFAULT_MB * MIB);
/// Log the terminal writers have put in since one of them last checkpointed.
static LOG_SINCE_RELIEF: AtomicI64 = AtomicI64::new(0);

/// Whether this append is the one that should checkpoint: `pending` has taken in
/// `WAL_RELIEF_EVERY` of log. All panes share the store's log, so they share the
/// count; whoever crosses the line resets it.
fn relief_due(pending: &AtomicI64, start: i64, len: usize) -> bool {
    let cost = crate::backend::storage::filestore::append_log_cost(start, len);
    if pending.fetch_add(cost, Ordering::Relaxed) + cost < WAL_RELIEF_EVERY {
        return false;
    }
    pending.store(0, Ordering::Relaxed);
    true
}

/// The cap in bytes; 0 means no limit.
pub fn cap_bytes() -> i64 {
    CAP_BYTES.load(Ordering::Relaxed)
}

/// Take the setting's value (`None`: not set, or not a whole number of
/// megabytes, which means the default). Called whenever settings change.
pub fn apply_setting(value: Option<&serde_json::Value>) {
    CAP_BYTES.store(cap_from_setting(value), Ordering::Relaxed);
}

fn cap_from_setting(value: Option<&serde_json::Value>) -> i64 {
    let mb = match value.and_then(serde_json::Value::as_i64) {
        Some(mb) if mb >= 0 => mb.min(MAX_MB),
        _ => DEFAULT_MB,
    };
    mb * MIB
}

/// The options a new `term` file is made with.
pub fn term_opts() -> FileOpts {
    opts_for(cap_bytes())
}

fn opts_for(cap: i64) -> FileOpts {
    FileOpts { circular: cap > 0, maxsize: cap, ..Default::default() }
}

/// After `data` was appended to a pane's `term` at offset `start`: bring the
/// file to the cap, and drop the parts that fell out of its window. Returns
/// true when output has been dropped from the file and not yet announced, and
/// `data` ends where a line of text can be inserted (not inside a character or
/// an escape sequence, which the next read of the PTY continues), so the caller
/// can write [`TRIMMED_NOTICE`] after it. At a bad boundary the notice waits
/// for a later append. A store error is logged and costs nothing but the trim.
pub fn after_append(fs: &FileStore, block_id: &str, start: i64, data: &[u8]) -> bool {
    after_append_capped(fs, block_id, start, data, cap_bytes())
}

/// Whether text can be written after `data` without cutting something the PTY's
/// next bytes continue: a UTF-8 character, or an escape sequence (CSI, OSC and
/// the other string sequences, or an escape that wants a final byte). Looks back
/// a bounded distance for the last escape, so a string sequence that began
/// further back than that counts as finished.
pub(super) fn ends_at_boundary(data: &[u8]) -> bool {
    const LOOK_BACK: usize = 256;
    let n = data.len();
    for back in 1..=3.min(n) {
        let b = data[n - back];
        if b & 0xC0 == 0x80 {
            continue; // a continuation byte: the character's start is further back
        }
        let width = match b {
            0xF0..=0xFF => 4,
            0xE0..=0xEF => 3,
            0xC0..=0xDF => 2,
            _ => 1,
        };
        if width > back {
            return false;
        }
        break;
    }
    let Some(esc) = data[n.saturating_sub(LOOK_BACK)..].iter().rposition(|&b| b == 0x1b) else {
        return true;
    };
    let seq = &data[n.saturating_sub(LOOK_BACK) + esc..];
    match seq.get(1) {
        None => false,
        Some(b'[') => seq[2..].iter().any(|b| (0x40..=0x7e).contains(b)),
        Some(b']' | b'P' | b'X' | b'^' | b'_') => seq[2..].iter().enumerate().any(|(i, &b)| b == 0x07 || (b == 0x1b && seq[2..].get(i + 1) == Some(&b'\\'))),
        Some(0x20..=0x2f) => seq.len() >= 3,
        Some(_) => true,
    }
}

fn after_append_capped(fs: &FileStore, block_id: &str, start: i64, data: &[u8], cap: i64) -> bool {
    let len = data.len();
    // Under a flood the log would only grow: empty it every few megabytes.
    if relief_due(&LOG_SINCE_RELIEF, start, len) {
        if let Err(e) = fs.checkpoint_between_writes() {
            tracing::debug!(error = %e, "term cap: checkpoint between writes failed");
        }
    }
    let mut trim = false;
    let current = match fs.stat(block_id, TERM_FILE) {
        Ok(Some(f)) => f,
        Ok(None) => return false,
        Err(e) => {
            tracing::warn!(block_id = %block_id, error = %e, "term cap: stat failed");
            return false;
        }
    };
    // A file made before the cap, or under another setting, is brought over.
    if current.opts.circular != (cap > 0) || current.opts.maxsize != cap {
        match fs.set_circular(block_id, TERM_FILE, cap) {
            Ok(changed) => trim = changed,
            Err(e) => {
                tracing::warn!(block_id = %block_id, error = %e, "term cap: could not set the limit");
                return false;
            }
        }
    }
    let start = start.max(0);
    let first_dead_before = fs_first_live(start, cap);
    if fs_first_live(start + len as i64, cap) > first_dead_before {
        trim = true;
    }
    let announced = current.meta.get("trimmed").and_then(serde_json::Value::as_bool) == Some(true);
    let mut removed = 0;
    if trim && cap > 0 {
        match fs.trim_circular_head(block_id, TERM_FILE) {
            Ok(n) => removed = n,
            Err(e) => {
                tracing::warn!(block_id = %block_id, error = %e, "term cap: trim failed");
                return false;
            }
        }
    }
    // Output actually deleted (now, or by an earlier append that wasn't at a
    // boundary) is announced once, at the first boundary where text fits. A
    // file merely converted to the cap, or under a wider one, lost nothing.
    let dropped = removed > 0 || current.meta.contains_key(crate::backend::storage::filestore::RETAINED_FROM_KEY);
    if announced || !dropped || !ends_at_boundary(data) {
        return false;
    }
    let mut meta = crate::backend::storage::filestore::FileMeta::new();
    meta.insert("trimmed".to_string(), serde_json::Value::Bool(true));
    if let Err(e) = fs.write_meta(block_id, TERM_FILE, meta, true) {
        tracing::warn!(block_id = %block_id, error = %e, "term cap: could not note the trim");
        return false;
    }
    true
}

fn fs_first_live(size: i64, cap: i64) -> i32 {
    crate::backend::storage::filestore::first_live_part(size, cap)
}

#[cfg(test)]
#[path = "tests_term_cap.rs"]
mod tests;
