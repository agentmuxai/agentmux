// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `output.idx` rebuild: a lazily-built, self-validating line-offset cache.

use crate::backend::storage::filestore::FileStore;

/// Magic header size for `output.idx`: the first 8 bytes are the `output` byte-size
/// the index was built for. The index is valid iff this equals `output`'s current
/// size; otherwise it is stale and must be rebuilt.
pub(crate) const OUTPUT_IDX_HEADER_LEN: i64 = 8;

/// Rebuild `output.idx` from `output` in a single streaming scan and atomically
/// replace it. The index is the byte offset of every **non-blank** line, matching
/// the reader's line addressing (`String::lines().filter(!trim().is_empty())`).
///
/// Layout: `[covered_size: u64-LE][offset_0: u64-LE][offset_1]...`. `covered_size`
/// records the `output` size this index reflects so the read path can detect
/// staleness in O(1) and rebuild only when `output` actually grew.
///
/// The scan streams `output` in 1 MiB windows so memory stays O(one line + offsets)
/// rather than loading the whole (potentially multi-GB) file. Line splitting is
/// done on raw bytes; since UTF-8 continuation bytes never collide with `\n`, lines
/// are never split mid-codepoint, so per-line `from_utf8_lossy` matches the reader.
///
/// Returns the number of indexed (non-blank) lines on success, or `None` if
/// `output` is unreadable or the index write fails (caller falls back to slow path).
pub(crate) fn rebuild_output_idx(
    fs: &FileStore,
    block_id: &str,
    output_size: u64,
) -> Option<u64> {
    const IDX: &str = "output.idx";
    const WIN: i64 = 1 << 20; // 1 MiB read window

    // Start/duration logging (docs/status/STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md
    // §7.1): the prior completion-only log recorded a rebuild happened but
    // not how long it ran for or when it started, so a live incident
    // couldn't be checked for whether a rebuild's execution window actually
    // overlapped some other stalled RPC — only that both happened "around
    // the same time." Kept as a lasting diagnostic, not a one-off debug
    // print — a slow rebuild is exactly the kind of thing worth being able
    // to correlate after the fact, the same way `mem_attribution` already is.
    let started = std::time::Instant::now();
    tracing::info!(block_id = %block_id, covered = output_size, "output.idx rebuild starting");

    // Offsets buffer starts with the covered-size header.
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&output_size.to_le_bytes());

    let mut line_count: u64 = 0;
    let mut cursor: u64 = 0; // byte offset where the current line begins
    let mut line_buf: Vec<u8> = Vec::new(); // bytes of the current line, excluding '\n'
    let mut read_pos: i64 = 0;

    let flush_line = |line_buf: &mut Vec<u8>,
                      cursor: &mut u64,
                      buf: &mut Vec<u8>,
                      line_count: &mut u64,
                      had_newline: bool| {
        // The reader strips a trailing '\r' (CRLF) and treats trim-empty as blank.
        let is_blank = String::from_utf8_lossy(line_buf).trim().is_empty();
        if !is_blank {
            buf.extend_from_slice(&cursor.to_le_bytes());
            *line_count += 1;
        }
        // Advance cursor past this line's bytes (+1 for the consumed '\n').
        *cursor += line_buf.len() as u64 + if had_newline { 1 } else { 0 };
        line_buf.clear();
    };

    while read_pos < output_size as i64 {
        let (_, chunk) = match fs.read_at(block_id, "output", read_pos, WIN) {
            Ok(v) => v,
            Err(e) => {
                // codex P2 on #2724: every exit path must log a terminal
                // event (with duration) — an unmatched "starting" event with
                // no completion reads as "still running," easy to mistake
                // for a rebuild that was active throughout an incident when
                // it actually failed fast and returned immediately.
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    duration_ms = started.elapsed().as_millis() as u64,
                    "output.idx rebuild failed: read_at error"
                );
                return None;
            }
        };
        if chunk.is_empty() {
            break;
        }
        for &b in &chunk {
            if b == b'\n' {
                flush_line(&mut line_buf, &mut cursor, &mut buf, &mut line_count, true);
            } else {
                line_buf.push(b);
            }
        }
        read_pos += chunk.len() as i64;
    }
    // Trailing line with no final '\n'.
    if !line_buf.is_empty() {
        flush_line(&mut line_buf, &mut cursor, &mut buf, &mut line_count, false);
    }

    if let Ok(None) = fs.stat(block_id, IDX) {
        let _ = fs.make_file(
            block_id,
            IDX,
            std::collections::HashMap::new(),
            crate::backend::storage::filestore::FileOpts::default(),
        );
    }
    match fs.write_file(block_id, IDX, &buf) {
        Ok(()) => {
            tracing::info!(
                block_id = %block_id,
                lines = line_count,
                covered = output_size,
                duration_ms = started.elapsed().as_millis() as u64,
                "output.idx rebuilt"
            );
            Some(line_count)
        }
        Err(e) => {
            tracing::warn!(
                block_id = %block_id,
                error = %e,
                duration_ms = started.elapsed().as_millis() as u64,
                "output.idx rebuild write failed"
            );
            None
        }
    }
}
