// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PTY shell HTTP handlers.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `POST /api/v1/ptyshell/create` — attach to (creating if needed) "this
/// pane's one shell."
///
/// Reuses the same sub-block a human's composer-drawer shell already uses
/// or will use (`META_KEY_SHELL_SUBBLOCK_ID`), in either direction: if the
/// human already opened the drawer, the agent joins that exact PTY — same
/// scrollback, same live output. If nothing exists yet, this creates it
/// (the same `view: "term", controller: "shell"` headless-sub-block shape
/// `createsubblock`'s WS handler builds for `AgentShellSubblock.tsx`) and
/// persists the pointer, so a drawer opened *afterward* attaches to what
/// the agent already started instead of spawning a second, independent
/// shell. Either way, `resync_controller` runs before returning — unlike
/// the WS path (which waits for a frontend to mount a `term` view and call
/// `ControllerResyncCommand`), the PTY is live the moment this call
/// returns; a UI viewer for it is optional, not a prerequisite.
pub(super) async fn handle_pty_shell_create(
    State(state): State<AppState>,
    Json(req): Json<PtyShellCreateRequest>,
) -> axum::response::Response {
    match create_steps::create_pty_shell(&state, &req).await {
        Ok(resp) | Err(resp) => resp,
    }
}

/// Answer Windows ConPTY's own startup handshake, if seen, before the
/// caller proceeds. Discovered live (not assumed): the moment a Windows
/// ConPTY is created, it queries the cursor position (`ESC[6n`, Device
/// Status Report) and blocks its own internal state — and therefore every
/// later write — until it sees a `ESC[<row>;<col>R` reply. A human-facing
/// `term` pane gets this for free because xterm.js, running in the
/// browser, answers real cursor queries as part of being an actual
/// terminal emulator; this headless facility has nothing playing that
/// role, so without this the PTY would sit permanently stuck before
/// printing even its first prompt — confirmed by a live round trip that
/// hung indefinitely until this was added. `ESC[1;1R` (cursor at row 1,
/// col 1) is a plausible-enough fixed answer: nothing here has rendered
/// anything yet, so "top-left" is as true as any other coordinate, and
/// ConPTY only wants to reconcile its internal buffer, not validate the
/// value against anything a person would see.
///
/// `baseline_len` MUST be the "term" file's byte length taken immediately
/// before `resync_controller` ran (0 for a genuinely new block) — see the
/// call sites. Only bytes appended AFTER that offset are ever inspected.
/// **Not** safe to scan the whole file regardless of reuse/fresh (an
/// earlier version of this function did exactly that): the "term" file is
/// append-only for the block's entire lifetime
/// (`handle_append_block_file`'s `FILE_OP_APPEND`), so a long-running
/// reused shell's history almost always still contains its own original,
/// long-since-answered `ESC[6n` from whenever it first started — matching
/// against that stale byte sequence would inject an unsolicited
/// `ESC[1;1R` into whatever a human is doing RIGHT NOW on that shell
/// (Codex P1 on PR #3194, confirmed live before this fix: `create` on an
/// already-running reused shell always found the old query and misfired).
/// The baseline makes every case correct uniformly: nothing new appears
/// (the common reuse case) → correctly finds nothing; `resync_controller`
/// had to respawn a dead process → correctly sees only ITS fresh query,
/// never anything from the block's former life.
#[cfg_attr(not(windows), allow(unused_variables))]
pub(super) async fn answer_conpty_handshake_if_seen(state: &AppState, block_id: &str, baseline_len: usize) {
    #[cfg(windows)]
    {
        let mut answered = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let saw_dsr = state
                .filestore
                .read_file(block_id, "term")
                .ok()
                .flatten()
                .map(|bytes| {
                    let new_bytes = bytes.get(baseline_len..).unwrap_or(&[]);
                    new_bytes.windows(4).any(|w| w == b"\x1b[6n")
                })
                .unwrap_or(false);
            if saw_dsr {
                let _ = blockcontroller::send_input(
                    block_id,
                    blockcontroller::BlockInputUnion::data(b"\x1b[1;1R".to_vec()),
                    None,
                );
                answered = true;
                break;
            }
        }
        tracing::info!(block_id = %block_id, answered_dsr = answered, "ptyshell: ConPTY handshake check");
    }
}

/// `POST /api/v1/ptyshell/input` — write raw text to the PTY, as if typed.
/// Unlike `/api/v1/shell/input`, no newline is appended: a real terminal
/// doesn't add one either, and PTY programs (readline, Sharprompt) expect
/// the caller to send exactly what a keyboard would (e.g. `"y\n"` to answer
/// a prompt and press Enter, or `"\x03"` for Ctrl+C — the OS PTY layer
/// translates that raw byte into a real interrupt delivered to the
/// foreground process, same as a human pressing Ctrl+C in a real terminal,
/// without ending the shell).
///
/// There is deliberately no separate "send a signal" endpoint: an earlier
/// version of this API had one (`PtyShellSignal`, `BlockInputUnion::signal`),
/// removed after Codex correctly flagged (PR #3177) that
/// `ShellController`'s input loop treats ANY signal as "close the PTY" —
/// `if input.sig_name.is_some() { break; }` in
/// `blockcontroller/shell/lifecycle.rs` — so it terminated the whole shell
/// instead of interrupting just the foreground command, the opposite of
/// what it advertised. Real signal delivery through a PTY is the raw
/// control byte, handled here, not a separate primitive.
pub(super) async fn handle_pty_shell_input(
    State(state): State<AppState>,
    Json(req): Json<PtyShellInputRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (
            StatusCode::OK,
            Json(PtyShellInputResponse {
                written: false,
                error: Some("not a shell belonging to this agent's own pane".to_string()),
            }),
        );
    }
    // Lock BEFORE writing, not after (ReAgent P1 on PR #3194): the
    // previous ordering left `term:agentlockuntil` unset for the entire
    // duration of `send_input` — the exact window `controllerinput`'s own
    // lock check (added for Codex's earlier P1 on this same PR) needs the
    // lock to already exist to catch anything. Locking first doesn't make
    // the race provably impossible (a human keystroke processed in the
    // sliver of time before this write actually commits could still slip
    // through — true elimination would need a real mutex shared between
    // this HTTP path and the WS `controllerinput` path, disproportionate
    // machinery for a UX-level collision guard, not a hard security
    // boundary), but it shrinks the window from "all of send_input's own
    // duration plus the lock write's" down to just the lock write's own
    // commit latency — the actual, meaningful fix here.
    lock_shell_for_agent(&state, &req.shell_id);
    let result = blockcontroller::send_input(
        &req.shell_id,
        blockcontroller::BlockInputUnion::data(req.text.into_bytes()),
        None,
    );
    match result {
        Ok(()) => (StatusCode::OK, Json(PtyShellInputResponse { written: true, error: None })),
        Err(e) => (StatusCode::OK, Json(PtyShellInputResponse { written: false, error: Some(e) })),
    }
}

/// Lock `shell_id` out from human keyboard input for `AGENT_LOCK_WINDOW_MS`
/// from now — called BEFORE every agent write (input/resize), not after
/// (see the call sites' own comment for why the ordering matters).
/// Best-effort: a failure to write the lock meta just means the human's
/// input isn't blocked this time, not that the agent's write itself failed.
pub(super) fn lock_shell_for_agent(state: &AppState, shell_id: &str) {
    let until_ms = agentmux_common::time::now_ms() + AGENT_LOCK_WINDOW_MS;

    // In-memory first, and unconditionally: this is what `controllerinput`
    // actually enforces against (see `blockcontroller::agent_lock`'s module
    // doc for why the enforcement path can't afford a `Store` read). It
    // cannot fail and costs a hash insert, so the lock is live before the
    // persisted/broadcast half is even attempted — which also shrinks the
    // pre-existing "human keystroke in the sliver before the write commits"
    // window described above to the width of a mutex acquire.
    blockcontroller::agent_lock::lock_until(shell_id, until_ms);

    // Then the meta write, which drives the frontend's own badge + gate
    // (`AgentShellSubblock.tsx`). Best-effort, exactly as before: a failure
    // here costs the human the "Agent is using this shell" indicator, not
    // the enforcement itself.
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(META_KEY_AGENT_LOCK_UNTIL.to_string(), json!(until_ms));
    if let Err(e) = broadcast_meta_update(state, shell_id, &meta) {
        tracing::warn!(shell_id = %shell_id, error = %e, "ptyshell: failed to write agent lock");
    }
}

/// `POST /api/v1/ptyshell/resize` — resize the PTY. Some TUIs render
/// differently or wrap badly at the fallback geometry (25x200).
pub(super) async fn handle_pty_shell_resize(
    State(state): State<AppState>,
    Json(req): Json<PtyShellResizeRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (
            StatusCode::OK,
            Json(PtyShellResizeResponse {
                resized: false,
                error: Some("not a shell belonging to this agent's own pane".to_string()),
            }),
        );
    }
    // Lock BEFORE writing — see the identical comment in
    // `handle_pty_shell_input` for why the ordering matters.
    lock_shell_for_agent(&state, &req.shell_id);
    let result = blockcontroller::send_input(
        &req.shell_id,
        blockcontroller::BlockInputUnion::resize(crate::backend::obj::TermSize {
            rows: req.rows as i64,
            cols: req.cols as i64,
        }),
        None,
    );
    match result {
        Ok(()) => (StatusCode::OK, Json(PtyShellResizeResponse { resized: true, error: None })),
        Err(e) => (StatusCode::OK, Json(PtyShellResizeResponse { resized: false, error: Some(e) })),
    }
}

/// `POST /api/v1/ptyshell/read` — read back the shell's raw output tail.
///
/// Reads the block's `term` file straight from `FileStore` — the same file
/// `blockcontroller::shell::lifecycle`'s real PTY read loop write-throughs
/// to (`handle_append_block_file(..., "term", ..., filestore, ...)`,
/// `SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md` §2.1). Deliberately
/// NOT the optimized `blockfile:read_range` path (byte-offset index,
/// cross-channel global-transcript fallback, and a different filename —
/// "output" — used for agent CLI transcripts, a different content stream
/// entirely), which solves problems this endpoint doesn't have: a PTY shell
/// created through this API is always local and short-lived, so a plain
/// whole-file read + in-memory tail is simpler and correct. This is a raw
/// line log, not a rendered-screen snapshot — see the type's own doc
/// comment in `api_types.rs`.
pub(super) async fn handle_pty_shell_read(
    State(state): State<AppState>,
    Json(req): Json<PtyShellReadRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (
            StatusCode::OK,
            Json(PtyShellReadResponse { content: String::new(), truncated: false, end: 0 }),
        )
            .into_response();
    }
    let mut end = 0u64;
    // Where the bytes `read_file` returns start in the whole log: 0, unless the
    // file is a circular one that has wrapped.
    let start_idx = match state.filestore.stat(&req.shell_id, "term") {
        Ok(Some(f)) => f.data_start_idx().max(0) as u64,
        _ => 0,
    };
    let content = match state.filestore.read_file(&req.shell_id, "term") {
        Ok(Some(bytes)) => {
            let (range, abs_end) = since_slice(&bytes, start_idx, req.since);
            end = abs_end;
            String::from_utf8_lossy(&bytes[range]).to_string()
        }
        Ok(None) => String::new(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("ptyshell.read: {e}") })),
            )
                .into_response();
        }
    };
    let tail_lines = req.tail_lines.unwrap_or(200).max(1) as usize;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(tail_lines);
    let truncated = start > 0;
    let tail = lines[start..].join("\n");
    (StatusCode::OK, Json(PtyShellReadResponse { content: tail, truncated, end })).into_response()
}

/// Where to start reading `len` bytes that begin at `start_idx` in the whole
/// log, for a reader that has seen everything before `since`; and the log's
/// absolute end, the next `since`. Offsets are absolute, so they keep growing
/// even when a circular file drops its oldest bytes. A `since` before what is
/// still held reads all of it; one past the end (a cleared log) reads it all
/// again. The range holds whole characters only: it starts past any
/// continuation bytes (a wrapped log can begin mid-character) and stops
/// before a character still being written, which the next read returns.
fn since_slice(bytes: &[u8], start_idx: u64, since: Option<u64>) -> (std::ops::Range<usize>, u64) {
    let len = bytes.len();
    let to = len - incomplete_tail(bytes);
    let end = start_idx + to as u64;
    let mut from = match since {
        Some(s) if s >= start_idx && s <= end => (s - start_idx) as usize,
        _ => 0,
    };
    while from < to && bytes[from] & 0xC0 == 0x80 {
        from += 1;
    }
    (from..to, end)
}

/// How many bytes at the end of `bytes` begin a UTF-8 character that isn't
/// complete yet.
fn incomplete_tail(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(4) {
        let b = bytes[len - back];
        if b & 0xC0 == 0x80 {
            continue;
        }
        let need = match b {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        return if need > back { back } else { 0 };
    }
    0
}

/// `POST /api/v1/ptyshell/status` — query whether the shell is still running.
pub(super) async fn handle_pty_shell_status(
    State(state): State<AppState>,
    Json(req): Json<PtyShellStatusRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (StatusCode::OK, Json(PtyShellStatusResponse { running: false, exit_code: None }));
    }
    match blockcontroller::get_block_controller_status(&req.shell_id) {
        Some(status) => {
            let running = status.shellprocstatus == blockcontroller::STATUS_RUNNING;
            let exit_code = (status.shellprocstatus == blockcontroller::STATUS_DONE)
                .then_some(status.shellprocexitcode);
            (StatusCode::OK, Json(PtyShellStatusResponse { running, exit_code }))
        }
        // No controller registered — unknown id or already torn down via
        // ptyshell/stop. Same "plain not-running answer, not an error"
        // contract ShellStatus already documents.
        None => (StatusCode::OK, Json(PtyShellStatusResponse { running: false, exit_code: None })),
    }
}

/// `POST /api/v1/ptyshell/stop` — release the agent's lock immediately,
/// without waiting for `AGENT_LOCK_WINDOW_MS` to lapse on its own.
///
/// Does not touch the process or the block at all (see
/// `PtyShellStopResponse`'s doc comment in `api_types.rs` for why: this
/// shell is a shared, pane-scoped resource now, not a private one this API
/// owns outright).
pub(super) async fn handle_pty_shell_stop(
    State(state): State<AppState>,
    Json(req): Json<PtyShellStopRequest>,
) -> impl IntoResponse {
    if !is_owned_by_agent(&state.mstore, &req.shell_id, &req.agent_block_id) {
        return (StatusCode::OK, Json(PtyShellStopResponse { released: false }));
    }
    // Released in memory first — that's the copy `controllerinput` enforces
    // against, so the human's keyboard is live again the instant this
    // returns, not once the meta write commits and the frontend hears about
    // it. The return value is the authoritative "was a lease actually held",
    // for the same reason.
    let was_locked = blockcontroller::agent_lock::release(&req.shell_id);

    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(META_KEY_AGENT_LOCK_UNTIL.to_string(), serde_json::Value::Null);
    let released = broadcast_meta_update(&state, &req.shell_id, &meta).is_ok() && was_locked;

    tracing::info!(block_id = %req.shell_id, released, "ptyshell.stop: released lock");
    (StatusCode::OK, Json(PtyShellStopResponse { released }))
}

/// The 409 for a pane whose agent shell is running (or starting, created by
/// this run) on another connection than `connection`, checked without
/// touching the shell; the same cases `try_attach_to_existing_shell` refuses.
fn running_shell_conflict(state: &AppState, agent_block_id: &str, connection: &str) -> Option<axum::response::Response> {
    let shell_id = state
        .mstore
        .get::<crate::backend::obj::Block>(agent_block_id)
        .ok()
        .flatten()
        .and_then(|b| b.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()).map(str::to_string))?;
    let shell = state.mstore.get::<crate::backend::obj::Block>(&shell_id).ok().flatten()?;
    // Running, or created by this srv run and still starting (a concurrent
    // create won the claim): either way `try_attach_to_existing_shell` would
    // refuse it, so it is refused here too, before anything is asked.
    let live = match blockcontroller::get_block_controller_status(&shell_id) {
        Some(s) => s.shellprocstatus == blockcontroller::STATUS_RUNNING,
        None => {
            crate::backend::obj::meta_get_string(&shell.meta, super::http_shell::META_KEY_PTYSHELL_BOOT_ID, "")
                == *state.boot_id
        }
    };
    if !live {
        return None;
    }
    super::http_shell::connection_conflict(&shell, connection)
}

#[cfg(test)]
mod since_tests {
    use super::since_slice;

    fn ascii(n: usize) -> Vec<u8> {
        vec![b'a'; n]
    }

    #[test]
    fn a_cursor_reads_only_what_came_after_it() {
        // A plain log: offsets are byte positions.
        assert_eq!(since_slice(&ascii(100), 0, None), (0..100, 100));
        assert_eq!(since_slice(&ascii(100), 0, Some(40)), (40..100, 100));
        assert_eq!(since_slice(&ascii(100), 0, Some(100)), (100..100, 100));
        // A cleared log (the cursor is past the end) is read again from its start.
        assert_eq!(since_slice(&ascii(10), 0, Some(100)), (0..10, 10));
    }

    #[test]
    fn offsets_stay_absolute_when_a_circular_log_wraps() {
        // 300 bytes written, the newest 256 held: they start at 44.
        assert_eq!(since_slice(&ascii(256), 44, Some(250)), (206..256, 300));
        assert_eq!(since_slice(&ascii(256), 44, Some(300)), (256..256, 300));
        // A cursor older than what is still held reads all of it.
        assert_eq!(since_slice(&ascii(256), 44, Some(10)), (0..256, 300));
    }

    #[test]
    fn a_read_never_splits_a_character() {
        let done = "ok é ✓".as_bytes().to_vec();
        assert_eq!(since_slice(&done, 0, None), (0..done.len(), done.len() as u64));
        // The last character is half written: it waits for the next read.
        let mut half = "ok ".as_bytes().to_vec();
        half.extend_from_slice(&"✓".as_bytes()[..2]);
        assert_eq!(since_slice(&half, 0, None), (0..3, 3));
        // A wrapped log that starts mid-character skips the leftover bytes.
        let wrapped = &"✓ok".as_bytes()[1..];
        assert_eq!(since_slice(wrapped, 1, None), (2..4, 5));
    }
}

#[path = "http_pty_shell_create.rs"]
mod create_steps;

#[cfg(test)]
#[path = "http_pty_shell_tests.rs"]
mod route_tests;
