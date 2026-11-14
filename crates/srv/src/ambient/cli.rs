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

/// The model every ambient call runs on.
pub const AMBIENT_MODEL: &str = "claude-haiku-5-5";

/// Invoke the Claude CLI on [`AMBIENT_MODEL`] for one ambient call, within
/// `timeout` (the purpose's own, `purpose::Purpose::timeout`).
///
/// `cancel` is this call's Ambient Model Call gateway cancellation token — if
/// a newer request for the same `(entity, purpose)` key is admitted while
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
    timeout: std::time::Duration,
) -> Result<(String, Option<crate::agents::TokenCounts>), String> {
    let auth_env = ambient_env(meta)?;
    // Not on an identity whose first spawn is still refreshing its token (#6081): held until that
    // spawn's first turn has ended, and over this call's own run. Gives up on cancel, or when the
    // wait alone uses up the call's time.
    let began = std::time::Instant::now();
    let _identity = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("cancelled: superseded by a newer request".to_string()),
        held = tokio::time::timeout(timeout, crate::identity::token_warmup::ambient_gate(auth_env.get("CLAUDE_CONFIG_DIR").map(String::as_str))) => {
            held.map_err(|_| format!("ambient CLI waited {}s for its identity's first turn", timeout.as_secs()))?
        }
    };

    let mut cmd = crate::backend::pane_env::make_cli_cmd(cli_path);
    // A bare text-generation call, not an agent session: no tools, one turn, no
    // slash commands or MCP servers, nothing saved, and our own system prompt in
    // place of Claude Code's. With the defaults the model answered as a
    // conversational assistant ("I don't have access to...") whenever the
    // digest was thin, and that text reached the UI.
    cmd.args(["-p", "--output-format", "stream-json", "--verbose",
              "--model", AMBIENT_MODEL,
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
    crate::backend::process_tracker::registry::own_process_group(&mut cmd);
    let mut child = agentmux_common::process::KillGroupOnDrop(cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn the ambient CLI: {e}"))?);
    crate::backend::process_tracker::registry::record_untracked_cli(child.id());

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await
            .map_err(|e| format!("ambient CLI stdin write: {e}"))?;
        stdin.shutdown().await
            .map_err(|e| format!("ambient CLI stdin shutdown: {e}"))?;
    }

    // Drained to the end whatever it holds, so a chatty child can't block on a full pipe; only the end is kept, as the
    // reason an exit that failed is logged with.
    let stderr_task = child.stderr.take().map(|pipe| tokio::spawn(read_tail(pipe, STDERR_TAIL_BYTES)));

    // What the gate wait left of the call's time is all the child gets.
    let budget = timeout;
    let timeout = budget.saturating_sub(began.elapsed());

    // `child.wait()` only borrows (unlike `wait_with_output()`, which
    // consumes `child` by value and would make the cancel branch's
    // group kill below impossible). Drain stdout concurrently via a
    // separate task — same rationale `wait_with_output()` itself uses
    // internally — so a chatty response can't fill the OS pipe buffer and
    // deadlock the child waiting to write while nothing is reading.
    let stdout_pipe = child.stdout.take()
        .ok_or_else(|| "ambient CLI: no stdout pipe".to_string())?;
    let stdout_task = tokio::spawn(read_tail(stdout_pipe, usize::MAX));

    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = agentmux_common::process::kill_child_and_group(&mut child).await;
            return Err("cancelled: superseded by a newer request".to_string());
        }
        result = tokio::time::timeout(timeout, child.wait()) => match result {
            Ok(status) => status.map_err(|e| format!("ambient CLI wait: {e}"))?,
            Err(_) => {
                let _ = agentmux_common::process::kill_child_and_group(&mut child).await;
                return Err(format!("ambient CLI timed out after {}s", budget.as_secs()));
            }
        },
    };

    if !status.success() {
        return Err(exit_error(&status.to_string(), &stderr_tail_of(stderr_task).await));
    }

    let stdout_bytes = stdout_task.await
        .map_err(|e| format!("ambient CLI stdout reader task: {e}"))?;
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    // An empty reply is a valid answer ("nothing to say"), and the call still cost
    // tokens, so it comes back as `Ok` with empty text rather than an error that
    // would drop the usage. Callers already treat empty text as "no result".
    Ok(parse_cli_stream(&stdout))
}

/// How much of the end of the CLI's stderr is kept for a failed exit.
const STDERR_TAIL_BYTES: usize = 2048;
/// The most characters of it that go into the error (and so the log).
const STDERR_LOGGED_CHARS: usize = 200;

/// Read `pipe` to its end, keeping only the last `cap` bytes.
async fn read_tail(mut pipe: impl tokio::io::AsyncRead + Unpin, cap: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut tail: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => return tail,
            Ok(n) => {
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > cap {
                    tail.drain(..tail.len() - cap);
                }
            }
        }
    }
}

/// What the stderr reader task kept, waiting at most a second for it (the child has exited, so its pipe is closed).
async fn stderr_tail_of(task: Option<tokio::task::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    let Some(task) = task else { return Vec::new() };
    tokio::time::timeout(std::time::Duration::from_secs(1), task).await.ok().and_then(Result::ok).unwrap_or_default()
}

/// The error for a CLI that exited unsuccessfully: its status, and the end of what it wrote to stderr on one line.
fn exit_error(status: &str, stderr_tail: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr_tail);
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = line.chars().count();
    if line.is_empty() {
        return format!("ambient CLI exited with status {status}");
    }
    let shown: String = if count > STDERR_LOGGED_CHARS { format!("…{}", line.chars().skip(count - STDERR_LOGGED_CHARS).collect::<String>()) } else { line };
    format!("ambient CLI exited with status {status}{}{shown}", super::outcome::CHILD_OUTPUT_MARK)
}

/// Whether the agent in this block is on a model server of its own, for which no ambient call is made (`ambient_env`).
pub(crate) fn has_own_model_server(meta: &obj::MetaMapType) -> bool {
    ambient_env(meta).is_err()
}

/// Whether the agent in this block runs on one of AgentMux's model routes. The route's variables are set at each spawn,
/// not kept in the pane's saved environment, so the pane's record of the route it really runs on is what says so.
pub(crate) fn on_model_route(meta: &obj::MetaMapType) -> bool {
    meta.get(crate::backend::model_gateway::launch::BLOCK_META_KEY).is_some_and(|v| !v.is_null())
}

/// Whether the background title recovery leaves this agent alone: it is on a model server of its own or on a model
/// route, and a recovery call would use the person's own sign-in, and send the conversation to Anthropic, instead.
pub(crate) fn recovery_skips(meta: &obj::MetaMapType) -> bool {
    has_own_model_server(meta) || on_model_route(meta)
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

/// The environment of an ambient call: the pane's saved one. Not for an agent on a model server of its own:
/// the call would read the person's stored sign-in from the shared config directory and send it, with a digest
/// of the conversation, to that server (or send the digest to Anthropic, which is what pointing the agent
/// elsewhere was meant to avoid). It makes no call at all.
fn ambient_env(meta: &obj::MetaMapType) -> Result<std::collections::HashMap<String, String>, String> {
    let env = crate::backend::blockcontroller::cmd_env_of(meta);
    match crate::backend::spawn_env::endpoint::requested(&env) {
        Some(_) => Err("no ambient call for an agent on a model server of its own".to_string()),
        None => Ok(env),
    }
}

#[cfg(test)]
mod ambient_endpoint_tests {
    use super::*;

    /// The person's sign-in and the conversation stay off a server an agent was pointed at: the call is not made.
    #[tokio::test]
    async fn an_agent_on_a_server_of_its_own_gets_no_ambient_call() {
        for env in [
            serde_json::json!({ "CLAUDE_CONFIG_DIR": "C:/shared/providers/claude", "ANTHROPIC_BASE_URL": "http://192.0.2.68:11434" }),
            serde_json::json!({ "anthropic_base_url": "http://192.0.2.68:11434" }),
        ] {
            let mut meta = obj::MetaMapType::new();
            meta.insert("cmd:env".to_string(), env);
            let out = invoke_haiku(
                "a-cli-that-does-not-exist",
                "prompt",
                &meta,
                tokio_util::sync::CancellationToken::new(),
                std::time::Duration::from_secs(5),
            )
            .await;
            assert_eq!(out.unwrap_err(), "no ambient call for an agent on a model server of its own");
        }
    }
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
mod ambient_cli_failure_tests {
    use super::*;

    #[tokio::test]
    async fn only_the_end_of_what_a_child_writes_is_kept() {
        let long: Vec<u8> = (0..10_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let tail = read_tail(&long[..], 100).await;
        assert_eq!(tail, long[long.len() - 100..]);
        assert_eq!(read_tail(&b"short"[..], 100).await, b"short");
    }

    #[test]
    fn a_failed_exit_carries_the_end_of_stderr_on_one_line() {
        let tail = b"Error: not logged in\n  run /login\n";
        assert_eq!(exit_error("exit code: 1", tail), "ambient CLI exited with status exit code: 1 | stderr: Error: not logged in run /login");
        assert_eq!(exit_error("exit code: 1", b""), "ambient CLI exited with status exit code: 1");
        let long = "x ".repeat(500);
        let e = exit_error("1", long.as_bytes());
        assert!(e.chars().count() < 260, "{}", e.chars().count());
        assert!(e.contains("…"));
    }

    #[test]
    fn an_agent_on_a_model_server_of_its_own_is_known_from_its_block() {
        let mut meta = obj::MetaMapType::new();
        assert!(!has_own_model_server(&meta));
        meta.insert("cmd:env".to_string(), serde_json::json!({ "ANTHROPIC_BASE_URL": "http://192.0.2.68:11434" }));
        assert!(has_own_model_server(&meta));
    }

    #[test]
    fn an_agent_on_a_model_route_is_known_from_the_route_its_pane_records() {
        let mut meta = obj::MetaMapType::new();
        assert!(!on_model_route(&meta) && !recovery_skips(&meta));
        meta.insert("agent:route_model".to_string(), serde_json::json!(null));
        assert!(!on_model_route(&meta), "a route left is recorded as null");
        meta.insert("agent:route_model".to_string(), serde_json::json!({ "route": "openrouter", "model": "m" }));
        assert!(on_model_route(&meta) && recovery_skips(&meta));
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

/// The side call's CLI leads its own process group, so cancelling it ends the
/// helpers it started, and a signal to its group can't reach srv.
#[cfg(all(test, unix))]
pub mod ambient_cli_group_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[tokio::test]
    async fn the_cli_leads_its_own_group_and_cancel_ends_its_helper() {
        let dir = std::env::temp_dir().join(format!("ambient-group-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = dir.join("report");
        let stub = dir.join("claude-stub.sh");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\ncat >/dev/null\nsleep 30 &\necho \"$$ $(ps -o pgid= -p $$ | tr -d ' ') $!\" > {r}.tmp && mv {r}.tmp {r}\nwait\n",
                r = report.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

        let cancel = tokio_util::sync::CancellationToken::new();
        let call = tokio::spawn({
            let (stub, cancel) = (stub.to_string_lossy().into_owned(), cancel.clone());
            async move {
                invoke_haiku(&stub, "hi", &Default::default(), cancel, std::time::Duration::from_secs(30)).await
            }
        });
        let mut fields = Vec::new();
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(&report) {
                fields = text.split_whitespace().filter_map(|f| f.parse::<u32>().ok()).collect();
                if fields.len() == 3 {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(fields.len(), 3, "the stub reported");
        let (pid, pgid, helper) = (fields[0], fields[1], fields[2]);
        assert_eq!(pid, pgid, "the CLI leads its own process group");
        assert_ne!(pgid as i32, unsafe { libc::getpgid(0) }, "not srv's group");

        cancel.cancel();
        assert!(call.await.unwrap().unwrap_err().starts_with("cancelled"));
        let mut gone = false;
        for _ in 0..100 {
            if agentmux_common::process::process_start_time(helper).is_none() || is_zombie(helper) {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(gone, "the CLI's helper {helper} outlived the cancel");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn is_zombie(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| s.rfind(')').map(|i| s[i + 1..].trim_start().starts_with('Z')))
            .unwrap_or(false)
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
            crate::ambient::purpose::NEXT_PROMPT_SUGGESTION.timeout,
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
            let text = match invoke_haiku(
                "claude",
                &prompt,
                &meta,
                tokio_util::sync::CancellationToken::new(),
                crate::ambient::purpose::NEXT_PROMPT_SUGGESTION.timeout,
            )
            .await
            {
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
            let Ok((text, _)) = invoke_haiku(
                "claude",
                &prompt,
                &meta,
                tokio_util::sync::CancellationToken::new(),
                crate::ambient::purpose::NEXT_PROMPT_SUGGESTION.timeout,
            )
            .await
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

    /// The same check for every other one-line purpose: titles (create and
    /// maintain), names (with an agent's task as the material, which Haiku used to
    /// try to carry out), narration and previews.
    /// `cargo test --bin agentmux-srv live_cli_reply_format_every -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "spawns the real claude CLI, about ten calls"]
    async fn live_cli_reply_format_holds_for_every_purpose() {
        use crate::ambient::prompt::*;
        use crate::ambient::reply::{parse, Parsed};
        let task = "Investigate the repository at C:\\work\\app. Read src/auth and report every place the session \
                    cookie is set, with file:line references. Do not change any files.";
        let prompts = [
            ("title, create", build_session_title_prompt("", Some("fix the login redirect loop"), None, 7)),
            ("title, create, nudge", build_session_title_prompt("", Some("continue"), None, 7)),
            ("title, maintain, same goal", build_session_title_prompt("Fix login redirect loop", Some("also check the logout path"), None, 7)),
            ("title, maintain, new goal", build_session_title_prompt("Fix login redirect loop", Some("now let's build the billing page"), None, 7)),
            ("title, from activity", build_session_title_prompt("", None, Some("[user] add a dark mode toggle\n[assistant] Added it."), 7)),
            ("subagent name", build_subagent_name_prompt(task)),
            ("dispatch name", build_dispatch_name_prompt(task)),
            ("narration", narration_prompt("background_task", "task dev").unwrap()),
            ("preview", build_definition_summary_prompt("[user] add a dark mode toggle\n[assistant] Added it and its tests.")),
            ("preview, thin", build_definition_summary_prompt("[user] hi\n[assistant] Hello.")),
        ];
        let meta = obj::MetaMapType::new();
        let mut bad = Vec::new();
        for (name, prompt) in prompts {
            let text = match invoke_haiku(
                "claude",
                &prompt,
                &meta,
                tokio_util::sync::CancellationToken::new(),
                crate::ambient::purpose::ACTIVITY_SUMMARY.timeout,
            )
            .await
            {
                Ok((text, _)) => text,
                Err(e) => {
                    println!("{name}: CLI failed: {e}");
                    continue;
                }
            };
            let parsed = parse(&text);
            println!("{name}: {parsed:?} <- {text:?}");
            if parsed == Parsed::Malformed {
                bad.push(format!("{name}: {text}"));
            }
        }
        assert!(bad.is_empty(), "replies not in the format: {bad:?}");
    }
}
