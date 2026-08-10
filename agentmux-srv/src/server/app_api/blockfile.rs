use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_blockfile_line_count(engine, state);
    register_blockfile_read_range(engine, state);
    register_blockfile_read_state(engine, state);
    register_blockfile_write_state(engine, state);
}

fn register_blockfile_line_count(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();
    let global_store = state.global_transcript_store.clone();

    engine.register_handler(
        COMMAND_BLOCKFILE_LINE_COUNT,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            let global_store = global_store.clone();
            Box::pin(async move {
                let cmd: CommandBlockfileLineCountData = serde_json::from_value(data)
                    .map_err(|e| format!("blockfile:line_count: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, filename = %cmd.filename, "blockfile:line_count");

                // Cross-channel fallback (checked first): when this channel has
                // no local `output` for the block, the local `session:line_count`
                // meta is absent/stale, so a fresh cross-channel open would
                // report 0 lines and the pane would render empty. Count from the
                // agent's GLOBAL transcript zone instead. See
                // `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
                if let Some((gfs, zone)) =
                    global_output_source(&filestore, &global_store, &wstore, &cmd.block_id, &cmd.filename)
                {
                    if let Some(count) = global_zone_line_count(&gfs, &zone) {
                        return Ok(Some(
                            serde_json::to_value(&BlockfileLineCountResult { count }).unwrap(),
                        ));
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
                    if let Ok(Some(block)) = wstore.get::<Block>(&cmd.block_id) {
                        if let Some(count) = block.meta.get("session:line_count").and_then(|v| v.as_u64()) {
                            return Ok(Some(serde_json::to_value(
                                &BlockfileLineCountResult { count },
                            ).unwrap()));
                        }
                    }
                }

                // Fallback: count from WPS event ring buffer (capped at MAX_PERSIST = 4096).
                let scope = format!("block:{}", cmd.block_id);
                let events = broker.read_event_history(
                    crate::backend::wps::EVENT_BLOCK_FILE,
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

                Ok(Some(serde_json::to_value(&BlockfileLineCountResult { count }).unwrap()))
            })
        }),
    );
}

fn register_blockfile_read_range(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();
    let filestore = state.filestore.clone();
    let global_store = state.global_transcript_store.clone();
    let wstore = state.wstore.clone();

    engine.register_handler(
        COMMAND_BLOCKFILE_READ_RANGE,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            let filestore = filestore.clone();
            let global_store = global_store.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandBlockfileReadRangeData = serde_json::from_value(data)
                    .map_err(|e| format!("blockfile:read_range: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, filename = %cmd.filename, offset = cmd.offset, limit = cmd.limit, "blockfile:read_range");

                let limit = cmd.limit.min(10_000) as usize;
                let offset = cmd.offset as usize;
                let end = offset.saturating_add(limit);

                // Cross-channel fallback: when this channel has no local `output`
                // for the block, read the agent's GLOBAL transcript zone
                // (`agent:<defId>:current`) instead. `read_block` is the zone for
                // every FileStore call below — the local block_id normally, the
                // agent zone when the agent ran in another build/channel.
                let (filestore, read_block) =
                    global_output_source(&filestore, &global_store, &wstore, &cmd.block_id, &cmd.filename)
                        .unwrap_or_else(|| (filestore.clone(), cmd.block_id.clone()));

                // Fast path: output.idx — a lazily-built, self-validating byte-offset
                // index of every non-blank line in `output`. It lets us seek directly
                // to the requested line range instead of loading the whole file.
                //
                // The index is a pure cache of `output` with NO incremental mutation:
                // its 8-byte header records the `output` size it was built for. If that
                // equals `output`'s current size the index is fresh; otherwise we rebuild
                // it from a single streaming scan (rebuild_output_idx). Because the index
                // is always derived from the current `output` in one shot, it can never
                // desync, mis-handle chunk-split lines, or miscount blank lines — the
                // failure modes an incremental index would have.
                //
                // Gated to non-circular files: circular `output` (terminal ring buffers)
                // drops early bytes, so absolute byte offsets wouldn't map cleanly.
                use crate::backend::blockcontroller::shell::{rebuild_output_idx, OUTPUT_IDX_HEADER_LEN};
                if cmd.filename == "output" {
                    let idx_result: Option<BlockfileReadRangeResult> = (|| {
                        let out_stat = filestore.stat(&read_block, "output").ok()??;
                        if out_stat.opts.circular {
                            return None; // circular files: fall back to slow path
                        }
                        let output_size = out_stat.size as u64;

                        // Determine total_lines, rebuilding the index iff it is missing or
                        // its covered-size header doesn't match the current output size.
                        let idx_stat = filestore.stat(&read_block, "output.idx").ok().flatten();
                        let fresh = match &idx_stat {
                            Some(s) if s.size >= OUTPUT_IDX_HEADER_LEN => {
                                let (_, h) = filestore
                                    .read_at(&read_block, "output.idx", 0, OUTPUT_IDX_HEADER_LEN)
                                    .ok()?;
                                u64::from_le_bytes(h.try_into().ok()?) == output_size
                            }
                            _ => false,
                        };
                        let total_lines: u64 = if fresh {
                            let s = idx_stat.unwrap();
                            ((s.size - OUTPUT_IDX_HEADER_LEN) / 8) as u64
                        } else {
                            rebuild_output_idx(&filestore, &read_block, output_size)?
                        };

                        // Empty result cases — answered from the index, no output read.
                        if limit == 0 || total_lines == 0 || (offset as u64) >= total_lines {
                            return Some(BlockfileReadRangeResult { lines: vec![], total: total_lines, stamps: None });
                        }

                        // entry(k) = byte offset of non-blank line k (past the 8-byte header).
                        let entry = |k: u64| -> Option<i64> {
                            let (_, b) = filestore
                                .read_at(
                                    &read_block,
                                    "output.idx",
                                    OUTPUT_IDX_HEADER_LEN + (k * 8) as i64,
                                    8,
                                )
                                .ok()?;
                            Some(u64::from_le_bytes(b.try_into().ok()?) as i64)
                        };

                        let byte_start = entry(offset as u64)?;
                        let byte_end: i64 = if (offset + limit) as u64 >= total_lines {
                            output_size as i64
                        } else {
                            entry((offset + limit) as u64)?
                        };
                        let read_len = (byte_end - byte_start).max(0);
                        let (_, raw) = filestore
                            .read_at(&read_block, "output", byte_start, read_len)
                            .ok()?;
                        let text = String::from_utf8_lossy(&raw);
                        let lines: Vec<String> = text
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(|l| l.to_string())
                            .collect();

                        // Receive-time stamps for the returned lines, joined
                        // from the output.tsidx sidecar (batch byte offset →
                        // unix ms; see agent_session::TSIDX_FILE). Per line:
                        // the newest batch stamp at-or-before the line's own
                        // byte offset. Best-effort — any failure yields no
                        // stamps, never a failed read.
                        let stamps: Option<Vec<i64>> = (|| {
                            use crate::backend::agent_session::TSIDX_FILE;
                            let ts_stat = filestore.stat(&read_block, TSIDX_FILE).ok().flatten()?;
                            if ts_stat.size == 0 {
                                return None;
                            }
                            let raw_ts = filestore.read_file(&read_block, TSIDX_FILE).ok().flatten()?;
                            let mut entries: Vec<(u64, i64)> = String::from_utf8_lossy(&raw_ts)
                                .lines()
                                .filter_map(|l| {
                                    let v: serde_json::Value = serde_json::from_str(l.trim()).ok()?;
                                    Some((v.get("off")?.as_u64()?, v.get("ms")?.as_i64()?))
                                })
                                .collect();
                            if entries.is_empty() {
                                return None;
                            }
                            // Appends serialize on the store so offsets are
                            // already monotonic in practice; sort defensively
                            // (cross-channel mirrors racing into the global
                            // zone).
                            entries.sort_by_key(|(off, _)| *off);

                            // Byte offset of every returned line, read from
                            // output.idx in one slice.
                            let returned = lines.len() as u64;
                            let (_, idx_raw) = filestore
                                .read_at(
                                    &read_block,
                                    "output.idx",
                                    OUTPUT_IDX_HEADER_LEN + (offset as u64 * 8) as i64,
                                    (returned * 8) as i64,
                                )
                                .ok()?;
                            if idx_raw.len() != (returned * 8) as usize {
                                return None;
                            }
                            let stamps = idx_raw
                                .chunks_exact(8)
                                .map(|b| {
                                    let line_off = u64::from_le_bytes(b.try_into().unwrap());
                                    match entries.partition_point(|(off, _)| *off <= line_off) {
                                        0 => 0,
                                        p => entries[p - 1].1,
                                    }
                                })
                                .collect();
                            Some(stamps)
                        })();

                        Some(BlockfileReadRangeResult { lines, total: total_lines, stamps })
                    })();
                    if let Some(result) = idx_result {
                        tracing::debug!(
                            block_id = %cmd.block_id,
                            offset,
                            limit,
                            lines = result.lines.len(),
                            "blockfile:read_range via output.idx fast path"
                        );
                        return Ok(Some(serde_json::to_value(&result).unwrap()));
                    }
                }

                // Phase 1.3: Prefer FileStore (persistent, no size cap) over the
                // WPS broker ring buffer (MAX_PERSIST = 4096 events).
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
                    // Fallback: reconstruct from WPS event ring buffer.
                    // The ring buffer holds at most MAX_PERSIST = 4096 events;
                    // older events are evicted. Offset 0 = oldest retained line.
                    let scope = format!("block:{}", cmd.block_id);
                    let events = broker.read_event_history(
                        crate::backend::wps::EVENT_BLOCK_FILE,
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

                Ok(Some(serde_json::to_value(&BlockfileReadRangeResult {
                    lines,
                    total,
                    stamps: None,
                }).unwrap()))
            })
        }),
    );
}

fn register_blockfile_read_state(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_BLOCKFILE_READ_STATE,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandBlockfileReadStateData = serde_json::from_value(data)
                    .map_err(|e| format!("blockfile:read_state: {e}"))?;
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

                Ok(Some(serde_json::to_value(&BlockfileReadStateResult { content }).unwrap()))
            })
        }),
    );
}

fn register_blockfile_write_state(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_BLOCKFILE_WRITE_STATE,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandBlockfileWriteStateData = serde_json::from_value(data)
                    .map_err(|e| format!("blockfile:write_state: {e}"))?;
                if cmd.filename.contains('/') || cmd.filename.contains('\\') || cmd.filename.contains("..") {
                    return Err("blockfile:write_state: filename must not contain path separators".to_string());
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

                Ok(Some(serde_json::to_value(&BlockfileWriteStateResult { bytes_written }).unwrap()))
            })
        }),
    );
}
