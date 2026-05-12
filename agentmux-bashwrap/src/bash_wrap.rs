// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `exec` subcommand — runs the user's bash command and streams chunks.
//!
//! Invariants:
//! - The user's command is decoded from `--b64-cmd` so quoting /
//!   multi-line / shell-metachar content survives the rewrite.
//! - The inner command runs under `bash -c` via piped stdio (NOT a PTY).
//!   PTY was tried in β.A and broke on Win10's ConPTY (child exited
//!   with `STATUS_DLL_INIT_FAILED` because `portable_pty`'s master
//!   handle was dropped during child startup — see
//!   `docs/retros/2026-05-11-live-log-streaming-wrapper-failures.md`
//!   §4.2 and §6.3). Plain pipes give us line streaming, which is what
//!   the live-log feature needs; PTY-only behaviors (spinner fidelity,
//!   interactive prompts, CR-only progress bars) are deferred.
//! - Bash is located via `$BASH` → `$AGENTMUX_BASH` → PATH search →
//!   well-known Windows locations (Git Bash). Fails loud if missing.
//! - stdout and stderr are streamed concurrently, each line published
//!   as its own chunk with `kind` set to `"stdout"` or `"stderr"`.
//!   Aggregated stdout (with stderr lines prefixed `[stderr] `) is
//!   captured into a buffer; on exit, a formatted summary lands on
//!   this process's stdout for Claude's native Bash tool to harvest as
//!   the `tool_result` content. Truncated head/tail at 50KB each per
//!   `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §4.5.
//! - If WPS publish fails (missing env, sidecar down, etc.), the
//!   command still runs to completion — output just doesn't stream.
//!   The model-visible blob is unaffected. A `kind: "system"` chunk
//!   announces the degradation at startup.

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};

use crate::wps_client::WpsClient;

/// CLI args for `exec`. `command` carried as base64 to sidestep every
/// quoting concern in the shell that invokes us.
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

/// Cap the model-visible head/tail sections of the aggregated blob.
const MODEL_BLOB_HEAD_BYTES: usize = 50_000;
const MODEL_BLOB_TAIL_BYTES: usize = 50_000;

/// Flush pending bytes to the publisher when buffered without a
/// newline AND the buffer reaches this size — so long lines and
/// newline-free output (minified JSON, CR-only progress bars) still
/// stream live instead of accumulating in memory unbounded.
const FLUSH_BYTES: usize = 4096;

/// Flush pending bytes after this much idle time even if no newline /
/// size threshold has been hit — keeps slow-trickle output visible
/// without waiting for the next byte that would have triggered a
/// natural flush.
const FLUSH_QUIET_WINDOW: Duration = Duration::from_millis(50);

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

/// Internal channel payload: a single line tagged with its source.
/// Bytes are raw — UTF-8 conversion is deferred to the publish /
/// aggregation site so non-UTF-8 output (binary `cat`, Windows
/// legacy-encoded tools) doesn't abort the reader.
#[derive(Debug)]
struct LineEvent {
    kind: &'static str, // "stdout" or "stderr"
    /// Raw bytes including the trailing `\n` if present. May be the
    /// final fragment before EOF without a newline, in which case it
    /// has no trailing delimiter.
    bytes: Vec<u8>,
}

/// Returns the inner command's exit code so main.rs can mirror it as
/// the wrapper's own process exit. Without this, Claude's native Bash
/// tool would see success for every wrapped command regardless of the
/// actual outcome.
pub async fn run(args: Args) -> Result<i32> {
    log_relevant_env();
    let command = decode_command(&args.b64_cmd)?;

    let wps = WpsClient::from_env();
    let degraded = wps.is_none();

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
    let status = run_proc(&args, &command, wps.as_ref(), buffered.clone()).await?;
    let elapsed = start.elapsed();

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

/// Log which streaming-relevant env vars the wrapper actually received,
/// so the sidecar log tells us at a glance whether the env-propagation
/// chain from agentmux-srv → claude → bash → hook → here is intact.
/// Values are not logged (auth key is sensitive); presence/absence is
/// the only signal we need to triage the propagation gap.
fn log_relevant_env() {
    let has = |k: &str| std::env::var(k).is_ok();
    tracing::info!(
        target: "bashwrap",
        auth_key = has("AGENTMUX_AUTH_KEY"),
        local_url = has("AGENTMUX_LOCAL_URL"),
        agent_id = has("AGENTMUX_AGENT_ID"),
        block_id = has("AGENTMUX_BLOCKID"),
        bash = std::env::var("BASH").ok(),
        agentmux_bash = std::env::var("AGENTMUX_BASH").ok(),
        "bashwrap exec env snapshot"
    );
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

async fn publish_line(
    client: &WpsClient,
    tool_id: &str,
    block_id: Option<&str>,
    kind: &str,
    line: &str,
) -> Result<()> {
    client
        .publish_chunk(
            tool_id,
            block_id,
            &ChunkMessage {
                op: "chunk",
                kind,
                content: line,
                timestamp: now_ms(),
            },
        )
        .await
}

/// Locate bash. Order:
/// 1. `$AGENTMUX_BASH` — explicit override from agentmux-srv config.
/// 2. `$BASH` — set by bash itself when it spawns a child; reliable
///    inside Claude Code's hook context because Claude shells out to
///    bash to run the hook.
/// 3. `which bash` on PATH.
/// 4. Well-known Windows paths (Git Bash, msys2). Skipped on Unix.
///
/// Fail-loud if not found — pass-through fallback would hide the issue.
pub(crate) fn locate_bash() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("AGENTMUX_BASH") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    if let Ok(p) = std::env::var("BASH") {
        if !p.is_empty() && PathBuf::from(&p).exists() {
            return Ok(PathBuf::from(p));
        }
    }
    if let Ok(p) = which::which("bash") {
        return Ok(p);
    }
    #[cfg(windows)]
    {
        for candidate in [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files\Git\usr\bin\bash.exe",
            r"C:\Program Files (x86)\Git\bin\bash.exe",
            r"C:\msys64\usr\bin\bash.exe",
            r"C:\msys2\usr\bin\bash.exe",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    Err(anyhow!(
        "bash not found — set AGENTMUX_BASH, install Git Bash, or add bash to PATH"
    ))
}

/// Read raw bytes from a child stdio handle and forward chunks to
/// the publisher with three flush triggers:
///
/// 1. **Newline.** As soon as a `\n` is in the buffer, drain everything
///    up to and including that newline as one chunk. Loop, since a
///    single read can contain multiple complete lines.
/// 2. **Size threshold** (`FLUSH_BYTES`). If the buffer accumulates
///    without a newline (minified JSON output, long progress lines,
///    `printf` without `\n`), flush as soon as it exceeds the
///    threshold. Prevents unbounded memory growth and keeps streaming
///    live for newline-free output.
/// 3. **Quiet-window** (`FLUSH_QUIET_WINDOW`). After this much idle
///    time with non-empty pending bytes, flush. Surfaces slow-trickle
///    output (one byte at a time) that would otherwise wait for the
///    next byte to trigger a natural flush.
async fn stream_reader<R>(mut reader: R, kind: &'static str, tx: mpsc::Sender<LineEvent>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut pending: Vec<u8> = Vec::with_capacity(8192);
    let mut buf = [0u8; 8192];
    loop {
        match tokio::time::timeout(FLUSH_QUIET_WINDOW, reader.read(&mut buf)).await {
            Ok(Ok(0)) => {
                // EOF: drain remainder and exit.
                if !pending.is_empty() {
                    let _ = tx
                        .send(LineEvent { kind, bytes: std::mem::take(&mut pending) })
                        .await;
                }
                return;
            }
            Ok(Ok(n)) => {
                pending.extend_from_slice(&buf[..n]);
                // Drain every complete line.
                while let Some(nl_pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=nl_pos).collect();
                    if tx.send(LineEvent { kind, bytes: line }).await.is_err() {
                        return;
                    }
                }
                // Newline-free residue past the size threshold: flush
                // as one chunk to keep memory bounded + streaming live.
                if pending.len() >= FLUSH_BYTES {
                    if tx
                        .send(LineEvent { kind, bytes: std::mem::take(&mut pending) })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(target: "bashwrap", error = %e, kind, "read error");
                return;
            }
            Err(_elapsed) => {
                // Quiet window: flush any pending bytes so slow-trickle
                // output doesn't sit indefinitely.
                if !pending.is_empty() {
                    if tx
                        .send(LineEvent { kind, bytes: std::mem::take(&mut pending) })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}

/// Spawn the bash child via piped stdio and stream its lines.
///
/// Why pipes instead of a PTY: on Windows, the previous `portable_pty`
/// path dropped `pair.master` immediately after cloning the reader,
/// which tears down the ConPTY mid-startup and yields
/// `STATUS_DLL_INIT_FAILED` for every child. The fix is either to keep
/// `master` alive across `child.wait()` OR drop PTY entirely. The
/// live-log feature wants line streaming, not spinner fidelity, so
/// plain pipes win — they sidestep the entire ConPTY lifetime class.
async fn run_proc(
    args: &Args,
    command: &str,
    wps: Option<&WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
) -> Result<i32> {
    let bash = locate_bash()?;
    tracing::info!(target: "bashwrap", bash = %bash.display(), "spawning bash -c");

    let mut child = Command::new(&bash)
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning bash at {}", bash.display()))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("bash child has no stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("bash child has no stderr"))?;

    let (tx, mut rx) = mpsc::channel::<LineEvent>(1024);
    tokio::spawn(stream_reader(stdout, "stdout", tx.clone()));
    tokio::spawn(stream_reader(stderr, "stderr", tx.clone()));
    drop(tx); // last surviving sender lives in the spawned tasks

    // Publisher: aggregate into buffer + publish to WPS.
    let tool_id = args.tool_id.clone();
    let block_id = args.block_id.clone();
    let wps_clone = wps.cloned();
    let buffered_clone = buffered.clone();
    let publisher_handle = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // Aggregate raw bytes into the model-visible buffer
            // FIRST, before any UTF-8 conversion — preserves binary
            // output fidelity. Stderr lines are prefixed so Claude
            // can reason about them distinctly in the tool_result.
            // `event.bytes` already includes the trailing `\n` from
            // read_until (or omits it on the final EOF fragment).
            {
                let mut buf = buffered_clone.lock().await;
                if event.kind == "stderr" {
                    buf.extend_from_slice(b"[stderr] ");
                }
                buf.extend_from_slice(&event.bytes);
            }

            // For the WPS chunk, the wire format is JSON so we must
            // produce a String. `from_utf8_lossy` replaces invalid
            // sequences with U+FFFD rather than aborting, preserving
            // the model-visible blob's fidelity (kept above) while
            // still publishing a readable line for the overlay.
            // Strip the trailing `\n` from the chunk content so the
            // frontend renderer doesn't add a stray blank line.
            if let Some(client) = wps_clone.as_ref() {
                let mut line_bytes: &[u8] = &event.bytes;
                if line_bytes.last() == Some(&b'\n') {
                    line_bytes = &line_bytes[..line_bytes.len() - 1];
                }
                let line_str = String::from_utf8_lossy(line_bytes);
                if let Err(e) = publish_line(
                    client,
                    &tool_id,
                    block_id.as_deref(),
                    event.kind,
                    &line_str,
                )
                .await
                {
                    tracing::warn!(target: "bashwrap", error = %e, "WPS publish failed");
                }
            }
        }
    });

    // Wait for the child; tokio::process gives us the real exit code.
    let exit_status = child.wait().await.context("waiting for bash child")?;
    let _ = publisher_handle.await;

    // tokio::process exposes Option<i32> for Unix-signal-terminated
    // children; surface -1 in that case so Claude sees a clearly
    // abnormal exit.
    Ok(exit_status.code().unwrap_or(-1))
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
/// replacement characters at the cut point — naive fixed-byte slicing
/// splits multi-byte sequences and the lossy decode emits replacement
/// chars.
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
    use std::sync::Mutex;

    // `std::env::set_var` / `remove_var` mutate process-global state;
    // cargo test runs tests in parallel by default, so without a
    // serial lock tests that touch the env race each other. Mirrors
    // the same pattern in `wps_client::tests`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let original = "echo \"line one\"\necho 'line two'";
        let b64 = URL_SAFE_NO_PAD.encode(original.as_bytes());
        let s = decode_command(&b64).unwrap();
        assert_eq!(s, original);
    }

    #[test]
    fn rejects_malformed_b64() {
        assert!(decode_command("not-valid-base64===").is_err());
    }

    #[test]
    fn format_model_blob_under_cap_passes_body_through() {
        let body = b"hello world\n";
        let out = format_model_blob(body, 0, std::time::Duration::from_millis(123));
        assert!(out.starts_with("<exited 0 in 0.12s>\n"));
        assert!(out.contains("hello world"));
    }

    #[test]
    fn format_model_blob_truncates_over_cap_at_char_boundary() {
        // 200KB of '🌟' (4-byte emoji) > 100KB cap → truncates.
        let s = "🌟".repeat(50_000);
        let bytes = s.as_bytes();
        let out = format_model_blob(bytes, 0, std::time::Duration::from_secs(1));
        // No replacement char `�` (U+FFFD) — boundary snap must keep
        // multi-byte sequences whole at the cut point.
        assert!(
            !out.contains('\u{FFFD}'),
            "found replacement char (lossy decode at boundary)"
        );
        assert!(out.contains("bytes elided"));
    }

    #[test]
    fn snap_floor_is_idempotent_at_ascii() {
        let buf = b"abcdef";
        assert_eq!(snap_to_char_boundary_floor(buf, 3), 3);
        assert_eq!(snap_to_char_boundary_floor(buf, 0), 0);
        assert_eq!(snap_to_char_boundary_floor(buf, 6), 6);
    }

    #[test]
    fn snap_floor_steps_back_off_continuation_bytes() {
        // "é" = [0xC3, 0xA9]; index 1 is a continuation byte.
        let buf = b"\xc3\xa9x";
        assert_eq!(snap_to_char_boundary_floor(buf, 1), 0);
    }

    #[test]
    fn locate_bash_via_explicit_env() {
        // Use the binary that runs the test as a stand-in for "bash"
        // — we only assert that the override is honored, not that the
        // path is a real bash. ENV_LOCK serializes against any future
        // env-touching test (and against `BASH` reads in the same fn).
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let exe = std::env::current_exe().expect("current_exe");
        let prev_agentmux_bash = std::env::var("AGENTMUX_BASH").ok();
        let prev_bash = std::env::var("BASH").ok();
        // Clear BASH so the override path is the one being asserted.
        unsafe {
            std::env::set_var("AGENTMUX_BASH", &exe);
            std::env::remove_var("BASH");
        }
        let found = locate_bash().expect("locate_bash with override");
        assert_eq!(found, exe);
        unsafe {
            match prev_agentmux_bash {
                Some(v) => std::env::set_var("AGENTMUX_BASH", v),
                None => std::env::remove_var("AGENTMUX_BASH"),
            }
            if let Some(v) = prev_bash {
                std::env::set_var("BASH", v);
            }
        }
    }
}
