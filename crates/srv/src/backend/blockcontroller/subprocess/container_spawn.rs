// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Container-exec turn spawn (`spawn_container_turn`) for [`SubprocessController`].
//!
//! Docker-exec analog of `host_spawn::spawn_turn`, used for container agents.
//! Drives the exec via `bollard`'s Docker socket API instead of a host
//! `std::process` child (P1a: no secrets in argv — env vars travel through
//! `CreateExecOptions.env`, never process argv / `/proc/<pid>/cmdline`).
//!
//! `spawn_container_turn` refuses, queues or accepts a request here and hands the
//! accepted turn to the steps in `container_spawn_steps.rs`.
//! `publish_line` is its private output-line helper (used only here).

use std::sync::Arc;

use crate::backend::blockcontroller::{core, session_stats, shell};

use super::{argv::build_turn_argv, SubprocessController, SubprocessControllerInner, SubprocessSpawnConfig, SUBPROCESS_OUTPUT_SUBJECT};

#[path = "container_spawn_steps.rs"]
mod steps;

#[cfg(test)]
#[path = "container_turn_tests.rs"]
mod container_turn_tests;

/// Wrap the CLI argv so it reads the turn prompt from `prompt_path`, a file
/// already written into the container by
/// [`ContainerManager::upload_turn_prompt`].
///
/// Redirecting from a file is the only transport that satisfies all three
/// constraints at once (see `upload_turn_prompt`'s doc for the measurements):
/// the CLI gets a real stdin that really reaches EOF, the message never touches
/// argv or env — so `docker top` / host `ps` / `/proc/<pid>/cmdline` can't see a
/// secret pasted into a chat message — and there is no `MAX_ARG_STRLEN` ceiling.
///
/// The CLI argv is passed through as `"$@"` rather than interpolated into the
/// script, so there is no shell quoting to get wrong, and it is passed
/// UNCHANGED — including a trailing `-` (codex) or a `-p ""` placeholder
/// (gemini/qwen/kimi/antigravity), both of which mean "read the prompt from
/// stdin" and are correct again now that stdin genuinely works (codex P1,
/// PR #2883).
///
/// The prompt file is removed after the CLI exits, and the CLI's own exit
/// status is preserved for `inspect_exec` to classify the turn.
///
/// The directory the provider CLI is installed into on first start is appended
/// to `PATH`, after the image's own, so a CLI baked into an older image still
/// wins over the installed one.
fn container_turn_exec(prompt_path: &str, cmd: Vec<String>) -> Vec<String> {
    // bash, not sh: the image's sh is dash, which drops the exported `git` and `gh` functions
    // (`BASH_FUNC_*` environment entries) before the provider starts.
    let mut out = vec![
        "bash".to_string(),
        "-c".to_string(),
        // $1 = prompt path, "$@" (after shift) = the CLI and its args.
        format!(
            r#"PATH="$PATH:{}"; export PATH; F="$1"; shift; "$@" < "$F"; rc=$?; rm -f "$F"; exit $rc"#,
            crate::backend::container::cli::cli_path_dirs()
        ),
        // $0 for the script -- a label only.
        "agentmux-turn".to_string(),
        prompt_path.to_string(),
    ];
    out.extend(cmd);
    out
}

#[cfg(test)]
mod turn_exec_tests {
    use super::container_turn_exec;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// The security property this shape exists for (reagent P1, PR #2883): the
    /// message must never reach argv, where `docker top` / `ps` /
    /// `/proc/<pid>/cmdline` would expose a pasted secret. It can't — the argv
    /// is built from the prompt's PATH and never sees the message at all.
    #[test]
    fn the_argv_carries_only_a_path_never_the_message() {
        let argv = container_turn_exec("/tmp/agentmux-turn-abc", v(&["claude", "-p"]));
        assert!(argv.iter().any(|a| a == "/tmp/agentmux-turn-abc"), "the path is there");
        assert!(
            argv.iter().all(|a| !a.contains("ghp_") && !a.contains("secret")),
            "nothing message-shaped is: {argv:?}",
        );
    }

    /// The CLI must actually read the file — this redirect IS the fix.
    #[test]
    fn redirects_the_clis_stdin_from_the_prompt_file() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(argv[2].contains(r#""$@" < "$F""#), "must redirect: {}", argv[2]);
    }

    /// The prompt file is transient state; leaving it behind would accumulate
    /// one file per turn for the life of the container.
    #[test]
    fn removes_the_prompt_file_afterwards() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(argv[2].contains(r#"rm -f "$F""#), "must clean up: {}", argv[2]);
    }

    /// `inspect_exec` classifies the turn from the exit code, so the CLI's own
    /// status must survive the cleanup that runs after it.
    #[test]
    fn preserves_the_clis_exit_status_across_cleanup() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(argv[2].contains("rc=$?"), "captures status before rm");
        assert!(argv[2].contains("exit $rc"), "and re-raises it: {}", argv[2]);
    }

    /// The installed CLI is found after the image's own `PATH`, not before it.
    #[test]
    fn appends_the_install_dir_to_path_after_the_images_own() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p"]));
        assert!(
            argv[2].starts_with(
                r#"PATH="$PATH:/home/agent/.agentmux/cli/node_modules/.bin:/home/agent/.agentmux/cli/bin:/home/agent/.agentmux/cli/venv/bin"; export PATH;"#
            ),
            "{}",
            argv[2]
        );
    }

    /// An exported function (`BASH_FUNC_*`) reaches the provider through the launcher: the container
    /// agent's `git` and `gh` are such functions, and the image's `sh` would drop them.
    #[cfg(unix)]
    #[test]
    fn an_exported_function_reaches_the_provider() {
        let prompt = std::env::temp_dir().join(format!("amx-turn-fn-{}", std::process::id()));
        std::fs::write(&prompt, "").unwrap();
        let argv = container_turn_exec(prompt.to_str().unwrap(), v(&["bash", "-c", "type -t gh"]));
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .env("BASH_FUNC_gh%%", "() {  echo from-function
}")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "function");
        let _ = std::fs::remove_file(&prompt);
    }

    /// Passed as "$@", so the CLI argv survives verbatim and in order.
    #[test]
    fn passes_the_cli_argv_through_unchanged() {
        let argv = container_turn_exec("/tmp/p", v(&["claude", "-p", "--model", "opus"]));
        assert_eq!(&argv[0], "bash");
        assert_eq!(&argv[1], "-c");
        // argv[3] is $0, argv[4] is the prompt path; the CLI argv follows.
        assert_eq!(&argv[5..], &v(&["claude", "-p", "--model", "opus"])[..]);
    }

    /// codex's trailing `-` means "read the prompt from stdin" and must NOT be
    /// stripped: stdin is a real, EOF-terminating file now. An earlier cut of
    /// this fix removed it, which was only necessary while the message was
    /// being smuggled through argv.
    #[test]
    fn keeps_codexs_trailing_stdin_positional() {
        let argv = container_turn_exec("/tmp/p", v(&["codex", "exec", "--json", "-"]));
        assert_eq!(argv.last().unwrap(), "-");
    }

    /// Same for the `-p ""` placeholder gemini/qwen/kimi/antigravity use to
    /// mean "prompt comes from stdin" (codex P1, PR #2883).
    #[test]
    fn keeps_an_empty_prompt_placeholder() {
        let argv = container_turn_exec("/tmp/p", v(&["gemini", "-p", "", "--json"]));
        assert_eq!(&argv[5..], &v(&["gemini", "-p", "", "--json"])[..]);
    }
}

impl SubprocessController {
    /// Record what a container turn does before its exec (start the container,
    /// install the CLI). Call before [`spawn_container_turn`](Self::spawn_container_turn):
    /// the step runs at the start of that turn's own task, and again for each
    /// message queued behind it, so a failed start-up is retried by the next one.
    pub fn set_container_startup(&self, step: super::StartupStep) {
        *self.container_startup.lock().unwrap() = Some(step);
    }

    /// Spawn a container agent turn via Docker socket (P1a: no secrets in argv).
    ///
    /// This is the secure alternative to `spawn_turn` for container agents. Instead
    /// of running `docker exec -e KEY=VALUE ...` as a CLI subprocess (which exposes
    /// secrets in process argv / `/proc/<pid>/cmdline`, CWE-214), this method calls
    /// `ContainerManager::exec` directly, passing env vars through
    /// `CreateExecOptions.env` (Docker socket). The exec I/O (file-backed prompt
    /// + stdout NDJSON stream) drives the same state machine as `spawn_turn`:
    ///   • appends `--resume <sid>` if a prior session_id is known
    ///   • uploads the turn message into the container as a file and redirects
    ///     the CLI's stdin from it — see `container_turn_exec` for why the
    ///     exec's own stdin, argv, and env are all unusable for this
    ///   • reads NDJSON from the output stream, publishing MPS blockfile events
    ///   • captures session_id from the provider's init event
    ///   • transitions status running → done
    ///   • drains the pending-message queue when the exec exits
    ///
    /// `base_cmd` is `[cli_command] + cli_args` WITHOUT resume — this method appends
    /// `--resume <sid>` internally before starting the exec.
    /// The exec env is derived from THIS message's `config.env_vars` (denylist
    /// applied here, per-turn) — not carried across queue drains — so a queued
    /// message runs with its own freshly-resolved auth/env, matching `spawn_turn`.
    ///
    /// Takes `cm` and `container_name` by value (not reference) so the returned
    /// future is `'static` — required for `tokio::spawn` in the queue-drain path.
    pub fn spawn_container_turn(
        &self,
        cm: crate::backend::container::ContainerManager,
        container_name: String,
        base_cmd: Vec<String>,
        config: SubprocessSpawnConfig,
    ) -> Result<(), String> {
        // The container path keeps the prompt out of argv on purpose (module
        // doc: a pasted secret must not show in `ps`), so a provider that
        // can only take its prompt as an argument doesn't run here.
        if !config.prompt_arg_flag.is_empty() {
            return Err(
                "This agent's CLI takes its message on the command line, which the container runtime doesn't do. Run it on the host."
                    .to_string(),
            );
        }
        // A host Codex pane compacts through a short-lived App Server
        // (codex_compact.rs); inside a container that isn't built yet, and
        // `exec` would take "/compact" as a prompt.
        if super::codex_compact::is_compact_request(&config) {
            return Err("Compacting a Codex agent that runs in a container isn't supported yet.".to_string());
        }
        if !self.try_lock_run() {
            let mut inner = self.inner.lock().unwrap();
            tracing::info!(
                block_id = %self.block_id,
                queue_depth = inner.pending_messages.len() + 1,
                "container exec busy — message queued"
            );
            inner.pending_messages.push_back(config);
            return Ok(());
        }

        // Shared for the whole turn: an update of this container waits for none, and refuses
        // while one holds it.
        let Ok(update_guard) = crate::backend::container::drift::update_gate(&container_name).try_read_owned() else {
            self.unlock_run();
            return Err("This agent's container is being updated. Send the message again in a moment.".to_string());
        };

        self.hydrate_session_id_from_config(config.session_id.as_deref());

        let session_id_hint = self.inner.lock().unwrap().session_id.clone();
        let cmd = match build_turn_argv(
            &base_cmd,
            &config.resume_strategy,
            &config.resume_flag,
            session_id_hint.as_deref(),
        ) {
            Ok(cmd) => cmd,
            Err(error) => {
                self.unlock_run();
                return Err(error);
            }
        };

        self.emit_message_accepted(&config);

        let turn = self.begin_container_turn(cm, container_name, base_cmd, cmd, &config);

        // Spawn all async work (exec + I/O) into a background task so this
        // function returns synchronously. This is required so the queue-drain
        // path inside the reader task can call `spawn_container_turn` without
        // needing the returned future to be `'static`.
        tokio::spawn(steps::run_turn(turn, update_guard));

        Ok(())
    }

    /// A queued message has already been removed from `pending_messages` when
    /// its next spawn is attempted. If provider argv validation rejects that
    /// spawn, persist a transcript error so the message does not disappear
    /// with only a server log. Mirrors the host queue-drain failure path.
    fn publish_queued_spawn_error(
        block_id: &str,
        error: &str,
        broker: &Option<Arc<crate::backend::mps::Broker>>,
        filestore: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
        mstore: &Option<Arc<crate::backend::storage::store::Store>>,
    ) {
        let Some(broker) = broker else {
            return;
        };
        let error_frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": {"message": format!("[AgentMux] queued message could not be sent: {error}")}
        }).to_string();
        // Into the agent's global zone too: that is what the pane reads on
        // reload (Phase 5a-3c).
        let global_zone = crate::backend::transcript::append::resolve_global_output_zone(mstore, block_id);
        shell::handle_append_block_file(
            broker,
            block_id,
            SUBPROCESS_OUTPUT_SUBJECT,
            format!("{error_frame}\n").as_bytes(),
            filestore.as_ref(),
            global_zone.as_deref(),
        );
    }

    /// Publish a single NDJSON line from container exec output: session-id
    /// capture, MPS blockfile event, and FileStore write-through. Used by
    /// `spawn_container_turn`'s output reader task.
    fn publish_line(
        line: &str,
        block_id: &str,
        session_id_field: &str,
        inner: &std::sync::Mutex<SubprocessControllerInner>,
        mstore: &Option<Arc<crate::backend::storage::store::Store>>,
        event_bus: &Option<Arc<crate::backend::eventbus::EventBus>>,
        broker: &Option<Arc<crate::backend::mps::Broker>>,
        filestore: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
        stats: &mut session_stats::SessionStatsAccumulator,
        global_output_zone: Option<&str>,
        last_result_frame: &mut Option<serde_json::Value>,
        last_inband_error: &mut Option<serde_json::Value>,
        parked_notices_due: &mut bool,
    ) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        stats.record_line(trimmed.len(), mstore);

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
            // Capture session_id from provider init event.
            if let Some(sid) = parsed.get(session_id_field).and_then(|v| v.as_str()) {
                let changed = SubprocessController::record_captured_session_id_inner(inner, sid);
                if changed {
                    tracing::info!(block_id = %block_id, session_id = %sid, "container exec: captured session id");
                    core::persist_session_id(block_id, sid, &mstore, &event_bus);
                }
            }

            // Retain the terminal `result` frame for completion-time failure
            // classification (mirrors host_spawn.rs's stdout_reader).
            if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
                *last_result_frame = Some(parsed);
            } else if parsed.get("type").and_then(|v| v.as_str()) == Some("assistant")
                && (parsed.get("isApiErrorMessage").and_then(|v| v.as_bool()).unwrap_or(false)
                    || parsed.get("error").is_some())
            {
                // In-band API error: 401 / auth failures arrive as a synthetic
                // assistant message (exit 0, is_error:false on result frame) —
                // capture it so the completion-time classifier below can trip
                // the failure gate.
                *last_inband_error = Some(parsed);
            } else if crate::agents::limit_reset::codex_failure_text(&parsed).is_some() {
                // Codex reports a failed turn as `error` / `turn.failed` frames.
                *last_inband_error = Some(parsed);
            } else if parsed.get("type").and_then(|v| v.as_str()) == Some("turn.completed") {
                *last_inband_error = None;
            }
        }

        // The host's stdout reader does the same with each frame: the context reading, the
        // compaction boundary that lets this pane's memory reinjection go out, and the memory
        // card parked for that boundary written after the summary that follows it. Without them a
        // container's compaction never leaves "compacting", and the card is written 30 s late
        // with a "no compaction boundary came" warning.
        let mut release_parked_notices = false;
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
            crate::backend::context_reading::observe_claude_frame(block_id, &parsed);
            if parsed.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary") {
                if let Some(uuid) = parsed.get("uuid").and_then(|v| v.as_str()) {
                    crate::backend::memory::delivery_state::record_compaction_boundary(block_id, uuid);
                }
                *parked_notices_due = true;
            } else if *parked_notices_due && !matches!(parsed.get("type").and_then(|v| v.as_str()), Some("user") | Some("system")) {
                release_parked_notices = true;
                *parked_notices_due = false;
            }
        }

        if let Some(ref broker) = broker {
            if release_parked_notices {
                for notice in crate::backend::memory::delivery_state::take_parked_notices(block_id) {
                    crate::backend::transcript::append::append_output_line(broker, block_id, &notice, filestore.as_ref(), global_output_zone);
                }
            }
            crate::backend::transcript::append::append_output_line(broker, block_id, trimmed, filestore.as_ref(), global_output_zone);
        }
    }
}
