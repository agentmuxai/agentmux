// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified agent runner — one-shot Claude Code spawn for the
//! workflow Agent block.
//!
//! Spawns `claude --print --output-format=stream-json` as a non-
//! interactive subprocess, drains its stdout through
//! `ClaudeTranslator`, forwards each `AgentEvent` on the caller's
//! `tx`, and resolves the handle's `final_result` with the
//! structured `AgentRunResult` once the stream emits `Done`.
//!
//! Headless and one-shot by design — the workflow Agent block's
//! contract is "send task, wait for done, return result." The
//! interactive agent pane has its own PTY-based controller in
//! `blockcontroller/shell.rs`; that path is NOT routed through
//! this runner (see `docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md`
//! §4.2 — what's shared is the translator + event shape, not the
//! spawn function).

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

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
/// value via `final_result` (workflow Agent block's downstream
/// output) and the `instance_id` of the backing `db_agent_instances`
/// row.
///
/// Dropping the caller's receiver implicitly cancels the run only if
/// the runner observes the send error — Phase 2 adds an explicit
/// `AbortHandle`.
pub struct AgentRunHandle {
    pub instance_id: String,
    pub final_result: oneshot::Receiver<Result<AgentRunResult, String>>,
}

/// Error returned by the runner.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent runner: invalid AgentRef: {0}")]
    InvalidRef(String),
    #[error("agent runner: spawn failed: {0}")]
    Spawn(String),
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
/// are NOT plumbed in Phase 1.5 PR 2 — the workflow Agent block
/// always spawns fresh (per spec §8 "workflow runs always allocate
/// fresh instance_name"). The bundles can be added in a follow-up
/// once the workflow inspector (PR 3) needs to surface them.
pub async fn run_agent(
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

    let bin = std::env::var(ENV_CLAUDE_BIN)
        .unwrap_or_else(|_| DEFAULT_CLAUDE_BIN.to_string());

    // `claude --print` runs in non-interactive mode and exits when
    // done. `--output-format=stream-json` emits one JSON object per
    // line, the format ClaudeTranslator consumes. `--verbose` is
    // required alongside stream-json (the CLI rejects stream-json
    // without it). `--include-partial-messages` gives us the
    // streaming text_deltas — the translator skips the resulting
    // `partial: true` snapshots when building the transcript.
    let mut child = Command::new(&bin)
        .arg("--print")
        .arg("--output-format=stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
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

    let instance_id = format!("workflow-agent-{}", uuid::Uuid::new_v4());
    let (result_tx, result_rx) = oneshot::channel();

    // Drain stderr in the background — captured into the spawn
    // error path if the run fails. Detached: it just reads bytes
    // until EOF; not used for AgentEvent translation.
    tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = BufReader::new(stderr);
        // Best-effort drain; ignore errors (the stream may close
        // mid-read on cancellation).
        let _ = AsyncBufReadExt::read_line(&mut reader, &mut buf).await;
        // We don't currently propagate stderr to the AgentRunResult.
        // Phase 2 surfaces it on the workflowrun:<id> broker as a
        // diagnostic event.
    });

    tokio::spawn(async move {
        let result = drain_and_collect(stdout, &tx, &mut child).await;
        let _ = result_tx.send(result);
    });

    Ok(AgentRunHandle {
        instance_id,
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
) -> Result<AgentRunResult, String> {
    let result = drain_async_reader(BufReader::new(stdout), tx).await;

    // Wait for child to exit so the OS reaps it cleanly.
    let exit = child.wait().await.map_err(|e| format!("wait: {e}"))?;

    match result {
        Ok(mut accumulated) if exit.success() => {
            // If the stream never emitted Done (e.g. claude died
            // mid-run), surface a synthetic error rather than
            // returning empty defaults silently.
            if accumulated.response.is_empty() && accumulated.transcript.is_empty() {
                return Err("claude exited 0 but stream produced no Done event".to_string());
            }
            accumulated.transcript.shrink_to_fit();
            Ok(accumulated)
        }
        Ok(_) => Err(format!(
            "claude exited with status {exit} but stream emitted no error"
        )),
        Err(e) => Err(e),
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

    #[test]
    fn agent_run_result_has_sensible_defaults() {
        let r = AgentRunResult::default();
        assert_eq!(r.response, "");
        assert_eq!(r.cost_usd, 0.0);
        assert!(r.transcript.is_empty());
        let _ = json!(r); // serializes without panic
    }
}
