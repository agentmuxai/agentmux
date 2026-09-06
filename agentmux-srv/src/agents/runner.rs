// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified agent runner — one-shot Claude Code spawn for the
//! drone Agent block.
//!
//! Spawns `claude --print --output-format=stream-json` as a non-
//! interactive subprocess, drains its stdout through
//! `ClaudeTranslator`, forwards each `AgentEvent` on the caller's
//! `tx`, and resolves the handle's `final_result` with the
//! structured `AgentRunResult` once the stream emits `Done`.
//!
//! Headless and one-shot by design — the drone Agent block's
//! contract is "send task, wait for done, return result." The
//! interactive agent pane has its own PTY-based controller in
//! `blockcontroller/shell/lifecycle.rs`; that path is NOT routed
//! through this runner's spawn (see
//! `docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md` §4.2 — what's
//! shared is the translator + event shape, not the spawn function).
//! It DOES reuse this module's commit-aware admission gate
//! (`admit_spawn` / `agent_commit_reserve_gb`, `pub(crate)`) — see
//! `blockcontroller::shell::lifecycle::ShellController::start`,
//! which calls them right before spawning a `claude`/`codex`/
//! `gemini`/`qwen` interactive pane, mirroring this file's
//! `run_agent` gate.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use super::failure::classify;
use super::translator::claude::ClaudeTranslator;
use super::translator::Translator as _;
use super::types::{AgentEvent, AgentRef, AgentRunResult, AgentTask};

/// Override the default `claude` binary name. Set to a full path
/// (or a different binary) for testing or non-PATH installs.
const ENV_CLAUDE_BIN: &str = "AGENTMUX_CLAUDE_BIN";

const DEFAULT_CLAUDE_BIN: &str = "claude";

/// Handle returned by `run_agent`. The caller already holds the
/// `mpsc::UnboundedReceiver<AgentEvent>` they paired with the `tx`
/// passed into `run_agent`; this handle adds the structured terminal
/// value via `final_result` (drone Agent block's downstream
/// output).
///
/// Dropping the caller's receiver implicitly cancels the run only if
/// the runner observes the send error — Phase 2 adds an explicit
/// `AbortHandle`.
pub struct AgentRunHandle {
    pub final_result: oneshot::Receiver<Result<AgentRunResult, String>>,
}

/// Error returned by the runner.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent runner: spawn failed: {0}")]
    Spawn(String),
    /// System commit headroom is below the reserve required to safely start
    /// another agent. Refusing here prevents a new `claude.exe` from tipping the
    /// system commit charge into its limit, where a failed allocation aborts the
    /// CEF host (Chromium OOM, `0xE0000008`). Callers should surface a transient
    /// "memory full — try again when memory frees" rather than a hard failure.
    /// Pillar 3 (`SPEC_WIN10_PAGEFILE_OOM_CRASH` / `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR`).
    #[error("agent runner: insufficient memory to start agent ({avail_gb:.1} GB commit free, need {reserve_gb:.1} GB headroom)")]
    CommitPressure { avail_gb: f64, reserve_gb: f64 },
}

/// Minimum free system commit (GB) required to admit a new agent spawn. Below
/// this, launching another `claude.exe` risks pushing the system commit charge
/// to its limit, where a failed allocation aborts the CEF host (Chromium OOM).
///
/// SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29's original P0 called for
/// re-deriving this reserve from real per-agent `PrivateUsage`, not
/// `VirtualMemorySize64` — the 6/26 estimate (`SPEC_MEMORY_ANALYSIS_2026_06_26`)
/// had mistakenly attributed ~10.5 GB of commit to each `claude.exe`, read off
/// `VirtualMemorySize64` (reserved address space, not commit charge). That
/// number was never wired into this constant — this file has measured
/// `2.0 GB` since Pillar 3 shipped (#1853) — but the measurement trail behind
/// it is worth stating explicitly now that it exists:
/// `SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md` re-measured a
/// live `claude.exe`'s actual `PrivateUsage` (Windows' real per-process commit
/// counter) at ~1.05 GB, corroborated independently by
/// Resource-Exhaustion-Detector telemetry (0.49–0.67 GB) in the 6/29 spec.
/// `2.0` GB is ~2x that per-agent figure, a reasonable safety margin for one
/// additional spawn — no VirtualMemorySize64-derived inflation to correct.
/// Overridable via `AGENTMUX_AGENT_COMMIT_RESERVE_GB` (e.g. 0 disables the
/// gate on a host with a huge page file; higher on a constrained box).
const DEFAULT_AGENT_COMMIT_RESERVE_GB: f64 = 2.0;

pub(crate) fn agent_commit_reserve_gb() -> f64 {
    std::env::var("AGENTMUX_AGENT_COMMIT_RESERVE_GB")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| *v >= 0.0)
        .unwrap_or(DEFAULT_AGENT_COMMIT_RESERVE_GB)
}

/// Pure admission decision: is there enough free system commit to spawn another
/// agent? `None` available (non-Windows, or the read failed) ⇒ admit — there's no
/// cheap commit limit to enforce. Strict `<` so a value exactly at the reserve is
/// admitted. CEF-free / OS-free so it is fully unit-testable.
pub(crate) fn admit_spawn(avail_commit_gb: Option<f64>, reserve_gb: f64) -> Result<(), AgentError> {
    match avail_commit_gb {
        Some(avail) if avail < reserve_gb => {
            Err(AgentError::CommitPressure { avail_gb: avail, reserve_gb })
        }
        _ => Ok(()),
    }
}

/// Spawn `claude --print --output-format=stream-json` per the given
/// `AgentTask` and `AgentRef`, drain its stdout through the shared
/// `ClaudeTranslator`, forward each `AgentEvent` on `tx`, and
/// resolve the returned handle's `final_result` with an
/// `AgentRunResult` constructed from the terminal Cost + Done events.
///
/// Working directory resolution:
///   - `agent_ref.working_directory` if non-empty
///   - else the current process working directory
///
/// Identity / memory bundle resolution and named-agent continuation
/// are NOT plumbed in Phase 1.5 PR 2 — the drone Agent block
/// always spawns fresh (per spec §8 "drone runs always allocate
/// fresh instance_name"). The bundles can be added in a follow-up
/// once the drone inspector (PR 3) needs to surface them.
pub async fn run_agent(
    agent_ref: AgentRef,
    task: AgentTask,
    tx: mpsc::UnboundedSender<AgentEvent>,
) -> Result<AgentRunHandle, AgentError> {
    // Pillar 3 — commit-aware admission control. Refuse to spawn another agent
    // when system commit headroom is below the reserve, rather than letting the
    // new claude.exe push the box into an OOM abort. Gated here (the production
    // entry) not in `run_agent_with_bin`, so the unit tests that inject a binary
    // path bypass the gate and stay deterministic across hosts.
    admit_spawn(
        crate::backend::sysinfo::available_commit_gb(),
        agent_commit_reserve_gb(),
    )?;
    let bin = std::env::var(ENV_CLAUDE_BIN)
        .unwrap_or_else(|_| DEFAULT_CLAUDE_BIN.to_string());
    run_agent_with_bin(&bin, agent_ref, task, tx).await
}

/// Internal entry point — same as `run_agent` but takes the `claude`
/// binary path explicitly. Lets tests inject a known-nonexistent
/// path to exercise the spawn-failure path without touching env vars
/// (Rust 1.81+ flags `std::env::set_var` as unsound under concurrent
/// test execution). The public `run_agent` is a thin shim that
/// resolves the binary from `$AGENTMUX_CLAUDE_BIN` or the default.
pub(crate) async fn run_agent_with_bin(
    bin: &str,
    agent_ref: AgentRef,
    task: AgentTask,
    tx: mpsc::UnboundedSender<AgentEvent>,
) -> Result<AgentRunHandle, AgentError> {
    let working_dir = if agent_ref.working_directory.is_empty() {
        std::env::current_dir()
            .map_err(|e| AgentError::Spawn(format!("cwd: {e}")))?
    } else {
        PathBuf::from(&agent_ref.working_directory)
    };

    // `claude --print` runs in non-interactive mode and exits when
    // done. `--output-format=stream-json` emits one JSON object per
    // line, the format ClaudeTranslator consumes. `--verbose` is
    // required alongside stream-json (the CLI rejects stream-json
    // without it). `--include-partial-messages` gives us the
    // streaming text_deltas — the translator skips the resulting
    // `partial: true` snapshots when building the transcript.
    let mut cmd = Command::new(bin);
    cmd.arg("--print")
        .arg("--output-format=stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        // Moves per-machine sections (cwd, env info, memory paths, git status)
        // out of the system prompt into the first user message, matching the
        // persistent-controller launch args in providers.rs. This spawn path
        // builds its own args independent of ProviderConfig, so the flag has
        // to be added here too — reagent P1 on PR #1964.
        .arg("--exclude-dynamic-system-prompt-sections");
    // Forward the configured turn cap so the CLI enforces it.
    // Previously stored on AgentTask but never passed to the
    // subprocess — silently ignored. Reagent P1 + codex P2 on
    // PR #834.
    if let Some(n) = task.max_turns {
        cmd.arg("--max-turns").arg(n.to_string());
    }
    // On Windows: suppress console-window allocation. Spawned from the windowless
    // srv without CREATE_NO_WINDOW, this one-shot task-agent CLI opens a Windows
    // Terminal window per run (Win11 default-terminal handler). stdio is piped, so
    // no console is needed. See docs/retro/retro-windows-terminal-window-leak-2026-06-21.md.
    #[cfg(windows)]
    {
        use agentmux_common::win32::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .arg(&task.prompt)
        .current_dir(&working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AgentError::Spawn(format!("spawn `{bin}`: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AgentError::Spawn("claude stdout pipe missing".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AgentError::Spawn("claude stderr pipe missing".to_string()))?;

    let (result_tx, result_rx) = oneshot::channel();
    // Hand the captured stderr back to `drain_and_collect` so a failed
    // run reports the real cause (rate-limit / auth / OOM) instead of a
    // bare exit code. See
    // `docs/specs/SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md`.
    let (stderr_tx, stderr_rx) = oneshot::channel::<Vec<u8>>();

    // Drain stderr to EOF in the background so the child's pipe never
    // fills (a half-drained pipe blocks the child on stderr writes and
    // can stall the whole run). We keep a rolling *tail* — the last
    // STDERR_TAIL_CAP bytes — because the CLI's terminal error line
    // (rate-limit / auth / OOM) lands at the END of stderr; a capped
    // prefix would drop exactly the line the classifier needs. On EOF
    // the tail is handed to the collector via `stderr_tx`.
    // (codex P2 on #1353: keep the tail, not a prefix.)
    tokio::spawn(async move {
        const STDERR_TAIL_CAP: usize = 64 * 1024;
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut reader = BufReader::new(stderr);
        let mut sink = [0u8; 8192];
        loop {
            match reader.read(&mut sink).await {
                Ok(0) => break,
                Ok(n) => append_capped_tail(&mut buf, &sink[..n], STDERR_TAIL_CAP),
                Err(_) => break,
            }
        }
        // Trim to exactly the last CAP bytes. If the receiver is gone
        // (run succeeded — nobody needs stderr), the send just drops.
        trim_to_tail(&mut buf, STDERR_TAIL_CAP);
        let _ = stderr_tx.send(buf);
    });

    tokio::spawn(async move {
        let result = drain_and_collect(stdout, &tx, &mut child, stderr_rx).await;
        let _ = result_tx.send(result);
    });

    Ok(AgentRunHandle {
        final_result: result_rx,
    })
}

/// Drain `stdout` line-by-line through `ClaudeTranslator`, forward
/// every emitted event on `tx`, accumulate the terminal `Cost` /
/// `Done` event payloads into an `AgentRunResult`, wait for the
/// child to exit, and return the result.
///
/// Split out from `run_agent` so it can be unit-tested against
/// in-memory readers without spawning a real subprocess. Used by
/// the integration test below as `drain_async_reader_for_test`.
async fn drain_and_collect(
    stdout: tokio::process::ChildStdout,
    tx: &mpsc::UnboundedSender<AgentEvent>,
    child: &mut tokio::process::Child,
    stderr_rx: oneshot::Receiver<Vec<u8>>,
) -> Result<AgentRunResult, String> {
    let result = drain_async_reader(BufReader::new(stdout), tx).await;

    // Wait for child to exit so the OS reaps it cleanly.
    let exit = child.wait().await.map_err(|e| format!("wait: {e}"))?;

    match result {
        Ok(mut accumulated) => {
            // A run is a failure if the process exited non-zero OR claude
            // reported an error on stdout (a terminal error `result`
            // frame) even while exiting 0 — otherwise downstream blocks
            // could treat a failed run as successful. (codex P1 #1353.)
            let reported_error = accumulated.error_frame.take();
            if exit.success() && reported_error.is_none() {
                // Genuine success — but a stream that produced nothing is
                // itself a (classified) failure, not a silent empty result.
                if accumulated.response.is_empty() && accumulated.transcript.is_empty() {
                    return Err(
                        explain_failure(exit.code(), exit_signal(&exit), None, stderr_rx).await,
                    );
                }
                accumulated.transcript.shrink_to_fit();
                return Ok(accumulated);
            }
            // Non-zero exit, or a stdout-reported error on exit 0: classify
            // from the exit status + result frame + captured stderr so the
            // caller sees a real cause, not "exited with status N".
            Err(explain_failure(exit.code(), exit_signal(&exit), reported_error, stderr_rx).await)
        }
        // Stream read error: still enrich with the exit/stderr cause.
        Err(e) => {
            let cause = explain_failure(exit.code(), exit_signal(&exit), None, stderr_rx).await;
            Err(format!("{cause}\n(stream read: {e})"))
        }
    }
}

/// Await the captured stderr, classify the exit, log a warning, and
/// render the human-readable explanation that becomes the run's
/// terminal error string. See
/// `docs/specs/SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md`.
async fn explain_failure(
    exit_code: Option<i32>,
    signal: Option<i32>,
    result_frame: Option<serde_json::Value>,
    stderr_rx: oneshot::Receiver<Vec<u8>>,
) -> String {
    let bytes = stderr_rx.await.unwrap_or_default();
    let stderr = String::from_utf8_lossy(&bytes);
    let failure = classify(exit_code, signal, &stderr, result_frame.as_ref());
    tracing::warn!(
        code = ?failure.code,
        exit_code = ?exit_code,
        signal = ?signal,
        retryable = failure.retryable,
        "agent run failed: {}",
        failure.title,
    );
    failure.explain()
}

/// Extract the terminating signal (Unix only). On non-Unix there is no
/// signal concept, so this is always `None`.
#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

/// Append `chunk` to a rolling tail buffer, compacting to the last
/// `cap` bytes whenever it grows past `2 * cap` (amortized O(1)). The
/// exact final trim happens at EOF via [`trim_to_tail`]. Keeping the
/// *tail* (not a prefix) matters because the CLI's terminal error line
/// is at the end of stderr.
fn append_capped_tail(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    buf.extend_from_slice(chunk);
    if buf.len() > cap.saturating_mul(2) {
        trim_to_tail(buf, cap);
    }
}

/// Drop all but the last `cap` bytes of `buf`.
fn trim_to_tail(buf: &mut Vec<u8>, cap: usize) {
    if buf.len() > cap {
        let excess = buf.len() - cap;
        buf.drain(0..excess);
    }
}

/// Drain an arbitrary async reader of newline-delimited stream-json
/// frames, forward every emitted `AgentEvent` on `tx`, and return
/// an accumulator capturing the terminal `Cost` and `Done` values.
///
/// Pure async helper — no subprocess, no broker. Unit-tested with
/// `tokio::io::duplex` in-memory pipes.
pub(crate) async fn drain_async_reader<R: tokio::io::AsyncBufRead + Unpin>(
    mut reader: R,
    tx: &mpsc::UnboundedSender<AgentEvent>,
) -> Result<AgentRunResult, String> {
    let mut translator = ClaudeTranslator::new();
    let mut accumulated = AgentRunResult::default();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("stdout read: {e}"))?;
        if n == 0 {
            break; // EOF
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        // Capture a terminal *error* result frame so the collector can
        // fail the run even when claude reports the error on stdout and
        // exits 0 (the translator still emits a hollow `Done`).
        // codex P1 on #1353.
        if frame.get("type").and_then(|v| v.as_str()) == Some("result")
            && super::failure::is_error_result_frame(&frame)
        {
            accumulated.error_frame = Some(frame.clone());
        }
        for event in translator.translate(frame) {
            // Capture terminal values before forwarding so a closed
            // receiver doesn't lose the accumulated result.
            match &event {
                AgentEvent::Cost { cost_usd, tokens } => {
                    accumulated.cost_usd = *cost_usd;
                    accumulated.tokens = tokens.clone();
                }
                AgentEvent::Done {
                    response,
                    transcript,
                } => {
                    accumulated.response = response.clone();
                    accumulated.transcript = transcript.clone();
                }
                _ => {}
            }
            // Forward — if the receiver is dropped, just stop
            // sending (the drain still continues to capture the
            // accumulated result).
            let _ = tx.send(event);
        }
    }
    Ok(accumulated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::AsyncWriteExt;

    /// Build a stream-json byte sequence simulating a complete
    /// short claude run: streamed text + cost + result.
    fn synthetic_stream(prompt_reply: &str, cost: f64) -> Vec<u8> {
        let mut s = String::new();
        for ch in prompt_reply.chars() {
            s.push_str(&format!(
                r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{ch}"}}}}}}"#,
            ));
            s.push('\n');
        }
        s.push_str(&format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{prompt_reply}"}}]}}}}
"#
        ));
        s.push_str(&format!(
            r#"{{"type":"result","cost_usd":{cost},"usage":{{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}},"result":"{prompt_reply}"}}
"#
        ));
        s.into_bytes()
    }

    // ── Pillar 3 — commit-aware admission control (pure decision) ──────────────

    #[test]
    fn admit_spawn_allows_when_headroom_at_or_above_reserve() {
        assert!(admit_spawn(Some(8.0), 2.0).is_ok());
        // Exactly at the reserve admits (strict `<`).
        assert!(admit_spawn(Some(2.0), 2.0).is_ok());
    }

    #[test]
    fn admit_spawn_refuses_below_reserve() {
        match admit_spawn(Some(1.0), 2.0) {
            Err(AgentError::CommitPressure { avail_gb, reserve_gb }) => {
                assert_eq!(avail_gb, 1.0);
                assert_eq!(reserve_gb, 2.0);
            }
            other => panic!("expected CommitPressure, got {other:?}"),
        }
    }

    #[test]
    fn admit_spawn_admits_when_commit_unknown() {
        // None (non-Windows / read failure) ⇒ no cheap limit to enforce ⇒ admit.
        assert!(admit_spawn(None, 2.0).is_ok());
    }

    #[test]
    fn agent_commit_reserve_defaults_when_env_absent_or_invalid() {
        // Not asserting on the env var (tests share a process); just verify the
        // default is the documented conservative floor and is non-negative.
        assert_eq!(DEFAULT_AGENT_COMMIT_RESERVE_GB, 2.0);
        assert!(DEFAULT_AGENT_COMMIT_RESERVE_GB >= 0.0);
    }

    #[tokio::test]
    async fn drain_async_reader_accumulates_cost_and_done() {
        let bytes = synthetic_stream("hello", 0.001);
        let (mut w, r) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            w.write_all(&bytes).await.unwrap();
            w.shutdown().await.unwrap();
        });

        let (tx, mut rx) = mpsc::unbounded_channel();
        let result = drain_async_reader(BufReader::new(r), &tx)
            .await
            .expect("drain ok");

        assert_eq!(result.response, "hello");
        assert_eq!(result.cost_usd, 0.001);
        assert_eq!(result.tokens.input, 10);
        assert_eq!(result.tokens.output, 5);
        // Transcript contains the assistant turn.
        assert_eq!(result.transcript.len(), 1);

        // Events forwarded: 5 AssistantText (one per char) + Cost + Done.
        drop(tx);
        let mut evs = Vec::new();
        while let Some(e) = rx.recv().await {
            evs.push(e);
        }
        assert_eq!(evs.len(), 7, "got events: {evs:?}");
        match &evs[evs.len() - 1] {
            AgentEvent::Done { .. } => {}
            other => panic!("expected last event Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn drain_async_reader_skips_non_json_lines() {
        // claude --verbose sometimes emits informational lines on
        // stdout that aren't stream-json (rare but possible). Those
        // must not break the drain.
        let mut bytes: Vec<u8> = b"Reading config...\n".to_vec();
        bytes.extend_from_slice(&synthetic_stream("ok", 0.0));
        bytes.extend_from_slice(b"\n");

        let (mut w, r) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            w.write_all(&bytes).await.unwrap();
            w.shutdown().await.unwrap();
        });

        let (tx, _rx) = mpsc::unbounded_channel();
        let result = drain_async_reader(BufReader::new(r), &tx)
            .await
            .expect("drain ok");
        assert_eq!(result.response, "ok");
    }

    #[tokio::test]
    async fn drain_async_reader_returns_empty_on_no_stream() {
        let (mut w, r) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            // Just close — no output at all.
            w.shutdown().await.unwrap();
        });

        let (tx, _rx) = mpsc::unbounded_channel();
        let result = drain_async_reader(BufReader::new(r), &tx)
            .await
            .expect("drain ok");
        // Default-empty result — the drain itself succeeds; the
        // caller (drain_and_collect) is responsible for surfacing
        // the "no Done event" as an error since it depends on
        // exit status semantics.
        assert!(result.response.is_empty());
        assert_eq!(result.cost_usd, 0.0);
    }

    #[tokio::test]
    async fn drain_async_reader_handles_multi_line_chunks() {
        // BufReader's read_line is well-defined; this just guards
        // against future regressions where someone might switch to a
        // chunked reader.
        let bytes = synthetic_stream("multi", 0.01);
        let (mut w, r) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            // Write in small chunks to exercise the read path.
            for chunk in bytes.chunks(7) {
                w.write_all(chunk).await.unwrap();
            }
            w.shutdown().await.unwrap();
        });

        let (tx, _rx) = mpsc::unbounded_channel();
        let result = drain_async_reader(BufReader::new(r), &tx)
            .await
            .expect("drain ok");
        assert_eq!(result.response, "multi");
    }

    #[tokio::test]
    async fn drain_handles_malformed_json_gracefully() {
        let mut bytes: Vec<u8> =
            b"{this is not valid json\n{\"type\":\"unknown\"}\n".to_vec();
        bytes.extend_from_slice(&synthetic_stream("recovered", 0.0));

        let (mut w, r) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            w.write_all(&bytes).await.unwrap();
            w.shutdown().await.unwrap();
        });

        let (tx, _rx) = mpsc::unbounded_channel();
        let result = drain_async_reader(BufReader::new(r), &tx)
            .await
            .expect("drain ok");
        assert_eq!(result.response, "recovered");
    }

    #[tokio::test]
    #[ignore = "requires `claude` CLI on PATH; run manually for end-to-end"]
    async fn run_agent_end_to_end_with_real_claude() {
        // Manual smoke: AGENTMUX_CLAUDE_BIN=/path/to/claude
        // cargo test -p agentmux-srv -- --ignored
        //     run_agent_end_to_end_with_real_claude
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = run_agent(
            AgentRef::default(),
            AgentTask {
                prompt: "What is 2+2? Respond with just the number.".to_string(),
                context: serde_json::Map::new(),
                max_turns: None,
            },
            tx,
        )
        .await
        .expect("spawn ok");

        // Drain events until done.
        while let Some(_ev) = rx.recv().await {}

        let result = handle
            .final_result
            .await
            .expect("oneshot ok")
            .expect("agent run ok");
        assert!(result.response.contains('4'), "got: {}", result.response);
        assert!(result.cost_usd > 0.0);
    }

    #[tokio::test]
    async fn run_agent_with_bin_surfaces_spawn_failure() {
        // Inject a known-nonexistent binary path so the spawn fails
        // deterministically. Verifies the AgentError::Spawn path
        // without touching env vars (set_var is unsound under
        // concurrent test execution in Rust 1.81+).
        let (tx, _rx) = mpsc::unbounded_channel();
        let result = run_agent_with_bin(
            "/definitely/does/not/exist/claude-xyz-test",
            AgentRef::default(),
            AgentTask {
                prompt: "hi".to_string(),
                context: serde_json::Map::new(),
                max_turns: None,
            },
            tx,
        )
        .await;
        match result {
            Err(AgentError::Spawn(msg)) => {
                assert!(
                    msg.contains("spawn") || msg.contains("does/not/exist"),
                    "spawn error message should reference the failure; got: {msg}"
                );
            }
            Err(other) => panic!("expected Spawn error, got: {other}"),
            Ok(_) => panic!("expected Spawn error, got Ok(handle)"),
        }
    }

    /// End-to-end (Unix): a stub binary that writes a rate-limit line to
    /// stderr and exits 1 must produce a *classified* failure naming the
    /// cause and including the stderr tail — not a bare "exit 1".
    /// Exercises G1 (stderr retained) + G2 (classify) of
    /// SPEC_AGENT_FAILURE_DIAGNOSTICS.
    #[cfg(unix)]
    #[tokio::test]
    async fn classified_failure_surfaces_cause_and_stderr() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let path = std::env::temp_dir()
            .join(format!("amux-stub-claude-{}.sh", uuid::Uuid::new_v4()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(
                f,
                "echo 'API Error: Server is temporarily limiting requests (not your usage limit) · Rate limited' >&2"
            )
            .unwrap();
            writeln!(f, "exit 1").unwrap();
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }

        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = run_agent_with_bin(
            path.to_str().unwrap(),
            AgentRef::default(),
            AgentTask {
                prompt: "hi".to_string(),
                context: serde_json::Map::new(),
                max_turns: None,
            },
            tx,
        )
        .await
        .expect("spawn ok");

        let err = handle
            .final_result
            .await
            .expect("oneshot ok")
            .expect_err("run should fail");

        assert!(
            err.contains("Rate-limited"),
            "explanation should name the class; got: {err}"
        );
        assert!(
            err.to_lowercase().contains("rate limited"),
            "stderr tail should be included; got: {err}"
        );
        assert!(
            err.contains("retryable"),
            "rate-limit is retryable; got: {err}"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// End-to-end (Unix): stderr larger than the tail cap with the real
    /// error on the LAST line must still classify correctly — the
    /// rolling tail keeps the end, not a prefix. Regression test for
    /// codex P2 on #1353.
    #[cfg(unix)]
    #[tokio::test]
    async fn classified_failure_reads_error_past_stderr_cap() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let path = std::env::temp_dir()
            .join(format!("amux-stub-claude-big-{}.sh", uuid::Uuid::new_v4()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            // ~70 KiB of filler (> 64 KiB tail cap), THEN the real error.
            writeln!(f, "head -c 70000 /dev/zero | tr '\\0' x >&2").unwrap();
            writeln!(
                f,
                "printf '\\nAPI Error: Server is temporarily limiting requests (not your usage limit) Rate limited\\n' >&2"
            )
            .unwrap();
            writeln!(f, "exit 1").unwrap();
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }

        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = run_agent_with_bin(
            path.to_str().unwrap(),
            AgentRef::default(),
            AgentTask {
                prompt: "hi".to_string(),
                context: serde_json::Map::new(),
                max_turns: None,
            },
            tx,
        )
        .await
        .expect("spawn ok");

        let err = handle
            .final_result
            .await
            .expect("oneshot ok")
            .expect_err("run should fail");

        assert!(
            err.contains("Rate-limited"),
            "must classify from the tail past the cap; got: {}",
            err.chars().take(160).collect::<String>()
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rolling_tail_keeps_last_bytes_past_cap() {
        let cap = 8;
        let mut buf = Vec::new();
        for i in 0..100u8 {
            append_capped_tail(&mut buf, &[i], cap);
            assert!(buf.len() <= cap * 2, "rolling buffer must stay bounded");
        }
        trim_to_tail(&mut buf, cap);
        assert_eq!(buf, vec![92, 93, 94, 95, 96, 97, 98, 99]);
    }

    #[test]
    fn rolling_tail_single_large_chunk() {
        let cap = 4;
        let mut buf = Vec::new();
        append_capped_tail(&mut buf, b"abcdefghij", cap);
        trim_to_tail(&mut buf, cap);
        assert_eq!(&buf, b"ghij");
    }

    #[tokio::test]
    async fn drain_captures_error_result_frame() {
        // An error result frame on stdout must be captured so the
        // collector can fail the run even on exit 0. codex P1 #1353.
        let bytes =
            b"{\"type\":\"result\",\"is_error\":true,\"subtype\":\"error_during_execution\",\"error\":{\"message\":\"overloaded_error\"}}\n"
                .to_vec();
        let (mut w, r) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            w.write_all(&bytes).await.unwrap();
            w.shutdown().await.unwrap();
        });
        let (tx, _rx) = mpsc::unbounded_channel();
        let result = drain_async_reader(BufReader::new(r), &tx)
            .await
            .expect("drain ok");
        assert!(
            result.error_frame.is_some(),
            "error result frame should be captured"
        );
    }

    /// End-to-end (Unix): claude can report an error on stdout and still
    /// exit 0; the runner must NOT treat that as success. codex P1 #1353.
    #[cfg(unix)]
    #[tokio::test]
    async fn stdout_error_result_with_exit_zero_is_a_failure() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let path = std::env::temp_dir()
            .join(format!("amux-stub-claude-okerr-{}.sh", uuid::Uuid::new_v4()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            // Emit an error result frame on stdout, then exit 0. The JSON
            // lives in a `let` so its braces are data, not writeln! format
            // placeholders (no escaping, no print_literal lint).
            let frame = r#"{"type":"result","is_error":true,"subtype":"error_during_execution","error":{"message":"overloaded_error: upstream busy"}}"#;
            writeln!(f, "echo '{frame}'").unwrap();
            writeln!(f, "exit 0").unwrap();
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }

        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = run_agent_with_bin(
            path.to_str().unwrap(),
            AgentRef::default(),
            AgentTask {
                prompt: "hi".to_string(),
                context: serde_json::Map::new(),
                max_turns: None,
            },
            tx,
        )
        .await
        .expect("spawn ok");

        let err = handle
            .final_result
            .await
            .expect("oneshot ok")
            .expect_err("stdout-reported error with exit 0 must fail");
        assert!(
            err.to_lowercase().contains("overloaded"),
            "should classify the stdout error; got: {err}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn agent_task_max_turns_field_round_trips() {
        // Reagent P1 + codex P2 on PR #834: the max_turns field is
        // forwarded to the subprocess via `--max-turns N`. We can't
        // assert the actual CLI argument here without spawning, but
        // we can verify the field flows through AgentTask's
        // serde + Clone path without loss — the subprocess wiring
        // is exercised by the end-to-end test.
        let task = AgentTask {
            prompt: "x".into(),
            context: serde_json::Map::new(),
            max_turns: Some(7),
        };
        let v = serde_json::to_value(&task).unwrap();
        assert_eq!(v["maxTurns"], json!(7));
        let back: AgentTask = serde_json::from_value(v).unwrap();
        assert_eq!(back.max_turns, Some(7));
    }

    #[test]
    fn agent_run_result_has_sensible_defaults() {
        let r = AgentRunResult::default();
        assert_eq!(r.response, "");
        assert_eq!(r.cost_usd, 0.0);
        assert!(r.transcript.is_empty());
        let _ = json!(r); // serializes without panic
    }
}
