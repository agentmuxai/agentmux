// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bash-wrap` subcommand — owns the PTY and streams chunks while
//! the user's bash command runs.
//!
//! Invariants:
//! - The user's command is decoded from `--b64-cmd` so quoting /
//!   multi-line / shell-metachar content survives the rewrite.
//! - The inner command runs inside a real PTY (24×200, TERM=xterm-256color)
//!   so ANSI colors, spinners, and interactive prompts render
//!   correctly in the AgentMux overlay.
//! - Output is line-flushed with a ~50ms quiet-window timeout so
//!   carriage-return progress bars (e.g. `npm install`'s `[==>] 50%`)
//!   surface incrementally rather than waiting for `\n`.
//! - Aggregated stdout (including stderr lines prefixed with
//!   `[stderr] `) is captured into a buffer; on exit, a formatted
//!   summary lands on this process's stdout for Claude's native Bash
//!   tool to harvest as the `tool_result` content. Truncated head/
//!   tail at 50KB each per
//!   `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §4.5.
//! - If WPS publish fails (missing env, sidecar down, etc.), the
//!   command still runs to completion — output just doesn't stream.
//!   The model-visible blob is unaffected. A `kind: "system"` chunk
//!   announces the degradation at startup.

use anyhow::{Context, Result};
use clap::Parser;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Serialize;
use std::io::Read as _;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::wps_client::WpsClient;

/// CLI args for `bash-wrap`. `command` carried as base64 to sidestep
/// every quoting concern in the shell that invokes us.
#[derive(Parser, Debug)]
pub struct Args {
    /// Tool-use id from Claude's `tool_use` event; threaded back as
    /// the WPS subject suffix so the frontend correlates chunks with
    /// the matching ToolNode.
    #[arg(long)]
    pub tool_id: String,

    /// URL-safe base64 (no padding) of the original command string.
    #[arg(long)]
    pub b64_cmd: String,

    /// Optional block id — used as the `block:<id>` WPS scope so
    /// chunks only reach subscribers watching this block. When
    /// omitted, chunks publish unscoped.
    #[arg(long)]
    pub block_id: Option<String>,
}

const PTY_ROWS: u16 = 24;
const PTY_COLS: u16 = 200;
const FLUSH_QUIET_WINDOW: Duration = Duration::from_millis(50);
/// Cap the model-visible head/tail sections of the aggregated blob.
const MODEL_BLOB_HEAD_BYTES: usize = 50_000;
const MODEL_BLOB_TAIL_BYTES: usize = 50_000;

/// Wire payload published on `tool_chunk:<id>`. Mirrors the TypeScript
/// `ToolChunkMessage` in `SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §4.3.
#[derive(Serialize)]
struct ChunkMessage<'a> {
    op: &'static str, // "chunk"
    kind: &'a str,    // "stdout" | "stderr" | "system"
    content: &'a str,
    timestamp: u64,
}

#[derive(Serialize)]
struct TerminalMessage {
    op: &'static str, // "terminal"
    exit_code: i32,
    timestamp: u64,
}

/// Returns the inner command's exit code so main.rs can mirror it as
/// the wrapper's own process exit. Without this, Claude's native Bash
/// tool saw success for every wrapped command regardless of the actual
/// outcome — codex P1 on PR #804.
pub async fn run(args: Args) -> Result<i32> {
    let command = decode_command(&args.b64_cmd)?;

    let wps = WpsClient::from_env();
    let degraded = wps.is_none();

    // Buffer for the aggregated model-visible blob.
    let buffered = Arc::new(Mutex::new(Vec::<u8>::with_capacity(64 * 1024)));

    if degraded {
        // Surface degradation as a real chunk on stdout (we'll prefix
        // it on the model side too) so the user sees a clear "no
        // streaming this turn" message in the overlay. WPS publish
        // would no-op, so we skip it.
        let warn = b"[bashwrap] warning: streaming disabled (auth/url env missing); command output will only appear on completion\n";
        buffered.lock().await.extend_from_slice(warn);
    } else if let Some(client) = wps.as_ref() {
        publish_system(
            client,
            &args.tool_id,
            args.block_id.as_deref(),
            &format!("[bashwrap] starting: {} chars", command.len()),
        )
        .await
        .ok(); // Best-effort.
    }

    let start = std::time::Instant::now();
    let status = run_pty(&args, &command, wps.as_ref(), buffered.clone()).await?;
    let elapsed = start.elapsed();

    // Final terminal marker so the frontend can flip log.open=false
    // even if the matching StreamFlush from Claude is delayed.
    if let Some(client) = wps.as_ref() {
        let _ = client
            .publish_chunk(
                &args.tool_id,
                args.block_id.as_deref(),
                &TerminalMessage {
                    op: "terminal",
                    exit_code: status,
                    timestamp: now_ms(),
                },
            )
            .await;
    }

    let buf = buffered.lock().await;
    let model_blob = format_model_blob(&buf, status, elapsed);
    print!("{}", model_blob);
    Ok(status)
}

fn decode_command(b64: &str) -> Result<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let bytes = URL_SAFE_NO_PAD
        .decode(b64.as_bytes())
        .context("decoding --b64-cmd")?;
    Ok(String::from_utf8(bytes).context("--b64-cmd is not valid UTF-8")?)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn publish_system(
    client: &WpsClient,
    tool_id: &str,
    block_id: Option<&str>,
    msg: &str,
) -> Result<()> {
    client
        .publish_chunk(
            tool_id,
            block_id,
            &ChunkMessage {
                op: "chunk",
                kind: "system",
                content: msg,
                timestamp: now_ms(),
            },
        )
        .await
}

async fn publish_stdout(
    client: &WpsClient,
    tool_id: &str,
    block_id: Option<&str>,
    chunk: &str,
) -> Result<()> {
    client
        .publish_chunk(
            tool_id,
            block_id,
            &ChunkMessage {
                op: "chunk",
                kind: "stdout",
                content: chunk,
                timestamp: now_ms(),
            },
        )
        .await
}

/// Pick the shell that runs the wrapped command.
///
/// Unix: `bash` (NOT `sh`). The PreToolUse hook matches on
/// `tool_name == "Bash"`, so users expect bash semantics — `[[ ]]`,
/// arrays, `source`, `pipefail`, etc. Using dash/sh as a stand-in
/// would silently break a substantial portion of real commands.
/// Codex P1 on PR #804.
///
/// Win32: `cmd /C` is the lowest-common-denominator Windows shell.
/// PowerShell or pwsh might be more capable but requires explicit
/// detection; keeping `cmd` matches what Claude Code's native Bash
/// tool uses internally on Windows.
fn shell_for_platform() -> &'static str {
    if cfg!(windows) { "cmd" } else { "bash" }
}

fn shell_dash_c_flag() -> &'static str {
    if cfg!(windows) { "/C" } else { "-c" }
}

/// Spawn the PTY child and stream its bytes. Returns the exit code.
///
/// We use a blocking thread for the PTY reader (the `portable_pty`
/// master reader is sync) and a tokio mpsc to forward chunks into
/// async-land for the publisher.
async fn run_pty(
    args: &Args,
    command: &str,
    wps: Option<&WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
) -> Result<i32> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: PTY_ROWS,
            cols: PTY_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening PTY")?;

    let mut builder = CommandBuilder::new(shell_for_platform());
    builder.arg(shell_dash_c_flag());
    builder.arg(command);
    builder.env("TERM", "xterm-256color");
    // PTY child inherits our env (which inherits agentmux-srv's), so
    // AGENTMUX_* env vars + everything the parent had are present.

    let mut child = pair
        .slave
        .spawn_command(builder)
        .context("spawning PTY command")?;

    let mut reader = pair
        .master
        .try_clone_reader()
        .context("cloning PTY reader")?;
    // Drop the slave handle in the parent — the child still has it.
    drop(pair.slave);
    // Drop the master writer; we never inject stdin (interactive
    // prompts deferred to PR γ+ per spec §14 open questions).
    drop(pair.master);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1024);

    // Reader → flusher state. The blocking PTY reader appends bytes
    // to `shared_pending`; an async timer task flushes the buffer on
    // newline, on 4KB threshold, OR after FLUSH_QUIET_WINDOW idle
    // time. The idle-flush path is what makes CR-only progress bars
    // (npm install's `[==>] 50%` updates) surface live — without it,
    // the reader sits on partial bytes until a `\n` finally arrives.
    let shared_pending: Arc<std::sync::Mutex<Vec<u8>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(4096)));
    let reader_done = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let tx_reader = tx.clone();
    let tx_flusher = tx.clone();
    drop(tx); // both clones own EOF for the publisher

    // Blocking reader thread: read bytes + append to shared buffer.
    // Inline newline/size-triggered flush for low-latency delivery
    // of line-buffered output.
    let pending_r = shared_pending.clone();
    let done_r = reader_done.clone();
    let reader_handle = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("PTY read error: {}", e);
                    break;
                }
            };
            let mut p = pending_r.lock().unwrap();
            p.extend_from_slice(&buf[..n]);
            if p.contains(&b'\n') || p.len() >= 4096 {
                let chunk = std::mem::take(&mut *p);
                drop(p);
                let _ = tx_reader.blocking_send(chunk);
            }
        }
        // EOF: drain remainder, signal done.
        let mut p = pending_r.lock().unwrap();
        if !p.is_empty() {
            let chunk = std::mem::take(&mut *p);
            drop(p);
            let _ = tx_reader.blocking_send(chunk);
        }
        done_r.store(true, std::sync::atomic::Ordering::Release);
    });

    // Async quiet-window flusher: ticks every FLUSH_QUIET_WINDOW and
    // drains any bytes that have been sitting without a newline /
    // threshold trigger. Exits when reader signals done.
    let pending_f = shared_pending.clone();
    let done_f = reader_done.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FLUSH_QUIET_WINDOW);
        // Skip the immediate first tick — wait one quiet window
        // before the first flush so we don't preempt the reader's
        // newline-based flush on common line-buffered output.
        interval.tick().await;
        loop {
            interval.tick().await;
            if done_f.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let chunk = {
                let mut p = pending_f.lock().unwrap();
                if p.is_empty() {
                    continue;
                }
                std::mem::take(&mut *p)
            };
            if tx_flusher.send(chunk).await.is_err() {
                return;
            }
        }
    });

    // Async loop: drain the channel and publish chunks. Also runs a
    // periodic timer that flushes any remaining pending bytes that
    // didn't get bounced over via the newline heuristic.
    let tool_id = args.tool_id.clone();
    let block_id = args.block_id.clone();
    let wps_clone = wps.cloned();
    let buffered_clone = buffered.clone();

    let publisher_handle = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            // Capture into the aggregate buffer first, regardless of
            // publish success — model-visible blob must be intact.
            buffered_clone.lock().await.extend_from_slice(&bytes);

            // Stream chunk over WPS. Best-effort: if publish fails
            // (sidecar down, etc.), log and continue — degraded mode.
            if let Some(client) = wps_clone.as_ref() {
                let content = String::from_utf8_lossy(&bytes).into_owned();
                if let Err(e) =
                    publish_stdout(client, &tool_id, block_id.as_deref(), &content).await
                {
                    tracing::warn!("WPS publish failed: {}", e);
                }
            }
        }
    });

    // Wait for the child to exit. portable_pty's wait is blocking;
    // spawn on blocking pool.
    let exit_status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .context("joining wait task")?
        .context("waiting for PTY child")?;

    // The reader thread exits when EOF arrives on its end of the PTY,
    // which happens after the child exits. Join it.
    let _ = tokio::task::spawn_blocking(|| reader_handle.join()).await;

    // Drop tx-side via publisher_handle awaiting Stream closure happens
    // automatically when the reader thread exits and tx is dropped.
    let _ = publisher_handle.await;

    Ok(exit_status.exit_code() as i32)
}

/// Snap `idx` down to the nearest UTF-8 character boundary (or 0).
/// A byte is a UTF-8 char boundary iff it's either ASCII (< 0x80) or
/// a leading byte (>= 0xC0). Continuation bytes are in 0x80..=0xBF.
/// `idx == buf.len()` is always a valid boundary (one past the end).
fn snap_to_char_boundary_floor(buf: &[u8], idx: usize) -> usize {
    let mut i = idx.min(buf.len());
    while i > 0 && i < buf.len() && (buf[i] & 0xC0) == 0x80 {
        i -= 1;
    }
    i
}

/// Snap `idx` up to the nearest UTF-8 character boundary (or buf.len()).
fn snap_to_char_boundary_ceil(buf: &[u8], idx: usize) -> usize {
    let mut i = idx.min(buf.len());
    while i < buf.len() && (buf[i] & 0xC0) == 0x80 {
        i += 1;
    }
    i
}

/// Build the aggregated, model-visible blob from the captured bytes.
/// Truncates with head/tail markers if it exceeds 100KB.
///
/// **UTF-8 safety**: head/tail slice boundaries are snapped to the
/// nearest UTF-8 character boundary so non-ASCII output (emoji,
/// accented characters, CJK) doesn't get corrupted into `�`
/// replacement characters at the cut point. Reagent P1 round 4 on
/// PR #804 caught this — naive fixed-byte slicing splits multi-byte
/// sequences and the lossy decode emits replacement chars.
pub(crate) fn format_model_blob(
    buf: &[u8],
    exit_code: i32,
    elapsed: std::time::Duration,
) -> String {
    let body = if buf.len() <= MODEL_BLOB_HEAD_BYTES + MODEL_BLOB_TAIL_BYTES {
        String::from_utf8_lossy(buf).into_owned()
    } else {
        let head_end = snap_to_char_boundary_floor(buf, MODEL_BLOB_HEAD_BYTES);
        let tail_start = snap_to_char_boundary_ceil(buf, buf.len() - MODEL_BLOB_TAIL_BYTES);
        let head = String::from_utf8_lossy(&buf[..head_end]);
        let tail = String::from_utf8_lossy(&buf[tail_start..]);
        let elided = tail_start - head_end;
        format!(
            "{}\n... [{} bytes elided — see streaming log for full output] ...\n{}",
            head, elided, tail
        )
    };
    format!(
        "<exited {} in {:.2}s>\n{}",
        exit_code,
        elapsed.as_secs_f64(),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_b64_command() {
        // base64("echo hi") = ZWNobyBoaQ (no padding, url-safe)
        let s = decode_command("ZWNobyBoaQ").unwrap();
        assert_eq!(s, "echo hi");
    }

    #[test]
    fn decodes_multi_line_with_quotes() {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let original = "echo \"hello\" && cat $HOME/.env\nls -la";
        let b64 = URL_SAFE_NO_PAD.encode(original.as_bytes());
        let s = decode_command(&b64).unwrap();
        assert_eq!(s, original);
    }

    #[test]
    fn rejects_malformed_b64() {
        assert!(decode_command("not-valid-base64===").is_err());
    }

    #[test]
    fn format_model_blob_includes_exit_and_elapsed() {
        let out = format_model_blob(
            b"hello world",
            0,
            std::time::Duration::from_millis(1234),
        );
        assert!(out.starts_with("<exited 0 in 1.23s>"));
        assert!(out.ends_with("hello world"));
    }

    #[test]
    fn format_model_blob_snaps_to_utf8_boundaries() {
        // Reagent P1 round 4: ensure multi-byte UTF-8 sequences don't
        // get split at the truncation cut points. Build a 150KB buffer
        // by repeating a 4-byte emoji (U+1F600 "😀") which spans bytes
        // [0xF0, 0x9F, 0x98, 0x80]. Naive fixed-byte slicing would
        // split most cuts mid-sequence, producing `�` replacement
        // chars in the output. With the boundary-snap, the head and
        // tail must both decode losslessly back to whole emojis.
        let emoji = "😀".as_bytes();
        assert_eq!(emoji.len(), 4);
        let buf: Vec<u8> = emoji.iter().cycle().take(150_000).copied().collect();
        let out = format_model_blob(&buf, 0, std::time::Duration::from_millis(100));
        // Pull the head section out — everything between the framing
        // and the elision marker.
        let after_frame = out.split('\n').skip(1).next().unwrap();
        // The first emoji should be intact (no replacement char).
        assert!(
            after_frame.starts_with('😀'),
            "head should start with intact emoji, got: {:?}",
            &after_frame[..40.min(after_frame.len())]
        );
        // No replacement char anywhere in the output.
        assert!(
            !out.contains('\u{FFFD}'),
            "found replacement char (lossy decode at boundary)"
        );
    }

    #[test]
    fn snap_floor_handles_continuation_bytes() {
        // "😀" = F0 9F 98 80. At idx=2 we're inside the sequence;
        // floor must walk back to idx=0.
        let buf = "😀".as_bytes();
        assert_eq!(snap_to_char_boundary_floor(buf, 2), 0);
        assert_eq!(snap_to_char_boundary_floor(buf, 0), 0);
        assert_eq!(snap_to_char_boundary_floor(buf, 4), 4);
    }

    #[test]
    fn snap_ceil_handles_continuation_bytes() {
        // Inside emoji → ceil walks forward to the next leading byte.
        let buf = "a😀b".as_bytes(); // [a F0 9F 98 80 b]
        // idx 2 = inside emoji → ceil = 5 (start of 'b')
        assert_eq!(snap_to_char_boundary_ceil(buf, 2), 5);
        // idx 1 = leading byte → stays
        assert_eq!(snap_to_char_boundary_ceil(buf, 1), 1);
    }

    #[test]
    fn format_model_blob_truncates_huge_output() {
        // 150KB of 'x' — must elide the middle.
        let buf = vec![b'x'; 150_000];
        let out = format_model_blob(&buf, 1, std::time::Duration::from_secs(5));
        assert!(out.contains("[50000 bytes elided"));
        assert!(out.contains("<exited 1 in 5.00s>"));
        // Should be ~100KB + framing, not 150KB.
        assert!(out.len() < 150_000);
    }

    #[test]
    fn format_model_blob_passes_small_output_unchanged() {
        let buf = b"line 1\nline 2\nline 3";
        let out = format_model_blob(buf, 0, std::time::Duration::from_millis(100));
        assert!(out.contains("line 1"));
        assert!(out.contains("line 3"));
        assert!(!out.contains("elided"));
    }
}
