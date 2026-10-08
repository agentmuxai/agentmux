// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Claude CLI subprocess behind every ambient call: spawn it as bare text
//! generation, race it against cancellation and a timeout, and parse its
//! stream-json reply.

use crate::backend::obj;
use super::sanitize::sanitize_ambient_text;

/// Where ambient side calls run: `<data dir>/ambient-calls`, created on
/// demand; `None` (inherit srv's cwd, as before) only if it cannot be made.
/// No agent works there, so no agent's history ever matches its transcripts
/// (#3629).
pub(crate) fn ambient_call_cwd() -> Option<std::path::PathBuf> {
    let dir = crate::backend::base::get_mux_data_dir().join("ambient-calls");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Invoke the Claude CLI with Haiku model for a lightweight ambient call
/// (activity summary, ghost-text next-prompt suggestion, or any future
/// purpose routed through the Ambient Model Call gateway). Uses
/// `--model claude-haiku-4-5-20251001` and a 15s timeout.
///
/// `cancel` is this call's Ambient Model Call gateway cancellation token — if
/// a newer request for the same `(block_id, purpose)` key is admitted while
/// this one is still running, `cancel` fires and the child process is killed
/// immediately rather than left to run to completion (and keep burning
/// tokens) only to have its result discarded on arrival.
///
/// Returns the response text plus token usage parsed from the CLI's `result`
/// stream-json line (same `usage` shape the main turn pipeline parses —
/// see `agents::translator::claude::parse_usage`), so ambient calls are
/// never silently excluded from token accounting.
pub async fn invoke_haiku(
    cli_path: &str,
    prompt: &str,
    meta: &obj::MetaMapType,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(String, Option<crate::agents::TokenCounts>), String> {
    invoke_haiku_with_timeout(cli_path, prompt, meta, cancel, std::time::Duration::from_secs(15)).await
}

/// [`invoke_haiku`] with its own timeout, for calls off any
/// user-facing path that need longer than a pane-header summary (the rolling
/// continuity state, `backend::continuity_state`).
pub async fn invoke_haiku_with_timeout(
    cli_path: &str,
    prompt: &str,
    meta: &obj::MetaMapType,
    cancel: tokio_util::sync::CancellationToken,
    timeout: std::time::Duration,
) -> Result<(String, Option<crate::agents::TokenCounts>), String> {
    let auth_env = crate::backend::blockcontroller::cmd_env_of(meta);

    let mut cmd = crate::server::cli_handlers::make_cli_cmd(cli_path);
    // A bare text-generation call, not an agent session: no tools, one turn, no
    // slash commands or MCP servers, nothing saved, and our own system prompt in
    // place of Claude Code's. With the defaults the model answered as a
    // conversational assistant ("I don't have access to...") whenever the
    // digest was thin, and that text reached the UI.
    cmd.args(["-p", "--output-format", "stream-json", "--verbose",
              "--model", "claude-haiku-4-5-20251001",
              "--tools", "", "--max-turns", "1",
              "--no-session-persistence", "--disable-slash-commands",
              "--strict-mcp-config",
              "--system-prompt", crate::ambient::prompt::AMBIENT_SYSTEM_PROMPT])
        .envs(&auth_env);
    // Run in a scratch dir of our own, never srv's cwd (the user's home):
    // Claude files each call's transcript under its cwd's project folder,
    // and an agent working in that same dir would find every side call —
    // whose prompts quote other agents' conversations — in its own history
    // (#3629).
    if let Some(dir) = ambient_call_cwd() {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn activity CLI: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await
            .map_err(|e| format!("activity CLI stdin write: {e}"))?;
        stdin.shutdown().await
            .map_err(|e| format!("activity CLI stdin shutdown: {e}"))?;
    }

    // `child.wait()` only borrows (unlike `wait_with_output()`, which
    // consumes `child` by value and would make the cancel branch's
    // `child.kill()` below impossible). Drain stdout concurrently via a
    // separate task — same rationale `wait_with_output()` itself uses
    // internally — so a chatty response can't fill the OS pipe buffer and
    // deadlock the child waiting to write while nothing is reading.
    let mut stdout_pipe = child.stdout.take()
        .ok_or_else(|| "activity CLI: no stdout pipe".to_string())?;
    let stdout_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf).await;
        buf
    });

    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err("cancelled: superseded by a newer activity-summary request".to_string());
        }
        result = tokio::time::timeout(timeout, child.wait()) => {
            result.map_err(|_| format!("activity CLI timed out after {}s", timeout.as_secs()))?
                .map_err(|e| format!("activity CLI wait: {e}"))?
        }
    };

    if !status.success() {
        return Err(format!("activity CLI exited with status {status}"));
    }

    let stdout_bytes = stdout_task.await
        .map_err(|e| format!("activity CLI stdout reader task: {e}"))?;
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    // An empty reply is a valid answer ("nothing to say"), and the call still cost
    // tokens, so it comes back as `Ok` with empty text rather than an error that
    // would drop the usage. Callers already treat empty text as "no result".
    Ok(parse_cli_stream(&stdout))
}

/// The last assistant text block (sanitized) and the usage from a `claude -p
/// --output-format stream-json` transcript. Text is empty when the model said
/// nothing, or only wrapped nothing in a fence.
fn parse_cli_stream(stdout: &str) -> (String, Option<crate::agents::TokenCounts>) {
    let mut last_text = String::new();
    let mut tokens: Option<crate::agents::TokenCounts> = None;
    for line in stdout.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match val.get("type").and_then(|v| v.as_str()) {
            Some("assistant") => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                last_text = text.trim().to_string();
                            }
                        }
                    }
                }
            }
            Some("result") => {
                tokens = Some(crate::agents::translator::claude::parse_usage(val.get("usage")));
            }
            _ => {}
        }
    }
    (sanitize_ambient_text(&last_text), tokens)
}

#[cfg(test)]
mod ambient_call_cwd_tests {
    /// #3629: side calls run in a dir of their own under the data dir —
    /// never srv's cwd (the user's home), where an agent may work.
    #[test]
    fn side_calls_run_in_their_own_scratch_dir() {
        let dir = super::ambient_call_cwd().expect("the scratch dir can be made");
        assert!(dir.is_dir());
        assert!(dir.ends_with("ambient-calls"));
        assert!(dir.starts_with(crate::backend::base::get_mux_data_dir()));
        assert_ne!(Some(dir.as_path()), dirs::home_dir().as_deref());
    }
}

#[cfg(test)]
mod ambient_cli_stream_tests {
    use super::*;

    #[test]
    fn an_empty_fence_is_empty_text_but_the_usage_survives() {
        let out = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"```\n```"}]}}"#,
            r#"{"type":"result","usage":{"input_tokens":235,"output_tokens":4}}"#,
        ].join("\n");
        let (text, tokens) = parse_cli_stream(&out);
        assert_eq!(text, "");
        assert!(tokens.is_some());
    }

    #[test]
    fn the_last_assistant_text_wins_and_thinking_blocks_are_ignored() {
        let out = [
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Run the tests"}]}}"#,
            r#"{"type":"result","usage":{"input_tokens":1,"output_tokens":2}}"#,
        ].join("\n");
        assert_eq!(parse_cli_stream(&out).0, "Run the tests");
    }

    #[test]
    fn a_transcript_with_no_result_line_has_no_usage() {
        let out = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"x"}]}}"#;
        assert!(parse_cli_stream(out).1.is_none());
    }
}

/// Needs a logged-in `claude` on PATH and spends a few tokens, so it is not part
/// of the default run: `cargo test --bin agentmux-srv live_cli -- --ignored`.
/// It exercises the real argument list (`--tools ""`, `--system-prompt`, ...)
/// that the stream-parsing tests above cannot.
#[cfg(test)]
mod live_cli_smoke {
    use super::*;

    #[tokio::test]
    #[ignore = "spawns the real claude CLI"]
    async fn live_cli_answers_a_thin_prompt_with_nothing_instead_of_a_refusal() {
        let meta = obj::MetaMapType::new();
        let prompt = crate::ambient::prompt::build_next_prompt_prompt("[user] hi");
        let (text, tokens) = invoke_haiku(
            "claude",
            &prompt,
            &meta,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("the CLI ran");
        assert!(tokens.is_some(), "usage is reported");
        // A thin digest: the answer is SKIP, in the reply format, never prose.
        assert_ne!(crate::ambient::reply::parse(&text), crate::ambient::reply::Parsed::Malformed, "unexpected reply: {text}");
        assert!(!text.to_lowercase().contains("i don't have"), "{text}");
    }

    /// The reply format (`reply`) has to hold on the real model before it can be
    /// relied on: every reply is `ANSWER: …` or `SKIP`, across activity that
    /// clearly has a next step, finished work, and activity too thin to read.
    /// `cargo test --bin agentmux-srv live_cli_reply_format -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "spawns the real claude CLI, about a dozen calls"]
    async fn live_cli_reply_format_holds_across_digests() {
        use crate::ambient::reply::{parse, Parsed};
        let digests = [
            // A clear next step.
            "[user] fix the login redirect loop\n[assistant] Found it: the session cookie was set after the redirect. Fixed in auth.ts; the login flow works now.",
            "[user] add a dark mode toggle\n[assistant] Added the toggle to the settings panel and wired it to the theme store. I haven't written tests yet.",
            "[user] why is CI red\n[tool] Bash\n[error] npm test exited 1\n[assistant] Two snapshot tests fail after the button restyle; the snapshots need updating.",
            "[user] profile the slow search\n[tool] Bash\n[assistant] Search spends 80% of its time re-sorting the whole index on every keystroke.",
            "[user] bump the version\n[assistant] Bumped to 2.4.0 in package.json and the lockfile. The changelog still lists these changes under Unreleased.",
            "[user] write the migration\n[assistant] The migration adds the column with a default and backfills in batches of 1000. It hasn't been run against staging.",
            // Finished, nothing obvious left.
            "[user] thanks, that's all\n[assistant] You're welcome. Everything is merged and deployed.",
            "[user] merge it\n[assistant] Merged. The branch is deleted and main is green.",
            // Thin or unreadable.
            "[user] hi\n[assistant] Hello.",
            "[user] ok\n[assistant] Done.",
            "[tool] Read\n[tool] Read\n[tool] Grep\n[assistant] Looking.",
            "[user] continue\n[assistant] Continuing with the next file.",
        ];
        let meta = obj::MetaMapType::new();
        let mut bad = Vec::new();
        let mut failed = 0;
        for digest in digests {
            let prompt = crate::ambient::prompt::build_next_prompt_prompt(digest);
            // A CLI failure (most often the time limit) says nothing about the
            // format; it is counted, not judged.
            let text = match invoke_haiku("claude", &prompt, &meta, tokio_util::sync::CancellationToken::new()).await {
                Ok((text, _)) => text,
                Err(e) => {
                    println!("CLI failed: {e}");
                    failed += 1;
                    continue;
                }
            };
            let parsed = parse(&text);
            println!("{parsed:?} <- {text:?}");
            if parsed == Parsed::Malformed {
                bad.push(text);
            }
        }
        println!("{failed} of {} calls failed in the CLI", digests.len());
        assert!(failed < digests.len(), "no call completed");
        assert!(bad.is_empty(), "replies not in the format: {bad:?}");
    }

    /// A message that waits for the user without a question mark passes the
    /// turn-ending gate, so the prompt's SKIP case is what keeps a guess out of
    /// the composer (#4476). Live, like the test above.
    #[tokio::test]
    #[ignore = "calls the real claude CLI; run with --ignored"]
    async fn live_cli_skips_a_wait_the_gate_cannot_see() {
        use crate::ambient::reply::{parse, Parsed};
        let digests = [
            "[user] pick a cache\n[assistant] Redis or an in-process LRU would both work. Tell me which one and I'll start.",
            "[user] plan the refactor\n[assistant] Here's the plan: split the parser, then move the tests. Let me know if that works for you before I begin.",
            "[user] deploy it\n[assistant] Ready to deploy to production. Waiting for your go-ahead.",
        ];
        let meta = obj::MetaMapType::new();
        let mut answered = Vec::new();
        for digest in digests {
            let prompt = crate::ambient::prompt::build_next_prompt_prompt(digest);
            let Ok((text, _)) = invoke_haiku("claude", &prompt, &meta, tokio_util::sync::CancellationToken::new()).await
            else {
                continue;
            };
            let parsed = parse(&text);
            println!("{parsed:?} <- {text:?}");
            if parsed != Parsed::Skip {
                answered.push(text);
            }
        }
        assert!(answered.is_empty(), "answered a message that waits for the user: {answered:?}");
    }
}
