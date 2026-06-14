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
    cmd.arg(format!("{{\n{}\n}} </dev/null", command));

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
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("PTY spawn of bash at {}", bash.display()))?;

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
    tokio::task::spawn_blocking(move || {
        pty_reader_loop(reader, tx_reader);
    });
    drop(tx);

    let publisher_handle = spawn_publisher_loop(args, wps.cloned(), buffered.clone(), rx);

    // Move pair AND writer into the wait task — both must outlive
    // child.wait() to satisfy the ConPTY lifetime contract on Windows.
    let exit_code = tokio::task::spawn_blocking(move || -> Result<i32> {
        let mut child = child;
        let status = child.wait().context("PTY child wait")?;
        let code = status.exit_code() as i32;
        tracing::info!(target: "bashwrap", exit_code = code, "PTY child exited");
        drop(writer);
        drop(pair);
        Ok(code)
    })
    .await
    .context("PTY wait task join")??;

    // `buffered` (read by the caller for the model blob) is populated
    // only by the publisher loop, so drain it before returning.
    let _ = publisher_handle.await;
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
    let mut child = Command::new(bash)
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("pipe spawn of bash at {}", bash.display()))?;

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

    let (tx, rx) = mpsc::channel::<LineEvent>(1024);
    tokio::spawn(stream_reader(stdout, "stdout", tx.clone()));
    tokio::spawn(stream_reader(stderr, "stderr", tx.clone()));
    drop(tx);

    let publisher_handle = spawn_publisher_loop(args, wps.cloned(), buffered.clone(), rx);

    let exit_status = child.wait().await.context("waiting for bash child")?;
    let _ = publisher_handle.await;
    Ok(exit_status.code().unwrap_or(-1))
}

/// PTY reader loop: drains bytes from the master, strips DSR / ANSI
/// control sequences, and forwards line-split chunks to the publisher.
fn pty_reader_loop(
    mut reader: Box<dyn std::io::Read + Send>,
    tx: mpsc::Sender<LineEvent>,
) {
    use std::io::Read;
    // PTY collapses stdout + stderr onto one stream — the slave's
    // terminal device is a single FD shared by both. Chunks are
    // labelled "stdout" because there is no way to recover the
    // original FD distinction from the master read. The pipe path
    // (`run_via_pipes`) preserves the split.
    let kind: &'static str = "stdout";
    let mut pending: Vec<u8> = Vec::with_capacity(8192);
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                if !pending.is_empty() {
                    let _ = tx.blocking_send(LineEvent {
                        kind,
                        bytes: std::mem::take(&mut pending),
                    });
                }
                return;
            }
            Ok(n) => {
                let raw_n = n;
                let mut chunk = buf[..n].to_vec();
                strip_dsr(&mut chunk);
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
                pending.extend_from_slice(&chunk);
                while let Some(nl_pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=nl_pos).collect();
                    if tx.blocking_send(LineEvent { kind, bytes: line }).is_err() {
                        return;
                    }
                }
                // Flush any remaining partial line at the end of each
                // read so progress prompts like `printf 'starting...'`
                // surface live without waiting for a newline.
                if !pending.is_empty() {
                    if tx
                        .blocking_send(LineEvent {
                            kind,
                            bytes: std::mem::take(&mut pending),
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(target: "bashwrap", error = %e, "PTY read error");
                return;
            }
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
///    newline), `\x07` (bell), `\x08` (backspace). We strip `\r`
///    when it immediately precedes `\n` (CRLF → LF for the chunk
///    list); other `\r` we keep as-is for now since simple progress
///    bars use it.
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
            // CRLF → LF (matches what bash sends after every line on
            // a PTY; the chunk list expects LF only).
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    *chunk = out;
}

/// Shared publisher loop — drains the LineEvent channel, aggregates
/// into the model-visible buffer, and publishes each line via WPS.
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
        while let Some(event) = rx.recv().await {
            {
                let mut buf = buffered.lock().await;
                if event.kind == "stderr" {
                    buf.extend_from_slice(b"[stderr] ");
                }
                buf.extend_from_slice(&event.bytes);
            }
            if let Some(client) = wps.as_ref() {
                let mut line_bytes: &[u8] = &event.bytes;
                if line_bytes.last() == Some(&b'\n') {
                    line_bytes = &line_bytes[..line_bytes.len() - 1];
                }
                let line_str = String::from_utf8_lossy(line_bytes);
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
                        tracing::warn!(target: "bashwrap", tool_id = %tool_id, error = %e, "WPS publish failed");
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
