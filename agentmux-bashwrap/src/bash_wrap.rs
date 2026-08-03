// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `exec` subcommand — runs the user's bash command and streams chunks.
//!
//! Invariants:
//! - The user's command is decoded from `--b64-cmd` so quoting /
//!   multi-line / shell-metachar content survives the rewrite.
//! - The inner command runs under `bash -c` inside a PTY by default
//!   (`run_via_pty`), with a pipe-stdio fallback (`run_via_pipes`)
//!   if PTY allocation fails. The PTY path keeps glibc's stdout
//!   line-buffered so partial chunks reach the overlay in real time;
//!   bash's startup DSR (`\x1b[6n`) is satisfied by pre-loading the
//!   master writer with `\x1b[1;1R`; the writer is held alive until
//!   after child.wait() (dropping it earlier sends CTRL_C_EVENT on
//!   Windows — ConPTY CONIN lifetime invariant, same as pair.master).
//!   The command is wrapped in a brace group redirected from /dev/null
//!   (`{ <cmd>; } </dev/null`) so stdin-reading children see EOF instead
//!   of blocking on the live PTY slave. We must NOT use `exec </dev/null`
//!   for this: running it inside bash closes the child's ConPTY console
//!   input from the child side, which ConPTY reports as a CTRL_C_EVENT,
//!   killing every command with exit 130 before it ran. The group
//!   redirect points only the group's fd 0 at /dev/null while bash's own
//!   console fd stays open, so children get EOF and ConPTY never fires.
//!   The pipe path remains as a safety net and is the only path that
//!   preserves the stdout/stderr split — PTY collapses both onto
//!   one stream. See `docs/specs/SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md`.
//! - Bash is located via `$BASH` → `$AGENTMUX_BASH` → PATH search →
//!   well-known Windows locations (Git Bash). Fails loud if missing.
//! - Each line is published as its own chunk with `kind` set to
//!   `"stdout"` (PTY path always; pipe path for stdout reads) or
//!   `"stderr"` (pipe path only — PTY merges both onto one stream).
//!   Aggregated output (with stderr lines prefixed `[stderr] ` in the
//!   pipe path) is captured into a buffer; on exit, a formatted summary
//!   lands on this process's stdout for Claude's native Bash tool to
//!   harvest as the `tool_result` content. Truncated head/tail at 50KB
//!   each per `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §4.5.
//! - If WPS publish fails (missing env, sidecar down, etc.), the
//!   command still runs to completion — output just doesn't stream.
//!   The model-visible blob is unaffected. A `kind: "system"` chunk
//!   announces the degradation at startup.

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, oneshot};

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

// ─────────────────────────────────────────────────────────────────────────────
// Cwd persistence across one-shot `exec` invocations
//
// Every Bash tool call the PreToolUse hook rewrites spawns a brand-new,
// disposable `agentmux-bashwrap exec` process (see main.rs's module doc —
// there is no daemon). Historically the inner bash's starting directory was
// seeded from `std::env::current_dir()`, i.e. THIS process's own inherited
// cwd, which is always the agent's fixed home directory (nothing in this
// process tree ever chdirs it). A `cd` run by the wrapped command only ever
// affected that one throwaway inner bash; the moment it exited, the change
// was gone, and the NEXT call started over at the same fixed directory again
// — every time, not intermittently. Claude Code's own Bash tool expects `cd`
// continuity across calls within a session and (at least on Windows, where
// this rewrite always applies) surfaces the mismatch as a
// "Shell cwd was reset to ..." notice on effectively every call. See
// docs/retro/RETRO_BASH_CWD_RESET_NOTICE_WINDOWS_2026_08_02.md for the full
// investigation.
//
// The fix: persist the shell's ending `$PWD` to a small per-agent state file
// after each call, and restore it as the starting directory of the next
// call. This doesn't change what Claude Code itself believes (that's opaque,
// out of process) but it does make the ACTUAL directory a `cd` in one Bash
// call leaves the agent in survive to the next call, which is the part that
// was silently broken.
// ─────────────────────────────────────────────────────────────────────────────

/// Env var bashwrap sets on the *child* bash process (never interpolated
/// into the script text, so no path-quoting concerns) pointing at the state
/// file `append_cwd_capture`'s appended script writes the ending `$PWD` to.
const CWD_STATE_ENV: &str = "AGENTMUX_BASHWRAP_CWD_STATE";

/// Optional override of the state file's own path, read from bashwrap's own
/// process env (distinct from `CWD_STATE_ENV`, which bashwrap sets rather
/// than reads). Exists so tests can point persistence at a tempdir instead
/// of a real agent's `~/.agentmux` state, and as an escape hatch for manual
/// debugging.
const CWD_STATE_FILE_OVERRIDE_ENV: &str = "AGENTMUX_BASHWRAP_CWD_STATE_FILE";

/// Resolves the cwd-persistence file location and any previously-restored
/// starting directory once per `exec` invocation.
struct CwdState {
    /// Where the ending `$PWD` gets written after this command runs, for the
    /// *next* `exec` invocation to pick up. `None` if we couldn't resolve a
    /// location or create its parent dir — persistence is then simply
    /// skipped for this call (never a hard failure; falls back to today's
    /// pre-fix behavior).
    path: Option<PathBuf>,
    /// The directory to actually start this command's shell in: the
    /// previously-persisted directory if one was found and still exists,
    /// otherwise this process's own (fixed, inherited) cwd — the same
    /// default used before this fix existed.
    start_dir: Option<PathBuf>,
}

impl CwdState {
    fn load() -> Self {
        let path = cwd_state_path();
        if let Some(dir) = path.as_deref().and_then(Path::parent) {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!(
                    target: "bashwrap",
                    error = %e,
                    dir = %dir.display(),
                    "failed to create cwd-state dir; cwd persistence disabled for this call",
                );
            }
        }
        let start_dir = path
            .as_deref()
            .and_then(restore_cwd)
            .or_else(|| std::env::current_dir().ok());
        Self { path, start_dir }
    }
}

/// Per-agent state file path. Keyed by `AGENTMUX_AGENT_ID` (set by
/// agentmux-srv on every agent spawn — already relied on elsewhere in this
/// file, see `log_relevant_env`) so concurrent agents on the same machine
/// never share state. Falls back to a sanitized form of this process's own
/// cwd (always the agent's fixed home directory) for the rare case the env
/// var is absent, e.g. a manual invocation outside AgentMux.
fn cwd_state_path() -> Option<PathBuf> {
    if let Ok(over) = std::env::var(CWD_STATE_FILE_OVERRIDE_ENV) {
        if !over.is_empty() {
            return Some(PathBuf::from(over));
        }
    }
    let dir = dirs::home_dir()?
        .join(".agentmux")
        .join("state")
        .join("bashwrap-cwd");
    let key = std::env::var("AGENTMUX_AGENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })?;
    Some(dir.join(format!("{}.cwd", sanitize_state_key(&key))))
}

/// Filesystem-safe form of an arbitrary key for use as a filename component.
fn sanitize_state_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Read back a previously-persisted cwd, if the file exists, is non-empty,
/// and still names a real directory (it may not — the directory could have
/// been deleted since, or this is the first call for this agent).
fn restore_cwd(state_path: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(state_path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_dir().then_some(path)
}

/// Appends bookkeeping to `command_block` that (1) captures its real exit
/// code before running anything else, (2) persists the shell's ending
/// `$PWD` to `$AGENTMUX_BASHWRAP_CWD_STATE` if that env var is set (skipped
/// entirely otherwise — e.g. when `CwdState::path` was `None`), written via
/// a temp-file-then-rename so a mid-write crash can't leave a half-written
/// state file, and (3) re-exits with the captured code so this bookkeeping
/// never changes the command's real exit status. `command_block` must be a
/// brace group or bare command, not something that already ends in its own
/// `exit`.
fn append_cwd_capture(command_block: &str) -> String {
    // `pwd` alone prints MSYS-style paths under Git Bash (`/c/Users/...`),
    // which Rust's `std::fs`/`Path::is_dir()` on native Windows can't
    // resolve (it looks for a literal `\c\Users\...` under the current
    // drive root). `-W` is Git Bash's own flag for "print the Windows-
    // native form instead" (`C:/Users/...`), which both `restore_cwd`'s
    // `is_dir()` check and `cmd.cwd()`/`cmd.current_dir()` understand
    // correctly. Not a POSIX `pwd` flag — Unix has no such mismatch to
    // work around, so plain `pwd` is correct there.
    let pwd_cmd = if cfg!(windows) { "pwd -W" } else { "pwd" };
    format!(
        "{command_block}\n\
__agentmux_bashwrap_rc=$?\n\
if [ -n \"${CWD_STATE_ENV}\" ]; then\n\
  {pwd_cmd} > \"${CWD_STATE_ENV}.tmp\" 2>/dev/null && mv -f \"${CWD_STATE_ENV}.tmp\" \"${CWD_STATE_ENV}\" 2>/dev/null\n\
fi\n\
exit $__agentmux_bashwrap_rc"
    )
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

/// If a wrapped command's PTY produces zero bytes of output for this long,
/// assume it's blocked waiting for interactive input that will never come
/// (e.g. `less`/`more` invoked as a pager, since the PTY deliberately makes
/// the child see `isatty(stdout) == true` and this wrapper never writes to
/// the PTY again after the startup DSR response) and forcibly kill it
/// rather than leak this whole process forever. See
/// docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md.
///
/// This is an IDLE timeout, not a total-runtime timeout: a command that's
/// silent for under this long but runs far longer overall (e.g. a build
/// with continuous compiler output over several minutes) is unaffected —
/// only a command producing literally zero bytes for the full window trips
/// it. Overridable via `AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS`.
const DEFAULT_IDLE_KILL_TIMEOUT: Duration = Duration::from_secs(600);

fn idle_kill_timeout() -> Duration {
    std::env::var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_IDLE_KILL_TIMEOUT)
}

/// Kill `pid` AND every descendant it spawned, not just the one process.
/// Supplements `ChildKiller::kill()` (portable-pty's Windows impl is a bare
/// `TerminateProcess` on a single handle, no job object — see the caller's
/// comment) — without this, a wrapped command whose direct child forks
/// further (a pipeline, or `git` spawning `less` as a child) can leave an
/// orphaned grandchild running and still attached to the PTY slave after
/// the "kill" (reagent P1, PR #2156).
///
/// Windows: `taskkill /T /F /PID <pid>` walks the OS-level parent-PID tree
/// (independent of shell job control) and force-kills every process in it.
/// No new dependency needed — this is a plain `std::process::Command`.
///
/// Unix: not implemented here. portable-pty's Unix `ChildKiller` sends
/// SIGHUP (its own doc comment: "we send the SIGHUP signal instead of
/// trying to kill") to what its source suggests is the child's process
/// group, which — if the pty child is a session/group leader, the normal
/// case for an interactive shell — should already reach pipeline
/// descendants without a supplemental step. All evidence for this bug
/// (PR #2156 / RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md) is
/// Windows-specific; adding an unverified Unix code path (which would need
/// a new `libc` dependency for a process-group `kill(-pid, ...)`) is
/// deferred until there's an actual repro to design against.
fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/T", "/F", "/PID", &pid.to_string()]);
        {
            // CREATE_NO_WINDOW: console-flash suppression — bashwrap is a
            // GUI-subsystem parent, so spawning taskkill without this pops a
            // visible console window. std::process::Command needs CommandExt.
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.output() {
            Ok(out) if out.status.success() => {
                tracing::info!(target: "bashwrap", pid, "kill_process_tree: taskkill succeeded");
            }
            Ok(out) => {
                // Non-fatal: the direct-handle kill above may have already
                // won the race (taskkill then reports "not found"), which
                // is fine — best-effort supplemental cleanup either way.
                tracing::warn!(
                    target: "bashwrap",
                    pid,
                    stderr = %String::from_utf8_lossy(&out.stderr),
                    "kill_process_tree: taskkill did not report success"
                );
            }
            Err(e) => {
                tracing::warn!(target: "bashwrap", pid, error = %e, "kill_process_tree: failed to spawn taskkill");
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid; // no supplemental step on this platform — see doc comment above
    }
}

/// Wire payload published on the `tool_chunk` event. The tool_use_id
/// rides in the payload (not the event name) so the frontend opens
/// a single per-block subscription on mount and routes by `tool_id`
/// rather than per-tool subscriptions racing the tool's execution.
#[derive(Serialize)]
struct ChunkMessage<'a> {
    op: &'static str, // "chunk"
    tool_id: &'a str,
    kind: &'a str,    // "stdout" | "stderr" | "system"
    content: &'a str,
    timestamp: u64,
}

#[derive(Serialize)]
struct TerminalMessage<'a> {
    op: &'static str, // "terminal"
    tool_id: &'a str,
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
pub async fn run(mut args: Args) -> Result<i32> {
    log_relevant_env();
    let command = decode_command(&args.b64_cmd)?;

    // `block_id` controls the WPS publish scope. Prefer the explicit
    // CLI arg, but fall back to AGENTMUX_BLOCKID env (set by
    // agentmux-srv when spawning Claude) so the hook doesn't have to
    // pass it explicitly. Without a scope, the frontend's per-block
    // subscription won't receive the events.
    if args.block_id.as_deref().filter(|s| !s.is_empty()).is_none() {
        if let Ok(v) = std::env::var("AGENTMUX_BLOCKID") {
            if !v.is_empty() {
                args.block_id = Some(v);
            }
        }
    }

    let wps = WpsClient::from_env();
    let degraded = wps.is_none();

    let buffered = Arc::new(Mutex::new(Vec::<u8>::with_capacity(64 * 1024)));

    tracing::info!(
        target: "bashwrap",
        tool_id = %args.tool_id,
        block_id = %args.block_id.as_deref().unwrap_or(""),
        degraded,
        command_len = command.len(),
        "exec start"
    );

    if degraded {
        // Surface degradation as a real chunk on stdout (we'll prefix
        // it on the model side too) so the user sees a clear "no
        // streaming this turn" message in the overlay. WPS publish
        // would no-op, so we skip it.
        let warn = b"[bashwrap] warning: streaming disabled (auth/url env missing); command output will only appear on completion\n";
        buffered.lock().await.extend_from_slice(warn);
    } else if let Some(client) = wps.as_ref() {
        match publish_system(
            client,
            &args.tool_id,
            args.block_id.as_deref(),
            &format!("[bashwrap] starting: {} chars", command.len()),
        )
        .await
        {
            Ok(()) => tracing::info!(target: "bashwrap", tool_id = %args.tool_id, "initial publish ok"),
            Err(e) => tracing::warn!(target: "bashwrap", tool_id = %args.tool_id, error = %e, "initial publish failed"),
        }
    }

    let start = std::time::Instant::now();
    let status = run_proc(&args, &command, wps.as_ref(), buffered.clone()).await?;
    let elapsed = start.elapsed();

    if let Some(client) = wps.as_ref() {
        let _ = client
            .publish_chunk(
                args.block_id.as_deref(),
                &TerminalMessage {
                    op: "terminal",
                    tool_id: &args.tool_id,
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
            block_id,
            &ChunkMessage {
                op: "chunk",
                tool_id,
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
            block_id,
            &ChunkMessage {
                op: "chunk",
                tool_id,
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
async fn stream_reader<R>(
    mut reader: R,
    kind: &'static str,
    tx: mpsc::Sender<LineEvent>,
    last_activity: Arc<std::sync::Mutex<std::time::Instant>>,
)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut pending: Vec<u8> = Vec::with_capacity(8192);
    let mut buf = [0u8; 8192];
    // P1b: CR override slot — when the quiet window expires and `pending`
    // starts with `\r`, hold it here rather than flushing immediately.
    // The next read prepends it so collapse_cr can overwrite it with the
    // new frame, collapsing throttled spinner frames (>50 ms apart) before
    // they become separate LineEvents.
    let mut pending_cr_override: Option<Vec<u8>> = None;
    // A1: speculative one-tick hold for a pending line with no leading OR
    // trailing `\r` yet. Mirrors the same slot in `pty_reader_loop`. See
    // docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A1.
    let mut deferred_pending: Option<Vec<u8>> = None;
    loop {
        match tokio::time::timeout(FLUSH_QUIET_WINDOW, reader.read(&mut buf)).await {
            Ok(Ok(0)) => {
                // EOF: flush any held CR override or A1 deferred line, then
                // drain remainder, stripping a dangling lone \r (stream
                // ended mid-spinner or mid-CRLF pair).
                if let Some(mut held) = pending_cr_override.take() {
                    if held.last() == Some(&b'\r') {
                        held.pop();
                    }
                    if !held.is_empty() {
                        let _ = tx.send(LineEvent { kind, bytes: held }).await;
                    }
                }
                if let Some(deferred) = deferred_pending.take() {
                    let _ = tx.send(LineEvent { kind, bytes: deferred }).await;
                }
                if !pending.is_empty() {
                    if pending.last() == Some(&b'\r') {
                        pending.pop();
                    }
                    if !pending.is_empty() {
                        let _ = tx
                            .send(LineEvent { kind, bytes: std::mem::take(&mut pending) })
                            .await;
                    }
                }
                return;
            }
            Ok(Ok(n)) => {
                // Idle-kill tracking (mirrors the PTY path's pty_reader_loop):
                // reset on ANY bytes read, regardless of whether they become
                // a published LineEvent — see run_via_pipes's idle watcher.
                *last_activity.lock().unwrap_or_else(|e| e.into_inner()) = std::time::Instant::now();
                // P1b: prepend any held CR override so collapse_cr can
                // overwrite it with the new frame content.
                if let Some(held) = pending_cr_override.take() {
                    let mut combined = held;
                    combined.extend_from_slice(&buf[..n]);
                    pending.splice(0..0, combined);
                } else if let Some(deferred) = deferred_pending.take() {
                    // A1: resolve last tick's speculative hold. A leading
                    // `\r` means this is the overwrite we were hoping for;
                    // otherwise the deferred line was genuinely final and
                    // unrelated — flush it on its own first.
                    if buf[..n].first() == Some(&b'\r') {
                        let mut combined = deferred;
                        combined.extend_from_slice(&buf[..n]);
                        pending.splice(0..0, combined);
                    } else {
                        if tx.send(LineEvent { kind, bytes: deferred }).await.is_err() {
                            return;
                        }
                        pending.extend_from_slice(&buf[..n]);
                    }
                } else {
                    pending.extend_from_slice(&buf[..n]);
                }
                collapse_cr(&mut pending);
                // Drain every complete line.
                while let Some(nl_pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=nl_pos).collect();
                    if tx.send(LineEvent { kind, bytes: line }).await.is_err() {
                        return;
                    }
                }
                // P2a: newline-free residue past the size threshold — flush
                // unconditionally to keep memory bounded. Prevents unbounded
                // accumulation from a trailing-\r spinner that never emits \n.
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
                // P1b/A1: quiet-window expiry.
                //
                // If a line was already speculatively deferred last tick
                // (A1) and nothing new arrived since — deferred_pending
                // being Some implies pending is empty, since any new data
                // would have resolved it in the Ok(Ok(n)) branch above —
                // give up waiting for a `\r`-prefixed overwrite and flush
                // it now.
                //
                // If `pending` starts with `\r`, it is a leading-\r spinner
                // frame. Stash it in the CR override slot so the next read
                // prepends it and collapse_cr can overwrite it with the new
                // frame.
                //
                // If `pending` ends with `\r` (but not starts), hold it —
                // a following `\n` will form a complete CRLF.
                //
                // Otherwise (non-`\r` partial output, e.g. `printf
                // 'Building...'`): first quiet window with this content —
                // defer it one tick (A1) instead of flushing immediately,
                // in case the very next read starts with `\r`. See
                // docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A1.
                //
                // Note: pending_cr_override / deferred_pending are always
                // None here at this point (after the take() above). Each is
                // set only by take(&mut pending), which empties pending; the
                // only way to reach this branch with non-empty pending is
                // after an Ok(Ok(n)) that already consumed and cleared both.
                if let Some(deferred) = deferred_pending.take() {
                    if tx.send(LineEvent { kind, bytes: deferred }).await.is_err() {
                        return;
                    }
                }
                if !pending.is_empty() {
                    if pending.first() == Some(&b'\r') {
                        pending_cr_override = Some(std::mem::take(&mut pending));
                    } else if pending.last() != Some(&b'\r') {
                        deferred_pending = Some(std::mem::take(&mut pending));
                    }
                    // else: trailing-\r hold — do nothing, next read resolves CRLF.
                }
            }
        }
    }
}

/// Spawn the bash child and stream its lines. PTY by default with a
/// pipe fallback if PTY allocation fails (G6 in
/// docs/specs/SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md).
///
/// Why PTY: when the child sees its stdout as a pipe, glibc switches
/// libc stdio from line-buffered to block-buffered (~4 KB). External
/// programs (grep, npm, cargo, python, ...) accumulate output in the
/// stdio buffer until flush, defeating the live-log overlay's premise.
/// PTY makes `isatty(STDOUT_FILENO) == 1` so glibc stays line-buffered.
///
/// CRITICAL ConPTY lifetime contract (Windows): `pair.master` MUST
/// stay alive across `child.wait()`. Dropping master during child
/// startup tears down the pseudoconsole anchor and produces
/// `STATUS_DLL_INIT_FAILED` (the β.A wedge). See retro §4.2.
///
/// HEADLESS PTY contract (cross-platform): bash queries the PTY at
/// startup with `\x1b[6n` (DSR — request cursor position) and blocks
/// on stdin waiting for a `\x1b[r;cR` response. A headless PTY (no
/// real terminal behind it) never answers — bash never proceeds —
/// `child.wait()` therefore never returns. Our PTY reader detects DSR
/// queries and writes a synthetic `\x1b[1;1R` response back via the
/// master writer. This is what xterm.js does for VS Code's agent-
/// mode terminal; here we do the minimum subset for non-interactive
/// `bash -c`. Verified via agentmux-pty-repro V2 before this landed.
async fn run_proc(
    args: &Args,
    command: &str,
    wps: Option<&WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
) -> Result<i32> {
    let bash = locate_bash()?;
    let pty_system = native_pty_system();
    match pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => {
            tracing::info!(
                target: "bashwrap",
                bash = %bash.display(),
                "spawning bash -c via PTY (live-buffered, headless DSR responder active)",
            );
            run_via_pty(args, command, wps, buffered, &bash, pair).await
        }
        Err(e) => {
            tracing::warn!(
                target: "bashwrap",
                bash = %bash.display(),
                error = %e,
                "PTY allocation failed — falling back to pipes (output may buffer at child level)",
            );
            run_via_pipes(args, command, wps, buffered, &bash).await
        }
    }
}

/// PTY-backed run path. Holds master alive across child.wait() and
/// answers DSR queries from bash at startup.
async fn run_via_pty(
    args: &Args,
    command: &str,
    wps: Option<&WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
    bash: &std::path::Path,
    pair: portable_pty::PtyPair,
) -> Result<i32> {
    let mut cmd = CommandBuilder::new(bash.as_os_str());
    // -c <cmd> for non-interactive run, WITHOUT -l. Login shell
    // startup costs ~1 second (sources /etc/profile, /etc/bashrc,
    // ~/.bash_profile, etc.) which the user sees as a "Running..."
    // delay before any chunk arrives. We pre-prepend the MSYS2 bin
    // dirs to PATH manually (see below) so external commands like
    // `date`, `grep`, `awk` resolve without needing rc-script setup.
    cmd.arg("-c");
    // Give stdin-reading children (`cat`, `read`, prompted scripts) EOF
    // instead of leaving them blocked on the live PTY slave, but do NOT use
    // `exec </dev/null` — running that inside bash closes the child's ConPTY
    // console-input handle from the child side, which ConPTY reports as a
    // CTRL_C_EVENT and kills the command with exit 130 before it runs (the
    // bug this wrapper exists to fix). A brace-group redirect points the
    // group's fd 0 at /dev/null for the command's duration while bash's own
    // fd 0 (the console) stays open, so children see EOF and ConPTY never
    // fires ctrl-c. The leading newline terminates the group list cleanly
    // regardless of how `command` ends (trailing comment, no newline, etc.).
    let cwd_state = CwdState::load();
    // Wrapped a second time by `append_cwd_capture` so the ending `$PWD`
    // survives this disposable process — see the cwd-persistence block
    // above `Args` for why that's necessary.
    cmd.arg(append_cwd_capture(&format!("{{\n{}\n}} </dev/null", command)));

    // Disable pagers. The PTY above deliberately makes the child see
    // `isatty(stdout) == true` so external tools stay line-buffered (see
    // the module doc comment) — but that's exactly what `git`
    // (diff/log/show/branch/...) uses to decide whether to auto-invoke
    // `core.pager` (`less` by default). We never write to the PTY again
    // after the startup DSR response below, so a pager waiting for a
    // keystroke blocks forever and leaks this whole process. `cat` passes
    // content straight through with no paging. See
    // docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md.
    cmd.env("GIT_PAGER", "cat");
    cmd.env("PAGER", "cat");

    // PATH fix-up: bashwrap is a Windows exe, so when MSYS2/Git Bash
    // spawned us it converted PATH to Windows form, which has
    // /c/Windows/system32, /c/Program Files/nodejs, etc. but NOT
    // /usr/bin, /usr/local/bin, /mingw64/bin where coreutils lives.
    // Without -l (above), the child bash inherits the Windows PATH
    // and every external command becomes "command not found".
    //
    // Derive MSYS2 bin dirs from bash's own location — typically
    // bash sits at `C:\Program Files\Git\usr\bin\bash.exe`, so the
    // three dirs we need are reachable as siblings/cousins of that.
    // Pre-pend them to the inherited PATH; the existing Windows
    // entries stay so platform-specific tools (cmd, powershell, gh,
    // ...) still resolve.
    if let Some(usr_bin) = bash.parent() {
        let mut prefix_dirs: Vec<std::ffi::OsString> = vec![usr_bin.to_path_buf().into()];
        if let Some(usr) = usr_bin.parent() {
            // /usr/local/bin sibling of /usr/bin.
            let local_bin = usr.join("local").join("bin");
            if local_bin.exists() {
                prefix_dirs.push(local_bin.into());
            }
            // /mingw64/bin sibling of /usr (Git for Windows layout).
            if let Some(git_root) = usr.parent() {
                let mingw64_bin = git_root.join("mingw64").join("bin");
                if mingw64_bin.exists() {
                    prefix_dirs.push(mingw64_bin.into());
                }
            }
        }
        let existing_path = std::env::var_os("PATH").unwrap_or_default();
        let mut all_dirs: Vec<std::path::PathBuf> =
            prefix_dirs.iter().map(std::path::PathBuf::from).collect();
        if !existing_path.is_empty() {
            for p in std::env::split_paths(&existing_path) {
                all_dirs.push(p);
            }
        }
        let new_path = std::env::join_paths(&all_dirs)
            .context("joining PATH entries for child bash")?;
        cmd.env("PATH", &new_path);
        tracing::info!(
            target: "bashwrap",
            prepended_dirs = ?prefix_dirs,
            "PATH fix-up for child bash (no login-shell startup needed)",
        );
    }
    if let Some(dir) = &cwd_state.start_dir {
        cmd.cwd(dir);
    }
    if let Some(state_path) = &cwd_state.path {
        cmd.env(CWD_STATE_ENV, state_path);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("PTY spawn of bash at {}", bash.display()))?;

    // Split out a killer before `child` moves into the wait task below, so
    // an idle-timeout detected on the reader side (a different task) can
    // still terminate it. See `ChildKiller::clone_killer`'s doc comment —
    // this is exactly the "send it signals independently from a thread
    // that may be blocked in `.wait`" case it exists for.
    //
    // `killer.kill()` alone only terminates THIS ONE process (portable-pty's
    // Windows impl is a bare `TerminateProcess` on the direct handle, no
    // job object) — it does not reach descendants bash forked (a pipeline,
    // or `git` spawning `less` as a child). An orphaned grandchild can
    // survive the kill, stay attached to the PTY slave, and keep blocking
    // forever — reproducing this exact leak one process removed instead of
    // eliminating it (reagent P1, PR #2156). `child_pid` + `kill_process_tree`
    // below is the supplemental fix: a full tree-kill by PID.
    let mut killer = child.clone_killer();
    let child_pid = child.process_id();

    let reader = pair.master.try_clone_reader().context("PTY try_clone_reader")?;

    // Write the DSR response into the master writer, then hold the
    // writer alive until after child.wait(). On Windows, closing the
    // CONIN pipe write-end while the pseudoconsole is still attached
    // sends CTRL_C_EVENT to the child (exit 130 / SIGINT). This is
    // the same ConPTY-lifetime invariant as pair.master itself — both
    // must outlive child.wait(). After the DSR response we never write
    // again; the handle is kept open only to hold CONIN alive.
    let writer = {
        use std::io::Write as _;
        let mut w = pair.master.take_writer().context("PTY take_writer")?;
        let _ = w.write_all(b"\x1b[1;1R");
        let _ = w.flush();
        w
    };

    let (tx, rx) = mpsc::channel::<LineEvent>(1024);
    let tx_reader = tx.clone();
    let (idle_tx, idle_rx) = oneshot::channel::<()>();
    tokio::task::spawn_blocking(move || {
        pty_reader_loop(reader, tx_reader, idle_tx);
    });
    drop(tx);

    let publisher_handle = spawn_publisher_loop(args, wps.cloned(), buffered.clone(), rx);

    // Move pair AND writer into the wait task — both must outlive
    // child.wait() to satisfy the ConPTY lifetime contract on Windows.
    let mut wait_task = tokio::task::spawn_blocking(move || -> Result<i32> {
        let mut child = child;
        let status = child.wait().context("PTY child wait")?;
        let code = status.exit_code() as i32;
        tracing::info!(target: "bashwrap", exit_code = code, "PTY child exited");
        drop(writer);
        drop(pair);
        Ok(code)
    });

    // `idle_rx` resolves for TWO distinct reasons, and they must not be
    // conflated (reagent P1, PR #2156): either `pty_reader_loop` really
    // did send the idle-timeout signal (`Ok(())`), OR its `idle_tx` was
    // simply DROPPED WITHOUT SENDING because the reader hit a normal EOF
    // and returned — which is exactly what happens on ordinary, fast,
    // successful command completion. A oneshot receiver's `.await`
    // resolves (with `Err`) in that drop case too, and `wait_task`
    // finishing is an independent race on a separate spawn_blocking
    // thread — under scheduling variance (or blocking-pool contention from
    // concurrent bashwrap invocations, common per this PR's own retro doc)
    // the reader's drop can resolve first even for a command that ran to
    // completion normally. Treating ANY `idle_rx` resolution as "idle
    // timeout fired" would misclassify and kill perfectly successful fast
    // commands. Only `Ok(())` is a real signal; `Err` just means "not an
    // idle timeout, keep waiting on wait_task directly."
    let mut idle_killed = false;
    let exit_code = tokio::select! {
        res = &mut wait_task => {
            res.context("PTY wait task join")??
        }
        idle_signal = idle_rx => {
            if idle_signal.is_err() {
                // idle_tx dropped without sending — not a real timeout.
                // idle_rx is now consumed either way, so just await the
                // real completion directly instead of re-entering select.
                wait_task.await.context("PTY wait task join")??
            } else {
                idle_killed = true;
                tracing::warn!(
                    target: "bashwrap",
                    tool_id = %args.tool_id,
                    idle_secs = idle_kill_timeout().as_secs(),
                    "child produced no PTY output for the idle timeout — likely \
                     blocked waiting for interactive input (e.g. a pager); killing",
                );
                // Tree-kill FIRST, `killer.kill()` second — order matters.
                // `taskkill /T` walks descendants by their recorded parent PID,
                // which only works while the root (bash) is still enumerable;
                // killing bash first (via killer.kill()) and THEN tree-killing
                // races taskkill against an already-dead root and can leave
                // grandchildren behind (verified empirically: an earlier version
                // of this fix that called killer.kill() first left 1 of 2
                // backgrounded test children alive — the test below is what
                // caught it). killer.kill() still runs afterward as a cheap,
                // harmless belt-and-suspenders in case kill_process_tree itself
                // didn't fully land (e.g. taskkill unavailable).
                if let Some(pid) = child_pid {
                    // spawn_blocking: kill_process_tree shells out synchronously
                    // (std::process::Command::output()) — running it inline here
                    // would block this tokio worker thread for the taskkill
                    // round-trip. Bounded (reagent P1, PR #2156): `taskkill /T`
                    // itself can hang (a documented Windows failure mode —
                    // unresponsive process enumeration) — without this timeout,
                    // an unresponsive taskkill reintroduces the exact unbounded
                    // hang this whole fix exists to eliminate.
                    let _ = tokio::time::timeout(
                        Duration::from_secs(5),
                        tokio::task::spawn_blocking(move || kill_process_tree(pid)),
                    )
                    .await;
                }
                let _ = killer.kill();
                // Bounded grace period for the kill to actually land and for
                // the wait task to finish (which drops writer/pair, releasing
                // the PTY). If it still doesn't, abandon the wait rather than
                // block forever — main()'s process::exit() tears down every
                // thread in this process regardless, once run() returns.
                match tokio::time::timeout(Duration::from_secs(5), &mut wait_task).await {
                    Ok(Ok(Ok(code))) => code,
                    _ => 124, // matches the conventional `timeout` command's exit code
                }
            }
        }
    };

    // `buffered` (read by the caller for the model blob) is populated
    // only by the publisher loop, so drain it BEFORE appending the
    // idle-kill diagnostic (reagent P2, PR #2156) — otherwise genuine
    // pre-kill output still queued in the channel could flush into
    // `buffered` after the diagnostic note, interleaving it ahead of
    // trailing real output instead of strictly after it. Bounded for the
    // same reason as the wait task above: if a surviving grandchild still
    // holds the PTY slave open after the kill, the reader thread's
    // blocking read may never see EOF.
    if tokio::time::timeout(Duration::from_secs(5), publisher_handle)
        .await
        .is_err()
    {
        tracing::warn!(target: "bashwrap", tool_id = %args.tool_id, "publisher drain timed out — proceeding without it");
    }

    if idle_killed {
        buffered.lock().await.extend_from_slice(
            b"\n[bashwrap] command produced no output for the idle timeout and was \
terminated automatically (likely blocked on a pager or other interactive \
prompt this wrapper can never answer, e.g. `git diff`/`log`/`show` auto- \
paging output that doesn't fit one screen). Try `git --no-pager <cmd>` or \
`| cat` on future invocations.\n",
        );
    }

    Ok(exit_code)
}

/// Pipe-backed run path. Safety net for environments where PTY
/// allocation fails (CI, sandboxes). Same byte semantics as the old
/// pipe-only wrapper.
async fn run_via_pipes(
    args: &Args,
    command: &str,
    wps: Option<&WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
    bash: &std::path::Path,
) -> Result<i32> {
    // Same cwd-persistence rationale as run_via_pty — see the block above
    // `Args`.
    let cwd_state = CwdState::load();
    let mut cmd = Command::new(bash);
    cmd.arg("-c")
        .arg(append_cwd_capture(command))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // Defense-in-depth, matching run_via_pty: stdout is a plain pipe
        // here (isatty == false), so git shouldn't auto-page in this path
        // at all — but set it anyway in case some tool pages on a
        // different heuristic. See
        // docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md.
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat");
    if let Some(dir) = &cwd_state.start_dir {
        cmd.current_dir(dir);
    }
    if let Some(state_path) = &cwd_state.path {
        cmd.env(CWD_STATE_ENV, state_path);
    }
    // Windows only: bash.exe is a console-subsystem binary, so spawning it
    // without CREATE_NO_WINDOW pops a new visible (and orphaned-looking)
    // console window for every fallback exec — this path only runs when
    // PTY allocation fails, which is why it slipped past
    // SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20's audit (that pass
    // covered bashwrap's own process subsystem and shell_node.rs, not this
    // rarely-exercised inner spawn). Same fix as shell_node.rs.
    #[cfg(windows)]
    {
        // `cmd` is `tokio::process::Command`, which provides `creation_flags`
        // as a native inherent method on Windows — unlike `std::process::
        // Command`, no `use std::os::windows::process::CommandExt` is needed
        // to call it here. Verified via a clean `cargo check --target
        // x86_64-pc-windows-msvc` (0 errors) and this crate's windows-latest
        // CI job, both passing without the import. See PR #2042 discussion.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("pipe spawn of bash at {}", bash.display()))?;
    // Captured before anything else can consume/reap the child — see the
    // idle-kill branch below for why this matters (reagent P1, PR #2156).
    let child_pid = child.id();

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

    // Idle-kill safety net (reagent P1, PR #2156): this path has no PTY, so
    // it can't hit the pager-hang mechanism specifically (isatty(stdout) ==
    // false here, the whole reason git wouldn't auto-page) — but nothing
    // else bounded child.wait() either, so ANY other hang cause still
    // leaked the wrapper forever, reproducing the root bug this PR exists
    // to fix. Mirrors run_via_pty's idle (not total-runtime) timeout: a
    // shared last-activity clock updated by both stdout/stderr readers on
    // every byte read, watched by a lightweight polling task.
    let last_activity = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));

    let (tx, rx) = mpsc::channel::<LineEvent>(1024);
    let stdout_reader = tokio::spawn(stream_reader(stdout, "stdout", tx.clone(), last_activity.clone()));
    let stderr_reader = tokio::spawn(stream_reader(stderr, "stderr", tx.clone(), last_activity.clone()));
    drop(tx);

    let publisher_handle = spawn_publisher_loop(args, wps.cloned(), buffered.clone(), rx);

    let (idle_tx, idle_rx) = oneshot::channel::<()>();
    let idle_timeout = idle_kill_timeout();
    let watcher_activity = last_activity.clone();
    let idle_watcher = tokio::spawn(async move {
        let mut idle_tx = Some(idle_tx);
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let elapsed = watcher_activity
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed();
            if elapsed >= idle_timeout {
                if let Some(tx) = idle_tx.take() {
                    let _ = tx.send(());
                }
                return;
            }
        }
    });

    // Same Ok/Err distinction as run_via_pty's select (reagent P1 follow-up):
    // idle_rx resolving with Err just means pty_reader_loop-equivalent
    // cleanup dropped it without signaling (not applicable on this path
    // directly, but kept for structural symmetry and defensiveness) — not
    // a real signal either way.
    let mut idle_killed = false;
    let exit_code: i32 = tokio::select! {
        res = child.wait() => {
            idle_watcher.abort();
            res.context("waiting for bash child")?.code().unwrap_or(-1)
        }
        idle_signal = idle_rx => {
            if idle_signal.is_err() {
                child.wait().await.context("waiting for bash child")?.code().unwrap_or(-1)
            } else {
                idle_killed = true;
                tracing::warn!(
                    target: "bashwrap",
                    tool_id = %args.tool_id,
                    idle_secs = idle_kill_timeout().as_secs(),
                    "child produced no output for the idle timeout (pipe path) — killing",
                );
                // Same tree-kill-first ordering and reasoning as run_via_pty
                // (reagent P1, PR #2156): child.start_kill() alone only
                // terminates the direct bash process, not descendants it
                // forked (a backgrounded `&` child, or a pipeline segment) —
                // those would otherwise survive, still attached to the
                // stdout/stderr pipes, leaving the reader tasks waiting for
                // an EOF that never comes. Bounded: taskkill /T can itself
                // hang (a documented Windows failure mode).
                if let Some(pid) = child_pid {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(5),
                        tokio::task::spawn_blocking(move || kill_process_tree(pid)),
                    )
                    .await;
                }
                let _ = child.start_kill();
                match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                    Ok(Ok(status)) => status.code().unwrap_or(-1),
                    _ => 124, // matches the conventional `timeout` command's exit code
                }
            }
        }
    };

    // Reader tasks: bound-JOIN, never `.abort()`. Empirically, aborting a
    // task that's inside `tokio::time::timeout(_, AsyncRead::read(..))` on
    // a just-killed child's Windows pipe leaves something in a state that
    // hangs this whole test/runtime's shutdown — reproduced in isolation
    // while building this fix (a bare `.read().await` with no per-call
    // timeout wrapper was fine to abort; adding the 50ms quiet-window
    // timeout wrapper, matching `stream_reader`'s real structure, is what
    // triggered it). A `JoinHandle` that's simply *dropped* without abort,
    // by contrast, does not cancel the task — it keeps running fully
    // detached and harmlessly finishes reading to EOF on its own once the
    // pipe closes, which is exactly what timing out on the join and
    // moving on (rather than aborting) achieves here.
    if tokio::time::timeout(
        Duration::from_secs(5),
        async { let _ = tokio::join!(stdout_reader, stderr_reader); },
    )
    .await
    .is_err()
    {
        tracing::warn!(target: "bashwrap", tool_id = %args.tool_id, "reader tasks did not finish within the grace period — leaving them to finish in the background rather than aborting (see run_via_pipes's comment)");
    }

    // Bounded for the same reason as run_via_pty: proceed rather than block
    // forever if a hung descendant somehow keeps a pipe end open.
    if tokio::time::timeout(Duration::from_secs(5), publisher_handle)
        .await
        .is_err()
    {
        tracing::warn!(target: "bashwrap", tool_id = %args.tool_id, "publisher drain timed out — proceeding without it");
    }

    if idle_killed {
        buffered.lock().await.extend_from_slice(
            b"\n[bashwrap] command produced no output for the idle timeout and was \
terminated automatically.\n",
        );
    }

    Ok(exit_code)
}

/// PTY reader loop: drains bytes from the master, strips DSR / ANSI
/// control sequences, and forwards line-split chunks to the publisher.
///
/// Uses a thread-bridge + `recv_timeout` to add a quiet-window flush
/// identical to `stream_reader`'s `tokio::time::timeout` approach, but
/// for the blocking sync I/O that the PTY master reader requires.
///
/// **P1a — `collapse_cr`:** called on the `pending` accumulator after
/// each PTY read (after `strip_ansi`), collapsing mid-buffer lone-`\r`
/// overwrites (trailing-`\r` convention, e.g. `"frame\r"`) and embedded
/// multi-frame chunks (e.g. `printf 'f1\rf2\rfinal\n'`). Previously the
/// PTY path passed `\r` bytes intact and relied on `spawn_publisher_loop`'s
/// `pending_cr_line` slot; that slot only fires per-LineEvent so embedded
/// `\r`s in a single write survived uncollapsed.
///
/// **P1b — leading-`\r` spinner collapse:** throttled spinners (npm,
/// cargo, ora, tqdm at ~80-100 ms/frame, above the 50 ms quiet window)
/// arrive as separate reads. When the quiet window expires and `pending`
/// starts with `\r`, the frame is stashed in a `pending_cr_override` slot
/// rather than flushed. The next read prepends it so `collapse_cr`
/// overwrites it with the new frame. The slot is flushed when a
/// `\n`-terminated line arrives or at EOF.
///
/// **P2a — `FLUSH_BYTES` guard:** flushed unconditionally when accumulated
/// size exceeds `FLUSH_BYTES`, preventing unbounded accumulation from a
/// trailing-`\r` spinner that never emits `\n`.
fn pty_reader_loop(
    reader: Box<dyn std::io::Read + Send>,
    tx: mpsc::Sender<LineEvent>,
    idle_tx: oneshot::Sender<()>,
) {
    use std::sync::mpsc as std_mpsc;
    // PTY collapses stdout + stderr onto one stream — the slave's
    // terminal device is a single FD shared by both. Chunks are
    // labelled "stdout" because there is no way to recover the
    // original FD distinction from the master read. The pipe path
    // (`run_via_pipes`) preserves the split.
    let kind: &'static str = "stdout";

    // Offload the blocking read to a dedicated thread so the main
    // loop can use recv_timeout for the quiet-window flush. Bound the
    // channel (64 slots ≈ 512 KiB headroom) so a flooding child can't
    // grow it unbounded — the SyncSender blocks when full, propagating
    // backpressure to the PTY buffer and ultimately to the child process.
    let (data_tx, data_rx) = std_mpsc::sync_channel::<Option<Vec<u8>>>(64);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = reader;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = data_tx.send(None); // EOF sentinel
                    return;
                }
                Ok(n) => {
                    if data_tx.send(Some(buf[..n].to_vec())).is_err() {
                        return; // consumer gone
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "bashwrap", error = %e, "PTY read error");
                    return;
                }
            }
        }
    });

    let mut pending: Vec<u8> = Vec::with_capacity(8192);
    // P1b: CR override slot — mirrors the same slot in stream_reader.
    // When the quiet window expires and `pending` starts with `\r`, hold
    // it here so throttled spinner frames collapse before becoming separate
    // LineEvents.
    let mut pending_cr_override: Option<Vec<u8>> = None;
    // A1: speculative one-tick hold for a pending line that has NO leading
    // OR trailing `\r` yet (the overwhelmingly common real pattern: print a
    // static label once, then every subsequent update is `\r`-prefixed).
    // Distinct from `pending_cr_override` above — that slot only ever holds
    // content that already starts with `\r`. See
    // docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A1.
    let mut deferred_pending: Option<Vec<u8>> = None;
    // A2: tracks whether this tool call's output has scrolled past its
    // first visual line yet — see `normalize_csi_overwrites`'s doc comment
    // for why CUP-to-home (`\x1b[H`) is only a `\r`-equivalent while this
    // is still true.
    let mut still_on_first_line = true;
    // A2: tracks whether the cursor is currently at column 0, across PTY
    // reads — see `normalize_csi_overwrites`'s doc comment for why a
    // standalone EL is only safe to treat as `\r` while this is true, and
    // why it must persist across calls rather than reset per-chunk.
    let mut at_col0 = true;
    // Idle-kill tracking: `last_activity` resets on every byte read from
    // the PTY (regardless of whether it becomes a published LineEvent —
    // even control-sequence-only output means the child is still doing
    // something). `idle_tx` fires exactly once, on the first quiet-window
    // timeout after `idle_kill_timeout()` has elapsed with zero activity.
    // See the constant's doc comment for why this is idle-based rather
    // than a total-runtime cap.
    let idle_timeout = idle_kill_timeout();
    let mut last_activity = std::time::Instant::now();
    let mut idle_tx = Some(idle_tx);
    loop {
        match data_rx.recv_timeout(FLUSH_QUIET_WINDOW) {
            Ok(Some(mut chunk)) => {
                last_activity = std::time::Instant::now();
                let raw_n = chunk.len();
                strip_dsr(&mut chunk);
                // Phase γ (partial): convert the CHA(col-1)/EL cursor-
                // repositioning idioms into `\r` bytes BEFORE strip_ansi
                // deletes them outright, so collapse_cr (below) can treat
                // CSI-based progress-bar redraws as overwrites the same way
                // it already does for bare `\r`. See
                // docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A2.
                normalize_csi_overwrites(&mut chunk, &mut still_on_first_line, &mut at_col0);
                // Phase β: strip remaining ANSI control sequences so
                // the plain-text chunk list doesn't render them as
                // garbled literal characters. Bash emits terminal-
                // init bytes (`\x1b[m`, `\x1b]0;<title>\x07`,
                // `\x1b[?25h`) on every `-c` invocation through a
                // PTY; without this strip every chunk list starts
                // with garbage.
                strip_ansi(&mut chunk);
                tracing::debug!(target: "bashwrap", raw_bytes = raw_n, post_strip = chunk.len(), "PTY read");
                if chunk.is_empty() {
                    continue;
                }
                // P1b: prepend any held CR override so collapse_cr can
                // overwrite it with the new frame content.
                if let Some(held) = pending_cr_override.take() {
                    let mut combined = held;
                    combined.extend_from_slice(&chunk);
                    pending.splice(0..0, combined);
                } else if let Some(deferred) = deferred_pending.take() {
                    // A1: the new chunk resolves last tick's speculative
                    // hold. A leading `\r` means it's the overwrite we were
                    // hoping for — merge and let collapse_cr do the rest,
                    // same as the pending_cr_override case above. Anything
                    // else means the deferred line was genuinely final and
                    // unrelated to this new content — flush it as its own
                    // line first so the two are never wrongly concatenated.
                    if chunk.first() == Some(&b'\r') {
                        let mut combined = deferred;
                        combined.extend_from_slice(&chunk);
                        pending.splice(0..0, combined);
                    } else {
                        if tx.blocking_send(LineEvent { kind, bytes: deferred }).is_err() {
                            return;
                        }
                        pending.extend_from_slice(&chunk);
                    }
                } else {
                    pending.extend_from_slice(&chunk);
                }
                // P1a: collapse lone \r in the accumulated pending buffer,
                // normalising mid-buffer overwrites and embedded multi-frame
                // chunks identically to the pipe path.
                collapse_cr(&mut pending);
                // Any held CR override was already consumed and prepended into
                // `pending` above, so it never survives to this drain loop —
                // mirrors the pipe path in `stream_reader` which omits a
                // take() here for the same reason.
                while let Some(nl_pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=nl_pos).collect();
                    if tx.blocking_send(LineEvent { kind, bytes: line }).is_err() {
                        return;
                    }
                }
                // P2a: size threshold — flush unconditionally to keep memory
                // bounded. Prevents unbounded accumulation from a trailing-\r
                // spinner that never emits \n.
                if pending.len() >= FLUSH_BYTES {
                    if tx.blocking_send(LineEvent {
                        kind,
                        bytes: std::mem::take(&mut pending),
                    }).is_err() {
                        return;
                    }
                }
            }
            Ok(None) => {
                // EOF: flush any held CR override or A1 deferred line first,
                // then drain remainder, stripping a dangling lone \r.
                if let Some(mut held) = pending_cr_override.take() {
                    if held.last() == Some(&b'\r') {
                        held.pop();
                    }
                    if !held.is_empty() {
                        let _ = tx.blocking_send(LineEvent { kind, bytes: held });
                    }
                }
                if let Some(deferred) = deferred_pending.take() {
                    let _ = tx.blocking_send(LineEvent { kind, bytes: deferred });
                }
                if !pending.is_empty() {
                    if pending.last() == Some(&b'\r') {
                        pending.pop();
                    }
                    if !pending.is_empty() {
                        let _ = tx.blocking_send(LineEvent {
                            kind,
                            bytes: std::mem::take(&mut pending),
                        });
                    }
                }
                return;
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                if idle_tx.is_some() && last_activity.elapsed() >= idle_timeout {
                    // Fires once (idle_tx.take() leaves None behind), even
                    // though this branch keeps running every quiet-window
                    // tick after that — the async side kills the child on
                    // receipt, which will eventually produce real EOF/error
                    // here and end the loop normally.
                    if let Some(sender) = idle_tx.take() {
                        let _ = sender.send(());
                    }
                }
                // P1b/A1: quiet-window expiry.
                //
                // If a line was ALREADY speculatively deferred last tick
                // (A1) and nothing new has arrived since — deferred_pending
                // being Some implies pending is empty, since any new data
                // would have resolved it in the Ok(Some) branch above — give
                // up waiting for a `\r`-prefixed overwrite and flush it now.
                //
                // If `pending` starts with `\r`, it is a leading-\r spinner
                // frame. Stash it in the CR override slot so the next read
                // prepends it and collapse_cr overwrites it with the new frame.
                //
                // If `pending` ends with `\r` (but not starts), hold it —
                // a following `\n` can form a complete CRLF.
                //
                // Otherwise (non-`\r` partial output, e.g. `printf
                // 'Building...'`): this is the FIRST quiet window with this
                // content, so defer it one tick (A1) rather than flushing
                // immediately — in case the very next read starts with `\r`
                // and this was actually the first frame of an overwrite
                // sequence (the overwhelmingly common real pattern: print a
                // static label once, then every subsequent update is
                // `\r`-prefixed). See
                // docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A1.
                if let Some(deferred) = deferred_pending.take() {
                    if tx.blocking_send(LineEvent { kind, bytes: deferred }).is_err() {
                        return;
                    }
                }
                if !pending.is_empty() {
                    if pending.first() == Some(&b'\r') {
                        // Leading-\r spinner frame: stash in the override slot so
                        // the next Ok(Some) prepends it and collapse_cr can overwrite
                        // it with the incoming frame.
                        //
                        // Note: pending_cr_override is always None here. The override
                        // is set only by take(&mut pending), which empties pending.
                        // The only way to reach this branch with non-empty pending is
                        // after an Ok(Some) that already consumed and cleared the
                        // override. No flush-prior is needed or reachable.
                        pending_cr_override = Some(std::mem::take(&mut pending));
                    } else if pending.last() != Some(&b'\r') {
                        // First quiet window with this non-\r content: defer
                        // instead of flushing (A1). deferred_pending is
                        // always None here for the same reason as above.
                        deferred_pending = Some(std::mem::take(&mut pending));
                    }
                    // else: trailing-\r hold — do nothing, next read resolves CRLF.
                }
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                // Reader thread died without EOF — flush CR override / A1
                // deferred line, then drain.
                if let Some(mut held) = pending_cr_override.take() {
                    if held.last() == Some(&b'\r') {
                        held.pop();
                    }
                    if !held.is_empty() {
                        let _ = tx.blocking_send(LineEvent { kind, bytes: held });
                    }
                }
                if let Some(deferred) = deferred_pending.take() {
                    let _ = tx.blocking_send(LineEvent { kind, bytes: deferred });
                }
                if !pending.is_empty() {
                    if pending.last() == Some(&b'\r') {
                        pending.pop();
                    }
                    if !pending.is_empty() {
                        let _ = tx.blocking_send(LineEvent {
                            kind,
                            bytes: std::mem::take(&mut pending),
                        });
                    }
                }
                return;
            }
        }
    }
}

/// Normalize CSI cursor-repositioning idioms that map cleanly onto the
/// existing `\r`-based overwrite model, converting them to a literal `\r`
/// byte so `collapse_cr` (which already understands `\r`) collapses them
/// too, with no new cases needed there. Runs on the raw `chunk` BEFORE
/// `strip_ansi`, which otherwise deletes CSI sequences unconditionally with
/// no overwrite-awareness — the "Phase γ" gap `strip_ansi`'s own doc
/// comment flags ("Cursor positioning escapes within the same line").
/// See docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A2.
///
/// `still_on_first_line` and `at_col0` are both caller-owned state that
/// persists across calls for the lifetime of one reader loop — this
/// function is invoked once per PTY read, and a chunk boundary is not a
/// terminal state boundary: a read ending mid-line (no trailing `\n`/`\r`)
/// must carry "cursor is NOT at column 0" into the next call, otherwise a
/// standalone EL split across two reads (`"prefix"` in one chunk,
/// `"\x1b[2Ksuffix"` in the next) would see a freshly-reset `at_col0 ==
/// true` and wrongly convert that EL to `\r`, reproducing the exact
/// mid-line truncation bug this function exists to prevent, just moved
/// from a mid-chunk split to a cross-chunk-read split (reagent P1, PR
/// #2330, caught after the first, chunk-internal-only version of this fix).
///
/// `still_on_first_line`: `true` until the first `\n` is
/// seen in this tool call's output, `false` forever after. Needed because
/// **Windows ConPTY re-serializes a literal `\r` as CUP-to-home
/// (`\x1b[H`)** rather than passing it through unchanged — verified
/// empirically via `a1_e2e_static_label_then_delayed_cr_overwrite_collapses_to_one_line`
/// capturing the raw PTY bytes: a child process writing `\rdone` produces
/// `\x1b[?25l\x1b[Hdone...\x1b[?25h` on the wire, not a literal `\r`. CUP
/// "home" (row 1, col 1) is unambiguous evidence of an overwrite ONLY when
/// nothing has scrolled the visual line off row 1 yet; once real multi-line
/// output exists, `\x1b[H` genuinely means "go back to the first of several
/// lines" (a multi-line redraw, spec §A3, deliberately out of scope) and
/// must NOT be treated as "restart the current line".
///
/// Cases handled, chosen because each means exactly "move to the start of
/// the (only) line so far" without needing true per-column tracking:
///   - CUP to row 1 col 1 (`\x1b[H`, `\x1b[1H`, `\x1b[1;1H`, and the
///     omitted-field variants of each), gated on `still_on_first_line` per
///     above — the ConPTY-observed idiom.
///   - CHA to column 1 (`\x1b[1G` / `\x1b[G`, "cursor to column 1") is
///     literally "move to start of line" — identical to `\r`. Unix PTYs
///     (and any tool not going through ConPTY's re-serialization) may use
///     this form directly.
///   - EL (`\x1b[2K` erase whole line, `\x1b[K`/`\x1b[0K` erase to end of
///     line) is treated as "reset to start of line" too, but ONLY when the
///     cursor is already known to be at column 0 (`*at_col0`) — i.e. the
///     dominant real-world `\r` + EL idiom (`\r\x1b[2K<text>`), where it's
///     genuinely redundant with the `\r` already just emitted. A
///     STANDALONE EL, mid-line (cursor NOT at column 0 — e.g.
///     `prefix\x1b[Ksuffix`), is a real, different case: EL only erases
///     from the cursor onward and never moves it, so collapsing it to `\r`
///     there would make `collapse_cr` truncate `prefix` even though EL
///     never asked for that. That standalone case is left unconverted
///     (falls through to the "unrecognized" branch below, so `strip_ansi`
///     removes the escape byte-for-byte without touching `prefix`) — a
///     bug reagent caught on this exact line (PR #2330): the prior,
///     unconditional version silently discarded `prefix` in
///     `prefix\x1b[Ksuffix\n`.
///
/// CUB (`\x1b[<n>D`, "cursor back n columns") is intentionally NOT handled
/// here — a faithful implementation needs true per-column tracking (a raw
/// byte position isn't a faithful proxy once `n` varies per frame), which
/// `collapse_cr`'s line-offset model doesn't have. Left as a documented
/// follow-up alongside CUU/multi-line redraw (spec §A3) rather than
/// guessed at with an approximation that could truncate the wrong amount.
fn normalize_csi_overwrites(chunk: &mut Vec<u8>, still_on_first_line: &mut bool, at_col0: &mut bool) {
    let mut out: Vec<u8> = Vec::with_capacity(chunk.len());
    let mut i = 0;
    while i < chunk.len() {
        let b = chunk[i];
        if b == b'\n' {
            *still_on_first_line = false;
        }
        if b == 0x1b && i + 1 < chunk.len() && chunk[i + 1] == b'[' {
            let mut j = i + 2;
            while j < chunk.len() && (0x30..=0x3F).contains(&chunk[j]) {
                j += 1;
            }
            if j < chunk.len() && (0x40..=0x7E).contains(&chunk[j]) {
                let params = &chunk[i + 2..j];
                let final_byte = chunk[j];
                let is_cha_col1 = final_byte == b'G' && (params.is_empty() || params == b"1");
                let is_el = final_byte == b'K' && *at_col0;
                let is_cup_home = *still_on_first_line
                    && final_byte == b'H'
                    && is_cup_row1_col1(params);
                if is_cha_col1 || is_el || is_cup_home {
                    out.push(b'\r');
                    *at_col0 = true;
                    i = j + 1;
                    continue;
                }
                // Unrecognized/CUB/mid-line-EL/out-of-scope CSI sequence:
                // fall through untouched, byte by byte, so strip_ansi
                // (which runs next) still recognizes and removes it as a
                // whole — this function only ever substitutes the matched
                // idioms above. Each of those bytes then runs through the
                // ordinary `*at_col0 = b == b'\n' || b == b'\r'` update
                // below like any other byte, which forces `at_col0` to
                // `false` (none of an escape sequence's bytes are `\n`/
                // `\r`) — conservative and safe (errs toward NOT
                // collapsing a subsequent EL), not "left as-is".
            }
        }
        out.push(b);
        *at_col0 = b == b'\n' || b == b'\r';
        i += 1;
    }
    *chunk = out;
}

/// Does a CUP (`\x1b[<params>H`) params string address row 1, column 1
/// ("home")? Each field defaults to 1 when empty, per ECMA-48 — so `""`,
/// `"1"`, `"1;1"`, `";1"`, `"1;"`, and `";"` are all row-1-col-1.
fn is_cup_row1_col1(params: &[u8]) -> bool {
    let field_is_one_or_empty = |f: &[u8]| f.is_empty() || f == b"1";
    match params.iter().position(|&b| b == b';') {
        None => field_is_one_or_empty(params),
        Some(idx) => {
            field_is_one_or_empty(&params[..idx]) && field_is_one_or_empty(&params[idx + 1..])
        }
    }
}

/// Scan `chunk` for DSR `\x1b[6n` sequences and remove them in place
/// so they don't leak into the model-visible blob or overlay log.
fn strip_dsr(chunk: &mut Vec<u8>) {
    const DSR: &[u8] = b"\x1b[6n";
    let mut i = 0;
    while i + DSR.len() <= chunk.len() {
        if &chunk[i..i + DSR.len()] == DSR {
            chunk.drain(i..i + DSR.len());
        } else {
            i += 1;
        }
    }
}

/// Phase β (lite): strip ANSI control sequences from a PTY chunk so
/// the overlay's plain-text ChunkList doesn't render them as garbled
/// literal characters.
///
/// Three families to handle (everything bash emits during init and
/// most programs emit during normal output):
///
/// 1. **CSI sequences** — `ESC [ <params> <final>` where params are
///    `0x30..=0x3F` (digits, `;`, `?`, etc.) and final is
///    `0x40..=0x7E` (letters + punctuation). Covers `\x1b[m` (reset),
///    `\x1b[?25h/l` (cursor visibility), color codes, cursor moves.
///
/// 2. **OSC sequences** — `ESC ] <text> BEL` (or `ESC ] <text> ESC \`).
///    Covers `\x1b]0;<title>\x07` (set window title — emitted by
///    Git Bash on every command).
///
/// 3. **Lone control chars** — `\r` (carriage return without
///    newline), `\x07` (bell), `\x08` (backspace). CRLF → LF for
///    the chunk list. Lone `\r` passes through so `collapse_cr` can
///    operate on the `pending` buffer across PTY reads (see below).
///
/// Things we DON'T handle yet (Phase γ territory):
/// - Alt-screen apps (`\x1b[?1049h`) — left in the stream.
/// - Cursor positioning escapes within the same line.
/// - OSC 633 (shell integration markers).
fn strip_ansi(chunk: &mut Vec<u8>) {
    let mut out: Vec<u8> = Vec::with_capacity(chunk.len());
    let mut i = 0;
    while i < chunk.len() {
        let b = chunk[i];
        if b == 0x1b && i + 1 < chunk.len() {
            match chunk[i + 1] {
                b'[' => {
                    // CSI: scan params (0x30..=0x3F) then final (0x40..=0x7E).
                    let mut j = i + 2;
                    while j < chunk.len() && (0x30..=0x3F).contains(&chunk[j]) {
                        j += 1;
                    }
                    // Intermediate bytes (rare for our case).
                    while j < chunk.len() && (0x20..=0x2F).contains(&chunk[j]) {
                        j += 1;
                    }
                    if j < chunk.len() && (0x40..=0x7E).contains(&chunk[j]) {
                        // Consume the whole CSI sequence.
                        i = j + 1;
                        continue;
                    }
                    // Malformed — keep ESC, skip past.
                    out.push(b);
                    i += 1;
                    continue;
                }
                b']' => {
                    // OSC: scan until BEL (0x07) or ST (`ESC \`).
                    let mut j = i + 2;
                    while j < chunk.len() {
                        if chunk[j] == 0x07 {
                            i = j + 1;
                            break;
                        }
                        if chunk[j] == 0x1b && j + 1 < chunk.len() && chunk[j + 1] == b'\\' {
                            i = j + 2;
                            break;
                        }
                        j += 1;
                    }
                    if i <= j {
                        // Reached end of chunk without terminator —
                        // drop the rest as a partial OSC. (Next read
                        // will resume with the trailing bytes; if it
                        // doesn't start with the terminator we'll
                        // mis-render once, but that's better than
                        // emitting garbage.)
                        i = chunk.len();
                    }
                    continue;
                }
                _ => {
                    // ESC followed by some other byte (e.g. `ESC c` =
                    // reset). Consume both.
                    i += 2;
                    continue;
                }
            }
        }
        if b == 0x07 {
            // Bare BEL — drop.
            i += 1;
            continue;
        }
        if b == b'\r' && i + 1 < chunk.len() && chunk[i + 1] == b'\n' {
            // CRLF → LF: drop the \r, keep the \n on the next pass.
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    *chunk = out;
}

/// Collapse lone carriage-returns in the `pending` byte buffer, simulating
/// terminal overwrite: each `\r` NOT followed by `\n` discards the current
/// visual line back to column 0.
///
/// **Why here and not in `strip_ansi`:** real spinner animations emit each
/// frame as a separate `write()` syscall, so the preceding frame content and
/// the overwriting `\r` arrive in different PTY reads. Running collapse on
/// the `pending` accumulator (which spans reads for the current visual line)
/// catches those cross-read overwrites.
///
/// **Leading `\r` is preserved — including through mid-buffer overwrites.**
/// When a lone `\r` appears at the very start of a visual line, it is emitted
/// as-is rather than discarded. When a subsequent lone `\r` overwrites a line
/// that *itself* started with `\r` (e.g. `"\rframe1\rframe2"`), the leading
/// `\r` is preserved in the result (`"\rframe2"`, not `"frame2"`). This lets
/// the caller's `pending_cr_override` slot chain-collapse throttled spinner
/// frames on successive reads: the collapsed result still starts with `\r`, so
/// the slot engages again on the next quiet-window expiry.
///
/// **Trailing `\r` is left in place.** A `\r` at the very end of `pending`
/// might be the first byte of a CRLF pair split across reads. Collapsing it
/// immediately would discard the line's content before the `\n` arrives,
/// causing silent data loss. The caller suppresses the eager partial-line
/// flush when `pending` ends with `\r` so the next read can disambiguate.
fn collapse_cr(pending: &mut Vec<u8>) {
    if !pending.contains(&b'\r') {
        return; // fast-path: no CR at all
    }
    let mut out: Vec<u8> = Vec::with_capacity(pending.len());
    let mut line_start = 0usize;
    let mut i = 0;
    while i < pending.len() {
        let b = pending[i];
        if b == b'\n' {
            out.push(b);
            line_start = out.len();
            i += 1;
        } else if b == b'\r' {
            if i + 1 < pending.len() {
                if pending[i + 1] == b'\n' {
                    // CRLF → LF (strip_ansi already handles this for
                    // within-chunk pairs, but CRLF can straddle reads).
                    out.push(b'\n');
                    line_start = out.len();
                    i += 2;
                } else if out.len() == line_start {
                    // Leading \r: nothing to overwrite on this line.
                    // Preserve it so the caller's pending_cr_override slot
                    // can recognise and hold leading-\r spinner frames
                    // ("\rframe", npm/ora/tqdm style).
                    out.push(b);
                    i += 1;
                } else {
                    // Lone \r mid-buffer: discard current line back to column 0.
                    // If the line opened with a leading \r (spinner convention),
                    // preserve that leading \r so the pending_cr_override slot in
                    // the reader can still recognise the result as a spinner frame
                    // after collapsing "\rframe1\rframe2" → "\rframe2".
                    let keep =
                        usize::from(out.len() > line_start && out[line_start] == b'\r');
                    out.truncate(line_start + keep);
                    i += 1;
                }
            } else {
                // Trailing \r — can't disambiguate yet; leave for next read.
                out.push(b);
                i += 1;
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    *pending = out;
}

/// Shared publisher loop — drains the LineEvent channel, aggregates
/// into the model-visible buffer, and publishes each line via WPS.
///
/// **Leading-`\r` spinner handling:** throttled spinner frames (npm, cargo,
/// ora, tqdm at >50 ms/frame) are collapsed upstream — in the
/// `pending_cr_override` slot inside `stream_reader` (pipe path) and
/// `pty_reader_loop` (PTY path) — before reaching this loop. The
/// `pending_cr_line` slot here remains as a secondary layer: when a frame
/// arrives with a leading `\r`, the prior stored frame is published first
/// (so no frame is silently dropped), then the new frame is stored and
/// flushed at the next non-`\r` line or EOF. Consecutive spinner frames
/// therefore appear as sequential published chunks in the live log.
///
/// The model-visible buffer (`buffered`) receives the post-collapse LineEvent
/// bytes. Intermediate spinner frames collapsed by `pending_cr_override` never
/// become LineEvents and are not present in `buffered`; the model sees only the
/// surviving final frame for each overwrite sequence.
fn spawn_publisher_loop(
    args: &Args,
    wps: Option<WpsClient>,
    buffered: Arc<Mutex<Vec<u8>>>,
    mut rx: mpsc::Receiver<LineEvent>,
) -> tokio::task::JoinHandle<()> {
    let tool_id = args.tool_id.clone();
    let block_id = args.block_id.clone();
    tokio::spawn(async move {
        let mut chunks_published = 0u64;
        let mut chunks_failed = 0u64;

        // Pending leading-\r spinner frame: holds (content_after_cr, kind).
        // Flushed on the next non-\r line or at EOF.
        let mut pending_cr_line: Option<(String, &'static str)> = None;

        while let Some(event) = rx.recv().await {
            {
                let mut buf = buffered.lock().await;
                if event.kind == "stderr" {
                    buf.extend_from_slice(b"[stderr] ");
                }
                // Strip a single leading `\r` carried by a collapsed leading-\r
                // spinner frame (e.g. a final "\r✔ done\n"), matching the WPS
                // live path's `strip_prefix('\r')` below, so the model-visible
                // blob and the live log agree byte-for-byte. The trailing `\n`
                // is preserved here — it separates lines in the blob.
                let body: &[u8] = match event.bytes.first() {
                    Some(&b'\r') => &event.bytes[1..],
                    _ => &event.bytes,
                };
                buf.extend_from_slice(body);
            }
            if let Some(client) = wps.as_ref() {
                let mut line_bytes: &[u8] = &event.bytes;
                if line_bytes.last() == Some(&b'\n') {
                    line_bytes = &line_bytes[..line_bytes.len() - 1];
                }
                let line_str = String::from_utf8_lossy(line_bytes);

                if let Some(stripped) = line_str.strip_prefix('\r') {
                    // Leading-\r spinner frame: publish any prior pending frame
                    // before storing the new one, so consecutive \r-prefixed
                    // lines are not silently dropped from the live WPS log.
                    // (This pending slot only affects the live WPS log; the
                    // model-visible blob already received this frame's bytes
                    // above, with the leading \r stripped to match.)
                    if let Some((prior_text, prior_kind)) = pending_cr_line.take() {
                        match publish_line(
                            client,
                            &tool_id,
                            block_id.as_deref(),
                            prior_kind,
                            &prior_text,
                        )
                        .await
                        {
                            Ok(()) => chunks_published += 1,
                            Err(e) => {
                                chunks_failed += 1;
                                tracing::warn!(
                                    target: "bashwrap",
                                    tool_id = %tool_id,
                                    error = %e,
                                    "WPS publish failed (prior pending_cr_line)"
                                );
                            }
                        }
                    }
                    pending_cr_line = Some((stripped.to_owned(), event.kind));
                } else {
                    // Non-\r line: flush any pending spinner frame first.
                    if let Some((cr_text, cr_kind)) = pending_cr_line.take() {
                        match publish_line(
                            client,
                            &tool_id,
                            block_id.as_deref(),
                            cr_kind,
                            &cr_text,
                        )
                        .await
                        {
                            Ok(()) => chunks_published += 1,
                            Err(e) => {
                                chunks_failed += 1;
                                tracing::warn!(
                                    target: "bashwrap",
                                    tool_id = %tool_id,
                                    error = %e,
                                    "WPS publish failed"
                                );
                            }
                        }
                    }
                    match publish_line(
                        client,
                        &tool_id,
                        block_id.as_deref(),
                        event.kind,
                        &line_str,
                    )
                    .await
                    {
                        Ok(()) => chunks_published += 1,
                        Err(e) => {
                            chunks_failed += 1;
                            tracing::warn!(
                                target: "bashwrap",
                                tool_id = %tool_id,
                                error = %e,
                                "WPS publish failed"
                            );
                        }
                    }
                }
            }
        }

        // EOF: flush any remaining pending spinner frame.
        if let Some((cr_text, cr_kind)) = pending_cr_line.take() {
            if let Some(client) = wps.as_ref() {
                match publish_line(client, &tool_id, block_id.as_deref(), cr_kind, &cr_text).await
                {
                    Ok(()) => chunks_published += 1,
                    Err(e) => {
                        chunks_failed += 1;
                        tracing::warn!(
                            target: "bashwrap",
                            tool_id = %tool_id,
                            error = %e,
                            "WPS publish failed (EOF flush)"
                        );
                    }
                }
            }
        }

        tracing::info!(
            target: "bashwrap",
            tool_id = %tool_id,
            chunks_published,
            chunks_failed,
            "publisher done"
        );
    })
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

    // ── cwd persistence tests ───────────────────────────────────────────────
    // See docs/retro/RETRO_BASH_CWD_RESET_NOTICE_WINDOWS_2026_08_02.md for
    // the bug this exists to fix: each `exec` invocation used to always seed
    // its inner bash's cwd from this process's own (fixed) cwd, silently
    // discarding whatever directory a previous call's `cd` left the agent
    // in.

    /// Unique-per-call scratch directory under the OS temp dir, so parallel
    /// test runs (cargo's default) never share state. Mirrors the marker-tag
    /// pattern `run_via_pty_kills_idle_child_and_returns_promptly` already
    /// uses for the same reason.
    fn unique_temp_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentmux-bashwrap-test-{tag}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create unique temp dir");
        dir
    }

    #[test]
    fn sanitize_state_key_replaces_unsafe_chars() {
        assert_eq!(sanitize_state_key("agent3-0630k"), "agent3-0630k");
        assert_eq!(sanitize_state_key("Agent With Spaces"), "Agent_With_Spaces");
        assert_eq!(sanitize_state_key(r"C:\Users\asafe"), "C__Users_asafe");
    }

    #[test]
    fn cwd_state_path_honors_explicit_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = unique_temp_dir("override");
        let target = scratch.join("explicit.cwd");
        unsafe {
            std::env::set_var(CWD_STATE_FILE_OVERRIDE_ENV, &target);
        }
        let resolved = cwd_state_path();
        unsafe {
            std::env::remove_var(CWD_STATE_FILE_OVERRIDE_ENV);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        assert_eq!(resolved, Some(target));
    }

    #[test]
    fn cwd_state_path_derives_from_agent_id_when_no_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_override = std::env::var(CWD_STATE_FILE_OVERRIDE_ENV).ok();
        let prev_agent = std::env::var("AGENTMUX_AGENT_ID").ok();
        unsafe {
            std::env::remove_var(CWD_STATE_FILE_OVERRIDE_ENV);
            std::env::set_var("AGENTMUX_AGENT_ID", "Test-Agent-42");
        }
        let resolved = cwd_state_path();
        unsafe {
            match prev_override {
                Some(v) => std::env::set_var(CWD_STATE_FILE_OVERRIDE_ENV, v),
                None => std::env::remove_var(CWD_STATE_FILE_OVERRIDE_ENV),
            }
            match prev_agent {
                Some(v) => std::env::set_var("AGENTMUX_AGENT_ID", v),
                None => std::env::remove_var("AGENTMUX_AGENT_ID"),
            }
        }
        let resolved = resolved.expect("home dir should resolve in any test environment");
        assert!(resolved.ends_with("Test-Agent-42.cwd"), "{resolved:?}");
        assert!(resolved.to_string_lossy().contains(".agentmux"));
    }

    #[test]
    fn restore_cwd_none_when_file_missing() {
        let scratch = unique_temp_dir("missing");
        let missing = scratch.join("does-not-exist.cwd");
        assert_eq!(restore_cwd(&missing), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_cwd_none_when_persisted_dir_no_longer_exists() {
        let scratch = unique_temp_dir("stale");
        let state_file = scratch.join("state.cwd");
        let gone = scratch.join("deleted-dir");
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::write(&state_file, gone.to_string_lossy().as_bytes()).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert_eq!(restore_cwd(&state_file), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_cwd_some_when_persisted_dir_exists() {
        let scratch = unique_temp_dir("valid");
        let state_file = scratch.join("state.cwd");
        std::fs::write(&state_file, scratch.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(restore_cwd(&state_file), Some(scratch.clone()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_cwd_ignores_blank_file() {
        let scratch = unique_temp_dir("blank");
        let state_file = scratch.join("state.cwd");
        std::fs::write(&state_file, b"   \n").unwrap();
        assert_eq!(restore_cwd(&state_file), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn append_cwd_capture_preserves_exit_code_and_gates_on_env() {
        let script = append_cwd_capture("false");
        // The real command's exit code must survive the appended
        // bookkeeping regardless of whether the state env var is set.
        assert!(script.contains("__agentmux_bashwrap_rc=$?"));
        assert!(script.trim_end().ends_with("exit $__agentmux_bashwrap_rc"));
        // The write is gated on the env var being non-empty, and uses
        // temp-then-rename so a crash mid-write can't corrupt the file.
        assert!(script.contains(&format!("-n \"${CWD_STATE_ENV}\"")));
        assert!(script.contains(&format!("${CWD_STATE_ENV}.tmp")));
        assert!(script.contains("mv -f"));
    }

    /// End-to-end proof of the actual fix: running a `cd` in one `exec`
    /// invocation must change the starting directory of the *next*
    /// invocation, via the persisted state file — not just leave a
    /// same-process illusion of persistence.
    #[tokio::test]
    async fn run_via_pipes_persists_cwd_across_invocations() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = unique_temp_dir("e2e-pipes");
        let state_file = scratch.join("state.cwd");
        let target_dir = scratch.join("moved-here");
        std::fs::create_dir_all(&target_dir).unwrap();

        let prev = std::env::var(CWD_STATE_FILE_OVERRIDE_ENV).ok();
        unsafe {
            std::env::set_var(CWD_STATE_FILE_OVERRIDE_ENV, &state_file);
        }

        let bash = locate_bash().expect("locate_bash for test — same dependency the whole binary needs");
        let args = Args {
            tool_id: "test-cwd-persist".to_string(),
            b64_cmd: String::new(),
            block_id: None,
        };

        // Call 1: cd into target_dir. Nothing about this process's own env
        // changes — the fix must work purely through the state file.
        let cd_cmd = format!("cd \"{}\"", target_dir.display());
        let buffered1 = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        let status1 = run_via_pipes(&args, &cd_cmd, None, buffered1, &bash)
            .await
            .expect("call 1 should succeed");
        assert_eq!(status1, 0, "cd should succeed");

        // Call 2: a fresh, unrelated invocation with no cd of its own —
        // `pwd` should report target_dir, proving the restored starting
        // directory came from call 1's persisted state, not from this
        // process's own (unchanged) cwd.
        let buffered2 = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        let status2 = run_via_pipes(&args, "pwd", None, buffered2.clone(), &bash)
            .await
            .expect("call 2 should succeed");
        assert_eq!(status2, 0);

        unsafe {
            match prev {
                Some(v) => std::env::set_var(CWD_STATE_FILE_OVERRIDE_ENV, v),
                None => std::env::remove_var(CWD_STATE_FILE_OVERRIDE_ENV),
            }
        }

        let pwd_output = String::from_utf8_lossy(&buffered2.lock().await).trim().to_string();
        let expected_name = target_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap()
            .to_string();
        let _ = std::fs::remove_dir_all(&scratch);

        // Compare by trailing path component only — Windows `pwd` under Git
        // Bash reports an MSYS-style path (`/c/...`), not the Windows-style
        // path this test constructed the `cd` from.
        assert!(
            pwd_output.ends_with(&expected_name),
            "expected pwd output to end in {expected_name:?}, got {pwd_output:?}"
        );
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

    // ── collapse_cr tests ────────────────────────────────────────────────────

    fn cr(input: &[u8]) -> Vec<u8> {
        let mut v = input.to_vec();
        collapse_cr(&mut v);
        v
    }

    #[test]
    fn collapse_cr_no_cr_is_noop() {
        assert_eq!(cr(b"hello\nworld\n"), b"hello\nworld\n");
    }

    #[test]
    fn collapse_cr_trailing_cr_preserved() {
        // Trailing \r must not be collapsed — may be first byte of CRLF split
        // across reads. The caller suppresses the partial flush in this case.
        assert_eq!(cr(b"frame1\r"), b"frame1\r");
    }

    #[test]
    fn collapse_cr_crlf_within_buffer_becomes_lf() {
        assert_eq!(cr(b"line\r\n"), b"line\n");
    }

    #[test]
    fn collapse_cr_crlf_straddle_resolved_on_second_call() {
        // Simulate two reads: first leaves trailing \r, second sees \n next.
        // collapse_cr is called after each extend.
        let mut pending = b"line1\r".to_vec();
        collapse_cr(&mut pending);
        assert_eq!(pending, b"line1\r"); // trailing \r preserved
        pending.extend_from_slice(b"\nmore");
        collapse_cr(&mut pending);
        assert_eq!(pending, b"line1\nmore"); // CRLF resolved, "more" accumulated
    }

    #[test]
    fn collapse_cr_trailing_spinner_frames_collapse() {
        // Trailing-\r convention: "frame\r" per write, accumulated before \n.
        let mut pending = b"frame1\r".to_vec();
        collapse_cr(&mut pending);
        // trailing \r kept, frame1 still present
        assert_eq!(pending, b"frame1\r");
        pending.extend_from_slice(b"frame2\r");
        collapse_cr(&mut pending);
        // \r mid-buffer (pos 6, followed by 'f'): frame1 collapsed, trailing \r kept
        assert_eq!(pending, b"frame2\r");
        pending.extend_from_slice(b"done\n");
        collapse_cr(&mut pending);
        // \r mid-buffer (followed by 'd'): frame2 collapsed, done\n remains
        assert_eq!(pending, b"done\n");
    }

    #[test]
    fn collapse_cr_leading_spinner_frames_collapse() {
        // Leading-\r convention: "\rframe" per write (npm/ora/gh style).
        // Case 1: non-\r line overwritten by \r-prefixed content (normal path).
        let mut pending = b"frame1".to_vec();
        collapse_cr(&mut pending);
        assert_eq!(pending, b"frame1"); // no \r, unchanged
        pending.extend_from_slice(b"\rframe2");
        collapse_cr(&mut pending);
        // "frame1\rframe2": \r at pos 6 overwrites non-\r line → "frame2"
        assert_eq!(pending, b"frame2");
        pending.extend_from_slice(b"\rdone\n");
        collapse_cr(&mut pending);
        // "frame2\rdone\n": \r at pos 6 overwrites non-\r line, done\n remains
        assert_eq!(pending, b"done\n");
    }

    #[test]
    fn collapse_cr_leading_cr_multi_frame_preserves_leading_cr() {
        // Case 2: \r-prefixed line overwritten by another \r-prefixed line.
        // "\rframe1\rframe2" must collapse to "\rframe2", NOT "frame2".
        // Without the leading-\r preservation, the mid-buffer \r at pos 7
        // would truncate to line_start=0, discarding the leading \r, and
        // pending_cr_override can no longer detect the result as a spinner.
        assert_eq!(cr(b"\rframe1\rframe2"), b"\rframe2");
        // Three frames: only last survives, leading \r preserved.
        assert_eq!(cr(b"\rframe1\rframe2\rframe3"), b"\rframe3");
        // Normal (non-leading-\r) line overwritten: no leading \r in result.
        assert_eq!(cr(b"frame1\rframe2"), b"frame2");
    }

    #[test]
    fn collapse_cr_preserves_prior_lines() {
        // \r only rewinds to the start of the current visual line, not past \n.
        assert_eq!(cr(b"line1\npartial\rreplace\n"), b"line1\nreplace\n");
    }

    // ── normalize_csi_overwrites tests (spec §A2) ──────────────────────────

    fn csi(input: &[u8]) -> Vec<u8> {
        let mut v = input.to_vec();
        let mut still_on_first_line = true;
        let mut at_col0 = true;
        normalize_csi_overwrites(&mut v, &mut still_on_first_line, &mut at_col0);
        v
    }

    #[test]
    fn normalize_csi_cha_col1_becomes_cr() {
        assert_eq!(csi(b"progress\x1b[1Gdone"), b"progress\rdone");
        // Bare \x1b[G (no param) means column 1 too, per ECMA-48 default.
        assert_eq!(csi(b"progress\x1b[Gdone"), b"progress\rdone");
    }

    #[test]
    fn normalize_csi_cha_other_column_left_untouched() {
        // Only column 1 maps cleanly onto "\r" (start of line); other
        // columns are intentionally out of scope (need true column
        // tracking) and must survive unmangled for strip_ansi to remove.
        let out = csi(b"progress\x1b[10Gdone");
        assert!(!out.contains(&b'\r'), "unrelated CHA column must not synthesize a \\r");
        assert_eq!(out, b"progress\x1b[10Gdone");
    }

    #[test]
    fn normalize_csi_el_at_line_start_becomes_cr() {
        // EL at the very start of a chunk (nothing written on this line
        // yet) is safe: there's nothing to lose by treating it as \r.
        assert_eq!(csi(b"\x1b[2Kfresh"), b"\rfresh");
        assert_eq!(csi(b"\x1b[Kfresh"), b"\rfresh");
        assert_eq!(csi(b"\x1b[0Kfresh"), b"\rfresh");
        // Same, after a real newline -- also column 0.
        assert_eq!(csi(b"line1\n\x1b[2Kfresh"), b"line1\n\rfresh");
    }

    #[test]
    fn normalize_csi_standalone_mid_line_el_is_left_untouched_not_synthesized_to_cr() {
        // reagent P1, PR #2330: EL only erases from the cursor onward and
        // never moves it -- it is NOT equivalent to \r when real content
        // ("stale") already precedes it on the line. The previous,
        // unconditional version turned this into "stale\rfresh", which
        // collapse_cr then truncated to just "fresh", silently discarding
        // "stale" even though EL never asked for that. Leaving it
        // unconverted here means strip_ansi removes only the escape bytes
        // afterward, preserving "stale" + "fresh" concatenated.
        let out = csi(b"stale\x1b[2Kfresh");
        assert!(!out.contains(&b'\r'), "mid-line EL must not synthesize a \\r: {out:?}");
        assert_eq!(out, b"stale\x1b[2Kfresh");

        let out_k = csi(b"stale\x1b[Kfresh");
        assert!(!out_k.contains(&b'\r'));
        assert_eq!(out_k, b"stale\x1b[Kfresh");
    }

    #[test]
    fn normalize_csi_at_col0_persists_across_calls_mid_line_el_split_across_reads_stays_untouched() {
        // reagent P1, PR #2330 (round 2): normalize_csi_overwrites is called
        // once per PTY read in the real reader loop, with at_col0 threaded
        // in by &mut reference across calls -- NOT reset per call. A prior
        // version of this fix declared at_col0 as a local reset to `true`
        // on every call, so a standalone EL split across two PTY reads
        // ("prefix" in one read, "\x1b[2Ksuffix" in the next) would see a
        // freshly-true at_col0 on the second call and wrongly convert the
        // EL to \r, reproducing the exact mid-line truncation bug the first
        // round of this fix addressed -- just moved from a mid-chunk split
        // to a cross-chunk-read split.
        let mut still_on_first_line = true;
        let mut at_col0 = true;

        let mut first_read = b"prefix".to_vec();
        normalize_csi_overwrites(&mut first_read, &mut still_on_first_line, &mut at_col0);
        assert_eq!(first_read, b"prefix");
        assert!(!at_col0, "cursor is mid-line after a read with no trailing \\n/\\r");

        let mut second_read = b"\x1b[2Ksuffix".to_vec();
        normalize_csi_overwrites(&mut second_read, &mut still_on_first_line, &mut at_col0);
        assert!(
            !second_read.contains(&b'\r'),
            "EL split across a PTY-read boundary, still mid-line, must not synthesize a \\r: {second_read:?}"
        );
        assert_eq!(second_read, b"\x1b[2Ksuffix");

        // Simulate the reader loop's actual buffering: both reads accumulate
        // into one pending buffer before collapse_cr/strip_ansi run on it.
        let mut pending = first_read;
        pending.extend_from_slice(&second_read);
        assert_eq!(pending, b"prefix\x1b[2Ksuffix");
    }

    #[test]
    fn normalize_csi_at_col0_persists_across_calls_leading_cr_read_then_bare_el_read_becomes_cr() {
        // Complement of the above: a read ending in \r (cursor genuinely at
        // column 0) followed by a read that STARTS with EL must still
        // convert -- persisted state must correctly stay `true` too, not
        // just correctly go false.
        let mut still_on_first_line = true;
        let mut at_col0 = true;

        let mut first_read = b"frame1\r".to_vec();
        normalize_csi_overwrites(&mut first_read, &mut still_on_first_line, &mut at_col0);
        assert!(at_col0, "trailing \\r puts the cursor back at column 0");

        let mut second_read = b"\x1b[2Kframe2".to_vec();
        normalize_csi_overwrites(&mut second_read, &mut still_on_first_line, &mut at_col0);
        assert_eq!(second_read, b"\rframe2");
    }

    #[test]
    fn normalize_csi_el_immediately_after_real_cr_still_becomes_cr() {
        // The dominant real-world idiom (\r\x1b[2K<text>): the \r already
        // puts the cursor at column 0, so a following EL is genuinely
        // redundant and safe to also represent as \r.
        assert_eq!(csi(b"\r\x1b[2Kfresh"), b"\r\rfresh");
    }

    #[test]
    fn normalize_csi_cup_home_becomes_cr_on_first_line() {
        // Windows ConPTY re-serializes a literal \r as CUP-to-home rather
        // than passing it through -- verified empirically via the e2e test
        // below. All the omitted-field variants mean row 1, col 1.
        assert_eq!(csi(b"progress\x1b[Hdone"), b"progress\rdone");
        assert_eq!(csi(b"progress\x1b[1Hdone"), b"progress\rdone");
        assert_eq!(csi(b"progress\x1b[1;1Hdone"), b"progress\rdone");
        assert_eq!(csi(b"progress\x1b[;1Hdone"), b"progress\rdone");
        assert_eq!(csi(b"progress\x1b[1;Hdone"), b"progress\rdone");
    }

    #[test]
    fn normalize_csi_cup_other_position_left_untouched() {
        // Row/col other than 1,1 isn't "start of line" and must survive
        // unmangled.
        let out = csi(b"progress\x1b[2;1Hdone");
        assert!(!out.contains(&b'\r'));
        assert_eq!(out, b"progress\x1b[2;1Hdone");
    }

    #[test]
    fn normalize_csi_cup_home_ignored_once_output_has_scrolled_past_first_line() {
        // Once real multi-line output exists, \x1b[H means "go back to the
        // FIRST of several lines" (a multi-line redraw, spec §A3,
        // deliberately out of scope) -- must NOT be treated as "restart
        // the current line", which would corrupt the multi-line content.
        let mut v = b"line1\nline2\n\x1b[Hline1-updated".to_vec();
        let mut still_on_first_line = true;
        let mut at_col0 = true;
        normalize_csi_overwrites(&mut v, &mut still_on_first_line, &mut at_col0);
        assert!(!still_on_first_line, "must flip false after the first \\n");
        assert!(
            !v.contains(&b'\r'),
            "CUP-home after multi-line output must not synthesize a \\r: {:?}",
            String::from_utf8_lossy(&v)
        );
        assert_eq!(v, b"line1\nline2\n\x1b[Hline1-updated");
    }

    #[test]
    fn normalize_csi_cub_left_untouched() {
        // CUB (cursor-back N) is documented out-of-scope for this pass
        // (spec §A2) -- needs true per-column tracking. Must survive
        // unmangled here so strip_ansi still strips it as a whole sequence
        // afterward (not left as garbage).
        let out = csi(b"progress\x1b[5Ddone");
        assert!(!out.contains(&b'\r'));
        assert_eq!(out, b"progress\x1b[5Ddone");
    }

    #[test]
    fn normalize_csi_dominant_cr_plus_el_idiom_is_redundant_but_correct() {
        // The overwhelmingly common real-world idiom: \r\x1b[2K<text>. The
        // \r is already present; normalize_csi_overwrites additionally
        // turning \x1b[2K into a second \r is harmless -- collapse_cr
        // treats consecutive \r as idempotent resets to the same line
        // start.
        let normalized = csi(b"\r\x1b[2Knew frame");
        let mut pending = normalized;
        collapse_cr(&mut pending);
        assert_eq!(pending, b"\rnew frame");
    }

    #[test]
    fn normalize_csi_composes_with_collapse_cr_for_a_full_overwrite() {
        // End-to-end of the two functions together, as the reader loop
        // actually calls them: a progress line first written plainly, then
        // rewritten via CHA(1) on the next PTY read, must collapse exactly
        // like a \r-based rewrite would.
        let mut pending = b"Downloading 10%".to_vec();
        collapse_cr(&mut pending); // no-op, no \r yet

        let mut next_chunk = b"\x1b[1GDownloading 45%".to_vec();
        let mut still_on_first_line = true;
        let mut at_col0 = true;
        normalize_csi_overwrites(&mut next_chunk, &mut still_on_first_line, &mut at_col0);
        pending.extend_from_slice(&next_chunk);
        collapse_cr(&mut pending);

        assert_eq!(pending, b"Downloading 45%");
    }

    /// Verify that the quiet-window semantics are: hold when pending ends
    /// with \r; flush when it doesn't. These are unit tests of the policy
    /// (not of the reader loop directly) — they encode the contract so a
    /// future refactor can't accidentally revert to pop-and-flush.
    #[test]
    fn quiet_window_holds_when_trailing_cr() {
        // A spinner frame "⠋ Loading...\r" should NOT be flushed by the
        // quiet-window — the \r signals that the next frame will overwrite.
        let pending = b"Loading...\r".to_vec();
        assert!(
            pending.last() == Some(&b'\r'),
            "quiet-window must hold pending ending with \\r"
        );
        // Simulate what collapse_cr does when the next frame arrives.
        let mut combined = pending.clone();
        combined.extend_from_slice(b"Done!\n");
        collapse_cr(&mut combined);
        assert_eq!(combined, b"Done!\n");
    }

    #[test]
    fn quiet_window_defers_once_when_no_leading_or_trailing_cr() {
        // "Building..." (no \r at either end) is a candidate for A1's
        // one-tick speculative defer, not an immediate flush — see
        // docs/specs/SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md §A1.
        let pending = b"Building...".to_vec();
        assert!(
            pending.first() != Some(&b'\r') && pending.last() != Some(&b'\r'),
            "quiet-window must defer (not flush) pending with no leading/trailing \\r"
        );
    }

    /// A1: the exact real-world bug this closes — a static label is
    /// printed once with no `\r`, THEN (after the label has already been
    /// speculatively deferred) a `\r`-prefixed overwrite arrives. Before
    /// A1, the label would already have been flushed as its own permanent
    /// LineEvent by the time the overwrite showed up, producing two lines
    /// instead of one settled line.
    #[test]
    fn a1_deferred_line_merges_with_a_following_cr_prefixed_overwrite() {
        // What deferred_pending holds after tick 1 (no \r either end).
        let deferred = b"Installing deps...".to_vec();
        // What arrives on the next read: a \r-prefixed overwrite.
        let next_chunk = b"\rInstalling deps... done\n".to_vec();
        // Mirrors the reader's merge step: prepend deferred, let
        // collapse_cr do the rest.
        let mut combined = deferred;
        combined.extend_from_slice(&next_chunk);
        collapse_cr(&mut combined);
        assert_eq!(
            combined, b"Installing deps... done\n",
            "deferred label + \\r-overwrite must collapse to ONE line, not two"
        );
    }

    /// A1: the deferred line must NOT be merged with unrelated content
    /// that arrives without a leading `\r` — that's two genuinely
    /// different lines and must stay two LineEvents.
    #[test]
    fn a1_deferred_line_is_not_merged_with_unrelated_non_cr_content() {
        let deferred = b"Installing deps...".to_vec();
        let next_chunk = b"Running tests...\n".to_vec();
        // Mirrors the reader's policy: no leading \r on the new chunk means
        // flush `deferred` as its own event, then start fresh with the new
        // chunk (never concatenate the two).
        assert_ne!(next_chunk.first(), Some(&b'\r'));
        let mut fresh = Vec::new();
        fresh.extend_from_slice(&next_chunk);
        collapse_cr(&mut fresh);
        assert_eq!(fresh, b"Running tests...\n");
        // The deferred line, flushed separately, is untouched by the above.
        assert_eq!(deferred, b"Installing deps...");
    }

    #[test]
    fn eof_strips_trailing_cr() {
        // At EOF, a dangling \r (stream ended mid-spinner or mid-CRLF) is
        // stripped before the final flush so the rendered chunk is clean.
        let mut pending = b"partial\r".to_vec();
        if pending.last() == Some(&b'\r') {
            pending.pop();
        }
        assert_eq!(pending, b"partial");
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

    // ── idle_kill_timeout tests ─────────────────────────────────────────────
    // RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md: these cover the env-var
    // override for the idle-kill timeout. The kill/PTY behavior itself isn't
    // unit-testable without a real PTY (see the retro's verification section
    // for the manual repro this was checked against instead).

    #[test]
    fn idle_kill_timeout_defaults_when_unset() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS").ok();
        unsafe {
            std::env::remove_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS");
        }
        assert_eq!(idle_kill_timeout(), DEFAULT_IDLE_KILL_TIMEOUT);
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", v);
            }
        }
    }

    #[test]
    fn idle_kill_timeout_honors_env_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS").ok();
        unsafe {
            std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "5");
        }
        assert_eq!(idle_kill_timeout(), Duration::from_secs(5));
        unsafe {
            match prev {
                Some(v) => std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", v),
                None => std::env::remove_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS"),
            }
        }
    }

    #[test]
    fn idle_kill_timeout_falls_back_on_unparseable_value() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS").ok();
        unsafe {
            std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "not-a-number");
        }
        assert_eq!(idle_kill_timeout(), DEFAULT_IDLE_KILL_TIMEOUT);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", v),
                None => std::env::remove_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS"),
            }
        }
    }

    /// End-to-end proof that the idle-kill mechanism actually fires AND
    /// reaches the whole process tree, not just the direct PTY child.
    ///
    /// Uses two backgrounded `sleep` grandchildren (`bash -c '{ sleep
    /// <marker>1 & sleep <marker>2 & wait; } </dev/null'`) rather than a
    /// single-process command: killing only the direct `bash` child (what
    /// `ChildKiller::kill()` alone does — see `kill_process_tree`'s doc
    /// comment) would leave these two running as orphans, exactly
    /// reproducing the leak one process removed instead of fixing it
    /// (reagent P1, PR #2156). The marker values are unique per test run
    /// (derived from the current PID) so a leftover orphan from a
    /// *previous* failed run of this same test, or an unrelated `sleep`
    /// elsewhere on a shared dev machine, can't produce a false pass.
    ///
    /// The `sleep <marker>` argument doubles as the zero-PTY-output
    /// condition this exists to catch (a clean, portable stand-in for "any
    /// command silently blocked forever," the same shape as the pager hang
    /// — see docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md).
    /// Without the fix, this test would hang until the outer
    /// `tokio::time::timeout` fires and fails it.
    #[tokio::test]
    async fn run_via_pty_kills_idle_child_and_returns_promptly() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS").ok();
        unsafe {
            std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "1");
        }

        let bash = locate_bash().expect("locate_bash for test — same dependency the whole binary needs");
        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => {
                unsafe {
                    match prev {
                        Some(v) => std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", v),
                        None => std::env::remove_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS"),
                    }
                }
                eprintln!("skipping: PTY unavailable in this environment: {e}");
                return;
            }
        };

        let args = Args {
            tool_id: "test-idle-kill".to_string(),
            b64_cmd: String::new(),
            block_id: None,
        };
        // `Mutex` in this module scope resolves to `std::sync::Mutex`
        // (shadowed for `ENV_LOCK` above) — `run_via_pty` needs the async
        // `tokio::sync::Mutex` its `buffered: Arc<Mutex<Vec<u8>>>` param
        // expects, so spell it out.
        let buffered = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));

        // Unique-per-run, but still a valid numeric duration for `sleep`
        // (GNU coreutils sleep accepts fractional seconds and arbitrarily
        // large values). A non-numeric marker string would make `sleep`
        // error out instantly instead of blocking — caught by this test's
        // own first run failing with exit code 0 (~0.1s elapsed) instead
        // of the expected kill.
        //
        // The `100` tag (vs. `bashwrap_binary_idle_kill_cleans_up_full_process_tree`'s
        // `300`) is NOT cosmetic: `std::process::id()` is the SAME across
        // every test in this binary (they're threads in one process, not
        // separate processes), and Rust's test harness runs tests in
        // parallel by default — so a bare `pid.to_string()` marker would
        // let this test's sleep processes get miscounted as "survivors" by
        // that other test's WMI substring search (and vice versa) if both
        // happen to overlap in time. Found this the hard way: the other
        // test failed with "3 survivors" once this test's marker collided
        // with its own. The tag makes the two tests' marker strings
        // non-overlapping substrings of each other.
        let pid = std::process::id();
        let marker = format!("{pid}100");
        let command = format!("sleep {marker}.001 & sleep {marker}.002 & wait");

        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            run_via_pty(&args, &command, None, buffered.clone(), &bash, pair),
        )
        .await
        .expect("run_via_pty must not hang past the outer test timeout — idle-kill should have fired well before this")
        .expect("run_via_pty should return Ok even when it had to kill the child");

        // The exact code is platform/kill-mechanism-specific (portable-pty's
        // Windows ChildKiller calls TerminateProcess with code 127, but the
        // wrapping `bash -c` process's own reported code after being killed
        // is what we actually observe here, not necessarily 127 — verified
        // empirically as 1 on this Windows dev machine). `124` is only the
        // fallback sentinel used if the wait task doesn't resolve within
        // the post-kill grace period at all. What actually matters: it's
        // never the clean-success `0`, and (checked below) it returns
        // promptly instead of hanging.
        assert_ne!(
            result, 0,
            "an idle-killed child must not report a clean success exit code"
        );
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "should return well within the 1s idle timeout + grace periods, not the 20s outer bound — got {:?}",
            start.elapsed()
        );

        let blob = buffered.lock().await;
        assert!(
            String::from_utf8_lossy(&blob).contains("terminated automatically"),
            "model-visible blob should explain why the command was cut short, got: {:?}",
            String::from_utf8_lossy(&blob)
        );
        drop(blob);

        // NOTE on what this test does NOT prove: calling `run_via_pty`
        // in-process (as a library function inside `cargo test`'s own
        // process) returning does not, by itself, guarantee every
        // descendant is gone yet — see
        // `bashwrap_binary_idle_kill_cleans_up_full_process_tree` below for
        // why, and for the test that actually proves full-tree cleanup.
        let _ = &marker; // used by the binary-level test below, not here

        unsafe {
            match prev {
                Some(v) => std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", v),
                None => std::env::remove_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS"),
            }
        }
    }

    /// Reagent P1 follow-up, PR #2156: `idle_rx` resolves (with `Err`) not
    /// only when `pty_reader_loop` really signals an idle timeout, but also
    /// whenever it returns normally (EOF on a fast, successful command) and
    /// drops the still-unused `idle_tx`. The original `tokio::select! { _ =
    /// idle_rx => {...} }` treated BOTH cases identically, so a fast
    /// command completing could race `wait_task` and get spuriously
    /// killed + tagged "terminated automatically" even though it ran to
    /// completion normally. Fixed by branching on `idle_signal.is_err()`
    /// inside that arm instead of ignoring the value.
    ///
    /// A single run of a fast command isn't a reliable regression test for
    /// a race in general, so this runs `echo` 30 times with a very short
    /// idle timeout to widen the chance of catching it.
    ///
    /// Honesty check performed while writing this test: temporarily forced
    /// the OLD (buggy) unconditional-kill behavior back in and re-ran this
    /// exact test — it still passed 30/30. On this machine, `wait_task`
    /// (`child.wait()` resolving) appears to reliably win the race against
    /// `pty_reader_loop`'s EOF-then-drop for a trivially fast command like
    /// `echo hello`, so this test does NOT reliably fail on the unfixed
    /// code and can't be trusted as sole proof the bug is gone. ReAgent's
    /// code-level analysis of the race is still correct — a oneshot
    /// receiver genuinely does resolve identically for "sent" and
    /// "dropped without sending," and the fix (branching on
    /// `idle_signal.is_err()`) is the structurally correct response
    /// regardless of whether this specific test can force the window open.
    /// The scenario ReAgent named as the realistic trigger — blocking
    /// thread-pool contention from concurrent bashwrap invocations —
    /// wasn't reproduced here; doing so reliably would need deliberately
    /// saturating tokio's blocking pool, not attempted given time spent on
    /// this investigation already. This test is kept as basic coverage of
    /// the fast-success path (asserts real output, exit 0, no spurious
    /// diagnostic) — a real regression guard for *some* bugs in this area,
    /// just not proven to be one for this exact race.
    #[tokio::test]
    async fn run_via_pty_does_not_misclassify_fast_success_as_idle_timeout() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS").ok();
        unsafe {
            std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "1");
        }

        let bash = locate_bash().expect("locate_bash for test — same dependency the whole binary needs");

        for i in 0..30 {
            let pty_system = native_pty_system();
            let pair = match pty_system.openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("skipping: PTY unavailable in this environment: {e}");
                    break;
                }
            };
            let args = Args {
                tool_id: format!("test-fast-success-{i}"),
                b64_cmd: String::new(),
                block_id: None,
            };
            let buffered = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));

            let result = tokio::time::timeout(
                Duration::from_secs(10),
                run_via_pty(&args, "echo hello", None, buffered.clone(), &bash, pair),
            )
            .await
            .unwrap_or_else(|_| panic!("iteration {i}: run_via_pty must not hang on a trivial fast command"))
            .unwrap_or_else(|e| panic!("iteration {i}: run_via_pty errored: {e}"));

            assert_eq!(
                result, 0,
                "iteration {i}: a fast, genuinely successful `echo` must report clean exit 0, \
                 not get spuriously classified as idle-killed by the idle_rx/wait_task race"
            );
            let blob = String::from_utf8_lossy(&buffered.lock().await).into_owned();
            assert!(
                !blob.contains("terminated automatically"),
                "iteration {i}: fast successful command must not carry the idle-kill \
                 diagnostic — got blob: {blob:?}"
            );
            assert!(
                blob.contains("hello"),
                "iteration {i}: expected the command's real output in the blob, got: {blob:?}"
            );
        }

        unsafe {
            match prev {
                Some(v) => std::env::set_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", v),
                None => std::env::remove_var("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS"),
            }
        }
    }

    /// End-to-end proof of A1 against a REAL PTY + bash, reproducing the
    /// exact bug SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md
    /// closes: a static label printed with no `\r`, a pause past the
    /// quiet-window, then a `\r`-prefixed overwrite. Before A1, the label
    /// was already flushed as its own permanent LineEvent by the time the
    /// overwrite arrived, so the model-visible blob contained the label
    /// TWICE ("Installing deps..." followed by "Installing deps... done").
    ///
    /// The pause is deliberately tuned to land INSIDE A1's one-extra-tick
    /// window: greater than one `FLUSH_QUIET_WINDOW` (50ms, so the label is
    /// actually deferred rather than merged inline on the same read) but
    /// less than two (100ms, the point at which A1 gives up waiting and
    /// flushes the deferred line anyway per the spec's own "no added
    /// latency beyond one extra tick" scope). 70ms centers comfortably
    /// inside that ~50ms window in isolation, but real OS thread scheduling
    /// still occasionally pushes either boundary under load (observed: this
    /// test flaked ~1 run in 6 in the full-suite parallel run, though never
    /// in isolation) — `FLUSH_QUIET_WINDOW` is a hardcoded const with no
    /// test-only override, so retrying the wall-clock scenario a few times
    /// (same "widen the chance" philosophy already used by
    /// `run_via_pty_does_not_misclassify_fast_success_as_idle_timeout`
    /// above, just applied to reliably demonstrate a real behavior instead
    /// of reliably catching a rare regression) is more honest than either a
    /// single flaky assertion or silently loosening what's being proven.
    #[tokio::test]
    async fn a1_e2e_static_label_then_delayed_cr_overwrite_collapses_to_one_line() {
        let bash = locate_bash().expect("locate_bash for test — same dependency the whole binary needs");
        let command = "printf 'Installing deps...'; sleep 0.07; printf '\\rInstalling deps... done\\n'";

        const MAX_ATTEMPTS: u32 = 5;
        let mut last_blob = String::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let pty_system = native_pty_system();
            let pair = match pty_system.openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("skipping: PTY unavailable in this environment: {e}");
                    return;
                }
            };
            let args = Args {
                tool_id: format!("test-a1-e2e-{attempt}"),
                b64_cmd: String::new(),
                block_id: None,
            };
            let buffered = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));

            let result = tokio::time::timeout(
                Duration::from_secs(15),
                run_via_pty(&args, command, None, buffered.clone(), &bash, pair),
            )
            .await
            .expect("run_via_pty must not hang")
            .expect("run_via_pty should return Ok");
            assert_eq!(result, 0, "command should exit cleanly");

            let blob = String::from_utf8_lossy(&buffered.lock().await).into_owned();
            let occurrences = blob.matches("Installing deps").count();
            if occurrences == 1 && blob.contains("Installing deps... done") {
                return; // demonstrated: the collapse happened as designed
            }
            eprintln!(
                "attempt {attempt}/{MAX_ATTEMPTS}: expected 1 occurrence + the settled frame, \
                 got {occurrences} occurrence(s) — blob: {blob:?} (retrying: real OS scheduling \
                 can occasionally push the 70ms pause outside A1's ~50-100ms defer window)"
            );
            last_blob = blob;
        }
        panic!(
            "the static label + its delayed \\r-overwrite must collapse to ONE occurrence across \
             {MAX_ATTEMPTS} attempts — last blob: {last_blob:?}"
        );
    }
}
