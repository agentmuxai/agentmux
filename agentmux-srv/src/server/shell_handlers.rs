// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_SHELL_EXEC, COMMAND_SHELL_STOP, COMMAND_SHELL_STATUS,
    CommandShellExecData, ShellExecResult, CommandShellStopData, CommandShellStatusData,
};
use crate::backend::base::{expand_home_dir_safe, msys_to_windows_path};

use super::AppState;

pub fn register_shell_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // shellstop → tree-kill a running persistent shell node (Phase 3). Invoked
    // by the UI stop button on a running PersistentShellBlock. The runner then
    // publishes a `stopped` exit event.
    let shell_sessions_stop = state.shell_sessions.clone();
    engine.register_handler(
        COMMAND_SHELL_STOP,
        Box::new(move |data, _ctx| {
            let registry = shell_sessions_stop.clone();
            Box::pin(async move {
                let req: CommandShellStopData = serde_json::from_value(data)
                    .map_err(|e| format!("shellstop: {e}"))?;
                let stopped = registry.stop(&req.shell_id);
                tracing::info!(shell_id = %req.shell_id, stopped, "shellstop");
                Ok(Some(serde_json::json!({ "stopped": stopped })))
            })
        }),
    );

    // shellstatus → query a shell's current running state. Used by
    // useShellNodeStream to resolve the TRUE status of a shell whose
    // `shell_node_create` event is being replayed on pane mount/reconnect
    // (persist:64 ring), instead of assuming "running" — the frontend can't
    // otherwise tell a live spawn apart from a replay of an already-long-
    // exited shell, which was flashing stale rows in the Activity Dock.
    // See docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md.
    let shell_sessions_status = state.shell_sessions.clone();
    engine.register_handler(
        COMMAND_SHELL_STATUS,
        Box::new(move |data, _ctx| {
            let registry = shell_sessions_status.clone();
            Box::pin(async move {
                let req: CommandShellStatusData = serde_json::from_value(data)
                    .map_err(|e| format!("shellstatus: {e}"))?;
                let s = registry.get_status(&req.shell_id);
                Ok(Some(serde_json::json!({
                    "running": s.running,
                    "exit_code": s.exit_code,
                    "line_count": s.line_count,
                })))
            })
        }),
    );

    // shellexec → run a shell command and return output.
    // Invoked by the `!cmd` prefix in the agent pane composer.
    //
    // Host agents:      sh -c <cmd> in the agent's working directory (all platforms;
    //                   Git Bash provides sh on Windows).
    // Container agents: docker exec <container> sh -c <cmd> via bollard.
    //                   The host cmd:cwd is not valid inside the container — the
    //                   command runs in the container's own working directory.
    let wstore_se = state.wstore.clone();
    let container_manager = state.container_manager.clone();
    engine.register_handler(
        COMMAND_SHELL_EXEC,
        Box::new(move |data, _ctx| {
            let wstore = wstore_se.clone();
            let cm_opt = container_manager.clone();
            Box::pin(async move {
                let cmd: CommandShellExecData = serde_json::from_value(data)
                    .map_err(|e| format!("shellexec: {e}"))?;
                // Log block_id at info; keep the command at debug so secrets
                // passed as CLI args (API tokens, passwords) don't land in
                // ~/.agentmux/logs/ in plaintext.
                tracing::info!(block_id = %cmd.blockid, "ShellExec");
                tracing::debug!(command = %cmd.command, "ShellExec command");

                let block: crate::backend::obj::Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("shellexec: load block: {e}"))?
                    .ok_or_else(|| format!("shellexec: block {} not found", cmd.blockid))?;
                let agent_mode = crate::backend::obj::meta_get_string(
                    &block.meta, "agentMode", "host",
                );

                // 300s process timeout matches the frontend's RPC timeout so
                // the client sees a clean error rather than a silent EC-TIME.
                const TIMEOUT_SECS: u64 = 300;
                // 1 MB cap per stream — bounds memory for runaway commands
                // (`! yes`, `! dd if=/dev/zero`).
                const MAX_OUTPUT: u64 = 1_000_000;

                // Shared output formatter: if we accumulated more than MAX_OUTPUT
                // bytes, the stream was truncated — append a notice.
                fn format_output(bytes: Vec<u8>, cap: u64) -> String {
                    if bytes.len() > cap as usize {
                        let s = String::from_utf8_lossy(&bytes[..cap as usize]);
                        format!("{s}…[output capped at {cap} bytes]")
                    } else {
                        String::from_utf8_lossy(&bytes).into_owned()
                    }
                }

                // ── Container agents ─────────────────────────────────────────
                if agent_mode == "container" {
                    let cm = cm_opt.get().await.ok_or_else(|| {
                        "shellexec: Docker not available on this host; \
                         cannot exec in container agent".to_string()
                    })?;

                    let agent_id = crate::backend::obj::meta_get_string(
                        &block.meta, "agentId", "",
                    );
                    if agent_id.is_empty() {
                        return Err("shellexec: container agent missing agentId in block meta".to_string());
                    }
                    let container_name =
                        crate::backend::container::container_name_for_slug(&agent_id);

                    let session = cm.exec(
                        &container_name,
                        &["sh".to_string(), "-c".to_string(), cmd.command.clone()],
                        None, // container's own working directory (host path invalid inside)
                        &[],  // no extra env vars for !cmd
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
                        // not just cmd.command — a bare command like "python" or "sh"
                        // would over-match the agent's own running processes.
                        let kill_pattern = format!("sh -c {}", cmd.command);
                        cm.signal_exec_process(&container_name, &kill_pattern, true).await.ok();
                        return Err(format!("shellexec: timed out after {TIMEOUT_SECS}s"));
                    }
                    timeout_result.unwrap()?;

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
                    return Ok(Some(
                        serde_json::to_value(result).map_err(|e| format!("shellexec: {e}"))?
                    ));
                }

                // ── Host agents ───────────────────────────────────────────────
                let cwd: Option<std::path::PathBuf> = if cmd.working_dir.is_empty() {
                    None
                } else {
                    // Convert MSYS/Git-Bash paths like /c/Users/... to C:\Users\...
                    // before expansion. Without this, canonicalize() fails on Windows
                    // with os error 267 (ERROR_DIRECTORY) — same fix applied to the
                    // persistent shell node in d89c1458.
                    let native = msys_to_windows_path(&cmd.working_dir);
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
                    Some(canonical)
                };

                // Use sh -c on all platforms: agents run in a bash environment
                // (Git Bash on Windows, sh on Unix) so Unix commands like ls/pwd/grep
                // work consistently. cmd /C would exit 1 for any non-Windows command.
                let mut proc = {
                    let mut c = tokio::process::Command::new("sh");
                    c.args(["-c", &cmd.command]);
                    // CREATE_NO_WINDOW: console-flash suppression — on Windows
                    // this `sh` is Git Bash (console-subsystem), and srv is
                    // launched windowless, so without the flag every shellexec
                    // pops a console window. This is the highest-frequency
                    // console-flash site (fires on every MCP Shell tool call).
                    // See agentmux-common/src/cli.rs.
                    #[cfg(windows)]
                    {
                        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                        c.creation_flags(CREATE_NO_WINDOW);
                    }
                    c
                };
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
                    use std::os::unix::process::CommandExt as _;
                    proc.process_group(0);
                }
                if let Some(ref dir) = cwd {
                    proc.current_dir(dir);
                }

                let mut child = proc.spawn()
                    .map_err(|e| format!("shellexec: spawn failed: {e}"))?;

                // Capture the PID before taking stdout/stderr (id() requires
                // the Child to still have its stdio handles on some platforms).
                #[cfg(unix)]
                let child_pgid = child.id().map(|id| id as libc::pid_t);

                let mut stdout_pipe = child.stdout.take().expect("stdout piped");
                let mut stderr_pipe = child.stderr.take().expect("stderr piped");

                let mut stdout_buf: Vec<u8> = Vec::new();
                let mut stderr_buf: Vec<u8> = Vec::new();

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
                let timeout_result = tokio::time::timeout(
                    std::time::Duration::from_secs(TIMEOUT_SECS),
                    async {
                        let (sout, serr, wait) = tokio::join!(
                            // stdout branch: cap-read then drain
                            async {
                                let mut take = (&mut stdout_pipe).take(MAX_OUTPUT + 1);
                                take.read_to_end(&mut stdout_buf).await
                                    .map_err(|e| format!("shellexec: stdout read: {e}"))?;
                                drop(take); // release &mut borrow so stdout_pipe is usable
                                let mut sink = tokio::io::sink();
                                tokio::io::copy(&mut stdout_pipe, &mut sink).await
                                    .map_err(|e| format!("shellexec: drain stdout: {e}"))?;
                                Ok::<(), String>(())
                            },
                            // stderr branch: cap-read then drain
                            async {
                                let mut take = (&mut stderr_pipe).take(MAX_OUTPUT + 1);
                                take.read_to_end(&mut stderr_buf).await
                                    .map_err(|e| format!("shellexec: stderr read: {e}"))?;
                                drop(take);
                                let mut sink = tokio::io::sink();
                                tokio::io::copy(&mut stderr_pipe, &mut sink).await
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
                .await;

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
                Ok(Some(serde_json::to_value(result).map_err(|e| format!("shellexec: {e}"))?))
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(data["running"], serde_json::json!(true));
        assert_eq!(data["line_count"], serde_json::json!(3));
        assert!(data.get("exit_code").map_or(true, |v| v.is_null()));
    }

    #[tokio::test]
    async fn shellstatus_reports_exited_shell_with_exit_code() {
        let state = crate::server::tests::test_state();
        seed_status(&state, "sh-exited", false, Some(0));
        let data = call_shellstatus(&state, "sh-exited").await;
        assert_eq!(data["running"], serde_json::json!(false));
        assert_eq!(data["exit_code"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn shellstatus_reports_exited_err_shell() {
        let state = crate::server::tests::test_state();
        seed_status(&state, "sh-failed", false, Some(1));
        let data = call_shellstatus(&state, "sh-failed").await;
        assert_eq!(data["running"], serde_json::json!(false));
        assert_eq!(data["exit_code"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn shellstatus_unknown_id_reports_not_running_no_exit_code() {
        let state = crate::server::tests::test_state();
        let data = call_shellstatus(&state, "sh-never-existed").await;
        assert_eq!(data["running"], serde_json::json!(false));
        assert!(data.get("exit_code").map_or(true, |v| v.is_null()));
    }
}
