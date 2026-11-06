// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`AcpController::spawn_process`], one function each, in the order they ran: launch
//! the child, drain its stderr, mark it running, start the stdin writer, the stdout reader and the
//! process waiter, then send `initialize`. Moved out of `spawn_process` without a change in
//! behaviour.

use super::*;

/// The stdout reader task's captures, and the handling of one line of the agent's output.
struct AcpStdoutReader {
    block_id_stdout: String,
    broker_clone: Option<Arc<mps::Broker>>,
    filestore_clone: Option<Arc<FileStore>>,
    inner_clone: Arc<Mutex<AcpInner>>,
    health_clone: Arc<TurnActivityTracker>,
    rpc_id_clone: Arc<AtomicU64>,
    outstanding_prompt_ids_clone: Arc<Mutex<HashSet<u64>>>,
    mstore_clone: Option<Arc<Store>>,
    event_bus_clone: Option<Arc<EventBus>>,
    global_output_zone: Option<String>,
}

/// What one line of output turned out to be, for how it is written to the pane.
#[derive(Default)]
struct LineState {
    /// A failed `session/load` the handshake recovers from (it opens a new session): not shown,
    /// or the pane reads it as an error that ended a turn.
    recovered_error: bool,
    /// A line the agent sends while it replays a loaded session.
    replaying: bool,
    /// An error that ended the turn: a turn end follows it, since the error itself doesn't say
    /// whether other prompts are still running.
    turn_ended_by_error: bool,
}

impl AcpController {
    /// Start the child with its three pipes; it is in the block's process tracker before it runs.
    pub(super) fn launch_child(
        &self,
        cli_command: &str,
        cli_args: &[String],
        working_dir: &str,
        env_vars: &HashMap<String, String>,
    ) -> Result<(tokio::process::Child, u32), String> {
        let mut cmd = crate::backend::pane_env::make_cli_cmd(cli_command);
        cmd.args(cli_args);

        core::apply_working_dir(&mut cmd, &self.block_id, working_dir, env_vars);
        // Identity M4a: record what this process is actually given.
        crate::backend::identity_spawn::record_process_spawn(
            &self.block_id,
            crate::backend::identity_spawn::SpawnPath::Acp,
            env_vars,
        );
        // On Windows: suppress console-window allocation. Without CREATE_NO_WINDOW,
        // node.exe spawned from a windowless sidecar may try to create/attach to a
        // console, causing stdout to go to that console rather than the pipe.
        #[cfg(windows)]
        {
            cmd.no_window();
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // In the block's tracker before it runs (`spawn_tracked`); closes the
        // Process Broker Phase B gap for ACP agent panes
        // (SPEC_PROCESS_BROKER_PHASE_B_SHELL_ACP_REGISTRATION_2026_07_31.md).
        let child = crate::backend::process_tracker::registry::spawn_tracked(&self.block_id, &mut cmd, crate::backend::process_tracker::registry::Join::Agent).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "ACP process spawn failed");
            format!("failed to spawn ACP process: {e}")
        })?;

        let pid = child.id().unwrap_or(0);

        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %cli_command,
            args = ?cli_args,
            "ACP agent process spawned"
        );
        Ok((child, pid))
    }

    /// Drain stderr with explicit error/EOF logging.
    pub(super) fn spawn_stderr_drain(&self, stderr_pipe: tokio::process::ChildStderr) {
        let block_id_stderr = self.block_id.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr_pipe).lines();
            loop {
                match reader.next_line().await {
                    Err(e) => {
                        tracing::warn!(block_id = %block_id_stderr, error = %e, "ACP stderr read error");
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(line)) => {
                        if !line.trim().is_empty() {
                            tracing::warn!(
                                block_id = %block_id_stderr,
                                line = %line,
                                "ACP agent stderr"
                            );
                        }
                    }
                }
            }
        });
    }

    /// Mark the process running and open the stdin channel; returns its receiving end.
    pub(super) fn register_running(&self, pid: u32, kill_tx: tokio::sync::oneshot::Sender<bool>) -> mpsc::Receiver<String> {
        let (msg_tx, msg_rx) = mpsc::channel::<String>(32);

        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx.clone());
            Self::set_status(&mut inner, STATUS_RUNNING);
        }
        self.health_monitor.note_spawned();
        self.publish_status();
        msg_rx
    }

    pub(super) fn spawn_stdin_writer(&self, stdin: tokio::process::ChildStdin, mut msg_rx: mpsc::Receiver<String>) {
        let block_id_stdin = self.block_id.clone();
        tokio::spawn(async move {
            let mut stdin = tokio::io::BufWriter::new(stdin);
            while let Some(line) = msg_rx.recv().await {
                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "ACP stdin write error");
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "ACP stdin newline error");
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::error!(block_id = %block_id_stdin, error = %e, "ACP stdin flush error");
                    break;
                }
            }
        });
    }

    /// Reads NDJSON lines from the agent and broadcasts them via MPS.
    pub(super) fn spawn_stdout_reader(&self, stdout: tokio::process::ChildStdout) {
        let block_id_stdout = self.block_id.clone();
        let broker_clone = self.broker.clone();
        let filestore_clone = self.filestore.clone();
        let inner_clone = self.inner.clone();
        let health_clone = self.health_monitor.clone();
        let rpc_id_clone = self.next_rpc_id.clone();
        let outstanding_prompt_ids_clone = self.outstanding_prompt_ids.clone();
        let mstore_clone = self.mstore.clone();
        let event_bus_clone = self.event_bus.clone();
        // Resolve the agent's GLOBAL transcript zone once (see persistent/spawn.rs).
        let global_output_zone =
            crate::backend::transcript::append::resolve_global_output_zone(&self.mstore, &self.block_id);
        let reader_state = AcpStdoutReader {
            block_id_stdout,
            broker_clone,
            filestore_clone,
            inner_clone,
            health_clone,
            rpc_id_clone,
            outstanding_prompt_ids_clone,
            mstore_clone,
            event_bus_clone,
            global_output_zone,
        };
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            tracing::info!(block_id = %reader_state.block_id_stdout, "ACP stdout_reader started");

            loop {
                let line = match reader.next_line().await {
                    Err(e) => {
                        tracing::warn!(block_id = %reader_state.block_id_stdout, error = %e, "ACP stdout read error");
                        break;
                    }
                    Ok(None) => {
                        tracing::info!(block_id = %reader_state.block_id_stdout, "ACP stdout EOF");
                        break;
                    }
                    Ok(Some(l)) => l,
                };
                reader_state.handle_line(line);
            }
        });
    }

    pub(super) fn spawn_process_waiter(&self, mut child: tokio::process::Child, kill_rx: tokio::sync::oneshot::Receiver<bool>) {
        let generation = self.begin_process();
        let block_id_wait = self.block_id.clone();
        let inner_wait = self.inner.clone();
        let broker_wait = self.broker.clone();
        let health_wait = self.health_monitor.clone();
        let outstanding_prompt_ids_wait = self.outstanding_prompt_ids.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = kill_rx => {
                    let _ = agentmux_common::process::kill_child_and_group(&mut child).await;
                    tracing::info!(block_id = %block_id_wait, "ACP process killed");

                    let mut inner = inner_wait.lock().unwrap();
                    if inner.generation != generation {
                        // A newer process owns the controller now.
                        return;
                    }
                    inner.stdin_tx = None;
                    inner.current_pid = None;
                    AcpController::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    health_wait.set_active_turn(false);
                    // resync_controller (mod.rs) can REUSE this exact
                    // AcpController instance across a kill+restart cycle
                    // (needs_replace false, status STATUS_DONE — the
                    // non-forced pane-reopen path, or after watchdog.rs's
                    // max-runtime/idle-timeout stop) — outstanding_prompt_ids
                    // is the SAME Arc across that reuse. Any prompt ids still
                    // outstanding at kill time will never get a response (no
                    // process left to answer), so without clearing here they
                    // would linger forever: the NEXT turn's own prompt id
                    // resolving would never empty the set, permanently
                    // stranding turn_active at true for the rest of this
                    // instance's lifetime. reagentx P1 on PR #2338
                    // (thirty-third re-review).
                    outstanding_prompt_ids_wait.lock().unwrap().clear();

                    if let Some(ref broker) = broker_wait {
                        let status = agent_runtime_status(&block_id_wait, 0, STATUS_DONE, -1, false);
                        super::super::publish_controller_status(broker, &status);
                    }
                }
                status = child.wait() => {
                    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                    tracing::info!(
                        block_id = %block_id_wait,
                        exit_code = exit_code,
                        "ACP process exited"
                    );
                    let mut inner = inner_wait.lock().unwrap();
                    if inner.generation != generation {
                        // A newer process owns the controller now.
                        return;
                    }
                    inner.proc_exit_code = exit_code;
                    inner.stdin_tx = None;
                    AcpController::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    health_wait.set_active_turn(false);
                    // See the matching comment in the kill_rx branch above —
                    // same reasoning applies to a natural process exit.
                    outstanding_prompt_ids_wait.lock().unwrap().clear();

                    if let Some(ref broker) = broker_wait {
                        let status = agent_runtime_status(&block_id_wait, 0, STATUS_DONE, exit_code, false);
                        super::super::publish_controller_status(broker, &status);
                    }
                }
            }
        });
    }

    /// ACP v1 handshake: `initialize` only. Its result decides between
    /// `session/new` and `session/load`; the stdout reader sends that.
    pub(super) fn send_initialize(&self, working_dir: &str) {
        let init_id = self.next_id();
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": initialize_params(),
        })
        .to_string();
        let mut inner = self.inner.lock().unwrap();
        inner.init_request_id = Some(init_id);
        inner.session_cwd = session_cwd(working_dir);
        if let Some(ref tx) = inner.stdin_tx {
            if tx.try_send(init_req).is_err() {
                tracing::error!(block_id = %self.block_id, "[acp] initialize dropped — channel full or closed; agent will not start");
            }
        }
    }
}

impl AcpStdoutReader {
    fn handle_line(&self, line: String) {
        if line.is_empty() {
            return;
        }
        self.health_clone.note_output();
        let mut state = LineState::default();
        // Parse as JSON to check for session/update notifications
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
            self.reply_to_agent_request(&json);
            let loaded = self.advance_handshake(&json, &mut state);
            self.on_session(&json, loaded);
            self.on_prompt_resolved(&json, &mut state);
        }

        self.append_line(&line, &state);
    }

    fn reply_to_agent_request(&self, json: &serde_json::Value) {
        // A request from the agent (permission, fs, terminal):
        // answer it, or the agent waits forever.
        if let Some(reply) = reply_to_agent_request(json) {
            let inner = self.inner_clone.lock().unwrap();
            if let Some(ref tx) = inner.stdin_tx {
                if tx.try_send(reply.to_string()).is_err() {
                    tracing::warn!(block_id = %self.block_id_stdout, "[acp] reply to an agent request dropped — channel full or closed");
                }
            }
        }
    }

    /// Handshake: `initialize` answered → open or load the session; a failed `session/load` → open
    /// a new one. Returns the session a successful `session/load` loaded.
    fn advance_handshake(&self, json: &serde_json::Value, state: &mut LineState) -> Option<String> {
        // A loaded session's result carries no id: it is the one we asked for.
        let msg_id = json.get("id").and_then(|v| v.as_u64());
        let mut loaded: Option<String> = None;
        {
            let mut inner = self.inner_clone.lock().unwrap();
            let is = |id: Option<u64>| msg_id.is_some() && msg_id == id;
            state.replaying = is_session_replay(inner.loading_session.is_some(), is(inner.session_request_id));
            let next = if is(inner.init_request_id) && json.get("result").is_some() {
                inner.init_request_id = None;
                let can_load = json
                    .pointer("/result/agentCapabilities/loadSession")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let takes = |t: &str| {
                    json.pointer(&format!("/result/agentCapabilities/mcpCapabilities/{t}"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                };
                let env = &inner.mcp_env;
                let lookup = |k: &str| env.get(k).cloned().or_else(|| std::env::var(k).ok());
                let servers = inner.mcp_json.as_deref().map_or_else(Vec::new, |raw| {
                    let (http, sse) = (takes("http"), takes("sse"));
                    let path = env.get("PATH").map(String::as_str);
                    let resolve = |c: &str| super::super::acp_mcp::resolve_command(c, path);
                    super::super::acp_mcp::acp_mcp_servers(raw, http, sse, resolve, lookup)
                });
                inner.mcp_servers = servers;
                let resume = inner.resume_session_id.clone();
                let loading = if can_load { resume.clone() } else { None };
                let req = session_request(resume.as_deref(), can_load, &inner.session_cwd, &inner.mcp_servers);
                Some((req, loading))
            } else if is(inner.session_request_id) && json.get("error").is_some() && inner.loading_session.is_some() {
                inner.loading_session = None;
                state.recovered_error = true;
                Some((session_request(None, false, &inner.session_cwd, &inner.mcp_servers), None))
            } else {
                None
            };
            if is(inner.session_request_id) && json.get("result").is_some() {
                loaded = inner.loading_session.take();
            }
            if next.is_none() && is(inner.init_request_id) && json.get("error").is_some() {
                // initialize refused (an unsupported protocol
                // version): no session will ever open.
                inner.init_request_id = None;
                inner.pending_prompt = None;
                inner.session_failed = true;
                state.turn_ended_by_error = true;
            }
            if next.is_none() && is(inner.session_request_id) && json.get("error").is_some() {
                // session/new refused (pi: "Authentication
                // required"): the queued prompt can't run.
                inner.pending_prompt = None;
                inner.session_failed = true;
                state.turn_ended_by_error = true;
            }
            if let Some(((method, params), loading)) = next {
                let id = self.rpc_id_clone.fetch_add(1, Ordering::Relaxed);
                inner.session_request_id = Some(id);
                inner.loading_session = loading;
                let req = serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
                if let Some(ref tx) = inner.stdin_tx {
                    if tx.try_send(req).is_err() {
                        tracing::error!(block_id = %self.block_id_stdout, method, "[acp] session request dropped — channel full or closed; agent will not start");
                    }
                }
            }
        }
        loaded
    }

    fn on_session(&self, json: &serde_json::Value, loaded: Option<String>) {
        let session = json
            .pointer("/result/sessionId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(loaded);
        if let Some(sid_owned) = session {
            {
                let sid = sid_owned.as_str();
                {
                    let mut inner = self.inner_clone.lock().unwrap();
                    inner.session_id = Some(sid_owned.clone());
                    // Flush pending prompt now that session is ready.
                    self.flush_pending_prompt(&mut inner, sid);
                }
                tracing::info!(
                    block_id = %self.block_id_stdout,
                    session_id = %sid_owned,
                    "ACP session established"
                );
                // Persist to block metadata and broadcast so the frontend's
                // "My Agents" reattach path can read agent:sessionid from
                // block.meta. ACP previously captured the ID in memory only —
                // this mirrors the careful path from persistent/stdout_reader.rs /
                // subprocess/host_spawn.rs.
                core::persist_session_id(&self.block_id_stdout, &sid_owned, &self.mstore_clone, &self.event_bus_clone);
            }
        }
    }

    /// Send the prompt queued before the session was ready.
    fn flush_pending_prompt(&self, inner: &mut AcpInner, sid: &str) {
        if let Some(prompt) = inner.pending_prompt.take() {
            let id = self.rpc_id_clone.fetch_add(1, Ordering::Relaxed);
            self.outstanding_prompt_ids_clone.lock().unwrap().insert(id);
            // Mark the turn active + publish, mirroring
            // send_input's identical pattern — without
            // this, a prompt flushed here (queued before
            // the controller finished starting) is
            // invisible to is_active_turn()/wasTurnActive:
            // a concurrently-deferred /login controller
            // refresh could then read
            // isBackendTurnConfirmedIdle() as true and
            // force-restart the controller while this
            // flushed prompt is genuinely in flight.
            // reagentx P2 on PR #2338 (thirty-first
            // re-review).
            self.health_clone.mark_turn_active_returning_was_active();
            if let Some(ref broker) = self.broker_clone {
                let status = agent_runtime_status(
                    &self.block_id_stdout,
                    inner.status_version,
                    &inner.proc_status,
                    inner.proc_exit_code,
                    true,
                );
                super::super::publish_controller_status(broker, &status);
            }
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/prompt",
                "params": prompt_params(sid, &prompt),
            }).to_string();
            if let Some(ref tx) = inner.stdin_tx {
                if tx.try_send(req).is_err() {
                    tracing::warn!(block_id = %self.block_id_stdout, "[acp] session/prompt send failed — channel full or closed");
                    // Mirror send_input's enqueue-failure
                    // rollback: this prompt was never
                    // really sent, so it must not be
                    // left occupying the outstanding set
                    // or leave the turn marked active
                    // when nothing else is genuinely
                    // outstanding.
                    let now_empty = {
                        let mut outstanding = self.outstanding_prompt_ids_clone.lock().unwrap();
                        outstanding.remove(&id);
                        outstanding.is_empty()
                    };
                    if now_empty {
                        self.health_clone.set_active_turn(false);
                        if let Some(ref broker) = self.broker_clone {
                            let status = agent_runtime_status(
                                &self.block_id_stdout,
                                inner.status_version,
                                &inner.proc_status,
                                inner.proc_exit_code,
                                false,
                            );
                            super::super::publish_controller_status(broker, &status);
                        }
                    }
                }
            }
        }
    }

    fn on_prompt_resolved(&self, json: &serde_json::Value, state: &mut LineState) {
        // A prompt is RESOLVED — one way or another — by either
        // a real `stopReason` completion or a JSON-RPC `error`
        // rejection (mid-turn prompt acceptance is agent-
        // dependent; some agents reject a steering prompt sent
        // while an earlier one is still being processed).
        // Either way, remove it from the outstanding set and
        // only declare the turn genuinely over once NOTHING is
        // left outstanding — never based on "was this the most
        // recently sent prompt," which breaks under steering:
        // an earlier prompt's completion can arrive after a
        // newer one was sent, or a newer prompt's rejection can
        // arrive after an earlier one already completed. codex
        // P2 on PR #2338 (twenty-seventh through thirtieth
        // re-reviews).
        let resolved_id = if json.get("id").is_some()
            && json.get("result").and_then(|r| r.get("stopReason")).is_some()
        {
            json.get("id").and_then(|v| v.as_u64())
        } else if json.get("id").is_some() && json.get("error").is_some() {
            json.get("id").and_then(|v| v.as_u64())
        } else {
            None
        };
        if let Some(resolved_id) = resolved_id {
            let turn_is_over = {
                let mut outstanding = self.outstanding_prompt_ids_clone.lock().unwrap();
                resolve_prompt_and_check_idle(&mut outstanding, resolved_id)
            };
            if turn_is_over && json.get("error").is_some() {
                state.turn_ended_by_error = true;
            }
            if turn_is_over {
                self.health_clone.set_active_turn(false);
                // Publish the flip so live controllerstatus
                // subscribers see "turn ended" immediately,
                // mirroring persistent/stdout_reader.rs's matching publish
                // on its own normal (non-kill, non-exit)
                // turn-end path. Without this, nothing
                // publishes turn_active: false for an ACP
                // controller (OpenClaw/Copilot/Pi in
                // catalog.ts) until kill or process exit —
                // a normal turn-end here left the frontend's
                // `wasTurnActive` signal (fed only by live
                // controllerstatus events, never local
                // state) stuck at its last-seen value,
                // stranding useAgentCommands.ts's
                // flushPendingControllerRefresh (which
                // requires isBackendTurnConfirmedIdle —
                // wasTurnActive === false — before running a
                // /login-deferred controller restart) until
                // an unrelated event happened to republish
                // status (#2338).
                if let Some(ref broker) = self.broker_clone {
                    let status = {
                        let locked = self.inner_clone.lock().unwrap();
                        agent_runtime_status(
                            &self.block_id_stdout,
                            locked.status_version,
                            &locked.proc_status,
                            locked.proc_exit_code,
                            false,
                        )
                    };
                    super::super::publish_controller_status(broker, &status);
                }
            }
        }
    }

    /// Persist + broadcast via the shared helper (same as subprocess/host_spawn.rs).
    fn append_line(&self, line: &str, state: &LineState) {
        if state.recovered_error {
            tracing::info!(block_id = %self.block_id_stdout, "[acp] session/load refused; opening a new session");
        } else if state.replaying {
            // Earlier turns the agent replays while loading: the pane
            // already has them, and their tool-call ids would collide.
        } else if let Some(ref broker) = self.broker_clone {
            let line_with_newline = format!("{}\n", line);
            super::super::shell::handle_append_block_file(
                broker,
                &self.block_id_stdout,
                ACP_OUTPUT_SUBJECT,
                line_with_newline.as_bytes(),
                self.filestore_clone.as_ref(),
                self.global_output_zone.as_deref(),
            );
            if state.turn_ended_by_error {
                let end = format!("{}\n", TURN_ENDED_BY_ERROR);
                super::super::shell::handle_append_block_file(
                    broker,
                    &self.block_id_stdout,
                    ACP_OUTPUT_SUBJECT,
                    end.as_bytes(),
                    self.filestore_clone.as_ref(),
                    self.global_output_zone.as_deref(),
                );
            }
        }
    }
}
