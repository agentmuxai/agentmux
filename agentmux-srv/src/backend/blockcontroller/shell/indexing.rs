// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `output.idx` rebuild: a lazily-built, self-validating line-offset cache.

use crate::backend::storage::filestore::FileStore;

/// Magic header size for `output.idx`: the first 8 bytes are the `output` byte-size
/// the index was built for. The index is valid iff this equals `output`'s current
/// size; otherwise it is stale and must be rebuilt.
pub(crate) const OUTPUT_IDX_HEADER_LEN: i64 = 8;

/// Key in `output.idx`'s file metadata naming the `output` generation the
/// index was built for (Phase 5a-2b; `filestore/counter.rs`). Written in the
/// same transaction as the index, so the label always describes the bytes.
const IDX_GEN_META: &str = "for_gen";

/// `output`'s size and valid generation, from the database — never this
/// process's `stat` cache, which another srv instance's append makes stale.
pub(crate) fn output_now(fs: &FileStore, zone: &str) -> Option<(u64, Option<String>)> {
    let state = fs.line_state(zone, "output").ok()??;
    Some((state.size.max(0) as u64, state.counted.map(|c| c.gen)))
}

/// Whether an existing `output.idx` may describe `output`'s current content,
/// apart from its covered-size header (callers check that).
///
/// A size match alone is not proof: `output` replaced or restored with
/// content that happens to reach the old covered size would have its lines
/// read at another file's offsets. When `output` has a valid generation the
/// index must be labelled with it; any mismatch — unlabelled (older build),
/// or built for another generation — means rebuild, which is always safe.
/// An uncounted `output` (legacy or written by an older build) keeps the
/// covered-size rule alone, as before.
pub(crate) fn idx_generation_ok(fs: &FileStore, zone: &str, output_gen: Option<&str>) -> bool {
    let Some(gen) = output_gen else { return true };
    let label = fs.meta_db(zone, "output.idx").ok().flatten().and_then(|m| {
        m.get(IDX_GEN_META).and_then(|v| v.as_str()).map(str::to_string)
    });
    label.as_deref() == Some(gen)
}

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
    build_output_idx_from(fs, block_id, output_size, 0, Vec::new(), 0)
}

/// Bring an existing `output.idx` up to `output`'s current size by scanning
/// ONLY the appended bytes, instead of rescanning from byte zero.
///
/// Falls back to a full rebuild whenever the existing index can't be trusted
/// as a base: missing, header unreadable, no entries yet, or `output` shrank
/// below what the index already covers (rotation/truncation).
///
/// Why this exists: the index is a pure cache with no incremental mutation, so
/// a stale one used to mean a full O(file-size) rescan. On a live agent
/// `output` grows continuously, so "stale" was the steady state — a 30-second
/// line-count poll drove 37 full rebuilds in 19 minutes on a 759 MB
/// transcript, mean 3168 ms, ~10% duty cycle
/// (docs/reports/REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md §5).
///
/// Returning the stale count instead was tried and is WRONG: `line_count`
/// feeds `useHistoryPagination`'s tail window (`offset = total - PAGE_SIZE`),
/// so an undercount silently drops the most recent history on reopen — codex
/// P1 on PR #2838. The count has to stay exact; only the cost of keeping it
/// exact is negotiable.
pub(crate) fn extend_output_idx(
    fs: &FileStore,
    block_id: &str,
    output_size: u64,
) -> Option<u64> {
    const IDX: &str = "output.idx";

    let full = || rebuild_output_idx(fs, block_id, output_size);

    // An index built for another generation of `output` is no base at all.
    let output_gen = output_now(fs, block_id).and_then(|(_, gen)| gen);
    if !idx_generation_ok(fs, block_id, output_gen.as_deref()) {
        return full();
    }
    let Ok(Some(idx_stat)) = fs.stat(block_id, IDX) else { return full() };
    if idx_stat.size < OUTPUT_IDX_HEADER_LEN {
        return full();
    }
    let entry_count = ((idx_stat.size - OUTPUT_IDX_HEADER_LEN) / 8) as u64;
    let Ok((_, header)) = fs.read_at(block_id, IDX, 0, OUTPUT_IDX_HEADER_LEN) else { return full() };
    let Ok(header) = <[u8; 8]>::try_from(header.as_slice()) else { return full() };
    let covered = u64::from_le_bytes(header);

    if covered == output_size {
        return Some(entry_count); // already current
    }
    // Shrank (rotated/truncated) or nothing to anchor on — a base we can't trust.
    if covered > output_size || entry_count == 0 {
        return full();
    }

    // Re-scan from the START of the last indexed line, not from `covered`. The
    // previous build may have indexed a trailing line that had no newline yet;
    // bytes appended since continue THAT line rather than starting a new one,
    // so its entry has to be re-derived (it may also have been blank then and
    // non-blank now). Everything before it is settled and is reused verbatim.
    let seed_entries = entry_count - 1;
    let last_entry_at = OUTPUT_IDX_HEADER_LEN + (seed_entries * 8) as i64;
    let Ok((_, last_bytes)) = fs.read_at(block_id, IDX, last_entry_at, 8) else { return full() };
    let Ok(last_bytes) = <[u8; 8]>::try_from(last_bytes.as_slice()) else { return full() };
    let scan_start = u64::from_le_bytes(last_bytes);
    if scan_start > output_size {
        return full();
    }

    let seed = if seed_entries == 0 {
        Vec::new()
    } else {
        match fs.read_at(block_id, IDX, OUTPUT_IDX_HEADER_LEN, (seed_entries * 8) as i64) {
            Ok((_, bytes)) if bytes.len() == (seed_entries * 8) as usize => bytes,
            _ => return full(),
        }
    };

    build_output_idx_from(fs, block_id, output_size, scan_start, seed, seed_entries)
}

/// Shared scanner behind both entry points.
///
/// `scan_start` MUST be the byte offset of a line start (0, or an offset read
/// back out of the index). `seed` is the raw offset bytes for the lines before
/// it and `seed_count` how many — they are copied through untouched, so the
/// blank/CRLF rules only ever get applied by the one loop below and the two
/// paths can't drift apart.
fn build_output_idx_from(
    fs: &FileStore,
    block_id: &str,
    output_size: u64,
    scan_start: u64,
    seed: Vec<u8>,
    seed_count: u64,
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
    // The generation this index will be labelled with, taken BEFORE the
    // scan: if `output` is replaced mid-scan it gets a new generation, and
    // this label then marks the index stale instead of vouching for bytes
    // read from two files.
    let for_gen = output_now(fs, block_id).and_then(|(_, gen)| gen);
    tracing::info!(
        block_id = %block_id,
        covered = output_size,
        scan_start,
        scanned_bytes = output_size.saturating_sub(scan_start),
        incremental = scan_start > 0,
        "output.idx rebuild starting"
    );

    // Offsets buffer starts with the covered-size header, then any reused
    // entries for the region before `scan_start`.
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&output_size.to_le_bytes());
    buf.extend_from_slice(&seed);

    let mut line_count: u64 = seed_count;
    let mut cursor: u64 = scan_start; // byte offset where the current line begins
    let mut line_buf: Vec<u8> = Vec::new(); // bytes of the current line, excluding '\n'
    let mut read_pos: i64 = scan_start as i64;

    let flush_line = |line_buf: &mut Vec<u8>,
                      cursor: &mut u64,
                      buf: &mut Vec<u8>,
                      line_count: &mut u64,
                      had_newline: bool| {
        // The reader strips a trailing '\r' (CRLF) and treats trim-empty as
        // blank. Shared with the FileStore line counter, which must agree.
        let is_blank = crate::backend::storage::filestore::is_blank_line(line_buf);
        if !is_blank {
            buf.extend_from_slice(&cursor.to_le_bytes());
            *line_count += 1;
        }
        // Advance cursor past this line's bytes (+1 for the consumed '\n').
        *cursor += line_buf.len() as u64 + if had_newline { 1 } else { 0 };
        line_buf.clear();
    };

    while read_pos < output_size as i64 {
        // From the database, bounded by `output_size`: `read_at` clamps to
        // this process's cached size, which another instance's append can
        // leave behind — the index would then claim lines it never scanned.
        let len = WIN.min(output_size as i64 - read_pos);
        let chunk = match fs.read_bytes_db(block_id, "output", read_pos, len) {
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

    // Index and its generation label in one transaction (created if missing).
    let mut label = std::collections::HashMap::new();
    label.insert(
        IDX_GEN_META.to_string(),
        for_gen.map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    match fs.put_file_with_meta(block_id, IDX, &buf, label) {
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
