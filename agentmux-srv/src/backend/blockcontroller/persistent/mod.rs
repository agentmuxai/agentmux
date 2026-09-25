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
//!
//! Module layout (split 2026-09-22; one `impl PersistentSubprocessController`
//! block per file, all private methods `pub(super)`):
//!   - `mod.rs`          — types, shared helpers, construction, `Controller` impl
//!   - `status.rs`       — runtime status snapshot/publication and heartbeat
//!   - `eager_resume.rs` — spawn-on-construction when a session id is on file
//!   - `queue.rs`        — `send_message`, spawn claim, queue drain/replay
//!   - `resume_retry.rs` — stale-`--resume` recovery effects and retry batch
//!   - `input.rs`        — user messages, question answers, tool decisions, stdin
//!   - `spawn.rs`        — `spawn_process` and the per-session I/O tasks
//!   - `lifecycle.rs`    — stop/restart, `session_id`, `needs_spawn`
//!   - `tests/`          — one file per former inline test module

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::{
    BlockControllerRuntimeStatus, BlockInputUnion, Controller, DeliverPolicy, SendOutcome, STATUS_DONE,
    STATUS_INIT, STATUS_RUNNING,
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

/// The one spawn refusal that is a *decision*, not a failure: `--resume
/// <sid>` was asked for while another pane still holds (or is still
/// closing) that same conversation — `session_held_elsewhere`. Built here
/// so that `is_held_elsewhere_error` is checked against the same text
/// `spawn_process` actually returns, rather than a second copy that could
/// drift; `tests/reopen_guard.rs` pins the wording users see.
pub(super) fn held_elsewhere_error(other: &str, closing: bool) -> String {
    if closing {
        format!("This conversation's previous process (block {other}) is still shutting down. Try again in a few seconds.")
    } else {
        format!(
            "This conversation is already open in another pane (block {other}). Close it there, or switch to it, instead of opening a second copy."
        )
    }
}

/// Did `spawn_process` refuse because of `session_held_elsewhere`? The
/// answer decides whether a caller may fall back to a FRESH spawn: for any
/// other failure a blank session is a reasonable recovery, but for this one
/// it silently hands the user's accepted prompt to a new conversation
/// while the real one is open next door — exactly what the guard exists to
/// prevent (codex P1 on PR #3551).
pub(super) fn is_held_elsewhere_error(err: &str) -> bool {
    err.starts_with("This conversation's previous process (block ")
        || err.starts_with("This conversation is already open in another pane (block ")
}

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
    session_outcome_line_with(outcome, attempted_sid, actual_sid, false)
}

/// [`session_outcome_line`], plus whether the fresh session was given
/// AgentMux's record of the conversation (`continued`, SPEC_DURABLE_
/// CONVERSATION_MEMORY_2026_09_23.md §4.8). The outcome stays `fresh`: the
/// provider session is new, and every consumer that scopes scrollback on
/// `fresh` keeps doing so. Only the pane's label changes.
fn session_outcome_line_with(
    outcome: persistent_resume::SessionOutcome,
    attempted_sid: String,
    actual_sid: Option<String>,
    continued: bool,
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
            "continued": continued,
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

/// How much of a transcript's tail `find_continuation_session_id` reads. The
/// provider stamps its session id on every stream event, so the last few
/// events suffice; a long-lived agent's transcript runs to many MB.
const CONTINUATION_TAIL_BYTES: i64 = 256 * 1024;

/// The last non-empty `field` on a complete JSON line of `tail`.
/// `starts_mid_line`: `tail` was cut from a longer file, so its first line is
/// a fragment and is dropped.
fn last_session_id_in_stream(tail: &[u8], starts_mid_line: bool, field: &str) -> Option<String> {
    let text = String::from_utf8_lossy(tail);
    let skip = usize::from(starts_mid_line);
    let lines: Vec<&str> = text.split('\n').skip(skip).collect();
    lines.into_iter().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        let sid = v.get(field)?.as_str()?.trim();
        (!sid.is_empty()).then(|| sid.to_string())
    })
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
    /// A kill has been requested for the current process (`request_stop_on`).
    /// `stdin_tx` stays live until it actually exits, and a `result` already
    /// in the pipe still reaches the turn-boundary flush. Writing a deferred
    /// message then would put it into the dying process and lose it (codex
    /// P2 on #3562 for `stop`, and the same window on the Stop button).
    /// `try_write_stdin_locked` refuses while this is set, so the message
    /// stays queued for the next process.
    ///
    /// Set under the same lock as the kill request, cleared when the
    /// replacement process spawns, exactly like `restart_pending`.
    stop_pending: bool,
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
    /// Non-human messages (jekt, muxbus, bridges, MCP `SendMessage`) that
    /// arrived while a turn was in flight and were deferred to the next turn
    /// boundary rather than steering the agent mid-explanation.
    /// Spec: `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.
    ///
    /// Distinct from `pending_send_messages`, which orders messages around a
    /// *spawn*; this one orders them around a *turn*. Entries are the fully
    /// encoded stream-json stdin lines, ready to write.
    ///
    /// This field and the `turn_active` flag it coordinates with MUST be read
    /// and written under this one mutex — §4.2. `PersistentInner`'s lock is
    /// what serializes them, because the turn-end handler in `spawn.rs`
    /// already calls `health_monitor.set_active_turn(false)` while holding it.
    /// Splitting them across two locks reintroduces the stranding race in
    /// which a message enqueued just as a flush observes the queue empty has
    /// no future trigger.
    deferred_deliveries: VecDeque<String>,
    /// Whether a deferred-delivery watchdog task is running for this
    /// controller — see `ensure_deferred_watchdog`. Set and cleared only
    /// under this lock, and cleared only by the watchdog itself when it
    /// finds the queue empty, so an enqueue can never observe "armed" from a
    /// watchdog that has already decided to exit.
    deferred_watchdog_armed: bool,
    /// Whether the model is writing or waiting on its tool calls — see
    /// [`ToolWait`]. Changed only under this lock: by the stdout reader
    /// (`tool_wait_signal`), by each release, by a `result`, and by every
    /// spawn. Spec: `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.7.
    tool_wait: ToolWait,
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
    /// A recovery candidate adopted by `settle_empty_resume_retry` while
    /// accepted prompts were still queued behind the eager spawn claim
    /// (codex P1 on PR #3551, fifth round). Those prompts were never
    /// delivered to the doomed process, so they are not in the retry batch
    /// — the eager path's own drain will find the dead `stdin_tx`, stall,
    /// and hand them to `respawn_once_for_leftover_queue`, whose contract
    /// is otherwise "spawn fresh". This tells that respawn to `--resume`
    /// the candidate instead, so a recoverable session is not thrown away
    /// for a prompt that happened to arrive during the eager attempt.
    /// Consumed by that respawn; invalidated by any spawn (`spawn_process`
    /// clears it under the generation bump), since a candidate only means
    /// anything for the spawn that immediately follows its adoption.
    leftover_resume_candidate: Option<String>,
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
    /// The single-live-instance lease the current CLI process holds
    /// (`backend::agent_admission`, SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24
    /// Phase 1). Set by `spawn_process` once the child is running; cleared —
    /// which releases it — by the process-exit arms for the current
    /// generation. An `Arc` so the pre-turn fence can verify it without
    /// holding this lock across file I/O.
    agent_lease: Option<Arc<crate::backend::agent_admission::HeldAgentLease>>,
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
    /// decision involved at all. The turn is already marked active: the
    /// decision reserves it under the same `inner` acquisition, so an
    /// automated send cannot see "idle" in the gap before the write (codex P1
    /// on #3562). `was_active` is the pre-reservation state, for the caller
    /// to start the heartbeat and to hand the turn back if the write fails.
    DeliverDirect { was_active: bool },
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
    /// A kill is pending and the dying process still holds stdin: neither
    /// written nor queued, reported to the sender as this error instead.
    /// See `decide_send_action`.
    Refused(String),
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
    /// Identity M2: `AGENTMUX_AGENT_UID` from the spawn env, captured at
    /// spawn exactly like `stable_agent_id`. `None` until spawned, or when
    /// the spawn env carried no UID (no `db_agents` row yet). See
    /// `Controller::stable_agent_uid`.
    stable_agent_uid: Mutex<Option<String>>,
    /// The segment the latest spawn recorded (`segments.rs`), so a resume
    /// retry can tell the agent's previous segment from the one that just
    /// failed. `None` until a spawn with an agent UID.
    current_segment: Mutex<Option<String>>,
    /// Reports whatever is still deferred when this controller is dropped.
    /// See [`DeferredDropReport`].
    deferred_drop_report: DeferredDropReport,
    /// Where this agent's single-live-instance lease lives, and this srv's
    /// boot id (the lease owner). Set via `with_agent_lease_store()`; `None`
    /// (tests, a registry that can't be opened) means no lease is taken.
    lease_store: Option<Arc<crate::registry::LeaseStore>>,
    boot_id: Arc<str>,
}

/// The last line of defence for §4.5: whatever is still deferred when the
/// controller goes away is reported, not silently discarded with it.
///
/// `stop()` and `shutdown()` drain the queue, but a sender that fetched the
/// controller before it was unregistered can still enqueue after that drain
/// (codex P2 on #3562). The watchdog holds only a weak reference, so it dies
/// with the controller and cannot catch it. `stop()` cannot refuse those late
/// senders instead: the max-runtime watchdog stops controllers that stay
/// registered and get used again.
///
/// A field rather than `Drop` on the controller itself, so struct-update
/// construction (`..c`, used throughout the tests) keeps working. Holds the
/// controller's own `inner`; `None` only transiently inside `new()`.
struct DeferredDropReport(Option<(String, Arc<Mutex<PersistentInner>>)>);

impl Drop for DeferredDropReport {
    fn drop(&mut self) {
        let Some((block_id, inner)) = self.0.take() else { return };
        // Never panic in `drop`: a poisoned lock still holds the queue.
        let stranded: Vec<String> = match inner.lock() {
            Ok(mut g) => g.deferred_deliveries.drain(..).collect(),
            Err(poisoned) => poisoned.into_inner().deferred_deliveries.drain(..).collect(),
        };
        PersistentSubprocessController::log_stranded_deferred(&block_id, "controller dropped", &stranded);
    }
}

/// How long to wait after delivering an AskUserQuestion answer before assuming
/// the turn did not resume and re-delivering the answer as a follow-up message.
/// See `answer_question` and SPEC_ASK_USER_QUESTION_2026_06_15.md §10.1.
const ANSWER_RESUME_FALLBACK_MS: u64 = 4000;

/// Bound on `PersistentInner::deferred_deliveries`. Past this, enqueueing
/// returns an error instead of a false success, so a caller is never told a
/// message was delivered and then has it silently dropped
/// (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.5). Senders already handle
/// transient delivery failure — the reactive handler, muxbus and the bridges
/// all have to cope with an unreachable target instance — so surfacing a full
/// queue as one more retryable failure needs no new machinery on their side.
///
/// 64 is chosen to be far above any plausible legitimate backlog for a single
/// turn while still bounding memory: these are whole messages, and the queue
/// is per-block.
pub(super) const MAX_DEFERRED_DELIVERIES: usize = 64;

/// How often the deferred-delivery watchdog re-checks a non-empty queue.
/// It is the safety net, not the primary path: the `result`-frame flush in
/// `spawn.rs` releases a message the instant a turn ends, so this only sets
/// the latency of the cases that have no turn boundary to wait for.
pub(super) const DEFERRED_WATCHDOG_TICK: std::time::Duration = std::time::Duration::from_millis(500);

/// Consecutive watchdog ticks with no process and no spawn in flight before
/// the queue is declared stranded and reported (spec §4.5). Long enough to
/// ride out the gap between a process exit and the respawn that follows it
/// (a stale-`--resume` retry, a fallback respawn); short enough that a dead
/// agent's backlog is surfaced rather than held forever.
pub(super) const DEFERRED_ORPHAN_GRACE_TICKS: u32 = 20;

/// Outcome of [`PersistentSubprocessController::flush_one_deferred_locked`].
/// `Empty`, `Held` and `Failed` all write nothing, but they are not the same:
/// `Empty` lets the turn go idle, while `Held` (another writer owns stdin) and
/// `Failed` (the write itself failed) still have an accepted message waiting
/// and need the watchdog to finish it (codex + reagent P1s on #3562).
#[derive(Debug, PartialEq)]
pub(super) enum DeferredFlush {
    Empty,
    Released(String),
    Held,
    Failed,
}

/// Where the current turn is, as far as an automated message is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ToolWait {
    /// The model is (or may be) producing text or thinking. Messages wait.
    #[default]
    Writing,
    /// The model has finished writing and its tool calls are running. One
    /// message may be written now: the tool is not something to cut into, and
    /// the model reads it when the tool returns.
    Open,
    /// This tool wait already released its one message. The rest wait for the
    /// next one, or for the turn to end.
    Spent,
    /// The model called a tool that parks for the operator's answer
    /// ([`tool_parks_for_operator`]). Nothing is released: the agent is blocked
    /// on a person, and a message would land between the question and its
    /// answer. Set from the tool call's own `assistant` line, which precedes
    /// both the `message_delta` that would open the wait and the
    /// `control_request` that fills `pending_questions`, so the gate never
    /// depends on the park having happened yet.
    Blocked,
}

/// Whether a stdout line says the model is waiting on tools or is writing again.
#[derive(Debug, PartialEq)]
pub(super) enum ToolWaitSignal {
    /// A tool call is complete and running: the model is not producing text.
    Enter,
    /// The model is producing output again (or the turn moved on).
    Leave,
    /// The model called a tool that waits on the operator, not on a program.
    Blocked,
}

/// Whether a tool call parks for the operator's answer rather than running.
/// Must agree with `handle_control_frame`, which is what actually parks: an
/// AskUserQuestion always, and any tool `should_route_to_decision_panel` routes.
fn tool_parks_for_operator(tool_name: &str) -> bool {
    tool_name == "AskUserQuestion" || should_route_to_decision_panel(tool_name)
}

/// Classify one stdout line. Only the agent's own top-level output counts: a
/// subagent's lines (`parent_tool_use_id` set) say nothing about whether the
/// parent is mid-sentence.
///
/// `Enter` only on a `message_delta` whose `stop_reason` is `tool_use`: the
/// whole message, every tool call in it, has been written. `Blocked` on an
/// `assistant` line calling a tool that parks for the operator (it precedes the
/// `message_delta`, so `Blocked` is always seen first). `Leave` on
/// `message_start` or an `assistant` line with text or thinking. Anything
/// unrecognised is `None`, which keeps the previous state, and the default
/// state is "writing", so an unknown stream shape holds messages and never
/// releases them early.
pub(super) fn tool_wait_signal(parsed: &serde_json::Value) -> Option<ToolWaitSignal> {
    if parsed.get("parent_tool_use_id").is_some_and(|v| !v.is_null()) {
        return None;
    }
    match parsed.get("type")?.as_str()? {
        "stream_event" => {
            let event = parsed.get("event")?;
            match event.get("type")?.as_str()? {
                "message_start" => Some(ToolWaitSignal::Leave),
                "message_delta"
                    if event.pointer("/delta/stop_reason").and_then(|v| v.as_str())
                        == Some("tool_use") =>
                {
                    Some(ToolWaitSignal::Enter)
                }
                _ => None,
            }
        }
        "assistant" => {
            let blocks = parsed.pointer("/message/content")?.as_array()?;
            fn kind(b: &serde_json::Value) -> Option<&str> {
                b.get("type").and_then(|t| t.as_str())
            }
            let parks = blocks.iter().any(|b| {
                kind(b) == Some("tool_use")
                    && b.get("name").and_then(|n| n.as_str()).is_some_and(tool_parks_for_operator)
            });
            if parks {
                Some(ToolWaitSignal::Blocked)
            } else if blocks
                .iter()
                .any(|b| matches!(kind(b), Some("text" | "thinking" | "redacted_thinking")))
            {
                Some(ToolWaitSignal::Leave)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// What a current-generation `result` frame decided — see
/// [`PersistentSubprocessController::turn_boundary_locked`].
#[derive(Debug, PartialEq)]
pub(super) struct TurnBoundary {
    pub(super) flushed: DeferredFlush,
    pub(super) apply_deferred_restart: bool,
}

impl TurnBoundary {
    /// `Released` started a turn; `Held` means an earlier writer's prompt is
    /// still running or about to. Either way the turn is not over.
    pub(super) fn turn_still_active(&self) -> bool {
        matches!(self.flushed, DeferredFlush::Released(_) | DeferredFlush::Held)
    }
}

/// Whether the deferred-delivery watchdog keeps running after a tick.
#[derive(Debug, PartialEq)]
pub(super) enum WatchdogStep {
    Continue,
    Exit,
}

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
        let mut this = Self {
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
                stop_pending: false,
                resume: persistent_resume::ResumeState::default(),
                spawning_in_progress: false,
                pending_send_messages: VecDeque::new(),
                deferred_deliveries: VecDeque::new(),
                deferred_watchdog_armed: false,
                tool_wait: ToolWait::Writing,
                drain_claim: false,
                next_message_seq: 0,
                drain_send_in_flight: false,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                shutdown_generation: None,
                stop_exit: None,
                spawn_generation: 0,
                leftover_resume_candidate: None,
                pending_questions: HashMap::new(),
                pending_permissions: HashMap::new(),
                agent_lease: None,
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
            stable_agent_uid: Mutex::new(None),
            current_segment: Mutex::new(None),
            deferred_drop_report: DeferredDropReport(None),
            lease_store: None,
            boot_id: Arc::from(""),
        };
        this.deferred_drop_report =
            DeferredDropReport(Some((this.block_id.clone(), Arc::clone(&this.inner))));
        this
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

    /// Supplies the single-live-instance lease store (under the shared agent
    /// registry root) and this srv's boot id. A separate builder step for
    /// the same reason as `with_identity_stores`. Without it the controller
    /// takes no lease — the pre-Phase-1 behaviour.
    pub fn with_agent_lease_store(
        mut self,
        registry: Option<Arc<crate::registry::Registry>>,
        boot_id: Arc<str>,
    ) -> Self {
        self.lease_store = crate::backend::agent_admission::lease_store_for(registry);
        self.boot_id = boot_id;
        self
    }

    /// Claim (or keep) this agent's single-live-instance lease for the
    /// process `spawn_process` is about to start. `Ok(None)` when leasing is
    /// unavailable (no store) or the spawn env carries no agent UID — those
    /// run unguarded, as before Phase 1, and are logged. `Err` is the
    /// user-facing refusal: another instance runs this agent.
    ///
    /// A lease this controller already holds (an earlier generation's, still
    /// valid) is reused rather than re-claimed: a fresh handle for the same
    /// key would release the shared file when the old one dropped.
    pub(super) fn acquire_agent_lease(
        &self,
        config: &PersistentSpawnConfig,
        session_hint: Option<&str>,
    ) -> Result<Option<Arc<crate::backend::agent_admission::HeldAgentLease>>, String> {
        let Some(store) = self.lease_store.as_ref() else { return Ok(None) };
        let uid = config
            .env_vars
            .get("AGENTMUX_AGENT_UID")
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty());
        let Some(uid) = uid else {
            tracing::warn!(
                block_id = %self.block_id,
                "agent_admission: spawn env carries no agent UID — no single-instance lease for this process"
            );
            return Ok(None);
        };
        let existing = self.inner.lock().unwrap().agent_lease.clone();
        if let Some(existing) = existing {
            existing.verify()?;
            return Ok(Some(existing));
        }
        let agent = muxbus_agent_id_from_env(&config.env_vars).unwrap_or_default();
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let held = crate::backend::agent_admission::acquire(
            store,
            &uid,
            &agent,
            &self.boot_id,
            &self.block_id,
            session_hint,
            move |why| Self::stop_after_lease_loss(&inner, &block_id, &why),
        )?;
        Ok(Some(Arc::new(held)))
    }

    /// The early admission check (spec I9) for this controller's agent:
    /// refuses while another instance holds it. Runs before any shared write.
    pub(crate) async fn check_admission(&self, uid: &str, agent: &str) -> Result<(), String> {
        crate::backend::agent_admission::check_before_spawn(
            self.lease_store.clone(),
            uid,
            agent,
            &self.boot_id,
        )
        .await
    }

    /// The pre-turn fence (spec I4): `Err` if this pane's CLI process lost
    /// its lease to another instance. Also stops that process, so it cannot
    /// keep driving the agent. `Ok` when no lease is held (nothing spawned
    /// yet — the spawn will claim — or leasing is unavailable).
    pub(super) fn fence_check(&self) -> Result<(), String> {
        let lease = self.inner.lock().unwrap().agent_lease.clone();
        let Some(lease) = lease else { return Ok(()) };
        if let Err(e) = lease.verify() {
            Self::stop_after_lease_loss(&self.inner, &self.block_id, &e);
            return Err(e);
        }
        Ok(())
    }

    /// Hand this pane's agent over to another AgentMux instance that asked
    /// for it (spec §4.6, `POST /agentmux/agent/release`): if this pane's CLI
    /// process holds the agent's single-live-instance lease, stop it
    /// gracefully — stdin EOF, so a turn in flight finishes, then kill at
    /// the deadline — and let its exit release the lease. `false` when this
    /// pane holds nothing (no process, or no lease).
    pub fn release_to_other_instance(&self, deadline: std::time::Instant) -> bool {
        let kill = {
            let mut inner = self.inner.lock().unwrap();
            if inner.agent_lease.is_none() {
                return false;
            }
            inner.kill_tx.take()
        };
        tracing::info!(block_id = %self.block_id, "agent_admission.takeover: handing this agent to another instance");
        match kill {
            Some(tx) => tx.send(KillRequest::Graceful(deadline)).is_ok(),
            None => false,
        }
    }

    /// Kill the current CLI process after its lease was lost — the holder
    /// stops rather than racing the new one (spec §4.3).
    fn stop_after_lease_loss(inner: &Arc<Mutex<PersistentInner>>, block_id: &str, why: &str) {
        let kill = inner.lock().unwrap().kill_tx.take();
        tracing::error!(block_id, why, "agent_admission.fenced: stopping this pane's CLI process");
        if let Some(tx) = kill {
            let _ = tx.send(KillRequest::Force);
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
        // Not `stop_process` then a separate drain: see
        // `request_stop_draining_deferred` for the race between the two.
        let stranded = self.request_stop_draining_deferred(KillRequest::Force);
        Self::log_stranded_deferred(&self.block_id, "controller stopped", &stranded);
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
            let (running, stranded) = {
                let mut g = inner.lock().unwrap();
                // Drained BEFORE the no-process return: a message deferred
                // behind a spawn that then failed sits here with no process,
                // and the watchdog holds only a weak ref, so it dies with this
                // controller. Returning first discarded the message unreported
                // (codex P2 on #3562).
                let stranded: Vec<String> = g.deferred_deliveries.drain(..).collect();
                let running = if g.current_pid.is_none() {
                    // Never spawned (lazy), or already gone. A spawn still in
                    // flight is killed by the caller's tracker drop.
                    None
                } else {
                    // Gate writes now, in the same section as the drain, not
                    // later in `request_stop_on`: a sender that already holds
                    // this controller can still enqueue, and the interrupt's
                    // own `result` would flush that into the process being
                    // shut down (codex P2 on #3562). The interrupt itself
                    // goes out on `stdin_tx` directly, not through the gate.
                    g.stop_pending = true;
                    // Nothing queued may start a new turn after the interrupt.
                    g.pending_send_messages.clear();
                    // Before the interrupt: its `is_error` result is our stop,
                    // not a failure (§9.4).
                    g.shutdown_generation = Some(g.spawn_generation);
                    g.stop_exit = None;
                    Some((g.spawn_generation, g.stdin_tx.clone()))
                };
                (running, stranded)
            };
            Self::log_stranded_deferred(&block_id, "pane shutdown", &stranded);
            let Some((generation, stdin_tx)) = running else {
                return StopOutcome::NotRunning;
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

    fn turn_provenance(&self) -> Option<crate::backend::blockcontroller::health::TurnProvenance> {
        self.health_monitor.provenance()
    }

    fn set_agent_id(&self, id: Option<String>) {
        *self.agent_id.lock().unwrap() = id;
    }

    fn stable_agent_id(&self) -> Option<String> {
        self.stable_agent_id.lock().unwrap().clone()
    }

    fn stable_agent_uid(&self) -> Option<String> {
        self.stable_agent_uid.lock().unwrap().clone()
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

mod eager_resume;
mod input;
mod lifecycle;
mod queue;
mod resume_retry;
mod segments;
pub(crate) use resume_retry::pane_history_session_id;
mod spawn;
mod status;

#[cfg(test)]
mod tests;
