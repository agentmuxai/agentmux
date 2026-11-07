// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `blockfile:read_range`: where the lines come from (the `output.idx` index, the
//! whole file, the broker's ring buffer) and how a read is checked and trimmed before it is answered.

use super::*;

type FileStoreRef = Arc<crate::backend::storage::filestore::FileStore>;

/// What the guards at the end of a read need to know about it.
pub(super) struct RangeRead<'a> {
    pub(super) cmd: &'a CommandBlockfileReadRangeData,
    pub(super) offset: usize,
    pub(super) stream: String,
    /// Who signed the transcript's lines (a transcript only).
    pub(super) owner: Option<String>,
    /// `output`'s generation before the lines were read (a transcript only).
    pub(super) gen_before: Option<String>,
}

/// The answer for a read that expected a generation the file is no longer in.
pub(super) fn mismatch(stream: &str, gen: Option<String>) -> BlockfileReadRangeResult {
    BlockfileReadRangeResult { stream: Some(stream.to_string()), gen, gen_mismatch: Some(true), ..Default::default() }
}

impl RangeRead<'_> {
    /// Vets, trims and stamps the lines read, given `output`'s generation after the read.
    pub(super) fn finish(&self, mut result: BlockfileReadRangeResult, gen_after: Option<String>) -> BlockfileReadRangeResult {
        let cmd = self.cmd;
        if let Some(owner) = &self.owner {
            crate::backend::transcript::origin_sig::vet_lines(owner, &mut result.lines);
        }
        if let Some(turns) = cmd.tail_turns.filter(|t| *t > 0) {
            trim_to_last_turns(&mut result, self.offset as u64, turns as usize);
        }
        if let Some(bytes) = cmd.tail_bytes.filter(|b| *b > 0) {
            trim_to_tail_bytes(&mut result, self.offset as u64, bytes as usize);
        }
        if self.gen_before.is_some() && gen_after == self.gen_before {
            result.stream = Some(self.stream.clone());
            result.gen = self.gen_before.clone();
            result
        } else if cmd.expect_gen.is_some() {
            mismatch(&self.stream, gen_after)
        } else {
            result
        }
    }
}

/// The lines `output.idx` serves for this window, or `None` when the whole-file path has to answer.
pub(super) async fn indexed_window(
    filestore: &FileStoreRef,
    read_block: &str,
    block_id: &str,
    offset: usize,
    limit: usize,
    clock: &mut ReadRangeClock,
) -> Option<BlockfileReadRangeResult> {
    // Runs on the blocking pool (#2841). A full rebuild is a
    // streaming scan of `output`, which reaches hundreds of MB
    // on a long-lived agent, and every index/output read in
    // `read_from_index` is blocking file I/O as well — none of it may occupy a
    // Tokio runtime worker. The body (`read_from_index`) is unchanged; only
    // where it runs is.
    let idx_clock = std::sync::Arc::new(std::sync::Mutex::new(IndexClock::default()));
    let idx_result: Option<BlockfileReadRangeResult> = {
        let filestore = filestore.clone();
        let read_block = read_block.to_string();
        let idx_clock = idx_clock.clone();
        let spawned = std::time::Instant::now();
        let compute = move || read_from_index(&filestore, &read_block, offset, limit, &idx_clock, spawned);
        // A panic in the scan must degrade to the slow path
        // (the whole-file read), not fail the read — but it is logged rather
        // than silently swallowed.
        match tokio::task::spawn_blocking(compute).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    "blockfile:read_range: output.idx task failed; falling back"
                );
                None
            }
        }
    };
    clock.index = Some(std::mem::take(&mut *idx_clock.lock().unwrap()));
    idx_result
}

/// The read on the blocking pool: brings `output.idx` up to date and reads the window through it.
fn read_from_index(
    filestore: &FileStoreRef,
    read_block: &str,
    offset: usize,
    limit: usize,
    idx_clock: &std::sync::Mutex<IndexClock>,
    spawned: std::time::Instant,
) -> Option<BlockfileReadRangeResult> {
    use crate::backend::blockcontroller::shell::{extend_output_idx, read_via_index, read_via_stale_index};
    idx_clock.lock().unwrap().queued_ms = ms_since(spawned);
    let run = std::time::Instant::now();
    let out_stat = filestore.stat(read_block, "output").ok()??;
    if out_stat.opts.circular {
        return None; // circular files: fall back to slow path
    }
    // Everything the answer rests on — the index judged fresh,
    // its entries, the output bytes they point at — comes from
    // ONE database snapshot (Codex on #3634), never the stale
    // per-process `stat` cache. A missing or stale index is
    // rebuilt once, for the output as a snapshot saw it, and read
    // again in a new snapshot; if that still doesn't match
    // (replaced again meanwhile), the whole-file path answers.
    // A missing or stale index is brought up to date by
    // extending it from its last line — scanning only the
    // bytes appended since, as `line_count` did before it
    // answered from the counter (5a-3b). A full rebuild on
    // every read of a grown file cost seconds on a large
    // agent zone. `extend_output_idx` still rebuilds when
    // the index can't be a base (another generation's,
    // shrunk, missing).
    let read = match read_via_index(filestore, read_block, offset as u64, limit as u64) {
        Some(read) => read,
        None => {
            let extend = std::time::Instant::now();
            let extended = extend_output_idx(filestore, read_block);
            idx_clock.lock().unwrap().extend_ms = Some(ms_since(extend));
            // A failed extension leaves an index that
            // may be far behind: its prefix isn't a
            // stand-in for the read, so take the
            // whole-file path (#4384).
            extended?;
            match read_via_index(filestore, read_block, offset as u64, limit as u64) {
                Some(read) => read,
                // The extension succeeded, so only an
                // append that landed since (an agent's
                // spawn writing to the transcript its pane
                // is opening) can make this miss: serve
                // the indexed prefix, a few lines short at
                // most, rather than read the whole file
                // (docs/reports/REPORT_AGENT_OPEN_STALL_RCA_2026_10_05.md).
                None => {
                    let read = read_via_stale_index(filestore, read_block, offset as u64, limit as u64)?;
                    idx_clock.lock().unwrap().stale = true;
                    read
                }
            }
        }
    };
    idx_clock.lock().unwrap().run_ms = ms_since(run);
    let total_lines = read.total;

    // Empty result cases — answered from the index, no output read.
    if read.line_offsets.is_empty() {
        return Some(BlockfileReadRangeResult { lines: vec![], total: total_lines, ..Default::default() });
    }
    let raw = read.raw;
    let text = String::from_utf8_lossy(&raw);
    let lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();

    // Receive-time stamps for the returned lines, from the
    // output.tsidx sidecar in the same snapshot as the lines
    // (`read_via_index`; only a window of the sidecar is
    // read). Best-effort: no stamps is never a failed read.
    // Offsets and lines must pair up one to one.
    let stamps = read.stamps.filter(|s| s.len() == lines.len());

    Some(BlockfileReadRangeResult { lines, total: total_lines, stamps, ..Default::default() })
}


/// The lines the broker's ring buffer holds for this file of the block.
pub(super) fn ring_buffer_lines(broker: &Arc<crate::backend::mps::Broker>, cmd: &CommandBlockfileReadRangeData) -> Vec<String> {
    // Fallback: reconstruct from MPS event ring buffer.
    // The ring buffer holds at most MAX_PERSIST = 4096 events;
    // older events are evicted. Offset 0 = oldest retained line.
    let scope = format!("block:{}", cmd.block_id);
    let events = broker.read_event_history(
        crate::backend::mps::EVENT_BLOCK_FILE,
        &scope,
        usize::MAX, // broker clamps to MAX_PERSIST internally
    );

    let mut lines: Vec<String> = Vec::new();
    for event in events {
        let Some(ref event_data) = event.data else { continue };
        let ev_filename = event_data.get("filename")
            .and_then(|v| v.as_str()).unwrap_or("");
        if ev_filename != cmd.filename {
            continue;
        }
        let Some(data64) = event_data.get("data64").and_then(|v| v.as_str()) else { continue };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data64) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            if !line.trim().is_empty() {
                lines.push(line.to_string());
            }
        }
    }
    lines
}


/// The whole file from the store, when it has one worth reading, or the refusal to read a huge one.
/// `None` falls back to the ring buffer.
pub(super) fn whole_file_lines(
    filestore: &FileStoreRef,
    read_block: &str,
    cmd: &CommandBlockfileReadRangeData,
    started: std::time::Instant,
) -> Result<Option<Vec<String>>, String> {
    Ok(match filestore.stat(read_block, &cmd.filename) {
        Ok(Some(ref wf)) if wf.size > 0 => {
            if !whole_file_read_allowed(&cmd.filename, wf.opts.circular, wf.size) {
                tracing::warn!(
                    block_id = %cmd.block_id,
                    size = wf.size,
                    total_ms = ms_since(started),
                    "blockfile:read_range: output.idx can't serve this read and the transcript is too large to read whole; refusing"
                );
                return Err(format!(
                    "blockfile:read_range: the transcript index for {} is unavailable right now; retry",
                    cmd.block_id
                ));
            }
            if wf.size >= WHOLE_FILE_WARN_BYTES {
                tracing::warn!(
                    block_id = %cmd.block_id,
                    filename = %cmd.filename,
                    size = wf.size,
                    "blockfile:read_range: reading the whole file (no usable output.idx)"
                );
            }
            match filestore.read_file(read_block, &cmd.filename) {
                Ok(Some(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let lines: Vec<String> = text.lines()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| l.to_string())
                        .collect();
                    Some(lines)
                }
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!(
                        block_id = %cmd.block_id,
                        filename = %cmd.filename,
                        error = %e,
                        "blockfile:read_range: filestore read failed, falling back to ring buffer"
                    );
                    None
                }
            }
        }
        Ok(_) => None, // file absent or empty → fall back
        Err(e) => {
            tracing::warn!(
                block_id = %cmd.block_id,
                error = %e,
                "blockfile:read_range: filestore stat failed, falling back to ring buffer"
            );
            None
        }
    })
}
