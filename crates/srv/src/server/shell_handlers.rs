// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_SHELL_EXEC, COMMAND_SHELL_STOP, COMMAND_SHELL_STATUS,
    CommandShellExecData, ShellExecResult, CommandShellStopData, CommandShellStatusData,
    ShellStopResult, ShellStatusResult,
};
use crate::backend::base::{expand_home_dir_safe, msys_to_windows_path};

use super::AppState;

/// 300s process timeout matches the frontend's RPC timeout so
/// the client sees a clean error rather than a silent EC-TIME.
const TIMEOUT_SECS: u64 = 300;
/// 1 MB cap per stream — bounds memory for runaway commands
/// (`! yes`, `! dd if=/dev/zero`).
const MAX_OUTPUT: u64 = 1_000_000;

/// Shared output formatter: if we accumulated more than MAX_OUTPUT
/// bytes, the stream was truncated — append a notice.
fn format_output(bytes: Vec<u8>, cap: u64) -> String {
    if bytes.len() > cap as usize {
        let s = String::from_utf8_lossy(&bytes[..cap as usize]);
        format!("{s}…[output capped at {cap} bytes]")
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// shellstop → tree-kill a running persistent shell node (Phase 3). Invoked
/// by the UI stop button on a running PersistentShellBlock. The runner then
/// publishes a `stopped` exit event.
pub(crate) fn shell_stop(state: &AppState, req: CommandShellStopData) -> Result<ShellStopResult, String> {
    let stopped = state.shell_sessions.stop(&req.shell_id);
    tracing::info!(shell_id = %req.shell_id, stopped, "shellstop");
    Ok(ShellStopResult { stopped })
}

/// shellstatus → query a shell's current running state. Used by
/// useShellNodeStream to resolve the TRUE status of a shell whose
/// `shell_node_create` event is being replayed on pane mount/reconnect
/// (persist:64 ring), instead of assuming "running" — the frontend can't
/// otherwise tell a live spawn apart from a replay of an already-long-
/// exited shell, which was flashing stale rows in the Activity Dock.
/// See docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md.
///
/// Reports `known: false` (rather than collapsing into `running: false`,
/// like the MCP-facing HTTP route does) when no registry entry exists yet
/// — the shell may simply not have finished spawning/registering (see
/// `get_status_if_known`'s doc comment for the exact race this closes:
/// reagent P1 on PR #2770, a genuinely live shell misreported as already
/// exited). The frontend treats `known: false` as "don't correct."
pub(crate) fn shell_status(state: &AppState, req: CommandShellStatusData) -> Result<ShellStatusResult, String> {
    Ok(match state.shell_sessions.get_status_if_known(&req.shell_id) {
        Some(s) => ShellStatusResult {
            known: true,
            running: s.running,
            exit_code: s.exit_code,
            line_count: s.line_count,
        },
        // `known: false` with running/exit_code/line_count zeroed.
        // Not `Default::default()`: the zeros here are filler for
        // "we have nothing to say", and spelling them out keeps
        // that distinct from a shell that genuinely exited 0.
        None => ShellStatusResult {
            known: false,
            running: false,
            exit_code: None,
            line_count: 0,
        },
    })
}

/// Runs `command` in the agent's container and collects both streams, capped
/// at `MAX_OUTPUT + 1` bytes each. Returns the exec id (for the exit code) and
/// the two buffers.
async fn container_exec_collect(
    cm: &crate::backend::container::ContainerManager,
    container_name: &str,
    command: &str,
) -> Result<(String, Vec<u8>, Vec<u8>), String> {
    let session = cm.exec(
        container_name,
        &["sh".to_string(), "-c".to_string(), command.to_string()],
        None, // container's own working directory (host path invalid inside)
        &[],  // no extra env vars for !cmd
        false, // no stdin — `!cmd` output-only; see ContainerManager::exec
    ).await.map_err(|e| format!("shellexec: container exec failed: {e}"))?;

    let exec_id = session.exec_id.clone();
    // No stdin needed for !cmd — drop to signal EOF immediately.
    drop(session.input);

    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();

    use futures_util::StreamExt as _;
    use bollard::container::LogOutput;

    let timeout_result = tokio::time::timeout(
        std::time::Duration::from_secs(TIMEOUT_SECS),
        async {
            let mut output = std::pin::pin!(session.output);
            while let Some(item) = output.next().await {
                match item {
                    Err(e) => return Err(format!(
                        "shellexec: container output read: {e}"
                    )),
                    Ok(log) => match log {
                        LogOutput::StdOut { message } => {
                            // Accumulate MAX_OUTPUT+1 bytes (the +1 sentinel lets
                            // format_output distinguish "exactly at cap" from
                            // "truncated", matching the host branch's take logic).
                            let cap = (MAX_OUTPUT + 1) as usize;
                            let take = cap
                                .saturating_sub(stdout_buf.len())
                                .min(message.len());
                            stdout_buf.extend_from_slice(&message[..take]);
                        }
                        LogOutput::StdErr { message } => {
                            let cap = (MAX_OUTPUT + 1) as usize;
                            let take = cap
                                .saturating_sub(stderr_buf.len())
                                .min(message.len());
                            stderr_buf.extend_from_slice(&message[..take]);
                        }
                        _ => {} // StdIn / Console frames not relevant here
                    }
                }
            }
            Ok(())
        }
    ).await;

    if timeout_result.is_err() {
        // Kill the in-container process so it doesn't linger after
        // timeout. Mirrors the host branch's process-group SIGKILL.
        // signal_exec_process runs `pkill -KILL -f <pattern>` inside
        // the container; fire-and-forget (non-match is not an error).
        //
        // Pattern is "sh -c <command>" (the full cmdline we spawned),
        // not just the command — a bare command like "python" or "sh"
        // would over-match the agent's own running processes.
        let kill_pattern = format!("sh -c {command}");
        cm.signal_exec_process(container_name, &kill_pattern, true).await.ok();
        return Err(format!("shellexec: timed out after {TIMEOUT_SECS}s"));
    }
    timeout_result.unwrap()?;
    Ok((exec_id, stdout_buf, stderr_buf))
}

/// ── Container agents ─────────────────────────────────────────
async fn shell_exec_container(
    state: &AppState,
    cmd: &CommandShellExecData,
    block: &crate::backend::obj::Block,
) -> Result<ShellExecResult, String> {
    let cm = state.container_manager.get_or_explain().await.map_err(|e| format!("shellexec: {e}"))?;

    let agent_id = crate::backend::obj::meta_get_string(
        &block.meta, "agentId", "",
    );
    if agent_id.is_empty() {
        return Err("shellexec: container agent missing agentId in block meta".to_string());
    }
    let container_name =
        crate::backend::container::container_name_for_slug(&agent_id);

    let (exec_id, stdout_buf, stderr_buf) = container_exec_collect(&cm, &container_name, &cmd.command).await?;

    // The output stream closing does not carry the exit status —
    // retrieve it separately via inspect_exec.
    let exit_code = cm.inspect_exec(&exec_id).await
        .ok()
        .flatten()
        .unwrap_or(1) as i32;

    tracing::debug!(
        block_id = %cmd.blockid,
        container = %container_name,
        exit_code,
        "ShellExec container done",
    );

    let result = ShellExecResult {
        exit_code,
        stdout: format_output(stdout_buf, MAX_OUTPUT),
        stderr: format_output(stderr_buf, MAX_OUTPUT),
    };
    Ok(result)
}

/// The working directory a host `!cmd` runs in: absolute, symlinks resolved.
fn host_working_dir(working_dir: &str) -> Result<Option<std::path::PathBuf>, String> {
    if working_dir.is_empty() {
        return Ok(None);
    }
    // Convert MSYS/Git-Bash paths like /c/Users/... to C:\Users\...
    // before expansion. Without this, canonicalize() fails on Windows
    // with os error 267 (ERROR_DIRECTORY) — same fix applied to the
    // persistent shell node in d89c1458.
    let native = msys_to_windows_path(working_dir);
    let expanded = expand_home_dir_safe(&native);
    // Reject non-absolute paths to prevent relative traversal
    // (e.g. "../etc") from resolving against the sidecar's cwd.
    // expand_home_dir_safe turns "~" into an absolute home path;
    // anything still relative after expansion is suspicious.
    if !expanded.is_absolute() {
        return Err(format!(
            "shellexec: working_dir must be absolute, got: {}",
            expanded.display()
        ));
    }
    // Canonicalize to resolve all symlinks before handing the
    // path to the shell. Without this a symlinked working_dir
    // would silently run the shell in the symlink target
    // (potentially outside the agent workspace), matching the
    // symlink-escape protection in writeagentconfig.
    let canonical = expanded.canonicalize()
        .map_err(|e| format!(
            "shellexec: cannot resolve working directory '{}': {e}",
            expanded.display()
        ))?;
    Ok(Some(canonical))
}

/// The `sh -c <command>` process for a host `!cmd`, with its stdio, kill and
/// process-group settings.
fn host_command(shell: std::path::PathBuf, command: &str, cwd: Option<&std::path::Path>) -> tokio::process::Command {
    let mut proc = {
        let mut c = tokio::process::Command::new(&shell);
        c.args(["-c", command]);
        // shellexec runs arbitrary agent-supplied commands and, per
        // the note below, fires on every MCP Shell tool call — so a
        // build started this way would otherwise adopt this
        // instance's channel and data dir. Same class as
        // shell_node.rs; see backend::pane_env.
        crate::backend::pane_env::sanitize_process_command(&mut c);
        // CREATE_NO_WINDOW: console-flash suppression — on Windows
        // this `sh` is Git Bash (console-subsystem), and srv is
        // launched windowless, so without the flag every shellexec
        // pops a console window. This is the highest-frequency
        // console-flash site (fires on every MCP Shell tool call).
        // See crates/common/src/cli.rs.
        #[cfg(windows)]
        {
            c.no_window();
        }
        c
    };
    // stdin MUST NOT be inherited. srv's own stdin is the
    // launcher's lifeline pipe, with a thread parked in a blocking
    // read on it (bootstrap::install_shutdown_handlers). Git Bash's
    // MSYS runtime queries its inherited stdin at startup, and on
    // Windows that query waits behind srv's pending read — forever,
    // so every `!cmd` hung until the 300s timeout, `!echo hi`
    // included. A one-shot command has no input anyway; null gives
    // `!cat` an EOF instead of a read on srv's lifeline.
    proc.stdin(std::process::Stdio::null());
    proc.stdout(std::process::Stdio::piped());
    proc.stderr(std::process::Stdio::piped());
    // kill_on_drop: when the timeout fires the Child future is
    // dropped; without this flag the OS process keeps running
    // (e.g. `! sleep 1000` would linger indefinitely).
    proc.kill_on_drop(true);
    // Unix: put the shell in its own process group so that compound
    // commands (`! a | b`, `! foo &`) are in the same group and can
    // all be killed at once on timeout.  kill_on_drop only kills the
    // direct sh child; without process_group(0), grandchildren
    // get reparented to init and outlive the timeout.
    #[cfg(unix)]
    {
        proc.process_group(0);
    }
    if let Some(dir) = cwd {
        proc.current_dir(dir);
    }
    proc
}

/// Reads both streams (capped) and waits for the child, all under the
/// timeout. The outer `Err` is the timeout.
async fn collect_host_output(
    child: &mut tokio::process::Child,
    stdout_pipe: &mut tokio::process::ChildStdout,
    stderr_pipe: &mut tokio::process::ChildStderr,
    stdout_buf: &mut Vec<u8>,
    stderr_buf: &mut Vec<u8>,
) -> Result<Result<std::process::ExitStatus, String>, tokio::time::error::Elapsed> {
    use tokio::io::AsyncReadExt as _;
    // Each stream's capped-read and drain run in the SAME concurrent
    // branch.  A two-phase approach (cap-read all, then drain all)
    // deadlocks: when stdout exceeds the cap, its read_to_end returns
    // but the process is now blocked on write() to a full pipe, so it
    // never writes to stderr, so stderr's read_to_end never gets EOF.
    // Both reads block, the drain phase is never reached, and the whole
    // handler hangs for the full 300 s timeout.
    //
    // Solution: read MAX_OUTPUT+1 sentinel bytes per stream (≤ MAX_OUTPUT
    // → complete; MAX_OUTPUT+1 → truncated), then immediately drain the
    // remainder.  stdout and stderr pipelines run concurrently with each
    // other, and with child.wait().
    tokio::time::timeout(
        std::time::Duration::from_secs(TIMEOUT_SECS),
        async {
            let (sout, serr, wait) = tokio::join!(
                // stdout branch: cap-read then drain
                async {
                    let mut take = (&mut *stdout_pipe).take(MAX_OUTPUT + 1);
                    take.read_to_end(stdout_buf).await
                        .map_err(|e| format!("shellexec: stdout read: {e}"))?;
                    drop(take); // release &mut borrow so stdout_pipe is usable
                    let mut sink = tokio::io::sink();
                    tokio::io::copy(&mut *stdout_pipe, &mut sink).await
                        .map_err(|e| format!("shellexec: drain stdout: {e}"))?;
                    Ok::<(), String>(())
                },
                // stderr branch: cap-read then drain
                async {
                    let mut take = (&mut *stderr_pipe).take(MAX_OUTPUT + 1);
                    take.read_to_end(stderr_buf).await
                        .map_err(|e| format!("shellexec: stderr read: {e}"))?;
                    drop(take);
                    let mut sink = tokio::io::sink();
                    tokio::io::copy(&mut *stderr_pipe, &mut sink).await
                        .map_err(|e| format!("shellexec: drain stderr: {e}"))?;
                    Ok::<(), String>(())
                },
                child.wait(),
            );
            sout?;
            serr?;
            wait.map_err(|e| format!("shellexec: wait: {e}"))
        }
    )
    .await
}

/// ── Host agents ───────────────────────────────────────────────
async fn shell_exec_host(cmd: &CommandShellExecData) -> Result<ShellExecResult, String> {
    let cwd = host_working_dir(&cmd.working_dir)?;

    // Use sh -c on all platforms: agents run in a bash environment
    // (Git Bash on Windows, sh on Unix) so Unix commands like ls/pwd/grep
    // work consistently. cmd /C would exit 1 for any non-Windows command.
    // On Windows `sh` is located rather than assumed to be on PATH —
    // an Explorer-launched build's PATH has only Git\cmd, which has
    // no sh.exe. See backend::posix_shell.
    let shell = crate::backend::posix_shell::resolve_posix_shell()
        .map_err(|e| format!("shellexec: {e}"))?;
    let mut proc = host_command(shell.clone(), &cmd.command, cwd.as_deref());

    // Joins the agent's tracker before it runs: a `!cmd` that
    // backgrounds a child ends with the agent (agent_teardown spec §6.5).
    let mut child = crate::backend::process_tracker::registry::spawn_tracked(&cmd.blockid, &mut proc, crate::backend::process_tracker::registry::Join::Adopted)
        .map_err(|e| format!("shellexec: spawn failed ({}): {e}", shell.display()))?;

    // Capture the PID before taking stdout/stderr (id() requires
    // the Child to still have its stdio handles on some platforms).
    #[cfg(unix)]
    let child_pgid = child.id().map(|id| id as libc::pid_t);

    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");

    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();

    let timeout_result = collect_host_output(&mut child, &mut stdout_pipe, &mut stderr_pipe, &mut stdout_buf, &mut stderr_buf).await;

    // On timeout: kill_on_drop kills the direct sh child, but
    // compound commands (`! a | b`, `! foo &`) fork grandchildren
    // in the same process group.  Kill the whole group so they don't
    // linger.  (Unix only; on Windows the job-object approach is a
    // separate follow-up since tokio doesn't expose it yet.)
    #[cfg(unix)]
    if timeout_result.is_err() {
        if let Some(pgid) = child_pgid {
            // SAFETY: kill() is async-signal-safe; negative pid
            // addresses the process group with id=pgid.
            unsafe { libc::kill(-pgid, libc::SIGKILL); }
        }
    }

    let status = timeout_result
        .map_err(|_| format!("shellexec: timed out after {TIMEOUT_SECS}s"))??;

    let result = ShellExecResult {
        exit_code: status.code().unwrap_or(1),
        stdout: format_output(stdout_buf, MAX_OUTPUT),
        stderr: format_output(stderr_buf, MAX_OUTPUT),
    };
    Ok(result)
}

/// shellexec → run a shell command and return output.
/// Invoked by the `!cmd` prefix in the agent pane composer.
///
/// Host agents:      sh -c <cmd> in the agent's working directory (all platforms;
///                   Git Bash provides sh on Windows, located by
///                   backend::posix_shell).
/// Container agents: docker exec <container> sh -c <cmd> via bollard.
///                   The host cmd:cwd is not valid inside the container — the
///                   command runs in the container's own working directory.
pub(crate) async fn shell_exec(state: &AppState, cmd: CommandShellExecData) -> Result<ShellExecResult, String> {
    // Log block_id at info; keep the command at debug so secrets
    // passed as CLI args (API tokens, passwords) don't land in
    // ~/.agentmux/logs/ in plaintext.
    tracing::info!(block_id = %cmd.blockid, "ShellExec");
    tracing::debug!(command = %cmd.command, "ShellExec command");

    let block: crate::backend::obj::Block = state.mstore
        .get(&cmd.blockid)
        .map_err(|e| format!("shellexec: load block: {e}"))?
        .ok_or_else(|| format!("shellexec: block {} not found", cmd.blockid))?;
    let agent_mode = crate::backend::obj::meta_get_string(
        &block.meta, "agentMode", "host",
    );

    if crate::backend::runtime_adapter::RuntimeKind::of_agent_mode(&agent_mode).is_container() {
        return shell_exec_container(state, &cmd, &block).await;
    }
    shell_exec_host(&cmd).await
}

pub fn register_shell_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_SHELL_STOP, move |req: CommandShellStopData, _ctx| {
        let s = s.clone();
        async move { shell_stop(&s, req) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_SHELL_STATUS, move |req: CommandShellStatusData, _ctx| {
        let s = s.clone();
        async move { shell_status(&s, req) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_SHELL_EXEC, move |cmd: CommandShellExecData, _ctx| {
        let s = s.clone();
        async move { shell_exec(&s, cmd).await }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All three shell commands record their request and response types.
    ///
    /// `shellstop` and `shellstatus` previously answered inline
    /// `json!({..})` literals, so the registry would have recorded `Value` for
    /// both -- which is the shape of drift this migration exists to close.
    #[tokio::test]
    async fn register_typed_records_every_shell_command() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register_shell_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };

        for (cmd, req, resp) in [
            (COMMAND_SHELL_EXEC, "CommandShellExecData", "ShellExecResult"),
            (COMMAND_SHELL_STOP, "CommandShellStopData", "ShellStopResult"),
            (COMMAND_SHELL_STATUS, "CommandShellStatusData", "ShellStatusResult"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert_eq!(row["responseName"], resp, "{cmd} response");
        }
    }

    /// `exit_code` is always PRESENT and null, never omitted.
    ///
    /// The hand-written stub typed it `exit_code?: number`, but both arms of
    /// the handler write the key unconditionally -- the unknown arm literally
    /// wrote `"exit_code": null`. The generated type says `number | null`,
    /// which is the honest shape, and this is the half of that claim TypeScript
    /// cannot check.
    ///
    /// It matters here more than usual: `known: false` must stay
    /// distinguishable from `running: false` (reagent P1 on #2770 -- collapsing
    /// them misreported a live, still-registering shell as failed for its
    /// entire run), and a caller reaching for `exit_code === undefined` as a
    /// proxy for "unknown" would get the wrong answer either way.
    #[test]
    fn shellstatus_always_sends_exit_code_even_when_null() {
        let unknown = ShellStatusResult {
            known: false,
            running: false,
            exit_code: None,
            line_count: 0,
        };
        let v = serde_json::to_value(&unknown).unwrap();
        assert!(
            v.get("exit_code").is_some(),
            "exit_code must be present-and-null, not omitted: {v}",
        );
        assert!(v["exit_code"].is_null());
        assert_eq!(v["known"], false);

        let exited = ShellStatusResult {
            known: true,
            running: false,
            exit_code: Some(0),
            line_count: 12,
        };
        let v = serde_json::to_value(&exited).unwrap();
        assert_eq!(v["exit_code"], 0, "a real exit 0 is not the same as unknown");
        assert_eq!(v["known"], true);
    }

    use crate::backend::rpc_types::RpcMessage;
    use crate::backend::shell_node::ShellStatusInfo;
    use parking_lot::Mutex;

    // Seed a shell status directly (mirrors shell_node.rs's own
    // register_running_for_block/register_exited test helpers) — avoids
    // spawning a real child process just to exercise the RPC handler's
    // JSON shape.
    fn seed_status(state: &AppState, shell_id: &str, running: bool, exit_code: Option<i32>) {
        let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
        let status = Arc::new(Mutex::new(ShellStatusInfo {
            running,
            exit_code,
            line_count: 3,
            ..Default::default()
        }));
        state.shell_sessions.register_full(shell_id.to_string(), tx, None, status);
    }

    async fn call_shellstatus(state: &AppState, shell_id: &str) -> serde_json::Value {
        let (engine, mut output_rx) = WshRpcEngine::new();
        register_shell_handlers(&engine, state);
        engine.handle_message(RpcMessage {
            command: "shellstatus".to_string(),
            reqid: "req-shellstatus".to_string(),
            data: Some(serde_json::json!({ "shell_id": shell_id })),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(1), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        resp.data.unwrap()
    }

    #[tokio::test]
    async fn shellstatus_reports_running_shell() {
        let state = crate::server::tests::test_state();
        seed_status(&state, "sh-running", true, None);
        let data = call_shellstatus(&state, "sh-running").await;
        assert_eq!(data["known"], serde_json::json!(true));
        assert_eq!(data["running"], serde_json::json!(true));
        assert_eq!(data["line_count"], serde_json::json!(3));
        assert!(data.get("exit_code").map_or(true, |v| v.is_null()));
    }

    #[tokio::test]
    async fn shellstatus_reports_exited_shell_with_exit_code() {
        let state = crate::server::tests::test_state();
        seed_status(&state, "sh-exited", false, Some(0));
        let data = call_shellstatus(&state, "sh-exited").await;
        assert_eq!(data["known"], serde_json::json!(true));
        assert_eq!(data["running"], serde_json::json!(false));
        assert_eq!(data["exit_code"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn shellstatus_reports_exited_err_shell() {
        let state = crate::server::tests::test_state();
        seed_status(&state, "sh-failed", false, Some(1));
        let data = call_shellstatus(&state, "sh-failed").await;
        assert_eq!(data["known"], serde_json::json!(true));
        assert_eq!(data["running"], serde_json::json!(false));
        assert_eq!(data["exit_code"], serde_json::json!(1));
    }

    // Reagent P1 on PR #2770: a shell with no registry entry at all — either
    // a genuinely unknown id, OR (the actual race that motivated this)
    // `shell_node_create` was published but the runner hasn't reached
    // `register_full` yet (still spawning the child process). Both look
    // identical to this handler, and BOTH must report `known: false`, not
    // the same shape as a genuinely exited shell — otherwise a live shell
    // caught in that race gets misreported as failed for its entire run
    // (ShellChunkAppend never restores a status once set).
    #[tokio::test]
    async fn shellstatus_unknown_id_reports_known_false() {
        let state = crate::server::tests::test_state();
        let data = call_shellstatus(&state, "sh-never-existed").await;
        assert_eq!(data["known"], serde_json::json!(false));
        assert_eq!(data["running"], serde_json::json!(false));
        assert!(data.get("exit_code").map_or(true, |v| v.is_null()));
    }

    fn insert_block(state: &AppState, block_id: &str, meta: &[(&str, &str)]) {
        let mut m = crate::backend::obj::MetaMapType::new();
        for (k, v) in meta {
            m.insert(k.to_string(), serde_json::json!(v));
        }
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: m,
            subblockids: None,
        };
        state.mstore.insert(&mut block).unwrap();
    }

    fn exec_req(block_id: &str, command: &str, working_dir: &str) -> CommandShellExecData {
        serde_json::from_value(serde_json::json!({ "blockid": block_id, "command": command, "working_dir": working_dir })).unwrap()
    }

    /// The handlers run without the engine: stop answers whether a shell was
    /// stopped, and status keeps `known: false` apart from `running: false`.
    #[tokio::test]
    async fn shell_stop_and_status_answer_directly() {
        let state = crate::server::tests::test_state();
        assert!(!shell_stop(&state, serde_json::from_value(serde_json::json!({ "shell_id": "never" })).unwrap()).unwrap().stopped);

        seed_status(&state, "sh-direct", false, Some(2));
        let known = shell_status(&state, serde_json::from_value(serde_json::json!({ "shell_id": "sh-direct" })).unwrap()).unwrap();
        assert!(known.known && !known.running);
        assert_eq!(known.exit_code, Some(2));
        let unknown = shell_status(&state, serde_json::from_value(serde_json::json!({ "shell_id": "sh-unknown" })).unwrap()).unwrap();
        assert!(!unknown.known && unknown.exit_code.is_none() && unknown.line_count == 0);
    }

    #[tokio::test]
    async fn shell_exec_of_a_missing_block_is_an_error() {
        let state = crate::server::tests::test_state();
        let err = shell_exec(&state, exec_req("no-such-block", "echo hi", "")).await.unwrap_err();
        assert_eq!(err, "shellexec: block no-such-block not found");
    }

    #[tokio::test]
    async fn shell_exec_refuses_a_relative_working_directory() {
        let state = crate::server::tests::test_state();
        insert_block(&state, "exec-rel", &[]);
        let err = shell_exec(&state, exec_req("exec-rel", "echo hi", "../etc")).await.unwrap_err();
        assert!(err.starts_with("shellexec: working_dir must be absolute"), "{err}");
    }

    /// A container agent with no agentId in its block meta is refused before any exec.
    #[tokio::test]
    async fn shell_exec_of_a_container_block_without_an_agent_id_is_refused() {
        let state = crate::server::tests::test_state();
        insert_block(&state, "exec-container", &[("agentMode", "container")]);
        let err = shell_exec(&state, exec_req("exec-container", "echo hi", "")).await.unwrap_err();
        // Either there is no container runtime on this host, or the agentId check fires.
        assert!(err.starts_with("shellexec:"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_exec_runs_a_host_command_and_reports_streams_and_exit_code() {
        let state = crate::server::tests::test_state();
        insert_block(&state, "exec-host", &[]);
        let dir = tempfile::tempdir().unwrap();
        let wd = dir.path().canonicalize().unwrap();
        let out = shell_exec(&state, exec_req("exec-host", "pwd; echo err >&2; exit 3", wd.to_str().unwrap())).await.unwrap();
        assert_eq!(out.exit_code, 3);
        assert_eq!(out.stdout.trim(), wd.to_string_lossy());
        assert_eq!(out.stderr.trim(), "err");
    }

    /// Output past the cap is cut and says so, without hanging on the full pipe.
    #[cfg(unix)]
    #[tokio::test]
    async fn shell_exec_caps_runaway_output() {
        let state = crate::server::tests::test_state();
        insert_block(&state, "exec-cap", &[]);
        let out = shell_exec(&state, exec_req("exec-cap", "head -c 1500000 /dev/zero | tr '\\0' a", "")).await.unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.ends_with("…[output capped at 1000000 bytes]"), "tail: {:?}", &out.stdout[out.stdout.len().saturating_sub(40)..]);
    }
}
