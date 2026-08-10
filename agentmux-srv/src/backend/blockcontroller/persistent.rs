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
use super::persistent_resume;
use crate::backend::eventbus::EventBus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::subagent_watcher;
use crate::backend::wps;

/// WPS file subject name for persistent subprocess output.
pub const PERSISTENT_OUTPUT_SUBJECT: &str = "output";

pub const BLOCK_CONTROLLER_PERSISTENT: &str = "persistent";

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
                resume: persistent_resume::ResumeState::default(),
                spawning_in_progress: false,
                pending_send_messages: VecDeque::new(),
                drain_claim: false,
                next_message_seq: 0,
                drain_send_in_flight: false,
                current_pid: None,
                stdin_tx: None,
                kill_tx: None,
                spawn_generation: 0,
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
        if inner.stdin_tx.is_some() && !inner.spawning_in_progress && !inner.drain_claim {
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
        let global_output_zone = super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
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
            core::persist_last_failure(&self.block_id, Some(&failure), &self.wstore, &self.event_bus);
            broker.publish(wps::WaveEvent {
                event: wps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&failure).ok(),
            });
        }
    }

    /// `retry_generation` is the spawn generation whose `ProcessExited`
    /// fired this retry (the process-waiter's own `my_generation_wait`) —
    /// `decide_retry_batch_action` needs it to tell a live NEWER spawn
    /// apart from the impossible "our own process is somehow still
    /// running" case (issue #2367).
    fn retry_after_resume_failure(
        &self,
        retry_generation: u64,
        mut config: PersistentSpawnConfig,
        mut entries: Vec<persistent_resume::QueuedRetryEntry>,
        held_error_line: Option<String>,
    ) {
        config.session_id = String::new();
        let Some(first) = (!entries.is_empty()).then(|| entries.remove(0)) else {
            // Nothing to retry at all (shouldn't happen in practice —
            // the batch always has at least the triggering message) —
            // but if it ever does, this is a no-op, not a launch, so any
            // held error line must still reach the user.
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
                let spawn_result = self.spawn_process(config, None);
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
    fn spawn_process(
        &self,
        config: PersistentSpawnConfig,
        resume_retry_payload: Option<persistent_resume::QueuedRetryEntry>,
    ) -> Result<(), String> {
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
        let (my_generation, superseded_effects) = {
            let mut inner = self.inner.lock().unwrap();
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
                _ => inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation }),
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
            crate::backend::process_tracker::registry::track_spawned(&self.block_id, pid);
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
            // `_with_nonce` variants record this spawn's process-wide
            // registration nonce so this exact spawn's exit-handler can
            // compare-and-remove its own registrations instead of
            // blindly wiping a fallback respawn's (or replacement
            // controller's) fresh ones (issue #2363; codex P1 on PR
            // #2500 for why not the controller-local generation).
            match crate::backend::reactive::get_global_handler()
                .register_agent_with_nonce(agent_id, &self.block_id, Some(&self.tab_id), my_registration_nonce)
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
        let my_generation_read = my_generation;
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
                    let is_error_result = is_result_frame
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
                                core::persist_session_id(&block_id_read, &sid_string, &wstore_read, &event_bus_read);
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
                                            core::persist_last_failure(&block_id_read, Some(&failure), &wstore_read, &event_bus_read);
                                            if let Some(ref broker) = broker_read {
                                                broker.publish(wps::WaveEvent {
                                                    event: wps::EVENT_AGENT_FAILURE.to_string(),
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
                                            core::persist_last_failure(&block_id_read, Some(&failure), &wstore_read, &event_bus_read);
                                            if let Some(ref broker) = broker_read {
                                                broker.publish(wps::WaveEvent {
                                                    event: wps::EVENT_AGENT_FAILURE.to_string(),
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
                        core::persist_last_failure(&block_id_read, Some(&failure), &wstore_read, &event_bus_read);
                        if let Some(ref broker) = broker_read {
                            broker.publish(wps::WaveEvent {
                                event: wps::EVENT_AGENT_FAILURE.to_string(),
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
                        core::persist_last_failure(&block_id_read, None, &wstore_read, &event_bus_read);
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
        // Needed to persist a classified failure (rate-limit/overloaded/etc.)
        // into block meta alongside the WPS publish below — mirrors
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
        // This exact spawn's registration identity, for the guarded
        // muxbus/registry removals below (issue #2363 / codex P1 on PR
        // #2500 — see `my_registration_nonce`'s own doc comment).
        let nonce_wait = my_registration_nonce;

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
                    // `health_wait.set_exited` (a shared `HealthMonitor`
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
                            let data_dir = crate::backend::base::get_wave_data_dir();
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
                        if let Some(ref wstore) = wstore_wait {
                            super::session_recovery::clear_active_pid_if_pid(wstore, &block_id_wait, pid_wait);
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
                                    core::persist_last_failure(&block_id_wait, Some(&failure), &wstore_wait, &event_bus_wait);
                                    if let Some(ref broker) = broker_wait {
                                        broker.publish(wps::WaveEvent {
                                            event: wps::EVENT_AGENT_FAILURE.to_string(),
                                            scopes: vec![format!("block:{}", block_id_wait)],
                                            sender: String::new(),
                                            persist: 1,
                                            data: serde_json::to_value(&failure).ok(),
                                        });
                                    }
                                }
                            }
                            persistent_resume::ResumeEffect::FireRetry { retry, held_error_line } => {
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
                                        "stale --resume session id caused this exit — retrying fresh, without --resume"
                                    );
                                    ctrl.retry_after_resume_failure(
                                        my_generation_wait,
                                        retry.config,
                                        retry.messages,
                                        held_error_line,
                                    );
                                } else if let Some(line) = held_error_line {
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
                            persistent_resume::ResumeEffect::PublishDone => {
                                if let Some(ref broker) = broker_wait {
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
                        }
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
                        // Notify health monitor so Stalled/Dead watchdog
                        // stops — shared `Arc<HealthMonitor>` across
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
                            let data_dir = crate::backend::base::get_wave_data_dir();
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
                        if let Some(ref wstore) = wstore_wait {
                            super::session_recovery::clear_active_pid_if_pid(wstore, &block_id_wait, pid_wait);
                        }
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

    /// Shorthand for a retry-batch entry with an explicit queue seq
    /// (issue #2365 — retry batches carry identity, not just text).
    fn qentry(seq: u64, json: &str) -> persistent_resume::QueuedRetryEntry {
        persistent_resume::QueuedRetryEntry { seq, json: json.to_string() }
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None);

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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None);

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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], None);

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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], None);

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
        let broker = Arc::new(crate::backend::wps::Broker::new());
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], Some("boom\n".to_string()));

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
        let broker = Arc::new(crate::backend::wps::Broker::new());
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], Some("boom\n".to_string()));
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
        let broker = Arc::new(crate::backend::wps::Broker::new());
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
        c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], Some("boom\n".to_string()));
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
            resume: persistent_resume::ResumeState::default(),
            spawning_in_progress: false,
            pending_send_messages: VecDeque::new(),
            drain_claim: false,
            next_message_seq: 0,
            drain_send_in_flight: false,
            current_pid: None,
            stdin_tx: None,
            kill_tx: None,
            spawn_generation: 0,
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
