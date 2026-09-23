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
//! 2. stdout_reader: process stdout → .jsonl persistence + MPS blockfile events
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
use super::health::TurnActivityTracker;
use super::persistent_resume;
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::subagent_watcher;
use crate::backend::mps;

/// MPS file subject name for persistent subprocess output.
pub const PERSISTENT_OUTPUT_SUBJECT: &str = "output";

pub const BLOCK_CONTROLLER_PERSISTENT: &str = "persistent";

/// What the process-waiter task's kill arm is asked to do.
#[derive(Debug, Clone, Copy)]
enum KillRequest {
    /// Kill now.
    Force,
    /// Close stdin (EOF), wait for the process to exit until the deadline,
    /// then kill.
    Graceful(std::time::Instant),
}

/// The Claude control-protocol interrupt
/// (docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md). Ends the turn in
/// progress — measured at ~2s, with `result: error_during_execution` — while
/// leaving the process and session alive (pane-close spec §5.1).
fn interrupt_control_request_line() -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": format!("agentmux-shutdown-{}", uuid::Uuid::new_v4()),
        "request": { "subtype": "interrupt" },
    })
    .to_string()
}

/// Draws the next process-wide registration nonce (≥ 1) for a persistent
/// spawn's muxbus/registry registrations — see the doc comment at the
/// `my_registration_nonce` binding in `spawn_process` for why this is a
/// srv-wide counter rather than the controller-local spawn generation
/// (codex P1 on PR #2500: generations restart per controller instance
/// and can collide across a `resync_controller` replacement).
fn next_registration_nonce() -> u64 {
    static NEXT_REGISTRATION_NONCE: AtomicU64 = AtomicU64::new(0);
    NEXT_REGISTRATION_NONCE.fetch_add(1, Ordering::Relaxed) + 1
}

/// Builds the NDJSON line for a `persistent_resume::ResumeEffect::
/// EmitSessionOutcome` — a free function (not a method) so it's callable
/// directly from the stdout-reader, process-waiter, and stop-path match
/// arms below, which only hold `_read`/`_wait`-suffixed clones, not
/// `&self`. See SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1.
fn session_outcome_line(
    outcome: persistent_resume::SessionOutcome,
    attempted_sid: String,
    actual_sid: Option<String>,
) -> String {
    let outcome_str = match outcome {
        persistent_resume::SessionOutcome::Resumed => "resumed",
        persistent_resume::SessionOutcome::Fresh => "fresh",
    };
    format!(
        "{}\n",
        serde_json::json!({
            "type": "system",
            "subtype": "agentmux_session_outcome",
            "outcome": outcome_str,
            "attempted_sid": attempted_sid,
            "actual_sid": actual_sid,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })
    )
}

/// Should a spawn that attached NO `--resume` disclose itself as a fresh
/// start (subject to the caller also confirming prior history actually
/// exists — see `has_prior_transcript`)?
///
/// SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.1 deliberately left
/// this case unreported, reasoning that a spawn with no session id has
/// "nothing to lose". That's true for a brand-new agent and false for the
/// case `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
/// recorded: a long-lived named agent opened in a fresh channel whose shared
/// registry pointer is empty gets no `--resume` at all, so the CLI never
/// errors, `persistent_resume` never tracks anything, and no outcome is ever
/// emitted — while the pane renders the entire prior conversation through
/// `blockfile.rs`'s cross-channel read fallback. That is exactly the silent
/// disagreement between "what the pane shows" and "what the model has" that
/// `EmitSessionOutcome` exists to prevent.
///
/// `generation == 1` restricts this to a controller's FIRST spawn. Later
/// generations also spawn without `--resume`, but each already has its own
/// disclosure or deliberately has none: `retry_after_resume_failure`'s
/// no-recovery-candidate path emits `Fresh` itself, and
/// `respawn_once_for_leftover_queue` restarts a session whose fresh-vs-resumed
/// status was decided on an earlier generation. Only a first spawn can be the
/// "pane just opened onto history this process never had" case.
///
/// Kept a pure free function (the FileStore lookup stays at the call site) so
/// the gate is unit-testable without a controller or a spawned process.
fn fresh_start_needs_disclosure(attempted_resume_sid: Option<&str>, generation: u64) -> bool {
    attempted_resume_sid.is_none() && generation == 1
}

#[cfg(test)]
mod fresh_start_disclosure_tests {
    use super::fresh_start_needs_disclosure;

    /// The case STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md
    /// recorded: a pane's first spawn, no registry pointer to resume, prior
    /// history on disk. Nothing else in the resume machinery reports this.
    #[test]
    fn a_first_spawn_with_no_resume_is_disclosed() {
        assert!(fresh_start_needs_disclosure(None, 1));
    }

    /// A resume WAS attempted — `persistent_resume`'s own tracking owns the
    /// outcome from here (Resumed, or Fresh via the retry/recovery cascade).
    /// Disclosing here too would double-report and could contradict it.
    #[test]
    fn a_spawn_that_attempted_a_resume_is_never_disclosed_here() {
        assert!(!fresh_start_needs_disclosure(Some("some-sid"), 1));
        assert!(!fresh_start_needs_disclosure(Some("some-sid"), 4));
    }

    /// Later generations spawn without `--resume` too, but each already has
    /// its own disclosure or deliberately has none —
    /// `retry_after_resume_failure` emits `Fresh` itself when no recovery
    /// candidate exists, and `respawn_once_for_leftover_queue` restarts a
    /// session already decided on an earlier generation. Re-disclosing would
    /// stack a second divider onto an unchanged conversation.
    #[test]
    fn a_later_generation_respawn_is_not_re_disclosed() {
        assert!(!fresh_start_needs_disclosure(None, 2));
        assert!(!fresh_start_needs_disclosure(None, 17));
    }

    /// Generation 0 never reaches a spawn (`spawn_process` bumps before use),
    /// but the gate must not treat the sentinel as a first spawn.
    #[test]
    fn generation_zero_is_not_treated_as_a_first_spawn() {
        assert!(!fresh_start_needs_disclosure(None, 0));
    }
}

/// Publish a `mps::EVENT_AGENT_RESUME_RETRY` status ping — a free function
/// (not a method) for the same reason `session_outcome_line` above is one:
/// callable from the stdout-reader/process-waiter match arms, which only
/// hold `_read`/`_wait`-suffixed clones, not `&self`. `status` is `"retrying"`
/// (set when a stale `--resume` is detected and a retry/recovery attempt is
/// about to fire) or `"resolved"` (set the moment the retry's outcome —
/// Fresh or Resumed — is actually known). See
/// `docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md` §6.2.
/// No-ops if `broker` is `None` (tests / non-wired controllers), same
/// posture as every other best-effort MPS publish in this file.
fn publish_resume_retry_status(broker: &Option<Arc<mps::Broker>>, block_id: &str, status: &str) {
    let Some(broker) = broker else { return };
    let mut data = serde_json::json!({ "status": status });
    if status == "retrying" {
        data["startedAt"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
    }
    broker.publish(mps::MuxEvent {
        event: mps::EVENT_AGENT_RESUME_RETRY.to_string(),
        scopes: vec![format!("block:{}", block_id)],
        sender: String::new(),
        // `persist: 2`, not 1 — `Broker::persist_event` trims history down
        // to exactly this many MOST RECENT events per (event, scope): a
        // bare `persist: 1` would let the "resolved" publish immediately
        // evict "retrying" from history, so a pane that (re)subscribes in
        // the narrow window right after resolution sees only "resolved"
        // with no record a retry ever happened — harmless for the simple
        // "currently reconnecting?" check, but makes this event's own
        // history useless for anything else. 2 keeps the latest
        // retrying→resolved pair together; a later cascaded "retrying"
        // still correctly evicts the OLDER pair's "retrying", not this
        // one's "resolved".
        persist: 2,
        data: Some(data),
    });
}

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
/// `PartialEq` is derived for `persistent_resume::RetryPayload`'s own
/// derive and its exhaustive unit tests — not used elsewhere in this
/// file itself.
#[derive(Debug, Clone, PartialEq)]
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
    /// Set when a forced controller resync (a `/model`, `/effort` or
    /// `/permission-mode` change — see `frontend/.../runtime-apply.ts`) lands
    /// while a turn is in flight. The restart is DEFERRED to the end of that
    /// turn instead of killing the process mid-turn.
    ///
    /// Killing it was the old behaviour and it silently destroyed the user's
    /// in-flight message: `stop_process` records a `StopRequested`, which the
    /// resume state machine treats as an explicit user stop and therefore
    /// suppresses the retry; `remove_controller_entry_only` then discards the
    /// controller (and its queue) outright; and the replacement controller
    /// "spawns on first message", so nothing resumed. The message had already
    /// been written to the dead process's stdin, so no queue entry survived to
    /// replay. Net effect: the turn vanished with no response and no error —
    /// the pane simply went quiet. Diagnosed live on AgentX, 2026-08-28.
    ///
    /// Deferring is correct rather than merely safer: model/effort flags are
    /// baked in at spawn, so they could not have applied to the running turn
    /// anyway. Waiting until it ends costs nothing and loses nothing.
    restart_when_idle: bool,
    /// A deferred restart has COMMITTED — `stop_process` has been called and
    /// the process is on its way out, but `stdin_tx` is still live because
    /// `stop_process` only sends on `kill_tx` and leaves the writer channel
    /// alone until the process actually exits.
    ///
    /// Without this, `decide_send_action` would see a live `stdin_tx` and take
    /// `DeliverDirect` for any message arriving in that window — writing it
    /// into a process about to receive EOF, acknowledging it, and losing it
    /// (codex P1 on PR #2858). The window is not hypothetical: the restart
    /// fires exactly at turn end, which is precisely when the frontend flushes
    /// its queued follow-ups.
    ///
    /// Set under the same lock that consumes `restart_when_idle`, cleared when
    /// the replacement process spawns.
    restart_pending: bool,
    /// This spawn generation's stale-`--resume` retry decision, plus any
    /// held-back terminal error-result line — see
    /// `persistent_resume::ResumeState`'s own doc comment for the full
    /// design rationale. Replaces what used to be four separate fields
    /// (`pending_resume_retry`, `confirmed_stale_resume_retry`,
    /// `stop_requested_generation`, `pending_error_result_line`) mutated
    /// directly by four independently-scheduled tasks racing on this same
    /// mutex — that shape caused issue #2368 (and a live-reproduced
    /// recurrence, agent "Marks", 2026-07-30) because no single owner
    /// enforced valid transitions between them. Every mutation now goes
    /// through `persistent_resume::update()`, a pure, exhaustively unit
    /// tested `(state, event) -> (state, effects)` function — still
    /// called under this same mutex (no new concurrency primitive is
    /// introduced), but as ONE call per event instead of several separate
    /// field reads/writes that could observe each other mid-transition.
    resume: persistent_resume::ResumeState,
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
    /// Exclusive claim held by a stale-resume retry batch flush (issue
    /// #2367; spec §4 option 2 of
    /// SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09).
    /// Taken by `decide_retry_batch_action` in the SAME lock acquisition
    /// in which it decides a live, newer-generation process should
    /// receive the batch; while held, `decide_send_action`'s
    /// `DeliverDirect` branch routes to `Queued` instead (exactly how it
    /// already treats `spawning_in_progress`), making the queue the
    /// single ordering authority — a concurrent send can no longer
    /// `try_send` ahead of earlier-accepted retry messages. Released by
    /// the flush task (`drain_queue_with_claim`) once the queue runs dry
    /// or the process dies. Deliberately a NEW flag, not a reuse of
    /// `spawning_in_progress` — round-14's starvation analysis stands:
    /// that flag is never pre-asserted while no spawn is in progress.
    drain_claim: bool,
    /// Monotonic counter behind `QueuedMessage::seq` (issue #2365) —
    /// pre-incremented at every fresh enqueue, so real seqs start at 1
    /// and are never reused for this controller's lifetime. Redelivery
    /// paths (a stale-resume retry batch) preserve a message's original
    /// seq instead of drawing a new one — that preservation is what
    /// makes "already queued?" an identity check rather than a content
    /// comparison.
    next_message_seq: u64,
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
    kill_tx: Option<tokio::sync::oneshot::Sender<KillRequest>>,
    /// Set by [`Controller::shutdown`] to the spawn generation it is closing,
    /// BEFORE it interrupts the turn. The interrupted turn ends in an
    /// `is_error: true` result (`error_during_execution`) that is the stop we
    /// asked for, not a failure — the stdout reader must not classify it,
    /// raise a failure banner, or feed it to the stale-`--resume` retry
    /// machine (spec §9.4).
    shutdown_generation: Option<u64>,
    /// Set by the kill arm the moment a requested stop's process has exited:
    /// `(spawn generation, killed)`. `killed` = it had to be force-killed
    /// (graceful deadline passed, or a forced stop). Watched by `shutdown`,
    /// which must not wait on the kill arm's later cleanup (that can take
    /// seconds when a descendant holds stdout open).
    stop_exit: Option<(u64, bool)>,
    /// Monotonic counter bumped once per `spawn_process` call (in the same
    /// lock acquisition as stashing `pending_resume_retry`), uniquely
    /// identifying that one spawn attempt for the rest of this controller
    /// instance's lifetime. Note this is NOT the `spawn_epoch`/
    /// `should_skip_own_delivery` mechanism removed earlier in this same
    /// PR (see `spawning_in_progress`'s own doc comment) — that existed to
    /// dedup a spawn-claim race, a job `spawning_in_progress` now fully
    /// owns. This counter exists for a different, narrower purpose: giving
    /// `stop_requested_generation` something stable to compare against.
    ///
    /// ALSO bumped (without a spawn) by
    /// `clear_session_id_for_fresh_spawn` to atomically retire every
    /// existing generation's reader tasks alongside a session-id clear —
    /// see its doc comment (codex P1 on PR #2500, second round). The
    /// resulting numbering gap is deliberate and inert: generations are
    /// only ever compared for equality; the invariant is "never reused,"
    /// not "no gaps."
    spawn_generation: u64,
    /// AskUserQuestion `can_use_tool` control_requests awaiting a user answer:
    /// `tool_use_id -> (request_id, questions JSON)`. Filled by the stdout
    /// reader when the CLI sends a `can_use_tool` control_request for
    /// AskUserQuestion; consumed by `answer_question` to build the matching
    /// `control_response`. Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    pending_questions: HashMap<String, (String, serde_json::Value)>,
    /// Ordinary (non-AskUserQuestion) tool-permission `can_use_tool`
    /// control_requests awaiting a user decision: `tool_use_id ->
    /// (request_id, tool_name, input JSON)`. Filled by
    /// `park_tool_permission_request` when `should_route_to_decision_panel`
    /// says so; consumed by `decide_tool_permission` to build the matching
    /// `control_response`. Structurally the sibling of `pending_questions`
    /// above — same in-memory-only, per-controller-instance lifetime — but
    /// currently always empty in production: `should_route_to_decision_panel`
    /// is hardcoded `false` pending the Phase 2 policy decision
    /// (SPEC_DECISION_PROMPT_2026_04_24.md, SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §5).
    pending_permissions: HashMap<String, (String, String, serde_json::Value)>,
}

impl PersistentInner {
    /// Draws the next message seq (issue #2365) — pre-incremented, so
    /// real seqs start at 1 and 0 can serve as "never a valid seq" in
    /// tests. Must be called under the same lock acquisition as the
    /// enqueue it identifies.
    fn take_next_message_seq(&mut self) -> u64 {
        self.next_message_seq += 1;
        self.next_message_seq
    }

    /// Applies one `persistent_resume::ResumeEvent` to `self.resume`,
    /// storing the resulting state back and returning the effects for
    /// the caller to execute. The one and only place `self.resume` is
    /// ever mutated — see `persistent_resume`'s module doc comment for
    /// why routing every transition through this single pure function
    /// (instead of several fields each caller used to read/write
    /// directly) closes the exact race class issue #2368 kept
    /// resurfacing.
    fn apply_resume_event(
        &mut self,
        event: persistent_resume::ResumeEvent,
    ) -> Vec<persistent_resume::ResumeEffect> {
        let (new_state, effects) = persistent_resume::update(std::mem::take(&mut self.resume), event);
        self.resume = new_state;
        effects
    }

    /// Records `bad_sid` as confirmed-unreachable (the CLI reported "No
    /// conversation found" for it) and clears it from `session_id` if it's
    /// currently held there. Pairs with `try_capture_session_id` below —
    /// whichever of the stderr/stdout reader tasks runs first, the
    /// poisoned id never survives as the live `session_id`. `generation`
    /// is the spawn attempt this poison applies to — see
    /// `persistent_resume::ResumeEvent`'s own doc comment for why every
    /// event carries one.
    fn poison_resume(&mut self, bad_sid: &str, generation: u64) {
        self.resume_poisoned = Some(bad_sid.to_string());
        if self.session_id.as_deref() == Some(bad_sid) {
            self.session_id = None;
        }
        // Promotion to a confirmed retry (only if `bad_sid` matches this
        // generation's actual attempted sid — see
        // `persistent_resume::update`'s `ResumeUnreachable` handling) now
        // happens inside `update()` itself.
        self.apply_resume_event(persistent_resume::ResumeEvent::ResumeUnreachable {
            generation,
            sid: bad_sid.to_string(),
        });
    }

    /// Attempts to adopt `sid` as the live session id. Returns `false`
    /// (does not adopt) if a session id is already held, or if `sid` is
    /// the confirmed-poisoned id from a prior `poison_resume` call — the
    /// CLI echoes back whatever `--resume` it was given as its first
    /// stdout line even when that resume goes on to fail, so without this
    /// check a losing race would silently re-adopt a known-dead id right
    /// after `poison_resume` cleared it. A genuinely different (fresh)
    /// sid is unaffected and still captured normally. `generation` is the
    /// spawn attempt this capture applies to.
    ///
    /// codex P1 on PR #2371: resolves the tentative/confirmed retry
    /// tracking whenever `sid` is confirmed genuine — NOT only on the
    /// `adopted` (session_id was previously `None`) branch. A
    /// `--resume <sid>` spawn ALWAYS has `session_id` already `Some`
    /// BEFORE the process even starts (that's what makes `--resume` get
    /// attached at all — see `spawn_process`), so on the common
    /// resume-SUCCEEDED case the CLI's echoed sid matches what's already
    /// held and this always used to return `false` immediately, WITHOUT
    /// ever resolving tracking. Since persistent mode never exits between
    /// turns, that would leave a generation's resume state live for the
    /// rest of this potentially long-lived process's life, wrongly
    /// holding back every LATER, completely unrelated `is_error:true`
    /// result as if it might still need to be dropped for a stale-resume
    /// retry.
    ///
    /// reagentx P0 on PR #2371: `is_confirmed_success` (true only for a
    /// terminal `result` with `is_error:false`) is threaded through to
    /// `persistent_resume::update`, which is what ACTUALLY decides
    /// whether this capture is unambiguous enough to resolve tracking —
    /// see `ResumeEvent::SessionCaptured`'s own doc comment for why a
    /// same-sid echo alone (e.g. an early "system"/init frame) is NOT
    /// proof of genuine progress: the CLI echoes the attempted sid
    /// regardless of whether the resume goes on to fail.
    ///
    /// reagentx P0 on PR #2373: returns the effects `apply_resume_event`
    /// produced, not just the `adopted` bool — resolving tracking here
    /// can legitimately emit `FlushErrorLine` (an EARLIER turn on this
    /// same still-alive generation held an error line back before this
    /// LATER capture resolved tracking — see `SessionCaptured`'s own
    /// handling in `persistent_resume::update`). Discarding that
    /// silently lost the held-back line instead of flushing it,
    /// reproducing the exact #2368 bug class this PR exists to fix. The
    /// caller (the stdout reader) is responsible for actually executing
    /// it, same as every other `ResumeEffect` this module produces.
    fn try_capture_session_id(
        &mut self,
        sid: &str,
        generation: u64,
        is_confirmed_success: bool,
    ) -> (bool, Vec<persistent_resume::ResumeEffect>) {
        if self.resume_poisoned.as_deref() == Some(sid) {
            return (false, vec![]);
        }
        // reagentx P1 (round 4 on this PR): `adopted` used to be gated
        // solely on `session_id.is_none()` — but a `--resume <sid>` spawn
        // ALWAYS hydrates `session_id` to the attempted sid BEFORE the
        // process even starts (`spawn_process`), so that check can never
        // be true for a resume attempt. When the CLI genuinely gives up
        // on `--resume` internally and starts a fresh conversation with
        // a DIFFERENT sid, `session_id` was left stuck on the stale
        // attempted one — the controller believed it was still talking
        // to the old conversation while the live process had moved on.
        // `adopted` is now true whenever the captured sid actually
        // differs from whatever `session_id` currently holds AND either
        // nothing was captured yet at all, or this exact call is what
        // resolves resume tracking (checked via the state transition,
        // since that's the same unambiguous decision `update()` itself
        // already makes for `SessionCaptured`).
        let sid_is_new = self.session_id.as_deref() != Some(sid);
        let session_id_was_none = self.session_id.is_none();
        let was_tracking = !matches!(self.resume, persistent_resume::ResumeState::NotTracking { .. });
        let effects = self.apply_resume_event(persistent_resume::ResumeEvent::SessionCaptured {
            generation,
            sid: sid.to_string(),
            is_confirmed_success,
        });
        let just_resolved_tracking = was_tracking && matches!(self.resume, persistent_resume::ResumeState::NotTracking { .. });
        // Ambient adoption is gated on generation currency (issue #2366):
        // the state machine already drops a stale generation's
        // `SessionCaptured` for TRACKING purposes, but the
        // `session_id_was_none` arm below used to adopt regardless — so a
        // doomed generation's still-draining stdout reader, echoing its
        // stale attempted sid moments after `respawn_once_for_leftover_
        // queue`'s plain clear of `session_id` (see that function's
        // round-13 comment, which deferred exactly this race), could
        // re-install the stale sid into ambient state. `resume_poisoned`
        // doesn't cover that: the fallback path deliberately does NOT
        // poison (the death may have nothing to do with a stale resume),
        // and the fallback respawn passes `resume_retry_payload: None`,
        // so a later spawn re-attaching `--resume <stale-sid>` would
        // fail with nothing re-armed to catch it. A newer spawn owns the
        // session identity by definition — a superseded generation's
        // capture must never adopt.
        let is_current_generation = generation == self.spawn_generation;
        let adopted = is_current_generation && sid_is_new && (session_id_was_none || just_resolved_tracking);
        if adopted {
            self.session_id = Some(sid.to_string());
        }
        (adopted, effects)
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
    /// for the post-spawn drain. `own_seq` is that enqueued message's
    /// `QueuedMessage::seq`, handed back so the caller can later tell
    /// `release_spawn_claim_and_drain_queue` exactly which entry was its
    /// own (issue #2365 — content matching could discard a different,
    /// identical-text message instead).
    BecomeSpawner { own_seq: u64 },
    /// Another caller is already spawning (or a retry-batch flush holds
    /// the drain claim — issue #2367) — this message has been enqueued
    /// for that claim-holder's own drain to deliver.
    Queued,
}

/// What `decide_retry_batch_action` determined a stale-resume retry
/// batch's fate should be (issue #2367; spec §4). Split from
/// [`SendAction`] because the batch's live-process outcome is not a
/// direct delivery: the batch goes through the queue under the drain
/// claim, never through a caller's own `try_send`.
enum RetryBatchAction {
    /// A live, NEWER-generation process is running with no claim held.
    /// The batch was prepended to the queue AND `drain_claim` was taken
    /// in the same lock acquisition that decided this — the caller must
    /// start the queue flush
    /// (`drain_queue_with_claim(.., QueueDrainClaim::RetryFlush)`).
    FlushClaimed,
    /// Someone else holds a claim (a spawn in flight, or another flush)
    /// — the batch was prepended for that claim-holder's drain.
    Queued,
    /// Nobody holds a claim and no process is running — this retry
    /// claimed the exclusive spawn right; `own_seq` is the batch's first
    /// entry's seq (see `SendAction::BecomeSpawner`).
    BecomeSpawner { own_seq: u64 },
}

/// Which exclusivity claim a queue-drain task runs under — the spawn
/// claim (`spawning_in_progress`, the post-spawn drain) or the retry
/// flush claim (`drain_claim`, issue #2367). The task must release
/// exactly the claim its initiator took, and nothing else.
#[derive(Clone, Copy, PartialEq)]
enum QueueDrainClaim {
    SpawnClaim,
    RetryFlush,
}

impl QueueDrainClaim {
    fn release(self, inner: &mut PersistentInner) {
        match self {
            QueueDrainClaim::SpawnClaim => inner.spawning_in_progress = false,
            QueueDrainClaim::RetryFlush => inner.drain_claim = false,
        }
    }
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
    /// Queue identity (issue #2365): assigned once from
    /// `PersistentInner::next_message_seq` at first enqueue and preserved
    /// verbatim across redelivery (`QueuedRetryEntry::seq` carries it
    /// through a stale-resume retry batch), so dedup / own-message /
    /// seed checks are exact identity — two genuinely different messages
    /// with identical text can never be conflated.
    seq: u64,
    json_str: String,
    already_persisted: bool,
}

impl QueuedMessage {
    fn fresh(seq: u64, json_str: String) -> Self {
        Self { seq, json_str, already_persisted: false }
    }

    fn already_persisted(seq: u64, json_str: String) -> Self {
        Self { seq, json_str, already_persisted: true }
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

/// Result of `try_eager_resume` — see `start()`'s doc comment. `DeclinedTo`
/// carries a short, non-exhaustive reason for the log line; it is not meant
/// to be pattern-matched on by callers, just read.
enum EagerResumeOutcome {
    Spawned,
    DeclinedTo(&'static str),
}

/// PersistentSubprocessController keeps a long-running CLI process alive,
/// sending user messages as NDJSON lines on stdin.
pub struct PersistentSubprocessController {
    #[allow(dead_code)]
    tab_id: String,
    block_id: String,
    inner: Arc<Mutex<PersistentInner>>,
    broker: Option<Arc<mps::Broker>>,
    event_bus: Option<Arc<EventBus>>,
    mstore: Option<Arc<Store>>,
    /// FileStore for write-through persistence of output lines (Phase 1.3).
    filestore: Option<Arc<FileStore>>,
    /// Dependencies for `start()`'s eager-resume path
    /// (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`).
    /// Set via `with_identity_stores()`, not `new()` — see that method's own
    /// doc comment for why this is an opt-in builder step rather than a
    /// required constructor argument.
    id_store: Option<Arc<Store>>,
    identity_store: Option<Arc<Store>>,
    /// Empty when never supplied (every existing `new()` caller) — `start()`
    /// treats an empty `auth_key` as "identity stores unavailable" the same
    /// as `id_store`/`identity_store` being `None`, since a real spawn
    /// always has a real key.
    auth_key: String,
    health_monitor: Arc<TurnActivityTracker>,
    /// Monotonic counter bumped for every stdout line (including control frames).
    /// The AskUserQuestion dead-air fallback snapshots this *before* sending the
    /// answer and re-checks after a short window; any increment means the CLI
    /// produced output (assistant content OR a follow-up control_request), i.e.
    /// the turn resumed. Counting *all* frames — including control frames the
    /// reader otherwise skips — avoids a spurious fallback when the resumed
    /// turn's first activity is a tool-permission round-trip.
    stdout_seq: Arc<AtomicU64>,
    /// Weak self-reference for the stale-`--resume`-session retry (see
    /// `retry_after_resume_failure`) — set by `set_self_ref` right after
    /// construction. The process-waiter task (a detached `tokio::spawn`
    /// that only captures cloned Arc fields, never `&self`) needs a way to
    /// call back into an instance method once the underlying process
    /// actually exits; a `Weak` avoids a reference cycle (this same
    /// struct's `spawn_process` is what schedules that task).
    self_ref: Mutex<Option<std::sync::Weak<Self>>>,
    /// This controller's own muxbus/jekt identity, captured once at spawn
    /// time from `muxbus_agent_id_from_env` — the independent source of
    /// truth `Controller::agent_id()` exposes for `inject_message_inner`'s
    /// recipient-identity check. Deliberately its own field, not part of
    /// `PersistentInner` — `config` (and its `env_vars`, where the identity
    /// is read from) isn't available until `spawn_process`, well after
    /// `new()`, and this value doesn't participate in any of `PersistentInner`'s
    /// carefully-ordered spawn/resume/queue state transitions. `None` for a
    /// persistent block with no muxbus identity set (a non-agent process).
    agent_id: Mutex<Option<String>>,
    /// The STABLE jekt identity (`AGENTMUX_AGENT_ID`), captured once in
    /// `spawn_process` and never overwritten afterward — unlike `agent_id`
    /// above, which `input.rs`'s Register-tail deliberately refreshes every
    /// turn to track the live, renameable display name (#2697). Exists
    /// specifically so `Controller::stable_agent_id()` can back the jekt
    /// registry's alias entry and the #2695 recipient-identity check even
    /// after `agent_id` has moved on to a post-rename value. See
    /// `INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`.
    stable_agent_id: Mutex<Option<String>>,
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

/// Fixed, server-owned decline message for a cancelled AskUserQuestion (the
/// Cancel button / Escape in `AgentQuestionPanel.tsx`) — not client-suppliable,
/// since Cancel and Escape both mean exactly one thing and there is no user-
/// authored content to carry. See `deny_question` and
/// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
///
/// KEEP IN SYNC (no shared constant crosses the Rust/TypeScript boundary for
/// a plain string literal): `CANCEL_FALLBACK_MESSAGE` in
/// `frontend/app/view/agent/hooks/useAgentQuestions.ts` is a hand-copied
/// mirror of this exact text, used only for its SAFE_TO_RETRY_VIA_FOLLOWUP
/// fallback path. If you change this string, update that one too — the test
/// `ask_user_question_deny_message_matches_frontend_cancel_fallback_text`
/// below pins the literal on this side; the frontend test asserting
/// `sendMessage` was called with this exact text (in
/// `useAgentQuestions.test.ts`, "handleCancel fallback" describe block)
/// pins it on that side. Neither test can see the other language's
/// constant, so both must be updated by hand together.
pub(crate) const ASK_USER_QUESTION_DENY_MESSAGE: &str = "The user declined to answer this question.";

/// Compose the directive follow-up message used by the AskUserQuestion dead-air
/// fallback when the answer was a DECLINE rather than a real answer. Mirrors
/// `build_answer_resume_message`'s directive tone (resume the task, don't wait
/// for further input) but tells the model the outcome was a decline so it
/// doesn't hallucinate a value the user never provided.
fn build_deny_resume_message(message: &str) -> String {
    format!(
        "[AgentMux] Your earlier question was declined, but the turn had already ended, \
         so this is delivered here as a follow-up. Resume the task you were working on \
         without this input — do not wait for further input. The user's response was: \
         \"{message}\""
    )
}

/// Whether a `can_use_tool` control_request for `tool_name` (anything except
/// AskUserQuestion, which is always parked separately — see
/// `handle_control_frame`) should be routed to the frontend decision panel
/// instead of auto-allowed.
///
/// **Hardcoded to `false` — deliberately inert.** This is the one remaining
/// product decision `SPEC_DECISION_PROMPT_2026_04_24.md` Phase 2 and
/// `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md` §5 Phase 2 both leave open,
/// and it is NOT safe to default it to "route everything non-trivial" —
/// that spec's own §7 risk section says so directly:
///
/// > Permission chatter: `--permission-mode default` may route many tools
/// > through `can_use_tool`. Mitigation: auto-allow in the handler (Phase 1);
/// > consider `--permission-mode acceptEdits` or allow-rules to cut volume.
///
/// `--permission-mode default` is the mode baked into every persistent
/// Claude agent's launch args today (`static CLAUDE` in
/// `agentmux-srv/src/backend/providers.rs`), and it routes most non-trivial
/// tool calls through this control_request. Flipping this function to
/// unconditionally `true` would turn every persistent Claude agent into a
/// prompt-per-tool-call experience the instant it ships — a real regression,
/// not a hypothetical one, and not something an engineering change should
/// decide unilaterally.
///
/// The parking/decide mechanism this gate controls —
/// `park_tool_permission_request` below and
/// `PersistentSubprocessController::decide_tool_permission` — is complete
/// and independently tested (see `tool_permission_tests` below); flipping
/// this gate on is a policy call about which tools or modes actually
/// warrant a prompt (a risk-tiering scheme per
/// `SPEC_DECISION_PROMPT_2026_04_24.md`'s own speculative `risk` field,
/// `--permission-mode acceptEdits` instead of `default`, or persisted
/// allow-rules per that spec's §6 — none of which exist yet), not an
/// engineering task this function should pre-empt.
fn should_route_to_decision_panel(_tool_name: &str) -> bool {
    false
}

/// Park an ordinary (non-AskUserQuestion) tool-permission `can_use_tool`
/// control_request into `PersistentInner::pending_permissions`, mirroring
/// how AskUserQuestion is parked into `pending_questions` two branches up in
/// `handle_control_frame` — same in-memory-only, per-controller-instance
/// lifetime, and the same "process respawned" failure mode on decide
/// (`PersistentSubprocessController::decide_tool_permission`'s error text
/// explains it the same way `answer_question`'s does).
///
/// Currently unreachable in production: `should_route_to_decision_panel`
/// always returns `false`, so `handle_control_frame` never calls this. Kept
/// as its own function (rather than inlined, unlike the AskUserQuestion
/// branch) specifically so it can be unit tested directly, independent of
/// that gate.
fn park_tool_permission_request(
    inner: &Arc<Mutex<PersistentInner>>,
    tool_use_id: String,
    request_id: String,
    tool_name: String,
    input: serde_json::Value,
) {
    let mut guard = inner.lock().unwrap();
    guard
        .pending_permissions
        .insert(tool_use_id, (request_id, tool_name, input));
}

/// Compose the directive follow-up message used by the tool-permission
/// dead-air fallback (mirrors `build_answer_resume_message` /
/// `build_deny_resume_message`'s directive tone — resume the task, don't
/// wait for further input — but for an ordinary tool-permission decision
/// rather than an AskUserQuestion answer).
fn build_tool_decision_resume_message(tool_name: &str, outcome: &str, feedback: Option<&str>) -> String {
    if outcome == "allow" {
        format!(
            "[AgentMux] Your earlier request to use `{tool_name}` was approved, but the \
             turn had already ended, so this is delivered here as a follow-up. Resume the \
             task you were working on — the tool call is approved; do not wait for further \
             input."
        )
    } else {
        let reason = feedback.unwrap_or("Denied by user.");
        format!(
            "[AgentMux] Your earlier request to use `{tool_name}` was declined, but the \
             turn had already ended, so this is delivered here as a follow-up. Resume the \
             task you were working on without using that tool — do not wait for further \
             input. The user's response was: \"{reason}\""
        )
    }
}

impl PersistentSubprocessController {
    pub fn new(
        tab_id: String,
        block_id: String,
        broker: Option<Arc<mps::Broker>>,
        event_bus: Option<Arc<EventBus>>,
        mstore: Option<Arc<Store>>,
        filestore: Option<Arc<FileStore>>,
    ) -> Self {
        let health_monitor = Arc::new(TurnActivityTracker::new(block_id.clone()));
        Self {
            tab_id,
            block_id,
            inner: Arc::new(Mutex::new(PersistentInner {
                proc_status: STATUS_INIT.to_string(),
                proc_exit_code: 0,
                status_version: 0,
                session_id: None,
                resume_poisoned: None,
                restart_when_idle: false,
                restart_pending: false,
                resume: persistent_resume::ResumeState::default(),
                spawning_in_progress: false,
                pending_send_messages: VecDeque::new(),
                drain_claim: false,
                next_message_seq: 0,
                drain_send_in_flight: false,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                shutdown_generation: None,
                stop_exit: None,
                spawn_generation: 0,
                pending_questions: HashMap::new(),
                pending_permissions: HashMap::new(),
            })),
            broker,
            event_bus,
            mstore,
            filestore,
            id_store: None,
            identity_store: None,
            auth_key: String::new(),
            health_monitor,
            stdout_seq: Arc::new(AtomicU64::new(0)),
            self_ref: Mutex::new(None),
            agent_id: Mutex::new(None),
            stable_agent_id: Mutex::new(None),
        }
    }

    /// Supplies the dependencies `start()`'s eager-resume path needs
    /// (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`):
    /// the Layer 3 identity/credential spawn gate's two stores, and the
    /// auth key a spawned CLI needs for `AGENTMUX_AUTH_KEY` (MPS publish
    /// scoping via the bashwrap-invoked tool wrapper).
    ///
    /// A separate builder step, not three more `new()` parameters, so every
    /// existing caller — the ~15 test constructions in this file among
    /// them — is unaffected. A controller built without this call simply
    /// cannot eager-resume even when `agent:sessionid` is present; it falls
    /// back to `start()`'s ordinary lazy "wait for the first message" path
    /// rather than spawning without the identity gate. That fallback is the
    /// deliberately safe default: eagerly resuming a possibly-deauthed
    /// agent's session on whatever ambient credential happens to be present
    /// would silently reintroduce the exact vulnerability class
    /// `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` closed. See
    /// `start()`'s own doc comment for the full reasoning.
    pub fn with_identity_stores(
        mut self,
        id_store: Option<Arc<Store>>,
        identity_store: Option<Arc<Store>>,
        auth_key: String,
    ) -> Self {
        self.id_store = id_store;
        self.identity_store = identity_store;
        self.auth_key = auth_key;
        self
    }

    /// Atomically claims the exclusive spawn right `decide_send_action`'s
    /// `BecomeSpawner` branch also claims, or declines if one is already
    /// held (`stdin_tx` live, another spawn in flight, or a drain in
    /// progress). `true` means the caller now owns the claim and MUST
    /// release it — either directly (a decline with no message ever
    /// delivered) or via `drain_queue_after_successful_spawn`/
    /// `respawn_once_for_leftover_queue` (a spawn attempt was made).
    ///
    /// Split out of `try_eager_resume` specifically so this state
    /// transition is unit-testable on its own — codex P1 on PR #3513
    /// (the spawn-claim race) found that PR's first fix, tested only
    /// end-to-end, left this exact step's own removal undetected by every
    /// test in the file (confirmed by mutation: deleting the claim block
    /// entirely still passed all 6 eager-resume tests, since none of them
    /// exercised `try_eager_resume`'s OWN claim step in isolation).
    fn try_claim_eager_resume_spawn(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() || inner.spawning_in_progress || inner.drain_claim {
            return false;
        }
        inner.spawning_in_progress = true;
        true
    }

    /// `start()`'s eager-resume attempt for a pane with a captured
    /// `agent:sessionid`. Never returns an error — every failure mode here
    /// is "fall back to lazy", not "fail the resync"; see `start()`'s own
    /// doc comment for why.
    fn try_eager_resume(&self, block_meta: &super::super::obj::MetaMapType, session_id: &str) -> EagerResumeOutcome {
        let (Some(id_store), Some(identity_store)) = (self.id_store.clone(), self.identity_store.clone()) else {
            return EagerResumeOutcome::DeclinedTo("identity stores not configured for this controller");
        };
        if self.auth_key.is_empty() {
            return EagerResumeOutcome::DeclinedTo("no auth key configured for this controller");
        }
        let Some(mstore) = self.mstore.clone() else {
            return EagerResumeOutcome::DeclinedTo("no mstore configured for this controller");
        };

        // `cli_command`/`cli_args`/`working_dir` — same meta keys a live
        // message's spawn config reads. Deliberately NOT
        // `agent_handlers::input`'s own print-mode fallback default for a
        // missing `cmd:args`: that default is defensive for a code path this
        // eager-resume one never actually reaches in practice
        // (`agent_open.rs` always seeds `cmd:args` for every persistent
        // agent at launch time), and guessing the wrong CLI mode for a
        // persistent agent is worse than just not eager-resuming.
        let cli_command = crate::backend::obj::meta_get_string(block_meta, "cmd", "claude");
        let cli_args: Vec<String> = match block_meta.get("cmd:args") {
            Some(serde_json::Value::Array(arr)) => {
                arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
            }
            _ => return EagerResumeOutcome::DeclinedTo("cmd:args meta missing — refusing to guess CLI flags"),
        };
        let working_dir = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
        let base_env_vars: HashMap<String, String> = match block_meta.get("cmd:env") {
            Some(serde_json::Value::Object(obj)) => {
                obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()
            }
            _ => HashMap::new(),
        };
        let resume_flag = crate::backend::obj::meta_get_string(block_meta, "agent:resume_flag", "--resume");
        let session_id_field = crate::backend::obj::meta_get_string(block_meta, "agent:session_id_field", "session_id");

        // Identity gate, MuxBus token, reserved wrapper vars (unconditional
        // overwrite — AGENTMUX_AUTH_KEY/BLOCKID), agent identity, git
        // identity, tools PATH: the SAME function a live message send
        // builds its env with (`agent_handlers::input::
        // build_persistent_spawn_env`), not a second, independent copy.
        // codex P1 on PR #3513 found an earlier revision of this method had
        // built its own partial copy that silently drifted: missing PATH/
        // MuxBus injection entirely, and `entry().or_insert()` instead of
        // an unconditional overwrite for the two reserved variables (so a
        // stale persisted value for either would have survived an eager
        // resume that a live message send would have corrected).
        //
        // Claim exclusivity BEFORE the (possibly slow) identity/env
        // resolution below, not just before the eventual `spawn_process`
        // call — codex P1 on PR #3513: the controller is placed in the
        // global registry before `start()` runs (`resync_controller`'s
        // "Create new controller" branch), so a concurrent message can
        // reach `send_message` -> `decide_send_action` while this method is
        // still resolving the identity gate. Without a claim,
        // `decide_send_action` sees `stdin_tx: None` and
        // `spawning_in_progress: false` and becomes ITS OWN spawner —
        // two processes launched against the same `--resume <sid>`, one
        // silently overwriting the other's `current_pid`/`stdin_tx`/
        // `kill_tx` while the first stays alive on the same conversation.
        // Same exclusivity flag `decide_send_action`'s `BecomeSpawner`
        // branch claims for a live message — this is the same lock a
        // concurrent `send_message` acquires, so the check-and-set below is
        // atomic with respect to it.
        if !self.try_claim_eager_resume_spawn() {
            return EagerResumeOutcome::DeclinedTo(
                "already running or another spawn already in flight — nothing to eagerly resume",
            );
        }

        // `block_in_place` + `block_on`, not `.await` — `start()` is a sync
        // trait method (shared by every non-async controller type), but its
        // CALLER is not: `resync_controller` (this method's only path in)
        // is invoked synchronously and inline from inside
        // `Box::pin(async move { ... })` handler bodies (`websocket.rs`'s
        // `COMMAND_CONTROLLER_RESYNC`) and from `async fn open_agent_impl`
        // (`agent_open.rs`, both call sites). An earlier revision of this
        // comment argued sync-fn-therefore-safe-to-block and reagent P1 on
        // PR #3513 correctly rejected that: `build_persistent_spawn_env`
        // does a real synchronous-equivalent SQLite query plus, for
        // `SecretRef::Keychain` accounts, a blocking D-Bus/keyring read
        // (async only via `spawn_blocking`/`.await` internally) — run on a
        // bare `.await`-less call from here, that starves the tokio worker
        // it lands on for every unrelated task queued behind it, exactly
        // the failure class already named and fixed once in this codebase
        // (`sysinfo.rs`, `identity_auth_spawn.rs`, `websocket.rs`'s own
        // `COMMAND_CONTROLLER_INPUT` handler — see the latter's comment
        // citing incident #1782). Worse here than a one-off: this runs once
        // per persistent pane on `resync_controller`, i.e. potentially many
        // panes at once, on exactly the mass-reconnect-after-restart
        // scenario this whole PR exists to improve. `block_in_place` hands
        // this worker's other queued tasks off to the pool for the
        // duration; `Handle::current().block_on` then drives the async
        // function to completion on this now-isolated thread.
        let block_id = self.block_id.clone();
        let auth_key = self.auth_key.clone();
        let gate_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                crate::server::agent_handlers::input::build_persistent_spawn_env(
                    mstore,
                    id_store,
                    identity_store,
                    self.broker.clone(),
                    block_meta,
                    &block_id,
                    &auth_key,
                    base_env_vars,
                ),
            )
        });
        let env_vars = match gate_result {
            Ok(env) => env,
            Err(gate_err) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %gate_err,
                    "eager-resume declined: identity/credential spawn gate did not pass — \
                     falling back to lazy (spawns on next message, gated the same way then)"
                );
                // Release the claim taken above. Safe even if a message
                // queued during the gate's wait (`decide_send_action` sees
                // `spawning_in_progress: true` and routes to `Queued`
                // rather than becoming its own spawner concurrently with
                // us) — that entry is not stranded: whichever call next
                // successfully claims and spawns for this block drains the
                // WHOLE queue (`drain_queue_after_successful_spawn` doesn't
                // clear it first), so it rides along on the next attempt.
                self.inner.lock().unwrap().spawning_in_progress = false;
                return EagerResumeOutcome::DeclinedTo("identity/credential spawn gate did not pass");
            }
        };

        let config = PersistentSpawnConfig {
            cli_command,
            cli_args,
            working_dir,
            env_vars,
            session_id_field,
            resume_flag,
            session_id: session_id.to_string(),
            message_id: None,
        };
        let retry_config = config.clone();
        match self.spawn_process(config, None) {
            Ok(()) => {
                // `mark_turn_active_and_publish()` (the PUBLISH side of the
                // flag `spawn_process`'s own internal check already set the
                // RAW side of) only when something is actually about to be
                // delivered. Calling it unconditionally, the way every
                // other spawner does, would reintroduce the exact
                // "perpetually WORKING with nothing queued" bug — eager
                // resume's common case is spawning with an empty queue,
                // which no other spawner ever does (they all carry at
                // least their own triggering message).
                //
                // reagent P1 on PR #3523: that emptiness check and the
                // drain below used to be two separate lock acquisitions.
                // A `send_message` landing in the gap is routed to
                // `Queued` (the claim is still held), and `Queued`
                // deliberately does NOT publish turn-active — the contract
                // is that the spawn-claim holder does it. Neither does the
                // drain loop. So that message got delivered with
                // turn_active never published true: the pane, Swarm view
                // and subagent watcher all read idle while a turn was
                // genuinely in flight — the inverse of the bug the
                // conditional exists to prevent.
                //
                // Both are closed by deciding under ONE acquisition of the
                // same lock `decide_send_action` makes its own three-way
                // decision under, so a concurrent sender is strictly
                // before us (we observe its message, publish, and drain
                // it) or strictly after (it observes a released claim and
                // a live `stdin_tx`, takes `DeliverDirect`, and publishes
                // turn-active itself).
                let has_queued_work = {
                    let mut inner = self.inner.lock().unwrap();
                    if inner.pending_send_messages.is_empty() {
                        // Nothing to drain, so release the spawn claim
                        // right here instead of spawning a drain task
                        // whose only job would be to release it — this is
                        // exactly what `QueueDrainClaim::SpawnClaim::
                        // release` does, and what the drain's own
                        // empty-queue branch would have done one task
                        // hop later.
                        inner.spawning_in_progress = false;
                        false
                    } else {
                        true
                    }
                };
                if has_queued_work {
                    self.mark_turn_active_and_publish();
                    // Delivers what we observed plus anything that queues
                    // while it runs (the claim stays held for the drain's
                    // whole lifetime, and its own release path re-checks
                    // the queue under the lock before letting go) — same
                    // mechanism `respawn_once_for_leftover_queue` uses for
                    // the identical "spawned with nothing of our own, but
                    // the queue might not be empty" shape. Everything in
                    // that batch is covered by the single publish above.
                    // `true`, not `false` — codex P2 on PR #3523. The spawn
                    // can succeed at the OS level and the process still exit
                    // before this drain ever obtains `stdin_tx` (a stale
                    // `--resume` that dies immediately is exactly that
                    // shape, and is the case eager resume is most likely to
                    // hit). The drain's stalled branch then only releases
                    // the claim and publishes status unless fallback is
                    // allowed, stranding a prompt that `send_message`
                    // already reported as accepted and that is not yet in
                    // the resume retry batch either.
                    //
                    // `false` is for `respawn_once_for_leftover_queue`'s own
                    // recursive call, where it bounds the retry to one hop;
                    // every ordinary entry point passes `true` (see
                    // `release_spawn_claim_and_drain_queue`). This is an
                    // ordinary entry point and was simply the odd one out.
                    // Safe here because the fallback clears `session_id`
                    // before respawning, so it starts a fresh process rather
                    // than re-attempting the `--resume` that just died.
                    //
                    // Only reachable with work to do at all since the
                    // emptiness fix above: this drain now runs only when the
                    // queue is non-empty, which is precisely when a stall
                    // has something to strand.
                    self.drain_queue_after_successful_spawn(retry_config, true);
                }
                EagerResumeOutcome::Spawned
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %e,
                    "eager-resume declined: spawn failed — falling back to lazy"
                );
                // Mirrors `release_spawn_claim_and_drain_queue`'s
                // `!spawn_succeeded` branch: hand off to the fallback
                // respawn if something queued during the attempt (it needs
                // a live process to go to), otherwise just release the
                // claim — there's nothing to drain.
                //
                // Decided under ONE acquisition of the lock
                // `decide_send_action` also decides under — codex P2 on PR
                // #3523, the same defect already fixed on the success
                // branch above and missed here. As two acquisitions, a send
                // arriving in the gap saw the claim still held, queued its
                // prompt and returned success, and then this branch cleared
                // the claim without rechecking or draining — stranding an
                // accepted prompt until some unrelated later send happened
                // to become the next spawner.
                let has_leftovers = {
                    let mut inner = self.inner.lock().unwrap();
                    if inner.pending_send_messages.is_empty() {
                        inner.spawning_in_progress = false;
                        false
                    } else {
                        true
                    }
                };
                if has_leftovers {
                    self.respawn_once_for_leftover_queue(retry_config);
                }
                EagerResumeOutcome::DeclinedTo("spawn failed")
            }
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
            shellprocpid: None,
            shellprocname: String::new(),
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
    /// `Arc`s, not `&self`; worth factoring out if a second controller type
    /// needs the same heartbeat.
    ///
    /// A latent duplicate-loop race is an existing, already-accepted
    /// contract (reagent P2 on the PR that introduced this function): if a
    /// turn ends and a new one starts again within one
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
                        shellprocpid: None,
                        shellprocname: String::new(),
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

    /// Marks a turn active (re-arming the status heartbeat only if it was
    /// previously idle) and publishes the resulting status flip. Shared by
    /// `send_message` and `retry_after_resume_failure` — both represent "a
    /// user message is about to be delivered," just via different spawn
    /// paths. The heartbeat is re-armed conditionally: a mid-turn steering
    /// send already has one running, so re-spawning on every call would
    /// leak duplicate heartbeat tasks.
    fn mark_turn_active_and_publish(&self) {
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
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
    /// `skip_if_seq_queued`: `Some(seq)` marks this call a KNOWN
    /// re-delivery of the message originally enqueued under `seq` — skip
    /// enqueueing if that exact entry is still queued, and preserve the
    /// original seq (not a fresh one) if it must be re-queued, so the
    /// message keeps one identity for its whole lifetime (issue #2365:
    /// the old content-equality check here treated a genuinely
    /// different, identical-text message as "already queued" and
    /// silently dropped it).
    fn decide_send_action(&self, json_str: &str, skip_if_seq_queued: Option<u64>) -> SendAction {
        let mut inner = self.inner.lock().unwrap();
        // `!restart_pending`: a committed deferred restart leaves `stdin_tx`
        // live until the process actually exits, and writing into a process
        // about to be killed loses the message (codex P1 on PR #2858). Falling
        // through queues it for the replacement instead.
        if inner.stdin_tx.is_some()
            && !inner.spawning_in_progress
            && !inner.drain_claim
            && !inner.restart_pending
        {
            SendAction::DeliverDirect
        } else if inner.spawning_in_progress || inner.drain_claim {
            let already_queued = skip_if_seq_queued
                .is_some_and(|seq| inner.pending_send_messages.iter().any(|m| m.seq == seq));
            if !already_queued {
                let seq = skip_if_seq_queued.unwrap_or_else(|| inner.take_next_message_seq());
                inner
                    .pending_send_messages
                    .push_back(QueuedMessage::fresh(seq, json_str.to_string()));
            }
            SendAction::Queued
        } else {
            inner.spawning_in_progress = true;
            let own_seq = skip_if_seq_queued.unwrap_or_else(|| inner.take_next_message_seq());
            inner
                .pending_send_messages
                .push_back(QueuedMessage::fresh(own_seq, json_str.to_string()));
            SendAction::BecomeSpawner { own_seq }
        }
    }

    /// Releases the exclusive spawn claim taken by `decide_send_action`
    /// returning `BecomeSpawner`. `spawn_succeeded` distinguishes two very
    /// different situations:
    ///
    /// - **Failed spawn**: discards only the entry with `own_seq` — the
    ///   specific message THIS spawner pushed when it claimed
    ///   `BecomeSpawner` (`SendAction::BecomeSpawner::own_seq`), found by
    ///   queue identity rather than assumed to be at the front. codex P2
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
    ///   fallback respawn. Originally fixed by matching content instead
    ///   of position, which still left two GENUINELY DIFFERENT messages
    ///   sharing identical text (e.g. two "yes" replies) ambiguous; seq
    ///   matching (issue #2365) closes that residue too. codex P1 on PR
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
    fn release_spawn_claim_and_drain_queue(&self, spawn_succeeded: bool, retry_config: PersistentSpawnConfig, own_seq: u64) {
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
                if let Some(idx) = inner.pending_send_messages.iter().position(|m| m.seq == own_seq) {
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
        self.drain_queue_with_claim(retry_config, allow_fallback_respawn, QueueDrainClaim::SpawnClaim);
    }

    /// The queue-drain loop itself, parameterized by which exclusivity
    /// claim it runs under ([`QueueDrainClaim`]) — the post-spawn drain
    /// (`SpawnClaim`) and issue #2367's retry-batch flush (`RetryFlush`)
    /// share every delivery invariant (Sender::send backpressure,
    /// seed-aware retry-batch appends, delivery-order persistence,
    /// stall handling); the ONLY difference is which flag they release.
    /// `RetryFlush` callers always pass `allow_fallback_respawn: false`:
    /// the flush targets an already-running process, so a stall means
    /// that process died — leftovers stay queued for the next spawn and
    /// the stalled branch publishes a status update.
    fn drain_queue_with_claim(
        &self,
        retry_config: PersistentSpawnConfig,
        allow_fallback_respawn: bool,
        claim: QueueDrainClaim,
    ) {
        debug_assert!(
            !(claim == QueueDrainClaim::RetryFlush && allow_fallback_respawn),
            "a retry flush never respawns — see this function's doc comment"
        );
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
            // would then redeliver that one message TWICE. Originally
            // fixed by matching content (with `seed_already_matched` as a
            // one-shot so an identical-text later delivery wasn't ALSO
            // skipped); now matched by queue identity
            // (`QueuedMessage::seq`, issue #2365), which identifies the
            // seeded entry exactly regardless of position or duplicate
            // text. The one-shot flag is kept as cheap defense-in-depth —
            // seqs are never reused, so it can no longer fire twice.
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
                    // Re-check the queue under THIS lock before releasing:
                    // the pop above observing "empty" and this release are
                    // two separate acquisitions, and a concurrent caller
                    // routed to `Queued` (claim still held) can enqueue in
                    // that gap. With a live tx, releasing now would strand
                    // that message — no claim holder ever drains it, and
                    // every future send takes `DeliverDirect` straight
                    // past it. Loop back and drain it instead (issue
                    // #2367's queue-authority guarantee; the same gap
                    // existed for the spawn-claim drain).
                    if inner.stdin_tx.is_some() && !inner.pending_send_messages.is_empty() {
                        drop(inner);
                        continue;
                    }
                    let stalled = inner.stdin_tx.is_none() && !inner.pending_send_messages.is_empty();
                    if !(stalled && allow_fallback_respawn) {
                        claim.release(&mut inner);
                    }
                    break stalled;
                };
                let QueuedMessage { seq, json_str, already_persisted } = queued;
                let delivered_copy = json_str.clone();
                let is_the_seed = !seed_already_matched && {
                    let inner = inner_arc.lock().unwrap();
                    inner.resume.is_seeded_message(inner.spawn_generation, seq)
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
                    inner.pending_send_messages.push_front(QueuedMessage { seq, json_str: e.0, already_persisted });
                    let stalled = !inner.pending_send_messages.is_empty();
                    if !(stalled && allow_fallback_respawn) {
                        claim.release(&mut inner);
                    }
                    break stalled;
                }
                // codex P1 on PR #2360 (sixth review pass, round 5): track
                // every message actually handed to this process's stdin
                // channel beyond the first — not just the one that
                // triggered the spawn — so a confirmed stale-resume retry
                // redelivers all of them (see `RetryPayload`'s own doc
                // comment), since channel acceptance is not proof the CLI
                // ever read it. `MessageAppendedToRetryBatch` is applied
                // whether the state is still `AwaitingOutcome` or has
                // already been promoted to `ConfirmedRetry` — codex P1 on
                // PR #2360 (sixth review pass, round 6): `poison_resume`
                // (the stderr-reader task, running concurrently) can
                // promote at any point, and `persistent_resume::update`
                // handles the append identically either way (see its own
                // `MessageAppendedToRetryBatch` match arms), so there's no
                // window where a message delivered right after that
                // promotion is silently dropped.
                if !is_the_seed {
                    // reagentx P1 on PR #2373: reading `spawn_generation`
                    // and applying the event were two SEPARATE lock
                    // acquisitions — a concurrent respawn in between would
                    // bump `spawn_generation`, making this event carry a
                    // now-stale generation that `update()`'s catch-all
                    // silently ignores, losing the message from the retry
                    // batch. One lock acquisition closes the gap.
                    let mut inner = inner_arc.lock().unwrap();
                    let generation = inner.spawn_generation;
                    inner.apply_resume_event(persistent_resume::ResumeEvent::MessageAppendedToRetryBatch {
                        generation,
                        entry: persistent_resume::QueuedRetryEntry { seq, json: delivered_copy.clone() },
                    });
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
                        claim.release(&mut inner_arc.lock().unwrap());
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
    /// Atomically clears the ambient session id AND reserves a fresh
    /// spawn generation, so every reader task belonging to any EXISTING
    /// generation is stale from this point on. Used by both fresh-start
    /// respawn paths (`respawn_once_for_leftover_queue`,
    /// `retry_after_resume_failure`'s `BecomeSpawner` arm) in place of a
    /// bare `session_id = None`.
    ///
    /// codex P1 on PR #2500 (second round): clearing alone left a window
    /// — until `spawn_process`'s own generation bump, which happens AFTER
    /// the `--resume` decision reads `session_id` and after the process
    /// is spawned — where the dying generation still equaled
    /// `spawn_generation`, so its stdout reader's stale-sid echo passed
    /// `try_capture_session_id`'s currency gate (issue #2366) and was
    /// re-adopted: the supposedly fresh spawn could reattach
    /// `--resume <stale-sid>` with no retry payload armed to catch the
    /// repeat failure. Reserving the next generation in the same lock
    /// acquisition as the clear closes the whole window.
    ///
    /// The reserved generation is never itself spawned (`spawn_process`
    /// bumps again) — see `spawn_generation`'s doc comment for why the
    /// gap is inert. A `stop_process` racing into the reserve window
    /// records a `StopRequested` for the never-spawned generation, which
    /// the resume state machine ignores; both callers only reach this
    /// point after the prior generation's tracking has already resolved,
    /// so no stop-intent is lost that wasn't equally lost before.
    fn clear_session_id_for_fresh_spawn(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.spawn_generation += 1;
        inner.session_id = None;
    }

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
        // meant to close — a still-racing reader task from the doomed
        // process re-capturing the same sid right after a plain clear —
        // is now closed WITHOUT poisoning: the clear also reserves a
        // fresh spawn generation, making every existing generation's
        // capture stale to `try_capture_session_id`'s currency gate (see
        // `clear_session_id_for_fresh_spawn`'s own doc comment).
        self.clear_session_id_for_fresh_spawn();
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
        let event = crate::backend::mps::MuxEvent {
            event: crate::backend::mps::EVENT_AGENT_MESSAGE_ACCEPTED.to_string(),
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
    /// the next pane open. No MPS event is published here — the
    /// live-display is handled by the `agent-message-accepted` path (UUID
    /// node), avoiding a duplicate.
    fn persist_message_to_blockfile(&self, json_str: &str) {
        let global_zone = super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
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

        match self.decide_send_action(&json_str, None) {
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
                let mut inner = self.inner.lock().unwrap();
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
                // codex P1 on PR #3513: track this message into the CURRENT
                // generation's retry batch, same as the drain loop already
                // does for queued deliveries (see that call site's own doc
                // comment on why every message beyond the seed needs this,
                // not just the one that triggered the spawn). `DeliverDirect`
                // itself never did this — harmless for a spawn that never
                // attempted `--resume` (`MessageAppendedToRetryBatch` is a
                // no-op on `NotTracking`, see `persistent_resume::update`'s
                // catch-all) or one whose resume already confirmed success,
                // but a real gap for a still-unconfirmed one: eager resume
                // (issue #3463) can reach `DeliverDirect` with its `--resume`
                // attempt not yet confirmed and NO seed message already
                // tracked (unlike every other resume-respawn, which always
                // has one) — this is what actually closes that gap, not just
                // the `SpawnedWithResume` routing fix above. Read the
                // generation and apply in this SAME lock acquisition
                // (already held for `try_send`) — reagentx P1 on PR #2373
                // via the drain loop's identical concern: a separate
                // acquisition risks a concurrent respawn bumping
                // `spawn_generation` in between, making this event carry a
                // stale generation that `update()`'s catch-all silently
                // ignores.
                let generation = inner.spawn_generation;
                let seq = inner.take_next_message_seq();
                inner.apply_resume_event(persistent_resume::ResumeEvent::MessageAppendedToRetryBatch {
                    generation,
                    entry: persistent_resume::QueuedRetryEntry { seq, json: json_str.clone() },
                });
                drop(inner);
                self.persist_message_to_blockfile(&json_str);
                self.emit_message_accepted(config.message_id.as_deref());
                Ok(())
            }
            SendAction::BecomeSpawner { own_seq } => {
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
                let spawn_result = self.spawn_process(
                    config,
                    Some(persistent_resume::QueuedRetryEntry { seq: own_seq, json: json_str }),
                );
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
                self.release_spawn_claim_and_drain_queue(spawn_result.is_ok(), retry_config, own_seq);
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
    /// codex P2 on PR #2371: a held-back error line must reach the user
    /// if a confirmed retry turns out NOT to actually launch (this
    /// controller already being torn down, or the fresh `spawn_process`
    /// call itself failing) — otherwise an already-accepted prompt ends
    /// in total silence: neither the original error nor a replacement
    /// one. `held_error_line` is dropped only when delivery is CONFIRMED
    /// (a successful `BecomeSpawner` spawn, or every message in a
    /// `DeliverDirect` batch landing via `try_send`) — every OTHER path
    /// (`Queued`, or `DeliverDirect`'s own `any_failed` fallback via
    /// `drain_queue_after_successful_spawn`) hands off to a background
    /// drain whose own `stalled_with_leftovers` branch already publishes
    /// a status update on genuine total failure, so those paths drop the
    /// line instead — see reagentx P1 (round 2 on PR #2371) on the
    /// `Queued` arm below for why flushing eagerly there would reproduce
    /// this PR's own bug via a different path.
    fn flush_error_line_now(&self, line: String) {
        let Some(ref broker) = self.broker else { return };
        let global_output_zone = super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        super::shell::handle_append_block_file(
            broker,
            &self.block_id,
            PERSISTENT_OUTPUT_SUBJECT,
            line.as_bytes(),
            self.filestore.as_ref(),
            global_output_zone.as_deref(),
        );
        // Same gap reagentx flagged (PR #2421 P2) at the other FlushErrorLine
        // call sites: this is also a previously held-back turn's error only
        // now confirmed final (a fresh spawn superseding a still-tracking
        // generation, or a retry batch exhausted with nothing left to
        // deliver) — give it the same classify/persist/publish treatment so
        // it isn't silently dropped from the pane's failure-recovery UI. No
        // exit code exists for this now-superseded turn.
        if let Some(failure) = classify_exit_line(None, &line) {
            core::persist_last_failure(&self.block_id, Some(&failure), &self.mstore, &self.event_bus);
            broker.publish(mps::MuxEvent {
                event: mps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&failure).ok(),
            });
        }
    }

    /// Publish a session-outcome line immediately, via the same append
    /// path `session_outcome_line`'s other call sites use
    /// (`persistent_resume::ResumeEffect::EmitSessionOutcome`'s normal
    /// handling). Used by `retry_after_resume_failure` for the ONE case
    /// it can decide unambiguously and immediately: no recovery candidate
    /// exists, so this is genuinely, unconditionally a fresh conversation
    /// — see that function's own doc comment for why the Resumed case is
    /// deliberately NOT emitted here.
    fn emit_session_outcome_now(
        &self,
        outcome: persistent_resume::SessionOutcome,
        attempted_sid: String,
        actual_sid: Option<String>,
    ) {
        let Some(ref broker) = self.broker else { return };
        let line = session_outcome_line(outcome, attempted_sid, actual_sid);
        let global_output_zone = super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        super::shell::handle_append_block_file(
            broker,
            &self.block_id,
            PERSISTENT_OUTPUT_SUBJECT,
            line.as_bytes(),
            self.filestore.as_ref(),
            global_output_zone.as_deref(),
        );
    }

    /// Does a transcript already exist for this pane — either in this
    /// channel's own blockfile, or in the agent's GLOBAL transcript zone?
    ///
    /// The second half is the one that matters for
    /// [`fresh_start_needs_disclosure`]: on a cross-channel or cross-version
    /// open this channel's blockfile is empty, but the pane still renders the
    /// full prior conversation through `app_api::global_output_source`'s read
    /// fallback. Mirrors that function's checks (agent-anchored, not archived,
    /// non-empty `output`) so "the pane will show history" and "we disclose a
    /// fresh start" can't disagree.
    ///
    /// Two `stat` calls at most, on the spawn path only — cheap enough to run
    /// unconditionally behind the caller's own generation gate.
    fn has_prior_transcript(&self) -> bool {
        if let Some(ref fs) = self.filestore {
            if matches!(fs.stat(&self.block_id, PERSISTENT_OUTPUT_SUBJECT), Ok(Some(ref f)) if f.size > 0)
            {
                return true;
            }
        }
        let Some(ref store) = self.mstore else {
            return false;
        };
        let Ok(block) = store.must_get::<crate::backend::obj::Block>(&self.block_id) else {
            return false;
        };
        let archived = block
            .meta
            .get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT)
            .and_then(|v| v.as_i64())
            .map(|v| v > 0)
            .unwrap_or(false);
        if archived {
            return false;
        }
        let Some(zone) = crate::backend::agent_session::agent_zone_for_block_meta(&block.meta) else {
            return false;
        };
        let Some(gfs) = crate::backend::agent_session::global_transcript_store() else {
            return false;
        };
        matches!(
            gfs.stat(&zone, crate::backend::agent_session::OUTPUT_FILE),
            Ok(Some(ref f)) if f.size > 0
        )
    }

    /// After a confirmed-stale `--resume` failure, try to recover a REAL
    /// session instead of giving up and starting blank
    /// (`docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`).
    /// Looks for the largest on-disk session under this spawn's own
    /// `CLAUDE_CONFIG_DIR`/working dir — the same "largest session wins"
    /// recovery `session_backfill::backfill_session_ids` already trusts
    /// for the analogous cross-channel-open case.
    ///
    /// Returns `None` (caller falls back to starting blank, same as
    /// before this existed) when: `CLAUDE_CONFIG_DIR` isn't set on this
    /// config, no session exists there, or the only candidate found is
    /// the SAME id already confirmed poisoned — guards against looping
    /// forever "recovering" an id that is itself dead (nothing on disk
    /// changes between attempts, so an unguarded retry would rediscover
    /// the identical bad id every time).
    fn find_recovery_session_id(&self, config: &PersistentSpawnConfig) -> Option<String> {
        let config_dir = config.env_vars.get("CLAUDE_CONFIG_DIR")?;
        let candidate = crate::backend::session_backfill::find_largest_session_for_working_dir(
            config_dir,
            &config.working_dir,
        )?;
        let already_poisoned = self.inner.lock().unwrap().resume_poisoned.as_deref() == Some(candidate.as_str());
        if already_poisoned {
            return None;
        }
        Some(candidate)
    }

    /// `retry_generation` is the spawn generation whose `ProcessExited`
    /// fired this retry (the process-waiter's own `my_generation_wait`) —
    /// `decide_retry_batch_action` needs it to tell a live NEWER spawn
    /// apart from the impossible "our own process is somehow still
    /// running" case (issue #2367). `attempted_sid` is the id that was
    /// just confirmed unreachable — threaded through from
    /// `persistent_resume::ResumeEffect::FireRetry` so this function can
    /// emit the eventual `EmitSessionOutcome` itself once recovery is
    /// known (reagentx + Codex on PR #2693 — `persistent_resume.rs` can
    /// no longer decide Fresh-vs-Resumed at the point it fires this
    /// retry, since recovery might succeed).
    fn retry_after_resume_failure(
        &self,
        retry_generation: u64,
        mut config: PersistentSpawnConfig,
        mut entries: Vec<persistent_resume::QueuedRetryEntry>,
        held_error_line: Option<String>,
        attempted_sid: String,
    ) {
        // "Reconnecting…" starts here regardless of which branch below is
        // taken — the user-visible gap begins the moment a retry is known
        // to be needed, not once recovery search finishes. See §6.2.
        publish_resume_retry_status(&self.broker, &self.block_id, "retrying");
        let recovered = self.find_recovery_session_id(&config);
        config.session_id = recovered.clone().unwrap_or_default();
        match &recovered {
            // A real on-disk session was found — don't claim "Resumed"
            // yet (codex P1 on PR #2693: the CLI can still reject this
            // recovered id too, e.g. a corrupt file or a non-top-level
            // session). Left unemitted here on purpose: the spawn below
            // now threads this attempt through the SAME resume-tracking
            // machinery a genuine first-time `--resume` uses (`Some(first
            // .clone())`, not `None`), so `persistent_resume.rs`'s
            // already-correct, already-tested `SessionCaptured` handling
            // emits `Resumed` once the CLI actually confirms it — or, if
            // this recovered id is ALSO stale, cascades into another
            // `ConfirmedRetry` → this same function again, where
            // `find_recovery_session_id`'s poison guard refuses to
            // re-recover the identical dead id and this arm's `None`
            // branch below correctly emits `Fresh` instead. "Reconnecting…"
            // is deliberately NOT resolved here — the eventual
            // `EmitSessionOutcome` handling (stdout-reader / process-exit
            // match arms) is what clears it, once the CLI actually confirms
            // this recovered id one way or the other.
            Some(_) => {}
            // No recovery candidate — this IS genuinely, unambiguously a
            // fresh conversation, decided right now. Safe to emit
            // immediately: nothing downstream can turn this back into a
            // resume. Also resolves "Reconnecting…" immediately, in step
            // with the outcome itself.
            None => {
                publish_resume_retry_status(&self.broker, &self.block_id, "resolved");
                self.emit_session_outcome_now(
                    persistent_resume::SessionOutcome::Fresh,
                    attempted_sid,
                    None,
                )
            }
        }
        let Some(first) = (!entries.is_empty()).then(|| entries.remove(0)) else {
            // Nothing to retry at all. This USED to be unreachable — "the
            // batch always has at least the triggering message" — and the
            // comment here said so. Eager resume (issue #3463) made it
            // reachable: it starts `AwaitingOutcome` tracking with an
            // deliberately EMPTY batch, because it revives a session with
            // nothing queued rather than in response to a message. If that
            // `--resume` turns out to be stale before any prompt arrives,
            // we land here with no entries.
            //
            // codex P2 on PR #3523: this path must still RESOLVE the
            // "Reconnecting…" state published at the top of this function.
            // Returning without it left the pane showing "Reconnecting…"
            // forever — there is no spawn below to eventually emit an
            // outcome, precisely because there is no first entry to trigger
            // one.
            //
            // ONLY when a recovery candidate was found. The no-recovery arm
            // above already published "resolved" synchronously, and
            // publishing a second one here is not harmless: the event is
            // `persist: 2`, so the duplicate evicts "retrying" from history
            // and a pane subscribing just afterwards sees no record that a
            // retry ever happened. The recovery-found arm is the one that
            // deliberately DEFERS "resolved" to whichever `EmitSessionOutcome`
            // site confirms the recovered id — a deferral that never
            // completes when there is no spawn to confirm it. (Caught by
            // this fix's own test, which failed with
            // `[resolved, resolved]` when the publish was unconditional.)
            if recovered.is_some() {
                // The recovery candidate found above is deliberately NOT
                // persisted here. An earlier cut of this branch wrote it
                // back so the next prompt would resume it
                // (codex P2 on PR #3523 — otherwise the recovery scan's
                // result is simply discarded and the next prompt starts a
                // fresh conversation). That write then produced two P1s in
                // consecutive rounds, both the same shape: nothing here can
                // establish that this retry still owns the block's session
                // metadata by the time it writes.
                //
                // A `spawn_generation` check fixes only the intra-controller
                // case. `spawn_generation` is controller-LOCAL and
                // `stop_for_replace` does not advance it, so a forced resync
                // that REPLACES this controller leaves the check passing
                // while the replacement is already capturing and persisting
                // its own live session — and this write lands on top of it.
                // The prompt then runs in one conversation while the next
                // restart resumes another.
                //
                // That is the same "an evicted controller cannot tell it was
                // evicted" problem that got the registry replace check and
                // the launch-specific instance gate dropped from this PR;
                // see issue #3525. Trading a discarded recovery (P2) for
                // corrupted session metadata (P1) is the wrong direction, so
                // the recovery is dropped here until that invalidation
                // actually exists.
                //
                // Resolving the status is still correct and carries no such
                // risk — it writes no shared state, and the "retrying" ping
                // was published unconditionally at the top of this call, so
                // resolving it is this call's own bookkeeping. Skipping it is
                // what strands the pane on "Reconnecting…".
                publish_resume_retry_status(&self.broker, &self.block_id, "resolved");
            }
            // A no-op, not a launch, so any held error line must still
            // reach the user.
            if let Some(line) = held_error_line {
                self.flush_error_line_now(line);
            }
            return;
        };
        let rest = entries;

        match self.decide_retry_batch_action(retry_generation, &first, &rest) {
            RetryBatchAction::FlushClaimed => {
                // A live, NEWER-generation process was already running by
                // the time this retry got scheduled (issue #2367 — the
                // retry's own process exited; a live `stdin_tx` is by
                // definition an unrelated spawn that raced ahead). The
                // batch was prepended to the queue and `drain_claim` was
                // taken in the SAME lock acquisition that decided this
                // arm, so every concurrent `decide_send_action` caller
                // already routes to `Queued` behind it — the queue is the
                // single ordering authority. Flush through the same drain
                // loop a successful spawn uses (`Sender::send`
                // backpressure, seed-aware retry-batch appends,
                // delivery-order persistence); it releases `drain_claim`
                // once the queue runs dry. This subsumes the previous
                // `try_send`-then-fall-back-to-queue design: everything
                // goes through the queue up front, so the batch can never
                // reorder relative to itself or to racing sends, and the
                // spawn flag is no longer borrowed for a non-spawn (the
                // round-7/round-8 fallback machinery this replaces).
                self.mark_turn_active_and_publish();
                // `allow_fallback_respawn: false` — the flush targets an
                // already-running process; a stall means that process
                // died, and its own exit handling (or the next send's
                // spawn) picks the leftovers up.
                self.drain_queue_with_claim(config.clone(), false, QueueDrainClaim::RetryFlush);
                // reagentx P1 (round 2 on PR #2371): eventual delivery is
                // the overwhelmingly common outcome — flushing the held
                // line eagerly here would show a stale, wrong error
                // bubble immediately followed by the real (successful)
                // response. On the rare total-failure path the drain's
                // own `stalled_with_leftovers` branch (with
                // `allow_fallback_respawn: false`) already calls
                // `publish_status()` — never silent forever, even without
                // the specific original error text.
                drop(held_error_line);
            }
            RetryBatchAction::Queued => {
                // Someone else is already spawning — their own
                // `release_spawn_claim_and_drain_queue` will deliver this
                // (or, if their process turns out to already be dead, its
                // own bounded fallback respawn will).
                //
                // reagentx P1 on PR #2371 (round 2): flushing eagerly HERE
                // (an earlier cut of this fix did, reasoning that losing
                // it silently on eventual failure was worse) contradicts
                // this codebase's own established pattern for a `Queued`
                // outcome — `decide_send_action`'s doc comment: side
                // effects for a queued item happen "later, inside the
                // drain, at the exact moment this message is actually
                // delivered," not eagerly at enqueue time. Eventual
                // success is the OVERWHELMINGLY common outcome for a
                // queued message (that's the whole point of the
                // queue/drain/fallback-respawn infrastructure below), so
                // flushing eagerly would show a stale, wrong error bubble
                // in the common case, immediately followed by the real
                // (successful) response — reproducing the exact bug this
                // PR exists to fix, just via a different path. Dropped
                // instead: on the rare total-failure path (the fallback
                // respawn ALSO fails), `release_spawn_claim_and_drain_queue`'s
                // own stalled-fallback branch already publishes a status
                // update and keeps the messages queued for a future spawn
                // attempt — never silent forever, even without the
                // specific original error text.
                drop(held_error_line);
            }
            RetryBatchAction::BecomeSpawner { own_seq } => {
                // Only clear inner.session_id now that THIS retry is
                // actually about to spawn — codex P2 on PR #2360 (sixth
                // review pass, round 3): clearing it unconditionally up
                // front could erase a session id a DIFFERENT,
                // concurrently-installed process had already legitimately
                // captured, if this retry instead resolved via
                // `DeliverDirect` or `Queued` above — breaking in-memory
                // session tracking and turn-end subagent reconciliation
                // for that process's remaining lifetime.
                // Clear + reserve a fresh generation in one lock
                // acquisition (same race as the leftover-queue fallback —
                // see `clear_session_id_for_fresh_spawn`).
                self.clear_session_id_for_fresh_spawn();
                let retry_config = config.clone();
                // `Some(first.clone())`, not `None` (codex P1 on PR
                // #2693): when `config.session_id` holds a recovered
                // on-disk session (not empty), this MUST thread through
                // as a real resume-tracking seed — same as any first-time
                // `--resume` attempt (mirrors `SendAction::BecomeSpawner`'s
                // own `spawn_process` call above) — or a recovered id
                // that turns out to be ALSO stale/rejected has no
                // detection/retry-cascade at all, stranding the batch on
                // that attempt's raw error instead of eventually falling
                // through to the promised blank-conversation fallback.
                // Harmless when there's nothing to resume: `spawn_process`
                // only constructs `SpawnedWithResume` tracking when
                // `inner.session_id` actually ends up populated, which it
                // won't if `config.session_id` is empty — this always
                // correctly falls back to `SpawnedFresh` in that case,
                // same as passing `None` used to.
                let spawn_result = self.spawn_process(config, Some(first.clone()));
                match &spawn_result {
                    Ok(_) => self.mark_turn_active_and_publish(),
                    Err(e) => tracing::error!(
                        block_id = %self.block_id,
                        error = %e,
                        "failed to respawn after a stale --resume session id"
                    ),
                }
                self.release_spawn_claim_and_drain_queue(spawn_result.is_ok(), retry_config, own_seq);
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
                    // codex P2 on PR #2371: the retry never actually
                    // launched — flush any held error line now instead
                    // of silently dropping it, so the user gets at least
                    // one explanation (the original error) instead of
                    // total silence.
                    if let Some(line) = held_error_line {
                        self.flush_error_line_now(line);
                    }
                }
            }
        }
    }

    /// Same shape of decision as `decide_send_action`, but atomically
    /// enqueues the ENTIRE batch (not just one message) in every arm —
    /// used only by `retry_after_resume_failure`. codex P2 on PR #2360
    /// (sixth review pass, round 6): deciding and enqueueing a
    /// multi-message retry batch one call at a time (each through its own
    /// `decide_send_action` call) left a window between them where a
    /// genuinely new, unrelated message could interleave into the MIDDLE
    /// of the same original batch, reordering it relative to how the
    /// doomed process actually received it. Batch prepend/dedup rules
    /// live in [`Self::prepend_retry_batch`].
    ///
    /// Issue #2367 (spec §4, option 2): the live-process outcome is no
    /// longer a caller-side `try_send` (`DeliverDirect`) — that let a
    /// concurrent send race ahead of, or into the middle of, the batch.
    /// It is now [`RetryBatchAction::FlushClaimed`]: batch prepended and
    /// `drain_claim` taken under this one lock, then flushed through the
    /// shared queue drain.
    fn decide_retry_batch_action(
        &self,
        retry_generation: u64,
        first: &persistent_resume::QueuedRetryEntry,
        rest: &[persistent_resume::QueuedRetryEntry],
    ) -> RetryBatchAction {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() && !inner.spawning_in_progress && !inner.drain_claim {
            // The retry's own process exited — that exit is what fired
            // this retry, and the exit-handler clears `stdin_tx` under
            // the same `is_current_generation` gate — so a live
            // `stdin_tx` here is by definition a NEWER, unrelated spawn
            // that raced ahead during the exit-handler's cleanup window
            // (issue #2367).
            debug_assert!(
                inner.spawn_generation != retry_generation,
                "a retry's own generation cannot still be current while stdin_tx is live"
            );
            // Prepend the batch and take the drain claim in THIS SAME
            // lock acquisition (spec §4 option 2): from this instant
            // every `decide_send_action` caller routes to `Queued`
            // behind the batch, so the queue — not a caller's own
            // `try_send` — is the single ordering authority. The only
            // residue is a `DeliverDirect` *decided* before this lock
            // was taken that lands mid-batch: that send was already
            // racing the process exit itself, and no claim scheme can
            // sequence it.
            inner.drain_claim = true;
            Self::prepend_retry_batch(&mut inner, first, rest);
            RetryBatchAction::FlushClaimed
        } else if inner.spawning_in_progress || inner.drain_claim {
            Self::prepend_retry_batch(&mut inner, first, rest);
            RetryBatchAction::Queued
        } else {
            inner.spawning_in_progress = true;
            inner
                .pending_send_messages
                .push_back(QueuedMessage::already_persisted(first.seq, first.json.clone()));
            for entry in rest {
                inner
                    .pending_send_messages
                    .push_back(QueuedMessage::already_persisted(entry.seq, entry.json.clone()));
            }
            RetryBatchAction::BecomeSpawner { own_seq: first.seq }
        }
    }

    /// Prepends a retry batch to `pending_send_messages`, deduping only
    /// `first` by seq. codex P2 on PR #2360 (sixth review pass, round
    /// 11): prepend rather than append — this batch represents messages
    /// accepted by the doomed process BEFORE whatever's currently
    /// queued (anything queued arrived after the original spawn claimed
    /// `spawning_in_progress`, so it's chronologically later); appending
    /// would let the fresh process receive later-arriving input first.
    ///
    /// `first` may itself STILL be sitting in the queue (see
    /// `decide_send_action`'s doc comment on `skip_if_seq_queued`) —
    /// always at index 0 if so, since it's the ONLY thing ever present
    /// when a claim starts and nothing but `push_back` ever touches
    /// this queue elsewhere. In that case `rest` belongs immediately
    /// after it (same batch, preserving order), not ahead of it. `rest`
    /// is always pushed as-is: dedup must never apply WITHIN the same
    /// batch (two entries can legitimately carry identical text and
    /// both need redelivering).
    fn prepend_retry_batch(
        inner: &mut PersistentInner,
        first: &persistent_resume::QueuedRetryEntry,
        rest: &[persistent_resume::QueuedRetryEntry],
    ) {
        let first_already_queued = inner.pending_send_messages.iter().any(|m| m.seq == first.seq);
        if first_already_queued {
            for (i, entry) in rest.iter().enumerate() {
                inner
                    .pending_send_messages
                    .insert(i + 1, QueuedMessage::already_persisted(entry.seq, entry.json.clone()));
            }
        } else {
            let mut front: VecDeque<QueuedMessage> = VecDeque::new();
            front.push_back(QueuedMessage::already_persisted(first.seq, first.json.clone()));
            for entry in rest {
                front.push_back(QueuedMessage::already_persisted(entry.seq, entry.json.clone()));
            }
            front.append(&mut inner.pending_send_messages);
            inner.pending_send_messages = front;
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
        // including the heartbeat re-arm-only-if-was-idle rationale and why
        // this must be the atomic read-and-set (send_message and
        // send_user_message can race on the same block).
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
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
            let mut inner = self.inner.lock().unwrap();
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
            // codex P1 on PR #3523: track this into the CURRENT generation's
            // retry batch, exactly as `send_message`'s `DeliverDirect` branch
            // does. That fix (commit 849434d7f in this PR) covered the
            // typed-in-the-UI path and missed this one — the MuxBus/reactive
            // injection path (`deliver_agent_message` -> here).
            //
            // The gap matters most for the case this PR added: eager resume
            // spawns with an unconfirmed `--resume` and NO seed message, so
            // the retry batch starts empty. An injected prompt written
            // straight to stdin and never appended leaves that batch empty,
            // so when the resumed sid turns out to be stale the retry fires
            // with nothing to redeliver — and the prompt is gone, even though
            // the caller was told `AgentDelivery::Structured` and the
            // transcript already rendered it to the operator. Silent loss of
            // a message we acknowledged.
            //
            // Read the generation and apply in this SAME lock acquisition
            // (already held for `try_send`) — a separate one risks a
            // concurrent respawn bumping `spawn_generation` in between,
            // making the event carry a stale generation that `update()`'s
            // catch-all silently ignores. Same reasoning as the
            // `DeliverDirect` call site.
            //
            // A no-op when there's no unconfirmed resume in flight
            // (`MessageAppendedToRetryBatch` does nothing on `NotTracking`),
            // so this costs nothing on the ordinary path.
            let generation = inner.spawn_generation;
            let seq = inner.take_next_message_seq();
            inner.apply_resume_event(persistent_resume::ResumeEvent::MessageAppendedToRetryBatch {
                generation,
                entry: persistent_resume::QueuedRetryEntry { seq, json: json_str.clone() },
            });
        }

        // Persist the injected message to the blockfile WITH a live event —
        // unlike `send_message`, there is no `agent-message-accepted` pending
        // echo to pair with (nothing was typed in the UI), so without this the
        // injection is invisible to the human operator: a silent injection,
        // which SPEC_JEKT_SECURITY_AND_VISIBILITY §3.1/G1 forbids. The live
        // blockfile append renders it in the open pane; the persisted line lets
        // `parseHistoryLines` rebuild the node on reopen.
        if let Some(ref broker) = self.broker {
            let global_zone = super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
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

    /// Decline a parked AskUserQuestion via the Agent SDK **control protocol**
    /// — the Cancel button / Escape in `AgentQuestionPanel.tsx`. Structurally a
    /// mirror of `answer_question` (same `pending_questions` lookup/removal,
    /// same dead-air safety net — the CLI can abandon a pending tool_use whose
    /// turn already ended regardless of whether the response was an allow or a
    /// deny), but sends `behavior: "deny"` with a `message` instead of
    /// `behavior: "allow"` with `updatedInput`. This is a general, documented
    /// Agent SDK mechanism (`PermissionResult` deny case for the `canUseTool`
    /// callback) — AskUserQuestion goes through the exact same callback as
    /// ordinary tool permission requests, confirmed against the official Agent
    /// SDK docs (code.claude.com/docs/en/agent-sdk/user-input). Spec:
    /// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    ///
    /// See `answer_question`'s doc comment for the `pending_questions`
    /// in-memory-only caveat and the exact error-prefix stability requirement
    /// (`useAgentQuestions.ts`'s `SAFE_TO_RETRY_VIA_FOLLOWUP` allowlist matches
    /// on this method's error text too, since it shares the identical lookup).
    pub fn deny_question(&self, tool_use_id: String, message: String) -> Result<(), String> {
        let (request_id, _questions, tx) = {
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
                .ok_or("persistent process not running (cannot deliver decline)")?
                .clone();
            (rid, qs, tx)
        };

        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "deny",
                    "message": message,
                    "toolUseID": tool_use_id,
                }
            }
        });
        // Snapshot stdout activity BEFORE sending, same reasoning as
        // answer_question (codex review on #1536).
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net — identical mechanism to answer_question's, using
        // the decline-flavored resume message. Reuses ANSWER_RESUME_FALLBACK_MS:
        // same failure mode (turn already ended before the response arrived), no
        // reason for a different timeout.
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let resume_msg = build_deny_resume_message(&message);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_RESUME_FALLBACK_MS)).await;
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
                        "AskUserQuestion decline did not resume the turn — re-delivered as a follow-up message (dead-air fallback)"
                    );
                }
                Some(_) => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion deny dead-air fallback: stdin send failed"
                ),
                None => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion deny dead-air fallback skipped: process not running"
                ),
            }
        });
        Ok(())
    }

    /// Decide a parked ordinary tool-permission request via the Agent SDK
    /// **control protocol** — the eventual `AgentDecisionPanel` Allow/Deny
    /// buttons, once `should_route_to_decision_panel` is flipped on (Phase 2,
    /// `SPEC_DECISION_PROMPT_2026_04_24.md` / `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`
    /// §5). Mirrors `answer_question`/`deny_question` exactly — same
    /// `pending_*` map lookup/removal, same dead-air safety net — combined
    /// into one method because the frontend's `tool:decision` RPC already
    /// carries `outcome` as a single field rather than two separate
    /// commands.
    ///
    /// `outcome` must be `"allow"` or `"deny"` — validated by the caller
    /// (`websocket.rs`'s `tooldecision` handler) before this is reached, the
    /// same division of responsibility as that handler's existing `scope`
    /// validation. An unrecognized value is treated as `"deny"` (fail
    /// closed) rather than panicking or silently allowing.
    ///
    /// On allow, `updatedInput` echoes the ORIGINAL input verbatim — this
    /// method does not support editing the call before approving it (no UI
    /// for that exists; `SPEC_DECISION_PROMPT_2026_04_24.md` never scoped
    /// one). On deny, `feedback` becomes the CLI-facing `message`
    /// (`SPEC_DECISION_PROMPT`'s G6: "Denials carry user-typed feedback
    /// verbatim to the agent"); a caller with no feedback gets a generic
    /// default so the model still learns the call was refused.
    ///
    /// See `answer_question`'s doc comment for the `pending_permissions`
    /// in-memory-only caveat (a fresh controller instance — pane reopen, any
    /// process respawn — starts with an empty map) and for why the error
    /// text names the tool_use_id and the likely cause.
    pub fn decide_tool_permission(
        &self,
        tool_use_id: String,
        outcome: &str,
        feedback: Option<String>,
    ) -> Result<(), String> {
        let (request_id, tool_name, input, tx) = {
            let mut inner = self.inner.lock().unwrap();
            let (rid, tool_name, input) = inner
                .pending_permissions
                .remove(&tool_use_id)
                .ok_or_else(|| format!(
                    "no pending tool-permission request for tool_use_id {tool_use_id} — this \
                     controller instance never recorded it (process likely respawned since the \
                     request was made, e.g. a pane close/reopen); the caller should redeliver \
                     as a follow-up message"
                ))?;
            let tx = inner
                .stdin_tx
                .as_ref()
                .ok_or("persistent process not running (cannot deliver decision)")?
                .clone();
            (rid, tool_name, input, tx)
        };

        let allow = outcome == "allow";
        let response_body = if allow {
            serde_json::json!({
                "behavior": "allow",
                "updatedInput": input,
                "toolUseID": tool_use_id,
            })
        } else {
            serde_json::json!({
                "behavior": "deny",
                "message": feedback.clone().unwrap_or_else(|| "Denied by user.".to_string()),
                "toolUseID": tool_use_id,
            })
        };
        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response_body,
            }
        });

        // Snapshot stdout activity BEFORE sending, same reasoning as
        // answer_question/deny_question (codex review on #1536).
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net — identical mechanism to answer_question's /
        // deny_question's (the CLI can abandon a pending tool_use whose turn
        // already ended regardless of whether the decision was allow or deny).
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let resume_msg = build_tool_decision_resume_message(&tool_name, outcome, feedback.as_deref());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_RESUME_FALLBACK_MS)).await;
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
                        "tool-permission decision did not resume the turn — re-delivered as a follow-up message (dead-air fallback)"
                    );
                }
                Some(_) => tracing::warn!(
                    block_id = %block_id,
                    "tool-permission dead-air fallback: stdin send failed"
                ),
                None => tracing::warn!(
                    block_id = %block_id,
                    "tool-permission dead-air fallback skipped: process not running"
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
    /// `answer_question`); every other tool is routed to
    /// `should_route_to_decision_panel`, which today always says no, so it is
    /// **auto-allowed** to preserve the current bypass/yolo UX (Phase 1; see
    /// that function's own doc comment for why Phase 2, #551, isn't simply
    /// "flip it to true"). `control_response` frames (replies to requests we
    /// initiate, none today) are logged and dropped. These frames are NOT
    /// conversation output and never reach the blockfile.
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
        } else if should_route_to_decision_panel(tool_name) {
            // PHASE2-GATE: unreachable in production today (see that
            // function's doc comment). Park exactly like AskUserQuestion
            // above; the eventual AgentDecisionPanel Allow/Deny answers via
            // `PersistentSubprocessController::decide_tool_permission`.
            park_tool_permission_request(inner, tool_use_id.clone(), request_id, tool_name.to_string(), input);
            tracing::info!(block_id = %block_id, tool_use_id = %tool_use_id, tool_name = %tool_name, "tool-permission request parked; awaiting user decision");
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
    fn spawn_process(
        &self,
        config: PersistentSpawnConfig,
        resume_retry_payload: Option<persistent_resume::QueuedRetryEntry>,
    ) -> Result<(), String> {
        // Captured before `resume_retry_payload` is moved further down (the
        // resume-bookkeeping match), for the turn-active decision near the
        // end of this function — see that check's own comment. Every
        // pre-existing caller either passes `Some(payload)` (a message is
        // about to be delivered directly) or `None` because a message is
        // already sitting in `pending_send_messages` waiting to be drained
        // right after this spawn succeeds (`respawn_once_for_leftover_
        // queue`) — this flag alone can't tell those two `None` cases
        // apart, which is exactly why the check below also looks at the
        // queue.
        let had_retry_payload = resume_retry_payload.is_some();

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
        // One session, one process. Resuming a session another live process
        // is still running on — the agent's other pane, or one that is
        // closing and hasn't exited yet — puts two CLIs on one transcript.
        // Refuse instead: the user closes the other one (or waits the few
        // seconds a close takes) and tries again. Covers every reopen path
        // (picker reattach, launch modal, MCP), not just `agent.open`
        // (SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §4.7).
        let requested_sid = if config.resume_flag.is_empty() {
            None
        } else {
            self.inner.lock().unwrap().session_id.clone()
        };
        // Check and claim must be one step: the check reads other
        // controllers' `current_pid`, which a concurrent resume of the same
        // session only sets further down this function. Held from here until
        // this spawn's `current_pid` is set (or it returns early), so two
        // reopens of one session — two picker clicks, picker + MCP — can't
        // both pass. Keyed by session (reagent P1 on #3421): a fixed stripe
        // of locks chosen by hashing the session id — every reopen of one
        // session takes the same lock, unrelated agents' resume respawns
        // almost never share one, and memory stays bounded (a per-session map
        // would grow forever — reagent P2). Only resume spawns take one;
        // `spawn_process` is synchronous, and no caller holds an `inner` lock
        // across it.
        let resume_claim = requested_sid.as_deref().map(|sid| {
            const STRIPES: usize = 64;
            static RESUME_SPAWN_LOCKS: [Mutex<()>; STRIPES] = [const { Mutex::new(()) }; STRIPES];
            let stripe = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                sid.hash(&mut h);
                (h.finish() as usize) % STRIPES
            };
            RESUME_SPAWN_LOCKS[stripe].lock().unwrap_or_else(|e| e.into_inner())
        });
        if let Some(sid) = requested_sid.as_deref() {
            if let Some((other, closing)) = self.session_held_elsewhere(sid) {
                return Err(if closing {
                    format!(
                        "This conversation's previous process (block {other}) is still shutting down. Try again in a few seconds."
                    )
                } else {
                    format!(
                        "This conversation is already open in another pane (block {other}). Close it there, or switch to it, instead of opening a second copy."
                    )
                });
            }
        }
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
            use agentmux_common::win32::CREATE_NO_WINDOW;
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
        let (my_generation, superseded_effects) = {
            let mut inner = self.inner.lock().unwrap();
            // The replacement process is here, so the quiesce window is over —
            // messages may take `DeliverDirect` against this new `stdin_tx`
            // again. Cleared unconditionally rather than only when set: any
            // spawn ends the window by definition, whatever opened it.
            inner.restart_pending = false;
            // …and any deferred restart is moot now, for the same reason: this
            // spawn read `cmd:args` fresh from block meta, so the new config is
            // already applied and there is nothing left to restart FOR.
            //
            // This is what stops a leak (reagent P1 on PR #2858):
            // `restart_when_idle` is consumed at the `is_result_frame` turn
            // end, but a generation that dies abnormally instead — a user
            // Stop/SIGINT, a crash, a permanently-failed turn resolving via
            // `ProcessExited` + `PublishDone` — never reaches that branch. The
            // stale `true` would then ride into an unrelated later generation
            // and kill a healthy process at the end of some future turn.
            // Clearing per-spawn scopes the flag to the generation that
            // requested it, which is the only one it ever meant anything for.
            inner.restart_when_idle = false;
            inner.spawn_generation += 1;
            let generation = inner.spawn_generation;
            let effects = match (attempted_resume_sid.clone(), resume_retry_payload) {
                (Some(sid), Some(retry_json)) => {
                    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                        generation,
                        attempted_sid: sid,
                        retry: persistent_resume::RetryPayload { config: config.clone(), messages: vec![retry_json] },
                    })
                }
                // `--resume <sid>` WAS attempted, just with no message of
                // our own to seed the retry batch with — the eager-resume
                // path (issue #3463), which revives a session with nothing
                // queued rather than in response to a message. codex P1 on
                // PR #3513: routing this to `SpawnedFresh` below (the
                // catch-all's original behavior) discarded `attempted_sid`
                // entirely, leaving `NotTracking` in place for a spawn that
                // in fact has an unconfirmed `--resume` in flight. If that
                // resume turns out to be stale and a direct message arrives
                // before the failure is detected, `MessageAppendedToRetryBatch`
                // below has nothing to append it to and no retry ever fires
                // when the doomed process exits — the message is silently
                // lost. An empty `messages` starts the SAME `AwaitingOutcome`
                // tracking a seeded resume gets; `MessageAppendedToRetryBatch`
                // (`DeliverDirect`, below) is what fills it in as messages
                // actually arrive. Never reachable before eager resume
                // existed — every other caller either omits `--resume`
                // entirely (`respawn_once_for_leftover_queue` clears
                // `session_id` first) or always has a real payload
                // (`BecomeSpawner`, `retry_after_resume_failure`).
                (Some(sid), None) => {
                    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                        generation,
                        attempted_sid: sid,
                        retry: persistent_resume::RetryPayload { config: config.clone(), messages: vec![] },
                    })
                }
                (None, _) => inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation }),
            };
            (generation, effects)
        };
        // Identity for this spawn's muxbus/registry registrations
        // (`registration_nonce` on `AgentRegistration`/`AgentEntry`).
        // Deliberately NOT `my_generation`: the generation is
        // controller-LOCAL (starts at 0 per controller instance), so a
        // replacement controller for this same block
        // (`resync_controller` stops the old one asynchronously and
        // constructs a new one immediately) restarts at generation 1 —
        // the old and new processes could then share a generation, and
        // the old exit-handler's compare-and-remove would accept the
        // replacement's fresh registration as its own and delete it
        // (codex P1 on PR #2500). A process-wide counter can never
        // collide across controller instances.
        let my_registration_nonce = next_registration_nonce();
        // reagentx P1 (round 7 on this PR): this fresh spawn can supersede
        // a PRIOR generation that was still AwaitingOutcome/ConfirmedRetry
        // with a held error line (see `resolve_superseded_generation`'s
        // own doc comment for the exact race — reachable via
        // `respawn_once_for_leftover_queue`) — flush it now, outside the
        // lock, same as every other `ResumeEffect` call site in this
        // module. Discarding this return value silently lost the exact
        // "error disappears" bug class (#2368) this PR exists to fix.
        for effect in superseded_effects {
            match effect {
                persistent_resume::ResumeEffect::FlushErrorLine(line)
                | persistent_resume::ResumeEffect::PersistImmediately(line) => {
                    self.flush_error_line_now(line);
                }
                other => {
                    tracing::warn!(
                        block_id = %self.block_id,
                        effect = ?other,
                        "unexpected ResumeEffect from a fresh spawn superseding a prior generation"
                    );
                }
            }
        }

        // This process starts with none of the conversation the pane is about
        // to display — say so, in the transcript, before any of its own output
        // lands. See `fresh_start_needs_disclosure` for why
        // `persistent_resume`'s `SpawnedFresh` can't decide this itself.
        //
        // `attempted_sid` is empty: there was no id to attempt, which is the
        // whole point. The frontend renders that as "—" rather than a blank
        // (`DocumentRow.tsx`'s session-outcome body).
        if fresh_start_needs_disclosure(attempted_resume_sid.as_deref(), my_generation)
            && self.has_prior_transcript()
        {
            tracing::info!(
                block_id = %self.block_id,
                "spawned with no --resume while prior history exists — disclosing a fresh start"
            );
            self.emit_session_outcome_now(
                persistent_resume::SessionOutcome::Fresh,
                String::new(),
                None,
            );
        }

        let pid = child.id().unwrap_or(0);

        // Notify the turn-activity tracker that a turn is starting — but
        // only if one actually is. Every pre-existing caller of this method
        // always has a message about to flow through, one way or the other
        // (see `had_retry_payload`'s own comment), so this was previously
        // unconditionally correct. `PersistentSubprocessController::
        // start()`'s eager-resume path (issue #3463) is the first caller
        // that genuinely has nothing queued — it revives a session so it is
        // ready for the NEXT message, the same as `ShellController::
        // start()` reviving a shell leaves it idle rather than mid-command.
        // Marking a turn active here with no message ever sent meant a
        // freshly eager-resumed pane was reported as perpetually WORKING —
        // to the pane UI, Swarm view, subagent watcher, and shutdown logic,
        // all of which read this flag — until a human happened to notice
        // and send it something, since nothing would ever produce the
        // `result` frame that normally clears it (codex P1 on PR #3513).
        if had_retry_payload || !self.inner.lock().unwrap().pending_send_messages.is_empty() {
            self.health_monitor.set_active_turn(true);
        }

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
            crate::backend::process_tracker::registry::track_spawned(&self.block_id, pid);
        }

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<KillRequest>();
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
            let mstore_stderr = self.mstore.clone();
            let event_bus_stderr = self.event_bus.clone();
            let attempted_resume_sid = attempted_resume_sid.clone();
            let my_generation_stderr = my_generation;
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
                            inner_stderr.lock().unwrap().poison_resume(bad_sid, my_generation_stderr);
                            tracing::warn!(
                                block_id = %block_id_stderr,
                                session_id = %bad_sid,
                                "stale --resume session id unreachable under the current config dir — \
                                 clearing so the next message starts a fresh conversation"
                            );
                            core::persist_session_id(&block_id_stderr, "", &mstore_stderr, &event_bus_stderr);
                            // Surface this to the user — previously silent
                            // (only the warn! above). See
                            // SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md
                            // §4.2: a resumed conversation silently starting
                            // fresh, with no indication anything happened, is
                            // exactly the failure mode this flag exists to close.
                            if let Some(ref store) = mstore_stderr {
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
        // Now visible to `session_held_elsewhere` — a concurrent resume of
        // the same session may run its check.
        drop(resume_claim);
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
        *self.agent_id.lock().unwrap() = agent_id_for_muxbus.clone();
        // Write-once: unlike `agent_id` above (refreshed every turn by
        // `input.rs`'s Register-tail), `stable_agent_id` is only ever set
        // here, at spawn, and never touched again — see its field doc
        // comment. `spawn_process` can in principle run again for the same
        // controller on a respawn; re-writing the SAME env-derived value
        // each time is harmless (idempotent), and a respawn with genuinely
        // different env (rare, config change) correctly updates the alias
        // to match, same as the primary registration already does.
        *self.stable_agent_id.lock().unwrap() = agent_id_for_muxbus.clone();
        // Defaults to this spawn's own nonce; overridden below if
        // registration is skipped (reagent P1 on PR #3084 — see the `Err`
        // arm just below for why `my_registration_nonce` alone is wrong
        // once a skip is possible).
        let mut exit_cleanup_nonce = my_registration_nonce;
        if let Some(ref agent_id) = agent_id_for_muxbus {
            // `_with_nonce` variants record this spawn's process-wide
            // registration nonce so this exact spawn's exit-handler can
            // compare-and-remove its own registrations instead of
            // blindly wiping a fallback respawn's (or replacement
            // controller's) fresh ones (issue #2363; codex P1 on PR
            // #2500 for why not the controller-local generation).
            // `try_register_agent_with_nonce`, not the plain
            // `register_agent_with_nonce` — this spawn can be running on the
            // same thread as an in-flight `inject_message` (the
            // reactive-delivery fallback's synchronous respawn), and that
            // call already holds this same handler's lock. The plain
            // version would re-lock it on that thread and deadlock the
            // reactive handler process-wide. See
            // `ReactiveHandler::try_register_agent_with_nonce`'s doc comment
            // and `docs/incident/INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`.
            match crate::backend::reactive::get_global_handler()
                .try_register_agent_with_nonce(
                    agent_id,
                    &self.block_id,
                    Some(&self.tab_id),
                    my_registration_nonce,
                    // Also park `agent_id` (== AGENTMUX_AGENT_ID here) as a
                    // permanent alias for this block, independent of the
                    // primary registration key — `input.rs`'s Register-tail
                    // will re-key the primary registration to the live
                    // display name on this agent's very first turn, which
                    // would otherwise evict this exact string with nothing
                    // left to answer a jekt tagged with the stable ID. See
                    // `INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`.
                    Some(agent_id.as_str()),
                )
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
                        let data_dir = crate::backend::base::get_mux_data_dir();
                        crate::backend::reactive::registry::write_with_nonce(
                            &data_dir,
                            agent_id,
                            &local_url,
                            &self.block_id,
                            my_registration_nonce,
                        );
                        crate::backend::reactive::registry::write_shared_from_env_with_nonce(
                            agent_id,
                            &local_url,
                            &self.block_id,
                            my_registration_nonce,
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        block_id = %self.block_id,
                        agent_id = %agent_id,
                        error = %e,
                        "muxbus: persistent auto-register failed"
                    );
                    // reagent P1 on PR #3084: registration was skipped, so
                    // `my_registration_nonce` was never written to the
                    // handler. Using it as this spawn's exit-time cleanup
                    // key would always mismatch whatever nonce IS on
                    // record, making the exit-handler wrongly conclude "a
                    // newer respawn already took over" and skip BOTH the
                    // reactive-handler unregister and the cloud_subscriber
                    // deregister — leaking both on every skip. Target
                    // whichever nonce is actually on record instead, so
                    // this spawn's own eventual exit can still correctly
                    // clean it up. `registration_nonce == 0` (no nonce on
                    // record at all) is handled safely by
                    // `unregister_block_if_nonce` itself — it never
                    // matches, so this degrades to today's no-cleanup
                    // behavior rather than a wrong one.
                    //
                    // `try_get_agent_by_block`, NOT the plain
                    // `get_agent_by_block` — this Err arm is reached
                    // precisely when we might be on the same thread as an
                    // in-flight `inject_message` (reagent P0 on PR #3084,
                    // caught after the first attempt used the blocking
                    // version here and reproduced INCIDENT_2026_09_07's
                    // exact deadlock). In that reentrant case this
                    // correctly returns `None` after its own retry budget
                    // instead of hanging forever; `exit_cleanup_nonce`
                    // then simply stays at its default, same safe
                    // no-cleanup degradation as before this fix existed.
                    if let Some(current) = crate::backend::reactive::get_global_handler()
                        .try_get_agent_by_block(&self.block_id)
                    {
                        exit_cleanup_nonce = current.registration_nonce;
                    }
                }
            }
        }

        // Record active pid for crash recovery (Phase 4.2). If the server
        // dies while this subprocess is running, scan_orphans() will find
        // the stale pid on next boot and flag the session as interrupted.
        if let Some(ref mstore) = self.mstore {
            super::session_recovery::mark_active_pid(mstore, &self.block_id, pid);
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
        let mstore_read = self.mstore.clone();
        let event_bus_read = self.event_bus.clone();
        let filestore_read = self.filestore.clone();
        let health_read = Arc::clone(&self.health_monitor);
        // Weak, like every other self-reference here: the reader task must not
        // keep the controller alive. Used only for the deferred
        // runtime-config restart at turn end (`request_restart_when_idle`).
        let self_ref_read = self.self_ref.lock().unwrap().clone();
        let stdout_seq_read = Arc::clone(&self.stdout_seq);
        let session_id_field = config.session_id_field.clone();
        let my_generation_read = my_generation;
        // Resolve the agent's GLOBAL transcript zone (`agent:<defId>:current`)
        // once, from the block's `agentId` meta, so every `output` line is also
        // mirrored to the cross-channel store. `None` for non-agent blocks.
        let global_output_zone =
            super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
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
                // including control frames handled via `continue` below — so
                // the AskUserQuestion dead-air fallback can tell whether the
                // turn resumed. See `answer_question`.
                stdout_seq_read.fetch_add(1, Ordering::Relaxed);

                // Track session metadata (debounced 1 s)
                stats.record_line(line.len(), &mstore_read);

                // Set (instead of persisted immediately) when this line turns
                // out to be a terminal `result`/`is_error:true` event arriving
                // while a stale-`--resume` retry could still be confirmed for
                // this exact attempt — see
                // `PersistentInner::pending_error_result_line`. `false` for
                // every other line, matching today's behavior exactly.
                let mut hold_back_for_resume_retry = false;

                // Parse JSON for control-frame handling, turn-active tracking,
                // and session ID capture
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
                    let is_result_frame =
                        parsed.get("type").and_then(|v| v.as_str()) == Some("result");
                    // Claude's turn-ending marker. Persistent mode never exits
                    // between turns, so this is the only place `turn_active`
                    // can go back to false without waiting for process exit —
                    // see `send_message`'s matching `set_active_turn(true)`.
                    if is_result_frame {
                        // A runtime-config change (model/effort/permission)
                        // arrived mid-turn and was deferred rather than
                        // killing this turn — see
                        // `request_restart_when_idle`. The turn is over now,
                        // so stop the process: the next message respawns it,
                        // and `input.rs` rebuilds `cli_args` from block meta
                        // at that point, so the new flags take effect then.
                        // Not `poison_resume` and not a user Stop — the
                        // session id is retained, so the respawn `--resume`s
                        // the same conversation.
                        // `set_active_turn(false)` happens under `inner` so it
                        // serializes with `request_restart_when_idle`'s own
                        // check — see that method for the interleaving this
                        // closes. `restart_pending` is committed in the SAME
                        // acquisition so no send can slip into `DeliverDirect`
                        // between the decision and the kill.
                        let deferred_restart = {
                            let mut locked = inner_read.lock().unwrap();
                            health_read.set_active_turn(false);
                            let deferred = std::mem::replace(&mut locked.restart_when_idle, false);
                            if deferred {
                                locked.restart_pending = true;
                            }
                            deferred
                        };
                        if deferred_restart {
                            tracing::info!(
                                block_id = %block_id_read,
                                "turn ended — applying the deferred runtime-config restart"
                            );
                            if let Some(ctrl) = self_ref_read.as_ref().and_then(|w| w.upgrade()) {
                                let _ = ctrl.stop_process(false);
                            }
                        }
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
                                    shellprocpid: None,
                                    shellprocname: String::new(),
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
                        //
                        // On a BLOCKING thread, not this one. Reconciliation
                        // used to be a pure in-memory status flip under a
                        // mutex (O(1)), but as of the completion-detection
                        // fix (#3007) it reads the parent transcript from
                        // disk — routinely tens of MB, plus per-line JSON
                        // parsing — to decide whether each member actually
                        // returned. Doing that inline would block a tokio
                        // worker thread for the whole read, and this fires on
                        // EVERY turn-end, so several blocks finishing at once
                        // could stall the runtime (reagent P1 on #3007).
                        //
                        // Fire-and-forget: the result is a status correction
                        // broadcast to whoever's listening, not something this
                        // stdout-reader task consumes, so there is nothing to
                        // await. The backfill call site (`scan_session_
                        // subagents`) needs no equivalent — it already runs in
                        // a blocking context that walks up to 200 files.
                        let session_id_snapshot = inner_read.lock().unwrap().session_id.clone();
                        if let Some(sid) = session_id_snapshot {
                            if let Some(watcher) = subagent_watcher::global() {
                                let block_id = block_id_read.clone();
                                tokio::task::spawn_blocking(move || {
                                    watcher.reconcile_stale_subagents(&block_id, &sid);
                                });
                            }
                        }
                    }
                    // A turn WE interrupted to close the pane ends in an
                    // `is_error: true` result (`error_during_execution`).
                    // That is the requested stop, not a failure: treat it
                    // as an ordinary end of turn — no failure banner, no
                    // stale-`--resume` retry tracking (pane-close spec §9.4).
                    let closing_this_generation = is_result_frame
                        && inner_read.lock().unwrap().shutdown_generation == Some(my_generation_read);
                    let is_error_result = is_result_frame
                        && !closing_this_generation
                        && parsed.get("is_error").and_then(|v| v.as_bool()) == Some(true);
                    // reagentx P0 on PR #2371: the real CLI's stream-json
                    // protocol embeds `session_id_field` on EVERY event,
                    // including the terminal `result` — so the doomed
                    // attempt's own `is_error:true` line ALSO carries the
                    // (stale) sid it was given. Calling
                    // `try_capture_session_id` for THIS exact line would
                    // resolve this generation's resume tracking (a
                    // non-poisoned confirmation does — codex P1's fix,
                    // needed for the genuinely-successful-resume case)
                    // BEFORE the `ErrorResultLine` event below ever runs,
                    // reproducing the exact bubble this PR exists to
                    // suppress AND preventing PR #2360's own retry from
                    // ever being confirmed. An error frame's echoed sid is
                    // never genuine progress (the turn failed) — skip
                    // session-id capture entirely for this exact frame;
                    // every other frame type (system/init, a successful
                    // result) still captures normally.
                    // Set when the capture_effects loop below classifies and
                    // persists a flushed OLDER turn's failure this tick — the
                    // clear-on-success step further down must not immediately
                    // wipe out state it just recorded (SPEC_PERSISTENT_
                    // CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md).
                    let mut flushed_failure_this_tick = false;
                    if !is_error_result {
                        if let Some(sid) = parsed.get(&session_id_field).and_then(|v| v.as_str()) {
                            let sid_string = sid.to_string();
                            // reagentx P0 on PR #2371: a genuinely
                            // successful terminal result (is_result_frame
                            // && !is_error, which is guaranteed true here
                            // since is_error_result already excluded the
                            // failing case) is the ONLY unambiguous proof
                            // that resuming THIS sid actually worked —
                            // any earlier frame (e.g. a "system"/init
                            // frame) echoes the same attempted sid
                            // regardless of whether the resume goes on to
                            // fail, per persistent_resume::update's own
                            // handling of `SessionCaptured`.
                            let is_confirmed_success = is_result_frame;
                            // See PersistentInner::try_capture_session_id — refuses
                            // to (re-)adopt an id the stderr reader (above) already
                            // confirmed unreachable, whichever task wins the race.
                            let (should_capture, capture_effects) = inner_read.lock().unwrap().try_capture_session_id(
                                &sid_string,
                                my_generation_read,
                                is_confirmed_success,
                            );
                            if should_capture {
                                tracing::info!(
                                    block_id = %block_id_read,
                                    session_id = %sid_string,
                                    "persistent session ID captured"
                                );
                                core::persist_session_id(&block_id_read, &sid_string, &mstore_read, &event_bus_read);
                            }
                            // reagentx P0 on PR #2373: resolving tracking
                            // here can legitimately flush a held-back
                            // error line from an earlier turn on this
                            // same still-alive generation — execute it,
                            // same as every other `ResumeEffect` call
                            // site in this module. `SessionCaptured` can
                            // also now resolve tracking outright and
                            // produce an `EmitSessionOutcome` (see
                            // `persistent_resume::update`'s own handling,
                            // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                            // §2.1) — handled explicitly below; anything
                            // else falls to the catch-all, kept exhaustive
                            // rather than assuming the effect set never
                            // grows again.
                            for effect in capture_effects {
                                match effect {
                                    persistent_resume::ResumeEffect::EmitSessionOutcome {
                                        outcome,
                                        attempted_sid,
                                        actual_sid,
                                    } => {
                                        // The retry (if any led here) is now resolved one
                                        // way or the other — clear "Reconnecting…". A no-op
                                        // publish (still fine) when this outcome came from a
                                        // plain first-time resume that was never retried.
                                        publish_resume_retry_status(&broker_read, &block_id_read, "resolved");
                                        // A recovery scan can turn an
                                        // already-disclosed resume failure
                                        // into a genuine resume — retract the
                                        // banner so it can't contradict the
                                        // `resumed` divider appended just
                                        // below. See
                                        // `session_recovery::clear_resume_failed`.
                                        if matches!(outcome, persistent_resume::SessionOutcome::Resumed) {
                                            if let Some(ref store) = mstore_read {
                                                super::session_recovery::clear_resume_failed(
                                                    store,
                                                    &event_bus_read,
                                                    &block_id_read,
                                                );
                                            }
                                        }
                                        if let Some(ref broker) = broker_read {
                                            let line = session_outcome_line(outcome, attempted_sid, actual_sid);
                                            super::shell::handle_append_block_file(
                                                broker,
                                                &block_id_read,
                                                PERSISTENT_OUTPUT_SUBJECT,
                                                line.as_bytes(),
                                                filestore_read.as_ref(),
                                                global_output_zone.as_deref(),
                                            );
                                        }
                                    }
                                    persistent_resume::ResumeEffect::FlushErrorLine(line) => {
                                        if let Some(ref broker) = broker_read {
                                            super::shell::handle_append_block_file(
                                                broker,
                                                &block_id_read,
                                                PERSISTENT_OUTPUT_SUBJECT,
                                                line.as_bytes(),
                                                filestore_read.as_ref(),
                                                global_output_zone.as_deref(),
                                            );
                                        }
                                        // reagentx P2 on PR #2421: this flushes
                                        // an earlier held-back turn's error,
                                        // finally confirmed final now that
                                        // session-id tracking resolved — must
                                        // get the same classify/persist/publish
                                        // treatment as the identical
                                        // FlushErrorLine handled at the
                                        // process-exit arm, or this turn's
                                        // failure silently loses its recovery
                                        // banner. No exit code exists for this
                                        // now-superseded turn.
                                        if let Some(failure) = classify_exit_line(None, &line) {
                                            flushed_failure_this_tick = true;
                                            core::persist_last_failure(&block_id_read, Some(&failure), &mstore_read, &event_bus_read);
                                            if let Some(ref broker) = broker_read {
                                                broker.publish(mps::MuxEvent {
                                                    event: mps::EVENT_AGENT_FAILURE.to_string(),
                                                    scopes: vec![format!("block:{}", block_id_read)],
                                                    sender: String::new(),
                                                    persist: 1,
                                                    data: serde_json::to_value(&failure).ok(),
                                                });
                                            }
                                        }
                                    }
                                    other => {
                                        tracing::warn!(
                                            block_id = %block_id_read,
                                            effect = ?other,
                                            "unexpected ResumeEffect from a SessionCaptured event"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Issue #2368: this generation's resume tracking
                    // (`persistent_resume::ResumeState`) decides whether
                    // this line is still a retry candidate — if it has
                    // already resolved (a session id was captured above or
                    // on an earlier line, or this generation never
                    // attempted an untrusted `--resume`), `update()`
                    // returns a `PersistImmediately` effect and today's
                    // immediate-persist behavior is unchanged; otherwise
                    // the line is held back pending the retry decision at
                    // process exit.
                    //
                    // reagentx P2 on PR #2373: hold-back is now decided
                    // from the RESULTING state, not `effects.is_empty()`
                    // — a still-tracking result can now ALSO carry a
                    // `PersistImmediately` effect for a SUPERSEDED
                    // held-back line from an earlier turn on this same
                    // generation (a second `is_error:true` while tracking
                    // is undecided means the first was a separate,
                    // already-settled turn's error). That effect must be
                    // flushed here explicitly; when NOT still tracking,
                    // any effect returned is THIS exact line's own
                    // `PersistImmediately`, already handled by the
                    // unchanged fallthrough below — executing it here too
                    // would double-persist it.
                    if is_error_result {
                        let (effects, still_tracking) = {
                            let mut inner = inner_read.lock().unwrap();
                            let effects = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
                                generation: my_generation_read,
                                line: format!("{}\n", line),
                            });
                            let still_tracking = matches!(
                                &inner.resume,
                                persistent_resume::ResumeState::AwaitingOutcome { generation, .. }
                                    if *generation == my_generation_read
                            ) || matches!(
                                &inner.resume,
                                persistent_resume::ResumeState::ConfirmedRetry { generation, .. }
                                    if *generation == my_generation_read
                            );
                            (effects, still_tracking)
                        };
                        hold_back_for_resume_retry = still_tracking;
                        if still_tracking {
                            for effect in effects {
                                match effect {
                                    persistent_resume::ResumeEffect::PersistImmediately(old_line)
                                    | persistent_resume::ResumeEffect::FlushErrorLine(old_line) => {
                                        if let Some(ref broker) = broker_read {
                                            super::shell::handle_append_block_file(
                                                broker,
                                                &block_id_read,
                                                PERSISTENT_OUTPUT_SUBJECT,
                                                old_line.as_bytes(),
                                                filestore_read.as_ref(),
                                                global_output_zone.as_deref(),
                                            );
                                        }
                                        // reagentx P1 on PR #2421 (round 2):
                                        // this is a SEPARATE, already-settled
                                        // older turn's error, superseded by
                                        // the current still-tracking line —
                                        // now confirmed final, same as the
                                        // other three FlushErrorLine/
                                        // PersistImmediately call sites this
                                        // PR wired up. `is_error_result` is
                                        // true for the rest of this tick, so
                                        // this can never collide with the
                                        // clear-on-success step below.
                                        if let Some(failure) = classify_exit_line(None, &old_line) {
                                            flushed_failure_this_tick = true;
                                            core::persist_last_failure(&block_id_read, Some(&failure), &mstore_read, &event_bus_read);
                                            if let Some(ref broker) = broker_read {
                                                broker.publish(mps::MuxEvent {
                                                    event: mps::EVENT_AGENT_FAILURE.to_string(),
                                                    scopes: vec![format!("block:{}", block_id_read)],
                                                    sender: String::new(),
                                                    persist: 1,
                                                    data: serde_json::to_value(&failure).ok(),
                                                });
                                            }
                                        }
                                    }
                                    other => {
                                        tracing::warn!(
                                            block_id = %block_id_read,
                                            effect = ?other,
                                            "unexpected ResumeEffect from a still-tracking ErrorResultLine event"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Classify + surface a genuine, non-retried error result
                    // (429/overloaded/auth/etc.) to the pane's failure-recovery
                    // UI — SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION.
                    // Gated on `!hold_back_for_resume_retry`: when the stale-
                    // `--resume` machinery above is still tracking this exact
                    // error as a live retry candidate, it must stay invisible
                    // to the user (per its own "must never reach the user"
                    // invariant below at the ProcessExited/FireRetry arm) —
                    // classify() only runs once this error is confirmed final.
                    if is_error_result && !hold_back_for_resume_retry {
                        let failure = crate::agents::failure::classify(None, None, "", Some(&parsed));
                        core::persist_last_failure(&block_id_read, Some(&failure), &mstore_read, &event_bus_read);
                        if let Some(ref broker) = broker_read {
                            broker.publish(mps::MuxEvent {
                                event: mps::EVENT_AGENT_FAILURE.to_string(),
                                scopes: vec![format!("block:{}", block_id_read)],
                                sender: String::new(),
                                persist: 1,
                                data: serde_json::to_value(&failure).ok(),
                            });
                        }
                    } else if is_result_frame && !is_error_result && !flushed_failure_this_tick {
                        // reagentx P1 on PR #2421: unlike host_spawn.rs, this
                        // controller never exits between turns, so nothing
                        // else ever clears a previously recorded failure —
                        // once one rate-limit/overloaded error was persisted,
                        // the pane's onMount seed logic kept re-showing that
                        // stale banner on every future reload, even after
                        // many later successful turns. A genuinely successful
                        // terminal result on this still-alive process is the
                        // signal that it's stale. persist_last_failure is a
                        // no-op when there's nothing to clear, so this is
                        // cheap on the (overwhelmingly common) already-clear
                        // path. Skipped when the capture_effects loop above
                        // just persisted a freshly-flushed OLDER failure this
                        // same tick — that state must survive, not be
                        // immediately wiped by this frame's own success.
                        core::persist_last_failure(&block_id_read, None, &mstore_read, &event_bus_read);
                    }
                }

                if hold_back_for_resume_retry {
                    continue;
                }

                // Publish line as MPS blockfile event and write-through to FileStore
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

        self.spawn_status_heartbeat();

        // Spawn process waiter task
        let block_id_wait = self.block_id.clone();
        let inner_wait = Arc::clone(&self.inner);
        let broker_wait = self.broker.clone();
        let mstore_wait = self.mstore.clone();
        // Needed to persist a classified failure (rate-limit/overloaded/etc.)
        // into block meta alongside the MPS publish below — mirrors
        // `event_bus_read`'s equivalent clone for the stdout-reader task.
        let event_bus_wait = self.event_bus.clone();
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
        // This exact spawn's OS pid, for the compare-and-clear below — the
        // unconditional clear could wipe a fallback respawn's fresh
        // registration (issue #2363, see clear_active_pid_if_pid).
        let pid_wait = pid;
        // This spawn's registration identity, for the guarded
        // muxbus/registry removals below (issue #2363 / codex P1 on PR
        // #2500 — see `my_registration_nonce`'s own doc comment). NOT
        // `my_registration_nonce` directly — `exit_cleanup_nonce` is that
        // same value UNLESS registration was skipped above, in which case
        // it's whatever nonce actually ended up on record instead (reagent
        // P1 on PR #3084; see the skip arm's own comment for why using our
        // own never-written nonce here would leak the registration).
        let nonce_wait = exit_cleanup_nonce;

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

                    // codex P1 on PR #2371 (round 1): the stdout reader
                    // performs synchronous FileStore/SQLite writes that
                    // can legitimately contend for multiple seconds
                    // (SQLite's own busy timeout) — a short bound (the
                    // stderr reader's 500ms above) would abort it
                    // mid-line under ordinary contention, discarding
                    // still-unread assistant/result frames and
                    // truncating the persisted transcript.
                    //
                    // codex P1 on PR #2371 (round 2): but an UNBOUNDED
                    // await isn't safe either — if the CLI spawned a
                    // background descendant that inherited its stdout
                    // descriptor, killing/waiting for the direct child
                    // does NOT close that descriptor, so this reader's
                    // `lines.next_line()` may never see EOF, hanging this
                    // exit-handling step (and everything after it —
                    // health status, muxbus deregistration, the retry
                    // decision itself) forever.
                    //
                    // 10s is the compromise: generous enough that
                    // ordinary SQLite contention never triggers the
                    // abort (avoiding the round-1 truncation risk), but
                    // still a hard ceiling so a genuinely stuck
                    // descendant-held pipe (or anything else gone wrong)
                    // can't hang this task indefinitely (closing the
                    // round-2 gap). Aborting our own read loop doesn't
                    // require the OS pipe to actually close — it just
                    // stops OUR wait, accepting we may not have drained
                    // every last buffered line, the same risk profile
                    // the original 500ms bound already accepted, just at
                    // a bound wide enough not to fire under normal load.
                    let abort_handle = stdout_reader_handle.abort_handle();
                    if tokio::time::timeout(std::time::Duration::from_secs(10), stdout_reader_handle)
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            block_id = %block_id_wait,
                            "stdout reader did not finish within 10s of process exit \
                             (SQLite contention, or a descendant process holding stdout open?) \
                             — aborting it"
                        );
                        abort_handle.abort();
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

                    let mut inner = inner_wait.lock().unwrap();
                    // reagentx P1 (round 6 on PR #2373, extended round 8):
                    // this belated exit-handling can run AFTER a fresh
                    // spawn has already superseded this generation (see
                    // `respawn_once_for_leftover_queue`'s own doc comment
                    // and the kill arm below for the documented race that
                    // makes this reachable) — `inner.spawn_generation` is
                    // bumped on every NEW spawn, so a mismatch here means
                    // this exit is for an already-superseded generation.
                    // Everything gated below belongs to THIS exact
                    // process (its own pid/exit code/stdin/kill channel,
                    // the shared `proc_status` this process last knew to
                    // be true, its own health-monitor/muxbus/registry/
                    // session-recovery registration) — running any of it
                    // unconditionally would corrupt or tear down a newer,
                    // actively-running generation's own state as if IT
                    // had exited. reagentx round 8: the round-6 fix only
                    // gated the field writes above, missing
                    // `health_wait.set_exited` (a shared `TurnActivityTracker`
                    // across generations) and the deregistration block
                    // below — both keyed by `block_id`/`agent_id`, not
                    // generation, so a stale exit incorrectly marked a
                    // live process's health as exited and tore down its
                    // muxbus/registry/session-recovery registration while
                    // it kept running.
                    let is_current_generation = inner.spawn_generation == my_generation_wait;
                    if is_current_generation {
                        inner.proc_exit_code = exit_code;
                        inner.current_pid = None;
                        inner.stdin_tx = None;
                        inner.kill_tx = None;
                    }
                    // One event resolves the ENTIRE retry/error-line
                    // decision — including any earlier `StopRequested`
                    // (see `stop_process`), already baked into the state
                    // by `persistent_resume::update` before this event
                    // ever arrives. See `persistent_resume`'s module doc
                    // comment for why this replaced four separate field
                    // reads/writes. Safe to call unconditionally even for
                    // a superseded generation — `update()`'s own
                    // generation-matching arms (and `NotTracking`'s
                    // `current_generation`) already no-op a stale event
                    // on their own.
                    let effects = inner
                        .apply_resume_event(persistent_resume::ResumeEvent::ProcessExited { generation: my_generation_wait });
                    if is_current_generation {
                        Self::set_status(&mut inner, STATUS_DONE);
                    }
                    drop(inner);

                    if is_current_generation {
                        // Notify health monitor so Stalled/Dead watchdog stops.
                        health_wait.set_exited(exit_code);

                        // Deregister from muxbus so later sends fall through to the
                        // lower tiers instead of resolving to a dead block. Mirrors
                        // the shell controller's exit path. This exact process's
                        // resources are gone either way; if a retry/fallback
                        // respawn has ALREADY re-registered, the guards below
                        // leave its fresh registration in place.
                        // All removals are compare-and-remove keyed on this
                        // spawn's process-wide registration nonce (issue
                        // #2363): the `is_current_generation` gate above
                        // was read once, and a fallback/retry respawn's
                        // fresh registration can land on a parallel task
                        // between that read and these calls — an
                        // unconditional removal here would wipe the NEW
                        // spawn's entry with nothing left to re-register
                        // it. Nonce, not generation: a replacement
                        // controller's spawn restarts at generation 1 and
                        // could collide with ours (codex P1 on PR #2500).
                        let registration_was_ours = crate::backend::reactive::get_global_handler()
                            .unregister_block_if_nonce(&block_id_wait, nonce_wait);
                        if let Some(ref agent_id) = agent_id_wait {
                            let data_dir = crate::backend::base::get_mux_data_dir();
                            crate::backend::reactive::registry::remove_if_nonce(
                                &data_dir,
                                agent_id,
                                nonce_wait,
                            );
                            crate::backend::reactive::registry::remove_shared_from_env_if_nonce(
                                agent_id,
                                nonce_wait,
                            );
                            // The cloud subscriber's agent set records no
                            // per-agent identity to compare against, so the
                            // in-memory registration's outcome above stands
                            // in: if a newer spawn already re-registered,
                            // its cloud subscription must survive too.
                            if registration_was_ours {
                                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                                    sub.remove_agent(agent_id);
                                }
                            }
                        }

                        // Clear active pid — clean exit, no recovery needed.
                        // Compare-and-clear (issue #2363): only if the
                        // recorded pid is still THIS process's — a fallback
                        // respawn may have re-registered a fresh pid on a
                        // parallel task between the generation gate above
                        // and this call.
                        if let Some(ref mstore) = mstore_wait {
                            super::session_recovery::clear_active_pid_if_pid(mstore, &block_id_wait, pid_wait);
                        }
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
                    // controller type in PR #2338 (see docs/retro/
                    // RETRO_STALE_RESUME_SESSION_ID_ACROSS_CHANNELS_2026_07_29.md).
                    for effect in effects {
                        match effect {
                            // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                            // §2.1: the `ConfirmedRetry` + `ProcessExited`
                            // (not-stopped) arm now bundles this alongside
                            // `FireRetry` — the resume's fate (Fresh) is
                            // already known here, even though the retry
                            // below hasn't launched yet.
                            persistent_resume::ResumeEffect::EmitSessionOutcome {
                                outcome,
                                attempted_sid,
                                actual_sid,
                            } => {
                                publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");
                                // Same retraction as the stdout-reader site
                                // above — this arm reaches `Fresh` today, but
                                // the clear is keyed on the outcome rather
                                // than on which arm produced it, so it stays
                                // correct if that ever changes.
                                if matches!(outcome, persistent_resume::SessionOutcome::Resumed) {
                                    if let Some(ref store) = mstore_wait {
                                        super::session_recovery::clear_resume_failed(
                                            store,
                                            &event_bus_wait,
                                            &block_id_wait,
                                        );
                                    }
                                }
                                if let Some(ref broker) = broker_wait {
                                    let line = session_outcome_line(outcome, attempted_sid, actual_sid);
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
                            persistent_resume::ResumeEffect::PersistImmediately(line)
                            | persistent_resume::ResumeEffect::FlushErrorLine(line) => {
                                // Safety net: a "Reconnecting…" left showing from
                                // an earlier retry attempt on this same block must
                                // not get stuck forever just because THIS exit
                                // ended up genuinely non-retryable — a harmless
                                // no-op publish in the (common) case where no
                                // retry was in flight at all.
                                publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");
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
                                // Classify + surface this exit's error to the
                                // pane's failure-recovery UI —
                                // SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION.
                                // Reached only for PersistImmediately/FlushErrorLine
                                // — the resume machinery has already decided this
                                // exit is NOT being silently retried (contrast
                                // FireRetry below, which must stay invisible to
                                // the user).
                                if let Some(failure) = classify_exit_line(Some(exit_code), &line) {
                                    core::persist_last_failure(&block_id_wait, Some(&failure), &mstore_wait, &event_bus_wait);
                                    if let Some(ref broker) = broker_wait {
                                        broker.publish(mps::MuxEvent {
                                            event: mps::EVENT_AGENT_FAILURE.to_string(),
                                            scopes: vec![format!("block:{}", block_id_wait)],
                                            sender: String::new(),
                                            persist: 1,
                                            data: serde_json::to_value(&failure).ok(),
                                        });
                                    }
                                }
                            }
                            persistent_resume::ResumeEffect::FireRetry { retry, held_error_line, attempted_sid } => {
                                // Issue #2368: the retry is firing and will
                                // very likely succeed within milliseconds —
                                // the doomed attempt's own terminal error
                                // result must never reach the user, so it's
                                // dropped (not flushed) as long as the retry
                                // actually launches. Handed to
                                // `retry_after_resume_failure` itself (codex
                                // P2 on PR #2371) rather than dropped here
                                // unconditionally — a retry that turns out
                                // NOT to launch (this controller already
                                // gone, or the fresh spawn itself failing)
                                // must still flush it, or an already-
                                // accepted prompt ends in total silence.
                                if let Some(ctrl) = self_ref_wait.upgrade() {
                                    tracing::warn!(
                                        block_id = %block_id_wait,
                                        "stale --resume session id caused this exit — retrying now (find_recovery_session_id may still resume a real, on-disk session rather than starting blank)"
                                    );
                                    ctrl.retry_after_resume_failure(
                                        my_generation_wait,
                                        retry.config,
                                        retry.messages,
                                        held_error_line,
                                        attempted_sid,
                                    );
                                } else {
                                    // reagentx P2 on PR #2776: the controller
                                    // itself is already gone — no retry will
                                    // ever fire for this batch, so any
                                    // "Reconnecting…" left showing from an
                                    // earlier attempt on this same block must
                                    // be resolved here; nothing else will.
                                    publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");
                                    if let Some(line) = held_error_line {
                                    // The controller itself is already gone
                                    // (weak ref invalidated) — nothing can
                                    // retry this batch at all, so flush
                                    // directly via this task's own captured
                                    // broker/filestore rather than going
                                    // through `ctrl`.
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
                                }
                            }
                            persistent_resume::ResumeEffect::PublishDone => {
                                if let Some(ref broker) = broker_wait {
                                    let status = BlockControllerRuntimeStatus {
                                        blockid: block_id_wait.clone(),
                                        version: 0,
                                        shellprocstatus: STATUS_DONE.to_string(),
                                        shellprocconnname: "local".to_string(),
                                        shellprocexitcode: exit_code,
                                        shellprocpid: None,
                                        shellprocname: String::new(),
                                        spawn_ts_ms: None,
                                        is_agent_pane: true,
                                        turn_active: false,
                                    };
                                    super::publish_controller_status(broker, &status);
                                }
                            }
                        }
                    }
                }
                Ok(request) = kill_rx => {
                    let graceful_deadline = match request {
                        KillRequest::Force => None,
                        KillRequest::Graceful(deadline) => Some(deadline),
                    };
                    tracing::info!(
                        block_id = %block_id_wait,
                        force = graceful_deadline.is_none(),
                        "persistent process kill requested"
                    );
                    if let Some(deadline) = graceful_deadline {
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
                        // Wait until the caller's deadline (a close shares
                        // one deadline across every agent it stops — spec
                        // §9.2), then kill.
                        let killed = tokio::select! {
                            _ = child.wait() => false,
                            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                                let _ = child.kill().await;
                                true
                            }
                        };
                        inner_wait.lock().unwrap().stop_exit = Some((my_generation_wait, killed));
                    } else {
                        let _ = child.kill().await;
                        inner_wait.lock().unwrap().stop_exit = Some((my_generation_wait, true));
                    }

                    // reagentx P1 on PR #2371: mirror the child.wait() arm's
                    // bounded await+abort of both reader tasks (above,
                    // codex P1) before taking `pending_error_result_line`
                    // below. Without this, a stop racing the doomed
                    // attempt's in-flight terminal error line could take
                    // `None` here while the stdout reader is still about to
                    // stash it — silently losing a genuine error the stop
                    // itself interrupted, and leaving a stale stash for a
                    // LATER, unrelated exit on a reused controller instance
                    // to wrongly pick up.
                    if let Some(handle) = stderr_reader_handle {
                        let abort_handle = handle.abort_handle();
                        if tokio::time::timeout(std::time::Duration::from_millis(500), handle).await.is_err() {
                            tracing::warn!(
                                block_id = %block_id_wait,
                                "stderr reader did not finish within 500ms of kill — aborting it"
                            );
                            abort_handle.abort();
                        }
                    }
                    // codex P1 on PR #2371 (round 2): a bounded wait, not
                    // an unconditional one — see the child.wait() arm's
                    // identical comment above for the full reasoning. This
                    // matters MORE here: codex flagged that every
                    // remaining cleanup step (clearing current_pid/
                    // kill_tx, STATUS_DONE, muxbus deregistration) runs
                    // AFTER this await, so an unbounded hang here would
                    // leave a user-initiated Stop making the controller
                    // appear permanently alive if a descendant process
                    // ever holds the stdout descriptor open.
                    let abort_handle = stdout_reader_handle.abort_handle();
                    if tokio::time::timeout(std::time::Duration::from_secs(10), stdout_reader_handle)
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            block_id = %block_id_wait,
                            "stdout reader did not finish within 10s of kill \
                             (SQLite contention, or a descendant process holding stdout open?) \
                             — aborting it"
                        );
                        abort_handle.abort();
                    }

                    let mut inner = inner_wait.lock().unwrap();
                    // reagentx P1 (round 6 on PR #2373, extended round 8):
                    // same reasoning as the child.wait() arm above — this
                    // graceful-stop cleanup can itself run AFTER the
                    // documented race just above (dropping `stdin_tx`
                    // early to trigger EOF lets
                    // `respawn_once_for_leftover_queue` spawn a brand-new
                    // generation while this kill is still mid-flight) has
                    // already superseded this generation. Everything
                    // gated below belongs to THIS exact kill (its own
                    // pid/exit code/stdin/kill channel, its own spawn
                    // claim/queue, the shared `proc_status` this stop
                    // last knew to be true, its own health-monitor/
                    // muxbus/registry/session-recovery registration) —
                    // running any of it unconditionally would corrupt or
                    // tear down a newer, actively-running generation's
                    // own state as if IT had been stopped.
                    let is_current_generation = inner.spawn_generation == my_generation_wait;
                    if is_current_generation {
                        inner.proc_exit_code = -1;
                        inner.current_pid = None;
                        inner.stdin_tx = None;
                        inner.kill_tx = None;
                    }
                    // A user-initiated kill overrides any resume-retry
                    // decision in flight, for a REUSED controller instance
                    // (`resync_controller` can reuse the same instance
                    // across a kill+restart cycle) the same way an
                    // in-flight Stop already does for the child.wait() arm
                    // — reusing that exact `StopRequested` + `ProcessExited`
                    // event pair here instead of duplicating the "stop
                    // wins" logic against raw fields. codex P2 on PR #2371:
                    // this also means a stashed error line is never
                    // silently lost on a user-initiated stop (it may be a
                    // genuine error the stop itself interrupted) — it's
                    // flushed below via the effects this produces, same as
                    // the "genuinely done" case. Safe to call
                    // unconditionally even for a superseded generation —
                    // `update()`'s own generation-matching arms already
                    // no-op a stale event on their own.
                    inner.apply_resume_event(persistent_resume::ResumeEvent::StopRequested {
                        generation: my_generation_wait,
                    });
                    let effects = inner
                        .apply_resume_event(persistent_resume::ResumeEvent::ProcessExited { generation: my_generation_wait });
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
                    // fallback respawn triggered. Gated the same way as
                    // above — a NEWER generation's own claim/queue must
                    // never be cleared by this stale one's cleanup.
                    if is_current_generation {
                        inner.pending_send_messages.clear();
                        inner.spawning_in_progress = false;
                        Self::set_status(&mut inner, STATUS_DONE);
                    }
                    drop(inner);

                    // reagentx P1 on PR #2776: a user-initiated Stop can
                    // land at any point, including mid stale-`--resume`
                    // retry — "Reconnecting…" has no other clearing path on
                    // this branch (unlike `compacting`, which several
                    // turn-end transitions defensively reset), so without
                    // this it would stick on-screen forever with a growing
                    // counter, and a later retry on the same pane would
                    // wrongly keep the stale `startedAt` (ResumeRetryStarted
                    // is a no-op while already reconnecting). Unconditional
                    // and un-gated by `is_current_generation` on purpose —
                    // a harmless no-op when nothing was reconnecting, and
                    // the pane is stopping either way, so there's no
                    // "which generation" ambiguity worth encoding here.
                    publish_resume_retry_status(&broker_wait, &block_id_wait, "resolved");

                    // Flush a held-back error line now, if the resume
                    // state machine produced one — the `StopRequested`
                    // sent above guarantees `ProcessExited` resolves via
                    // the "stop wins" branch (see `persistent_resume::
                    // update`), so `FireRetry` is never actually possible
                    // here, but it's still matched defensively rather
                    // than assumed.
                    for effect in effects {
                        let line = match effect {
                            persistent_resume::ResumeEffect::PersistImmediately(line)
                            | persistent_resume::ResumeEffect::FlushErrorLine(line) => Some(line),
                            persistent_resume::ResumeEffect::FireRetry { held_error_line, .. } => held_error_line,
                            persistent_resume::ResumeEffect::PublishDone => None,
                            // Not actually reachable here today — the "stop
                            // wins" branch of `update()`'s ConfirmedRetry +
                            // ProcessExited arm never produces this effect
                            // (see SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                            // §2.1) — but matched defensively, same as
                            // `FireRetry` above, rather than assumed. Reuses
                            // the same "append this line" path as every
                            // other variant here.
                            persistent_resume::ResumeEffect::EmitSessionOutcome {
                                outcome,
                                attempted_sid,
                                actual_sid,
                            } => Some(session_outcome_line(outcome, attempted_sid, actual_sid)),
                        };
                        if let Some(line) = line {
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
                    }

                    if is_current_generation {
                        // Notify the turn-activity tracker of the exit —
                        // shared `Arc<TurnActivityTracker>` across
                        // generations, gated the same way as the
                        // child.wait() arm above.
                        health_wait.set_exited(-1);

                        // Deregister from muxbus (see the clean-exit arm above).
                        // All removals are compare-and-remove keyed on this
                        // spawn's process-wide registration nonce (issue
                        // #2363): the `is_current_generation` gate above
                        // was read once, and a fallback/retry respawn's
                        // fresh registration can land on a parallel task
                        // between that read and these calls — an
                        // unconditional removal here would wipe the NEW
                        // spawn's entry with nothing left to re-register
                        // it. Nonce, not generation: a replacement
                        // controller's spawn restarts at generation 1 and
                        // could collide with ours (codex P1 on PR #2500).
                        let registration_was_ours = crate::backend::reactive::get_global_handler()
                            .unregister_block_if_nonce(&block_id_wait, nonce_wait);
                        if let Some(ref agent_id) = agent_id_wait {
                            let data_dir = crate::backend::base::get_mux_data_dir();
                            crate::backend::reactive::registry::remove_if_nonce(
                                &data_dir,
                                agent_id,
                                nonce_wait,
                            );
                            crate::backend::reactive::registry::remove_shared_from_env_if_nonce(
                                agent_id,
                                nonce_wait,
                            );
                            // The cloud subscriber's agent set records no
                            // per-agent identity to compare against, so the
                            // in-memory registration's outcome above stands
                            // in: if a newer spawn already re-registered,
                            // its cloud subscription must survive too.
                            if registration_was_ours {
                                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                                    sub.remove_agent(agent_id);
                                }
                            }
                        }

                        // Clear active pid — user-initiated stop, no recovery needed.
                        // Compare-and-clear (issue #2363), same as the
                        // clean-exit arm: the `is_current_generation` gate
                        // above was read once under the lock, and a fallback
                        // respawn's re-registration can land between that
                        // read and this call.
                        if let Some(ref mstore) = mstore_wait {
                            super::session_recovery::clear_active_pid_if_pid(mstore, &block_id_wait, pid_wait);
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub fn stop_process(&self, force: bool) -> Result<(), String> {
        let request = if force {
            KillRequest::Force
        } else {
            KillRequest::Graceful(std::time::Instant::now() + super::SHUTDOWN_GRACE)
        };
        self.request_stop(request)
    }

    /// Another process already running on session `sid`: `(block id, closing)`.
    /// Checks live persistent controllers in the registry, then blocks
    /// mid-close (out of the registry, process not yet exited).
    fn session_held_elsewhere(&self, sid: &str) -> Option<(String, bool)> {
        for (block_id, ctrl) in super::get_all_controllers() {
            if block_id == self.block_id {
                continue;
            }
            if let Some(other) = ctrl.as_any().downcast_ref::<PersistentSubprocessController>() {
                let g = other.inner.lock().unwrap();
                if g.current_pid.is_some() && g.session_id.as_deref() == Some(sid) {
                    return Some((block_id, false));
                }
            }
        }
        let store = self.mstore.as_ref()?;
        super::closing_blocks_still_running()
            .into_iter()
            .filter(|block_id| *block_id != self.block_id)
            .find(|block_id| {
                store
                    .get::<crate::backend::obj::Block>(block_id)
                    .ok()
                    .flatten()
                    .is_some_and(|b| crate::backend::obj::meta_get_string(&b.meta, core::META_SESSION_ID, "") == sid)
            })
            .map(|block_id| (block_id, true))
    }

    fn request_stop(&self, request: KillRequest) -> Result<(), String> {
        Self::request_stop_on(&self.inner, request)
    }

    /// [`request_stop`] over just the shared state, for the `'static`
    /// future [`Controller::shutdown`] returns.
    fn request_stop_on(inner_arc: &Arc<Mutex<PersistentInner>>, request: KillRequest) -> Result<(), String> {
        let kill_tx = {
            let mut inner = inner_arc.lock().unwrap();
            // Recorded unconditionally, not only when `kill_tx` is already
            // `None` — codex P1 on PR #2360 (round 16, commit ce1642d90):
            // `stop_process` can race a process that already exited (a
            // confirmed stale-`--resume` death) and is about to be
            // silently retried — sending through `kill_tx` is futile in
            // that window regardless of whether it's already `None` (the
            // exit-handler cleared it) or still `Some` (`tokio::select!`
            // already committed to the `child.wait()` exit arm before
            // this call reached the lock, so the `kill_rx` arm will never
            // be polled again even if the send succeeds). Recording this
            // as a `StopRequested` event — resolved by
            // `persistent_resume::update` once `ProcessExited` arrives —
            // is the only way the exit-handler's retry decision can know
            // the user explicitly asked to stop.
            let generation = inner.spawn_generation;
            inner.apply_resume_event(persistent_resume::ResumeEvent::StopRequested { generation });
            inner.kill_tx.take()
        };
        if let Some(tx) = kill_tx {
            let _ = tx.send(request);
        }
        Ok(())
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.lock().unwrap().session_id.clone()
    }

    /// True when this controller is registered but has **no live process and no
    /// spawn already in flight** — the state a lazily-registered controller sits
    /// in between `register` ("spawns on first message") and its first
    /// `send_message`.
    ///
    /// In that state `inject_message` cannot deliver: it has no spawn config and
    /// only writes to a live `stdin_tx`, so it returns "persistent process not
    /// running". The caller can, however, start a *turn* — `run_agent_turn`
    /// re-reads the spawn config from block metadata and calls `send_message`,
    /// which does spawn. This predicate is what lets the reactive delivery path
    /// tell those two situations apart. See
    /// `docs/reports/REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03.md`.
    ///
    /// **The invariant this must uphold:** `needs_spawn() == true` implies a
    /// subsequent `send_message` takes `decide_send_action`'s `BecomeSpawner`
    /// branch — i.e. it really spawns and delivers. It must never be true in a
    /// state where `send_message` would instead return `SendAction::Queued`,
    /// because `Queued` returns `Ok(())` having delivered nothing, the reactive
    /// path maps that to `Ok(true)`, and the caller is told the message landed
    /// when it is still sitting in the queue. `cloud_subscriber` only retries
    /// on `!success`, so a false success is a permanently lost message.
    ///
    /// That is why all three exclusions are here, and each mirrors one of
    /// `decide_send_action`'s own guards:
    ///
    /// - `stdin_tx.is_none()` — a live process is steerable; `inject_message`
    ///   handles it and no turn start is wanted.
    /// - `!spawning_in_progress` — a caller has already claimed this spawn
    ///   round (see that field's doc comment on the concurrent-spawn TOCTOU
    ///   race). Reporting "needs spawn" here would invite a second, racing
    ///   spawn and orphan a child.
    /// - `!drain_claim` — a `RetryFlush` drain owns the round. This one is
    ///   subtle and was missed on the first cut (reagent P1 on PR #2960): when
    ///   a drain's target process dies mid-flush, the drain **deliberately
    ///   retains** its claim for the fallback respawn while the exit handler
    ///   clears `stdin_tx`. Without this term, that window reports "needs
    ///   spawn", `decide_send_action` then returns `Queued` on the
    ///   still-held claim, and the message is silently queued while the caller
    ///   is told it was delivered.
    ///
    /// Both excluded windows are retryable rather than lost: the caller gets
    /// the original delivery error back, which is what lets it come again.
    ///
    /// Racy by nature, and safe to be: the process can exit the instant after
    /// this returns `false`. It is a routing hint, not a guarantee — the spawn
    /// claim inside `send_message` is what actually serialises spawners. What
    /// it must not do is report `true` for a state that cannot spawn.
    pub fn needs_spawn(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.stdin_tx.is_none() && !inner.spawning_in_progress && !inner.drain_claim
    }

    /// Ask this controller to restart itself once the current turn ends,
    /// instead of being torn down and replaced right now.
    ///
    /// Returns `true` when the restart was deferred (a turn is in flight, so
    /// the caller must NOT replace the controller), `false` when the pane is
    /// idle and an immediate replace is safe — the caller then proceeds
    /// exactly as before.
    ///
    /// Called from `resync_controller`'s forced-replace path. See
    /// [`PersistentInner::restart_when_idle`] for what the old
    /// kill-immediately behaviour destroyed.
    ///
    /// Deliberately keyed on the health monitor's `is_active_turn` rather
    /// than on `stdin_tx.is_some()`: a live process between turns is idle and
    /// should be replaced immediately (that's the common `/model` case, and
    /// deferring it would leave the change unapplied until the next turn
    /// happened to end). It's specifically an IN-FLIGHT turn that must not be
    /// interrupted.
    pub fn request_restart_when_idle(&self) -> bool {
        // The turn-active read and the flag write happen under ONE acquisition
        // of `inner`, and the turn-end consumer clears `active_turn` under that
        // same lock — so the two serialize (codex P2 on PR #2858). Interleaved,
        // this would otherwise observe an active turn, have the consumer run to
        // completion (seeing `restart_when_idle` still false, so restarting
        // nothing), and then set the flag — leaving it stuck until the NEXT
        // turn ended, so the user's next prompt ran with the old config.
        //
        // Lock order is `inner` -> health monitor here and in the consumer;
        // `TurnActivityTracker` never holds its own lock across an `inner`
        // acquisition, so the order can't invert.
        let mut inner = self.inner.lock().unwrap();
        if !self.health_monitor.is_active_turn() {
            return false;
        }
        inner.restart_when_idle = true;
        drop(inner);
        tracing::info!(
            block_id = %self.block_id,
            "runtime-config change arrived mid-turn — deferring the restart to the end of this turn \
             instead of killing the in-flight message"
        );
        true
    }
}

impl Controller for PersistentSubprocessController {
    /// Lazy by default — "spawns on first message" — EXCEPT for a pane
    /// resuming an existing session across a backend restart/reconnect, per
    /// `SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`.
    /// Without this, every persistent agent pane comes back from a crash/
    /// OOM/update as an inert shell — correct layout, correct chrome, `view:
    /// "agent"`, no live process — until a human or another agent happens to
    /// message it. Live incident: 3 of 7 panes in one window silently never
    /// respawned after 2026-09-20's restarts (issue #3463).
    ///
    /// `agent:sessionid` meta present and non-empty is the signal: this pane
    /// has a real, interrupted conversation rather than being genuinely new
    /// (which still spawns lazily, unchanged — no reason to burn a process
    /// before it's needed). Mirrors `ShellController::start()`'s already-
    /// established "transparently revive on restart" behavior for shells.
    ///
    /// **Eager resume is gated on the SAME Layer 3 identity/credential spawn
    /// gate a live message send goes through**
    /// (`identity::resolver::inject_identity_env`,
    /// `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`) — this is not
    /// an optional hardening pass, it is why this method does not simply
    /// spawn unconditionally the moment a session id is found. Skipping the
    /// gate here would let an agent whose bound account was deleted/revoked
    /// between the crash and the restart get silently respawned anyway, on
    /// whatever ambient credential happens to be lying around: exactly the
    /// vulnerability class that gate exists to close, reintroduced through a
    /// call site the gate was never wired into. Any of the following falls
    /// back to today's lazy behavior — NOT a hard error, since a pane that
    /// merely stays inert is still recoverable by a message, unlike one
    /// spawned on the wrong credentials:
    /// - `id_store`/`identity_store`/`auth_key` weren't supplied (i.e. this
    ///   controller was constructed without `with_identity_stores()` —
    ///   should not happen for a production `resync_controller` call, but a
    ///   test or future caller that omits it must not silently spawn
    ///   ungated rather than just not eager-resuming).
    /// - `mstore` is unavailable.
    /// - `cmd`/`cmd:args` meta is missing (should not happen for a pane
    ///   that has ever actually run — `agent_open.rs` always seeds them —
    ///   but guessing CLI flags for a persistent agent is worse than not
    ///   eager-resuming).
    /// - The gate itself returns `Err` (`SpawnGateError`).
    fn start(
        &self,
        block_meta: super::super::obj::MetaMapType,
        _rt_opts: Option<serde_json::Value>,
        _force: bool,
    ) -> Result<(), String> {
        let session_id = crate::backend::obj::meta_get_string(&block_meta, core::META_SESSION_ID, "");
        if session_id.is_empty() {
            tracing::info!(
                block_id = %self.block_id,
                "persistent controller registered (spawns on first message)"
            );
            return Ok(());
        }

        match self.try_eager_resume(&block_meta, &session_id) {
            EagerResumeOutcome::Spawned => {}
            EagerResumeOutcome::DeclinedTo(reason) => {
                tracing::info!(
                    block_id = %self.block_id,
                    session_id = %session_id,
                    reason = %reason,
                    "persistent controller has a prior session but is not eager-resuming it \
                     — registered lazily instead (spawns on next message)"
                );
            }
        }
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

    /// Pane-close shutdown (spec §9.3), built on the Phase 0 measurements
    /// (§5.1): EOF alone does NOT end a running turn — the CLI finishes it
    /// first — but the control-protocol interrupt ends it in ~2s, and EOF
    /// then exits the process in ~0.5s. So: interrupt an active turn, wait
    /// for its result, EOF, and force-kill only at the deadline.
    fn shutdown(&self, deadline: std::time::Instant) -> super::ShutdownFuture {
        use super::StopOutcome;
        const POLL: std::time::Duration = std::time::Duration::from_millis(50);
        let inner = Arc::clone(&self.inner);
        let health = Arc::clone(&self.health_monitor);
        let block_id = self.block_id.clone();
        Box::pin(async move {
            let (generation, stdin_tx) = {
                let mut g = inner.lock().unwrap();
                if g.current_pid.is_none() {
                    // Never spawned (lazy), or already gone. A spawn still in
                    // flight is killed by the caller's tracker drop.
                    return StopOutcome::NotRunning;
                }
                // Nothing queued may start a new turn after the interrupt.
                g.pending_send_messages.clear();
                // Before the interrupt: its `is_error` result is our stop,
                // not a failure (§9.4).
                g.shutdown_generation = Some(g.spawn_generation);
                g.stop_exit = None;
                (g.spawn_generation, g.stdin_tx.clone())
            };

            if health.is_active_turn() {
                if let Some(tx) = stdin_tx.as_ref() {
                    if tx.send(interrupt_control_request_line()).await.is_ok() {
                        // Leave a second for EOF + exit after the turn ends.
                        let turn_deadline = deadline
                            .checked_sub(std::time::Duration::from_secs(1))
                            .unwrap_or(deadline);
                        while health.is_active_turn() && std::time::Instant::now() < turn_deadline {
                            tokio::time::sleep(POLL).await;
                        }
                    }
                }
            }
            // Our clone of the stdin sender would keep stdin open — EOF needs
            // every sender gone.
            drop(stdin_tx);
            let _ = Self::request_stop_on(&inner, KillRequest::Graceful(deadline));

            // The kill arm records the exit the moment it happens; allow a
            // little past the deadline for the forced kill itself.
            let give_up = deadline + std::time::Duration::from_secs(2);
            loop {
                {
                    let g = inner.lock().unwrap();
                    if let Some((gen, killed)) = g.stop_exit {
                        if gen == generation {
                            return if killed { StopOutcome::Killed } else { StopOutcome::Exited };
                        }
                    }
                    // Exited on its own through the natural-exit arm, or was
                    // replaced — either way this process is gone.
                    if g.spawn_generation != generation || g.current_pid.is_none() {
                        return StopOutcome::Exited;
                    }
                }
                if std::time::Instant::now() >= give_up {
                    tracing::warn!(
                        block_id = %block_id,
                        "agent_shutdown: no exit recorded by the deadline; the caller's tracker drop will kill it"
                    );
                    return StopOutcome::Killed;
                }
                tokio::time::sleep(POLL).await;
            }
        })
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

    fn agent_id(&self) -> Option<String> {
        self.agent_id.lock().unwrap().clone()
    }

    fn set_agent_id(&self, id: Option<String>) {
        *self.agent_id.lock().unwrap() = id;
    }

    fn stable_agent_id(&self) -> Option<String> {
        self.stable_agent_id.lock().unwrap().clone()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Classify a flushed error line, if it's confident enough to surface to
/// the pane's failure-recovery UI. `line` is usually the same result-frame
/// JSON the mid-stream error-result path handles, but
/// `persistent_resume::ResumeEffect::FlushErrorLine` can also carry an
/// earlier held-back turn's line — falls back to raw-text keyword matching
/// (the same `classify()` uses for a stderr tail) if it doesn't parse as
/// JSON. `exit_code` is `Some` at the process-exit arm and `None` at the
/// mid-generation flush sites (a superseded spawn generation, or a
/// `SessionCaptured` resolution), where no fresh exit code exists for the
/// line being flushed. Returns `None` for an unrecognized, non-retryable
/// `FailureClass::UnknownNonZero` to avoid a low-confidence banner from
/// noisy flushed text — every other recognized class is surfaced, not just
/// the retryable ones, since the recovery banner has value for e.g. `Auth`
/// too (see `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md`'s per-class
/// action matrix), just without auto-retry.
/// See `SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md`.
fn classify_exit_line(exit_code: Option<i32>, line: &str) -> Option<crate::agents::failure::AgentFailure> {
    let parsed_line: Option<serde_json::Value> = serde_json::from_str(line).ok();
    let stderr_text = if parsed_line.is_none() { line } else { "" };
    let failure = crate::agents::failure::classify(exit_code, None, stderr_text, parsed_line.as_ref());
    if failure.retryable || failure.code != crate::agents::failure::FailureClass::UnknownNonZero {
        Some(failure)
    } else {
        None
    }
}

#[cfg(test)]
mod classify_exit_line_tests {
    use super::*;
    use crate::agents::failure::FailureClass;

    #[test]
    fn json_result_frame_with_overloaded_text_is_surfaced() {
        let line = r#"{"type":"result","is_error":true,"result":"Overloaded"}"#;
        let failure = classify_exit_line(Some(1), line).expect("overloaded should surface");
        assert_eq!(failure.code, FailureClass::Overloaded);
        assert!(failure.retryable);
    }

    #[test]
    fn json_result_frame_with_rate_limit_text_is_surfaced() {
        let line = r#"{"type":"result","is_error":true,"result":"429 rate limited, please retry"}"#;
        let failure = classify_exit_line(Some(1), line).expect("rate-limited should surface");
        assert_eq!(failure.code, FailureClass::RateLimited);
        assert!(failure.retryable);
    }

    #[test]
    fn non_json_raw_text_falls_back_to_keyword_matching() {
        // FlushErrorLine can carry an earlier held-back turn's raw line,
        // not guaranteed to be well-formed JSON — the fallback path must
        // still classify it from the raw text.
        let line = "connection error: rate limited (429)";
        let failure = classify_exit_line(Some(1), line).expect("raw-text 429 should surface");
        assert_eq!(failure.code, FailureClass::RateLimited);
    }

    #[test]
    fn unrecognized_json_error_is_suppressed() {
        // A real error frame, but with no keyword classify() recognizes —
        // must not produce a low-confidence UnknownNonZero banner.
        let line = r#"{"type":"result","is_error":true,"result":"something unusual happened"}"#;
        assert_eq!(classify_exit_line(Some(1), line), None);
    }

    #[test]
    fn unrecognized_raw_text_is_suppressed() {
        let line = "some unrelated noise flushed from an earlier turn";
        assert_eq!(classify_exit_line(Some(1), line), None);
    }

    #[test]
    fn auth_error_is_surfaced_even_though_not_retryable() {
        // Non-retryable classes still have recovery-banner value (Login
        // Again / Armory actions) — only UnknownNonZero is suppressed.
        let line = r#"{"type":"result","is_error":true,"result":"invalid api key (401)"}"#;
        let failure = classify_exit_line(Some(1), line).expect("auth errors should still surface");
        assert_eq!(failure.code, FailureClass::Auth);
        assert!(!failure.retryable);
    }

    #[test]
    fn no_exit_code_still_classifies_from_line_content() {
        // Mid-generation flush sites (a superseded spawn generation, a
        // SessionCaptured resolution) have no fresh exit code for the line
        // being flushed — classification must still work from content alone.
        let line = r#"{"type":"result","is_error":true,"result":"Overloaded"}"#;
        let failure = classify_exit_line(None, line).expect("overloaded should surface without an exit code");
        assert_eq!(failure.code, FailureClass::Overloaded);
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

    // ── Deferred runtime-config restart (AgentX, 2026-08-28) ────────────
    //
    // A `/model` change mid-turn used to kill the CLI with the user's message
    // already on its stdin: the retry was suppressed (StopRequested reads as a
    // user stop), the queue died with the discarded controller, and the
    // replacement only "spawns on first message". The turn vanished silently.

    /// Mid-turn: the caller must be told to leave the controller alone, and
    /// the intent must be recorded for the turn-end hook to act on.
    #[test]
    fn request_restart_when_idle_defers_while_a_turn_is_in_flight() {
        let c = controller();
        c.health_monitor.set_active_turn(true);

        assert!(
            c.request_restart_when_idle(),
            "a turn is in flight — the caller must NOT replace the controller",
        );
        assert!(
            c.inner.lock().unwrap().restart_when_idle,
            "the deferred restart must be recorded for the turn-end hook",
        );
    }

    /// Idle: an immediate replace is safe and is what should happen — this is
    /// the ordinary `/model` case. Deferring here would strand the change
    /// until some future turn happened to end.
    #[test]
    fn request_restart_when_idle_does_not_defer_on_an_idle_pane() {
        let c = controller();
        c.health_monitor.set_active_turn(false);

        assert!(
            !c.request_restart_when_idle(),
            "no turn in flight — the caller should replace the controller immediately",
        );
        assert!(
            !c.inner.lock().unwrap().restart_when_idle,
            "nothing to defer, so nothing should be recorded",
        );
    }

    /// The flag is consumed, not merely read: the turn-end hook uses
    /// `mem::replace`, so a single deferred change causes exactly one restart
    /// rather than one at the end of every subsequent turn.
    #[test]
    fn the_deferred_restart_flag_is_consumed_exactly_once() {
        let c = controller();
        c.health_monitor.set_active_turn(true);
        c.request_restart_when_idle();

        let first = std::mem::replace(&mut c.inner.lock().unwrap().restart_when_idle, false);
        let second = std::mem::replace(&mut c.inner.lock().unwrap().restart_when_idle, false);
        assert!(first, "the first turn-end must see the deferred restart");
        assert!(!second, "a later turn-end must not restart again");
    }

    /// Repeated changes mid-turn (a user flipping model then effort) collapse
    /// into one restart, not a queue of them.
    #[test]
    fn repeated_mid_turn_changes_collapse_into_one_restart() {
        let c = controller();
        c.health_monitor.set_active_turn(true);
        assert!(c.request_restart_when_idle());
        assert!(c.request_restart_when_idle());
        assert!(c.request_restart_when_idle());

        assert!(std::mem::replace(&mut c.inner.lock().unwrap().restart_when_idle, false));
        assert!(!c.inner.lock().unwrap().restart_when_idle);
    }

    /// reagent P1 on PR #2858: `restart_when_idle` is consumed at the
    /// `is_result_frame` turn end, but a generation that dies abnormally
    /// instead — user Stop/SIGINT, a crash, a permanently-failed turn
    /// resolving via `ProcessExited` + `PublishDone` — never reaches that
    /// branch. A leaked `true` would ride into an unrelated later generation
    /// and kill a healthy process at the end of some future turn. Scoping the
    /// flag per-spawn is what prevents that.
    #[test]
    fn a_deferred_restart_does_not_leak_into_the_next_generation() {
        let c = controller();
        c.health_monitor.set_active_turn(true);
        assert!(c.request_restart_when_idle());
        assert!(c.inner.lock().unwrap().restart_when_idle);

        // The generation dies WITHOUT a result frame (crash / user stop), so
        // nothing consumed the flag. The next spawn must start clean —
        // emulating spawn_process's own clear, which happens in the same
        // acquisition that bumps the generation.
        {
            let mut inner = c.inner.lock().unwrap();
            inner.restart_pending = false;
            inner.restart_when_idle = false;
            inner.spawn_generation += 1;
        }
        assert!(
            !c.inner.lock().unwrap().restart_when_idle,
            "a new generation must not inherit the previous one's deferred restart",
        );
    }

    /// codex P1 on PR #2858: `stop_process` only sends on `kill_tx` — it
    /// leaves `stdin_tx` live until the process actually exits. A follow-up
    /// arriving in that window (and turn end is EXACTLY when the frontend
    /// flushes queued follow-ups) would take `DeliverDirect` into a process
    /// about to receive EOF: acknowledged, then lost.
    // ---- needs_spawn: the reactive-delivery routing predicate ----
    // REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03.md

    #[test]
    fn needs_spawn_is_true_for_a_freshly_registered_controller() {
        // The state every persistent controller sits in after an srv restart:
        // registered ("spawns on first message"), no process yet. This is the
        // case that used to fail agent-to-agent delivery permanently.
        let c = controller();
        assert!(c.needs_spawn());
    }

    #[test]
    fn needs_spawn_is_false_once_a_process_is_live() {
        // A live process can be steered mid-turn by the ordinary delivery path;
        // starting a second turn here would be wrong.
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);
        assert!(!c.needs_spawn());
    }

    #[test]
    fn needs_spawn_is_false_while_a_spawn_is_already_in_flight() {
        // The load-bearing case. A caller that has claimed the spawn owns this
        // round; reporting "needs spawn" here would invite a second, racing
        // spawn — the orphaned-child bug `spawning_in_progress` exists to
        // prevent. This window is retryable ("still starting up"), not
        // spawnable.
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = None;
            inner.spawning_in_progress = true;
        }
        assert!(
            !c.needs_spawn(),
            "must not invite a second spawn while one is already claimed",
        );
    }

    #[test]
    fn needs_spawn_is_false_while_a_retry_flush_drain_holds_its_claim() {
        // reagent P1 on PR #2960. When a RetryFlush drain's target process dies
        // mid-flush, the drain DELIBERATELY retains `drain_claim` for the
        // fallback respawn while the exit handler clears `stdin_tx`. Without
        // the `!drain_claim` term this window reported "needs spawn"; the
        // reactive path then called send_message, decide_send_action returned
        // Queued on the still-held claim, send_message returned Ok(()) having
        // delivered nothing, and the caller was told the message landed.
        // cloud_subscriber only retries on !success — so that is a lost message.
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = None;
            inner.spawning_in_progress = false;
            inner.drain_claim = true;
        }
        assert!(
            !c.needs_spawn(),
            "a held drain claim must not be reported as spawnable — send_message would only queue",
        );
    }

    #[test]
    fn needs_spawn_true_implies_decide_send_action_would_actually_spawn() {
        // The invariant, asserted directly rather than trusted: whenever
        // needs_spawn() is true, a real send must take the BecomeSpawner branch
        // (which spawns and delivers) and never Queued (which returns Ok having
        // delivered nothing). Covers all four flag combinations.
        for (stdin, spawning, drain) in [
            (false, false, false), // the only spawnable state
            (false, false, true),
            (false, true, false),
            (true, false, false),
        ] {
            let c = controller();
            let (tx, _rx) = mpsc::channel::<String>(4);
            {
                let mut inner = c.inner.lock().unwrap();
                inner.stdin_tx = if stdin { Some(tx) } else { None };
                inner.spawning_in_progress = spawning;
                inner.drain_claim = drain;
            }
            if c.needs_spawn() {
                assert!(
                    matches!(c.decide_send_action("m", None), SendAction::BecomeSpawner { .. }),
                    "needs_spawn() was true for (stdin={stdin}, spawning={spawning}, drain={drain}) \
                     but a real send would not have spawned",
                );
            }
        }
    }

    #[test]
    fn a_committed_restart_refuses_deliver_direct_even_with_a_live_stdin() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            // Sanity: without the restart commit this is DeliverDirect — so
            // the assertion below is about `restart_pending`, not about some
            // other precondition failing.
            assert!(inner.stdin_tx.is_some());
        }
        assert!(matches!(c.decide_send_action("m1", None), SendAction::DeliverDirect));

        c.inner.lock().unwrap().restart_pending = true;
        assert!(
            !matches!(c.decide_send_action("m2", None), SendAction::DeliverDirect),
            "a message must not be written into a process that is being killed",
        );
    }

    /// …and the message is not dropped either — it queues for the replacement.
    #[test]
    fn a_message_arriving_during_the_quiesce_window_is_queued_not_lost() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = Some(tx);
            inner.restart_pending = true;
        }
        let action = c.decide_send_action("m1", None);
        assert!(
            matches!(action, SendAction::BecomeSpawner { .. } | SendAction::Queued),
            "must fall through to the spawn/queue path, not vanish",
        );
        assert_eq!(
            c.inner.lock().unwrap().pending_send_messages.len(),
            1,
            "the message must be retained for the replacement process",
        );
    }

    /// The quiesce window ends when the replacement arrives — otherwise every
    /// later send would keep queueing behind a flag nothing clears.
    #[test]
    fn the_quiesce_flag_does_not_outlive_the_restart() {
        let c = controller();
        c.inner.lock().unwrap().restart_pending = true;
        // spawn_process clears it in the same acquisition that bumps the
        // generation; emulate that contract here (spawning a real process in
        // a unit test isn't practical).
        {
            let mut inner = c.inner.lock().unwrap();
            inner.restart_pending = false;
            inner.spawn_generation += 1;
        }
        let (tx, _rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);
        assert!(
            matches!(c.decide_send_action("m1", None), SendAction::DeliverDirect),
            "once the replacement is up, ordinary direct delivery resumes",
        );
    }

    /// Shorthand for a retry-batch entry with an explicit queue seq
    /// (issue #2365 — retry batches carry identity, not just text).
    fn qentry(seq: u64, json: &str) -> persistent_resume::QueuedRetryEntry {
        persistent_resume::QueuedRetryEntry { seq, json: json.to_string() }
    }

    /// `has_prior_transcript` gates the fresh-start disclosure, so a false
    /// positive would stamp "New session started" onto a brand-new agent's
    /// very first turn — and, worse, clamp its scrollback against a boundary
    /// that has nothing before it. With no filestore and no store there is
    /// provably no history, and it must say so rather than defaulting to
    /// "assume there might be".
    #[test]
    fn has_prior_transcript_is_false_without_any_backing_store() {
        assert!(!controller().has_prior_transcript());
    }

    /// codex P1 on PR #2500 (second round): the fresh-start clear must
    /// retire every existing generation IN THE SAME lock acquisition —
    /// a bare `session_id = None` left the dying generation still equal
    /// to `spawn_generation` until `spawn_process`'s own (later) bump,
    /// so its stdout reader's stale echo passed the #2366 currency gate
    /// during exactly the window where `spawn_process` reads
    /// `session_id` for the `--resume` decision.
    #[test]
    fn fresh_spawn_clear_makes_the_dying_generations_capture_stale_immediately() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawn_generation = 1;
            inner.session_id = Some("stale-sid".to_string());
        }

        // The fallback/retry path clears — BEFORE any new spawn exists.
        c.clear_session_id_for_fresh_spawn();

        // The gen-1 reader's buffered echo lands in the pre-spawn window.
        let mut inner = c.inner.lock().unwrap();
        let (adopted, _) = inner.try_capture_session_id("stale-sid", 1, false);

        assert!(
            !adopted,
            "the dying generation must be stale from the instant of the clear, \
             not only after spawn_process's own later bump"
        );
        assert_eq!(inner.session_id, None, "the fresh spawn must not see a --resume sid");
        assert_eq!(inner.spawn_generation, 2, "the clear reserves the next generation");
    }

    // codex P1 on PR #2360 (round 16, commit ce1642d90): `stop_process`
    // must record which generation a stop was requested for even when
    // there's no live `kill_tx` to send through — see
    // `persistent_resume::ResumeEvent::StopRequested`'s own doc comment
    // for why a `kill_tx` send alone can't be trusted (the process-
    // waiter's `tokio::select!` can have already committed to its exit
    // branch before this call reaches the lock, making the send futile
    // even when `kill_tx` was still `Some`). Simulates the exact race
    // here via no `kill_tx` at all (the narrower, always-reachable
    // sub-case), with a resume attempt already in flight so the
    // recorded stop has something to override.
    #[test]
    fn stop_process_records_the_current_generation_even_with_no_live_kill_tx() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawn_generation = 3;
            inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                generation: 3,
                attempted_sid: "dead-sid".to_string(),
                retry: persistent_resume::RetryPayload {
                    config: PersistentSpawnConfig {
                        cli_command: "claude".to_string(),
                        cli_args: vec![],
                        working_dir: String::new(),
                        env_vars: HashMap::new(),
                        session_id_field: "session_id".to_string(),
                        resume_flag: "--resume".to_string(),
                        session_id: "dead-sid".to_string(),
                        message_id: None,
                    },
                    messages: vec![qentry(1, "{}")],
                },
            });
        }
        // kill_tx stays None — the process already exited (or never
        // started); stop_process must still succeed and record intent.
        let result = c.stop_process(false);
        assert!(result.is_ok(), "must still return Ok when there's nothing live to signal");
        let resume_state = c.inner.lock().unwrap().resume.clone();
        match resume_state {
            persistent_resume::ResumeState::AwaitingOutcome { stop_requested, .. } => {
                assert!(
                    stop_requested,
                    "must record a signal the exit-handler can check even with no live kill_tx to send through"
                );
            }
            other => panic!("expected AwaitingOutcome with stop_requested, got {other:?}"),
        }
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

    // deny_question shares the identical pending_questions lookup as
    // answer_question, so the frontend's SAFE_TO_RETRY_VIA_FOLLOWUP allowlist
    // (useAgentQuestions.ts) matches this error text too — keep the "no
    // pending AskUserQuestion" prefix stable if this message ever changes.
    #[test]
    fn deny_question_on_untracked_tool_use_id_is_descriptive() {
        let c = controller();
        let err = c
            .deny_question("tu-unknown".to_string(), "declined".to_string())
            .unwrap_err();
        assert!(
            err.contains("tu-unknown") && err.contains("respawned"),
            "error should name the tool_use_id and explain the likely cause, got {err:?}"
        );
    }

    // The dead-air fallback for a decline must still tell the model to resume
    // (not wait for further input it will never get) while making clear the
    // outcome was a DECLINE, not a real answer — otherwise the model could
    // hallucinate a value the user never provided.
    // Pins the exact literal (not just a substring) so an edit to this
    // constant is impossible to make silently — see the KEEP IN SYNC comment
    // on ASK_USER_QUESTION_DENY_MESSAGE's own definition. There is no way for
    // this test to reach across the Rust/TypeScript boundary and check
    // useAgentQuestions.ts's CANCEL_FALLBACK_MESSAGE directly; failing loudly
    // here is what prompts a human/reviewer to go update that copy too.
    #[test]
    fn ask_user_question_deny_message_matches_frontend_cancel_fallback_text() {
        assert_eq!(
            ASK_USER_QUESTION_DENY_MESSAGE,
            "The user declined to answer this question.",
            "this literal is hand-mirrored as CANCEL_FALLBACK_MESSAGE in \
             frontend/app/view/agent/hooks/useAgentQuestions.ts — update both \
             together"
        );
    }

    #[test]
    fn deny_resume_message_is_directive_and_includes_the_reason() {
        let msg = build_deny_resume_message(ASK_USER_QUESTION_DENY_MESSAGE);
        assert!(msg.contains("Resume the task"), "must be directive: {msg}");
        assert!(
            msg.contains("declined to answer"),
            "must carry the decline reason: {msg}"
        );
    }

    // ── Phase 2 gate: tool-permission parking + decision (SPEC_DECISION_PROMPT_2026_04_24.md) ──
    //
    // should_route_to_decision_panel is hardcoded false in production, so
    // park_tool_permission_request/decide_tool_permission are unreachable
    // from handle_control_frame today. These tests call them DIRECTLY
    // (both are in scope via `use super::*;`), independent of that gate —
    // exercising the mechanism without relying on, or changing, today's
    // default auto-allow behavior.

    // Pins the gate itself: whatever tool name is asked about, today's
    // answer must be "auto-allow", not "prompt". This is the test that
    // would need to change (deliberately, not by accident) the day this
    // gate is flipped on for real.
    #[test]
    fn should_route_to_decision_panel_is_inert_for_any_tool_today() {
        for tool in ["Bash", "Edit", "Write", "WebFetch", "AskUserQuestion", ""] {
            assert!(
                !should_route_to_decision_panel(tool),
                "gate must stay false for {tool:?} until the Phase 2 policy decision is made"
            );
        }
    }

    // decide_tool_permission spawns the dead-air fallback task
    // (tokio::spawn) as part of every call, same as answer_question/
    // deny_question — needs a running runtime even though this test never
    // waits out that 4s fallback itself (this crate doesn't enable tokio's
    // test-util feature; see status_heartbeat_republishes_while_active's own
    // comment above for why a virtual clock isn't available here).
    #[tokio::test]
    async fn parking_then_deciding_allow_sends_the_original_input_back_unmodified() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        park_tool_permission_request(
            &c.inner,
            "tu-1".to_string(),
            "req-1".to_string(),
            "Bash".to_string(),
            serde_json::json!({ "command": "ls" }),
        );

        c.decide_tool_permission("tu-1".to_string(), "allow", None)
            .expect("a freshly-parked request must be decidable");

        let sent = rx.try_recv().expect("decide_tool_permission must write to stdin");
        let parsed: serde_json::Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(parsed["type"], "control_response");
        assert_eq!(parsed["response"]["request_id"], "req-1");
        let inner_resp = &parsed["response"]["response"];
        assert_eq!(inner_resp["behavior"], "allow");
        assert_eq!(inner_resp["toolUseID"], "tu-1");
        // The original input comes back byte-for-byte — this method has no
        // "edit before approving" UI (none exists yet).
        assert_eq!(inner_resp["updatedInput"], serde_json::json!({ "command": "ls" }));

        // Consumed on decide — a second decide on the same id must fail,
        // same lifecycle as answer_question/deny_question.
        assert!(c.decide_tool_permission("tu-1".to_string(), "allow", None).is_err());
    }

    #[tokio::test]
    async fn parking_then_deciding_deny_carries_user_feedback_verbatim() {
        // SPEC_DECISION_PROMPT_2026_04_24.md G6: "Denials carry user-typed
        // feedback verbatim to the agent."
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        park_tool_permission_request(
            &c.inner,
            "tu-2".to_string(),
            "req-2".to_string(),
            "Bash".to_string(),
            serde_json::json!({ "command": "rm -rf /tmp/x" }),
        );

        c.decide_tool_permission("tu-2".to_string(), "deny", Some("too risky, use trash instead".to_string()))
            .expect("a freshly-parked request must be decidable");

        let sent = rx.try_recv().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sent).unwrap();
        let inner_resp = &parsed["response"]["response"];
        assert_eq!(inner_resp["behavior"], "deny");
        assert_eq!(inner_resp["message"], "too risky, use trash instead");
        assert_eq!(inner_resp["toolUseID"], "tu-2");
    }

    #[tokio::test]
    async fn deciding_deny_with_no_feedback_still_tells_the_model_it_was_refused() {
        let c = controller();
        let (tx, mut rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        park_tool_permission_request(
            &c.inner,
            "tu-3".to_string(),
            "req-3".to_string(),
            "Write".to_string(),
            serde_json::json!({}),
        );

        c.decide_tool_permission("tu-3".to_string(), "deny", None).unwrap();

        let sent = rx.try_recv().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(parsed["response"]["response"]["message"], "Denied by user.");
    }

    /// Same shape as answer_question_on_untracked_tool_use_id_is_descriptive
    /// below — a fresh controller instance (pane reopen, any process
    /// respawn) has an empty pending_permissions map.
    #[test]
    fn decide_tool_permission_on_untracked_tool_use_id_is_descriptive() {
        let c = controller();
        let err = c
            .decide_tool_permission("tu-unknown".to_string(), "allow", None)
            .unwrap_err();
        assert!(
            err.contains("tu-unknown") && err.contains("respawned"),
            "error should name the tool_use_id and explain the likely cause, got {err:?}"
        );
    }

    #[test]
    fn tool_decision_resume_message_names_the_tool_and_the_outcome() {
        let allow_msg = build_tool_decision_resume_message("Bash", "allow", None);
        assert!(allow_msg.contains("Bash"));
        assert!(allow_msg.contains("approved"));
        assert!(allow_msg.contains("Resume the task"));

        let deny_msg = build_tool_decision_resume_message("Write", "deny", Some("no"));
        assert!(deny_msg.contains("Write"));
        assert!(deny_msg.contains("declined"));
        assert!(deny_msg.contains("\"no\""));
    }

    // `turn_active` on the runtime status snapshot must track the health
    // monitor's active-turn flag directly — this is the signal the frontend
    // seeds TurnPhase from at mount instead of always defaulting to Idle
    // (see docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    // Finding 1). Exercised here via the health monitor directly rather than
    // send_message()/the stdout reader, which both require a real spawned
    // process.
    #[test]
    fn status_snapshot_turn_active_tracks_turn_activity_tracker() {
        let c = controller();
        assert!(
            !c.get_status_snapshot().turn_active,
            "freshly constructed controller has no turn in flight"
        );

        c.health_monitor.set_active_turn(true);
        assert!(
            c.get_status_snapshot().turn_active,
            "turn_active must flip true once the turn-activity tracker marks a turn active"
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
        let broker = Arc::new(crate::backend::mps::Broker::new());
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
            crate::backend::mps::EVENT_CONTROLLER_STATUS,
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
        let broker = Arc::new(crate::backend::mps::Broker::new());
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
            crate::backend::mps::EVENT_CONTROLLER_STATUS,
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

        assert_eq!(
            c.inner.lock().unwrap().session_id,
            None,
            "must clear inner.session_id directly, not rely on poison_resume having already done so"
        );
    }

    /// Regression test for
    /// `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`:
    /// a confirmed-stale `--resume` must try to recover the largest REAL
    /// session on disk before giving up and starting blank.
    #[test]
    fn find_recovery_session_id_recovers_the_largest_on_disk_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_string_lossy().to_string();
        let working_dir = r"C:\Users\asafe\.agentmux\agents\agentx-0623n".to_string();
        let slug = crate::backend::session_backfill::encode_project_slug(&working_dir);
        let dir = tmp.path().join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

        let c = controller();
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
        let config = PersistentSpawnConfig {
            cli_command: "claude".to_string(),
            cli_args: vec![],
            working_dir,
            env_vars,
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "d019e2e4-stale".to_string(),
            message_id: None,
        };

        assert_eq!(
            c.find_recovery_session_id(&config).as_deref(),
            Some("972a6a4f-live"),
            "must recover the largest real on-disk session, not give up"
        );
    }

    #[test]
    fn find_recovery_session_id_is_none_without_a_config_dir() {
        let c = controller();
        let config = PersistentSpawnConfig {
            cli_command: "claude".to_string(),
            cli_args: vec![],
            working_dir: "/wherever".to_string(),
            env_vars: HashMap::new(), // no CLAUDE_CONFIG_DIR — same as the pre-fix world
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        };
        assert_eq!(c.find_recovery_session_id(&config), None);
    }

    /// Guards against an infinite retry loop: if the ONLY candidate
    /// recovery finds is the exact id already confirmed poisoned this
    /// attempt, don't recover it again — nothing on disk changes between
    /// attempts, so an unguarded recovery would rediscover the identical
    /// dead id forever.
    #[test]
    fn find_recovery_session_id_refuses_an_already_poisoned_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_string_lossy().to_string();
        let working_dir = "/agents/agentx".to_string();
        let slug = crate::backend::session_backfill::encode_project_slug(&working_dir);
        let dir = tmp.path().join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dead-sid.jsonl"), vec![b'x'; 1_000]).unwrap();

        let c = controller();
        c.inner.lock().unwrap().resume_poisoned = Some("dead-sid".to_string());
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
        let config = PersistentSpawnConfig {
            cli_command: "claude".to_string(),
            cli_args: vec![],
            working_dir,
            env_vars,
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        };

        assert_eq!(
            c.find_recovery_session_id(&config),
            None,
            "must not re-recover the same id already confirmed dead this attempt"
        );
    }

    /// End-to-end through `retry_after_resume_failure`: when a real,
    /// larger session exists on disk under this spawn's own
    /// `CLAUDE_CONFIG_DIR`, the respawn must hydrate `inner.session_id` to
    /// THAT recovered id — not fall back to a blank conversation — even
    /// though the actual process spawn itself fails fast here (nonexistent
    /// binary), mirroring
    /// `retry_after_resume_failure_clears_inner_session_id_even_when_poison_resume_has_not_run_yet`
    /// above but for the recovery path instead of the give-up path.
    #[test]
    fn retry_after_resume_failure_hydrates_inner_session_id_from_the_recovered_session() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_string_lossy().to_string();
        let working_dir = r"C:\Users\asafe\.agentmux\agents\agentx-0623n".to_string();
        let slug = crate::backend::session_backfill::encode_project_slug(&working_dir);
        let dir = tmp.path().join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

        let c = controller();
        c.inner.lock().unwrap().session_id = Some("d019e2e4-stale".to_string());
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir,
            env_vars,
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "d019e2e4-stale".to_string(),
            message_id: None,
        };

        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

        assert_eq!(
            c.inner.lock().unwrap().session_id,
            Some("972a6a4f-live".to_string()),
            "must resume the recovered real session, not silently start blank"
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
        let generation = inner.spawn_generation;
        let (captured, _effects) = inner.try_capture_session_id("valid-unrelated-sid", generation, true);
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
        let action = c.decide_send_action("msg-a", None);
        assert!(matches!(action, SendAction::BecomeSpawner { .. }));
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

        let action = c.decide_send_action("msg-b", None);
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

        c.decide_send_action("hello", None);
        let action = c.decide_send_action("hello", None);

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
    /// re-delivery of a message that may ALREADY be sitting in the queue —
    /// pushed by the very spawn attempt whose failure triggered this
    /// retry, if that spawn's own drain hasn't reached it yet. Blindly
    /// queueing another copy (as the `None`/`send_message` path
    /// correctly does for a genuine new message) would let a fallback
    /// spawn eventually deliver the same prompt twice. Passing the
    /// original entry's seq must therefore skip re-enqueueing while that
    /// exact entry is still present (issue #2365: matched by identity,
    /// not text).
    #[test]
    fn decide_send_action_dedups_a_known_retry_of_an_already_queued_message() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            inner
                .pending_send_messages
                .push_back(QueuedMessage::fresh(7, "original-payload".to_string()));
        }

        let action = c.decide_send_action("original-payload", Some(7));

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            1,
            "a retry of a message still queued under its own seq must not add a duplicate copy"
        );
    }

    /// Issue #2365 regression: the dedup must key on the entry's seq, not
    /// its text — a DIFFERENT message that happens to share identical
    /// content with the retried one must not satisfy the check (the old
    /// content-equality version silently dropped the retry here).
    #[test]
    fn decide_send_action_does_not_dedup_a_retry_against_an_identical_text_different_message() {
        let c = controller();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            // A genuinely different message (seq 9) with the same text as
            // the retried entry (seq 7).
            inner
                .pending_send_messages
                .push_back(QueuedMessage::fresh(9, "original-payload".to_string()));
        }

        let action = c.decide_send_action("original-payload", Some(7));

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.len(),
            2,
            "identical text under a different seq is a different message — the retry must still be queued"
        );
        assert_eq!(
            inner.pending_send_messages[1].seq, 7,
            "the re-queued retry must keep its original seq, not draw a fresh one"
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

        let action = c.decide_send_action("not-yet-queued", Some(42));

        assert!(matches!(action, SendAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(inner.pending_send_messages.len(), 1);
        assert_eq!(inner.pending_send_messages[0], "not-yet-queued");
        assert_eq!(
            inner.pending_send_messages[0].seq, 42,
            "a re-queued known redelivery must preserve its original seq"
        );
    }

    #[test]
    fn decide_send_action_delivers_directly_when_already_running() {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        c.inner.lock().unwrap().stdin_tx = Some(tx);

        let action = c.decide_send_action("msg-c", None);
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

        let action = c.decide_send_action("msg-late-arrival", None);
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
        c.decide_send_action("hello", None);
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
        c.decide_retry_batch_action(1, &qentry(1, "hello"), &[qentry(2, "world")]);
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
                std::thread::spawn(move || match c.decide_send_action(&format!("msg-{i}"), None) {
                    SendAction::BecomeSpawner { .. } => {
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "second".to_string()));
        }

        // Never used for a fallback spawn in this test — the drain fully
        // succeeds without ever stalling. `own_message` is only consulted
        // on the failed-spawn path, so its value doesn't matter here.
        c.release_spawn_claim_and_drain_queue(true, unreachable_fallback_config(), 0);

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
        let broker = Arc::new(crate::backend::mps::Broker::new());
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "stuck-one".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "stuck-two".to_string()));
            // stdin_tx stays None — simulates the process this claim was
            // spawning for having already died before the drain ran.
        }

        c.release_spawn_claim_and_drain_queue(true, unreachable_fallback_config(), 0);
        // Let the spawned background task, and the fallback respawn
        // attempt it triggers, run to completion.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let history = broker.read_event_history(
            crate::backend::mps::EVENT_CONTROLLER_STATUS,
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "the-one-that-failed".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "queued-by-someone-else".to_string()));
        }

        c.release_spawn_claim_and_drain_queue(false, unreachable_fallback_config(), 1);

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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "older-leftover-from-a-different-caller".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "this-spawners-own-message-that-just-failed".to_string()));
        }

        c.release_spawn_claim_and_drain_queue(
            false,
            unreachable_fallback_config(),
            2,
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
            let broker = StdArc::new(crate::backend::mps::Broker::new());
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
                inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "the-one-that-failed".to_string()));
            }

            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let c = StdArc::clone(&c);
                    std::thread::spawn(move || {
                        let _ = c.decide_send_action(&format!("racer-{iteration}-{i}"), None);
                    })
                })
                .collect();

            c.release_spawn_claim_and_drain_queue(false, unreachable_fallback_config(), 1);

            for h in handles {
                h.join().unwrap();
            }

            let inner = c.inner.lock().unwrap();
            let stranded = !inner.spawning_in_progress && !inner.pending_send_messages.is_empty();
            if stranded {
                let published = !broker
                    .read_event_history(crate::backend::mps::EVENT_CONTROLLER_STATUS, &format!("block:{block_id}"), 10)
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

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
            1,
            config,
            vec![qentry(1, "msg-1"), qentry(2, "msg-2"), qentry(3, "msg-3")],
            None,
            "dead-sid".to_string(),
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], None, "dead-sid".to_string());

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
            1,
            config,
            vec![qentry(1, "msg-1"), qentry(2, "msg-2"), qentry(3, "msg-3")],
            None,
            "dead-sid".to_string(),
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

    /// Issue #2367 (supersedes the round-7 spawn-flag borrow): a retry
    /// batch targeting a live process must take the DRAIN claim in the
    /// same lock acquisition that decides the flush — so no concurrent
    /// caller bypasses the queue via `DeliverDirect` — while leaving
    /// `spawning_in_progress` untouched (round 14's starvation analysis:
    /// it is never pre-asserted while no spawn is in progress). Even
    /// against a dead sender the flush must still release the claim
    /// afterward (not leave it stuck forever) while preserving the
    /// message for a future spawn.
    #[tokio::test]
    async fn retry_after_resume_failure_takes_the_drain_claim_not_the_spawn_flag() {
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], None, "dead-sid".to_string());

        {
            let inner = c.inner.lock().unwrap();
            assert!(
                inner.drain_claim,
                "must take the drain claim immediately so no concurrent caller bypasses the queue via DeliverDirect"
            );
            assert!(
                !inner.spawning_in_progress,
                "the spawn flag must never be borrowed for a non-spawn (round 14 starvation analysis)"
            );
        }

        // Let the background drain (which will also fail against this
        // same dead sender) run to completion.
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let inner = c.inner.lock().unwrap();
        assert!(!inner.drain_claim, "the drain claim must eventually be released, not stuck forever");
        assert_eq!(inner.pending_send_messages.len(), 1, "message must remain queued, not lost");
    }

    /// Issue #2367, the actual reordering bug: while the retry batch is
    /// being flushed into a live newer process, a concurrent
    /// `send_message` must route through the queue BEHIND the batch —
    /// under the old `try_send` design it took `DeliverDirect` and could
    /// jump ahead of (or into the middle of) earlier-accepted retry
    /// messages. The queue is now the single ordering authority: batch
    /// first, later send after, and the claim releases once dry.
    #[tokio::test]
    async fn retry_flush_orders_a_concurrent_send_behind_the_whole_batch() {
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
            1,
            config,
            vec![qentry(1, "batch-1"), qentry(2, "batch-2")],
            None,
            "dead-sid".to_string(),
        );

        // Races in while the flush task holds the drain claim: must be
        // queued behind the batch, never delivered directly.
        let action = c.decide_send_action("later-send", None);
        assert!(
            matches!(action, SendAction::Queued),
            "a send during a retry flush must queue behind the batch, not DeliverDirect ahead of it"
        );

        assert_eq!(rx.recv().await.unwrap(), "batch-1");
        assert_eq!(rx.recv().await.unwrap(), "batch-2");
        assert_eq!(
            rx.recv().await.unwrap(),
            "later-send",
            "the concurrent send must arrive strictly after the whole batch"
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let inner = c.inner.lock().unwrap();
        assert!(!inner.drain_claim, "claim must be released once the queue runs dry");
        assert!(inner.pending_send_messages.is_empty());
    }

    /// codex P2 on PR #2371: a confirmed retry that turns out NOT to
    /// actually launch (here, `spawn_process` failing against a
    /// nonexistent binary — the `BecomeSpawner` path's own failure case)
    /// must flush a held-back error line instead of silently dropping
    /// it. Without this, an already-accepted prompt whose stale-resume
    /// retry then ALSO fails to spawn would end in total silence:
    /// neither the original error nor a replacement one.
    #[test]
    fn retry_after_resume_failure_flushes_the_held_error_line_when_the_respawn_itself_fails() {
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-flush-on-failed-retry".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker),
            None,
            None,
            Some(filestore.clone()),
        );

        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], Some("boom\n".to_string()), "dead-sid".to_string());

        let flushed = filestore
            .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
            .unwrap()
            .map(|bytes| String::from_utf8_lossy(&bytes).contains("boom"))
            .unwrap_or(false);
        assert!(
            flushed,
            "a held error line must be flushed to the blockfile when the retry's own respawn fails, \
             not silently dropped"
        );
    }

    /// Regression test for reagentx + Codex (PR #2693 review): when no
    /// recovery candidate exists, this IS unambiguously known right now
    /// — the outcome must be emitted immediately, not silently dropped
    /// just because the premature emission at the `persistent_resume.rs`
    /// state-machine layer was removed.
    #[test]
    fn retry_after_resume_failure_emits_fresh_outcome_immediately_when_no_recovery_found() {
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-emits-fresh-no-recovery".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker),
            None,
            None,
            Some(filestore.clone()),
        );
        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(), // no CLAUDE_CONFIG_DIR — nothing to recover
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "dead-sid".to_string(),
            message_id: None,
        };
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

        let content = filestore
            .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
            .unwrap()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        assert!(
            content.contains("agentmux_session_outcome") && content.contains("\"fresh\""),
            "a genuinely blank fallback (no recovery candidate) must emit the Fresh outcome \
             immediately — got: {content:?}"
        );
    }

    /// Regression test for reagentx + Codex (PR #2693 review), the other
    /// half: when a recovery candidate IS found, this function must NOT
    /// emit ANY outcome yet — the CLI hasn't confirmed it (Codex P1: it
    /// could still reject the recovered id too). The eventual outcome is
    /// left to `persistent_resume.rs`'s already-tested `SessionCaptured`
    /// handling, once the (now resume-tracked) recovery attempt actually
    /// resolves.
    #[test]
    fn retry_after_resume_failure_does_not_emit_an_outcome_yet_when_recovery_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_string_lossy().to_string();
        let working_dir = r"C:\Users\asafe\.agentmux\agents\agentx-0623n".to_string();
        let slug = crate::backend::session_backfill::encode_project_slug(&working_dir);
        let dir = tmp.path().join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-defers-outcome-on-recovery".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker),
            None,
            None,
            Some(filestore.clone()),
        );
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir,
            env_vars,
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "d019e2e4-stale".to_string(),
            message_id: None,
        };
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "d019e2e4-stale".to_string());

        let content = filestore
            .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
            .unwrap()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        assert!(
            !content.contains("agentmux_session_outcome"),
            "must not claim any outcome until the CLI actually confirms the recovered id — got: {content:?}"
        );
    }

    /// codex P2 on PR #3523. An empty retry batch used to be unreachable —
    /// "the batch always has at least the triggering message". Eager resume
    /// (issue #3463) made it reachable: it starts `AwaitingOutcome` tracking
    /// with a deliberately empty batch, since it revives a session with
    /// nothing queued. If that `--resume` is stale and NO prompt has arrived
    /// yet, this runs with no entries.
    ///
    /// Scoped to the recovery-FOUND case on purpose, because that is the only
    /// one that hangs: that arm defers "resolved" to whichever
    /// `EmitSessionOutcome` site confirms the recovered id, and with no first
    /// entry there is no spawn to ever confirm it, so the pane sits on
    /// "Reconnecting…" forever. The no-recovery arm resolves synchronously and
    /// was never broken — an earlier cut of this fix published unconditionally
    /// and regressed it to `[resolved, resolved]`, evicting "retrying" from
    /// this `persist: 2` event's history.
    #[test]
    fn retry_after_resume_failure_resolves_an_empty_batch_instead_of_hanging() {
        // Same on-disk fixture the recovery-found test uses: a live session
        // file big enough to be picked as a recovery candidate.
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_string_lossy().to_string();
        let working_dir = r"C:\Users\asafe\.agentmux\agents\agentx-0623n".to_string();
        let slug = crate::backend::session_backfill::encode_project_slug(&working_dir);
        let dir = tmp.path().join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-reconnecting-empty-batch".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker.clone()),
            None,
            None,
            Some(filestore),
        );
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir,
            env_vars,
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "d019e2e4-stale".to_string(),
            message_id: None,
        };
        // The eager-resume shape: unconfirmed `--resume`, nothing queued.
        c.retry_after_resume_failure(1, config, vec![], None, "d019e2e4-stale".to_string());

        let history = broker.read_event_history(
            crate::backend::mps::EVENT_AGENT_RESUME_RETRY,
            &format!("block:{block_id}"),
            10,
        );
        let statuses: Vec<Option<&str>> = history
            .iter()
            .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            statuses,
            vec![Some("retrying"), Some("resolved")],
            "an empty batch must still resolve 'Reconnecting...', not strand the pane in it — got: {statuses:?}"
        );
    }

    /// docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md
    /// §6.2: a stale-`--resume` retry must publish a "Reconnecting…" status
    /// ping so the pane can show *something* during the gap instead of going
    /// silent. When no recovery candidate exists, the retry AND its
    /// resolution are both known synchronously in the same call — both
    /// pings must land, in order.
    #[test]
    fn retry_after_resume_failure_publishes_retrying_then_resolved_when_no_recovery_found() {
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-reconnecting-no-recovery".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker.clone()),
            None,
            None,
            Some(filestore),
        );
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

        let history = broker.read_event_history(
            crate::backend::mps::EVENT_AGENT_RESUME_RETRY,
            &format!("block:{block_id}"),
            10,
        );
        let statuses: Vec<Option<&str>> = history
            .iter()
            .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            statuses,
            vec![Some("retrying"), Some("resolved")],
            "expected retrying then resolved, got: {statuses:?}"
        );
        let retrying_data = history[0].data.as_ref().unwrap();
        assert!(
            retrying_data.get("startedAt").and_then(|v| v.as_str()).is_some(),
            "the retrying ping must carry a startedAt timestamp for the frontend's elapsed-time readout"
        );
    }

    /// The other half of the pair above: when a recovery candidate IS found,
    /// only "retrying" fires within this call — "resolved" is deferred to
    /// whichever `EmitSessionOutcome` handling site eventually confirms the
    /// recovered id (or rejects it, cascading into another retry — which
    /// would republish "retrying" again, not "resolved").
    #[test]
    fn retry_after_resume_failure_only_publishes_retrying_when_recovery_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_string_lossy().to_string();
        let working_dir = r"C:\Users\asafe\.agentmux\agents\agentx-0623n".to_string();
        let slug = crate::backend::session_backfill::encode_project_slug(&working_dir);
        let dir = tmp.path().join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-reconnecting-with-recovery".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker.clone()),
            None,
            None,
            Some(filestore),
        );
        let mut env_vars = HashMap::new();
        env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
        let config = PersistentSpawnConfig {
            cli_command: "definitely-not-a-real-binary-xyz".to_string(),
            cli_args: vec![],
            working_dir,
            env_vars,
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: "d019e2e4-stale".to_string(),
            message_id: None,
        };
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "d019e2e4-stale".to_string());

        let history = broker.read_event_history(
            crate::backend::mps::EVENT_AGENT_RESUME_RETRY,
            &format!("block:{block_id}"),
            10,
        );
        let statuses: Vec<Option<&str>> = history
            .iter()
            .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            statuses,
            vec![Some("retrying")],
            "must not resolve yet — the CLI hasn't confirmed the recovered id — got: {statuses:?}"
        );
    }

    /// reagentx P1 (round 2 on PR #2371): `DeliverDirect`'s own fallback
    /// (every message in the batch fails `try_send`, so delivery hands
    /// off to `drain_queue_after_successful_spawn` — a fire-and-forget
    /// background task) must NOT eagerly flush a held-back error line —
    /// an earlier cut of this fix did, reasoning it couldn't confirm
    /// eventual delivery, but that contradicts the same established
    /// pattern as the `Queued` arm: eventual success is the
    /// overwhelmingly common outcome, so flushing eagerly would show a
    /// stale, wrong error bubble immediately followed by the real
    /// (successful) response — reproducing this PR's own bug via a
    /// different path. The drain's own `stalled_with_leftovers` branch
    /// already publishes a status update on genuine total failure. A
    /// closed stdin receiver forces every `try_send` in the batch to
    /// fail deterministically.
    #[tokio::test]
    async fn retry_after_resume_failure_does_not_flush_the_held_error_line_when_the_deliver_direct_fallback_is_needed() {
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-flush-on-deliver-direct-fallback".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker),
            None,
            None,
            Some(filestore.clone()),
        );
        let (tx, rx) = mpsc::channel::<String>(1);
        drop(rx); // closed receiver — every try_send below fails
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], Some("boom\n".to_string()), "dead-sid".to_string());
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let flushed = filestore
            .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
            .unwrap()
            .map(|bytes| String::from_utf8_lossy(&bytes).contains("boom"))
            .unwrap_or(false);
        assert!(
            !flushed,
            "a held error line must NOT be eagerly flushed when DeliverDirect's own fallback drain \
             is needed — eventual delivery is the common case, and the drain's own stalled branch \
             already surfaces a genuine total failure"
        );
    }

    /// Closes issue #2368 (the visible-error-flash residue of #2360/#2373's
    /// stale-`--resume` retry): unlike the two tests above (retry itself
    /// fails to launch → flush; `DeliverDirect` fallback needed → don't
    /// flush yet), this covers the actual `BecomeSpawner` HAPPY path — the
    /// fresh, no-`--resume` respawn launches successfully. `held_error_line`
    /// must be silently dropped here, never reaching the blockfile: the
    /// doomed first attempt's error was never the user's problem to see
    /// once the transparent retry it triggered actually worked. This is
    /// the regression test
    /// `docs/specs/SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md`
    /// §5's verification list asked for before #2368 could be closed with
    /// evidence.
    #[tokio::test]
    async fn retry_after_resume_failure_drops_the_held_error_line_when_the_respawn_succeeds() {
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "block-drop-on-successful-retry".to_string();
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker),
            None,
            None,
            Some(filestore.clone()),
        );

        // codex P1 on this PR: "echo" is a cmd.exe BUILT-IN on Windows, not
        // a standalone executable — `Command::new("echo")` fails to spawn
        // on the required Windows CI leg, flipping this test's assertion
        // (spawn `Err` routes through the FAILURE branch, which flushes
        // "boom", the opposite of what's being proven here). "git" is a
        // real, standalone executable guaranteed present on every
        // supported CI platform (Windows/macOS/Linux all need it to check
        // the repo out in the first place) and exits 0 near-instantly.
        let config = PersistentSpawnConfig {
            cli_command: "git".to_string(),
            cli_args: vec!["--version".to_string()],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], Some("boom\n".to_string()), "dead-sid".to_string());
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let flushed = filestore
            .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
            .unwrap()
            .map(|bytes| String::from_utf8_lossy(&bytes).contains("boom"))
            .unwrap_or(false);
        assert!(
            !flushed,
            "a held error line from the doomed first attempt must be silently dropped — never \
             flushed to the blockfile — when the stale-`--resume` retry's fresh respawn actually \
             succeeds (issue #2368): the user should see only the real response, not a stale \
             error bubble followed by it"
        );
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "second".to_string()));
            // Simulates spawn_process's own synchronous stash for "first"
            // — the message that triggered this spawn.
            let generation = inner.spawn_generation;
            inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                generation,
                attempted_sid: "sid".to_string(),
                retry: persistent_resume::RetryPayload { config: retry_config.clone(), messages: vec![qentry(1, "first")] },
            });
        }

        c.drain_queue_after_successful_spawn(retry_config, true);

        assert_eq!(rx.recv().await.unwrap(), "first");
        assert_eq!(rx.recv().await.unwrap(), "second");
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let inner = c.inner.lock().unwrap();
        assert!(
            !inner.drain_send_in_flight,
            "must be cleared once the send-then-append sequence for the last message has fully completed"
        );
        match &inner.resume {
            persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => assert_eq!(
                retry.messages,
                vec![qentry(1, "first"), qentry(2, "second")],
                "must contain the ORIGINAL message exactly once plus every later delivery, in order"
            ),
            other => panic!("expected AwaitingOutcome, got {other:?}"),
        }
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "older-leftover".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "new-trigger".to_string()));
            // spawn_process's own synchronous stash — seeded with the
            // ACTUAL trigger message, not whatever happens to sit at the
            // front of the queue.
            let generation = inner.spawn_generation;
            inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                generation,
                attempted_sid: "sid".to_string(),
                retry: persistent_resume::RetryPayload {
                    config: retry_config.clone(),
                    messages: vec![qentry(2, "new-trigger")],
                },
            });
        }

        c.drain_queue_after_successful_spawn(retry_config, true);

        assert_eq!(rx.recv().await.unwrap(), "older-leftover");
        assert_eq!(rx.recv().await.unwrap(), "new-trigger");
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let inner = c.inner.lock().unwrap();
        let delivered = match &inner.resume {
            persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => &retry.messages,
            other => panic!("expected AwaitingOutcome, got {other:?}"),
        };
        assert_eq!(
            delivered.len(),
            2,
            "must contain exactly the older leftover plus the trigger, no omission, no duplication: {delivered:?}"
        );
        assert!(
            delivered.iter().any(|e| e.json == "older-leftover"),
            "the older leftover must not be silently dropped from the retry batch"
        );
        assert_eq!(
            delivered.iter().filter(|e| e.json == "new-trigger").count(),
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
            inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "second".to_string()));
            // Simulates poison_resume having ALREADY promoted the tentative
            // retry to confirmed before the drain got to "second".
            let generation = inner.spawn_generation;
            inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
                generation,
                attempted_sid: "sid".to_string(),
                retry: persistent_resume::RetryPayload { config: retry_config.clone(), messages: vec![qentry(1, "first")] },
            });
            inner.apply_resume_event(persistent_resume::ResumeEvent::ResumeUnreachable {
                generation,
                sid: "sid".to_string(),
            });
        }

        c.drain_queue_after_successful_spawn(retry_config, true);

        assert_eq!(rx.recv().await.unwrap(), "first");
        assert_eq!(rx.recv().await.unwrap(), "second");
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let inner = c.inner.lock().unwrap();
        let delivered = match &inner.resume {
            persistent_resume::ResumeState::ConfirmedRetry { retry, .. } => &retry.messages,
            other => {
                panic!("expected ConfirmedRetry (already promoted), got {other:?}")
            }
        };
        assert_eq!(
            delivered,
            &vec![qentry(1, "first"), qentry(2, "second")],
            "must still be tracking this spawn's delivered messages, even though it was already confirmed"
        );
    }

    /// codex P2 on PR #2360 (sixth review pass, round 6): deciding and
    /// enqueueing a multi-message retry batch one call at a time left a
    /// window where a genuinely new, unrelated message could interleave
    /// into the MIDDLE of the batch. `decide_retry_batch_action` must
    /// enqueue the whole batch atomically instead.
    #[test]
    fn decide_retry_batch_action_enqueues_the_whole_batch_atomically() {
        let c = controller();
        let action = c.decide_retry_batch_action(1, &qentry(1, "first"), &[qentry(2, "second"), qentry(3, "third")]);
        assert!(matches!(action, RetryBatchAction::BecomeSpawner { .. }));
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
            c.decide_retry_batch_action(1, &qentry(1, "hello"), &[qentry(2, "hello"), qentry(3, "hello")]);

        assert!(matches!(action, RetryBatchAction::Queued));
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
        }

        let action = c.decide_retry_batch_action(1, &qentry(1, "first"), &[qentry(2, "second")]);

        assert!(matches!(action, RetryBatchAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner
                .pending_send_messages
                .iter()
                .map(|m| (m.seq, m.json_str.clone()))
                .collect::<Vec<_>>(),
            vec![(1, "first".to_string()), (2, "second".to_string())],
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
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "later-message".to_string()));
        }

        let action = c.decide_retry_batch_action(1, &qentry(2, "A"), &[]);

        assert!(matches!(action, RetryBatchAction::Queued));
        let inner = c.inner.lock().unwrap();
        assert_eq!(
            inner.pending_send_messages.iter().cloned().collect::<Vec<_>>(),
            vec!["A".to_string(), "later-message".to_string()],
            "the retry's own (chronologically earlier) message must precede the later, unrelated one"
        );
    }
}

/// Covers `PersistentInner`'s thin `poison_resume` / `try_capture_session_id`
/// wrappers — that they correctly plumb `session_id`/`resume_poisoned`
/// bookkeeping (unrelated to the race, still simple fields) alongside
/// delegating to `persistent_resume::update` for the resume/retry
/// decision itself. The exhaustive race-condition coverage (poison-
/// before/after the error line, message-batch growth, stop overrides,
/// mismatched sids, stale generations) now lives in
/// `persistent_resume::tests` against the pure function directly —
/// deterministic, and without needing this `PersistentInner` scaffolding
/// at all. Keeping both would just duplicate the same assertions at two
/// layers.
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
            restart_when_idle: false,
            restart_pending: false,
            resume: persistent_resume::ResumeState::default(),
            spawning_in_progress: false,
            pending_send_messages: VecDeque::new(),
            drain_claim: false,
            next_message_seq: 0,
            drain_send_in_flight: false,
            current_pid: None,
            stdin_tx: None,
            kill_tx: None,
            shutdown_generation: None,
            stop_exit: None,
            spawn_generation: 0,
            pending_questions: HashMap::new(),
            pending_permissions: HashMap::new(),
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

    fn spawned_with_resume(inner: &mut PersistentInner, generation: u64, attempted_sid: &str) {
        // Mirror production's invariant (`spawn_process` bumps
        // `spawn_generation` in the same lock acquisition that applies
        // the spawn event): ambient adoption in `try_capture_session_id`
        // is gated on generation currency (issue #2366), so a helper
        // that left `spawn_generation` at 0 would make every capture
        // look stale.
        inner.spawn_generation = generation;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
            generation,
            attempted_sid: attempted_sid.to_string(),
            retry: persistent_resume::RetryPayload { config: dummy_spawn_config(), messages: vec![persistent_resume::QueuedRetryEntry { seq: 1, json: "{}".to_string() }] },
        });
    }

    /// Issue #2366 regression: a superseded generation's still-draining
    /// stdout reader must not re-install its stale sid into ambient
    /// `session_id` after a fallback respawn's plain clear.
    /// `respawn_once_for_leftover_queue` deliberately does NOT poison
    /// (the death may be unrelated to a stale resume — see its round-13
    /// comment), so `resume_poisoned` cannot catch this echo; only the
    /// generation gate can.
    #[test]
    fn a_stale_generations_capture_does_not_adopt_into_ambient_session_id() {
        let mut inner = inner_with_session_id(Some("stale-sid"));
        spawned_with_resume(&mut inner, 1, "stale-sid");

        // The gen-1 process died; the fallback respawn cleared the
        // ambient sid and spawned gen 2 fresh (no --resume, so no new
        // resume tracking).
        inner.session_id = None;
        inner.spawn_generation = 2;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation: 2 });

        // Gen 1's stdout reader finally drains its buffered echo of the
        // stale attempted sid.
        let (adopted, _effects) = inner.try_capture_session_id("stale-sid", 1, false);

        assert!(!adopted, "a superseded generation's echo must not be adopted");
        assert_eq!(
            inner.session_id, None,
            "ambient session_id must stay clear for the live generation's own capture"
        );
    }

    /// Control case for the gate above: the CURRENT generation's capture
    /// into a cleared ambient sid must still adopt normally.
    #[test]
    fn the_current_generations_capture_still_adopts_after_a_clear() {
        let mut inner = inner_with_session_id(None);
        inner.spawn_generation = 2;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation: 2 });

        let (adopted, _effects) = inner.try_capture_session_id("fresh-sid", 2, false);

        assert!(adopted, "the live generation's first capture must adopt");
        assert_eq!(inner.session_id.as_deref(), Some("fresh-sid"));
    }

    // stderr wins the race: poisons the id and clears it from session_id,
    // then the stdout reader's later echo of the same dead id is refused.
    #[test]
    fn stderr_first_then_stdout_echo_is_refused() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        inner.poison_resume("dead-sid", 1);
        assert_eq!(inner.session_id, None, "poisoning the live session id clears it");

        let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, true);
        assert!(!captured, "must refuse to re-adopt a confirmed-poisoned id");
        assert_eq!(inner.session_id, None);
    }

    // stdout wins the race (echoes the dead id before stderr's "No
    // conversation found" arrives): the later poison must still clear it.
    #[test]
    fn stdout_first_then_stderr_poison_still_clears() {
        let mut inner = inner_with_session_id(None);
        // A capture for generation 1 can only originate from a gen-1
        // spawn's own reader task (issue #2366's currency gate).
        inner.spawn_generation = 1;
        let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, true);
        assert!(captured, "first capture with no prior state succeeds");
        assert_eq!(inner.session_id.as_deref(), Some("dead-sid"));

        inner.poison_resume("dead-sid", 1);
        assert_eq!(inner.session_id, None, "poison must clear it even though stdout set it first");
    }

    // A genuinely fresh session id (the CLI gave up on --resume and started
    // a new conversation) is unaffected by an unrelated prior poison.
    #[test]
    fn different_fresh_session_id_is_captured_normally() {
        let mut inner = inner_with_session_id(None);
        // A capture for generation 1 can only originate from a gen-1
        // spawn's own reader task (issue #2366's currency gate).
        inner.spawn_generation = 1;
        inner.poison_resume("dead-sid", 1);

        let (captured, _effects) = inner.try_capture_session_id("fresh-sid", 1, true);
        assert!(captured, "a different id is not blocked by an unrelated poison");
        assert_eq!(inner.session_id.as_deref(), Some("fresh-sid"));
    }

    // reagentx P1 (round 4 on this PR): the realistic precondition here
    // is `session_id` already holding the STALE attempted sid — a
    // `--resume <sid>` spawn always hydrates `session_id` to it BEFORE
    // the process even starts (`spawn_process`). `adopted` used to be
    // gated solely on `session_id.is_none()`, which this exact scenario
    // (session_id already the stale sid, resume tracking still live)
    // never satisfies — leaving `session_id` stuck on the stale sid
    // forever even though the CLI had genuinely moved on to a brand-new
    // conversation.
    #[test]
    fn adopts_a_genuinely_different_sid_even_when_session_id_already_holds_the_stale_attempted_one() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        spawned_with_resume(&mut inner, 1, "dead-sid");

        // A DIFFERENT (fresh) sid — the CLI gave up on --resume
        // internally and started its own new conversation without ever
        // hitting the stderr "No conversation found" path. A different
        // sid is unambiguous proof of progress on its own, even from a
        // frame that isn't itself a confirmed terminal success.
        let (captured, _effects) = inner.try_capture_session_id("brand-new-sid", 1, false);
        assert!(
            captured,
            "a genuinely different sid must be adopted even though session_id already held \
             the stale attempted one"
        );
        assert_eq!(
            inner.session_id.as_deref(),
            Some("brand-new-sid"),
            "session_id must move on to the fresh conversation, not stay stuck on the stale \
             attempted sid"
        );
        assert_eq!(inner.resume, persistent_resume::ResumeState::NotTracking { current_generation: 1 });
    }

    // Once a session id is already held, a second stdout line (e.g. a
    // duplicate echo) must not overwrite it.
    #[test]
    fn does_not_overwrite_an_already_captured_session_id() {
        let mut inner = inner_with_session_id(Some("first-sid"));
        let (captured, _effects) = inner.try_capture_session_id("second-sid", 1, true);
        assert!(!captured, "must not overwrite an already-captured session id");
        assert_eq!(inner.session_id.as_deref(), Some("first-sid"));
    }

    // codex P1 on PR #2371: a `--resume <sid>` spawn ALWAYS has
    // `session_id` already `Some` before the process starts (that's what
    // makes `--resume` get attached at all) — so on the common
    // resume-SUCCEEDED case, the CLI's first-line echo of that SAME sid
    // must still resolve this generation's resume tracking even though
    // `captured` itself is `false` (nothing new was adopted). Without
    // this, persistent mode never exiting between turns meant that
    // tentative state sat live for the rest of the process's potentially
    // long lifetime, wrongly holding back every LATER, unrelated
    // `is_error:true` result as if it might still need to be dropped for
    // a stale-resume retry.
    #[test]
    fn resolves_pending_retry_on_a_successful_resume_even_though_session_id_was_already_held() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        spawned_with_resume(&mut inner, 1, "dead-sid");

        let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, true);

        assert!(!captured, "nothing new was adopted — session_id was already this exact sid");
        assert_eq!(inner.session_id.as_deref(), Some("dead-sid"));
        assert_eq!(
            inner.resume,
            persistent_resume::ResumeState::NotTracking { current_generation: 1 },
            "a successful resume must stand down the retry safety net even when \
             session_id was already held before this call"
        );
        assert_eq!(
            inner.resume,
            persistent_resume::ResumeState::NotTracking { current_generation: 1 },
            "a successful resume must stand down the retry safety net even when \
             session_id was already held before this call"
        );
    }

    // reagentx P0 on PR #2373: `try_capture_session_id` must surface the
    // `FlushErrorLine` effect `apply_resume_event` returns, not just the
    // `adopted` bool — an earlier cut of this method discarded it
    // entirely, silently losing an EARLIER turn's held-back error line
    // the moment a LATER, genuinely successful capture on this same
    // still-alive generation resolved tracking. This is exactly the
    // integration-level gap a pure `persistent_resume::update()` unit
    // test can't catch (that function's own return value was always
    // correct — see `persistent_resume::tests::
    // session_captured_flushes_a_held_error_line_from_an_earlier_turn`);
    // the bug was the caller silently dropping what `update()` handed
    // back.
    #[test]
    fn try_capture_session_id_surfaces_a_held_error_line_from_an_earlier_turn() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        spawned_with_resume(&mut inner, 1, "dead-sid");
        // An earlier turn on this same generation held an error line back
        // (tracking still undecided at the time).
        let held = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
            generation: 1,
            line: "boom\n".to_string(),
        });
        assert!(held.is_empty(), "sanity: the line must be held back, not persisted immediately");

        // A LATER, genuinely successful capture on this same generation
        // resolves tracking — the caller must see (and flush) the
        // earlier held line, not silently lose it.
        let (_, effects) = inner.try_capture_session_id("dead-sid", 1, true);
        assert_eq!(
            effects,
            vec![
                persistent_resume::ResumeEffect::FlushErrorLine("boom\n".to_string()),
                persistent_resume::ResumeEffect::EmitSessionOutcome {
                    outcome: persistent_resume::SessionOutcome::Resumed,
                    attempted_sid: "dead-sid".to_string(),
                    actual_sid: None,
                },
            ],
            "a held-back error line from an earlier turn must be surfaced to the caller \
             when a later capture resolves tracking, not silently discarded"
        );
    }

    // reagentx P0 on PR #2371 (originally), superseded by reagentx P0 on
    // PR #2373: the real CLI's stream-json protocol embeds
    // `session_id_field` on EVERY event, including the terminal `result`
    // — so the doomed attempt's OWN `is_error:true` line carries the same
    // (stale) sid it was given. The stdout reader's `!is_error_result`
    // gate skips calling `try_capture_session_id` for that exact line,
    // but this test proves the deeper fix: even if it WERE called here
    // (belt-and-suspenders against a caller mistake, or a future frame
    // type the gate doesn't anticipate), passing the correct
    // `is_confirmed_success: false` (an error is never a confirmed
    // success) means the ambiguous same-sid echo alone no longer resolves
    // tracking — see `ResumeEvent::SessionCaptured`'s own doc comment.
    #[test]
    fn calling_try_capture_session_id_on_the_doomed_error_frame_does_not_clear_tracking() {
        let mut inner = inner_with_session_id(Some("dead-sid"));
        spawned_with_resume(&mut inner, 1, "dead-sid");

        // Simulates the terminal error-result line's OWN embedded
        // session_id field — the same sid this generation attempted,
        // not yet poisoned (the stderr reader hasn't necessarily run
        // yet), with `is_confirmed_success` correctly computed as `false`
        // since this frame IS the error.
        let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, false);
        assert!(!captured);
        assert!(
            matches!(inner.resume, persistent_resume::ResumeState::AwaitingOutcome { .. }),
            "an ambiguous same-sid echo on a frame that isn't a confirmed success must not \
             resolve tracking"
        );

        // Tracking is still live, so the ErrorResultLine event correctly
        // holds the line back pending the retry decision.
        let effects = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
            generation: 1,
            line: r#"{"type":"result","is_error":true}"#.to_string() + "\n",
        });
        assert!(
            effects.is_empty(),
            "tracking is still live, so the line must be held back, not persisted immediately"
        );
    }
}

#[cfg(test)]
mod agent_id_tests {
    use super::*;

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

    /// reagentx P1 on #2697: `agent_id()` was captured once at spawn and
    /// never refreshed on re-registration, so a legitimately renamed or
    /// reconfigured agent's own messages could be falsely rejected as an
    /// identity mismatch by `inject_message_inner`'s recipient-identity
    /// check (#2695). `set_agent_id` (called from `handle_reactive_register`
    /// on every re-registration) must actually overwrite the captured value,
    /// not just the initial spawn-time one.
    #[test]
    fn set_agent_id_overwrites_a_stale_captured_value() {
        let ctrl = controller();
        assert_eq!(ctrl.agent_id(), None);

        ctrl.set_agent_id(Some("agentx".to_string()));
        assert_eq!(ctrl.agent_id(), Some("agentx".to_string()));

        ctrl.set_agent_id(Some("agenty".to_string()));
        assert_eq!(ctrl.agent_id(), Some("agenty".to_string()));

        ctrl.set_agent_id(None);
        assert_eq!(ctrl.agent_id(), None);
    }
}

/// `Controller::shutdown` against a real process: a small node script that
/// speaks just enough stream-json to stand in for the Claude CLI, with the
/// behaviour measured in the pane-close spec §5.1 — a running turn survives
/// EOF, the interrupt ends it with `result: error_during_execution`, EOF
/// then exits with code 1.
#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use crate::backend::blockcontroller::{Controller, StopOutcome};

    const STUB: &str = r#"
const mode = process.argv[2];
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type === "control_request" && m.request && m.request.subtype === "interrupt") {
    out({ type: "control_response", response: { subtype: "success", request_id: m.request_id } });
    if (mode !== "stubborn") {
      out({ type: "result", subtype: "error_during_execution", is_error: true, session_id: "stub-session" });
    }
    return;
  }
  if (m.type === "user" && mode === "idle") {
    out({ type: "result", subtype: "success", is_error: false, result: "ok", session_id: "stub-session" });
  }
  // "turn" / "stubborn": the turn never finishes on its own.
});
rl.on("close", () => {
  if (mode === "stubborn") { setInterval(() => {}, 1000); } else { process.exit(1); }
});
"#;

    /// `None` (test skipped with a note) when `node` isn't on PATH.
    fn stub_path() -> Option<std::path::PathBuf> {
        // A PATH lookup, not `node --version`: a probe spawn would be one more
        // unsanitized spawn site for `pane_env`'s I7 inventory to flag.
        let has_node = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path)
                .any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
        });
        if !has_node {
            eprintln!("shutdown_tests: `node` not on PATH — skipping");
            return None;
        }
        let path = std::env::temp_dir().join(format!("agentmux-shutdown-stub-{}.js", uuid::Uuid::new_v4()));
        std::fs::write(&path, STUB).unwrap();
        Some(path)
    }

    fn start(block_id: &str, mode: &str, stub: &std::path::Path) -> (PersistentSubprocessController, Arc<mps::Broker>) {
        let broker = Arc::new(mps::Broker::new());
        let c = PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.to_string(),
            Some(Arc::clone(&broker)),
            None,
            None,
            None,
        );
        let config = PersistentSpawnConfig {
            cli_command: "node".to_string(),
            cli_args: vec![stub.to_string_lossy().to_string(), mode.to_string()],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        let msg = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        c.send_message(msg.to_string(), config).unwrap();
        (c, broker)
    }

    /// Wait until the stub is actually running: it has printed its init line
    /// and the controller captured the session id from it. A bare pid isn't
    /// enough — node can take seconds to start on a loaded CI runner, and the
    /// timing assertions must not include that. The budget is generous: a
    /// Windows runner scanning a freshly written .js file took over 15s once
    /// (the #3409 CI run). The panic says what state it was stuck in.
    async fn wait_for_pid(c: &PersistentSubprocessController) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let (pid, sid) = {
                let g = c.inner.lock().unwrap();
                (g.current_pid, g.session_id.clone())
            };
            if pid.is_some() && sid.as_deref() == Some("stub-session") {
                return;
            }
            if std::time::Instant::now() >= deadline {
                panic!("stub process never became ready: current_pid={pid:?} session_id={sid:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    fn failures(broker: &mps::Broker, block_id: &str) -> usize {
        broker
            .read_event_history(mps::EVENT_AGENT_FAILURE, &format!("block:{block_id}"), 10)
            .len()
    }

    #[tokio::test]
    async fn never_spawned_is_not_running() {
        let c = PersistentSubprocessController::new("tab".into(), "blk-never".into(), None, None, None, None);
        let outcome = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(1)).await;
        assert_eq!(outcome, StopOutcome::NotRunning);
    }

    /// Mid-turn: EOF alone would let the turn run on (§5.1). The interrupt
    /// ends it, the process exits well inside the deadline, and the
    /// interrupted turn's `is_error` result is NOT reported as a failure.
    #[tokio::test]
    async fn mid_turn_is_interrupted_and_exits_without_a_failure() {
        let Some(stub) = stub_path() else { return };
        let block_id = "blk-shutdown-midturn";
        let (c, broker) = start(block_id, "turn", &stub);
        wait_for_pid(&c).await;
        assert!(c.health_monitor.is_active_turn(), "precondition: a turn is running");

        let started = std::time::Instant::now();
        let outcome = c.shutdown(started + std::time::Duration::from_secs(5)).await;
        let elapsed = started.elapsed();

        assert_eq!(outcome, StopOutcome::Exited, "must exit on its own, not be killed");
        assert!(elapsed < std::time::Duration::from_secs(3), "took {elapsed:?}");
        assert_eq!(failures(&broker, block_id), 0, "a requested stop is not a failure");
        let _ = std::fs::remove_file(stub);
    }

    #[tokio::test]
    async fn idle_exits_on_eof() {
        let Some(stub) = stub_path() else { return };
        let (c, _broker) = start("blk-shutdown-idle", "idle", &stub);
        wait_for_pid(&c).await;
        // The stub answers the message with a `result`, ending the turn. Node
        // startup on a loaded CI runner can take seconds — wait for it rather
        // than assume.
        for _ in 0..200 {
            if !c.health_monitor.is_active_turn() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(!c.health_monitor.is_active_turn(), "precondition: idle");

        let started = std::time::Instant::now();
        let outcome = c.shutdown(started + std::time::Duration::from_secs(5)).await;
        assert_eq!(outcome, StopOutcome::Exited);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
        let _ = std::fs::remove_file(stub);
    }

    /// A process that ignores both the interrupt and EOF is killed at the
    /// deadline — not before, and not long after.
    #[tokio::test]
    async fn stubborn_process_is_killed_at_the_deadline() {
        let Some(stub) = stub_path() else { return };
        let (c, _broker) = start("blk-shutdown-stubborn", "stubborn", &stub);
        wait_for_pid(&c).await;

        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(1500);
        let outcome = c.shutdown(deadline).await;
        let elapsed = started.elapsed();

        assert_eq!(outcome, StopOutcome::Killed);
        assert!(elapsed >= std::time::Duration::from_millis(1400), "killed early: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_secs(4), "killed late: {elapsed:?}");
        let _ = std::fs::remove_file(stub);
    }
}

/// One session, one process: a spawn that would `--resume` a session another
/// live process is still on is refused (pane-close spec §4.7).
#[cfg(test)]
mod reopen_guard_tests {
    use super::*;
    use crate::backend::obj::Block;

    fn resume_config(sid: &str) -> PersistentSpawnConfig {
        PersistentSpawnConfig {
            // Never actually spawned when the guard refuses; "git" (a real
            // executable on every CI platform) keeps the allowed case honest.
            cli_command: "git".to_string(),
            cli_args: vec!["--version".to_string()],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: sid.to_string(),
            message_id: None,
        }
    }

    fn controller(block_id: &str, mstore: Option<Arc<Store>>) -> Arc<PersistentSubprocessController> {
        Arc::new(PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.to_string(),
            None,
            None,
            mstore,
            None,
        ))
    }

    const MSG: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;

    #[tokio::test]
    async fn a_session_live_in_another_pane_is_not_resumed_twice() {
        let sid = format!("sid-{}", uuid::Uuid::new_v4());
        let live_block = format!("live-{}", uuid::Uuid::new_v4());
        let live = controller(&live_block, None);
        {
            let mut g = live.inner.lock().unwrap();
            g.session_id = Some(sid.clone());
            g.current_pid = Some(4242);
        }
        crate::backend::blockcontroller::register_controller(&live_block, live.clone());

        let reopen = controller(&format!("reopen-{}", uuid::Uuid::new_v4()), None);
        let err = reopen.send_message(MSG.to_string(), resume_config(&sid)).unwrap_err();
        crate::backend::blockcontroller::delete_controller(&live_block);

        assert!(err.contains("already open in another pane"), "got: {err}");
        assert!(reopen.inner.lock().unwrap().current_pid.is_none(), "no second process");
    }

    #[tokio::test]
    async fn a_session_whose_process_is_still_closing_is_not_resumed_yet() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let sid = format!("sid-{}", uuid::Uuid::new_v4());
        let closing_block = format!("closing-{}", uuid::Uuid::new_v4());
        let mut block = Block {
            oid: closing_block.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert(core::META_SESSION_ID.to_string(), serde_json::json!(sid));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
        crate::backend::blockcontroller::mark_closing(&closing_block);

        let reopen = controller(&format!("reopen-{}", uuid::Uuid::new_v4()), Some(store.clone()));
        let err = reopen.send_message(MSG.to_string(), resume_config(&sid)).unwrap_err();
        assert!(err.contains("still shutting down"), "got: {err}");

        // Once that process has exited, the same reopen is allowed.
        crate::backend::blockcontroller::mark_closing_stopped(&closing_block);
        let allowed = controller(&format!("reopen-{}", uuid::Uuid::new_v4()), Some(store));
        let result = allowed.send_message(MSG.to_string(), resume_config(&sid));
        crate::backend::blockcontroller::unmark_closing(&closing_block);
        assert!(result.is_ok(), "got: {result:?}");
    }

    /// reagent P1 on #3421: check-then-spawn must be atomic across
    /// controllers. Several reopens of one session at once — exactly one may
    /// win. Uses a node process that stays alive (EOF-only exit), so the
    /// winner's `current_pid` stays set while the others check.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_reopens_of_one_session_start_exactly_one_process() {
        let has_node = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
        });
        if !has_node {
            eprintln!("reopen_guard_tests: `node` not on PATH — skipping");
            return;
        }
        let stub = std::env::temp_dir().join(format!("agentmux-reopen-stub-{}.js", uuid::Uuid::new_v4()));
        std::fs::write(&stub, "process.stdin.on('data', () => {}); process.stdin.on('end', () => process.exit(0));").unwrap();

        let sid = format!("sid-{}", uuid::Uuid::new_v4());
        let controllers: Vec<Arc<PersistentSubprocessController>> =
            (0..4).map(|i| controller(&format!("race-{i}-{}", uuid::Uuid::new_v4()), None)).collect();
        for c in &controllers {
            crate::backend::blockcontroller::register_controller(&c.block_id, c.clone());
        }
        let config = PersistentSpawnConfig {
            cli_command: "node".to_string(),
            cli_args: vec![stub.to_string_lossy().to_string()],
            ..resume_config(&sid)
        };

        let barrier = Arc::new(std::sync::Barrier::new(controllers.len()));
        let handles: Vec<_> = controllers
            .iter()
            .map(|c| {
                let (c, config, barrier) = (c.clone(), config.clone(), barrier.clone());
                tokio::task::spawn_blocking(move || {
                    barrier.wait();
                    c.send_message(MSG.to_string(), config)
                })
            })
            .collect();
        let mut results = Vec::new();
        for h in handles {
            results.push(h.await.unwrap());
        }

        for c in &controllers {
            let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(3)).await;
            crate::backend::blockcontroller::delete_controller(&c.block_id);
        }
        let _ = std::fs::remove_file(stub);

        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok, 1, "exactly one reopen may win; got {results:?}");
        assert!(
            results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.contains("already open in another pane")),
            "the others are refused by the guard: {results:?}"
        );
    }
}

/// `start()`'s eager-resume path
/// (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`,
/// issue #3463). Covers the security property that made this a bigger change
/// than the original spec scoped: eager resume must go through the SAME
/// Layer 3 identity/credential spawn gate a live message send does
/// (`SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`), not spawn
/// unconditionally the moment a session id is found.
#[cfg(test)]
mod eager_resume_tests {
    use super::*;
    use crate::backend::obj::MetaMapType;
    use crate::backend::storage::store::{
        AgentDefinition, AgentInstance, IdentityAccount, InstanceStatus, SecretRef,
    };

    fn make_store() -> Arc<Store> {
        Arc::new(Store::open_in_memory().unwrap())
    }

    /// A block with NO agent-instance row. `inject_identity_env`'s own Step 1
    /// treats this as "outside the managed-credentials contract" and returns
    /// `Ok(())` unconditionally — the simplest fixture for "the gate passes",
    /// requiring no agent/account/binding setup at all.
    fn meta_with_session(session_id: &str, cli_args: &[&str]) -> MetaMapType {
        let mut m = MetaMapType::new();
        m.insert(core::META_SESSION_ID.to_string(), serde_json::json!(session_id));
        m.insert("cmd".to_string(), serde_json::json!("node"));
        m.insert(
            "cmd:args".to_string(),
            serde_json::json!(cli_args.iter().collect::<Vec<_>>()),
        );
        m.insert("cmd:cwd".to_string(), serde_json::json!(""));
        m.insert("cmd:env".to_string(), serde_json::json!({}));
        m
    }

    /// A minimal `Block` + `AgentDefinition` + `IdentityAccount` +
    /// `AgentInstance` wiring an oauth-class provider ("claude") bound to an
    /// account whose `SecretRef` can never resolve — the same shape
    /// `identity::resolver::inject::tests` uses to prove the gate blocks a
    /// spawn (`MissingCredentials`). Duplicated here (not imported) because
    /// that module's fixture helpers are private to its own `#[cfg(test)]`.
    fn wire_ungated_agent(store: &Store, block_id: &str) {
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert("agentId".to_string(), serde_json::json!("def-1"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "\u{2726}".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        let bad = IdentityAccount {
            id: "acct-bad".to_string(),
            name: "claude-acct-bad".to_string(),
            provider: "claude".to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::Env {
                env_var: "AGENTMUX_TEST_TOKEN_DOES_NOT_EXIST".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&bad).unwrap();
        store.agent_identity_link("def-1", "acct-bad", "claude").unwrap();

        let inst = AgentInstance {
            id: format!("inst-{block_id}"),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: block_id.to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: "id-1".to_string(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
    }

    /// A block plus a minimal `AgentDefinition` + `AgentInstance`, for tests
    /// that need the identity gate to PASS rather than fail — the definition's
    /// empty `provider` classifies as neither oauth- nor api-key-class
    /// (`provider_class`'s catch-all), so the gate's oauth-required check
    /// never fires. Simpler than `wire_ungated_agent`'s full
    /// definition+account+link setup.
    ///
    /// Originally written to satisfy a launch-specific instance-existence
    /// check in `try_eager_resume`. That check was reverted in PR #3523 (see
    /// issue #3525 — `db_agents` has no per-block launch record, so the check
    /// could not be made correct), so the instance row is no longer load-
    /// bearing for the gate; the fixture is kept because several tests below
    /// still want a block that looks like a real launch.
    fn wire_bare_instance(store: &Store, block_id: &str) {
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        // `instance_create` enforces a real FK to `db_agents` (with a
        // shared-registry backfill attempt first) — a `definition_id` with
        // no matching row is `NotFound`, not silently tolerated the way a
        // missing definition is on the READ side (`inject_identity_env`'s
        // Step 5). An empty `provider` classifies as neither oauth- nor
        // api-key-class (`provider_class`'s catch-all), so the gate's
        // oauth-required check never fires — the minimal definition that
        // satisfies instance_create without needing a real provider/account.
        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: format!("def-for-{block_id}"),
            slug: String::new(),
            name: "T".to_string(),
            icon: "\u{2726}".to_string(),
            provider: String::new(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        let inst = AgentInstance {
            id: format!("inst-{block_id}"),
            definition_id: def.id.clone(),
            parent_instance_id: String::new(),
            block_id: block_id.to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: "id-1".to_string(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
    }

    fn controller(block_id: &str) -> PersistentSubprocessController {
        PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.to_string(),
            None,
            None,
            None,
            None,
        )
    }

    async fn wait_for_spawn(c: &PersistentSubprocessController) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if c.inner.lock().unwrap().current_pid.is_some() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// `node` on PATH is how every other real-process test in this file
    /// gates itself (see `shutdown_tests::stub_path`) — matched here for
    /// consistency, not reinvented.
    fn has_node() -> bool {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
        })
    }

    const IDLE_STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
out({ type: "system", subtype: "init", session_id: "resumed-session" });
setInterval(() => {}, 1000);
"#;

    fn write_stub() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("agentmux-eager-resume-stub-{}.js", uuid::Uuid::new_v4()));
        std::fs::write(&path, IDLE_STUB).unwrap();
        path
    }

    /// Force-kills the wrapped controller's spawned process on drop —
    /// including on a PANICKING unwind (a failed `assert!`), not just the
    /// success path. Without this, an assertion added after a spawn (like
    /// the `turn_active` one below) leaks the stub's `setInterval`-forever
    /// node process on failure: on Windows that process holds the test
    /// binary's inherited stdio pipes open, hanging the ENTIRE test
    /// binary's shutdown rather than just failing this one test — observed
    /// directly while mutation-testing that assertion.
    struct KillOnDrop<'a>(&'a PersistentSubprocessController);
    impl Drop for KillOnDrop<'_> {
        fn drop(&mut self) {
            // NOT `self.0.stop_process(true)`. That only SENDS a kill
            // request over a channel to an async task that does the actual
            // `child.kill()` — `Drop::drop` is sync and can't wait for that
            // task to run it, and if the test's own `#[tokio::test]`
            // runtime is tearing down at the same moment (exactly when a
            // test function is returning), that task can be dropped before
            // it ever processes the message, leaving the child alive
            // despite this guard. Confirmed live: with the async form, this
            // test module hung the whole test BINARY's shutdown on two of
            // three consecutive runs, always immediately after the first
            // test that reaches this guard via the success path. A direct,
            // synchronous OS-level kill by pid has no such gap.
            // `.unwrap_or_else(PoisonError::into_inner)`, not `.unwrap()`:
            // this Drop impl runs during a panicking unwind whenever a test
            // assertion fails while it holds `inner`'s lock (every
            // assertion in this module that pattern-matches `inner.resume`
            // does, since the match arms borrow from the guard). A plain
            // `.unwrap()` here would panic a SECOND time on the resulting
            // `PoisonError` — a panic during a panic's unwind, which Rust
            // escalates straight to `abort()`. Confirmed live: an
            // intentionally-failing assertion in this exact spot surfaced
            // as `STATUS_STACK_BUFFER_OVERRUN` with no test-failure message
            // printed at all, not a normal, readable assertion failure —
            // exactly this double-panic. The underlying data is still
            // valid after a poison (the panic happened elsewhere, this
            // struct's own fields are untouched); recovering it here is
            // safe and is what actually lets a future real assertion
            // failure in this module report itself normally.
            if let Some(pid) = self.0.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).current_pid {
                #[cfg(windows)]
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output();
                #[cfg(not(windows))]
                let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output();
            }
        }
    }

    #[tokio::test]
    async fn does_not_spawn_when_no_session_id() {
        // Regression test: the lazy path (no agent:sessionid) must be
        // byte-for-byte unchanged, including when a controller HAS identity
        // stores configured — the branch decision is on the session id,
        // not on whether eager resume is theoretically possible.
        let store = make_store();
        let c = controller("blk-no-sid").with_identity_stores(Some(store.clone()), Some(store), "key".to_string());
        let meta = MetaMapType::new(); // no agent:sessionid
        let result = Controller::start(&c, meta, None, false);
        assert!(result.is_ok());
        assert!(c.inner.lock().unwrap().current_pid.is_none());
    }

    #[tokio::test]
    async fn does_not_spawn_when_identity_stores_not_configured() {
        // A controller constructed via plain `new()` (no
        // `with_identity_stores()`) must decline to eager-resume rather than
        // spawn WITHOUT the gate — the safe-by-default case.
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let stub = write_stub();
        let c = controller("blk-no-stores");
        let meta = meta_with_session("sid-123", &[stub.to_string_lossy().as_ref()]);
        let result = Controller::start(&c, meta, None, false);
        assert!(result.is_ok());
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            c.inner.lock().unwrap().current_pid.is_none(),
            "must not spawn without identity stores configured, even though node was available"
        );
    }

    // `flavor = "multi_thread"`: this test's spawn attempt reaches
    // `try_eager_resume`'s `tokio::task::block_in_place` call — which
    // panics outright on the default current-thread test runtime (fewer
    // than one worker thread to hand off to). Production always runs
    // multi-threaded (`#[tokio::main]`, no flavor override), so this is
    // what actually reproduces the real call context, not a workaround.
    #[tokio::test(flavor = "multi_thread")]
    async fn declines_when_gate_denies() {
        // The security property this whole change exists for: an agent
        // whose bound account can never resolve (the same shape a deleted/
        // revoked account produces) must NOT be eagerly spawned, even though
        // node is on PATH and the session id is present.
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let store = make_store();
        wire_ungated_agent(&store, "blk-ungated");
        let stub = write_stub();
        let c = controller("blk-ungated").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        // mstore is also required — set directly since `new()` takes it as a
        // constructor arg, not the builder.
        let c = PersistentSubprocessController {
            mstore: Some(store),
            ..c
        };
        let meta = meta_with_session("sid-ungated", &[stub.to_string_lossy().as_ref()]);
        let result = Controller::start(&c, meta, None, false);
        assert!(result.is_ok(), "declining eager-resume must not fail the resync");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            c.inner.lock().unwrap().current_pid.is_none(),
            "the identity gate must have blocked the spawn"
        );
        // codex P1 on PR #3513 (the spawn-claim race): the exclusive claim
        // taken before the gate runs MUST be released on a decline, not
        // just on success. A leaked claim here would be worse than the
        // original bug — instead of a pane that revives on the next
        // message, it would refuse EVERY future message forever
        // (`decide_send_action` treats `spawning_in_progress: true` as
        // "something is already spawning" indefinitely).
        assert!(
            !c.inner.lock().unwrap().spawning_in_progress,
            "the spawn claim must be released when the gate declines, or this pane can never send again"
        );
    }

    // reagent P1 on PR #3523. The success path used to check "is anything
    // queued?" and kick off the drain as two SEPARATE lock acquisitions,
    // holding the spawn claim across the gap. A `send_message` landing in
    // that gap is routed to `Queued` — which deliberately does NOT publish
    // turn-active, because the spawn-claim holder is supposed to — and the
    // drain loop does not publish it either. So the message was delivered
    // while the pane, Swarm view and subagent watcher all still read idle:
    // the exact inverse of the "perpetually WORKING" bug the conditional
    // was added to prevent.
    //
    // Every other spawner publishes unconditionally before draining, so
    // only eager resume (the one spawner that routinely has nothing of its
    // own to deliver) ever had this gap.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_queue_eager_resume_releases_its_claim_before_returning() {
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let store = make_store();
        wire_bare_instance(&store, "blk-claim-sync");
        let stub = write_stub();
        let c = controller("blk-claim-sync").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        let c = PersistentSubprocessController { mstore: Some(store), ..c };
        let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
        let _kill_on_drop = KillOnDrop(&c);

        let result = Controller::start(&c, meta, None, false);
        assert!(result.is_ok(), "{result:?}");
        assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

        // Synchronously — deliberately NOT a polling loop. "Eventually
        // released, one tokio task hop later" is precisely the window the
        // bug lived in, so a test that waits for it would still pass
        // against the broken version.
        assert!(
            !c.inner.lock().unwrap().spawning_in_progress,
            "an empty-queue eager resume must release its spawn claim before start() returns"
        );

        // What that release actually buys, asserted at the mechanism that
        // matters: the next message is delivered by its own caller — and
        // `DeliverDirect` publishes turn-active itself — instead of being
        // queued behind a claim whose holder has already decided not to
        // publish for it.
        assert!(
            matches!(
                c.decide_send_action("{\"probe\":true}", None),
                SendAction::DeliverDirect
            ),
            "with the claim released and stdin live, the next send must go direct"
        );

        // And the empty-queue resume itself still reads idle — the
        // original "perpetually WORKING with nothing queued" property this
        // conditional exists for, which the fix must not regress.
        assert!(
            !c.health_monitor.is_active_turn(),
            "an eager resume with nothing queued must still not report an active turn"
        );
    }

    // See `declines_when_gate_denies`'s comment on `flavor = "multi_thread"`.
    #[tokio::test(flavor = "multi_thread")]
    async fn spawns_with_resume_flag_when_gate_passes() {
        // The happy path, end to end: a real process spawn, through the
        // SAME meta-reading code (`cmd`/`cmd:args`/`cmd:cwd`/`cmd:env`) a
        // live message send uses, gated by a real (trivially-passing)
        // identity check — not a shortcut around either.
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let store = make_store();
        wire_bare_instance(&store, "blk-eager");
        let stub = write_stub();
        let c = controller("blk-eager").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        let c = PersistentSubprocessController {
            mstore: Some(store),
            ..c
        };
        let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
        let _kill_on_drop = KillOnDrop(&c);

        let result = Controller::start(&c, meta, None, false);
        assert!(result.is_ok(), "{result:?}");
        assert!(wait_for_spawn(&c).await, "expected a real process to spawn");
        assert_eq!(c.inner.lock().unwrap().session_id.as_deref(), Some("resumed-session"));

        // codex P1 on PR #3513: no message was ever sent, so this pane must
        // read as idle, not perpetually WORKING. `spawn_process` used to
        // mark a turn active unconditionally on every spawn — correct for
        // every OTHER caller (a message is always about to flow through
        // one way or another) but wrong for eager resume, which revives a
        // session so it's ready for the next message, not mid-turn.
        assert!(
            !c.health_monitor.is_active_turn(),
            "an eager resume with nothing queued must not report an active turn"
        );
        // codex P1 on PR #3513 (the spawn-claim race): the SUCCESS path
        // must also release the claim, via `drain_queue_after_successful_
        // spawn`. A leaked claim here would refuse every future message to
        // this pane forever. That release happens inside a `tokio::spawn`ed
        // task, not synchronously before `start()` returns — `current_pid`
        // (what `wait_for_spawn` polls) is set synchronously inside
        // `spawn_process` itself, well before that task is even scheduled,
        // so this needs its own short wait rather than piggybacking on
        // `wait_for_spawn`'s already-satisfied condition.
        let claim_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if !c.inner.lock().unwrap().spawning_in_progress {
                break;
            }
            assert!(std::time::Instant::now() < claim_deadline, "spawn claim was never released");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // `_kill_on_drop` force-kills the stub on scope exit, success or
        // panic — see `KillOnDrop`'s own doc comment.
    }

    // See `declines_when_gate_denies`'s comment on `flavor = "multi_thread"`.
    #[tokio::test(flavor = "multi_thread")]
    async fn eager_resume_config_carries_the_resume_flag_and_session_id() {
        // Direct proof the built PersistentSpawnConfig is correct: the stub
        // writes its OWN received argv to a file before printing anything,
        // so this reads back exactly what the process was launched with —
        // no dependency on stdout-parsing timing (unlike asserting on
        // `session_id`, which is only captured after the init line is read
        // and races the pid becoming visible).
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let argv_out = std::env::temp_dir().join(format!("agentmux-eager-resume-argv-{}.json", uuid::Uuid::new_v4()));
        let echo_argv_stub = std::env::temp_dir().join(format!("agentmux-eager-resume-echo-{}.js", uuid::Uuid::new_v4()));
        std::fs::write(
            &echo_argv_stub,
            format!(
                r#"
require("fs").writeFileSync({argv_out:?}, JSON.stringify(process.argv.slice(2)));
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
out({{ type: "system", subtype: "init", session_id: "echoed-session" }});
setInterval(() => {{}}, 1000);
"#,
                argv_out = argv_out.to_string_lossy(),
            ),
        )
        .unwrap();

        let store = make_store();
        wire_bare_instance(&store, "blk-echo");
        let c = controller("blk-echo").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        let c = PersistentSubprocessController {
            mstore: Some(store),
            ..c
        };
        let meta = meta_with_session("sid-to-resume", &[echo_argv_stub.to_string_lossy().as_ref()]);
        let _kill_on_drop = KillOnDrop(&c);

        Controller::start(&c, meta, None, false).unwrap();
        assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let argv: Vec<String> = loop {
            if let Ok(raw) = std::fs::read_to_string(&argv_out) {
                if let Ok(parsed) = serde_json::from_str(&raw) {
                    break parsed;
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!("stub never wrote its argv file");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert!(
            argv.windows(2).any(|w| w[0] == "--resume" && w[1] == "sid-to-resume"),
            "expected --resume sid-to-resume in argv, got {argv:?}"
        );

        // `_kill_on_drop` force-kills the stub on scope exit, success or
        // panic — see `KillOnDrop`'s own doc comment.
    }

    // codex P1 on PR #3513: the controller is placed in the global registry
    // before `start()` runs, so a concurrent message can reach
    // `send_message` while `try_eager_resume` is still resolving the
    // identity gate. Without a claim held for that whole window, this
    // concurrent message would see `stdin_tx: None` and
    // `spawning_in_progress: false` and become its own spawner — a SECOND
    // process launched against the same `--resume <sid>`, silently
    // overwriting the first one's `current_pid`/`stdin_tx`/`kill_tx` while
    // it stays alive on the same conversation.
    //
    // Doesn't need real timing/concurrency to prove: `spawning_in_progress:
    // true` IS the exact state `try_eager_resume` holds for that whole
    // window (see the claim block at the top of that method). Setting it
    // directly and calling `send_message` exercises the same
    // `decide_send_action` branch a genuinely concurrent message would hit.
    #[test]
    fn a_message_arriving_while_spawning_in_progress_queues_instead_of_double_spawning() {
        let c = controller("blk-concurrent");
        c.inner.lock().unwrap().spawning_in_progress = true;

        let msg = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        let config = PersistentSpawnConfig {
            cli_command: "node".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        let result = c.send_message(msg.to_string(), config);

        assert!(result.is_ok(), "a queued message is accepted, not an error: {result:?}");
        assert!(
            c.inner.lock().unwrap().current_pid.is_none(),
            "must NOT spawn a second process while one is already claimed as spawning"
        );
        assert_eq!(
            c.inner.lock().unwrap().pending_send_messages.len(),
            1,
            "the message must be queued for the in-flight spawn to drain, not dropped"
        );
    }

    // Direct coverage for `try_claim_eager_resume_spawn` itself — see its
    // own doc comment for why this exists as a separate test rather than
    // trusting the end-to-end tests above to exercise it: mutation-tested,
    // deleting `try_eager_resume`'s call to this function (or the function's
    // own body) is exactly what those tests failed to catch.
    #[test]
    fn claim_succeeds_when_nothing_else_is_running_or_spawning() {
        let c = controller("blk-claim-free");
        assert!(c.try_claim_eager_resume_spawn());
        assert!(c.inner.lock().unwrap().spawning_in_progress);
    }

    #[test]
    fn claim_fails_when_spawning_is_already_in_progress() {
        let c = controller("blk-claim-spawning");
        c.inner.lock().unwrap().spawning_in_progress = true;
        assert!(!c.try_claim_eager_resume_spawn());
    }

    #[test]
    fn claim_fails_when_a_stdin_channel_is_already_live() {
        // A live `stdin_tx` means the process is already running — eager
        // resume must not attempt a second spawn just because
        // `spawning_in_progress` happens to be false at this instant (e.g.
        // between an earlier spawn completing and the caller having set
        // anything else).
        let (tx, _rx) = mpsc::channel::<String>(1);
        let c = controller("blk-claim-live");
        c.inner.lock().unwrap().stdin_tx = Some(tx);
        assert!(!c.try_claim_eager_resume_spawn());
    }

    #[test]
    fn claim_fails_during_a_drain() {
        let c = controller("blk-claim-draining");
        c.inner.lock().unwrap().drain_claim = true;
        assert!(!c.try_claim_eager_resume_spawn());
    }

    // End-to-end wiring check, deterministic rather than timing-dependent:
    // the four tests above prove `try_claim_eager_resume_spawn` itself is
    // correct in isolation, but none of the OTHER eager-resume tests would
    // have caught `try_eager_resume` simply never calling it — confirmed by
    // mutation, removing that one call left every other test in this module
    // still passing. This drives the claim through the REAL `start()` entry
    // point instead of calling the claim function directly, so a future
    // regression that skips the call (not just breaks the function) fails
    // here.
    //
    // Pre-claiming `spawning_in_progress` before `start()` runs stands in
    // for "another spawn raced in first" without needing to actually win a
    // timing race: same effect the eager-resume path itself achieves by
    // claiming BEFORE its identity gate work, on a real concurrent message.
    // The identity gate and stub here are otherwise IDENTICAL to
    // `spawns_with_resume_flag_when_gate_passes`, which proves this exact
    // setup DOES spawn when unclaimed — so `current_pid` staying `None`
    // here is real evidence of the early decline, not an artifact of a
    // broken fixture.
    #[tokio::test(flavor = "multi_thread")]
    async fn start_declines_the_whole_attempt_when_the_claim_is_already_held() {
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let store = make_store();
        wire_bare_instance(&store, "blk-preclaimed");
        let stub = write_stub();
        let c = controller("blk-preclaimed").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        let c = PersistentSubprocessController {
            mstore: Some(store),
            ..c
        };
        c.inner.lock().unwrap().spawning_in_progress = true;

        let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
        // Correct behavior never spawns anything here, so there's normally
        // nothing for this to clean up — it's for a FUTURE regression that
        // reintroduces the bug this test catches: without it, that failure
        // mode is a leaked real process hanging the whole test binary's
        // shutdown (see `KillOnDrop`'s own doc comment), not a clean
        // assertion failure.
        let _kill_on_drop = KillOnDrop(&c);
        let result = Controller::start(&c, meta, None, false);
        assert!(result.is_ok(), "declining must not fail the resync: {result:?}");

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            c.inner.lock().unwrap().current_pid.is_none(),
            "must not spawn at all while another spawn's claim is already held"
        );
    }

    // codex P1 + reagent P1 on PR #3523, found independently by both: the
    // SIBLING of the test below. That one covers `send_message`'s
    // `DeliverDirect`, the path a human typing in the UI takes. This covers
    // `send_user_message` — the MuxBus/reactive injection path
    // (`deliver_agent_message`), which writes straight to stdin and was
    // never appended to the retry batch at all.
    //
    // Same consequence, worse optics: the caller was told delivery
    // succeeded and the transcript already rendered the message to the
    // operator, so a stale resume silently loses a prompt everyone has
    // been told landed.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_injected_message_after_eager_resume_is_tracked_for_stale_resume_retry() {
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let store = make_store();
        wire_bare_instance(&store, "blk-track-injected");
        let stub = write_stub();
        let c = controller("blk-track-injected").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        let c = PersistentSubprocessController { mstore: Some(store), ..c };
        let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
        let _kill_on_drop = KillOnDrop(&c);

        Controller::start(&c, meta, None, false).unwrap();
        assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

        // Precondition: an unconfirmed `--resume` with an empty batch — the
        // state in which losing an injected message is possible at all.
        {
            let inner = c.inner.lock().unwrap();
            match &inner.resume {
                persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
                    assert!(retry.messages.is_empty(), "nothing delivered yet");
                }
                other => panic!("expected AwaitingOutcome, got {other:?}"),
            }
        }

        // Raw text, not a stream-json envelope — `send_user_message` does
        // the wrapping itself, same as `send_message`.
        let msg = "injected from muxbus";
        c.send_user_message(msg.to_string()).unwrap();

        let inner = c.inner.lock().unwrap();
        match &inner.resume {
            persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
                assert_eq!(
                    retry.messages.len(),
                    1,
                    "the injected message must be tracked into the retry batch, not silently dropped"
                );
                let tracked: serde_json::Value = serde_json::from_str(&retry.messages[0].json)
                    .expect("tracked entry must be valid JSON");
                assert_eq!(tracked["message"]["content"], msg);
            }
            other => panic!("expected AwaitingOutcome with the tracked message, got {other:?}"),
        }
    }

    // codex P1 on PR #3513 (re-review): eager resume attempts `--resume
    // <sid>` but starts with no message of its own to seed the retry batch
    // with (unlike every other resume-respawn, which always has one). If
    // that resume turns out to be stale and a direct message arrives before
    // the failure is detected, the message must still be tracked so a
    // confirmed stale-resume retry can redeliver it — otherwise it is
    // silently lost when the doomed process exits. Proven at the STATE
    // level rather than by actually killing the process and racing the
    // stderr reader: this is really two composed fixes (spawn_process's
    // event-routing, and DeliverDirect's own tracking call), and asserting
    // on `ResumeState` directly pins each independently of the other's
    // timing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_direct_message_after_eager_resume_is_tracked_for_stale_resume_retry() {
        if !has_node() {
            eprintln!("eager_resume_tests: `node` not on PATH — skipping");
            return;
        }
        let store = make_store();
        wire_bare_instance(&store, "blk-track-direct");
        let stub = write_stub();
        let c = controller("blk-track-direct").with_identity_stores(
            Some(store.clone()),
            Some(store.clone()),
            "key".to_string(),
        );
        let c = PersistentSubprocessController {
            mstore: Some(store),
            ..c
        };
        let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
        let _kill_on_drop = KillOnDrop(&c);

        Controller::start(&c, meta, None, false).unwrap();
        assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

        // The resume attempt must be tracked as AwaitingOutcome (not
        // NotTracking/SpawnedFresh) even with nothing delivered yet — this
        // is the `spawn_process` routing half of the fix.
        {
            let inner = c.inner.lock().unwrap();
            match &inner.resume {
                persistent_resume::ResumeState::AwaitingOutcome { attempted_sid, retry, .. } => {
                    assert_eq!(attempted_sid, "resumed-session");
                    assert!(retry.messages.is_empty(), "no seed message — nothing delivered yet");
                }
                other => panic!("expected AwaitingOutcome with no messages yet, got {other:?}"),
            }
        }

        // A direct message now must land in that SAME tracking state — the
        // DeliverDirect half of the fix.
        //
        // Plain raw text, NOT a pre-formatted stream-json envelope —
        // `send_message` does that wrapping itself (`content: message`).
        // Every OTHER test in this file passing a full envelope as `message`
        // (e.g. `shutdown_tests::start()`) gets away with it because those
        // stubs only ever check the outer `m.type`, never `m.message.
        // content` — so the resulting double-wrap is invisible to them.
        // THIS test asserts on the tracked entry's actual content, so it
        // needs the API used as a real caller (a human typing "hi") would.
        let msg = "hi";
        let config = PersistentSpawnConfig {
            cli_command: "node".to_string(),
            cli_args: vec![],
            working_dir: String::new(),
            env_vars: HashMap::new(),
            session_id_field: "session_id".to_string(),
            resume_flag: "--resume".to_string(),
            session_id: String::new(),
            message_id: None,
        };
        c.send_message(msg.to_string(), config).unwrap();

        let inner = c.inner.lock().unwrap();
        match &inner.resume {
            persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
                assert_eq!(
                    retry.messages.len(),
                    1,
                    "the direct message must be tracked into the retry batch, not silently dropped"
                );
                // Parse and check the semantic content rather than hardcode
                // `send_message`'s exact wrapped-string format — this proves
                // the RIGHT message was tracked without this test being
                // fragile to that internal format ever changing.
                let tracked: serde_json::Value = serde_json::from_str(&retry.messages[0].json)
                    .expect("tracked entry must be valid JSON");
                assert_eq!(tracked["message"]["content"], msg);
            }
            other => panic!("expected AwaitingOutcome with the tracked message, got {other:?}"),
        }
    }
}
