// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent:open_tail`: an agent pane's first screen from one small read.
//! docs/specs/SPEC_AGENT_OPEN_FIRST_SCREEN_THEN_REST_2026_10_19.md §3.1
//!
//! The pane used to open with three or four calls (the snapshot, the line
//! count, a range of up to 10,000 lines) before it showed anything, and those
//! could write under the store's single write lock: extending `output.idx`,
//! catching the line counter up, a `stat` on a cache miss. This returns the
//! newest ~64 KB of the transcript, the line positions the live feed resumes
//! from, and the snapshot overlay, from **one reader snapshot**: it takes the
//! WAL reader pool only, never the writer mutex, and never writes or extends
//! an index.
//!
//! Line positions come from the line counter, caught up in memory when the
//! file has grown past it (`SnapshotReader::tail_state`). A transcript with no
//! usable counter epoch comes back without `stream`/`gen`/line positions and
//! the pane opens the way it always did.

use super::blockfile::{trim_to_last_turns, trim_to_tail_bytes};
use super::*;

use crate::backend::storage::filestore::{is_blank_line, FileStore, SnapshotReader};
use crate::backend::storage::StoreError;

/// A first screen: about what fits one pane, plus a margin.
const DEFAULT_TAIL_BYTES: u64 = 64 * 1024;
/// The most a counter that is only behind is caught up by in memory.
const MAX_CATCH_UP_BYTES: i64 = 4 << 20;
/// The most a tail read grows to when the newest line is larger than the window.
const MAX_WINDOW_BYTES: i64 = 16 << 20;
/// How much of the receive-time sidecar's end is read to stamp the slice.
const TSIDX_WINDOW_BYTES: i64 = 64 * 1024;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();
    let global_store = state.global_transcript_store.clone();
    engine.register_typed(COMMAND_AGENT_OPEN_TAIL, move |cmd: CommandAgentOpenTailData, _ctx| {
        let mstore = mstore.clone();
        let filestore = filestore.clone();
        let global_store = global_store.clone();
        async move {
            let started = std::time::Instant::now();
            let block_id = cmd.block_id.clone();
            let result = tokio::task::spawn_blocking(move || open_tail(&filestore, global_store.as_ref(), &mstore, cmd))
                .await
                .map_err(|e| format!("agent:open_tail: {e}"))??;
            tracing::info!(
                block_id = %block_id,
                lines = result.lines.len(),
                size = result.size.unwrap_or(0),
                counted = result.gen.is_some(),
                ms = started.elapsed().as_millis() as u64,
                "agent:open_tail"
            );
            Ok(result)
        }
    });
}

/// Which store and zone a block's transcript is read from: the agent's global
/// zone when it has content, else the block's own (the choice
/// `global_output_source` makes, decided here from a reader snapshot rather
/// than `stat`, which takes the writer mutex on a cache miss).
fn pick_source(
    filestore: &Arc<FileStore>,
    global: Option<&Arc<FileStore>>,
    mstore: &Arc<Store>,
    block_id: &str,
) -> (Arc<FileStore>, String, String) {
    let local = || (filestore.clone(), block_id.to_string(), format!("b:{block_id}"));
    let Some(gfs) = global else { return local() };
    let Some(block) = mstore.get::<Block>(block_id).ok().flatten() else { return local() };
    let archived = block
        .meta
        .get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT)
        .and_then(|v| v.as_i64())
        .map(|v| v > 0)
        .unwrap_or(false);
    if archived {
        return local();
    }
    let Some(zone) = crate::backend::agent_session::agent_zone_for_block_meta(&block.meta) else { return local() };
    let size = gfs
        .read_snapshot(|r| Ok(r.file(&zone, "output")?.map(|f| f.size)))
        .ok()
        .flatten()
        .unwrap_or(0);
    if size > 0 {
        let stream = format!("g:{zone}");
        (gfs.clone(), zone, stream)
    } else {
        local()
    }
}

/// What one pass over the reader snapshot found.
enum Read {
    /// A slice, with where it sits.
    Slice(Slice),
    /// `expect_gen` is not the transcript's generation (named here).
    Mismatch(String),
    /// No usable counter epoch, or the slice could not be read: no line positions.
    Uncounted { size: i64 },
}

struct Slice {
    gen: String,
    size: i64,
    /// `(byte offset, line)` of every returned line, oldest first.
    lines: Vec<(i64, String)>,
    stamps: Option<Vec<i64>>,
    /// Byte offset just past the last complete line.
    end_byte: i64,
    /// Complete non-blank lines in the stream at the snapshot.
    total_lines: u64,
}

/// The newest `want` bytes (the newest line always) of the complete lines in
/// `[0, complete_end)`, with the non-blank lines in them and each one's byte
/// offset. `None` when the bytes aren't all stored or the newest line is
/// larger than [`MAX_WINDOW_BYTES`].
fn read_window(r: &SnapshotReader<'_>, zone: &str, complete_end: i64, want: i64) -> Result<Option<Vec<(i64, String)>>, StoreError> {
    let mut window = want.max(1);
    loop {
        let start = (complete_end - window).max(0);
        // One byte before the window says whether it starts on a line start.
        let pre = i64::from(start > 0);
        let Some(buf) = r.bytes(zone, "output", start - pre, complete_end - start + pre)? else { return Ok(None) };
        // Offset in `buf` of the first byte that starts a whole line.
        let first = if start == 0 {
            Some(0usize)
        } else if buf.first() == Some(&b'\n') {
            Some(1usize)
        } else {
            buf.iter().position(|b| *b == b'\n').map(|i| i + 1)
        };
        let mut lines = Vec::new();
        if let Some(first) = first {
            let base = start - pre + first as i64;
            let mut at = first;
            while at < buf.len() {
                let end = buf[at..].iter().position(|b| *b == b'\n').map(|i| at + i).unwrap_or(buf.len());
                let line = &buf[at..end];
                if !is_blank_line(line) {
                    lines.push((base + (at - first) as i64, String::from_utf8_lossy(line).into_owned()));
                }
                at = end + 1;
            }
        }
        if !lines.is_empty() || start == 0 {
            return Ok(Some(lines));
        }
        // No whole line fits: the newest line is larger than the window.
        if window >= MAX_WINDOW_BYTES.min(complete_end) {
            return Ok(None);
        }
        window = (window * 2).min(MAX_WINDOW_BYTES);
    }
}

/// Receive-time stamps for lines starting at `offsets`, from the end of
/// `output.tsidx` (`{"off":<batch start offset>,"ms":<unix ms>}` per batch):
/// the newest batch that began at or before the line. `None` without a sidecar.
fn read_stamps(r: &SnapshotReader<'_>, zone: &str, offsets: &[i64]) -> Result<Option<Vec<i64>>, StoreError> {
    let Some(file) = r.file(zone, crate::backend::agent_session::TSIDX_FILE)? else { return Ok(None) };
    if file.size <= 0 {
        return Ok(None);
    }
    let from = (file.size - TSIDX_WINDOW_BYTES).max(0);
    let Some(buf) = r.bytes(zone, crate::backend::agent_session::TSIDX_FILE, from, file.size - from)? else { return Ok(None) };
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    if from > 0 {
        lines.next(); // a fragment
    }
    let mut entries: Vec<(i64, i64)> = lines
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| Some((v.get("off")?.as_i64()?, v.get("ms")?.as_i64()?)))
        .collect();
    entries.sort_by_key(|e| e.0);
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        offsets
            .iter()
            .map(|o| {
                let i = entries.partition_point(|e| e.0 <= *o);
                if i == 0 { 0 } else { entries[i - 1].1 }
            })
            .collect(),
    ))
}

fn read_slice(r: &SnapshotReader<'_>, zone: &str, cmd: &CommandAgentOpenTailData) -> Result<Read, StoreError> {
    let Some(state) = r.tail_state(zone, "output", MAX_CATCH_UP_BYTES)? else {
        let size = r.file(zone, "output")?.map(|f| f.size).unwrap_or(0);
        return Ok(Read::Uncounted { size });
    };
    if let Some(expected) = &cmd.expect_gen {
        if *expected != state.gen {
            return Ok(Read::Mismatch(state.gen));
        }
    }
    let want = cmd.tail_bytes.filter(|b| *b > 0).unwrap_or(DEFAULT_TAIL_BYTES) as i64;
    // Read twice the budget, so the trimming below sees lines it must drop and
    // moves the start up to a turn start the way `read_range`'s tail_bytes does
    // (it only does that when the byte budget cut something).
    let Some(window) = read_window(r, zone, state.complete_end, want.saturating_mul(2))? else {
        return Ok(Read::Uncounted { size: state.size });
    };
    // A counter that disagrees with the bytes (more lines read than it counts) can't position anything.
    if window.len() as u64 > state.lines {
        return Ok(Read::Uncounted { size: state.size });
    }
    let offsets: Vec<i64> = window.iter().map(|(o, _)| *o).collect();
    let stamps = read_stamps(r, zone, &offsets)?.filter(|s| s.len() == window.len());

    // Trim by the rules `blockfile:read_range` uses, on the same shape.
    let mut trimmed = BlockfileReadRangeResult {
        lines: window.iter().map(|(_, l)| l.clone()).collect(),
        stamps,
        ..Default::default()
    };
    if let Some(turns) = cmd.tail_turns.filter(|t| *t > 0) {
        trim_to_last_turns(&mut trimmed, 0, turns as usize);
    }
    trim_to_tail_bytes(&mut trimmed, 0, want as usize);
    let cut = window.len() - trimmed.lines.len();
    let lines: Vec<(i64, String)> = window.into_iter().skip(cut).collect();
    Ok(Read::Slice(Slice {
        gen: state.gen,
        size: state.size,
        lines,
        stamps: trimmed.stamps,
        end_byte: state.complete_end,
        total_lines: state.lines,
    }))
}

/// The agent's session snapshot, from reader snapshots: the global store first
/// (it always holds the agent-anchored copy), the block's store as a fallback,
/// as `agent:session:read` does.
fn read_overlay(filestore: &Arc<FileStore>, global: Option<&Arc<FileStore>>, definition_id: &str) -> Option<(String, i64)> {
    use crate::backend::agent_session::{agent_current_zone, is_valid_definition_id, SNAPSHOT_FILE};
    if !is_valid_definition_id(definition_id) {
        return None;
    }
    let zone = agent_current_zone(definition_id);
    let from = |fs: &Arc<FileStore>| -> Option<(String, i64)> {
        fs.read_snapshot(|r| {
            let Some(file) = r.file(&zone, SNAPSHOT_FILE)? else { return Ok(None) };
            let bytes = r.bytes(&zone, SNAPSHOT_FILE, 0, file.size)?.unwrap_or_default();
            Ok(Some((String::from_utf8_lossy(&bytes).into_owned(), file.modts)))
        })
        .ok()
        .flatten()
    };
    global.and_then(from).or_else(|| from(filestore))
}

fn open_tail(
    filestore: &Arc<FileStore>,
    global: Option<&Arc<FileStore>>,
    mstore: &Arc<Store>,
    cmd: CommandAgentOpenTailData,
) -> Result<AgentOpenTailResult, String> {
    if cmd.filename != crate::backend::agent_session::OUTPUT_FILE {
        return Err(format!("agent:open_tail: only `{}` has a tail", crate::backend::agent_session::OUTPUT_FILE));
    }
    let (fs, zone, stream) = pick_source(filestore, global, mstore, &cmd.block_id);
    let read = fs
        .read_snapshot(|r| read_slice(r, &zone, &cmd))
        .map_err(|e| format!("agent:open_tail: {e}"))?;
    let mut out = match read {
        Read::Mismatch(gen) => AgentOpenTailResult {
            stream: Some(stream),
            gen: Some(gen),
            gen_mismatch: Some(true),
            ..Default::default()
        },
        Read::Uncounted { size } => AgentOpenTailResult { size: Some(size.max(0) as u64), ..Default::default() },
        Read::Slice(slice) => {
            let end_line = slice.total_lines;
            let start_line = end_line - slice.lines.len() as u64;
            let start_byte = slice.lines.first().map(|(o, _)| *o).unwrap_or(slice.end_byte);
            AgentOpenTailResult {
                stamps: slice.stamps,
                stream: Some(stream),
                gen: Some(slice.gen),
                start_line: Some(start_line),
                end_line: Some(end_line),
                total_lines: Some(end_line),
                start_byte: Some(start_byte.max(0) as u64),
                end_byte: Some(slice.end_byte.max(0) as u64),
                size: Some(slice.size.max(0) as u64),
                has_more_before: Some(start_line > 0),
                lines: slice.lines.into_iter().map(|(_, l)| l).collect(),
                ..Default::default()
            }
        }
    };
    if let Some(def_id) = cmd.definition_id.as_deref().filter(|d| !d.is_empty()) {
        if let Some((content, modts)) = read_overlay(filestore, global, def_id) {
            out.snapshot = Some(content);
            out.snapshot_modts = Some(modts);
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "open_tail_tests.rs"]
mod open_tail_tests;
