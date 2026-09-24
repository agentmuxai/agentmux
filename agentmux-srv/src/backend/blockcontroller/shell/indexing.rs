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

const IDX: &str = "output.idx";

/// `output`'s size and valid generation, from the database — never this
/// process's `stat` cache, which another srv instance's append makes stale.
pub(crate) fn output_now(fs: &FileStore, zone: &str) -> Option<(u64, Option<String>)> {
    let state = fs.line_state(zone, "output").ok()??;
    Some((state.size.max(0) as u64, state.counted.map(|c| c.gen)))
}

/// `output` and its `output.idx` as they were at ONE moment — one database
/// snapshot (Codex on #3634): separate reads of the index's size, header and
/// label could each see a different version of an index another srv instance
/// is rebuilding, and pass an old count off as fresh.
pub(crate) struct OutputIndex {
    pub output_size: u64,
    /// `output`'s valid generation; `None` when it is not counted.
    pub output_gen: Option<String>,
    /// The line count `output.idx` gives, if it describes `output` exactly:
    /// its covered size is `output_size` and it is labelled with
    /// `output_gen`. `None`: missing, stale, or another generation's.
    pub fresh_lines: Option<u64>,
}

/// See [`OutputIndex`]. `None` when there is no `output`.
pub(crate) fn output_index(fs: &FileStore, zone: &str) -> Option<OutputIndex> {
    let snap = fs.derived_snapshot(zone, "output", IDX, Some(OUTPUT_IDX_HEADER_LEN)).ok()??;
    let output_size = snap.output_size.max(0) as u64;
    let fresh_lines = snap.derived.as_ref().and_then(|idx| {
        let header = idx.bytes.as_deref()?;
        let covered = u64::from_le_bytes(header.get(..8)?.try_into().ok()?);
        (idx.size >= OUTPUT_IDX_HEADER_LEN
            && covered == output_size
            && labelled_for(&idx.meta, snap.output_gen.as_deref()))
        .then_some(((idx.size - OUTPUT_IDX_HEADER_LEN) / 8) as u64)
    });
    Some(OutputIndex { output_size, output_gen: snap.output_gen, fresh_lines })
}

/// Whether an index with metadata `meta` may describe `output` of generation
/// `output_gen`, apart from its covered size.
///
/// A size match alone is not proof: `output` replaced or restored with
/// content that happens to reach the old covered size would have its lines
/// read at another file's offsets. When `output` has a valid generation the
/// index must be labelled with it; any mismatch — unlabelled (older build),
/// or built for another generation — means rebuild, which is always safe.
/// An uncounted `output` (legacy or written by an older build) keeps the
/// covered-size rule alone, as before.
fn labelled_for(meta: &crate::backend::storage::filestore::FileMeta, output_gen: Option<&str>) -> bool {
    match output_gen {
        None => true,
        Some(gen) => meta.get(IDX_GEN_META).and_then(|v| v.as_str()) == Some(gen),
    }
}

/// Lines `[offset, offset + limit)` of `output`, read through `output.idx`.
pub(crate) struct IndexedRead {
    /// Total line count of `output` in the snapshot.
    pub total: u64,
    pub output_size: u64,
    pub output_gen: Option<String>,
    /// The bytes of the requested lines (from the first line's offset up to
    /// the next line's, or the end of `output`).
    pub raw: Vec<u8>,
    /// Byte offset in `output` of each returned line, in order.
    pub line_offsets: Vec<u64>,
}

/// Read lines `[offset, offset + limit)` of `output` through a fresh
/// `output.idx`, with the freshness check, the index entries and the output
/// bytes all from ONE database snapshot (Codex on #3634): validating in one
/// read and fetching entries and bytes in later ones would let another srv
/// instance replace `output` and rebuild the index in between — the answer
/// would mix the new file's lines with the old file's total.
///
/// `None` when the index doesn't describe `output` in that snapshot (missing,
/// stale, another generation's) or a read hits bytes not stored yet: the
/// caller rebuilds the index and retries once, or takes its slow path.
pub(crate) fn read_via_index(fs: &FileStore, zone: &str, offset: u64, limit: u64) -> Option<IndexedRead> {
    const H: i64 = OUTPUT_IDX_HEADER_LEN;
    fs.read_snapshot(|snap| {
        let Some(out) = snap.file(zone, "output")? else { return Ok(None) };
        let Some(idx) = snap.file(zone, IDX)? else { return Ok(None) };
        let Some(header) = snap.bytes(zone, IDX, 0, H)? else { return Ok(None) };
        let covered = u64::from_le_bytes(header.as_slice().try_into().unwrap_or([0; 8]));
        let output_size = out.size.max(0) as u64;
        if idx.size < H || covered != output_size || !labelled_for(&idx.meta, out.gen.as_deref()) {
            return Ok(None);
        }
        let total = ((idx.size - H) / 8) as u64;
        let mut read = IndexedRead { total, output_size, output_gen: out.gen.clone(), raw: Vec::new(), line_offsets: Vec::new() };
        if limit == 0 || offset >= total {
            return Ok(Some(read));
        }
        let count = limit.min(total - offset);
        // Entries for the returned lines, plus the next line's (the end of
        // the last returned line) when there is one.
        let want = count + u64::from(offset + count < total);
        let Some(entries) = snap.bytes(zone, IDX, H + (offset * 8) as i64, (want * 8) as i64)? else { return Ok(None) };
        let at = |k: u64| u64::from_le_bytes(entries[(k * 8) as usize..(k * 8 + 8) as usize].try_into().unwrap());
        let start = at(0);
        let end = if want > count { at(count) } else { output_size };
        if start > end || end > output_size {
            return Ok(None);
        }
        let Some(raw) = snap.bytes(zone, "output", start as i64, (end - start) as i64)? else { return Ok(None) };
        read.raw = raw;
        read.line_offsets = (0..count).map(at).collect();
        Ok(Some(read))
    })
    .ok()
    .flatten()
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
/// `output_size` and `output_gen` must be read together (`output_index`, or
/// `output_now`): the index is published only if `output` is still
/// `output_gen`, so a size taken from one file and a generation from its
/// replacement would publish a partial index under the new generation (Codex
/// on #3634).
///
/// Returns the number of indexed (non-blank) lines on success, or `None` if
/// `output` is unreadable, was replaced during the scan, or the index write
/// fails (caller falls back to slow path).
pub(crate) fn rebuild_output_idx(
    fs: &FileStore,
    block_id: &str,
    output_size: u64,
    output_gen: Option<String>,
) -> Option<u64> {
    build_output_idx_from(fs, block_id, output_size, 0, Vec::new(), 0, output_gen)
}

/// Bring an existing `output.idx` up to `output`'s current size by scanning
/// ONLY the appended bytes, instead of rescanning from byte zero.
///
/// Falls back to a full rebuild whenever the existing index can't be trusted
/// as a base: missing, header unreadable, no entries yet, built for another
/// generation, or `output` shrank below what the index already covers
/// (rotation/truncation).
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
pub(crate) fn extend_output_idx(fs: &FileStore, block_id: &str) -> Option<u64> {
    // `output` and the whole existing index in one snapshot: the seed entries,
    // the covered size, the label and the output state all describe the same
    // moment, and the guarded write below refuses if `output` has since been
    // replaced.
    let snap = fs.derived_snapshot(block_id, "output", IDX, None).ok()??;
    let output_size = snap.output_size.max(0) as u64;
    let output_gen = snap.output_gen;
    let full = || build_output_idx_from(fs, block_id, output_size, 0, Vec::new(), 0, output_gen.clone());

    let Some(idx) = snap.derived else { return full() };
    // An index built for another generation of `output` is no base at all.
    if !labelled_for(&idx.meta, output_gen.as_deref()) {
        return full();
    }
    let Some(bytes) = idx.bytes else { return full() };
    if bytes.len() < OUTPUT_IDX_HEADER_LEN as usize {
        return full();
    }
    let entry_count = ((bytes.len() - OUTPUT_IDX_HEADER_LEN as usize) / 8) as u64;
    let Ok(header) = <[u8; 8]>::try_from(&bytes[..8]) else { return full() };
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
    let last_at = OUTPUT_IDX_HEADER_LEN as usize + seed_entries as usize * 8;
    let Ok(last) = <[u8; 8]>::try_from(&bytes[last_at..last_at + 8]) else { return full() };
    let scan_start = u64::from_le_bytes(last);
    if scan_start > output_size {
        return full();
    }
    let seed = bytes[OUTPUT_IDX_HEADER_LEN as usize..last_at].to_vec();

    build_output_idx_from(fs, block_id, output_size, scan_start, seed, seed_entries, output_gen.clone())
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
    for_gen: Option<String>,
) -> Option<u64> {
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
    // `for_gen`: the generation of `output` this index is built for, taken by
    // the caller BEFORE anything it builds on (seed entries, scanned bytes)
    // was read. The write below only lands if it is still current.
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

    // Index and its generation label in one transaction (created if missing),
    // and only if `output` is still that generation (Codex on #3634): a
    // replace during the scan must neither publish an index of the old bytes
    // nor hand this caller offsets it would then apply to the new file.
    let mut label = std::collections::HashMap::new();
    label.insert(
        IDX_GEN_META.to_string(),
        for_gen.clone().map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    match fs.put_file_with_meta_if(block_id, IDX, &buf, label, "output", for_gen.as_deref()) {
        Ok(false) => {
            tracing::info!(
                block_id = %block_id,
                duration_ms = started.elapsed().as_millis() as u64,
                "output.idx rebuild discarded: output was replaced during the scan"
            );
            None
        }
        Ok(true) => {
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    #[test]
    fn a_rebuild_for_a_generation_that_is_no_longer_current_publishes_nothing() {
        // Codex P2 on #3634: `output` replaced after the builder captured its
        // generation. The index of the old bytes must not be written, and the
        // caller must not get a count (it would apply those offsets to the new
        // file).
        let fs = FileStore::open_in_memory().unwrap();
        let zone = "blk-idx-guard";
        fs.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
        fs.append_lines(zone, "output", b"a\nb\nc\n").unwrap();
        let old = output_now(&fs, zone).and_then(|(_, g)| g);
        fs.write_file(zone, "output", b"x\n").unwrap();

        assert_eq!(build_output_idx_from(&fs, zone, 6, 0, Vec::new(), 0, old), None);
        assert!(fs.line_state(zone, "output.idx").unwrap().is_none(), "no index may be published");

        // With the current generation it builds and publishes.
        assert_eq!(rebuild_output_idx(&fs, zone, 2, output_now(&fs, zone).and_then(|(_, g)| g)), Some(1));
        assert!(fs.line_state(zone, "output.idx").unwrap().is_some());
    }

    #[test]
    fn a_read_through_the_index_comes_from_one_snapshot_or_not_at_all() {
        let fs = FileStore::open_in_memory().unwrap();
        let zone = "blk-idx-read";
        fs.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
        fs.append_lines(zone, "output", b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        // No index yet: nothing to read through.
        assert!(read_via_index(&fs, zone, 0, 10).is_none());
        let view = output_index(&fs, zone).unwrap();
        assert_eq!(rebuild_output_idx(&fs, zone, view.output_size, view.output_gen), Some(3));

        let r = read_via_index(&fs, zone, 1, 10).unwrap();
        assert_eq!((r.total, r.line_offsets.clone()), (3, vec![8, 16]));
        assert_eq!(r.raw, b"{\"b\":2}\n{\"c\":3}\n");
        let r = read_via_index(&fs, zone, 0, 1).unwrap();
        assert_eq!((r.line_offsets.clone(), r.raw), (vec![0], b"{\"a\":1}\n".to_vec()));
        assert!(read_via_index(&fs, zone, 5, 10).unwrap().line_offsets.is_empty());

        // Replaced by content of the same size and a different layout: the
        // index no longer describes it, so nothing is read through it.
        fs.write_file(zone, "output", b"{\"x\":11111}\n{\"y\":22222}\n").unwrap();
        assert_eq!(fs.line_state(zone, "output").unwrap().unwrap().size, 24);
        assert!(read_via_index(&fs, zone, 0, 10).is_none());
    }
}
