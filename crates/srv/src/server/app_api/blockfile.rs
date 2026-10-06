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
                let started = std::time::Instant::now();
                let mut clock = ReadRangeClock::default();

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
                clock.source_ms = ms_since(started);

                // Generation (Phase 5a-3): read before and after the lines. A
                // replace always mints a new one, so the same value on both
                // sides proves every line came from that generation — only then
                // does the response name it. `expect_gen` turns any other
                // outcome into `gen_mismatch` instead of lines of another file.
                let transcript = cmd.filename == crate::backend::agent_session::OUTPUT_FILE;
                let lap = std::time::Instant::now();
                let gen_before = if transcript { db_generation(&filestore, &read_block).await } else { None };
                clock.gen_before_ms = ms_since(lap);
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
                    if let Some(turns) = cmd.tail_turns.filter(|t| *t > 0) {
                        trim_to_last_turns(&mut result, offset as u64, turns as usize);
                    }
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
                    let idx_clock = std::sync::Arc::new(std::sync::Mutex::new(IndexClock::default()));
                    let idx_result: Option<BlockfileReadRangeResult> = {
                        let filestore = filestore.clone();
                        let read_block = read_block.clone();
                        let idx_clock = idx_clock.clone();
                        let spawned = std::time::Instant::now();
                        let compute = move || -> Option<BlockfileReadRangeResult> {
                        idx_clock.lock().unwrap().queued_ms = ms_since(spawned);
                        let run = std::time::Instant::now();
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
                                let extend = std::time::Instant::now();
                                let extended = extend_output_idx(&filestore, &read_block);
                                idx_clock.lock().unwrap().extend_ms = Some(ms_since(extend));
                                extended?;
                                read_via_index(&filestore, &read_block, offset as u64, limit as u64)?
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
                    clock.index = Some(std::mem::take(&mut *idx_clock.lock().unwrap()));
                    if let Some(result) = idx_result {
                        let lap = std::time::Instant::now();
                        let gen_after = if transcript { db_generation(&filestore, &read_block).await } else { None };
                        clock.gen_after_ms = ms_since(lap);
                        let result = finish(result, gen_after);
                        clock.log_done(&cmd.block_id, offset, limit, "index", started, &result);
                        return Ok(result);
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

                let lap = std::time::Instant::now();
                let gen_after = if transcript { db_generation(&filestore, &read_block).await } else { None };
                clock.gen_after_ms = ms_since(lap);
                let result = finish(BlockfileReadRangeResult { lines, total, ..Default::default() }, gen_after);
                clock.log_done(&cmd.block_id, offset, limit, "whole_file", started, &result);
                Ok(result)
            }
        },
    );
}

fn ms_since(t: std::time::Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

/// A `blockfile:read_range` call this slow is logged as a warning, with where
/// its time went.
const SLOW_READ_RANGE_MS: u64 = 1_000;

/// Where a `blockfile:read_range` call spent its time. Two pane opens waited
/// about 17 s on this call while the log recorded only when it started
/// (docs/reports/REPORT_AGENT_OPEN_STALL_RCA_2026_10_05.md); with
/// `filestore: connection held a long time`, this names the step that waited.
#[derive(Default)]
struct ReadRangeClock {
    /// Choosing the store (the block's, or the agent's zone).
    source_ms: u64,
    /// `output`'s generation, read before the lines.
    gen_before_ms: u64,
    /// The `output.idx` read on the blocking pool, when it ran.
    index: Option<IndexClock>,
    /// `output`'s generation, read after the lines.
    gen_after_ms: u64,
}

#[derive(Default)]
struct IndexClock {
    /// Waiting for a blocking-pool thread.
    queued_ms: u64,
    /// Bringing `output.idx` up to date, when the first read found it stale.
    extend_ms: Option<u64>,
    /// The whole read on that thread, `extend_ms` included.
    run_ms: u64,
}

impl ReadRangeClock {
    fn log_done(
        &self,
        block_id: &str,
        offset: usize,
        limit: usize,
        path: &str,
        started: std::time::Instant,
        result: &BlockfileReadRangeResult,
    ) {
        let total_ms = ms_since(started);
        let bytes: usize = result.lines.iter().map(String::len).sum();
        let (queued_ms, extend_ms, run_ms) =
            self.index.as_ref().map_or((0, None, 0), |i| (i.queued_ms, i.extend_ms, i.run_ms));
        macro_rules! done {
            ($level:ident, $msg:literal) => {
                tracing::$level!(
                    block_id = %block_id,
                    offset,
                    limit,
                    path,
                    lines = result.lines.len(),
                    bytes,
                    total_ms,
                    source_ms = self.source_ms,
                    gen_before_ms = self.gen_before_ms,
                    queued_ms,
                    extend_ms = ?extend_ms,
                    run_ms,
                    gen_after_ms = self.gen_after_ms,
                    $msg
                )
            };
        }
        if total_ms >= SLOW_READ_RANGE_MS {
            done!(warn, "blockfile:read_range slow");
        } else {
            done!(debug, "blockfile:read_range done");
        }
    }
}

/// Whether `line` starts a turn in a Claude stream-json transcript: the same
/// rule as the frontend's `ClaudeTranslator.handleUserMessage` producing a
/// `user_message` — a `user` record whose content is a non-empty string, or
/// an array with an image and some text and no `tool_result` (a text-only
/// array is how the CLI records "[Request interrupted by user]" and other
/// meta lines, never rendered as a turn).
pub(crate) fn is_claude_turn_start(line: &str) -> bool {
    use serde_json::Value;
    if !line.contains("\"user\"") {
        return false;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return false;
    }
    match v.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(blocks)) => {
            let ty = |b: &Value| b.get("type").and_then(Value::as_str).map(str::to_owned);
            let has = |t: &str| blocks.iter().any(|b| ty(b).as_deref() == Some(t));
            !has("tool_result")
                && has("image")
                && blocks.iter().any(|b| {
                    ty(b).as_deref() == Some("text")
                        && b.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|t| !t.is_empty())
                })
        }
        _ => false,
    }
}

/// Index in `lines` where the last `turns` turns begin; `None` when there
/// are fewer. Scans from the end and stops at the `turns`-th start.
pub(crate) fn last_turns_start(lines: &[String], turns: usize) -> Option<usize> {
    if turns == 0 {
        return None;
    }
    let mut seen = 0;
    for (i, line) in lines.iter().enumerate().rev() {
        if is_claude_turn_start(line) {
            seen += 1;
            if seen == turns {
                return Some(i);
            }
        }
    }
    None
}

/// `tail_turns`: drop everything before the last `turns` turns (stamps
/// with it) and record where `lines` now starts.
fn trim_to_last_turns(result: &mut BlockfileReadRangeResult, first_line: u64, turns: usize) {
    if result.gen_mismatch.is_some() {
        return;
    }
    let cut = last_turns_start(&result.lines, turns).unwrap_or(0);
    if cut > 0 {
        result.lines.drain(..cut);
        if let Some(stamps) = result.stamps.as_mut() {
            stamps.drain(..cut.min(stamps.len()));
        }
    }
    result.offset = Some(first_line + cut as u64);
}

#[cfg(test)]
mod tail_turns_tests {
    use super::*;

    fn user(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"role": "user", "content": text}})
            .to_string()
    }
    fn tool_result() -> String {
        serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t", "content": "ok"}]}}).to_string()
    }
    fn delta(text: &str) -> String {
        serde_json::json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": text}}}).to_string()
    }

    #[test]
    fn turn_starts_match_what_the_pane_renders_as_a_user_message() {
        assert!(is_claude_turn_start(&user("deploy now")));
        assert!(!is_claude_turn_start(&user("")));
        assert!(!is_claude_turn_start(&tool_result()));
        assert!(!is_claude_turn_start(&delta("the user said")));
        let interrupted = serde_json::json!({"type": "user", "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}}).to_string();
        assert!(!is_claude_turn_start(&interrupted));
        let with_image = serde_json::json!({"type": "user", "message": {"content": [
            {"type": "image", "source": {"type": "base64", "data": "AAAA"}},
            {"type": "text", "text": "what is this?"}
        ]}})
        .to_string();
        assert!(is_claude_turn_start(&with_image));
        let image_only =
            serde_json::json!({"type": "user", "message": {"content": [{"type": "image"}]}})
                .to_string();
        assert!(!is_claude_turn_start(&image_only));
        assert!(!is_claude_turn_start("not json \"user\""));
    }

    fn transcript() -> Vec<String> {
        // 3 turns: [0..3) [3..6) [6..9)
        vec![
            user("one"),
            delta("a"),
            tool_result(),
            user("two"),
            delta("b"),
            delta("c"),
            user("three"),
            delta("d"),
            delta("e"),
        ]
    }

    #[test]
    fn finds_where_the_last_turns_begin() {
        let lines = transcript();
        assert_eq!(last_turns_start(&lines, 1), Some(6));
        assert_eq!(last_turns_start(&lines, 2), Some(3));
        assert_eq!(last_turns_start(&lines, 3), Some(0));
        assert_eq!(last_turns_start(&lines, 4), None);
        assert_eq!(last_turns_start(&lines, 0), None);
    }

    #[test]
    fn trims_lines_and_stamps_and_reports_the_new_start() {
        let mut r = BlockfileReadRangeResult {
            lines: transcript(),
            total: 109,
            stamps: Some((0..9).collect()),
            ..Default::default()
        };
        trim_to_last_turns(&mut r, 100, 2);
        assert_eq!(r.offset, Some(103));
        assert_eq!(r.lines, transcript()[3..].to_vec());
        assert_eq!(r.stamps, Some(vec![3, 4, 5, 6, 7, 8]));
        assert_eq!(r.total, 109, "total is the file's, untouched");
    }

    #[test]
    fn fewer_turns_than_asked_returns_the_whole_range() {
        let mut r = BlockfileReadRangeResult {
            lines: transcript(),
            total: 9,
            ..Default::default()
        };
        trim_to_last_turns(&mut r, 0, 7);
        assert_eq!(r.lines.len(), 9);
        assert_eq!(r.offset, Some(0));
    }

    #[test]
    fn a_generation_mismatch_is_left_alone() {
        let mut r = BlockfileReadRangeResult {
            gen_mismatch: Some(true),
            ..Default::default()
        };
        trim_to_last_turns(&mut r, 5, 2);
        assert_eq!(r.offset, None);
    }

    #[test]
    fn the_request_field_is_optional_on_the_wire() {
        let old: CommandBlockfileReadRangeData = serde_json::from_value(serde_json::json!({
            "block_id": "b", "filename": "output", "offset": 0, "limit": 10
        }))
        .unwrap();
        assert_eq!(old.tail_turns, None);
        let new: CommandBlockfileReadRangeData = serde_json::from_value(serde_json::json!({
            "block_id": "b", "filename": "output", "offset": 0, "limit": 10, "tail_turns": 7
        }))
        .unwrap();
        assert_eq!(new.tail_turns, Some(7));
        let wire = serde_json::to_value(BlockfileReadRangeResult::default()).unwrap();
        assert!(
            wire.get("offset").is_none(),
            "absent unless tail_turns was asked"
        );
    }
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
