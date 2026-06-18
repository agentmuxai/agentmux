// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PersistentSubprocessController: manages agent CLI as a long-running process
//! with bidirectional NDJSON streaming via stdin/stdout.
//!
//! Architecture:
//!   A single CLI process is spawned on first message and kept alive for the
//!   entire session. User messages are written as NDJSON lines to stdin without
//!   closing it. This enables mid-turn input (redirecting the agent while it
//!   is still processing).
//!
//! State machine:
//!   INIT ─(first message)─> RUNNING ─(idle between turns)─> RUNNING
//!   RUNNING ─(kill/stop)─> DONE
//!   RUNNING ─(process crash)─> DONE (auto-restart possible via session_id)
//!
//! I/O model (3 async tasks per session):
//! 1. stdin_writer: mpsc channel → process stdin (NDJSON lines)
//! 2. stdout_reader: process stdout → .jsonl persistence + WPS blockfile events
//! 3. process_waiter: wait for exit, update status

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, STATUS_DONE, STATUS_INIT,
    STATUS_RUNNING,
};
use super::health::{classify_output_line, HealthMonitor};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::wps;

/// WPS file subject name for persistent subprocess output.
pub const PERSISTENT_OUTPUT_SUBJECT: &str = "output";

/// Controller type constant.
pub const BLOCK_CONTROLLER_PERSISTENT: &str = "persistent";

/// Resolve the muxbus address (the agent's display name) from a spawn env map.
/// `AGENTMUX_AGENT_ID` (= `agent.name`, set at block creation) is canonical;
/// `WAVEMUX_AGENT_ID` is the legacy fallback. Returns `None` — i.e. not
/// muxbus-addressable — when neither is present (a non-agent persistent block).
fn muxbus_agent_id_from_env(env: &HashMap<String, String>) -> Option<String> {
    for key in ["AGENTMUX_AGENT_ID", "WAVEMUX_AGENT_ID"] {
        if let Some(v) = env.get(key) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod muxbus_registration_tests {
    use super::muxbus_agent_id_from_env;
    use std::collections::HashMap;

    #[test]
    fn resolves_agentmux_agent_id() {
        let mut env = HashMap::new();
        env.insert("AGENTMUX_AGENT_ID".to_string(), "Naki".to_string());
        assert_eq!(muxbus_agent_id_from_env(&env), Some("Naki".to_string()));
    }

    #[test]
    fn falls_back_to_legacy_wavemux_id() {
        let mut env = HashMap::new();
        env.insert("WAVEMUX_AGENT_ID".to_string(), "clamk".to_string());
        assert_eq!(muxbus_agent_id_from_env(&env), Some("clamk".to_string()));
    }

    #[test]
    fn prefers_agentmux_over_legacy() {
        let mut env = HashMap::new();
        env.insert("AGENTMUX_AGENT_ID".to_string(), "new".to_string());
        env.insert("WAVEMUX_AGENT_ID".to_string(), "old".to_string());
        assert_eq!(muxbus_agent_id_from_env(&env), Some("new".to_string()));
    }

    #[test]
    fn none_when_absent_or_blank() {
        let mut env: HashMap<String, String> = HashMap::new();
        assert_eq!(muxbus_agent_id_from_env(&env), None);
        env.insert("AGENTMUX_AGENT_ID".to_string(), "   ".to_string());
        assert_eq!(muxbus_agent_id_from_env(&env), None);
    }
}

/// Configuration for spawning the persistent process.
#[derive(Debug, Clone)]
pub struct PersistentSpawnConfig {
    pub cli_command: String,
    pub cli_args: Vec<String>,
    pub working_dir: String,
    pub env_vars: HashMap<String, String>,
    pub session_id_field: String,
    /// Resume flag for this provider (e.g. "--resume"), read from
    /// `agent:resume_flag` meta. Empty = provider has no simple-flag resume.
    /// Mirrors `SubprocessSpawnConfig::resume_flag` so a respawn (after a
    /// runtime/model change or the picker reattach path) continues the same
    /// conversation instead of starting fresh.
    pub resume_flag: String,
    /// Session id to hydrate `inner.session_id` with BEFORE spawning, when the
    /// controller hasn't captured one yet (fresh controller after a forced
    /// restart, or picker reattach). Read from `agent:sessionid` meta. With a
    /// non-empty `resume_flag` this makes `--resume <sid>` land on the respawn.
    pub session_id: String,
    /// Echoed back as `agent-message-accepted` so the frontend can promote the
    /// pending entry. Matches `CommandAgentInputData.message_id` on the AgentInput
    /// command; absent for legacy callers.
    pub message_id: Option<String>,
}

/// Inner state protected by mutex.
struct PersistentInner {
    proc_status: String,
    proc_exit_code: i32,
    status_version: i32,
    session_id: Option<String>,
    current_pid: Option<u32>,
    /// Channel to send messages to the stdin writer task.
    stdin_tx: Option<mpsc::Sender<String>>,
    /// Handle to kill the process.
    kill_tx: Option<tokio::sync::oneshot::Sender<bool>>,
    /// AskUserQuestion `can_use_tool` control_requests awaiting a user answer:
    /// `tool_use_id -> (request_id, questions JSON)`. Filled by the stdout
    /// reader when the CLI sends a `can_use_tool` control_request for
    /// AskUserQuestion; consumed by `answer_question` to build the matching
    /// `control_response`. Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    pending_questions: HashMap<String, (String, serde_json::Value)>,
}

/// PersistentSubprocessController keeps a long-running CLI process alive,
/// sending user messages as NDJSON lines on stdin.
pub struct PersistentSubprocessController {
    #[allow(dead_code)]
    tab_id: String,
    block_id: String,
    inner: Arc<Mutex<PersistentInner>>,
    broker: Option<Arc<wps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    wstore: Option<Arc<Store>>,
    /// FileStore for write-through persistence of output lines (Phase 1.3).
    filestore: Option<Arc<FileStore>>,
    health_monitor: Arc<HealthMonitor>,
    /// Monotonic counter bumped for every stdout line (including control frames).
    /// The AskUserQuestion dead-air fallback snapshots this *before* sending the
    /// answer and re-checks after a short window; any increment means the CLI
    /// produced output (assistant content OR a follow-up control_request), i.e.
    /// the turn resumed. Counting *all* frames — not just `record_output`, which
    /// the reader skips for control frames — avoids a spurious fallback when the
    /// resumed turn's first activity is a tool-permission round-trip.
    stdout_seq: Arc<AtomicU64>,
}

/// How long to wait after delivering an AskUserQuestion answer before assuming
/// the turn did not resume and re-delivering the answer as a follow-up message.
/// See `answer_question` and SPEC_ASK_USER_QUESTION_2026_06_15.md §10.1.
const ANSWER_RESUME_FALLBACK_MS: u64 = 4000;

/// Compose the directive follow-up message used by the AskUserQuestion dead-air
/// fallback. `answers` maps each question's text to the selected label(s) or free
/// text (the same object delivered in the control_response). The message is
/// deliberately directive so the model resumes the task instead of treating it
/// as a no-op (the "user sent an empty message" failure mode).
fn build_answer_resume_message(answers: &serde_json::Value) -> String {
    let mut out = String::from(
        "[AgentMux] Your earlier question was answered, but the turn had already ended, \
         so the answer is delivered here as a follow-up. Resume the task you were working \
         on using this answer — do not wait for further input:\n",
    );
    match answers.as_object() {
        Some(map) if !map.is_empty() => {
            for (question, answer) in map {
                let rendered = match answer {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(items) => items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                        .join(", "),
                    other => other.to_string(),
                };
                out.push_str(&format!("\n• {question}: {rendered}"));
            }
        }
        _ => out.push_str(&format!("\nAnswer: {answers}")),
    }
    out
}

impl PersistentSubprocessController {
    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        wstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        let health_monitor = Arc::new(HealthMonitor::new(
            block_id.clone(),
            broker.clone(),
        ));
        Self {
            tab_id,
            block_id,
            inner: Arc::new(Mutex::new(PersistentInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                session_id: None,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                pending_questions: HashMap::new(),
            })),
            broker,
            event_bus,
            wstore,
            filestore,
            health_monitor,
            stdout_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    fn set_status(inner: &mut PersistentInner, status: &str) {
        inner.proc_status = status.to_string();
        inner.status_version += 1;
    }

    fn get_status_snapshot(&self) -> BlockControllerRuntimeStatus {
        let inner = self.inner.lock().unwrap();
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            version: inner.status_version,
            shellprocstatus: inner.proc_status.clone(),
            shellprocconnname: "local".to_string(),
            shellprocexitcode: inner.proc_exit_code,
            spawn_ts_ms: None,
            is_agent_pane: true,
        }
    }

    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::publish_controller_status(broker, &status);
        }
    }

    fn is_running(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.stdin_tx.is_some()
    }

    /// Send a user message to the running CLI process.
    /// If the process isn't spawned yet, spawns it first.
    /// Emit `agent-message-accepted` for a given message_id, if set.
    /// Mirrors the subprocess controller's `emit_message_accepted` — signals the
    /// frontend to promote the pending entry from queued to in-document.
    fn emit_message_accepted(&self, message_id: Option<&str>) {
        let Some(id) = message_id else { return };
        let Some(ref broker) = self.broker else { return };
        let event = crate::backend::wps::WaveEvent {
            event: crate::backend::wps::EVENT_AGENT_MESSAGE_ACCEPTED.to_string(),
            scopes: vec![format!("block:{}", self.block_id)],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({
                "block_id": self.block_id,
                "message_id": id,
            })),
        };
        broker.publish(event);
        tracing::info!(
            block_id = %self.block_id,
            message_id = %id,
            "emitted agent-message-accepted"
        );
    }

    pub fn send_message(&self, message: String, config: PersistentSpawnConfig) -> Result<(), String> {
        // Spawn process if not running
        if !self.is_running() {
            self.spawn_process(config.clone())?;
        }

        // Format as stream-json user message
        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });
        let json_str = json_msg.to_string();

        // Silently persist the user message to the blockfile + global zone so
        // `parseHistoryLines` can reconstruct `user_message` nodes on the next
        // open. No WPS event is published here — the live-display is handled by
        // the `agent-message-accepted` path (UUID node), avoiding a duplicate.
        let global_zone = super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
        let line_with_newline = format!("{json_str}\n");
        super::shell::persist_to_blockfile_silent(
            &self.block_id,
            crate::backend::agent_session::OUTPUT_FILE,
            line_with_newline.as_bytes(),
            self.filestore.as_ref(),
            global_zone.as_deref(),
        );

        let inner = self.inner.lock().unwrap();
        let tx = inner.stdin_tx.as_ref()
            .ok_or("persistent process not running after spawn")?;
        tx.try_send(json_str)
            .map_err(|e| format!("stdin send failed: {e}"))?;
        drop(inner);
        self.emit_message_accepted(config.message_id.as_deref());
        Ok(())
    }

    /// Deliver a user message to the **already-running** persistent process,
    /// without a spawn config. Unlike `send_message`, this never spawns — it errors
    /// if the process is not running. Used for controller-aware muxbus/reactive
    /// delivery (`deliver_agent_message`), where the agent is live (busy or idle)
    /// and we have no `PersistentSpawnConfig` to hand. Writing on the live stdin lets
    /// the message land mid-turn (steering) instead of waiting for idle.
    /// Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §6 (Phase 3).
    pub fn send_user_message(&self, message: String) -> Result<(), String> {
        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });

        let inner = self.inner.lock().unwrap();
        let tx = inner
            .stdin_tx
            .as_ref()
            .ok_or("persistent process not running")?;
        tx.try_send(json_msg.to_string())
            .map_err(|e| format!("stdin send failed: {e}"))
    }

    /// Answer a parked AskUserQuestion via the Agent SDK **control protocol**.
    ///
    /// The CLI asked us with a `can_use_tool` control_request (parked in
    /// `pending_questions` by the stdout reader); we reply with a
    /// `control_response` carrying `updatedInput.answers`. This is the ONLY
    /// mechanism the CLI accepts — delivering a `tool_result` on stdin does NOT
    /// work (the CLI auto-rejects AskUserQuestion within the turn). `answers` is
    /// the JSON object mapping each question's text to the selected label(s) or
    /// free-text. Process must already be running (agent is mid-turn, blocked on
    /// this answer). Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §2.3.
    pub fn answer_question(&self, tool_use_id: String, answers: serde_json::Value) -> Result<(), String> {
        let (request_id, questions, tx) = {
            let mut inner = self.inner.lock().unwrap();
            let (rid, qs) = inner
                .pending_questions
                .remove(&tool_use_id)
                .ok_or_else(|| format!("no pending AskUserQuestion for tool_use_id {tool_use_id}"))?;
            let tx = inner
                .stdin_tx
                .as_ref()
                .ok_or("persistent process not running (cannot deliver answer)")?
                .clone();
            (rid, qs, tx)
        };

        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "allow",
                    "updatedInput": { "questions": questions, "answers": answers.clone() },
                    "toolUseID": tool_use_id,
                }
            }
        });
        // Snapshot stdout activity BEFORE sending the answer, so a fast resume
        // that emits between the send and the snapshot can't be mistaken for
        // "no activity" (codex review on #1536).
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net. The CLI *abandons* a pending AskUserQuestion
        // tool_use if its turn already ended, silently dropping the
        // control_response above — the model then sees an empty message and
        // stalls (SPEC_ASK_USER_QUESTION_2026_06_15.md §9/§10.1; the dead-air
        // report). If no stdout activity appears shortly after the answer, the
        // turn did not resume, so re-deliver the answer as a normal follow-up
        // user turn — the same resilience the one-shot controllers already use.
        // Gated on stdout activity (every frame, incl. control frames), so it is
        // mutually exclusive with a real resume and never double-delivers.
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let resume_msg = build_answer_resume_message(&answers);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_RESUME_FALLBACK_MS)).await;
            // Any stdout frame since the snapshot means the turn resumed — nothing to do.
            if stdout_seq.load(Ordering::Relaxed) != before_seq {
                return;
            }
            let line = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": resume_msg }
            })
            .to_string();
            let stdin_tx = { inner.lock().unwrap().stdin_tx.clone() };
            match stdin_tx {
                Some(stdin_tx) if stdin_tx.try_send(line).is_ok() => {
                    tracing::warn!(
                        block_id = %block_id,
                        tool_use_id = %tool_use_id,
                        fallback_ms = ANSWER_RESUME_FALLBACK_MS,
                        "AskUserQuestion answer did not resume the turn — re-delivered as a follow-up message (dead-air fallback)"
                    );
                }
                Some(_) => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion dead-air fallback: stdin send failed"
                ),
                None => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion dead-air fallback skipped: process not running"
                ),
            }
        });
        Ok(())
    }

    /// Push a raw NDJSON line to the live stdin (used to emit control_responses
    /// from the stdout-reader task, which only holds an `Arc<Mutex<Inner>>`).
    fn push_stdin(inner: &Arc<Mutex<PersistentInner>>, line: String) {
        let guard = inner.lock().unwrap();
        if let Some(tx) = guard.stdin_tx.as_ref() {
            let _ = tx.try_send(line);
        }
    }

    /// Handle a control-protocol frame from the CLI's stdout. `control_request`
    /// of subtype `can_use_tool`: AskUserQuestion is **parked** (the frontend
    /// panel — rendered from the assistant stream — answers it via
    /// `answer_question`); every other tool is **auto-allowed** to preserve the
    /// current bypass/yolo UX (Phase 1; Phase 2 routes these to the decision
    /// prompt, #551). `control_response` frames (replies to requests we initiate,
    /// none today) are logged and dropped. These frames are NOT conversation
    /// output and never reach the blockfile.
    /// Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §4.2.
    fn handle_control_frame(
        kind: &str,
        parsed: &serde_json::Value,
        block_id: &str,
        inner: &Arc<Mutex<PersistentInner>>,
    ) {
        if kind == "control_response" {
            return;
        }
        // control_request
        let req = match parsed.get("request") {
            Some(r) => r,
            None => return,
        };
        let subtype = req.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
        let request_id = parsed
            .get("request_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if subtype != "can_use_tool" {
            tracing::info!(block_id = %block_id, subtype = %subtype, "persistent control_request: unhandled subtype, ignoring");
            return;
        }

        let tool_name = req.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let tool_use_id = req
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let input = req.get("input").cloned().unwrap_or_else(|| serde_json::json!({}));

        if tool_name == "AskUserQuestion" {
            // Park; the frontend question panel will answer via answer_question().
            let questions = input
                .get("questions")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            {
                let mut guard = inner.lock().unwrap();
                guard
                    .pending_questions
                    .insert(tool_use_id.clone(), (request_id, questions));
            }
            tracing::info!(block_id = %block_id, tool_use_id = %tool_use_id, "AskUserQuestion parked; awaiting user answer");
        } else {
            // Auto-allow every other tool (preserve today's bypass UX).
            let resp = serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": { "behavior": "allow", "updatedInput": input }
                }
            });
            Self::push_stdin(inner, resp.to_string());
        }
    }

    /// Spawn the persistent CLI process.
    fn spawn_process(&self, config: PersistentSpawnConfig) -> Result<(), String> {
        // Build command — use make_cli_cmd to resolve .cmd wrappers to node on Windows
        let mut cmd = crate::server::cli_handlers::make_cli_cmd(&config.cli_command);

        // Hydrate the captured session id from the config when we don't have one
        // yet (fresh controller after a forced restart — e.g. a /model change —
        // or the picker reattach path). Mirrors SubprocessController::
        // hydrate_session_id_from_config so the respawn resumes the same
        // conversation instead of starting blank.
        if !config.session_id.is_empty() {
            let mut inner = self.inner.lock().unwrap();
            if inner.session_id.is_none() {
                inner.session_id = Some(config.session_id.clone());
            }
        }

        // Append `--resume <sid>` when we have a session id and the provider
        // supports simple-flag resume — same construction as
        // SubprocessController::spawn_turn. This is what makes a model/effort
        // change (which respawns the persistent CLI with new flags) preserve the
        // conversation. cli_args carries the runtime flags (model/effort/perm)
        // already rebuilt by the frontend (useAgentCommands buildRuntimeArgs).
        let mut spawn_args = config.cli_args.clone();
        {
            let inner = self.inner.lock().unwrap();
            if let Some(ref sid) = inner.session_id {
                if !config.resume_flag.is_empty() {
                    spawn_args.push(config.resume_flag.clone());
                    spawn_args.push(sid.clone());
                }
            }
        }
        cmd.args(&spawn_args);

        // Working directory
        if !config.working_dir.is_empty() {
            let expanded_dir = if config.working_dir.starts_with("~/") || config.working_dir == "~" {
                if let Some(home) = dirs::home_dir() {
                    home.join(config.working_dir.trim_start_matches("~/")).to_string_lossy().to_string()
                } else {
                    config.working_dir.clone()
                }
            } else {
                config.working_dir.clone()
            };
            let dir_path = std::path::Path::new(&expanded_dir);
            if !dir_path.exists() {
                if let Err(e) = std::fs::create_dir_all(dir_path) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        dir = %expanded_dir,
                        error = %e,
                        "failed to create working directory"
                    );
                }
            }
            if dir_path.exists() {
                cmd.current_dir(&expanded_dir);

                // 4.2 follow-up: warn loudly if the agent's working directory
                // contains a nested .git (typically because the agent, or the
                // user on its behalf, cloned a repo into its cwd). A 3.5 GB
                // nested clone was found under ~/.agentmux/agents/agentx/ in
                // an earlier session and confused agents into reading stale
                // pre-SolidJS code. We can't prevent the clone (the agent is
                // an external process) so the best we can do is make it
                // impossible to miss in the logs, with the exact cleanup
                // command the user needs to run. Single fs::metadata call —
                // no directory walk.
                let looks_like_agent_workspace = expanded_dir.contains("/.agentmux/agents/")
                    || expanded_dir.contains("\\.agentmux\\agents\\");
                if looks_like_agent_workspace {
                    let git_dir = dir_path.join(".git");
                    if git_dir.exists() {
                        tracing::warn!(
                            block_id = %self.block_id,
                            cwd = %expanded_dir,
                            ".git detected inside agent workspace — this is \
                             usually an unintended nested clone and can waste \
                             gigabytes of disk. Clean up with: rm -rf {}/.git",
                            expanded_dir
                        );
                    }
                }
            }
        }

        // Environment variables (with tilde expansion)
        for (k, v) in &config.env_vars {
            let expanded = crate::backend::base::expand_home_dir_safe(v);
            cmd.env(k, expanded.to_string_lossy().as_ref());
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "persistent process spawn failed");
            format!("failed to spawn persistent process: {e}")
        })?;

        let pid = child.id().unwrap_or(0);

        // Notify health monitor that a turn is starting. This arms the Stalled
        // (30 s) and Dead (120 s) thresholds so the frontend learns the agent
        // is not responding rather than silently waiting forever.
        self.health_monitor.set_active_turn(true);

        tracing::info!(
            block_id = %self.block_id,
            pid = pid,
            cmd = %config.cli_command,
            args = ?spawn_args,
            working_dir = %config.working_dir,
            "persistent process spawned"
        );

        // Assign the persistent CLI to this block's process tracker.
        // Matches `SubprocessController`'s identical path — both controller
        // types share the same swarm-pane visibility story.
        if pid != 0 {
            if let Some(registry) = crate::backend::process_tracker::registry::global() {
                let tracker = registry.ensure_tracker(&self.block_id);
                if let Err(e) = tracker.assign_process(pid) {
                    tracing::warn!(
                        block_id = %self.block_id,
                        pid = pid,
                        err = %e,
                        "[process-tracker] assign_process failed"
                    );
                }
            }
        }

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<bool>();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take();

        // Drain stderr in background — log lines for debugging
        if let Some(stderr_pipe) = stderr {
            let block_id_stderr = self.block_id.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::warn!(
                        block_id = %block_id_stderr,
                        line = %line,
                        "persistent stderr"
                    );
                }
            });
        }

        // Create stdin writer channel
        let (msg_tx, mut msg_rx) = mpsc::channel::<String>(32);

        {
            let mut inner = self.inner.lock().unwrap();
            inner.current_pid = Some(pid);
            inner.kill_tx = Some(kill_tx);
            inner.stdin_tx = Some(msg_tx);
            Self::set_status(&mut inner, STATUS_RUNNING);
        }
        self.publish_status();

        // Auto-register with the muxbus reactive handler so inter-agent
        // messages reach this persistent (no-PTY) agent. The PTY shell
        // controller (shell.rs) was the only prior auto-register path, so
        // stream-json agents were in the directory but absent from the
        // delivery registry — `inject_message` returned "agent not found"
        // (issue #1470). Tier-1 delivery is routed through the controller-
        // aware MessageSender (→ send_user_message), not PTY keystrokes.
        // See SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.
        let agent_id_for_muxbus = muxbus_agent_id_from_env(&config.env_vars);
        if let Some(ref agent_id) = agent_id_for_muxbus {
            match crate::backend::reactive::get_global_handler()
                .register_agent(agent_id, &self.block_id, Some(&self.tab_id))
            {
                Ok(()) => {
                    tracing::info!(
                        block_id = %self.block_id,
                        agent_id = %agent_id,
                        "muxbus: auto-registered persistent agent"
                    );
                    // Also write the cross-instance (Tier-2) file registry.
                    if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::write(
                            &data_dir,
                            agent_id,
                            &local_url,
                            &self.block_id,
                        );
                    }
                }
                Err(e) => tracing::warn!(
                    block_id = %self.block_id,
                    agent_id = %agent_id,
                    error = %e,
                    "muxbus: persistent auto-register failed"
                ),
            }
        }

        // Record active pid for crash recovery (Phase 4.2). If the server
        // dies while this subprocess is running, scan_orphans() will find
        // the stale pid on next boot and flag the session as interrupted.
        if let Some(ref wstore) = self.wstore {
            super::session_recovery::mark_active_pid(wstore, &self.block_id, pid);
        }

        // Spawn stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(msg) = msg_rx.recv().await {
                if let Err(e) = stdin.write_all(msg.as_bytes()).await {
                    tracing::warn!("persistent stdin write error: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    tracing::warn!("persistent stdin newline error: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::warn!("persistent stdin flush error: {}", e);
                    break;
                }
            }
            // Channel closed or write error → stdin drops → process gets EOF
            drop(stdin);
        });

        // Spawn stdout reader task
        let block_id_read = self.block_id.clone();
        let broker_read = self.broker.clone();
        let inner_read = Arc::clone(&self.inner);
        let wstore_read = self.wstore.clone();
        let event_bus_read = self.event_bus.clone();
        let filestore_read = self.filestore.clone();
        let health_read = Arc::clone(&self.health_monitor);
        let stdout_seq_read = Arc::clone(&self.stdout_seq);
        let session_id_field = config.session_id_field.clone();
        // Resolve the agent's GLOBAL transcript zone (`agent:<defId>:current`)
        // once, from the block's `agentId` meta, so every `output` line is also
        // mirrored to the cross-channel store. `None` for non-agent blocks.
        let global_output_zone =
            super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut stats = super::session_stats::SessionStatsAccumulator::new(block_id_read.clone());

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                // Bump the activity counter for EVERY non-empty stdout line —
                // including control frames (which `continue` below before
                // `record_output`) — so the AskUserQuestion dead-air fallback can
                // tell whether the turn resumed. See `answer_question`.
                stdout_seq_read.fetch_add(1, Ordering::Relaxed);

                // Track session metadata (debounced 1 s)
                stats.record_line(line.len(), &wstore_read);

                // Parse JSON for health monitoring and session ID capture
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                    // Control-protocol frames (can_use_tool / AskUserQuestion) are
                    // NOT conversation output — handle them and skip the blockfile
                    // so the frontend stream never sees them.
                    // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
                    if let Some(kind) = parsed.get("type").and_then(|v| v.as_str()) {
                        if kind == "control_request" || kind == "control_response" {
                            Self::handle_control_frame(kind, &parsed, &block_id_read, &inner_read);
                            continue;
                        }
                    }
                    let (meaningful, _error) = classify_output_line(&parsed);
                    health_read.record_output(meaningful);
                    if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                        let sid_string = sid.to_string();
                        let already_captured = inner_read.lock().unwrap().session_id.is_some();
                        if !already_captured {
                            tracing::info!(
                                block_id = %block_id_read,
                                session_id = %sid_string,
                                "persistent session ID captured"
                            );
                            {
                                let mut inner = inner_read.lock().unwrap();
                                inner.session_id = Some(sid_string.clone());
                            }
                            // Persist to block metadata (same pattern as subprocess.rs)
                            if let Some(ref store) = wstore_read {
                                let oref_str = format!("block:{}", block_id_read);
                                let mut meta_update =
                                    crate::backend::obj::MetaMapType::new();
                                meta_update.insert(
                                    "agent:sessionid".to_string(),
                                    serde_json::Value::String(sid_string),
                                );
                                if let Err(e) = crate::server::service::update_object_meta(
                                    store, &oref_str, &meta_update,
                                ) {
                                    tracing::warn!(
                                        block_id = %block_id_read,
                                        error = %e,
                                        "failed to persist agent:sessionid"
                                    );
                                } else if let Some(ref event_bus) = event_bus_read {
                                    if let Ok(updated_block) = store.must_get::<crate::backend::obj::Block>(&block_id_read) {
                                        let update_data = serde_json::to_value(
                                            &crate::backend::obj::WaveObjUpdate {
                                                updatetype: "update".into(),
                                                otype: "block".into(),
                                                oid: block_id_read.clone(),
                                                obj: Some(crate::backend::obj::wave_obj_to_value(&updated_block)),
                                            },
                                        )
                                        .ok();
                                        event_bus.broadcast_event(
                                            &crate::backend::eventbus::WSEventType {
                                                eventtype: "waveobj:update".to_string(),
                                                oref: oref_str,
                                                data: update_data,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // Publish line as WPS blockfile event and write-through to FileStore
                // for persistent history (Phase 1.3).
                tracing::info!(
                    block_id = %block_id_read,
                    line_len = line.len(),
                    "persistent stdout → blockfile"
                );
                let line_with_newline = format!("{}\n", line);
                if let Some(ref broker) = broker_read {
                    super::shell::handle_append_block_file(
                        broker,
                        &block_id_read,
                        PERSISTENT_OUTPUT_SUBJECT,
                        line_with_newline.as_bytes(),
                        filestore_read.as_ref(),
                        global_output_zone.as_deref(),
                    );
                } else {
                    tracing::warn!(block_id = %block_id_read, "persistent stdout: no broker available");
                }
            }

            tracing::info!(block_id = %block_id_read, "persistent stdout reader finished");
        });

        // Spawn health watchdog — checks every 5 s while turn is active.
        // Emits `agenthealth` WPS events when the process stalls (30 s) or
        // dies (120 s) without producing meaningful output, giving the
        // frontend enough signal to show a "not responding" warning.
        let health_watchdog = Arc::clone(&self.health_monitor);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if !health_watchdog.is_active_turn() {
                    break;
                }
                health_watchdog.check();
            }
        });

        // Spawn process waiter task
        let block_id_wait = self.block_id.clone();
        let inner_wait = Arc::clone(&self.inner);
        let broker_wait = self.broker.clone();
        let wstore_wait = self.wstore.clone();
        let health_wait = Arc::clone(&self.health_monitor);
        // Captured so the waiter can deregister this agent from muxbus on exit.
        let agent_id_wait = agent_id_for_muxbus.clone();

        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                    tracing::info!(
                        block_id = %block_id_wait,
                        exit_code = exit_code,
                        "persistent process exited"
                    );

                    // Notify health monitor so Stalled/Dead watchdog stops.
                    health_wait.set_exited(exit_code);

                    let mut inner = inner_wait.lock().unwrap();
                    inner.proc_exit_code = exit_code;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    Self::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    // Deregister from muxbus so later sends fall through to the
                    // lower tiers instead of resolving to a dead block. Mirrors
                    // the shell controller's exit path.
                    crate::backend::reactive::get_global_handler()
                        .unregister_block(&block_id_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::remove(&data_dir, agent_id);
                    }

                    // Clear active pid — clean exit, no recovery needed.
                    if let Some(ref wstore) = wstore_wait {
                        super::session_recovery::clear_active_pid(wstore, &block_id_wait);
                    }

                    // Publish status
                    if let Some(ref broker) = broker_wait {
                        let status = BlockControllerRuntimeStatus {
                            blockid: block_id_wait.clone(),
                            version: 0,
                            shellprocstatus: STATUS_DONE.to_string(),
                            shellprocconnname: "local".to_string(),
                            shellprocexitcode: exit_code,
                            spawn_ts_ms: None,
                            is_agent_pane: true,
                        };
                        super::publish_controller_status(broker, &status);
                    }
                }
                Ok(force) = kill_rx => {
                    tracing::info!(
                        block_id = %block_id_wait,
                        force = force,
                        "persistent process kill requested"
                    );
                    if force {
                        let _ = child.kill().await;
                    } else {
                        // Graceful: drop stdin to send EOF, then wait briefly
                        {
                            let mut inner = inner_wait.lock().unwrap();
                            inner.stdin_tx = None; // drops the sender → stdin writer exits → stdin closes
                        }
                        tokio::select! {
                            _ = child.wait() => {}
                            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                                let _ = child.kill().await;
                            }
                        }
                    }

                    health_wait.set_exited(-1);

                    let mut inner = inner_wait.lock().unwrap();
                    inner.proc_exit_code = -1;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    Self::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    // Deregister from muxbus (see the clean-exit arm above).
                    crate::backend::reactive::get_global_handler()
                        .unregister_block(&block_id_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::remove(&data_dir, agent_id);
                    }

                    // Clear active pid — user-initiated stop, no recovery needed.
                    if let Some(ref wstore) = wstore_wait {
                        super::session_recovery::clear_active_pid(wstore, &block_id_wait);
                    }
                }
            }
        });

        Ok(())
    }

    pub fn stop_process(&self, force: bool) -> Result<(), String> {
        let kill_tx = {
            let mut inner = self.inner.lock().unwrap();
            inner.kill_tx.take()
        };
        match kill_tx {
            Some(tx) => {
                let _ = tx.send(force);
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().unwrap().session_id.clone()
    }
}

impl Controller for PersistentSubprocessController {
    fn start(
        &self,
        _block_meta: super::super::obj::MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        tracing::info!(
            block_id = %self.block_id,
            "persistent controller registered (spawns on first message)"
        );
        Ok(())
    }

    fn stop(&self, _graceful: bool, new_status: &str) -> Result<(), String> {
        self.stop_process(true)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.proc_status != new_status {
            Self::set_status(&mut inner, new_status);
        }
        Ok(())
    }

    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.get_status_snapshot()
    }

    fn send_input(&self, input: BlockInputUnion, _seq: Option<u64>) -> Result<(), String> {
        // Persistent controllers have no PTY and don't take raw keystrokes —
        // user messages go through send_message(). But the agent-pane Stop
        // button / Esc delivers an *interrupt* as a signal via
        // `ControllerInputCommand({signame:"SIGINT"})` (see useAgentCommands
        // `stopAgent`). Without handling it here, stopping a persistent (e.g.
        // Claude stream-json) agent failed with "does not accept raw input".
        // Route the interrupt to the same kill path `stop()` uses, mirroring
        // SubprocessController. The session_id is retained, so the next message
        // resumes the conversation.
        if let Some(sig) = input.sig_name.as_deref() {
            if sig == "SIGINT" || sig == "SIGTERM" {
                tracing::info!(
                    block_id = %self.block_id,
                    sig = %sig,
                    "persistent controller: received signal, stopping current process"
                );
                return self.stop_process(true);
            }
            return Err(format!(
                "persistent controller: unsupported signal {sig} (only SIGINT/SIGTERM)"
            ));
        }
        // Raw keystrokes are genuinely unsupported — user messages go through
        // send_message(), not the PTY input channel.
        if input.input_data.is_some() {
            return Err(
                "persistent controller does not accept raw input; use send_message()".to_string(),
            );
        }
        // Term resize / other benign input types: accepted no-op. A persistent
        // controller has no PTY, so there is nothing to resize — but the agent
        // pane's `usePtyWidth` hook sends a `termsize` on every running turn
        // (it can't tell a PTY-backed controller from a PTY-less one). Returning
        // an error here surfaced a spurious "resize to N cols failed" warning in
        // the agent pane's activity log. Mirror SubprocessController, which
        // already no-ops termsize. See AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
        Ok(())
    }

    fn controller_type(&self) -> &str {
        BLOCK_CONTROLLER_PERSISTENT
    }

    fn block_id(&self) -> &str {
        &self.block_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod send_input_tests {
    use super::*;
    use crate::backend::obj::TermSize;

    fn controller() -> PersistentSubprocessController {
        PersistentSubprocessController::new(
            "tab".to_string(),
            "block".to_string(),
            None,
            None,
            None,
            None,
        )
    }

    // A persistent controller has no PTY, but the agent pane's usePtyWidth hook
    // sends a termsize resize on every running turn. It must be accepted as a
    // no-op, not rejected — otherwise the pane logs a spurious "resize to N cols
    // failed" warning. See AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
    #[test]
    fn termsize_resize_is_accepted_noop() {
        let c = controller();
        let res = c.send_input(BlockInputUnion::resize(TermSize { rows: 25, cols: 117 }), None);
        assert!(res.is_ok(), "termsize resize should be a no-op Ok, got {res:?}");
    }

    // The AskUserQuestion dead-air fallback re-delivers the answer as a directive
    // follow-up message; the rendering must surface each Q/A so the model can
    // resume with the decision in context. See SPEC_ASK_USER_QUESTION §10.1.
    #[test]
    fn answer_resume_message_renders_qa_pairs() {
        let answers = serde_json::json!({
            "Pick a color": "blue",
            "Pick toppings": ["cheese", "olives"],
        });
        let msg = build_answer_resume_message(&answers);
        assert!(msg.contains("Resume the task"), "must be directive: {msg}");
        assert!(msg.contains("Pick a color: blue"), "string answer: {msg}");
        assert!(
            msg.contains("Pick toppings: cheese, olives"),
            "multi-select joins labels: {msg}"
        );
    }

    #[test]
    fn answer_resume_message_handles_non_object() {
        let msg = build_answer_resume_message(&serde_json::json!("just text"));
        assert!(msg.contains("Resume the task"), "still directive: {msg}");
        assert!(msg.contains("Answer: "), "non-object falls back: {msg}");
    }

    // Raw keystrokes are genuinely unsupported on a persistent controller —
    // user messages go through send_message(), so they must still be rejected.
    #[test]
    fn raw_input_is_still_rejected() {
        let c = controller();
        let err = c
            .send_input(BlockInputUnion::data(b"ls\n".to_vec()), None)
            .unwrap_err();
        assert!(
            err.contains("does not accept raw input"),
            "raw input should be rejected, got {err:?}"
        );
    }
}
