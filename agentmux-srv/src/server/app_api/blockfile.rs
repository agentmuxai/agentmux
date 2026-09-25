use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_blockfile_line_count(engine, state);
    register_blockfile_read_range(engine, state);
    register_blockfile_read_state(engine, state);
    register_blockfile_write_state(engine, state);
}

fn register_blockfile_line_count(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();
    let global_store = state.global_transcript_store.clone();

    engine.register_typed(
        COMMAND_BLOCKFILE_LINE_COUNT,
        move |cmd: CommandBlockfileLineCountData, _ctx| {
            let broker = broker.clone();
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            let global_store = global_store.clone();
            async move {

                tracing::info!(block_id = %cmd.block_id, filename = %cmd.filename, "blockfile:line_count");

                // Cross-channel fallback (checked first): when this channel has
                // no local `output` for the block, the local `session:line_count`
                // meta is absent/stale, so a fresh cross-channel open would
                // report 0 lines and the pane would render empty. Count from the
                // agent's GLOBAL transcript zone instead. See
                // `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
                if let Some((gfs, zone)) =
                    global_output_source(&filestore, &global_store, &mstore, &cmd.block_id, &cmd.filename)
                {
                    // Blocking pool (#2841). A counted zone answers from its
                    // counter (O(1), Phase 5a-3); a zone without an epoch gets
                    // one first (`init_line_counter`, one read of the file,
                    // once). Only if it can't be counted does this extend or
                    // rebuild `output.idx`, as before.
                    let count = match tokio::task::spawn_blocking(move || {
                        let stream = format!("g:{zone}");
                        counted_line_count(&gfs, &zone, stream).or_else(|| {
                            global_zone_line_count(&gfs, &zone)
                                .map(|count| BlockfileLineCountResult { count, ..Default::default() })
                        })
                    })
                    .await
                    {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                block_id = %cmd.block_id,
                                error = %e,
                                "blockfile:line_count: global zone count task failed; falling back"
                            );
                            None
                        }
                    };
                    if let Some(result) = count {
                        return Ok(result);
                    }
                }

                // The block's own `output`, from its counter when it is a
                // counted transcript (Phase 5a-3): exact, O(1), and the same
                // count `read_range` serves lines from.
                if cmd.filename == crate::backend::agent_session::OUTPUT_FILE {
                    let (fs, block_id) = (filestore.clone(), cmd.block_id.clone());
                    let counted = tokio::task::spawn_blocking(move || {
                        let stream = format!("b:{block_id}");
                        counted_line_count(&fs, &block_id, stream)
                    })
                    .await
                    .ok()
                    .flatten();
                    if let Some(result) = counted {
                        return Ok(result);
                    }
                }

                // Fast path: read session:line_count meta (O(1), maintained
                // by SessionStatsAccumulator). For "output" filename this is
                // the authoritative total — matches the unbounded counter
                // that SessionStats increments on every line. FileStore's
                // persisted line count will trail meta by up to the debounce
                // interval (1s), and reading the full file just to count
                // lines is O(file size) which defeats the point of a fast
                // line_count endpoint.
                if cmd.filename == "output" {
                    if let Ok(Some(block)) = mstore.get::<Block>(&cmd.block_id) {
                        if let Some(count) = block.meta.get("session:line_count").and_then(|v| v.as_u64()) {
                            return Ok(BlockfileLineCountResult { count, ..Default::default() });
                        }
                    }
                }

                // Fallback: count from MPS event ring buffer (capped at MAX_PERSIST = 4096).
                let scope = format!("block:{}", cmd.block_id);
                let events = broker.read_event_history(
                    crate::backend::mps::EVENT_BLOCK_FILE,
                    &scope,
                    usize::MAX, // broker clamps to MAX_PERSIST internally
                );

                let mut count: u64 = 0;
                for event in events {
                    if let Some(ref event_data) = event.data {
                        let ev_filename = event_data.get("filename")
                            .and_then(|v| v.as_str()).unwrap_or("");
                        if ev_filename != cmd.filename {
                            continue;
                        }
                        if let Some(data64) = event_data.get("data64").and_then(|v| v.as_str()) {
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data64) {
                                let text = String::from_utf8_lossy(&bytes);
                                for line in text.lines() {
                                    if !line.trim().is_empty() {
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                Ok(BlockfileLineCountResult { count, ..Default::default() })
            }
        },
    );
}

fn register_blockfile_read_range(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();
    let filestore = state.filestore.clone();
    let global_store = state.global_transcript_store.clone();
    let mstore = state.mstore.clone();

    engine.register_typed(
        COMMAND_BLOCKFILE_READ_RANGE,
        move |cmd: CommandBlockfileReadRangeData, _ctx| {
            let broker = broker.clone();
            let filestore = filestore.clone();
            let global_store = global_store.clone();
            let mstore = mstore.clone();
            async move {

                tracing::info!(block_id = %cmd.block_id, filename = %cmd.filename, offset = cmd.offset, limit = cmd.limit, "blockfile:read_range");

                let limit = cmd.limit.min(10_000) as usize;
                let offset = cmd.offset as usize;
                let end = offset.saturating_add(limit);

                // Cross-channel fallback: when this channel has no local `output`
                // for the block, read the agent's GLOBAL transcript zone
                // (`agent:<defId>:current`) instead. `read_block` is the zone for
                // every FileStore call below — the local block_id normally, the
                // agent zone when the agent ran in another build/channel.
                let source = global_output_source(&filestore, &global_store, &mstore, &cmd.block_id, &cmd.filename);
                let stream = match &source {
                    Some((_, zone)) => format!("g:{zone}"),
                    None => format!("b:{}", cmd.block_id),
                };
                let (filestore, read_block) = source.unwrap_or_else(|| (filestore.clone(), cmd.block_id.clone()));

                // Generation (Phase 5a-3): read before and after the lines. A
                // replace always mints a new one, so the same value on both
                // sides proves every line came from that generation — only then
                // does the response name it. `expect_gen` turns any other
                // outcome into `gen_mismatch` instead of lines of another file.
                let transcript = cmd.filename == crate::backend::agent_session::OUTPUT_FILE;
                let gen_before = if transcript { db_generation(&filestore, &read_block).await } else { None };
                let mismatch = |gen: Option<String>| BlockfileReadRangeResult {
                    stream: Some(stream.clone()),
                    gen,
                    gen_mismatch: Some(true),
                    ..Default::default()
                };
                if let Some(expected) = &cmd.expect_gen {
                    if gen_before.as_deref() != Some(expected.as_str()) {
                        return Ok(mismatch(gen_before));
                    }
                }
                let finish = |mut result: BlockfileReadRangeResult, gen_after: Option<String>| {
                    if gen_before.is_some() && gen_after == gen_before {
                        result.stream = Some(stream.clone());
                        result.gen = gen_before.clone();
                        result
                    } else if cmd.expect_gen.is_some() {
                        mismatch(gen_after)
                    } else {
                        result
                    }
                };

                // Fast path: output.idx — a lazily-built, self-validating byte-offset
                // index of every non-blank line in `output`. It lets us seek directly
                // to the requested line range instead of loading the whole file.
                //
                // The index is a pure cache of `output`: its 8-byte header records
                // the `output` size it was built for. If that equals `output`'s
                // current size (and it is labelled with `output`'s generation) the
                // index is fresh; otherwise THIS path extends it (`extend_output_idx`,
                // #2838): it scans just the bytes appended since, re-deriving the
                // last indexed line so a straddling partial line isn't
                // double-counted, and rebuilds from byte 0 only when the index
                // can't be a base (missing, shrunk, another generation's).
                //
                // Gated to non-circular files: circular `output` (terminal ring buffers)
                // drops early bytes, so absolute byte offsets wouldn't map cleanly.
                use crate::backend::blockcontroller::shell::{extend_output_idx, read_via_index};
                if cmd.filename == "output" {
                    // Runs on the blocking pool (#2841). A full rebuild is a
                    // streaming scan of `output`, which reaches hundreds of MB
                    // on a long-lived agent, and every index/output read below
                    // is blocking file I/O as well — none of it may occupy a
                    // Tokio runtime worker. The closure body is unchanged; only
                    // where it runs is.
                    let idx_result: Option<BlockfileReadRangeResult> = {
                        let filestore = filestore.clone();
                        let read_block = read_block.clone();
                        let compute = move || -> Option<BlockfileReadRangeResult> {
                        let out_stat = filestore.stat(&read_block, "output").ok()??;
                        if out_stat.opts.circular {
                            return None; // circular files: fall back to slow path
                        }
                        // Everything the answer rests on — the index judged fresh,
                        // its entries, the output bytes they point at — comes from
                        // ONE database snapshot (Codex on #3634), never the stale
                        // per-process `stat` cache. A missing or stale index is
                        // rebuilt once, for the output as a snapshot saw it, and read
                        // again in a new snapshot; if that still doesn't match
                        // (replaced again meanwhile), the slow path below answers.
                        // A missing or stale index is brought up to date by
                        // extending it from its last line — scanning only the
                        // bytes appended since, as `line_count` did before it
                        // answered from the counter (5a-3b). A full rebuild on
                        // every read of a grown file cost seconds on a large
                        // agent zone. `extend_output_idx` still rebuilds when
                        // the index can't be a base (another generation's,
                        // shrunk, missing).
                        let read = match read_via_index(&filestore, &read_block, offset as u64, limit as u64) {
                            Some(read) => read,
                            None => {
                                extend_output_idx(&filestore, &read_block)?;
                                read_via_index(&filestore, &read_block, offset as u64, limit as u64)?
                            }
                        };
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
                        };
                        // A panic in the scan must degrade to the slow path
                        // below, not fail the read — but it is logged rather
                        // than silently swallowed.
                        match tokio::task::spawn_blocking(compute).await {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!(
                                    block_id = %cmd.block_id,
                                    error = %e,
                                    "blockfile:read_range: output.idx task failed; falling back"
                                );
                                None
                            }
                        }
                    };
                    if let Some(result) = idx_result {
                        tracing::debug!(
                            block_id = %cmd.block_id,
                            offset,
                            limit,
                            lines = result.lines.len(),
                            "blockfile:read_range via output.idx fast path"
                        );
                        let gen_after = if transcript { db_generation(&filestore, &read_block).await } else { None };
                        return Ok(finish(result, gen_after));
                    }
                }

                // Phase 1.3: Prefer FileStore (persistent, no size cap) over the
                // MPS broker ring buffer (MAX_PERSIST = 4096 events).
                //
                // If FileStore has the file and it is non-empty, read from disk.
                // Otherwise fall back to ring buffer for backward compatibility.
                let filestore_lines = match filestore.stat(&read_block, &cmd.filename) {
                    Ok(Some(ref wf)) if wf.size > 0 => {
                        match filestore.read_file(&read_block, &cmd.filename) {
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
                };

                let all_lines = if let Some(lines) = filestore_lines {
                    lines
                } else {
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
                };

                let total = all_lines.len() as u64;
                let clamped_offset = offset.min(all_lines.len());
                let clamped_end = end.min(all_lines.len());
                let lines: Vec<String> = if clamped_offset >= clamped_end {
                    Vec::new()
                } else {
                    all_lines[clamped_offset..clamped_end].to_vec()
                };

                let gen_after = if transcript { db_generation(&filestore, &read_block).await } else { None };
                Ok(finish(BlockfileReadRangeResult { lines, total, ..Default::default() }, gen_after))
            }
        },
    );
}

fn register_blockfile_read_state(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let filestore = state.filestore.clone();
    engine.register_typed(
        COMMAND_BLOCKFILE_READ_STATE,
        move |cmd: CommandBlockfileReadStateData, _ctx| {
            let filestore = filestore.clone();
            async move {
                if cmd.filename.contains('/') || cmd.filename.contains('\\') || cmd.filename.contains("..") {
                    return Err("blockfile:read_state: filename must not contain path separators".to_string());
                }
                tracing::debug!(block_id = %cmd.block_id, filename = %cmd.filename, "blockfile:read_state");

                let content = match filestore.read_file(&cmd.block_id, &cmd.filename) {
                    Ok(Some(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                    Ok(None) => None,
                    Err(e) => {
                        // NotFound is the common case (no snapshot yet). Suppress.
                        if matches!(e, crate::backend::storage::StoreError::NotFound) {
                            None
                        } else {
                            tracing::warn!(block_id = %cmd.block_id, error = %e, "blockfile:read_state: read failed");
                            None
                        }
                    }
                };

                Ok(BlockfileReadStateResult { content })
            }
        },
    );
}

fn register_blockfile_write_state(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let filestore = state.filestore.clone();
    engine.register_typed(
        COMMAND_BLOCKFILE_WRITE_STATE,
        move |cmd: CommandBlockfileWriteStateData, _ctx| {
            let filestore = filestore.clone();
            async move {
                if cmd.filename.contains('/') || cmd.filename.contains('\\') || cmd.filename.contains("..") {
                    return Err("blockfile:write_state: filename must not contain path separators".to_string());
                }
                // The transcript and its sidecars are written only by the
                // transcript paths, which keep its generation and sidecars
                // consistent (5a-2b). This RPC is for pane state files; letting
                // it replace `output` would leave `output.idx` / `output.tsidx`
                // describing other bytes.
                if matches!(cmd.filename.as_str(), "output" | "output.idx" | "output.tsidx") {
                    return Err(format!(
                        "blockfile:write_state: {} is a transcript file and can't be written through this RPC",
                        cmd.filename
                    ));
                }
                let bytes = cmd.content.as_bytes();
                let bytes_written = bytes.len() as u64;
                tracing::debug!(block_id = %cmd.block_id, filename = %cmd.filename, bytes = bytes_written, "blockfile:write_state");

                // FileStore.write_file is atomic at the DB level (single
                // tx replaces all data parts) — no torn write surfaces.
                // Need make_file first if the sidecar doesn't yet exist.
                use crate::backend::storage::filestore::{FileMeta, FileOpts};
                use crate::backend::storage::StoreError;
                match filestore.write_file(&cmd.block_id, &cmd.filename, bytes) {
                    Ok(()) => {}
                    Err(StoreError::NotFound) => {
                        filestore
                            .make_file(&cmd.block_id, &cmd.filename, FileMeta::default(), FileOpts::default())
                            .map_err(|e| format!("blockfile:write_state: make_file: {e}"))?;
                        filestore
                            .write_file(&cmd.block_id, &cmd.filename, bytes)
                            .map_err(|e| format!("blockfile:write_state: write_file: {e}"))?;
                    }
                    Err(e) => return Err(format!("blockfile:write_state: {e}")),
                }

                Ok(BlockfileWriteStateResult { bytes_written })
            }
        },
    );
}

/// A transcript stream's line count from its counter, with the stream and
/// generation it counts (Phase 5a-3). A file with no epoch yet gets one
/// first — `init_line_counter`, one read of the file, once per epoch; call on
/// the blocking pool. `None` when it can't be counted (no such file, or
/// mid-write by a build without transactions), and the caller falls back.
fn counted_line_count(
    fs: &crate::backend::storage::filestore::FileStore,
    zone: &str,
    stream: String,
) -> Option<BlockfileLineCountResult> {
    use crate::backend::agent_session::OUTPUT_FILE;
    let state = fs.line_state(zone, OUTPUT_FILE).ok()??;
    let state = if state.counted.is_some() { state } else { fs.init_line_counter(zone, OUTPUT_FILE).ok()?? };
    let counted = state.counted?;
    Some(BlockfileLineCountResult { count: counted.lines, stream: Some(stream), gen: Some(counted.gen) })
}

/// The valid generation of a transcript's `output`, read from the database on
/// the blocking pool (the global store can be held by another srv instance).
async fn db_generation(fs: &Arc<crate::backend::storage::filestore::FileStore>, zone: &str) -> Option<String> {
    let (fs, zone) = (fs.clone(), zone.to_string());
    tokio::task::spawn_blocking(move || {
        crate::backend::blockcontroller::shell::output_now(&fs, &zone).and_then(|(_, gen)| gen)
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
#[path = "blockfile_transcript_tests.rs"]
mod transcript_rpc_tests;
