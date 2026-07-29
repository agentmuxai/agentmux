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
use super::core;
use super::health::{classify_output_line, HealthMonitor};
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::subagent_watcher;
use crate::backend::wps;

/// WPS file subject name for persistent subprocess output.
pub const PERSISTENT_OUTPUT_SUBJECT: &str = "output";

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
    /// A `--resume` id the CLI has confirmed (via stderr) it can't find under
    /// the current config dir. The stdout reader echoes back whatever
    /// `--resume` value it was given as its first line REGARDLESS of whether
    /// resume actually succeeds, racing the stderr reader's own clear of
    /// `session_id` — this stops that race from re-adopting a known-dead id.
    /// Never reset back to `None`: a genuinely fresh session id (a new CLI-
    /// generated UUID) will never equal this one, so a stale poison value
    /// is permanently inert rather than something that needs clearing.
    resume_poisoned: Option<String>,
    /// TENTATIVE: set synchronously inside `spawn_process`, before any
    /// background task exists, right after a FRESH spawn attaches
    /// `--resume <sid>` — captures the exact spawn config + already-
    /// formatted stdin line for the message that triggered it. This alone
    /// does NOT mean a retry will happen: it only means "IF the CLI goes on
    /// to confirm this exact sid unreachable, here is what to retry."
    /// `poison_resume` is what promotes this to `confirmed_stale_resume_retry`
    /// (the only thing the process-waiter task actually acts on) — see its
    /// doc comment for why the two are kept separate. Cleared by
    /// `try_capture_session_id` on ANY successful capture (resume
    /// succeeded, or the CLI fell through to a fresh conversation on its
    /// own) since neither a retry nor a later confirmation is possible
    /// anymore; also cleared on kill and on a normal exit so a reused
    /// controller instance never carries a tentative retry into an
    /// unrelated later lifetime.
    pending_resume_retry: Option<(String, PersistentSpawnConfig, String)>,
    /// CONFIRMED: only `poison_resume` ever sets this, and only when the
    /// sid it just confirmed unreachable is the exact one
    /// `pending_resume_retry` was captured for. This is the ONLY field the
    /// process-waiter task checks before retrying — reagentx P1 on PR
    /// #2360: checking `pending_resume_retry` directly (as an earlier cut
    /// of this fix did) would retry on ANY exit before the first session id
    /// is captured, including an auth failure, network blip, or rate limit
    /// that has nothing to do with a stale `--resume` — silently discarding
    /// the user's existing conversation and the specific error they should
    /// have seen instead of the confirmed case this mechanism exists for.
    /// Cleared on kill and on a normal exit for the same
    /// reused-instance reason as `pending_resume_retry`.
    confirmed_stale_resume_retry: Option<(String, PersistentSpawnConfig, String)>,
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

impl PersistentInner {
    /// Records `bad_sid` as confirmed-unreachable (the CLI reported "No
    /// conversation found" for it) and clears it from `session_id` if it's
    /// currently held there. Pairs with `try_capture_session_id` below —
    /// whichever of the stderr/stdout reader tasks runs first, the
    /// poisoned id never survives as the live `session_id`.
    fn poison_resume(&mut self, bad_sid: &str) {
        self.resume_poisoned = Some(bad_sid.to_string());
        if self.session_id.as_deref() == Some(bad_sid) {
            self.session_id = None;
        }
        // Promote the tentative retry to CONFIRMED only if it was captured
        // for this EXACT sid — see `confirmed_stale_resume_retry`'s doc
        // comment. Compared against the ACTUAL sid the spawn attempted
        // (stored alongside the payload in `spawn_process`), not
        // `config.session_id` — those can differ once hydration has
        // already happened on an earlier call. A tentative retry captured
        // for a DIFFERENT (later, still in-flight) spawn attempt is left
        // untouched.
        if let Some((ref attempted_sid, _, _)) = self.pending_resume_retry {
            if attempted_sid == bad_sid {
                self.confirmed_stale_resume_retry = self.pending_resume_retry.take();
            }
        }
    }

    /// Attempts to adopt `sid` as the live session id. Returns `false`
    /// (does not adopt) if a session id is already held, or if `sid` is
    /// the confirmed-poisoned id from a prior `poison_resume` call — the
    /// CLI echoes back whatever `--resume` it was given as its first
    /// stdout line even when that resume goes on to fail, so without this
    /// check a losing race would silently re-adopt a known-dead id right
    /// after `poison_resume` cleared it. A genuinely different (fresh)
    /// sid is unaffected and still captured normally.
    fn try_capture_session_id(&mut self, sid: &str) -> bool {
        if self.session_id.is_some() || self.resume_poisoned.as_deref() == Some(sid) {
            return false;
        }
        self.session_id = Some(sid.to_string());
        // Any successful capture — a resumed conversation confirming the
        // sid it was given, or a fresh conversation the CLI started on its
        // own — proves this spawn is genuinely progressing. Neither the
        // tentative nor the (mutually-exclusive-in-practice, but cleared
        // defensively) confirmed retry is needed anymore.
        self.pending_resume_retry = None;
        self.confirmed_stale_resume_retry = None;
        true
    }
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
    /// Weak self-reference for the stale-`--resume`-session retry (see
    /// `retry_after_resume_failure`) — set by `set_self_ref` right after
    /// construction. The process-waiter task (a detached `tokio::spawn`
    /// that only captures cloned Arc fields, never `&self`) needs a way to
    /// call back into an instance method once the underlying process
    /// actually exits; a `Weak` avoids a reference cycle (this same
    /// struct's `spawn_process` is what schedules that task).
    self_ref: Mutex<Option<std::sync::Weak<Self>>>,
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
            wstore.clone(),
            event_bus.clone(),
        ));
        Self {
            tab_id,
            block_id,
            inner: Arc::new(Mutex::new(PersistentInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                session_id: None,
                resume_poisoned: None,
                pending_resume_retry: None,
                confirmed_stale_resume_retry: None,
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
            self_ref: Mutex::new(None),
        }
    }

    /// Sets the weak self-reference used by the process-waiter task to call
    /// back into `retry_after_resume_failure` once a doomed process (stale
    /// `--resume` session id) actually exits. Mirrors `SubprocessController::
    /// set_self_ref`'s queued-message-drain pattern. Must be called by the
    /// caller right after wrapping a fresh instance in `Arc` — a controller
    /// that's never had this called simply never retries (the check is a
    /// harmless no-op `Weak::upgrade()` failure), so this is safe to skip
    /// for e.g. throwaway test instances.
    pub fn set_self_ref(self: &Arc<Self>) {
        *self.self_ref.lock().unwrap() = Some(Arc::downgrade(self));
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
            turn_active: self.health_monitor.is_active_turn(),
        }
    }

    fn publish_status(&self) {
        if let Some(ref broker) = self.broker {
            let status = self.get_status_snapshot();
            super::publish_controller_status(broker, &status);
        }
    }

    /// Periodic (low-frequency) republish of the current controllerstatus
    /// while a turn is active — a self-healing backstop independent of
    /// `publish_controller_status`'s `persist: 1` (which only helps a
    /// reconnecting subscriber) and the frontend's focus-triggered reconcile
    /// (which only fires on a background→foreground transition). If a single
    /// live push is missed for some other reason (e.g. a throttled/backgrounded
    /// renderer coalescing WS messages) while the window stays foregrounded
    /// and connected the whole time, nothing else corrects it until the turn
    /// actually ends. This shrinks that window from "forever" to at most one
    /// heartbeat interval. `HEARTBEAT_SECS` is well below the frontend's own
    /// `STUCK_THRESHOLD_MS` (45s, diagnostic-only) and `LIVENESS_RECOVERY_MS`
    /// (180s, force-recovery) so a missed push self-heals long before either
    /// of those fire. See REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md
    /// §4 item 5. Duplicates `get_status_snapshot`'s field construction
    /// rather than calling it, since the spawned task only holds cloned
    /// `Arc`s, not `&self` — matches the existing precedent noted on
    /// `core::spawn_health_watchdog` ("duplicated verbatim... before this
    /// extraction"); worth factoring out if a second controller type needs
    /// the same heartbeat.
    ///
    /// Same latent duplicate-loop race as `spawn_health_watchdog`'s existing,
    /// already-accepted contract (reagent P2 on the PR that introduced this
    /// function): if a turn ends and a new one starts again within one
    /// `HEARTBEAT_SECS` window, the old loop hasn't yet woken up to observe
    /// `is_active_turn() == false` and break, so both the old and new loop
    /// can run concurrently for that window. Harmless — `publish_status`
    /// republishing the same (or a slightly stale) snapshot twice is
    /// idempotent from the frontend's point of view — so this is accepted
    /// rather than fixed, matching the pre-existing pattern.
    fn spawn_status_heartbeat(&self) {
        const HEARTBEAT_SECS: u64 = 20;
        self.spawn_status_heartbeat_with_interval(tokio::time::Duration::from_secs(HEARTBEAT_SECS));
    }

    /// Interval parameterized out of `spawn_status_heartbeat` so a test can
    /// drive it with a short, real (not virtual-clock) interval instead of
    /// waiting out `HEARTBEAT_SECS` — this crate doesn't enable tokio's
    /// `test-util` feature (needed for `start_paused`/`time::advance`), and
    /// adding it crate-wide for one test wasn't judged worth it.
    fn spawn_status_heartbeat_with_interval(&self, heartbeat_interval: tokio::time::Duration) {
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let broker = self.broker.clone();
        let health_monitor = Arc::clone(&self.health_monitor);
        tokio::spawn(async move {
            let Some(broker) = broker else { return };
            let mut interval = tokio::time::interval(heartbeat_interval);
            interval.tick().await; // first tick is immediate; publish_status() already ran at turn start
            loop {
                interval.tick().await;
                let still_active = health_monitor.is_active_turn();
                let status = {
                    let g = inner.lock().unwrap();
                    BlockControllerRuntimeStatus {
                        blockid: block_id.clone(),
                        version: g.status_version,
                        shellprocstatus: g.proc_status.clone(),
                        shellprocconnname: "local".to_string(),
                        shellprocexitcode: g.proc_exit_code,
                        spawn_ts_ms: None,
                        is_agent_pane: true,
                        turn_active: still_active,
                    }
                };
                super::publish_controller_status(&broker, &status);
                // Publish the final `turn_active: false` snapshot BEFORE
                // exiting, not just break silently — reagent P1: this
                // heartbeat exists specifically to backstop a missed live
                // turn-end push (the exact stuck-"Working"-forever bug this
                // PR fixes). Breaking without this last publish meant the
                // one case it's most needed for — the terminal push being
                // the one that got dropped — was the one case it didn't
                // help.
                if !still_active {
                    break;
                }
            }
        });
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
        // Format as stream-json user message. Computed BEFORE spawning so a
        // fresh spawn can hand it straight to spawn_process, which stashes
        // it as the resume-retry payload SYNCHRONOUSLY — before any
        // background task (including the process-waiter that later reads
        // it back) even exists. reagentx P1 on PR #2360: stashing it here
        // instead, after spawn_process returned, left a window where a
        // process that dies fast enough (the exact case this exists to
        // catch) lets the already-scheduled, concurrently-running waiter
        // task observe the exit and take() this payload as still `None`,
        // silently losing the retry for the very case it's meant to catch.
        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });
        let json_str = json_msg.to_string();

        // Spawn process if not running
        let is_fresh_spawn = !self.is_running();
        if is_fresh_spawn {
            self.spawn_process(config.clone(), Some(json_str.clone()))?;
        }
        // spawn_process already marks a fresh process's first turn active
        // (and starts its watchdog); for an already-running process (the
        // common case — every turn after the first) this is the only place
        // that re-marks the turn active, since the persistent process never
        // exits between turns. Without this, `turn_active` would go stale
        // after turn 1 and never distinguish "generating" from "idle between
        // turns" again. The watchdog spawned per-turn (`spawn_health_watchdog`
        // exits as soon as `is_active_turn()` goes false — see
        // `core::spawn_health_watchdog`'s doc comment) also needs
        // re-arming here, but only when actually resuming from idle: a
        // mid-turn steering send (`send_user_message`) already has one
        // running, so re-spawning on every call would leak duplicate
        // watchdog tasks. `mark_turn_active_returning_was_active` reads and
        // flips the flag under one lock — a separate is_active_turn() +
        // set_active_turn(true) would race a concurrent send_user_message
        // (muxbus delivery) on the same block, letting both observe `false`
        // and both spawn a watchdog.
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
            core::spawn_health_watchdog(&self.health_monitor);
            self.spawn_status_heartbeat();
        }
        // Publish the turn_active flip so the Swarm view's live
        // ControllerStatus subscription picks it up immediately instead of
        // waiting for the next unrelated status change (or process exit).
        self.publish_status();

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

    /// Retries the message that triggered a `--resume <sid>` attempt this
    /// controller's own stderr reader just confirmed is unreachable ("No
    /// conversation found with session ID" — see `poison_resume`). Called
    /// from the process-waiter task once the doomed process has actually
    /// exited, via the weak self-reference (mirrors `SubprocessController`'s
    /// queued-message drain — see `set_self_ref`), and ONLY when
    /// `confirmed_stale_resume_retry` was actually set — never for an
    /// unrelated exit.
    ///
    /// Spawns fresh with `session_id` cleared, so no `--resume` is attempted
    /// again. Explicitly clears `inner.session_id` itself rather than
    /// relying on `poison_resume` having already done so — reagentx P0 on PR
    /// #2360: `poison_resume` runs on the stderr-reader task, this runs on
    /// the process-waiter task; the two are independently scheduled with no
    /// ordering guarantee, so this function cannot assume the former has
    /// already completed by the time it runs. Redelivers the EXACT
    /// already-formatted stdin line directly on the new process — does NOT
    /// re-persist to the blockfile or re-emit `agent-message-accepted`,
    /// since both already happened correctly on the original (failed)
    /// attempt; only the underlying CLI process needed a fresh, resume-less
    /// start.
    fn retry_after_resume_failure(&self, mut config: PersistentSpawnConfig, json_str: String) {
        config.session_id = String::new();
        self.inner.lock().unwrap().session_id = None;
        if let Err(e) = self.spawn_process(config, None) {
            tracing::error!(
                block_id = %self.block_id,
                error = %e,
                "failed to respawn after a stale --resume session id"
            );
            return;
        }
        // Mirrors send_message's own post-spawn turn-active bookkeeping —
        // this retry IS the turn's real first (and only user-visible) send,
        // just deferred past one doomed process.
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
            core::spawn_health_watchdog(&self.health_monitor);
            self.spawn_status_heartbeat();
        }
        self.publish_status();

        let inner = self.inner.lock().unwrap();
        let Some(tx) = inner.stdin_tx.as_ref() else {
            tracing::error!(
                block_id = %self.block_id,
                "stale-resume retry spawn reported success but stdin_tx is unset"
            );
            return;
        };
        if let Err(e) = tx.try_send(json_str) {
            tracing::warn!(
                block_id = %self.block_id,
                error = %e,
                "failed to redeliver message after stale-resume retry spawn"
            );
        }
    }

    /// Deliver a user message to the **already-running** persistent process,
    /// without a spawn config. Unlike `send_message`, this never spawns — it errors
    /// if the process is not running. Used for controller-aware muxbus/reactive
    /// delivery (`deliver_agent_message`), where the agent is live (busy or idle)
    /// and we have no `PersistentSpawnConfig` to hand. Writing on the live stdin lets
    /// the message land mid-turn (steering) instead of waiting for idle.
    /// Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §6 (Phase 3).
    pub fn send_user_message(&self, message: String) -> Result<(), String> {
        // Whether the process was busy or idle, delivering this message
        // (re)starts an active turn — see the comment in `send_message`,
        // including the watchdog re-arm-only-if-was-idle rationale and why
        // this must be the atomic read-and-set (send_message and
        // send_user_message can race on the same block).
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
            core::spawn_health_watchdog(&self.health_monitor);
            self.spawn_status_heartbeat();
        }
        self.publish_status();

        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });
        let json_str = json_msg.to_string();

        {
            let inner = self.inner.lock().unwrap();
            let tx = inner
                .stdin_tx
                .as_ref()
                .ok_or("persistent process not running")?;
            tx.try_send(json_str.clone())
                .map_err(|e| format!("stdin send failed: {e}"))?;
        }

        // Persist the injected message to the blockfile WITH a live event —
        // unlike `send_message`, there is no `agent-message-accepted` pending
        // echo to pair with (nothing was typed in the UI), so without this the
        // injection is invisible to the human operator: a silent injection,
        // which SPEC_JEKT_SECURITY_AND_VISIBILITY §3.1/G1 forbids. The live
        // blockfile append renders it in the open pane; the persisted line lets
        // `parseHistoryLines` rebuild the node on reopen.
        if let Some(ref broker) = self.broker {
            let global_zone = super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
            let line_with_newline = format!("{json_str}\n");
            super::shell::handle_append_block_file(
                broker,
                &self.block_id,
                crate::backend::agent_session::OUTPUT_FILE,
                line_with_newline.as_bytes(),
                self.filestore.as_ref(),
                global_zone.as_deref(),
            );
        }
        Ok(())
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
    /// `pending_questions` is in-memory-only, scoped to THIS controller
    /// instance — a fresh instance (pane reopen, or any process respawn)
    /// starts with an empty map even though the persisted transcript can
    /// still show the question as the tail node (deliberately preserved by
    /// `scrubOrphanedInProgress` as "may still be answerable"). The frontend
    /// (`useAgentQuestions.ts`'s `SAFE_TO_RETRY_VIA_FOLLOWUP` allowlist)
    /// matches on this error's text (the "no pending AskUserQuestion" prefix)
    /// to redeliver as a follow-up message instead of rolling back — keep
    /// that exact prefix stable if this message ever changes. See
    /// docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §2.7/§2.8.
    pub fn answer_question(&self, tool_use_id: String, answers: serde_json::Value) -> Result<(), String> {
        let (request_id, questions, tx) = {
            let mut inner = self.inner.lock().unwrap();
            let (rid, qs) = inner
                .pending_questions
                .remove(&tool_use_id)
                .ok_or_else(|| format!(
                    "no pending AskUserQuestion for tool_use_id {tool_use_id} — this controller \
                     instance never recorded it (process likely respawned since the question was \
                     asked, e.g. a pane close/reopen); the caller should redeliver as a follow-up message"
                ))?;
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
    fn spawn_process(&self, config: PersistentSpawnConfig, resume_retry_payload: Option<String>) -> Result<(), String> {
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
        // Recorded so the stderr reader can tell "No conversation found" apart
        // from an unrelated CLI error, and so it knows exactly which id to
        // poison against the stdout reader's own capture (see below) — a
        // provider that echoes back whatever --resume it was given as its
        // first stdout line, even when that id turns out to be unreachable.
        let mut attempted_resume_sid: Option<String> = None;
        {
            let inner = self.inner.lock().unwrap();
            if let Some(ref sid) = inner.session_id {
                if !config.resume_flag.is_empty() {
                    spawn_args.push(config.resume_flag.clone());
                    spawn_args.push(sid.clone());
                    attempted_resume_sid = Some(sid.clone());
                }
            }
        }
        cmd.args(&spawn_args);

        core::apply_working_dir(&mut cmd, &self.block_id, &config.working_dir, &config.env_vars);

        // On Windows: suppress console-window allocation. The srv runs without a
        // console of its own, so spawning the agent CLI without CREATE_NO_WINDOW
        // makes Windows allocate a fresh console — which Windows 11's default-
        // terminal handler renders as a NEW Windows Terminal window. One leaks per
        // agent start / resume / respawn; a flapping or restart-heavy session
        // accumulates dozens. stdio is piped here, so the console is never needed.
        // See docs/retro/retro-windows-terminal-window-leak-2026-06-21.md.
        // Matches acp.rs / subprocess.rs; sibling of shell.rs's PTY path.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "persistent process spawn failed");
            format!("failed to spawn persistent process: {e}")
        })?;

        // Stash the TENTATIVE resume-retry payload synchronously, right
        // here — before any background task (stdin writer, stdout/stderr
        // readers, process-waiter) is created below. reagentx P1 on PR
        // #2360: stashing this later, back in send_message after this
        // function returned, left a window where a process that dies fast
        // enough (the exact case this exists to catch) lets the
        // process-waiter task — already racing on another thread once it's
        // spawned — observe the exit and take() this payload while it's
        // still `None`, silently losing the retry for the very case it's
        // meant to catch. Keyed on the EXACT sid this spawn attempted (not
        // `config.session_id`, which can differ from what's actually held
        // in `inner.session_id` once an earlier call has already
        // hydrated it) so `poison_resume`'s later confirmation check is
        // unambiguous.
        if let (Some(sid), Some(retry_json)) = (attempted_resume_sid.clone(), resume_retry_payload) {
            let mut inner = self.inner.lock().unwrap();
            inner.pending_resume_retry = Some((sid, config.clone(), retry_json));
        }

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
        let stdin = child.stdin.take()
            .ok_or_else(|| format!("[persistent] stdin not captured for block {}", self.block_id))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| format!("[persistent] stdout not captured for block {}", self.block_id))?;
        let stderr = child.stderr.take();

        // Drain stderr in background — log lines for debugging. The
        // JoinHandle is kept (not discarded) so the process-waiter task
        // can await this task's full completion before deciding whether a
        // stale-resume retry was confirmed — codex P1/P2 on PR #2360
        // (second review pass): `child.wait()` resolving is NOT proof this
        // task has already seen and reacted to a "No conversation found"
        // line, or finished its OWN subsequent `persist_session_id("")`
        // call below. See the process-waiter's own comment for the two
        // failure modes this closes.
        let stderr_reader_handle: Option<tokio::task::JoinHandle<()>> = stderr.map(|stderr_pipe| {
            let block_id_stderr = self.block_id.clone();
            let inner_stderr = Arc::clone(&self.inner);
            let wstore_stderr = self.wstore.clone();
            let event_bus_stderr = self.event_bus.clone();
            let attempted_resume_sid = attempted_resume_sid.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr_pipe).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::warn!(
                        block_id = %block_id_stderr,
                        line = %line,
                        "persistent stderr"
                    );
                    // Claude Code's own message when `--resume <sid>` targets a
                    // conversation its current CLAUDE_CONFIG_DIR can't see — e.g.
                    // after a relogin/reseed moves the agent onto a different
                    // config dir than the one the session was recorded under. Left
                    // uncleared, EVERY future respawn (one per message, since a
                    // dead persistent process auto-restarts on next send) keeps
                    // retrying the same unreachable --resume and immediately
                    // exits again — a permanent "Agent encountered an error" with
                    // no path to recovery. Clear it so the next respawn starts a
                    // fresh conversation instead.
                    if line.contains("No conversation found with session ID") {
                        if let Some(ref bad_sid) = attempted_resume_sid {
                            // See PersistentInner::poison_resume — also guards
                            // against the stdout reader's own capture (below)
                            // re-adopting this same dead id if it wins the race.
                            inner_stderr.lock().unwrap().poison_resume(bad_sid);
                            tracing::warn!(
                                block_id = %block_id_stderr,
                                session_id = %bad_sid,
                                "stale --resume session id unreachable under the current config dir — \
                                 clearing so the next message starts a fresh conversation"
                            );
                            core::persist_session_id(&block_id_stderr, "", &wstore_stderr, &event_bus_stderr);
                            // Surface this to the user — previously silent
                            // (only the warn! above). See
                            // SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md
                            // §4.2: a resumed conversation silently starting
                            // fresh, with no indication anything happened, is
                            // exactly the failure mode this flag exists to close.
                            if let Some(ref store) = wstore_stderr {
                                crate::backend::blockcontroller::session_recovery::mark_resume_failed(
                                    store,
                                    &event_bus_stderr,
                                    &block_id_stderr,
                                );
                            }
                        }
                    }
                }
            })
        });

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

            // NOTE: OSC window-title extraction is NOT done here.
            // PersistentSubprocessController uses piped stdout with stream-json
            // NDJSON protocol. Claude Code sets window titles via process.title
            // (SetConsoleTitle on Windows; argv[0] on Unix), which does NOT
            // produce OSC escape sequences in the piped stdout stream. Inserting
            // OSC bytes into stream-json stdout would corrupt the JSON protocol.
            // block:activity events for agent panes are instead published by
            // the terminalSequence hooks path — see spec §2.5 and the future
            // SPEC_AGENT_HOOKS_TERMINAL_SEQUENCE spec.

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
                    // Claude's turn-ending marker. Persistent mode never exits
                    // between turns, so this is the only place `turn_active`
                    // can go back to false without waiting for process exit —
                    // see `send_message`'s matching `set_active_turn(true)`.
                    if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
                        health_read.set_active_turn(false);
                        // Publish the flip so the Swarm view's live
                        // ControllerStatus subscription reflects "turn
                        // ended" immediately instead of only on the next
                        // unrelated status change (or process exit) — see
                        // send_message's matching publish_status() call for
                        // the turn-start side of this pair.
                        if let Some(ref broker) = broker_read {
                            let status = {
                                let locked = inner_read.lock().unwrap();
                                BlockControllerRuntimeStatus {
                                    blockid: block_id_read.clone(),
                                    version: locked.status_version,
                                    shellprocstatus: locked.proc_status.clone(),
                                    shellprocconnname: "local".to_string(),
                                    shellprocexitcode: locked.proc_exit_code,
                                    spawn_ts_ms: None,
                                    is_agent_pane: true,
                                    turn_active: false,
                                }
                            };
                            super::publish_controller_status(broker, &status);
                        }
                        // SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20
                        // Phase A: reconcile any subagent still Active for this
                        // block the instant its turn ends, not just at the next
                        // pane reopen — closes SPEC_SUBAGENT_LIFECYCLE_
                        // RECONCILIATION_2026_07_12.md's Open Question 1. A
                        // subagent runs inside the parent's own CLI process (a
                        // Task-tool call is synchronous within the parent's
                        // turn), so this is the same "turn ended" signal
                        // scan_session_subagents already reconciles against at
                        // reopen — just fired live instead of waiting.
                        // `global()` is `None` in tests that don't call
                        // `subagent_watcher::set_global` — a safe no-op, same
                        // pattern `process_tracker::registry` already uses.
                        let session_id_snapshot = inner_read.lock().unwrap().session_id.clone();
                        if let Some(sid) = session_id_snapshot {
                            if let Some(watcher) = subagent_watcher::global() {
                                watcher.reconcile_stale_subagents(&block_id_read, &sid);
                            }
                        }
                    }
                    if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                        let sid_string = sid.to_string();
                        // See PersistentInner::try_capture_session_id — refuses
                        // to (re-)adopt an id the stderr reader (above) already
                        // confirmed unreachable, whichever task wins the race.
                        let should_capture =
                            inner_read.lock().unwrap().try_capture_session_id(&sid_string);
                        if should_capture {
                            tracing::info!(
                                block_id = %block_id_read,
                                session_id = %sid_string,
                                "persistent session ID captured"
                            );
                            core::persist_session_id(&block_id_read, &sid_string, &wstore_read, &event_bus_read);
                        }
                    }
                }

                // Publish line as WPS blockfile event and write-through to FileStore
                // for persistent history (Phase 1.3).
                //
                // debug, not info: fires on EVERY output line a streaming agent
                // produces — the single largest contributor (~27%) to an
                // unrotated 406 MB launcher-log mirror on a real machine
                // (SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 P1). Default
                // production filter is info, so this is now suppressed unless
                // RUST_LOG=debug is set.
                tracing::debug!(
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
        core::spawn_health_watchdog(&self.health_monitor);
        self.spawn_status_heartbeat();

        // Spawn process waiter task
        let block_id_wait = self.block_id.clone();
        let inner_wait = Arc::clone(&self.inner);
        let broker_wait = self.broker.clone();
        let wstore_wait = self.wstore.clone();
        let health_wait = Arc::clone(&self.health_monitor);
        // Captured so the waiter can deregister this agent from muxbus on exit.
        let agent_id_wait = agent_id_for_muxbus.clone();
        // See `set_self_ref` / `retry_after_resume_failure` — lets this
        // detached task call back into an instance method once the process
        // actually exits, to transparently retry a stale-`--resume` failure.
        let self_ref_wait = self.self_ref.lock().unwrap().clone().unwrap_or_default();

        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                    tracing::info!(
                        block_id = %block_id_wait,
                        exit_code = exit_code,
                        "persistent process exited"
                    );

                    // Give the stderr reader a bounded chance to fully
                    // drain and react to whatever it saw right before this
                    // process exited — codex P1/P2 on PR #2360 (second
                    // review pass): `child.wait()` resolving does NOT mean
                    // the stderr reader (an independently-scheduled task)
                    // has already called `poison_resume` for a "No
                    // conversation found" line, or finished ITS OWN
                    // subsequent `persist_session_id("")` call. Without
                    // this, two failure modes were possible: (1) this task
                    // could clear `pending_resume_retry` below before the
                    // stderr reader ever promotes it, permanently losing
                    // the retry for the exact case it exists to catch, and
                    // (2) a confirmed retry's fresh session id (persisted
                    // by the NEW process's own stdout reader once
                    // respawned) could be silently overwritten by this
                    // exiting process's stderr task finally getting around
                    // to persisting an empty one, corrupting continuity
                    // despite the retry having succeeded. 500ms is
                    // generous — the stderr pipe closes and drains almost
                    // immediately once the process has genuinely exited —
                    // and only ever delays shutdown handling, never blocks
                    // it indefinitely.
                    if let Some(handle) = stderr_reader_handle {
                        if tokio::time::timeout(std::time::Duration::from_millis(500), handle).await.is_err() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stderr reader did not finish within 500ms of process exit"
                            );
                        }
                    }

                    // Notify health monitor so Stalled/Dead watchdog stops.
                    health_wait.set_exited(exit_code);

                    let mut inner = inner_wait.lock().unwrap();
                    inner.proc_exit_code = exit_code;
                    inner.current_pid = None;
                    inner.stdin_tx = None;
                    inner.kill_tx = None;
                    // Only `confirmed_stale_resume_retry` (never
                    // `pending_resume_retry`, which is merely "a resume was
                    // attempted, not-yet-known outcome") triggers a retry —
                    // see its doc comment and reagentx P1 on PR #2360. Taken
                    // (not just read) so it can only ever fire once. Any
                    // still-tentative `pending_resume_retry` is also
                    // dropped here — by now the stderr reader has already
                    // had its bounded chance to promote it above, so this
                    // process (successful or not) is genuinely done: it
                    // can never be confirmed either way, and a reused
                    // controller instance must not carry it into an
                    // unrelated later lifetime.
                    let retry_after_resume = inner.confirmed_stale_resume_retry.take();
                    inner.pending_resume_retry = None;
                    Self::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    // Deregister from muxbus so later sends fall through to the
                    // lower tiers instead of resolving to a dead block. Mirrors
                    // the shell controller's exit path. Done regardless of
                    // whether a retry follows — this exact process's
                    // resources are gone either way, and spawn_process's own
                    // fresh registration (if a retry follows) doesn't clean
                    // up state belonging to THIS dying process.
                    crate::backend::reactive::get_global_handler()
                        .unregister_block(&block_id_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::remove(&data_dir, agent_id);
                        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                            sub.remove_agent(agent_id);
                        }
                    }

                    // Clear active pid — clean exit, no recovery needed.
                    if let Some(ref wstore) = wstore_wait {
                        super::session_recovery::clear_active_pid(wstore, &block_id_wait);
                    }

                    // A stale `--resume <sid>` is exactly what killed this
                    // process (`retry_after_resume_failure`'s doc comment) —
                    // retry the same message once, fresh, WITHOUT publishing
                    // this transient failure as a completed turn first.
                    // codex P2 on PR #2360 (second review pass): publishing
                    // "done"/turn_active:false here would let the mounted
                    // UI (trackTurnJustEnded, a deferred controller refresh)
                    // treat this failed attempt as the real end of the
                    // user's turn before the retry's own fresh "running"
                    // status ever lands. reagentx/codex never reviewed this
                    // controller type in PR #2338 (see docs/retros/
                    // RETRO_STALE_RESUME_SESSION_ID_ACROSS_CHANNELS_2026_07_29.md).
                    if let Some((_attempted_sid, retry_config, retry_json)) = retry_after_resume {
                        if let Some(ctrl) = self_ref_wait.upgrade() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stale --resume session id caused this exit — retrying fresh, without --resume"
                            );
                            ctrl.retry_after_resume_failure(retry_config, retry_json);
                        }
                    } else if let Some(ref broker) = broker_wait {
                        // Genuinely done, not retrying — publish the
                        // terminal status.
                        let status = BlockControllerRuntimeStatus {
                            blockid: block_id_wait.clone(),
                            version: 0,
                            shellprocstatus: STATUS_DONE.to_string(),
                            shellprocconnname: "local".to_string(),
                            shellprocexitcode: exit_code,
                            spawn_ts_ms: None,
                            is_agent_pane: true,
                            turn_active: false,
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
                    // A user-initiated kill is unrelated to any stale-resume
                    // failure in flight; drop both so a future, unrelated
                    // exit on a REUSED controller instance (resync_controller
                    // can reuse the same instance across a kill+restart
                    // cycle) never retries a message from this now-dead
                    // lifetime.
                    inner.pending_resume_retry = None;
                    inner.confirmed_stale_resume_retry = None;
                    Self::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    // Deregister from muxbus (see the clean-exit arm above).
                    crate::backend::reactive::get_global_handler()
                        .unregister_block(&block_id_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::remove(&data_dir, agent_id);
                        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                            sub.remove_agent(agent_id);
                        }
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

    /// Regression for REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md
    /// §2.7/§2.8: a fresh controller instance (pane reopen, or any process
    /// respawn) has an empty `pending_questions` map. Confirms the error
    /// message is descriptive enough for `muxlog` diagnosis — the frontend
    /// no longer depends on matching this exact string (it falls back on
    /// ANY answer_question failure now), but a clear message still matters
    /// for debugging a future recurrence.
    #[test]
    fn answer_question_on_untracked_tool_use_id_is_descriptive() {
        let c = controller();
        let err = c
            .answer_question("tu-unknown".to_string(), serde_json::json!({}))
            .unwrap_err();
        assert!(
            err.contains("tu-unknown") && err.contains("respawned"),
            "error should name the tool_use_id and explain the likely cause, got {err:?}"
        );
    }

    // `turn_active` on the runtime status snapshot must track the health
    // monitor's active-turn flag directly — this is the signal the frontend
    // seeds TurnPhase from at mount instead of always defaulting to Idle
    // (see docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    // Finding 1). Exercised here via the health monitor directly rather than
    // send_message()/the stdout reader, which both require a real spawned
    // process.
    #[test]
    fn status_snapshot_turn_active_tracks_health_monitor() {
        let c = controller();
        assert!(
            !c.get_status_snapshot().turn_active,
            "freshly constructed controller has no turn in flight"
        );

        c.health_monitor.set_active_turn(true);
        assert!(
            c.get_status_snapshot().turn_active,
            "turn_active must flip true once the health monitor marks a turn active"
        );

        c.health_monitor.set_active_turn(false);
        assert!(
            !c.get_status_snapshot().turn_active,
            "turn_active must flip back false once the turn ends"
        );
    }

    /// Regression for reagent P2 (persist-controllerstatus PR): confirms
    /// `spawn_status_heartbeat` actually republishes while a turn stays
    /// active, using a short real interval (`spawn_status_heartbeat_with_interval`)
    /// instead of waiting out the production 20s — this crate doesn't enable
    /// tokio's `test-util` feature, so a real (short) interval is used
    /// rather than a virtual/paused clock.
    #[tokio::test]
    async fn status_heartbeat_republishes_while_active() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            "block-heartbeat".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        );
        c.health_monitor.set_active_turn(true);
        c.spawn_status_heartbeat_with_interval(tokio::time::Duration::from_millis(5));

        // Generous margin over several 5ms ticks — proves at least one
        // heartbeat tick actually published, without asserting an exact count
        // (real-time scheduling, not virtual-clock-deterministic).
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let history = broker.read_event_history(
            crate::backend::wps::EVENT_CONTROLLER_STATUS,
            "block:block-heartbeat",
            1,
        );
        assert_eq!(history.len(), 1, "heartbeat must have published at least once while active");
        let status: BlockControllerRuntimeStatus =
            serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
        assert!(status.turn_active, "published snapshot must reflect the active turn");
    }

    /// Regression for reagent P1 (round 2 on the persist-controllerstatus
    /// PR): the heartbeat loop must publish one final `turn_active: false`
    /// snapshot before exiting, not just break silently. This is the exact
    /// case the heartbeat exists to backstop — a missed live turn-end
    /// push — so a silent exit with no final publish would leave the
    /// client stuck showing "Working" in precisely the scenario this whole
    /// mechanism was built for.
    #[tokio::test]
    async fn status_heartbeat_publishes_final_inactive_status_before_stopping() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            "block-heartbeat-stop".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        );
        c.health_monitor.set_active_turn(true);
        c.spawn_status_heartbeat_with_interval(tokio::time::Duration::from_millis(5));

        // Let at least one active-turn tick land, then mark the turn ended —
        // simulating the exact scenario: the "real" turn-end publish (from
        // wherever normally calls publish_controller_status on completion)
        // is the one that got dropped, and the heartbeat is the only thing
        // left that can correct the client's stale "Working" state.
        tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
        c.health_monitor.set_active_turn(false);
        tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_CONTROLLER_STATUS,
            "block:block-heartbeat-stop",
            1,
        );
        assert_eq!(history.len(), 1, "must have published at least the final status");
        let status: BlockControllerRuntimeStatus =
            serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
        assert!(
            !status.turn_active,
            "the heartbeat's last publish before stopping must reflect turn_active: false, \
             not silently disappear leaving the client on a stale turn_active: true"
        );
    }

    /// reagentx P0 on PR #2360: `poison_resume` (the stderr-reader task) and
    /// `retry_after_resume_failure` (called from the process-waiter task)
    /// are two independently-scheduled tasks with no ordering guarantee —
    /// this must clear `inner.session_id` itself rather than assuming
    /// `poison_resume` already ran first. Uses a nonexistent binary so the
    /// respawn attempt inside this function fails fast (no real process
    /// needed) — the assertion only cares that `inner.session_id` was
    /// cleared BEFORE that attempt, which is what stops a later, genuinely
    /// successful respawn from ever re-attaching `--resume` to the same
    /// dead id.
    #[test]
    fn retry_after_resume_failure_clears_inner_session_id_even_when_poison_resume_has_not_run_yet() {
        let c = controller();
        c.inner.lock().unwrap().session_id = Some("dead-sid".to_string());

        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        };
        c.retry_after_resume_failure(config, "{}".to_string());

        assert_eq!(
            c.inner.lock().unwrap().session_id,
            None,
            "must clear inner.session_id directly, not rely on poison_resume having already done so"
        );
    }

    /// codex P1/P2 on PR #2360 (second review pass): the process-waiter
    /// task now awaits the stderr reader's `JoinHandle` (bounded by a
    /// timeout) before deciding whether a stale-resume retry was
    /// confirmed, and before publishing a terminal status — otherwise
    /// `child.wait()` resolving first could (1) wipe the tentative retry
    /// before the stderr reader ever promotes it, and (2) let a
    /// confirmed retry's fresh session id get overwritten by the stderr
    /// task's own delayed `persist_session_id("")` call. This isn't a
    /// full subprocess integration test (this module's established
    /// precedent — see its own doc comment — avoids spawning a real CLI
    /// process for this exact subsystem); it confirms the underlying
    /// synchronization primitive itself: a task that completes well
    /// within the bound is FULLY awaited — its side effect is guaranteed
    /// observable — before the timeout could possibly race it.
    #[tokio::test]
    async fn a_join_handle_completing_within_the_bound_is_fully_awaited_first() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            flag_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle).await;

        assert!(result.is_ok(), "a task well within the bound must not be treated as timed out");
        assert!(
            flag.load(std::sync::atomic::Ordering::SeqCst),
            "awaiting the handle must observe the task's side effect having already happened, \
             not race ahead of it"
        );
    }
}

/// Covers the stderr-poison / stdout-capture race directly against
/// `PersistentInner`'s decision logic, without spawning a real CLI
/// process — see `poison_resume` / `try_capture_session_id`.
#[cfg(test)]
mod resume_poison_tests {
    use super::*;

    fn inner_with_session_id(session_id: Option<&str>) -> PersistentInner {
        PersistentInner {
            proc_status: STATUS_INIT.to_string(),
            proc_exit_code: 0,
            status_version: 0,
            session_id: session_id.map(str::to_string),
            resume_poisoned: None,
            pending_resume_retry: None,
            confirmed_stale_resume_retry: None,
            current_pid: None,
            stdin_tx: None,
            kill_tx: None,
            pending_questions: HashMap::new(),
        }
    }

    fn dummy_spawn_config() -> PersistentSpawnConfig {
        PersistentSpawnConfig {
            cli_command: "claude".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        }
    }

    // stderr wins the race: poisons the id and clears it from session_id,
    // then the stdout reader's later echo of the same dead id is refused.
    #[test]
    fn stderr_first_then_stdout_echo_is_refused() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        inner.poison_resume("dead-sid");
        assert_eq!(inner.session_id, None, "poisoning the live session id clears it");

        let captured = inner.try_capture_session_id("dead-sid");
        assert!(!captured, "must refuse to re-adopt a confirmed-poisoned id");
        assert_eq!(inner.session_id, None);
    }

    // stdout wins the race (echoes the dead id before stderr's "No
    // conversation found" arrives): the later poison must still clear it.
    #[test]
    fn stdout_first_then_stderr_poison_still_clears() {
        let mut inner = inner_with_session_id(None);
        let captured = inner.try_capture_session_id("dead-sid");
        assert!(captured, "first capture with no prior state succeeds");
        assert_eq!(inner.session_id.as_deref(), Some("dead-sid"));

        inner.poison_resume("dead-sid");
        assert_eq!(inner.session_id, None, "poison must clear it even though stdout set it first");
    }

    // A genuinely fresh session id (the CLI gave up on --resume and started
    // a new conversation) is unaffected by an unrelated prior poison.
    #[test]
    fn different_fresh_session_id_is_captured_normally() {
        let mut inner = inner_with_session_id(None);
        inner.poison_resume("dead-sid");

        let captured = inner.try_capture_session_id("fresh-sid");
        assert!(captured, "a different id is not blocked by an unrelated poison");
        assert_eq!(inner.session_id.as_deref(), Some("fresh-sid"));
    }

    // Once a session id is already held, a second stdout line (e.g. a
    // duplicate echo) must not overwrite it.
    #[test]
    fn does_not_overwrite_an_already_captured_session_id() {
        let mut inner = inner_with_session_id(Some("first-sid"));
        let captured = inner.try_capture_session_id("second-sid");
        assert!(!captured, "must not overwrite an already-captured session id");
        assert_eq!(inner.session_id.as_deref(), Some("first-sid"));
    }

    // reagentx/codex never reviewed this path (PR #2338's review surface was
    // the ACP controller only) — see docs/retros/
    // RETRO_STALE_RESUME_SESSION_ID_ACROSS_CHANNELS_2026_07_29.md. A stale
    // `--resume <sid>` (first-ever use of a globally-known agent's session
    // under a brand-new build/channel's own CLI install) used to lose the
    // triggering message and surface a generic "Agent encountered an error"
    // — the safety net is `pending_resume_retry`/`confirmed_stale_resume_retry`,
    // and its correctness hinges entirely on promoting/clearing them if and
    // only if the spawn they were captured for actually confirmed dead or
    // made real progress.
    #[test]
    fn pending_resume_retry_is_cleared_once_the_resume_actually_succeeds() {
        let mut inner = inner_with_session_id(None);
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), "{}".to_string()));

        // The CLI echoes back the SAME sid it was given, confirming --resume
        // actually worked — this is genuine progress, so the retry safety
        // net is no longer needed.
        let captured = inner.try_capture_session_id("dead-sid");
        assert!(captured);
        assert!(
            inner.pending_resume_retry.is_none(),
            "a successful resume must stand down the retry safety net"
        );
        assert!(inner.confirmed_stale_resume_retry.is_none());
    }

    #[test]
    fn pending_resume_retry_is_cleared_when_the_cli_starts_a_fresh_conversation_on_its_own() {
        let mut inner = inner_with_session_id(None);
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), "{}".to_string()));

        // A DIFFERENT (fresh) sid — the CLI gave up on --resume internally
        // and started its own new conversation without ever hitting the
        // stderr "No conversation found" path. Also genuine progress.
        let captured = inner.try_capture_session_id("brand-new-sid");
        assert!(captured);
        assert!(inner.pending_resume_retry.is_none());
        assert!(inner.confirmed_stale_resume_retry.is_none());
    }

    // reagentx P1 on PR #2360: poison_resume must promote a MATCHING
    // tentative retry to confirmed — this is the ONLY path the
    // process-waiter task ever acts on. An earlier cut of this fix checked
    // `pending_resume_retry` directly, which retried on ANY exit before the
    // first session id capture — including an unrelated auth failure,
    // network blip, or rate limit — silently discarding the user's existing
    // conversation with no disclosure.
    #[test]
    fn poison_resume_promotes_a_matching_pending_retry_to_confirmed() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), "{}".to_string()));

        inner.poison_resume("dead-sid");

        assert!(
            inner.pending_resume_retry.is_none(),
            "a promoted retry is taken out of the tentative slot"
        );
        assert!(
            inner.confirmed_stale_resume_retry.is_some(),
            "poisoning the EXACT sid this retry was captured for must confirm it"
        );
    }

    // The stdout reader's later echo of the same dead id (losing the race)
    // must still be refused, and must NOT clear the now-confirmed retry —
    // a refused (no-op) capture represents no progress at all.
    #[test]
    fn confirmed_retry_survives_a_subsequent_refused_capture() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), "{}".to_string()));
        inner.poison_resume("dead-sid");
        assert!(inner.confirmed_stale_resume_retry.is_some());

        let captured = inner.try_capture_session_id("dead-sid");
        assert!(!captured, "the poisoned id must still be refused");
        assert!(
            inner.confirmed_stale_resume_retry.is_some(),
            "a refused (no-op) capture must not clear the confirmed retry"
        );
    }

    // A tentative retry captured for a DIFFERENT (still in-flight, unrelated)
    // spawn attempt must NOT be promoted by a poison for some OTHER sid —
    // the matching is keyed on the exact attempted sid, not "any pending
    // retry, whatever it's for."
    #[test]
    fn poison_resume_does_not_promote_a_retry_captured_for_a_different_sid() {
        let mut inner = inner_with_session_id(None);
        inner.pending_resume_retry =
            Some(("other-sid".to_string(), dummy_spawn_config(), "{}".to_string()));

        inner.poison_resume("dead-sid");

        assert!(
            inner.pending_resume_retry.is_some(),
            "an unrelated tentative retry must survive a poison for a different sid"
        );
        assert!(
            inner.confirmed_stale_resume_retry.is_none(),
            "must not confirm a retry that wasn't captured for the poisoned sid"
        );
    }
}
