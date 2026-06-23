// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_SHELL_EXEC, COMMAND_SHELL_STOP,
    CommandShellExecData, ShellExecResult, CommandShellStopData,
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

    // shellexec → run a shell command in the agent's working directory and return output.
    // Invoked by the `!cmd` prefix in the agent pane composer.
    let wstore_se = state.wstore.clone();
    engine.register_handler(
        COMMAND_SHELL_EXEC,
        Box::new(move |data, _ctx| {
            let wstore = wstore_se.clone();
            Box::pin(async move {
                let cmd: CommandShellExecData = serde_json::from_value(data)
                    .map_err(|e| format!("shellexec: {e}"))?;
                // Log block_id at info; keep the command at debug so secrets
                // passed as CLI args (API tokens, passwords) don't land in
                // ~/.agentmux/logs/ in plaintext.
                tracing::info!(block_id = %cmd.blockid, "ShellExec");
                tracing::debug!(command = %cmd.command, "ShellExec command");

                // Reject container agents: their filesystem lives inside the
                // Docker container, so running sh on the host gives misleading
                // results or mutates the wrong environment.  Routing through
                // `docker exec` is tracked as a follow-up feature.
                let block: crate::backend::obj::Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("shellexec: load block: {e}"))?
                    .ok_or_else(|| format!("shellexec: block {} not found", cmd.blockid))?;
                let agent_mode = crate::backend::obj::meta_get_string(
                    &block.meta, "agentMode", "host",
                );
                if agent_mode == "container" {
                    return Err(
                        "shellexec: container agents are not supported; \
                         run the command from within the agent session instead".to_string()
                    );
                }

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

                // 300s process timeout matches the frontend's RPC timeout so
                // the client sees a clean error rather than a silent EC-TIME.
                const TIMEOUT_SECS: u64 = 300;
                // 1 MB cap per stream — bounded during read via take() so a
                // runaway command (`! yes`, `! dd if=/dev/zero`) cannot exhaust
                // RAM before the timeout fires. The pipes are read concurrently
                // to prevent deadlock when one buffer fills while the process is
                // blocked writing to the other.
                const MAX_OUTPUT: u64 = 1_000_000;

                // Use sh -c on all platforms: agents run in a bash environment
                // (Git Bash on Windows, sh on Unix) so Unix commands like ls/pwd/grep
                // work consistently. cmd /C would exit 1 for any non-Windows command.
                let mut proc = {
                    let mut c = tokio::process::Command::new("sh");
                    c.args(["-c", &cmd.command]);
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

                // On timeout: kill_on_drop kills the direct sh/cmd child, but
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

                let format_output = |bytes: Vec<u8>| -> String {
                    // bytes.len() > MAX_OUTPUT means we read the MAX_OUTPUT+1
                    // sentinel — the stream was truncated.  Checking for equality
                    // with MAX_OUTPUT (without +1) would be a false positive for
                    // commands that emit exactly MAX_OUTPUT bytes.
                    if bytes.len() > MAX_OUTPUT as usize {
                        let s = String::from_utf8_lossy(&bytes[..MAX_OUTPUT as usize]);
                        format!("{s}…[output capped at {MAX_OUTPUT} bytes]")
                    } else {
                        String::from_utf8_lossy(&bytes).into_owned()
                    }
                };

                let result = ShellExecResult {
                    exit_code: status.code().unwrap_or(1),
                    stdout: format_output(stdout_buf),
                    stderr: format_output(stderr_buf),
                };
                Ok(Some(serde_json::to_value(result).map_err(|e| format!("shellexec: {e}"))?))
            })
        }),
    );
}
