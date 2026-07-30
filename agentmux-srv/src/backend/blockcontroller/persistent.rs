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

use std::collections::{HashMap, VecDeque};
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
    /// `--resume <sid>` — captures the exact spawn config + the already-
    /// formatted stdin line for the message that triggered it, as the
    /// first entry of a growing list. This alone does NOT mean a retry
    /// will happen: it only means "IF the CLI goes on to confirm this
    /// exact sid unreachable, here is what to retry."
    ///
    /// The list, not a single line: codex P1 on PR #2360 (sixth review
    /// pass, round 5): once other callers can queue additional messages
    /// behind this same spawn attempt (see `spawning_in_progress`), the
    /// background drain (`drain_queue_after_successful_spawn`) can
    /// successfully hand SEVERAL of them to this process's stdin channel
    /// before it turns out to be doomed — the channel accepting a message
    /// is not proof the CLI ever read it. Tracking only the first would
    /// silently lose every later-accepted input the moment this process
    /// dies; the drain appends each one it delivers here (while this is
    /// still `Some`) so a confirmed retry can redeliver ALL of them, in
    /// order, on the fresh respawn.
    ///
    /// `poison_resume` is what promotes this to `confirmed_stale_resume_retry`
    /// (the only thing the process-waiter task actually acts on) — see its
    /// doc comment for why the two are kept separate. Cleared by
    /// `try_capture_session_id` on ANY successful capture (resume
    /// succeeded, or the CLI fell through to a fresh conversation on its
    /// own) since neither a retry nor a later confirmation is possible
    /// anymore; also cleared on kill and on a normal exit so a reused
    /// controller instance never carries a tentative retry into an
    /// unrelated later lifetime.
    pending_resume_retry: Option<(String, PersistentSpawnConfig, Vec<String>)>,
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
    confirmed_stale_resume_retry: Option<(String, PersistentSpawnConfig, Vec<String>)>,
    /// True from the moment a caller commits to calling `spawn_process`
    /// (in `send_message` or `retry_after_resume_failure`) until it has
    /// delivered every message enqueued for that spawn — see
    /// `pending_send_messages`. Checked and set together with
    /// `stdin_tx.is_some()` under this same lock, in one acquisition —
    /// reagentx P1 on PR #2360 (sixth review pass): `send_message`'s
    /// `is_running()` check and its `spawn_process()` call used to be two
    /// separate operations; a second concurrent `send_message` call (a
    /// genuine second RPC, or a muxbus delivery) landing in the gap
    /// between them could ALSO observe "not running" and independently
    /// spawn a second child process, orphaning one (leaked, unkillable via
    /// `stop_process`, unregistered from muxbus). Whichever caller sees
    /// `stdin_tx.is_none() && !spawning_in_progress` first becomes the
    /// sole spawner for this round; every other caller queues instead of
    /// racing its own spawn.
    ///
    /// This also fully subsumes the earlier `spawn_epoch`/
    /// `should_skip_own_delivery` mechanism (rounds 3-5 of this same PR):
    /// that check existed only to catch a delivery landing in the window
    /// between a caller's own `spawn_process` returning and its own
    /// tail-end stdin write — a window that no longer exists, since
    /// delivery now happens as part of the same atomic spawn-claim, before
    /// the lock is released (see `release_spawn_claim_and_drain_queue`).
    spawning_in_progress: bool,
    /// Messages queued while `spawning_in_progress` was `true` — includes
    /// the enqueuing caller's own message when it became the spawner (see
    /// `SendAction::BecomeSpawner`), so the post-spawn drain uses one
    /// uniform delivery path regardless of whether a message triggered the
    /// spawn or arrived while someone else's spawn was already in flight.
    /// Drained by `release_spawn_claim_and_drain_queue`.
    pending_send_messages: VecDeque<QueuedMessage>,
    /// True while the drain (`drain_queue_after_successful_spawn`) is
    /// between successfully sending a message on the live stdin channel
    /// and finishing its OWN follow-up append of that message into
    /// `pending_resume_retry`/`confirmed_stale_resume_retry` — see
    /// `pending_resume_retry`'s own doc comment for why that append
    /// exists. reagentx P1 on PR #2360 (sixth review pass, round 9): the
    /// `Sender::send().await` and that follow-up append are two separate
    /// lock acquisitions with an unavoidable gap between them (a mutex
    /// can't be held across an `.await`). Without this flag, the
    /// process-waiter's own exit-handling — running concurrently on a
    /// DIFFERENT task — could `.take()` the confirmed retry batch in that
    /// exact gap, dispatching a retry that's missing a message the doomed
    /// process's channel had ALREADY accepted: the message stays marked
    /// "accepted" and gets persisted, but is never actually delivered to
    /// any process again. The exit-handling waits (briefly, bounded) for
    /// this to go false before deciding the retry batch is final — see
    /// its own comment at the `.take()` call site.
    drain_send_in_flight: bool,
    current_pid: Option<u32>,
    /// Channel to send messages to the stdin writer task.
    stdin_tx: Option<mpsc::Sender<String>>,
    /// Handle to kill the process.
    kill_tx: Option<tokio::sync::oneshot::Sender<bool>>,
    /// Monotonic counter bumped once per `spawn_process` call (in the same
    /// lock acquisition as stashing `pending_resume_retry`), uniquely
    /// identifying that one spawn attempt for the rest of this controller
    /// instance's lifetime. Note this is NOT the `spawn_epoch`/
    /// `should_skip_own_delivery` mechanism removed earlier in this same
    /// PR (see `spawning_in_progress`'s own doc comment) — that existed to
    /// dedup a spawn-claim race, a job `spawning_in_progress` now fully
    /// owns. This counter exists for a different, narrower purpose: giving
    /// `stop_requested_generation` something stable to compare against.
    spawn_generation: u64,
    /// codex P1 on PR #2360 (round 16, commit ce1642d90): `stop_process`
    /// can race a process that already exited (a confirmed stale-`--resume`
    /// death) and is about to be silently retried — sending through
    /// `kill_tx` is futile in that window regardless of whether it's
    /// already `None` (the exit-handler cleared it) or still `Some`
    /// (`tokio::select!` already committed to the `child.wait()` exit arm
    /// before this call reached the lock, so the `kill_rx` arm will never
    /// be polled again even if the send succeeds). Without an independent
    /// signal, the exit-handler's retry decision has no way to know the
    /// user explicitly asked to stop, and respawns a fresh CLI process
    /// with the same prompt anyway — an explicit Stop that doesn't
    /// actually stop anything. Set to the CURRENT `spawn_generation`
    /// whenever `stop_process` runs (see its own doc comment); the
    /// exit-handler compares this against the generation IT was spawned
    /// with before deciding whether to retry, consuming (clearing) it
    /// either way so it can never spuriously affect a later, different
    /// generation — monotonic generation numbers are never reused, so an
    /// unconsumed stale value here is inert, not a leak.
    stop_requested_generation: Option<u64>,
    /// AskUserQuestion `can_use_tool` control_requests awaiting a user answer:
    /// `tool_use_id -> (request_id, questions JSON)`. Filled by the stdout
    /// reader when the CLI sends a `can_use_tool` control_request for
    /// AskUserQuestion; consumed by `answer_question` to build the matching
    /// `control_response`. Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    pending_questions: HashMap<String, (String, serde_json::Value)>,
    /// A not-yet-persisted `result`/`is_error:true` stdout line, held back
    /// by the stdout reader instead of being appended to the blockfile
    /// immediately, whenever `pending_resume_retry` OR
    /// `confirmed_stale_resume_retry` is still live at the moment it
    /// arrives — i.e. this is the doomed first attempt of a
    /// stale-`--resume` spawn, and a retry may still be confirmed (or has
    /// just been confirmed) for it. See
    /// `hold_back_error_result_line_if_resume_pending`'s own doc comment
    /// for why BOTH fields must be checked, not just the pending one
    /// (issue #2368: PR #2360's transparent retry succeeds within
    /// milliseconds, but this line had already been unconditionally
    /// persisted before the retry decision ran, leaving a permanent
    /// "Agent encountered an error" bubble even though the retry worked).
    /// The process-waiter resolves it once the retry decision is final:
    /// dropped entirely if a stale-resume retry is confirmed and about to
    /// fire, or flushed to the blockfile otherwise (a genuine first-turn
    /// error — auth failure, rate limit, or an explicit stop overriding a
    /// confirmed retry — must still reach the user). Single-slot, like
    /// `pending_resume_retry`: only one spawn is ever in flight at a time
    /// (`spawning_in_progress` guarantees this), so no generation tag is
    /// needed. Cleared (taken) by the process-waiter on every exit.
    pending_error_result_line: Option<String>,
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

    /// Returns `true` (and clears `stop_requested_generation`) if a stop
    /// was requested for THIS exact spawn generation — see that field's
    /// own doc comment for why `kill_tx` alone can't be trusted to signal
    /// this. Extracted as its own method (mirroring `poison_resume`/
    /// `try_capture_session_id` above) so this decision is directly unit
    /// testable without needing a real child process exit.
    fn consume_stop_requested_for_generation(&mut self, generation: u64) -> bool {
        if self.stop_requested_generation == Some(generation) {
            self.stop_requested_generation = None;
            true
        } else {
            false
        }
    }

    /// Called by the stdout reader for a terminal `result`/`is_error:true`
    /// line. Stashes `line_with_newline` in `pending_error_result_line`
    /// instead of persisting it immediately, when a stale-`--resume` retry
    /// could still be confirmed OR has already been confirmed for this
    /// exact attempt (issue #2368). Returns `true` if the line was held
    /// back (the caller must skip its normal blockfile append for this
    /// line), `false` if it should persist immediately as always.
    /// Extracted as its own method (mirroring `poison_resume`/
    /// `consume_stop_requested_for_generation` above) so this decision is
    /// directly unit testable without needing a real child process.
    ///
    /// Checks BOTH `pending_resume_retry` and `confirmed_stale_resume_retry`
    /// — not just the former. `poison_resume` runs on the independently-
    /// scheduled stderr-reader task and `.take()`s `pending_resume_retry`
    /// the moment it promotes a retry to confirmed; live-reproduced
    /// evidence (agent "Marks", 2026-07-30) showed the stderr reader's
    /// poison_resume call landing ~500ms before the stdout reader finished
    /// draining lines — by the time the stdout reader reached the doomed
    /// line, `pending_resume_retry` was already `None` (already promoted),
    /// so a check of that field alone missed the stash entirely and the
    /// original bug reproduced despite this mechanism existing. Checking
    /// `confirmed_stale_resume_retry` too closes that gap: once EITHER
    /// field is `Some`, this attempt has not yet been proven safe to
    /// persist immediately (it's either still tentative, or a confirmed
    /// retry is about to fire and drop this line entirely).
    fn hold_back_error_result_line_if_resume_pending(&mut self, line_with_newline: String) -> bool {
        if self.pending_resume_retry.is_some() || self.confirmed_stale_resume_retry.is_some() {
            self.pending_error_result_line = Some(line_with_newline);
            true
        } else {
            false
        }
    }
}

/// What `decide_send_action` determined a message's fate should be —
/// see its own doc comment and `PersistentInner::spawning_in_progress`.
enum SendAction {
    /// The process is already running — deliver directly, no spawn
    /// decision involved at all.
    DeliverDirect,
    /// Nobody else is currently spawning — this caller claimed the
    /// exclusive right to do so and its message has already been enqueued
    /// for the post-spawn drain.
    BecomeSpawner,
    /// Another caller is already spawning — this message has been
    /// enqueued for that caller's own post-spawn drain to deliver.
    Queued,
}

/// A single entry in `pending_send_messages`: the formatted stdin JSON
/// payload plus whether it's already been persisted to the blockfile.
/// codex P2 on PR #2360 (sixth review pass, round 7): a stale-resume
/// retry's redelivery has already been correctly persisted on its
/// original (failed) attempt — see `retry_after_resume_failure`'s own doc
/// comment — but the drain used to persist EVERY message it delivers
/// unconditionally, double-persisting a replayed retry batch into the
/// blockfile transcript.
#[derive(Clone, Debug)]
struct QueuedMessage {
    json_str: String,
    already_persisted: bool,
}

impl QueuedMessage {
    fn fresh(json_str: String) -> Self {
        Self { json_str, already_persisted: false }
    }

    fn already_persisted(json_str: String) -> Self {
        Self { json_str, already_persisted: true }
    }
}

impl PartialEq<str> for QueuedMessage {
    fn eq(&self, other: &str) -> bool {
        self.json_str == other
    }
}

impl PartialEq<&str> for QueuedMessage {
    fn eq(&self, other: &&str) -> bool {
        self.json_str == *other
    }
}

impl PartialEq<String> for QueuedMessage {
    fn eq(&self, other: &String) -> bool {
        self.json_str == *other
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
                spawning_in_progress: false,
                pending_send_messages: VecDeque::new(),
                drain_send_in_flight: false,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                spawn_generation: 0,
                stop_requested_generation: None,
                pending_questions: HashMap::new(),
                pending_error_result_line: None,
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

    /// Marks a turn active (re-arming the health watchdog and heartbeat
    /// only if it was previously idle) and publishes the resulting status
    /// flip. Shared by `send_message` and `retry_after_resume_failure` —
    /// both represent "a user message is about to be delivered," just via
    /// different spawn paths. See `send_message`'s original inline
    /// comment (now here) for why the watchdog is re-armed conditionally:
    /// a mid-turn steering send already has one running, so re-spawning on
    /// every call would leak duplicate watchdog tasks.
    fn mark_turn_active_and_publish(&self) {
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
            core::spawn_health_watchdog(&self.health_monitor);
            self.spawn_status_heartbeat();
        }
        self.publish_status();
    }

    /// Atomically decides what to do with `json_str`, given the caller
    /// wants it delivered to the persistent process — see
    /// `PersistentInner::spawning_in_progress`'s doc comment for the race
    /// this closes. All three outcomes are decided under ONE lock
    /// acquisition so nothing can slip through the gaps between them:
    /// - the process is already running AND nobody is still draining a
    ///   backlog into it → `DeliverDirect`, no spawn decision at all.
    /// - someone is currently spawning OR still draining a backlog
    ///   (`spawning_in_progress`) → `json_str` is enqueued for THAT
    ///   caller's own drain to deliver, and this call returns `Queued`
    ///   with nothing further to do. reagentx P1 on PR #2360 (sixth
    ///   review pass, round 4): `spawn_process` sets `stdin_tx` well
    ///   before the queued message that triggered the spawn is actually
    ///   delivered (that happens later, on a background drain task —
    ///   see `drain_queue_after_successful_spawn`). Gating `DeliverDirect`
    ///   on `stdin_tx.is_some()` alone let a second, genuinely concurrent
    ///   `send_message` call land in that exact window and write straight
    ///   to stdin via `try_send`, racing ahead of the drain's own
    ///   `Sender::send().await` for the message that actually triggered
    ///   the spawn — silently reordering user input. Checking
    ///   `!spawning_in_progress` too routes it into the queue instead,
    ///   where the SAME already-running drain loop (it stays `true` for
    ///   its entire lifetime — see the field's own doc comment) picks it
    ///   up next, in order.
    /// - nobody is running AND nobody is spawning → this caller claims the
    ///   exclusive right to (`spawning_in_progress = true`), enqueues
    ///   `json_str` alongside that claim, and returns `BecomeSpawner` —
    ///   the caller must then call `spawn_process` and, regardless of
    ///   outcome, call `release_spawn_claim_and_drain_queue`.
    ///
    /// `skip_if_already_queued` — always `false` for the sole production
    /// call site (`send_message`): a user legitimately re-sending the
    /// exact same text while an unrelated spawn is in flight must still
    /// queue both, so this must never dedup by content there.
    ///
    /// reagentx P2 on PR #2360 (round 16, commit ce1642d90): `true` is NOT
    /// exercised by any production call site — `retry_after_resume_
    /// failure` was refactored (round 6) to use `decide_retry_batch_
    /// action` instead, a separate function with its own atomic,
    /// batch-aware dedup/prepend logic (see that function's own doc
    /// comment). The `true` path here now only exists for this file's own
    /// unit tests, which document the exact scenario `decide_retry_batch_
    /// action`'s own dedup check handles for a batch instead: codex P1 on
    /// PR #2360 (sixth review pass, round 4) — a KNOWN re-delivery of
    /// content that may ALREADY be sitting in `pending_send_messages`,
    /// pushed by the very spawn attempt whose failure triggered a retry,
    /// if that spawn's own drain hasn't reached it yet; blindly queueing
    /// another copy there let a fallback spawn eventually deliver the
    /// same prompt twice.
    fn decide_send_action(&self, json_str: &str, skip_if_already_queued: bool) -> SendAction {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() && !inner.spawning_in_progress {
            SendAction::DeliverDirect
        } else if inner.spawning_in_progress {
            let already_queued =
                skip_if_already_queued && inner.pending_send_messages.iter().any(|m| m == json_str);
            if !already_queued {
                inner.pending_send_messages.push_back(QueuedMessage::fresh(json_str.to_string()));
            }
            SendAction::Queued
        } else {
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh(json_str.to_string()));
            SendAction::BecomeSpawner
        }
    }

    /// Releases the exclusive spawn claim taken by `decide_send_action`
    /// returning `BecomeSpawner`. `spawn_succeeded` distinguishes two very
    /// different situations:
    ///
    /// - **Failed spawn**: discards only `own_message` — the specific
    ///   message THIS spawner pushed when it claimed `BecomeSpawner`, found
    ///   by content match rather than assumed to be at the front. codex P2
    ///   on PR #2360 (round 14, commit 8c2bc99ab): the queue is NOT always
    ///   empty at the moment a new spawner claims it — the "second stall"
    ///   path (`drain_queue_after_successful_spawn` with
    ///   `allow_fallback_respawn: false`) deliberately releases
    ///   `spawning_in_progress` while leaving genuinely leftover messages
    ///   queued (see that function's own doc comment). A later
    ///   `send_message` can then claim `BecomeSpawner` and `push_back` its
    ///   own message BEHIND those leftovers. Assuming "front == my own
    ///   message" in that case discarded an OLDER, unrelated, already-
    ///   accepted prompt instead of the actually-failed one — silent data
    ///   loss, plus handing the wrong (already-failed) message to the
    ///   fallback respawn. Matching by content instead of position fixes
    ///   this whenever the two differ, which is the overwhelming common
    ///   case; the narrow residual case of two GENUINELY DIFFERENT messages
    ///   sharing identical content (e.g. two "yes" replies) is the same
    ///   content-identity-ambiguity gap already tracked in #2365 — this fix
    ///   doesn't need to (and doesn't try to) close that, since either
    ///   duplicate is content-equivalent to discard here. codex P1 on PR
    ///   #2360 (sixth review pass): leaving this message queued let an
    ///   unrelated LATER successful spawn silently execute a prompt the
    ///   caller was already told had failed (`send_message`/
    ///   `retry_after_resume_failure` already report the failure),
    ///   sometimes duplicating a message the user had re-sent by hand.
    ///   Anything else queued (from other callers who got
    ///   `SendAction::Queued` and were already told "accepted") is left in
    ///   place for the next successful spawn.
    /// - **Successful spawn**: hands the drain off to a background task
    ///   that delivers everything queued, in order, via `Sender::send`
    ///   (which awaits free capacity) rather than `try_send`. codex P2 on
    ///   PR #2360 (sixth review pass): a synchronous `try_send` loop
    ///   popped a message off the queue and then discarded it outright if
    ///   the bounded stdin channel was momentarily full — losing input
    ///   despite already having told that caller "accepted". Deferring to
    ///   a task lets delivery simply wait for capacity instead.
    ///
    /// If the background task discovers the process it was meant to drain
    /// into has ALREADY died (`stdin_tx` gone) with messages still left
    /// queued, it hands off to `respawn_once_for_leftover_queue` using
    /// `retry_config` — reagentx/codex P1 on PR #2360 (sixth review pass,
    /// rounds 3-4): a fast-dying child can let the process-waiter run this
    /// SAME controller's own `retry_after_resume_failure` (via
    /// `decide_send_action` returning `Queued`, since this spawn's claim
    /// hasn't been released yet) BEFORE this caller's own thread even
    /// reaches this function; that retry's `Queued` branch then does
    /// nothing further, assuming (wrongly, in this exact race) that this
    /// drain will deliver it. Without a fallback respawn, NOTHING is ever
    /// left responsible for the leftover messages or for telling the
    /// frontend the turn ended — the ORIGINAL exit deliberately suppressed
    /// its own terminal-status publish expecting the retry to eventually
    /// publish one.
    fn release_spawn_claim_and_drain_queue(&self, spawn_succeeded: bool, retry_config: PersistentSpawnConfig, own_message: &str) {
        if !spawn_succeeded {
            // Discard only `own_message` — see this function's own doc
            // comment above for why position (front) is not a safe
            // assumption. If anything else is queued (from other callers
            // already told "accepted"), hand off to a bounded fallback
            // respawn rather than stranding it with nobody responsible for
            // delivering it.
            //
            // codex P2 on PR #2360 (round 13, commit e9678091f): clearing
            // `spawning_in_progress` in a SEPARATE, later lock acquisition
            // left a window between the emptiness check above and that
            // clear where a concurrent `send_message` could observe
            // `spawning_in_progress` still `true`, enqueue its message via
            // `decide_send_action`'s `Queued` branch, and be told
            // "accepted" — then this function's second lock would clear
            // the claim without ever rechecking the queue, stranding that
            // accepted message with no spawner and no drain ever
            // responsible for it. The emptiness check and the flag clear
            // must be one atomic decision under a single lock acquisition
            // (same shape as the round-9 regression this whole PR already
            // fixed once): only clear the claim here if the queue is STILL
            // empty at the exact moment we're about to clear it; otherwise
            // leave the claim held and hand off to the fallback respawn.
            let leftovers = {
                let mut inner = self.inner.lock().unwrap();
                if let Some(idx) = inner.pending_send_messages.iter().position(|m| m == own_message) {
                    inner.pending_send_messages.remove(idx);
                } else {
                    // Shouldn't normally happen — defensively log rather
                    // than guess which OTHER entry to discard instead.
                    tracing::warn!(
                        block_id = %self.block_id,
                        "failed spawn's own message was not found in the queue to discard"
                    );
                }
                if inner.pending_send_messages.is_empty() {
                    inner.spawning_in_progress = false;
                    false
                } else {
                    true
                }
            };
            if leftovers {
                self.respawn_once_for_leftover_queue(retry_config);
            }
            return;
        }
        self.drain_queue_after_successful_spawn(retry_config, true);
    }

    /// Drains the queue via a background task after a successful spawn —
    /// see `release_spawn_claim_and_drain_queue`'s doc comment for the
    /// `try_send` → `Sender::send` rationale. `allow_fallback_respawn`
    /// bounds retry depth to exactly one extra hop: `true` from the public
    /// entry point, `false` when called from `respawn_once_for_leftover_queue`
    /// itself, so a SECOND stall just publishes a status update instead of
    /// cascading indefinitely.
    fn drain_queue_after_successful_spawn(&self, retry_config: PersistentSpawnConfig, allow_fallback_respawn: bool) {
        let inner_arc = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let self_ref = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        tokio::spawn(async move {
            // The message `spawn_process` already stashed synchronously
            // into `pending_resume_retry` (before this task even existed —
            // necessary so a process that dies before this task's first
            // poll can't lose it, see that field's own doc comment) must
            // be identified by CONTENT, not by "whatever this drain pops
            // first" — codex P2 on PR #2360 (round 15, commit fdb8db6fd):
            // a purely positional flag breaks exactly the way
            // `release_spawn_claim_and_drain_queue`'s front-popping
            // assumption did (see that function's own fix history): a
            // prior "second stall" can leave older leftover messages
            // queued ahead of a later spawner's own triggering message
            // (`push_back` appends behind them), so the FIRST thing this
            // drain pops isn't necessarily the seeded one. Treating it as
            // if it were: (a) skips recording the OLDER leftover into the
            // retry-batch tracking at all — silently dropping it forever
            // if this process ALSO later dies from a stale resume, and
            // (b) records the ACTUAL seeded/triggering message a SECOND
            // time (once via `spawn_process`'s synchronous seed, once via
            // this drain's own append) — a confirmed stale-resume retry
            // would then redeliver that one message TWICE. Matching by
            // content instead correctly identifies the seeded entry
            // regardless of where it sits, exactly once (via
            // `seed_already_matched`, so a later, genuinely-different
            // delivery that happens to share identical content isn't ALSO
            // wrongly skipped — same known, accepted content-identity
            // limits as #2365).
            let mut seed_already_matched = false;
            let stalled_with_leftovers = loop {
                let next = {
                    let mut inner = inner_arc.lock().unwrap();
                    match inner.stdin_tx.clone() {
                        Some(tx) => inner.pending_send_messages.pop_front().map(|m| (m, tx)),
                        None => None,
                    }
                };
                let Some((queued, tx)) = next else {
                    // Either the queue is empty, or the process has
                    // already exited again before we got to it. Release
                    // the claim ONLY if we're not about to hand off to a
                    // fallback respawn — reagentx P1 on PR #2360 (sixth
                    // review pass, round 4): releasing it unconditionally
                    // here left a window, between this release and
                    // `respawn_once_for_leftover_queue`'s own
                    // `spawn_process` call re-establishing state, where a
                    // concurrent `send_message`/`retry_after_resume_failure`
                    // could see "not running, not spawning" and
                    // independently spawn its own child process for the
                    // same block — reintroducing the exact orphaned/
                    // duplicate-process race `spawning_in_progress` was
                    // added in this same PR to close.
                    let mut inner = inner_arc.lock().unwrap();
                    let stalled = inner.stdin_tx.is_none() && !inner.pending_send_messages.is_empty();
                    if !(stalled && allow_fallback_respawn) {
                        inner.spawning_in_progress = false;
                    }
                    break stalled;
                };
                let QueuedMessage { json_str, already_persisted } = queued;
                let delivered_copy = json_str.clone();
                let is_the_seed = !seed_already_matched && {
                    let inner = inner_arc.lock().unwrap();
                    inner
                        .pending_resume_retry
                        .as_ref()
                        .or(inner.confirmed_stale_resume_retry.as_ref())
                        .and_then(|(_, _, seeded)| seeded.first())
                        == Some(&delivered_copy)
                };
                if is_the_seed {
                    seed_already_matched = true;
                }
                // reagentx P1 on PR #2360 (sixth review pass, round 9):
                // mark "in flight" for the ENTIRE send-then-append
                // sequence below, not just the send — see
                // `PersistentInner::drain_send_in_flight`'s own doc
                // comment for the race this closes (the process-waiter's
                // exit-handling can `.take()` the confirmed retry batch
                // in the gap between a successful send and this task
                // getting back around to recording it there).
                inner_arc.lock().unwrap().drain_send_in_flight = true;
                if let Err(e) = tx.send(json_str).await {
                    tracing::warn!(
                        block_id = %block_id,
                        "failed to deliver a queued message — receiver dropped, process likely exited"
                    );
                    // The channel's gone, so no send from here will ever
                    // succeed again — put the message back (rather than
                    // silently discarding it) for a future spawn to pick
                    // up. Same claim-retention rule as above.
                    let mut inner = inner_arc.lock().unwrap();
                    inner.drain_send_in_flight = false;
                    inner.pending_send_messages.push_front(QueuedMessage { json_str: e.0, already_persisted });
                    let stalled = !inner.pending_send_messages.is_empty();
                    if !(stalled && allow_fallback_respawn) {
                        inner.spawning_in_progress = false;
                    }
                    break stalled;
                }
                // codex P1 on PR #2360 (sixth review pass, round 5): track
                // every message actually handed to this process's stdin
                // channel beyond the first — not just the one that
                // triggered the spawn — so a confirmed stale-resume retry
                // redelivers all of them (see `pending_resume_retry`'s own
                // doc comment), since channel acceptance is not proof the
                // CLI ever read it. Checks `confirmed_stale_resume_retry`
                // too — codex P1 on PR #2360 (sixth review pass, round 6):
                // `poison_resume` (the stderr-reader task, running
                // concurrently) can promote `pending_resume_retry` into
                // `confirmed_stale_resume_retry` at any point; checking
                // only the tentative field left a window where a message
                // delivered right after that promotion found `None` there
                // and was persisted/removed from the queue without ever
                // being added to the batch the replacement actually
                // replays.
                if !is_the_seed {
                    let mut inner = inner_arc.lock().unwrap();
                    if let Some((_, _, ref mut delivered)) = inner.pending_resume_retry {
                        delivered.push(delivered_copy.clone());
                    } else if let Some((_, _, ref mut delivered)) = inner.confirmed_stale_resume_retry {
                        delivered.push(delivered_copy.clone());
                    }
                }
                inner_arc.lock().unwrap().drain_send_in_flight = false;
                // Persist in actual delivery order — codex P2 on PR #2360
                // (sixth review pass, round 5): persisting at the
                // `decide_send_action` call site instead let two callers'
                // own synchronous code run (and thus persist) in a
                // different order than their messages are actually
                // delivered, producing a blockfile transcript that
                // doesn't match what the agent received. Skipped for a
                // stale-resume retry's redelivery — codex P2 on PR #2360
                // (sixth review pass, round 7): that content was already
                // correctly persisted on its ORIGINAL (failed) attempt;
                // persisting it again here duplicated every replayed
                // prompt in the blockfile transcript.
                if !already_persisted {
                    if let Some(ctrl) = self_ref.upgrade() {
                        ctrl.persist_message_to_blockfile(&delivered_copy);
                    }
                }
            };
            if stalled_with_leftovers {
                match self_ref.upgrade() {
                    Some(ctrl) if allow_fallback_respawn => ctrl.respawn_once_for_leftover_queue(retry_config),
                    Some(ctrl) => ctrl.publish_status(),
                    None => {
                        // Nobody left to call back (e.g. a throwaway
                        // instance that never called `set_self_ref`) — the
                        // claim was deliberately kept held above pending
                        // this hand-off, so it must still be released here
                        // or no future caller could ever spawn again.
                        inner_arc.lock().unwrap().spawning_in_progress = false;
                    }
                }
            }
        });
    }

    /// Attempts exactly one fallback respawn when either call site above is
    /// about to release its claim with messages still queued and nobody
    /// left responsible for delivering them. Forces `session_id` empty so
    /// this fallback spawn never attempts `--resume` — reusing a
    /// possibly-still-stale session id here would risk repeating the exact
    /// failure this whole retry mechanism exists to recover from. If THIS
    /// spawn also fails, or its own process also dies before its own drain
    /// completes, gives up and publishes a status update rather than
    /// cascading indefinitely — the queue itself is never discarded (this
    /// function never pops anything itself — it isn't tied to a specific
    /// message the way the original spawn attempt was), so a genuinely
    /// later, unrelated send will still eventually pick it up.
    fn respawn_once_for_leftover_queue(&self, mut config: PersistentSpawnConfig) {
        config.session_id = String::new();
        // reagentx P0 on PR #2360 (sixth review pass, round 11): clearing
        // `config.session_id` alone does nothing — `spawn_process`'s own
        // `--resume` decision reads `inner.session_id` directly (see its
        // own doc comment), never `config.session_id` (that field is only
        // consulted to HYDRATE `inner.session_id` when it's still `None`,
        // which is skipped here anyway since it's now empty). If the
        // doomed process's stderr reader hasn't cleared `inner.session_id`
        // yet (poison_resume races the drain's own `tx.send()` failure —
        // exactly the fast-fail case this whole mechanism targets), this
        // fallback respawn would reattach `--resume <stale-sid>` and
        // reproduce the identical failure — and since this call passes
        // `resume_retry_payload: None`, nothing re-arms to catch the
        // repeat, so the process-waiter finds no confirmed retry and
        // silently drops the message for good. Mirrors
        // `retry_after_resume_failure`'s own explicit clear for the exact
        // same reason.
        //
        // A plain clear, deliberately NOT `poison_resume` — reagentx P1
        // on PR #2360 (sixth review pass, round 13): a prior cut of this
        // fix called `poison_resume` here to defensively close a narrower
        // race (a still-racing stdout-reader task from the doomed process
        // could re-capture the same sid right after a plain clear). That
        // was a regression: `respawn_once_for_leftover_queue` is reached
        // from TWO triggers that have nothing to do with a CONFIRMED
        // stale `--resume` — `release_spawn_claim_and_drain_queue`'s
        // `!spawn_succeeded` branch (ANY `spawn_process` failure — a
        // missing binary, an OS error) and
        // `drain_queue_after_successful_spawn`'s stall branch (ANY
        // process exit/crash with messages still queued, not
        // specifically a stale-resume death). In both, `inner.session_id`
        // could just as easily be a legitimately hydrated-but-unattempted
        // id, or a genuinely valid, already-captured session from a
        // process that ran fine and crashed for an unrelated reason.
        // `poison_resume` is documented as PERMANENT (`resume_poisoned` is
        // "never reset back to None") — poisoning a sid never actually
        // confirmed dead by the CLI permanently breaks conversation
        // continuity for that session, with no disclosure, for the rest
        // of this controller instance's lifetime. A plain clear only
        // affects THIS respawn's own `--resume` decision (already forced
        // empty via `config.session_id` above) and carries no such
        // permanent, over-broad risk. The narrower race the poisoning was
        // meant to close is real but far rarer than the cases above;
        // tracked as a documented follow-up instead of reintroducing this
        // regression.
        self.inner.lock().unwrap().session_id = None;
        let retry_config = config.clone();
        let spawn_result = self.spawn_process(config, None);
        match &spawn_result {
            Ok(_) => {
                self.mark_turn_active_and_publish();
                self.drain_queue_after_successful_spawn(retry_config, false);
            }
            Err(e) => {
                tracing::error!(
                    block_id = %self.block_id,
                    error = %e,
                    "fallback respawn for a leftover queue failed"
                );
                self.inner.lock().unwrap().spawning_in_progress = false;
                self.publish_status();
            }
        }
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

    /// Persists a formatted stdin JSON line to the blockfile + global zone
    /// so `parseHistoryLines` can reconstruct the `user_message` node on
    /// the next pane open. No WPS event is published here — the
    /// live-display is handled by the `agent-message-accepted` path (UUID
    /// node), avoiding a duplicate.
    fn persist_message_to_blockfile(&self, json_str: &str) {
        let global_zone = super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
        let line_with_newline = format!("{json_str}\n");
        super::shell::persist_to_blockfile_silent(
            &self.block_id,
            crate::backend::agent_session::OUTPUT_FILE,
            line_with_newline.as_bytes(),
            self.filestore.as_ref(),
            global_zone.as_deref(),
        );
    }

    pub fn send_message(&self, message: String, config: PersistentSpawnConfig) -> Result<(), String> {
        // Format as stream-json user message.
        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });
        let json_str = json_msg.to_string();

        match self.decide_send_action(&json_str, false) {
            SendAction::Queued => {
                // Persistence happens later, inside the drain, at the
                // exact moment this message is actually delivered — see
                // `drain_queue_after_successful_spawn`. codex P2 on PR
                // #2360 (sixth review pass, round 5): persisting
                // immediately here instead let whichever caller's
                // synchronous code happened to run first persist first,
                // even when queue position said a DIFFERENT message
                // (already sitting there, from a caller further along in
                // its own `BecomeSpawner` spawn_process call) is actually
                // delivered first — producing a blockfile transcript that
                // doesn't match delivery order.
                self.emit_message_accepted(config.message_id.as_deref());
                Ok(())
            }
            SendAction::DeliverDirect => {
                // spawn_process already marks a fresh process's first turn
                // active (and starts its watchdog); for an already-running
                // process (the common case — every turn after the first)
                // this is the only place that re-marks the turn active,
                // since the persistent process never exits between turns.
                // Without this, `turn_active` would go stale after turn 1.
                self.mark_turn_active_and_publish();
                let inner = self.inner.lock().unwrap();
                let tx = inner.stdin_tx.as_ref()
                    .ok_or("persistent process not running after spawn")?;
                // Persist only AFTER a successful send — reagentx P1 on PR
                // #2360 (sixth review pass, round 5): `stdin_tx` can have
                // gone `None` (process died) between `decide_send_action`'s
                // check and this later lock re-acquisition, or `try_send`
                // can fail with `Full` under load; persisting beforehand
                // reproduces, for this path, the exact "persisted a
                // never-delivered message" bug the immediately prior
                // commit fixed for `BecomeSpawner`. Matches
                // `send_user_message`'s existing (correct) ordering.
                tx.try_send(json_str.clone())
                    .map_err(|e| format!("stdin send failed: {e}"))?;
                drop(inner);
                self.persist_message_to_blockfile(&json_str);
                self.emit_message_accepted(config.message_id.as_deref());
                Ok(())
            }
            SendAction::BecomeSpawner => {
                // `resume_retry_payload` is stashed SYNCHRONOUSLY inside
                // spawn_process, before any background task exists —
                // reagentx P1 on PR #2360: stashing it after spawn_process
                // returned left a window where a process that dies fast
                // enough lets the already-scheduled process-waiter task
                // observe the exit and take() this payload as still
                // `None`, silently losing the retry for the exact case it
                // exists to catch. This is independent of, and still
                // needed alongside, the spawn-claim/queue mechanism below:
                // that mechanism only prevents a SECOND process from being
                // spawned concurrently — it does nothing once THIS
                // process is running and later dies from a stale
                // `--resume`, which is what the retry payload is for.
                let message_id = config.message_id.clone();
                let retry_config = config.clone();
                let own_message = json_str.clone();
                let spawn_result = self.spawn_process(config, Some(json_str));
                // Only emit "accepted" on success — codex P2 on PR #2360
                // (sixth review pass, round 4): an earlier cut of this fix
                // persisted unconditionally here, letting a rejected spawn
                // (missing executable, bad launch config) leave a
                // "user_message" line in the blockfile for a prompt that
                // was NEVER actually delivered. Persistence itself now
                // happens later, inside the drain, at the exact moment
                // this message is actually delivered (round 5 — see
                // `drain_queue_after_successful_spawn`) — which already
                // only runs on a successful spawn, so the same guarantee
                // holds without needing a persist call here at all.
                if spawn_result.is_ok() {
                    self.mark_turn_active_and_publish();
                    self.emit_message_accepted(message_id.as_deref());
                }
                self.release_spawn_claim_and_drain_queue(spawn_result.is_ok(), retry_config, &own_message);
                spawn_result
            }
        }
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
    /// again. Redelivers EVERY message the doomed process's stdin channel
    /// had accepted (see `pending_resume_retry`'s own doc comment for why
    /// this is a batch, not just the one that triggered the spawn) — does
    /// NOT re-persist to the blockfile or re-emit `agent-message-accepted`
    /// for any of them, since both already happened correctly on each
    /// message's original (failed) attempt; only the underlying CLI
    /// process needed a fresh, resume-less start.
    ///
    /// This is itself a spawn attempt, and must not race a genuinely
    /// concurrent `send_message` call the same way the ORIGINAL doomed
    /// spawn could — see `PersistentInner::spawning_in_progress`'s doc
    /// comment. By the time this runs, the original `send_message` call
    /// that triggered the doomed process has long since returned (its own
    /// spawn-claim-and-deliver sequence completed synchronously, well
    /// before this process even exited), so the WHOLE batch can safely go
    /// through `decide_retry_batch_action` — the SAME decision
    /// `send_message` uses, but enqueueing everything atomically in one
    /// lock acquisition (see its own doc comment for why per-message
    /// decisions aren't safe for a batch).
    fn retry_after_resume_failure(&self, mut config: PersistentSpawnConfig, mut json_strs: Vec<String>) {
        config.session_id = String::new();
        let Some(first) = (!json_strs.is_empty()).then(|| json_strs.remove(0)) else {
            return;
        };
        let rest = json_strs;

        match self.decide_retry_batch_action(&first, &rest) {
            SendAction::DeliverDirect => {
                // Another caller's own spawn already installed a running
                // process by the time this retry got scheduled — no need
                // for a dedicated respawn; deliver straight to it.
                self.mark_turn_active_and_publish();
                let mut any_failed = false;
                {
                    let mut inner = self.inner.lock().unwrap();
                    let tx = inner.stdin_tx.clone();
                    for msg in std::iter::once(first).chain(rest) {
                        // codex P2 on PR #2360 (round 15, commit
                        // fdb8db6fd): once ANY earlier message in this
                        // batch has failed direct delivery, stop
                        // attempting `try_send` for the rest — the
                        // bounded stdin channel's receiver runs
                        // concurrently and can free capacity between
                        // iterations, letting a LATER message in this same
                        // batch succeed via `try_send` while an EARLIER
                        // one sits queued (from the `Full` failure below),
                        // reordering this batch relative to itself once
                        // the drain eventually delivers the queued one.
                        // `!any_failed` short-circuits every remaining
                        // message straight to the queued branch, in
                        // order, once the first failure is hit.
                        let delivered = !any_failed && tx.as_ref().is_some_and(|tx| tx.try_send(msg.clone()).is_ok());
                        if !delivered {
                            any_failed = true;
                            // codex P2 on PR #2360 (sixth review pass,
                            // round 6): this is the retry's own
                            // LAST-CHANCE delivery attempt for this
                            // message — nothing else will ever resend it,
                            // so discarding it here (an earlier cut of
                            // this fix did, on both a `Full` channel and a
                            // process that died again in this exact gap)
                            // would lose it permanently despite already
                            // having been reported as accepted.
                            inner.pending_send_messages.push_back(QueuedMessage::already_persisted(msg));
                        }
                    }
                    if any_failed {
                        // codex P2 on PR #2360 (sixth review pass, round
                        // 7): pushing onto the queue alone is not enough —
                        // `DeliverDirect` only fires when
                        // `spawning_in_progress` is `false`, so nothing
                        // would EVER drain this entry: future callers ALSO
                        // take `DeliverDirect` (bypassing the queue
                        // entirely) for as long as this same process stays
                        // alive, which could be indefinite, silently
                        // reordering it behind much later input.
                        //
                        // Claimed in this SAME lock acquisition as the
                        // push above — reagentx P1 on PR #2360 (sixth
                        // review pass, round 8): an earlier cut of this
                        // fix claimed it in a SEPARATE, later lock
                        // acquisition, leaving a window where a
                        // concurrent `send_message` could observe
                        // `stdin_tx.is_some() && !spawning_in_progress`
                        // and take `DeliverDirect` itself — jumping ahead
                        // of the just-queued retry messages — or, if the
                        // process also died in that same window, take
                        // `BecomeSpawner` and spawn a second, independent
                        // child process for the same block, reintroducing
                        // the exact concurrent-spawn TOCTOU
                        // `spawning_in_progress` exists to prevent.
                        inner.spawning_in_progress = true;
                    }
                }
                if any_failed {
                    tracing::warn!(
                        block_id = %self.block_id,
                        "failed to redeliver one or more messages after stale-resume retry (already-running path) — draining via the queue instead"
                    );
                    // Claim the spawn flag (above) and hand off to the
                    // SAME drain used after a fresh spawn (with
                    // backpressure via `Sender::send`, not `try_send`)
                    // instead of leaving it orphaned.
                    // `allow_fallback_respawn: false` — the process is
                    // still alive, there's nothing to fall back to spawn.
                    self.drain_queue_after_successful_spawn(config.clone(), false);
                }
            }
            SendAction::Queued => {
                // Someone else is already spawning — their own
                // `release_spawn_claim_and_drain_queue` will deliver this
                // (or, if their process turns out to already be dead, its
                // own bounded fallback respawn will).
            }
            SendAction::BecomeSpawner => {
                // Only clear inner.session_id now that THIS retry is
                // actually about to spawn — codex P2 on PR #2360 (sixth
                // review pass, round 3): clearing it unconditionally up
                // front could erase a session id a DIFFERENT,
                // concurrently-installed process had already legitimately
                // captured, if this retry instead resolved via
                // `DeliverDirect` or `Queued` above — breaking in-memory
                // session tracking and turn-end subagent reconciliation
                // for that process's remaining lifetime.
                self.inner.lock().unwrap().session_id = None;
                let retry_config = config.clone();
                let spawn_result = self.spawn_process(config, None);
                match &spawn_result {
                    Ok(_) => self.mark_turn_active_and_publish(),
                    Err(e) => tracing::error!(
                        block_id = %self.block_id,
                        error = %e,
                        "failed to respawn after a stale --resume session id"
                    ),
                }
                self.release_spawn_claim_and_drain_queue(spawn_result.is_ok(), retry_config, &first);
                if spawn_result.is_err() {
                    // Surface this, or the pane hangs forever with NO
                    // signal at all — codex P2 on PR #2360 (fifth review
                    // pass): the outer process-waiter already suppressed
                    // its own terminal-status publish for the ORIGINAL
                    // exit specifically because a retry was in flight, and
                    // send_message already returned success (possibly
                    // emitting agent-message-accepted) for the message
                    // this retry was supposed to deliver. If this respawn
                    // attempt ALSO fails, nothing else will ever tell the
                    // frontend this turn is over. `inner.proc_status`/
                    // `turn_active` are already `STATUS_DONE`/`false` (set
                    // by the original exit's own cleanup before this
                    // function was ever called) — this just actually
                    // broadcasts that state, which the original exit
                    // deliberately withheld pending this retry's outcome.
                    self.publish_status();
                }
            }
        }
    }

    /// Same three-way decision as `decide_send_action`, but atomically
    /// enqueues an ENTIRE batch of messages (not just one) when the
    /// outcome is `Queued` or `BecomeSpawner` — used only by
    /// `retry_after_resume_failure`. codex P2 on PR #2360 (sixth review
    /// pass, round 6): deciding and enqueueing a multi-message retry batch
    /// one call at a time (each through its own `decide_send_action` call)
    /// left a window between them where a genuinely new, unrelated message
    /// could interleave into the MIDDLE of the same original batch,
    /// reordering it relative to how the doomed process actually received
    /// it. `first` is checked for an already-queued duplicate the same way
    /// `decide_send_action` does (see its own doc comment on
    /// `skip_if_already_queued`) — it may still be sitting in
    /// `pending_send_messages` if the drain hasn't reached it yet. `rest`
    /// is always pushed as-is: content-based dedup must never apply WITHIN
    /// the same batch, since two entries can legitimately be identical
    /// (the user genuinely sent the same text twice) and both need
    /// redelivering (codex P2 on PR #2360, sixth review pass, round 6).
    fn decide_retry_batch_action(&self, first: &str, rest: &[String]) -> SendAction {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() && !inner.spawning_in_progress {
            SendAction::DeliverDirect
        } else if inner.spawning_in_progress {
            // codex P2 on PR #2360 (sixth review pass, round 11): prepend
            // rather than append. This retry batch represents messages
            // that were accepted by the doomed process BEFORE whatever's
            // currently sitting in the queue — anything already queued
            // here arrived AFTER the original spawn had already claimed
            // `spawning_in_progress`, so it's chronologically LATER than
            // this batch. Appending would let the fresh process receive
            // later-arriving input before this earlier-accepted batch.
            //
            // `first` may itself STILL be sitting in the queue (see
            // `decide_send_action`'s doc comment on `skip_if_already_
            // queued`) — always at index 0 if so, since it's the ONLY
            // thing ever present when a claim starts and nothing but
            // `push_back` ever touches this queue elsewhere. In that
            // case `rest` belongs immediately after it (same batch,
            // preserving order), not ahead of it.
            let first_already_queued = inner.pending_send_messages.iter().any(|m| m == first);
            if first_already_queued {
                for (i, msg) in rest.iter().enumerate() {
                    inner
                        .pending_send_messages
                        .insert(i + 1, QueuedMessage::already_persisted(msg.clone()));
                }
            } else {
                let mut front: VecDeque<QueuedMessage> = VecDeque::new();
                front.push_back(QueuedMessage::already_persisted(first.to_string()));
                for msg in rest {
                    front.push_back(QueuedMessage::already_persisted(msg.clone()));
                }
                front.append(&mut inner.pending_send_messages);
                inner.pending_send_messages = front;
            }
            SendAction::Queued
        } else {
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::already_persisted(first.to_string()));
            for msg in rest {
                inner.pending_send_messages.push_back(QueuedMessage::already_persisted(msg.clone()));
            }
            SendAction::BecomeSpawner
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
            // reagentx P1 on PR #2360 (sixth review pass, round 7):
            // `spawn_process` sets `stdin_tx` synchronously, well before
            // the queued message that triggered the spawn is actually
            // delivered by the background drain task
            // (`drain_queue_after_successful_spawn`). Gating purely on
            // `stdin_tx.is_some()` (as `decide_send_action` used to,
            // before round 4) let a message land in that exact window and
            // `try_send` straight to the live channel, jumping ahead of
            // whatever's still queued — the same reordering bug fixed for
            // `send_message`'s own delivery path. Unlike `send_message`,
            // this function has no spawn config and its persistence is a
            // LIVE, visible append (see below), not the silent persist
            // the generic queue drain performs — there's no safe way to
            // queue behind that drain without either bypassing its
            // ordering guarantee or losing the visibility requirement, so
            // this errors instead of reordering; the caller (muxbus/jekt
            // delivery) can retry shortly. Checked in the SAME lock
            // acquisition as `stdin_tx` below, not a separate one, so
            // nothing can slip through the gap between two checks.
            if inner.spawning_in_progress {
                return Err(
                    "persistent process is still starting up — try again shortly".to_string(),
                );
            }
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

    /// Spawn the persistent CLI process. Called only while the caller
    /// holds the exclusive spawn claim (`spawning_in_progress`, see
    /// `decide_send_action`) — never directly.
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
        // readers, process-waiter) is created below — reagentx P1 on PR
        // #2360: doing this later, back in send_message after this
        // function returned, left a window where a process that dies fast
        // enough (the exact case this exists to catch) lets the
        // process-waiter task — already racing on another thread once
        // it's spawned — observe the exit and take() this payload while
        // it's still `None`, silently losing the retry for the very case
        // it's meant to catch. Keyed on the EXACT sid this spawn attempted
        // (not `config.session_id`, which can differ from what's actually
        // held in `inner.session_id` once an earlier call has already
        // hydrated it) so `poison_resume`'s later confirmation check is
        // unambiguous.
        // Bumped in this SAME lock acquisition — see `spawn_generation`'s
        // own doc comment for what this identifies and why.
        let my_generation = {
            let mut inner = self.inner.lock().unwrap();
            if let (Some(sid), Some(retry_json)) = (attempted_resume_sid.clone(), resume_retry_payload) {
                inner.pending_resume_retry = Some((sid, config.clone(), vec![retry_json]));
            }
            inner.spawn_generation += 1;
            inner.spawn_generation
        };

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
                    // Also write the cross-instance (Tier-2) file registry,
                    // and its host-global sibling (Tier 2b, issue #1916) —
                    // this auto-register path bypasses the HTTP register
                    // handler entirely, so it needs its own mirror call
                    // exactly like that handler does.
                    if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::write(
                            &data_dir,
                            agent_id,
                            &local_url,
                            &self.block_id,
                        );
                        crate::backend::reactive::registry::write_shared_from_env(
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
        // Cloned before `global_output_zone` moves into the stdout-reader
        // task below — the process-waiter task (spawned further down) needs
        // its own copy to flush a held-back `pending_error_result_line`.
        let global_output_zone_wait = global_output_zone.clone();

        // codex P1 on PR #2371: the JoinHandle is kept (not discarded) so the
        // process-waiter task can await this task's full completion before
        // resolving the retry decision, mirroring `stderr_reader_handle`
        // below. `child.wait()` resolving is NOT proof this task has already
        // read and stashed the doomed attempt's terminal error-result line
        // in `pending_error_result_line` — without this wait, the waiter
        // could clear `pending_resume_retry` and launch the retry first,
        // after which this (now-lagging) reader would find
        // `pending_resume_retry` already `None` and append the error line
        // immediately, reproducing the exact bubble this PR exists to
        // suppress.
        let stdout_reader_handle: tokio::task::JoinHandle<()> = tokio::spawn(async move {
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

                // Set (instead of persisted immediately) when this line turns
                // out to be a terminal `result`/`is_error:true` event arriving
                // while a stale-`--resume` retry could still be confirmed for
                // this exact attempt — see
                // `PersistentInner::pending_error_result_line`. `false` for
                // every other line, matching today's behavior exactly.
                let mut hold_back_for_resume_retry = false;

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
                    let is_result_frame =
                        parsed.get("type").and_then(|v| v.as_str()) == Some("result");
                    // Claude's turn-ending marker. Persistent mode never exits
                    // between turns, so this is the only place `turn_active`
                    // can go back to false without waiting for process exit —
                    // see `send_message`'s matching `set_active_turn(true)`.
                    if is_result_frame {
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
                    // Issue #2368: `pending_resume_retry` being live right now
                    // is exactly the same signal `poison_resume` itself
                    // checks before confirming a retry (see its doc
                    // comment) — if it's already `None` (a session id was
                    // captured above or on an earlier line, or this
                    // generation never attempted an untrusted `--resume`),
                    // this can't be a stale-resume case and today's
                    // immediate-persist behavior is unchanged.
                    if is_result_frame && parsed.get("is_error").and_then(|v| v.as_bool()) == Some(true)
                    {
                        hold_back_for_resume_retry = inner_read
                            .lock()
                            .unwrap()
                            .hold_back_error_result_line_if_resume_pending(format!("{}\n", line));
                    }
                }

                if hold_back_for_resume_retry {
                    continue;
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
        // Needed only to flush a held-back `pending_error_result_line` when
        // the stale-resume retry is NOT confirmed (or is overridden by an
        // explicit stop) — see the exit-handler's own comment below.
        let filestore_wait = self.filestore.clone();
        // Captured so the waiter can deregister this agent from muxbus on exit.
        let agent_id_wait = agent_id_for_muxbus.clone();
        // See `set_self_ref` / `retry_after_resume_failure` — lets this
        // detached task call back into an instance method once the process
        // actually exits, to transparently retry a stale-`--resume` failure.
        let self_ref_wait = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        // This exact spawn's identity — see `stop_requested_generation`'s
        // doc comment for why the retry decision below needs it.
        let my_generation_wait = my_generation;

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
                    // immediately once the process has genuinely exited.
                    //
                    // If it DOES take longer than that (e.g. `persist_session_id`
                    // blocked on a slow store call), a bare `timeout()` alone
                    // is not enough — codex P1 on PR #2360 (fifth review
                    // pass): `timeout()` only stops WAITING for the handle,
                    // it does not cancel the underlying task, which keeps
                    // running in the background and can still call
                    // `poison_resume`/`persist_session_id("")` AFTER this
                    // task has already moved on (discarded the still-
                    // tentative retry, or started a fresh child whose own
                    // new session id that late write would then corrupt).
                    // `abort()` on a separately-obtained `AbortHandle`
                    // actually cancels it — the task stops at its next
                    // yield point and can never reach either call again.
                    if let Some(handle) = stderr_reader_handle {
                        let abort_handle = handle.abort_handle();
                        if tokio::time::timeout(std::time::Duration::from_millis(500), handle).await.is_err() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stderr reader did not finish within 500ms of process exit — aborting it"
                            );
                            abort_handle.abort();
                        }
                    }

                    // codex P1 on PR #2371: same reasoning as the stderr
                    // wait above, for the stdout reader. `child.wait()`
                    // resolving does NOT mean the stdout reader has already
                    // read and stashed the doomed attempt's terminal
                    // error-result line into `pending_error_result_line` —
                    // without this wait, `confirmed_stale_resume_retry.take()`
                    // below could clear `pending_resume_retry` and this task
                    // could launch the retry BEFORE the lagging stdout
                    // reader ever stashes the line; that reader would then
                    // find `pending_resume_retry` already `None` and append
                    // the error immediately, reproducing the exact bubble
                    // this PR exists to suppress.
                    {
                        let abort_handle = stdout_reader_handle.abort_handle();
                        if tokio::time::timeout(std::time::Duration::from_millis(500), stdout_reader_handle)
                            .await
                            .is_err()
                        {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stdout reader did not finish within 500ms of process exit — aborting it"
                            );
                            abort_handle.abort();
                        }
                    }

                    // Wait (briefly, bounded) for the drain to finish
                    // appending whatever message it's currently
                    // mid-delivery on before deciding the retry batch
                    // below is final — reagentx P1 on PR #2360 (sixth
                    // review pass, round 9): `drain_queue_after_
                    // successful_spawn`'s own "send, then append to the
                    // retry batch" sequence has an unavoidable gap at the
                    // `.await` (a mutex can't be held across it). Without
                    // this wait, the `.take()` below could run in that
                    // exact gap and dispatch a retry missing a message
                    // the doomed process's channel had ALREADY accepted —
                    // it stays marked "accepted" and gets persisted, but
                    // is never actually delivered to any process again.
                    // Same 500ms bound as the stderr-reader wait above,
                    // for the same "best effort, don't hang forever"
                    // reason.
                    for _ in 0..50 {
                        if !inner_wait.lock().unwrap().drain_send_in_flight {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
                    // Issue #2368: resolved below, once the final
                    // retry-vs-not decision (including the stop-override
                    // just past this block) is known — see
                    // `PersistentInner::pending_error_result_line`.
                    let pending_error_line = inner.pending_error_result_line.take();
                    // codex P1 on PR #2360 (round 16, commit ce1642d90): a
                    // stop meant for THIS exact spawn generation must
                    // override a pending retry — see `stop_requested_
                    // generation`'s own doc comment for why `kill_tx` alone
                    // can't be trusted to signal this.
                    let stop_was_requested = inner.consume_stop_requested_for_generation(my_generation_wait);
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
                        crate::backend::reactive::registry::remove_shared_from_env(agent_id);
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
                    // codex P1 on PR #2360 (round 16): an explicit stop
                    // must win over a pending retry — treat it exactly
                    // like "genuinely done, not retrying" below instead of
                    // silently reviving the agent with the same prompt
                    // right after the user asked it to stop.
                    let retry_after_resume = if stop_was_requested {
                        tracing::info!(
                            block_id = %block_id_wait,
                            "stop was requested while a stale-resume retry was pending — honoring the stop instead"
                        );
                        None
                    } else {
                        retry_after_resume
                    };
                    if let Some((_attempted_sid, retry_config, retry_json)) = retry_after_resume {
                        // Issue #2368: the retry is firing and will very
                        // likely succeed within milliseconds — the doomed
                        // attempt's own terminal error result must never
                        // reach the user, so it's dropped here instead of
                        // ever being appended to the blockfile.
                        if pending_error_line.is_some() {
                            tracing::info!(
                                block_id = %block_id_wait,
                                "dropping doomed attempt's error result — stale-resume retry is firing"
                            );
                        }
                        if let Some(ctrl) = self_ref_wait.upgrade() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stale --resume session id caused this exit — retrying fresh, without --resume"
                            );
                            ctrl.retry_after_resume_failure(retry_config, retry_json);
                        }
                    } else if let Some(ref broker) = broker_wait {
                        // Issue #2368: no retry is following after all
                        // (never confirmed, or an explicit stop overrode a
                        // confirmed one) — a held-back error result is a
                        // genuine first-turn failure the user must still
                        // see, just persisted here instead of at
                        // stdout-read time.
                        if let Some(line) = pending_error_line {
                            super::shell::handle_append_block_file(
                                broker,
                                &block_id_wait,
                                PERSISTENT_OUTPUT_SUBJECT,
                                line.as_bytes(),
                                filestore_wait.as_ref(),
                                global_output_zone_wait.as_deref(),
                            );
                        }
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
                            // reagentx P0 on PR #2360 (sixth review pass,
                            // round 10): must clear pending_send_messages/
                            // spawning_in_progress in this SAME lock
                            // acquisition as stdin_tx, not only later
                            // (after child.wait()/the 5s timeout below).
                            // During that window,
                            // drain_queue_after_successful_spawn's
                            // independently-scheduled background task
                            // could observe stdin_tx.is_none() with
                            // messages still queued, classify it as a
                            // stall, and call
                            // respawn_once_for_leftover_queue — spawning a
                            // brand-new CLI process while the user's
                            // graceful stop is still in progress. The
                            // force-kill path doesn't have this gap (it
                            // never clears stdin_tx early — all three
                            // clear together below, after child.kill()
                            // resolves), only this graceful one, since it
                            // specifically needs stdin_tx gone early to
                            // trigger EOF.
                            inner.pending_send_messages.clear();
                            inner.spawning_in_progress = false;
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
                    // codex P2 on PR #2371: a stashed error line must not be
                    // silently lost on a user-initiated stop (it may be a
                    // genuine error the stop itself interrupted), and must
                    // not survive to be wrongly appended by a LATER,
                    // unrelated exit on a reused controller instance —
                    // take it now, alongside the resume-retry fields it's
                    // cleared together with, and flush it below.
                    let pending_error_line = inner.pending_error_result_line.take();
                    // codex P1 on PR #2360 (sixth review pass, round 5):
                    // an active spawn claim's own background drain
                    // (`drain_queue_after_successful_spawn`) is a
                    // SEPARATE, independently-scheduled task — killing
                    // this process does not cancel it. Left untouched, its
                    // next check would see `stdin_tx` gone with messages
                    // still queued, treat that as a stall, and hand off to
                    // `respawn_once_for_leftover_queue` — silently
                    // reviving the agent moments after the user explicitly
                    // stopped it. Clearing the queue AND releasing the
                    // claim here means that same check instead sees an
                    // empty queue and just concludes normally, with no
                    // fallback respawn triggered.
                    inner.pending_send_messages.clear();
                    inner.spawning_in_progress = false;
                    Self::set_status(&mut inner, STATUS_DONE);
                    drop(inner);

                    // Flush a held-back error line now — see the take()
                    // above and `pending_error_result_line`'s own doc
                    // comment.
                    if let Some(line) = pending_error_line {
                        if let Some(ref broker) = broker_wait {
                            super::shell::handle_append_block_file(
                                broker,
                                &block_id_wait,
                                PERSISTENT_OUTPUT_SUBJECT,
                                line.as_bytes(),
                                filestore_wait.as_ref(),
                                global_output_zone_wait.as_deref(),
                            );
                        }
                    }

                    // Deregister from muxbus (see the clean-exit arm above).
                    crate::backend::reactive::get_global_handler()
                        .unregister_block(&block_id_wait);
                    if let Some(ref agent_id) = agent_id_wait {
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::remove(&data_dir, agent_id);
                        crate::backend::reactive::registry::remove_shared_from_env(agent_id);
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
            // Recorded unconditionally, not only when `kill_tx` is already
            // `None` — see `stop_requested_generation`'s own doc comment
            // for why a `Some(kill_tx)` here doesn't guarantee the send
            // below actually reaches anything.
            inner.stop_requested_generation = Some(inner.spawn_generation);
            inner.kill_tx.take()
        };
        if let Some(tx) = kill_tx {
            let _ = tx.send(force);
        }
        Ok(())
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

    // codex P1 on PR #2360 (round 16, commit ce1642d90): `stop_process`
    // must record which generation a stop was requested for even when
    // there's no live `kill_tx` to send through — see
    // `stop_requested_generation`'s own doc comment for why a `kill_tx`
    // send alone can't be trusted (the process-waiter's `tokio::select!`
    // can have already committed to its exit branch before this call
    // reaches the lock, making the send futile even when `kill_tx` was
    // still `Some`). Simulates the exact race here via no `kill_tx` at all
    // (the narrower, always-reachable sub-case).
    #[test]
    fn stop_process_records_the_current_generation_even_with_no_live_kill_tx() {
        let c = controller();
        c.inner.lock().unwrap().spawn_generation = 3;
        // kill_tx stays None — the process already exited (or never
        // started); stop_process must still succeed and record intent.
        let result = c.stop_process(false);
        assert!(result.is_ok(), "must still return Ok when there's nothing live to signal");
        assert_eq!(
            c.inner.lock().unwrap().stop_requested_generation,
            Some(3),
            "must record a signal the exit-handler can check even with no live kill_tx to send through"
        );
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
        c.retry_after_resume_failure(config, vec!["{}".to_string()]);

        assert_eq!(
            c.inner.lock().unwrap().session_id,
            None,
            "must clear inner.session_id directly, not rely on poison_resume having already done so"
        );
    }

    /// reagentx P0 on PR #2360 (sixth review pass, round 11): same class
    /// of bug as the test above, in a sibling fallback path added later —
    /// `respawn_once_for_leftover_queue` cleared only `config.session_id`,
    /// which `spawn_process`'s own `--resume` decision never reads (it
    /// reads `inner.session_id` directly). If the doomed process's stderr
    /// reader hasn't cleared `inner.session_id` yet, this fallback would
    /// reattach `--resume` to the same dead sid and reproduce the
    /// identical failure, with nothing left to catch the repeat.
    #[test]
    fn respawn_once_for_leftover_queue_clears_inner_session_id_even_when_poison_resume_has_not_run_yet() {
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
        c.respawn_once_for_leftover_queue(config);

        assert_eq!(
            c.inner.lock().unwrap().session_id,
            None,
            "must clear inner.session_id directly, not rely on config.session_id alone"
        );
    }

    /// reagentx P1 on PR #2360 (sixth review pass, round 13): an earlier
    /// cut of this fallback called `poison_resume` (not just a plain
    /// clear) on whatever sid was held, reasoning defensively about a
    /// narrower race. That was itself a regression: this fallback is
    /// reached from triggers that have nothing to do with a CONFIRMED
    /// stale `--resume` (a plain `spawn_process` failure, or ANY process
    /// crash with messages still queued) — `inner.session_id` could just
    /// as easily be a genuinely valid, already-captured session from a
    /// process that ran fine and crashed for an unrelated reason.
    /// `poison_resume` is PERMANENT (`resume_poisoned` is never reset),
    /// so poisoning a sid never actually confirmed dead by the CLI would
    /// permanently break that session's resume capability. Confirms a
    /// valid sid survives this fallback well enough to still be captured
    /// again later (i.e. NOT poisoned) — only the in-memory `session_id`
    /// itself is cleared, forcing this one respawn to skip `--resume`.
    #[test]
    fn respawn_once_for_leftover_queue_does_not_poison_the_sid_it_held() {
        let c = controller();
        c.inner.lock().unwrap().session_id = Some("valid-unrelated-sid".to_string());

        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "valid-unrelated-sid".to_string(),
            message_id: None,
        };
        c.respawn_once_for_leftover_queue(config);

        let mut inner = c.inner.lock().unwrap();
        assert_ne!(
            inner.resume_poisoned.as_deref(),
            Some("valid-unrelated-sid"),
            "must not permanently poison a sid that was never confirmed dead by the CLI"
        );
        let captured = inner.try_capture_session_id("valid-unrelated-sid");
        assert!(captured, "a genuinely valid sid must still be capturable again later");
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

    /// Confirms the bounded-wait PRIMITIVE the process-waiter's
    /// exit-handling relies on before deciding a confirmed stale-resume
    /// retry batch is final — reagentx P1 on PR #2360 (sixth review pass,
    /// round 9): it polls `drain_send_in_flight` every 10ms, bounded to
    /// 500ms, so a flag that clears shortly after being observed `true`
    /// must still be correctly picked up within the window (not missed by
    /// a single stale read). The full cross-task race this guards against
    /// isn't practical to reproduce deterministically (same reasoning as
    /// this file's other cross-task timing fixes — see e.g. the
    /// stderr-reader bound above), so this exercises the underlying
    /// polling primitive directly.
    #[tokio::test]
    async fn a_flag_clearing_shortly_after_is_observed_by_a_bounded_polling_wait() {
        let c = Arc::new(controller());
        c.inner.lock().unwrap().drain_send_in_flight = true;

        let c2 = Arc::clone(&c);
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            c2.inner.lock().unwrap().drain_send_in_flight = false;
        });

        let mut cleared = false;
        for _ in 0..50 {
            if !c.inner.lock().unwrap().drain_send_in_flight {
                cleared = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(cleared, "the bounded wait must observe the flag clearing within its window");
    }

    /// Baseline: a message sent while the process is already running is
    /// delivered directly, with no spawn decision involved at all.
    #[tokio::test]
    async fn send_message_delivers_directly_to_an_already_running_process() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(4);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
        }

        let config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: String::new(),
            session_id: String::new(),
            message_id: None,
        };

        c.send_message("hello".to_string(), config)
            .expect("delivery to an already-running process must succeed");

        let received = rx.try_recv().expect("the message must have been written to stdin_tx");
        assert!(received.contains("hello"));
    }

    /// reagentx P1 on PR #2360 (sixth review pass): `decide_send_action` is
    /// the primitive that closes the concurrent-spawn TOCTOU race — these
    /// three cases cover its full decision space deterministically,
    /// without needing to reproduce an actual multi-threaded race.
    #[test]
    fn decide_send_action_becomes_spawner_when_nothing_is_in_flight() {
        let c = controller();
        let action = c.decide_send_action("msg-a", false);
        assert!(matches!(action, SendAction::BecomeSpawner));
        let inner = c.inner.lock().unwrap();
        assert!(inner.spawning_in_progress, "must claim the exclusive spawn right");
        assert_eq!(
            inner.pending_send_messages.len(),
            1,
            "the caller's own message must be enqueued too, for the uniform post-spawn drain"
        );
    }

    #[test]
    fn decide_send_action_queues_when_a_spawn_is_already_in_flight() {
        let c = controller();
        c.inner.lock().unwrap().spawning_in_progress = true;

        let action = c.decide_send_action("msg-b", false);
        assert!(
            matches!(action, SendAction::Queued),
            "a second caller must queue instead of independently deciding to spawn"
        );
        let inner = c.inner.lock().unwrap();
        assert_eq!(inner.pending_send_messages.len(), 1);
        assert_eq!(inner.pending_send_messages[0], "msg-b");
    }

    /// codex P1 on PR #2360 (sixth review pass, round 4): a genuine second
    /// user message queuing behind an in-flight spawn must NOT be deduped
    /// by content — a user legitimately re-sending the exact same text
    /// must still see both delivered. `skip_if_already_queued=false`
    /// (what `send_message` always passes) must therefore always enqueue.
    #[test]
    fn decide_send_action_never_dedups_a_genuine_new_message() {
        let c = controller();
        c.inner.lock().unwrap().spawning_in_progress = true;

        c.decide_send_action("hello", false);
        let action = c.decide_send_action("hello", false);

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            2,
            "two genuinely separate sends of identical text must both be queued, not deduped"
        );
    }

    /// codex P1 on PR #2360 (sixth review pass, round 4): unlike a genuine
    /// new message, `retry_after_resume_failure`'s payload is a KNOWN
    /// re-delivery of content that may ALREADY be sitting in the queue —
    /// pushed by the very spawn attempt whose failure triggered this
    /// retry, if that spawn's own drain hasn't reached it yet. Blindly
    /// queueing another copy (as the `false`/`send_message` path
    /// correctly does for a genuine new message) would let a fallback
    /// spawn eventually deliver the same prompt twice. `skip_if_already_
    /// queued=true` (what `retry_after_resume_failure` always passes)
    /// must therefore skip re-enqueueing an identical, already-present
    /// entry.
    #[test]
    fn decide_send_action_dedups_a_known_retry_of_an_already_queued_message() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("original-payload".to_string()));
        }

        let action = c.decide_send_action("original-payload", true);

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            1,
            "a retry of content already queued must not add a duplicate copy"
        );
    }

    /// The dedup check must not accidentally skip a retry whose payload
    /// genuinely isn't in the queue yet (the drain already popped it, in
    /// the narrow window where the retry races in after that but before
    /// the drain releases the claim) — it must still queue normally.
    #[test]
    fn decide_send_action_still_queues_a_retry_when_its_payload_is_not_already_present() {
        let c = controller();
        c.inner.lock().unwrap().spawning_in_progress = true;

        let action = c.decide_send_action("not-yet-queued", true);

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(inner.pending_send_messages.len(), 1);
        assert_eq!(inner.pending_send_messages[0], "not-yet-queued");
    }

    #[test]
    fn decide_send_action_delivers_directly_when_already_running() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let action = c.decide_send_action("msg-c", false);
        assert!(matches!(action, SendAction::DeliverDirect));
        let inner = c.inner.lock().unwrap();
        assert!(
            inner.pending_send_messages.is_empty(),
            "must not queue when delivering directly to an already-running process"
        );
    }

    /// reagentx P1 on PR #2360 (sixth review pass, round 4): `spawn_process`
    /// sets `stdin_tx` synchronously, well before the queued message that
    /// triggered the spawn is actually delivered by the background drain
    /// task (`drain_queue_after_successful_spawn`). A second caller
    /// landing in that exact window — `stdin_tx` already live, but
    /// `spawning_in_progress` still `true` because the drain hasn't
    /// finished — must NOT take the `DeliverDirect` path: writing straight
    /// to stdin via `try_send` there would race ahead of the drain's own
    /// `Sender::send().await` for the message that actually triggered the
    /// spawn, silently reordering user input. It must queue behind
    /// whatever the still-active drain is working through instead.
    #[test]
    fn decide_send_action_queues_instead_of_delivering_direct_while_a_drain_is_still_active() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.spawning_in_progress = true;
        }

        let action = c.decide_send_action("msg-late-arrival", false);
        assert!(
            matches!(action, SendAction::Queued),
            "must queue, not deliver direct, while a drain for an earlier message is still active"
        );
        let inner = c.inner.lock().unwrap();
        assert_eq!(inner.pending_send_messages.len(), 1);
        assert_eq!(inner.pending_send_messages[0], "msg-late-arrival");
    }

    /// codex P2 on PR #2360 (sixth review pass, round 7): a fresh message
    /// (from `decide_send_action`) must be marked NOT already persisted —
    /// the drain is responsible for persisting it, in delivery order.
    #[test]
    fn decide_send_action_marks_a_fresh_message_as_not_yet_persisted() {
        let c = controller();
        c.decide_send_action("hello", false);
        let inner = c.inner.lock().unwrap();
        assert!(
            !inner.pending_send_messages[0].already_persisted,
            "a genuinely new message must not be marked already-persisted"
        );
    }

    /// codex P2 on PR #2360 (sixth review pass, round 7): a stale-resume
    /// retry's batch (from `decide_retry_batch_action`) must be marked
    /// already persisted — it was correctly persisted on its ORIGINAL
    /// attempt, and the shared drain must not persist it a second time.
    #[test]
    fn decide_retry_batch_action_marks_every_entry_as_already_persisted() {
        let c = controller();
        c.decide_retry_batch_action("hello", &["world".to_string()]);
        let inner = c.inner.lock().unwrap();
        assert!(inner.pending_send_messages[0].already_persisted);
        assert!(inner.pending_send_messages[1].already_persisted);
    }

    /// reagentx P1 on PR #2360 (sixth review pass, round 7): `spawn_process`
    /// sets `stdin_tx` synchronously, well before the queued message that
    /// triggered the spawn is actually delivered by the background drain.
    /// A muxbus/jekt steering message (`send_user_message`) landing in
    /// that window must not `try_send` straight to the live channel — it
    /// has no way to safely queue behind the drain (its persistence is a
    /// live, visible append, not the drain's silent persist), so it must
    /// error instead of reordering ahead of whatever the drain is still
    /// working through.
    #[tokio::test]
    async fn send_user_message_errors_instead_of_reordering_while_a_drain_is_still_active() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.spawning_in_progress = true;
        }

        let err = c.send_user_message("steer".to_string()).unwrap_err();
        assert!(
            err.contains("starting up"),
            "should surface a clear, retryable error instead of reordering, got {err:?}"
        );
    }

    /// Exercises the actual race with real OS threads, not just sequential
    /// state assertions — reagentx P1 on PR #2360 (sixth review pass): many
    /// concurrent callers landing on a controller with no process running
    /// (the exact shape of a genuine second `send_message` RPC, or a
    /// muxbus delivery, racing this controller's own stale-resume retry)
    /// must produce EXACTLY one spawner; everyone else must queue instead
    /// of each independently deciding to spawn their own child process.
    #[test]
    fn decide_send_action_produces_exactly_one_spawner_under_real_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::sync::Arc as StdArc;

        let c = StdArc::new(controller());
        let spawner_count = StdArc::new(AtomicUsize::new(0));
        let queued_count = StdArc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..16)
            .map(|i| {
                let c = StdArc::clone(&c);
                let spawner_count = StdArc::clone(&spawner_count);
                let queued_count = StdArc::clone(&queued_count);
                std::thread::spawn(move || match c.decide_send_action(&format!("msg-{i}"), false) {
                    SendAction::BecomeSpawner => {
                        spawner_count.fetch_add(1, AtomicOrdering::SeqCst);
                    }
                    SendAction::Queued => {
                        queued_count.fetch_add(1, AtomicOrdering::SeqCst);
                    }
                    SendAction::DeliverDirect => panic!("process was never running in this test"),
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            spawner_count.load(AtomicOrdering::SeqCst),
            1,
            "exactly one caller must claim the exclusive right to spawn"
        );
        assert_eq!(queued_count.load(AtomicOrdering::SeqCst), 15, "everyone else must queue");
        assert_eq!(
            c.inner.lock().unwrap().pending_send_messages.len(),
            16,
            "every message — the spawner's own plus all queued — must be present, none dropped"
        );
    }

    /// On a successful spawn, the drain must deliver everything queued
    /// (including a caller's own message, enqueued alongside the claim by
    /// `decide_send_action`) in order, then release the claim so a future
    /// caller can spawn again. Delivery happens on a spawned background
    /// task (see the function's own doc comment), so this must actually
    /// wait for it rather than asserting immediately.
    #[tokio::test]
    async fn release_spawn_claim_and_drain_queue_delivers_everything_on_success() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("first".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("second".to_string()));
        }

        // Never used for a fallback spawn in this test — the drain fully
        // succeeds without ever stalling. `own_message` is only consulted
        // on the failed-spawn path, so its value doesn't matter here.
        c.release_spawn_claim_and_drain_queue(true, unreachable_fallback_config(), "first");

        assert_eq!(rx.recv().await.unwrap(), "first");
        assert_eq!(rx.recv().await.unwrap(), "second");

        // Give the background drain task its final iteration (observing
        // the now-empty queue and releasing the claim) a chance to run.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let inner = c.inner.lock().unwrap();
        assert!(!inner.spawning_in_progress, "claim must be released once fully drained");
        assert!(inner.pending_send_messages.is_empty());
    }

    /// A `PersistentSpawnConfig` whose `cli_command` doesn't exist, so any
    /// `spawn_process` attempt made with it fails fast and deterministically
    /// (no real process, no hang) — used by tests that need to exercise
    /// `respawn_once_for_leftover_queue`'s fallback path without spawning a
    /// real CLI, matching this module's own established precedent (see
    /// `retry_after_resume_failure_clears_inner_session_id_even_when_poison_resume_has_not_run_yet`).
    fn unreachable_fallback_config() -> PersistentSpawnConfig {
        PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: String::new(),
            session_id: String::new(),
            message_id: None,
        }
    }

    /// reagentx/codex P1 on PR #2360 (sixth review pass, rounds 3-4): if
    /// the process a successful spawn just established dies before the
    /// drain even gets to run (found via `stdin_tx` already `None`) with
    /// messages still queued, nothing else will ever tell the frontend
    /// the turn ended — the ORIGINAL exit deliberately suppressed its own
    /// publish expecting a retry to publish one instead, and
    /// `retry_after_resume_failure`'s own `Queued` branch does nothing
    /// further (see its own doc comment). Confirms the drain hands off to
    /// `respawn_once_for_leftover_queue`, which — using a config that
    /// itself fails fast (no real process needed) — logs the failure and
    /// publishes a status update instead of leaving the pane hanging with
    /// no signal at all, while leaving the leftover messages queued
    /// (this fallback path never pops anything itself — it isn't tied to
    /// a specific message).
    #[tokio::test]
    async fn release_spawn_claim_and_drain_queue_falls_back_and_publishes_status_when_stalled() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let c = Arc::new(PersistentSubprocessController::new(
            "tab".to_string(),
            "block-stalled".to_string(),
            Some(broker.clone()),
            None,
            None,
            None,
        ));
        c.set_self_ref();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("stuck-one".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("stuck-two".to_string()));
            // stdin_tx stays None — simulates the process this claim was
            // spawning for having already died before the drain ran.
        }

        c.release_spawn_claim_and_drain_queue(true, unreachable_fallback_config(), "stuck-one");
        // Let the spawned background task, and the fallback respawn
        // attempt it triggers, run to completion.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_CONTROLLER_STATUS,
            "block:block-stalled",
            1,
        );
        assert_eq!(history.len(), 1, "must publish a status update instead of silently hanging");

        let inner = c.inner.lock().unwrap();
        assert!(!inner.spawning_in_progress, "claim must still be released");
        assert_eq!(
            inner.pending_send_messages.len(),
            2,
            "leftover messages must stay queued for whatever spawn comes next \
             (the fallback attempt itself failed, matching the config used)"
        );
    }

    /// codex P1 on PR #2360 (sixth review pass): a FAILED spawn must
    /// discard only the front item — the caller's own message, which
    /// `send_message`/`retry_after_resume_failure` already reported as a
    /// failure to their own caller — not leave it queued for an unrelated
    /// later spawn to silently execute. Anything ELSE queued behind it
    /// (from other callers who got `SendAction::Queued` and were already
    /// told "accepted") must survive — handed off to a bounded fallback
    /// respawn (codex P2, round 4) rather than stranded with nobody
    /// responsible for it; using a config that itself fails fast here, so
    /// the leftover message ends up back in the queue rather than
    /// delivered, but never discarded.
    #[test]
    fn release_spawn_claim_and_drain_queue_discards_only_the_failed_spawners_own_message() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("the-one-that-failed".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("queued-by-someone-else".to_string()));
        }

        c.release_spawn_claim_and_drain_queue(false, unreachable_fallback_config(), "the-one-that-failed");

        let inner = c.inner.lock().unwrap();
        assert!(
            !inner.spawning_in_progress,
            "claim must still be released even though the spawn and its fallback both failed"
        );
        assert_eq!(
            inner.pending_send_messages.len(),
            1,
            "only the failed spawner's own (front) message must be discarded"
        );
        assert_eq!(
            inner.pending_send_messages[0],
            "queued-by-someone-else",
            "a message queued by a DIFFERENT caller (already told \"accepted\") must survive for the next spawn"
        );
    }

    /// codex P2 on PR #2360 (round 14, commit 8c2bc99ab): the queue is NOT
    /// always empty at the moment a new spawner claims `BecomeSpawner` — the
    /// "second stall" path (`drain_queue_after_successful_spawn` with
    /// `allow_fallback_respawn: false`) deliberately releases
    /// `spawning_in_progress` while leaving genuine leftover messages
    /// queued. A later `send_message` can then claim `BecomeSpawner` and
    /// `push_back` its own message BEHIND those leftovers — so the failed
    /// spawner's own message is NOT at the front. Confirms
    /// `release_spawn_claim_and_drain_queue`'s failed-spawn path finds and
    /// discards the correct (content-matched) entry regardless of where it
    /// sits, instead of assuming the front and silently destroying an
    /// older, unrelated, already-accepted prompt.
    #[test]
    fn release_spawn_claim_and_drain_queue_discards_the_right_entry_when_it_is_not_at_the_front() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            // Simulates leftovers surviving a prior "second stall" release
            // (queue non-empty, claim already given up by that path) plus a
            // later BecomeSpawner appending its own message behind them.
            inner.pending_send_messages.push_back(QueuedMessage::fresh("older-leftover-from-a-different-caller".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("this-spawners-own-message-that-just-failed".to_string()));
        }

        c.release_spawn_claim_and_drain_queue(
            false,
            unreachable_fallback_config(),
            "this-spawners-own-message-that-just-failed",
        );

        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            1,
            "only the actually-failed spawner's own message must be discarded"
        );
        assert_eq!(
            inner.pending_send_messages[0],
            "older-leftover-from-a-different-caller",
            "an older, unrelated, already-accepted prompt must survive — not be silently destroyed \
             because it happened to be sitting at the front"
        );
    }

    /// Exercises the actual race with real OS threads — codex P2 on PR
    /// #2360 (round 13, commit e9678091f): a FAILED spawn's claim used to
    /// be released in a lock acquisition SEPARATE from the emptiness check
    /// that decided whether to release it at all. That left a window,
    /// between the two, where a concurrent `send_message` could observe
    /// `spawning_in_progress` still `true`, enqueue via `decide_send_action`'s
    /// `Queued` branch, and be told "accepted" — then this function's
    /// second lock would clear the claim without ever rechecking the
    /// queue, stranding that accepted message with nobody left responsible
    /// (no drain, no respawn, no disclosure). The fix merges the emptiness
    /// check and the flag clear into one lock acquisition, so a racer's
    /// push can now only land fully before or fully after that atomic
    /// block — never inside it. Across many iterations and threads, the
    /// invariant that must always hold: whenever a message is left queued
    /// with the claim released, it's because a fallback respawn was
    /// actually attempted (and disclosed its failure via a published
    /// status) — never silently, with no attempt at all.
    #[test]
    fn release_spawn_claim_and_drain_queue_never_silently_strands_a_racing_send() {
        use std::sync::Arc as StdArc;

        for iteration in 0..30 {
            let broker = StdArc::new(crate::backend::wps::Broker::new());
            let block_id = format!("block-race-{iteration}");
            let c = StdArc::new(PersistentSubprocessController::new(
                "tab".to_string(),
                block_id.clone(),
                Some(broker.clone()),
                None,
                None,
                None,
            ));
            c.set_self_ref();
            {
                let mut inner = c.inner.lock().unwrap();
                inner.spawning_in_progress = true;
                inner.pending_send_messages.push_back(QueuedMessage::fresh("the-one-that-failed".to_string()));
            }

            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let c = StdArc::clone(&c);
                    std::thread::spawn(move || {
                        let _ = c.decide_send_action(&format!("racer-{iteration}-{i}"), false);
                    })
                })
                .collect();

            c.release_spawn_claim_and_drain_queue(false, unreachable_fallback_config(), "the-one-that-failed");

            for h in handles {
                h.join().unwrap();
            }

            let inner = c.inner.lock().unwrap();
            let stranded = !inner.spawning_in_progress && !inner.pending_send_messages.is_empty();
            if stranded {
                let published = !broker
                    .read_event_history(crate::backend::wps::EVENT_CONTROLLER_STATUS, &format!("block:{block_id}"), 10)
                    .is_empty();
                assert!(
                    published,
                    "iteration {iteration}: a racing send was left queued with the claim already \
                     released and no respawn attempt ever disclosed via a status publish — stranded \
                     with nobody responsible for it"
                );
            }
        }
    }

    /// codex P2 on PR #2360 (sixth review pass, round 3): `retry_after_
    /// resume_failure` used to clear `inner.session_id` unconditionally at
    /// the top of the function, before even deciding whether this call is
    /// the one that's actually going to spawn. Confirms it's now scoped to
    /// only the `BecomeSpawner` path — a concurrently-installed session id
    /// (simulating a DIFFERENT spawn that's already running, so this call
    /// resolves via `DeliverDirect`) must survive.
    #[tokio::test]
    async fn retry_after_resume_failure_preserves_a_concurrently_installed_session_id_when_not_the_spawner() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            let (tx, _rx) = mpsc::channel::<String>(4);
            inner.stdin_tx = Some(tx);
            inner.session_id = Some("fresh-concurrently-installed-sid".to_string());
        }

        let config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        };
        c.retry_after_resume_failure(config, vec!["{}".to_string()]);

        assert_eq!(
            c.inner.lock().unwrap().session_id.as_deref(),
            Some("fresh-concurrently-installed-sid"),
            "must not erase a session id a concurrent spawn already legitimately captured"
        );
    }

    /// codex P1 on PR #2360 (sixth review pass, round 5): a doomed
    /// process's stdin channel can have accepted MULTIPLE messages before
    /// it turned out to be unreachable — not just the one that triggered
    /// the spawn. `retry_after_resume_failure` now takes the whole batch
    /// and must redeliver every one of them, in order. Uses an
    /// already-running process (`DeliverDirect` for each) so this is
    /// deterministic without needing a real subprocess spawn.
    #[tokio::test]
    async fn retry_after_resume_failure_redelivers_every_message_in_the_batch() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.retry_after_resume_failure(
            config,
            vec!["msg-1".to_string(), "msg-2".to_string(), "msg-3".to_string()],
        );

        assert_eq!(rx.recv().await.unwrap(), "msg-1");
        assert_eq!(rx.recv().await.unwrap(), "msg-2");
        assert_eq!(rx.recv().await.unwrap(), "msg-3");
    }

    /// codex P2 on PR #2360 (sixth review pass, round 6): the `DeliverDirect`
    /// path is the retry's own LAST-CHANCE delivery attempt — nothing else
    /// will ever resend a message that fails here (e.g. the process died
    /// again in the gap between `decide_retry_batch_action`'s check and
    /// this lock re-acquisition, simulated here via a dropped receiver).
    /// It must be requeued, not silently discarded.
    #[tokio::test]
    async fn retry_after_resume_failure_requeues_a_message_that_fails_direct_delivery() {
        let c = controller();
        let (tx, rx) = mpsc::channel::<String>(1);
        drop(rx);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.retry_after_resume_failure(config, vec!["stuck".to_string()]);

        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            1,
            "a message that fails its last-chance delivery must be requeued, not discarded"
        );
        assert_eq!(inner.pending_send_messages[0], "stuck");
    }

    /// codex P2 on PR #2360 (round 15, commit fdb8db6fd): once ANY message
    /// in a multi-message retry batch fails direct delivery, every
    /// remaining message must be queued too — not attempted via `try_send`
    /// — so their relative order can never be disturbed by the bounded
    /// stdin channel's receiver concurrently freeing capacity between
    /// iterations (which could otherwise let a later message succeed
    /// while an earlier, failed one sits queued behind it). Uses a
    /// permanently-closed receiver so every message in the batch fails
    /// deterministically; confirms all three end up queued in their
    /// original order, none skipped, none lost.
    #[tokio::test]
    async fn retry_after_resume_failure_queues_the_rest_of_the_batch_in_order_once_one_fails() {
        let c = controller();
        let (tx, rx) = mpsc::channel::<String>(1);
        drop(rx);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.retry_after_resume_failure(
            config,
            vec!["msg-1".to_string(), "msg-2".to_string(), "msg-3".to_string()],
        );

        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            3,
            "every message in the batch must be queued, none skipped or lost"
        );
        assert_eq!(inner.pending_send_messages[0], "msg-1");
        assert_eq!(inner.pending_send_messages[1], "msg-2");
        assert_eq!(inner.pending_send_messages[2], "msg-3");
    }

    /// codex P2 on PR #2360 (sixth review pass, round 7): pushing a failed
    /// direct-retry delivery onto the queue alone is not enough —
    /// `DeliverDirect` only fires when `spawning_in_progress` is `false`,
    /// so nothing would EVER drain this entry for as long as the same
    /// process stays alive (every future caller would ALSO take
    /// `DeliverDirect`, bypassing the queue entirely). The claim must be
    /// taken and a drain triggered instead — which, even against the same
    /// dead sender used here, must still correctly release the claim
    /// afterward (not leave it stuck forever) while preserving the
    /// message for a future spawn.
    #[tokio::test]
    async fn retry_after_resume_failure_claims_the_spawn_flag_when_direct_delivery_fails() {
        let c = controller();
        let (tx, rx) = mpsc::channel::<String>(1);
        drop(rx);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.retry_after_resume_failure(config, vec!["stuck".to_string()]);

        assert!(
            c.inner.lock().unwrap().spawning_in_progress,
            "must claim the spawn flag immediately so no concurrent caller bypasses the queue via DeliverDirect"
        );

        // Let the background drain (which will also fail against this
        // same dead sender) run to completion.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let inner = c.inner.lock().unwrap();
        assert!(!inner.spawning_in_progress, "claim must eventually be released, not stuck forever");
        assert_eq!(inner.pending_send_messages.len(), 1, "message must remain queued, not lost");
    }

    /// codex P1 on PR #2360 (sixth review pass, round 5): the drain must
    /// track every message it successfully delivers beyond the first
    /// (which `spawn_process` already stashed synchronously — see
    /// `pending_resume_retry`'s own doc comment) into
    /// `pending_resume_retry`'s own list, so a confirmed stale-resume
    /// retry redelivers the WHOLE batch rather than just the message that
    /// triggered the spawn.
    #[tokio::test]
    async fn drain_appends_later_messages_to_the_pending_resume_retry_without_duplicating_the_first() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let retry_config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("first".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("second".to_string()));
            // Simulates spawn_process's own synchronous stash for "first"
            // — the message that triggered this spawn.
            inner.pending_resume_retry =
                Some(("sid".to_string(), retry_config.clone(), vec!["first".to_string()]));
        }

        c.drain_queue_after_successful_spawn(retry_config, true);

        assert_eq!(rx.recv().await.unwrap(), "first");
        assert_eq!(rx.recv().await.unwrap(), "second");
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let inner = c.inner.lock().unwrap();
        let (_, _, delivered) = inner
            .pending_resume_retry
            .as_ref()
            .expect("must still be tracking this spawn's delivered messages");
        assert!(
            !inner.drain_send_in_flight,
            "must be cleared once the send-then-append sequence for the last message has fully completed"
        );
        assert_eq!(
            delivered,
            &vec!["first".to_string(), "second".to_string()],
            "must contain the ORIGINAL message exactly once plus every later delivery, in order"
        );
    }

    /// codex P2 on PR #2360 (round 15, commit fdb8db6fd): the message
    /// `spawn_process` already stashed into `pending_resume_retry` is not
    /// always the FIRST thing this drain pops — a prior "second stall" can
    /// leave an older leftover queued ahead of a later spawner's own
    /// triggering message (`push_back` appends behind it). A purely
    /// positional "is this the first delivery" check would treat the
    /// OLDER LEFTOVER as if it were the seed (dropping it from tracking
    /// entirely) while recording the ACTUAL trigger message a second time
    /// (once via the synchronous seed, once via this drain's own append).
    /// Confirms content-based matching identifies the true seed regardless
    /// of position: the older leftover is recorded, and the actual trigger
    /// is not duplicated.
    #[tokio::test]
    async fn drain_identifies_the_seed_by_content_even_when_a_leftover_is_delivered_first() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let retry_config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.spawning_in_progress = true;
            // Simulates a "second stall" leaving an older leftover queued
            // ahead of this spawn's own triggering message.
            inner.pending_send_messages.push_back(QueuedMessage::fresh("older-leftover".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("new-trigger".to_string()));
            // spawn_process's own synchronous stash — seeded with the
            // ACTUAL trigger message, not whatever happens to sit at the
            // front of the queue.
            inner.pending_resume_retry =
                Some(("sid".to_string(), retry_config.clone(), vec!["new-trigger".to_string()]));
        }

        c.drain_queue_after_successful_spawn(retry_config, true);

        assert_eq!(rx.recv().await.unwrap(), "older-leftover");
        assert_eq!(rx.recv().await.unwrap(), "new-trigger");
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let inner = c.inner.lock().unwrap();
        let (_, _, delivered) = inner
            .pending_resume_retry
            .as_ref()
            .expect("must still be tracking this spawn's delivered messages");
        assert_eq!(
            delivered.len(),
            2,
            "must contain exactly the older leftover plus the trigger, no omission, no duplication: {delivered:?}"
        );
        assert!(
            delivered.contains(&"older-leftover".to_string()),
            "the older leftover must not be silently dropped from the retry batch"
        );
        assert_eq!(
            delivered.iter().filter(|m| *m == "new-trigger").count(),
            1,
            "the actual trigger message must not be recorded twice"
        );
    }

    /// codex P1 on PR #2360 (sixth review pass, round 6): `poison_resume`
    /// (the stderr-reader task, running concurrently with this drain) can
    /// promote `pending_resume_retry` into `confirmed_stale_resume_retry`
    /// at any point. A message delivered right after that promotion must
    /// still be tracked — appending only to `pending_resume_retry` would
    /// silently drop it from the batch the replacement actually replays.
    #[tokio::test]
    async fn drain_appends_to_confirmed_retry_once_already_promoted_from_pending() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let retry_config = PersistentSpawnConfig {
            cli_command: "unused".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("first".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh("second".to_string()));
            // Simulates poison_resume having ALREADY promoted
            // pending_resume_retry to confirmed before the drain got to
            // "second".
            inner.confirmed_stale_resume_retry =
                Some(("sid".to_string(), retry_config.clone(), vec!["first".to_string()]));
        }

        c.drain_queue_after_successful_spawn(retry_config, true);

        assert_eq!(rx.recv().await.unwrap(), "first");
        assert_eq!(rx.recv().await.unwrap(), "second");
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let inner = c.inner.lock().unwrap();
        let (_, _, delivered) = inner
            .confirmed_stale_resume_retry
            .as_ref()
            .expect("must still be tracking this spawn's delivered messages, even though it was already confirmed");
        assert_eq!(delivered, &vec!["first".to_string(), "second".to_string()]);
    }

    /// codex P2 on PR #2360 (sixth review pass, round 6): deciding and
    /// enqueueing a multi-message retry batch one call at a time left a
    /// window where a genuinely new, unrelated message could interleave
    /// into the MIDDLE of the batch. `decide_retry_batch_action` must
    /// enqueue the whole batch atomically instead.
    #[test]
    fn decide_retry_batch_action_enqueues_the_whole_batch_atomically() {
        let c = controller();
        let action = c.decide_retry_batch_action("first", &["second".to_string(), "third".to_string()]);
        assert!(matches!(action, SendAction::BecomeSpawner));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.iter().cloned().collect::<Vec<_>>(),
            vec!["first".to_string(), "second".to_string(), "third".to_string()],
        );
    }

    /// codex P2 on PR #2360 (sixth review pass, round 6): content-based
    /// dedup must never apply WITHIN a retry batch — two entries can
    /// legitimately be identical (the user genuinely sent the same text
    /// twice, both accepted by the doomed process) and both need
    /// redelivering.
    #[test]
    fn decide_retry_batch_action_preserves_duplicate_content_within_the_batch() {
        let c = controller();
        c.inner.lock().unwrap().spawning_in_progress = true;

        let action =
            c.decide_retry_batch_action("hello", &["hello".to_string(), "hello".to_string()]);

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            3,
            "all three identical entries must be preserved, not deduped"
        );
    }

    /// The dedup check still applies to `first` alone, against whatever
    /// might ALREADY be queued from before this batch decision — e.g. the
    /// drain hasn't reached the original triggering message yet.
    #[test]
    fn decide_retry_batch_action_dedups_only_the_first_against_pre_existing_queue() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh("first".to_string()));
        }

        let action = c.decide_retry_batch_action("first", &["second".to_string()]);

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.iter().cloned().collect::<Vec<_>>(),
            vec!["first".to_string(), "second".to_string()],
            "the already-queued first entry must not be duplicated, but the rest of the batch must still be appended"
        );
    }

    /// codex P2 on PR #2360 (sixth review pass, round 11): the retry
    /// batch represents content the doomed process accepted BEFORE
    /// whatever's already sitting in the queue (which arrived AFTER the
    /// original spawn claimed `spawning_in_progress`) — it must be
    /// delivered first on the fresh process, not appended behind
    /// later-arriving input.
    #[test]
    fn decide_retry_batch_action_prepends_ahead_of_an_unrelated_later_message() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            // "later-message" arrived after the original spawn's claim
            // started, while the retry's own trigger ("A") had already
            // been popped and delivered (and is no longer in the queue —
            // it's now tracked only in the confirmed retry batch).
            inner.pending_send_messages.push_back(QueuedMessage::fresh("later-message".to_string()));
        }

        let action = c.decide_retry_batch_action("A", &[]);

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.iter().cloned().collect::<Vec<_>>(),
            vec!["A".to_string(), "later-message".to_string()],
            "the retry's own (chronologically earlier) message must precede the later, unrelated one"
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
            spawning_in_progress: false,
            pending_send_messages: VecDeque::new(),
            drain_send_in_flight: false,
            current_pid: None,
            stdin_tx: None,
            kill_tx: None,
            spawn_generation: 0,
            stop_requested_generation: None,
            pending_questions: HashMap::new(),
            pending_error_result_line: None,
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

    // codex P1 on PR #2360 (round 16, commit ce1642d90): the exit-handler's
    // retry decision needs to recognize a stop requested for its EXACT
    // generation and consume it exactly once.
    #[test]
    fn consume_stop_requested_for_generation_matches_and_clears() {
        let mut inner = inner_with_session_id(None);
        inner.stop_requested_generation = Some(5);
        assert!(inner.consume_stop_requested_for_generation(5));
        assert_eq!(
            inner.stop_requested_generation, None,
            "must clear once consumed so it can never spuriously affect a later generation"
        );
    }

    // A stop recorded for a DIFFERENT generation (e.g. an older process
    // that already finished handling it, or a not-yet-reached later one)
    // must not be consumed or affect this one.
    #[test]
    fn consume_stop_requested_for_generation_ignores_a_mismatched_generation() {
        let mut inner = inner_with_session_id(None);
        inner.stop_requested_generation = Some(5);
        assert!(!inner.consume_stop_requested_for_generation(7));
        assert_eq!(
            inner.stop_requested_generation,
            Some(5),
            "an unmatched generation must be left untouched, not discarded"
        );
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
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));

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
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));

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
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));

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
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));
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
            Some(("other-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));

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

    // Issue #2368: PR #2360's transparent stale-resume retry fires and
    // usually succeeds within milliseconds, but the doomed attempt's own
    // terminal `result`/`is_error:true` stdout line had already been
    // unconditionally persisted to the blockfile before the retry decision
    // ever ran — leaving a permanent "Agent encountered an error" bubble
    // even though the retry worked. These four tests cover
    // `hold_back_error_result_line_if_resume_pending`'s decision in
    // isolation, and the state combination the process-waiter's exit
    // handler actually reads.

    // While a stale-`--resume` retry could still be confirmed for this
    // exact attempt (`pending_resume_retry` still live), the line must be
    // held back rather than persisted immediately.
    #[test]
    fn holds_back_error_result_line_while_resume_retry_is_pending() {
        let mut inner = inner_with_session_id(None);
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));

        let held_back = inner.hold_back_error_result_line_if_resume_pending(
            r#"{"type":"result","is_error":true}"#.to_string() + "\n",
        );

        assert!(held_back, "must report the line was held back, not persisted");
        assert_eq!(
            inner.pending_error_result_line.as_deref(),
            Some("{\"type\":\"result\",\"is_error\":true}\n"),
            "the exact line must be stashed for the exit handler to resolve"
        );
    }

    // Live-reproduced regression (agent "Marks", 2026-07-30): the stderr
    // reader's poison_resume call landed well before the stdout reader
    // finished draining lines, so by the time the stdout reader reached
    // the doomed line, poison_resume had ALREADY `.take()`n
    // pending_resume_retry into confirmed_stale_resume_retry — a check of
    // pending_resume_retry alone found it `None` and let the line persist
    // immediately, reproducing the exact bug this mechanism exists to fix.
    #[test]
    fn holds_back_error_result_line_when_the_retry_is_already_confirmed() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));
        // Simulates poison_resume having already run and promoted the
        // retry BEFORE the stdout reader ever reaches the doomed line.
        inner.poison_resume("dead-sid");
        assert!(inner.pending_resume_retry.is_none(), "sanity: poison_resume must have consumed it");
        assert!(inner.confirmed_stale_resume_retry.is_some(), "sanity: promoted to confirmed");

        let held_back = inner.hold_back_error_result_line_if_resume_pending(
            r#"{"type":"result","is_error":true}"#.to_string() + "\n",
        );

        assert!(
            held_back,
            "must still hold back the line when the retry was already confirmed by the time \
             the stdout reader got here, not just while it's still merely pending"
        );
        assert!(inner.pending_error_result_line.is_some());
    }

    // No `--resume` was attempted for this generation (or a session id was
    // already captured, clearing `pending_resume_retry`) — this can't be a
    // stale-resume case, so today's immediate-persist behavior must be
    // completely unchanged: nothing is stashed.
    #[test]
    fn does_not_hold_back_error_result_line_when_no_resume_retry_is_pending() {
        let mut inner = inner_with_session_id(None);
        assert!(inner.pending_resume_retry.is_none());

        let held_back = inner.hold_back_error_result_line_if_resume_pending(
            r#"{"type":"result","is_error":true}"#.to_string() + "\n",
        );

        assert!(!held_back, "must not hold back a line with no resume retry in flight");
        assert!(
            inner.pending_error_result_line.is_none(),
            "nothing should be stashed when the line persists immediately"
        );
    }

    // The retry-firing case: once the stderr reader confirms the exact sid
    // this attempt used is unreachable, `poison_resume` promotes the
    // tentative retry to confirmed. The exit handler takes BOTH
    // `confirmed_stale_resume_retry` and `pending_error_result_line`
    // together — this proves they land in the same "retry is firing, drop
    // the stashed line" state at once, exactly what the exit handler's
    // `if let Some(...) = retry_after_resume` branch checks for.
    #[test]
    fn stashed_line_coincides_with_a_confirmed_retry_when_poisoned() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));
        let held_back = inner.hold_back_error_result_line_if_resume_pending(
            r#"{"type":"result","is_error":true}"#.to_string() + "\n",
        );
        assert!(held_back);

        inner.poison_resume("dead-sid");

        assert!(
            inner.confirmed_stale_resume_retry.is_some(),
            "poisoning the exact attempted sid must confirm the retry"
        );
        assert!(
            inner.pending_error_result_line.is_some(),
            "the stashed line must still be present for the exit handler to drop"
        );
    }

    // The genuinely-erroring, no-retry case (e.g. an auth failure or rate
    // limit on the very first message of a fresh `--resume` spawn, before
    // any session id was ever captured): the line gets held back exactly
    // like the stale-resume case (the stdout reader can't yet tell them
    // apart), but `poison_resume` never fires because the stderr reader
    // never saw "No conversation found". The exit handler must see
    // `confirmed_stale_resume_retry: None` alongside a still-present
    // `pending_error_result_line`, so it flushes (not drops) the line —
    // a real first-turn failure must still reach the user.
    #[test]
    fn stashed_line_survives_when_no_poison_ever_fires() {
        let mut inner = inner_with_session_id(None);
        inner.pending_resume_retry =
            Some(("dead-sid".to_string(), dummy_spawn_config(), vec!["{}".to_string()]));
        let held_back = inner.hold_back_error_result_line_if_resume_pending(
            r#"{"type":"result","is_error":true}"#.to_string() + "\n",
        );
        assert!(held_back);

        // No poison_resume call — this generation exits for some unrelated
        // reason (auth failure, rate limit, ...).

        assert!(
            inner.confirmed_stale_resume_retry.is_none(),
            "an unrelated error must never be treated as a confirmed stale-resume retry"
        );
        assert!(
            inner.pending_error_result_line.is_some(),
            "the stashed line must survive so the exit handler can flush it to the user"
        );
    }
}
