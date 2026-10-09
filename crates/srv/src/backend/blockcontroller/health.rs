// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-block turn-activity tracking (is a turn currently in flight, and
//! the last process exit code). Previously part of a larger "agent health"
//! detector that also did silence-based unresponsive detection; that
//! detection logic was removed (see
//! docs/specs/SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) —
//! this struct keeps only the turn-active bookkeeping, which several other
//! subsystems depend on independently of the removed detector
//! (`broker::process::lifecycle_from`, the Swarm pane's running/idle
//! badge, subagent-watcher reconciliation, `muxspect describe`).
//!
//! It also keeps the **turn ledger**: the turn as the user sees it, which
//! can span several CLI passes (`TurnLedger`,
//! docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §3, §4.3).
//! `active_turn` stays per pass; the ledger is display data on top of it.

use std::sync::{Arc, Mutex};

/// Who delivered input to an agent (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    /// The human typing in the agent's own pane.
    User,
    /// A jekt, cron, nudge, broadcast, loop, another agent — any automated sender.
    Automated,
    /// srv or the frontend on its own behalf: hidden memory reinjection, a
    /// side question.
    System,
}

/// One input, as told to the tracker by the path that delivers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnInput {
    pub origin: TurnOrigin,
    /// The message text (kept only for `User`, for the §6.3 quote check).
    pub text: String,
    /// The pane flushing a message it held while this turn ran: the turn's
    /// id, so the message joins that turn instead of starting a new one
    /// (turn-model spec §4.3, J3). Display only; provenance ignores it.
    pub joins_turn: Option<u64>,
}

/// How long after a pass ends a turn that expects more (input arrived
/// during the pass, or the CLI has a task notification queued) waits for its
/// next pass before it counts as over (turn-model spec §4.3, J4). The CLI
/// starts that pass within milliseconds; this is margin.
pub const TURN_SETTLE_MS: u64 = 2_000;

/// How long after a turn ended the pane's flush of a message it held during
/// that turn still joins it (§4.3, J3). The flush follows the idle push at
/// once; this covers a slow round trip.
pub const HELD_FLUSH_JOIN_MS: u64 = 10_000;

/// Why a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEnd {
    /// Its last pass finished and nothing joined it.
    Completed,
    /// The agent's process exited.
    Exited,
}

/// The turn as the user sees it: from leaving idle to returning to it with
/// nothing queued, over however many CLI passes that took (turn-model spec
/// §3.1, §4.3). Published as `agentturn` on every change.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct TurnLedger {
    /// Unique per block: the start time in ms, bumped past the previous id.
    pub turn_id: u64,
    /// What started the turn; `None` when its first pass was unlabelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<TurnOrigin>,
    pub started_at_ms: u64,
    /// CLI passes so far, the running one included.
    pub passes: u32,
    /// A pass is running now.
    pub active: bool,
    /// Inputs that arrived after the turn started (mid-pass, or starting a
    /// pass that joined).
    pub inputs: u32,
    /// Passes whose `result` figures are in the sums below. The pane adds
    /// its live count for the running pass only while that pass isn't
    /// counted yet, so a pass is never counted twice.
    pub counted_passes: u32,
    /// Summed over the counted passes' `result` frames.
    pub output_tokens: u64,
    pub cost_usd: f64,
    /// Model calls (`result.num_turns`, which counts steps, not turns).
    pub steps: u32,
    pub duration_api_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_pass_ended_at_ms: Option<u64>,
    /// Between passes, while a next pass is expected: the turn is over if
    /// none starts by then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle_until_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<TurnEnd>,
}

impl TurnLedger {
    fn add_stats(&mut self, stats: PassStats) {
        self.counted_passes += 1;
        self.output_tokens += stats.output_tokens;
        self.cost_usd += stats.cost_usd;
        self.steps += stats.steps;
        self.duration_api_ms += stats.duration_api_ms;
    }

    /// Whether a pass starting at `now` may still join this turn as a
    /// continuation: no pass running, not ended, and inside the settle window.
    fn settling_at(&self, now: u64) -> bool {
        !self.active && self.ended_at_ms.is_none() && self.settle_until_ms.is_some_and(|u| now <= u)
    }

    /// End a settling turn whose window ran out, at its last pass's end.
    fn close_if_lapsed(&mut self, now: u64) {
        if !self.active && self.ended_at_ms.is_none() && !self.settling_at(now) {
            self.ended_at_ms = Some(self.last_pass_ended_at_ms.unwrap_or(now));
            self.end = Some(TurnEnd::Completed);
            self.settle_until_ms = None;
        }
    }
}

/// One finished pass's figures, from its `result` frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassStats {
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub steps: u32,
    pub duration_api_ms: u64,
}

impl PassStats {
    /// Read from a Claude `result` frame; absent fields count as zero.
    pub fn from_result_frame(frame: &serde_json::Value) -> Self {
        let u64_at = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_u64()).unwrap_or(0);
        PassStats {
            output_tokens: u64_at(frame.pointer("/usage/output_tokens")),
            cost_usd: frame.get("total_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0),
            steps: u64_at(frame.get("num_turns")) as u32,
            duration_api_ms: u64_at(frame.get("duration_api_ms")),
        }
    }
}

/// How a pass is starting, for the ledger's join decision (§4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PassStart {
    /// Input written by srv now: the pane, a jekt, a broadcast …
    Input { origin: TurnOrigin, joins_turn: Option<u64> },
    /// Input srv had queued (a spawn's backlog, a retry), or a spawn.
    Queued,
    /// The CLI started the pass itself, after a task notification.
    CliWake,
}

/// Called with the ledger after every change, outside the tracker's lock.
pub type TurnLedgerPublisher = Arc<dyn Fn(&TurnLedger) + Send + Sync>;

/// A publisher that sends the ledger as the persisted `agentturn` event on
/// `block:<id>`, so a pane mounting mid-turn gets it at once.
pub fn ledger_publisher(broker: Arc<crate::backend::mps::Broker>, block_id: String) -> TurnLedgerPublisher {
    Arc::new(move |ledger: &TurnLedger| {
        let mut data = serde_json::to_value(ledger).unwrap_or_default();
        if let Some(obj) = data.as_object_mut() {
            obj.insert("block_id".into(), serde_json::Value::String(block_id.clone()));
        }
        broker.publish(crate::backend::mps::MuxEvent {
            event: crate::backend::mps::EVENT_AGENT_TURN.to_string(),
            scopes: vec![format!("block:{block_id}")],
            sender: String::new(),
            persist: 1,
            data: Some(data),
        });
    })
}

/// What started the current turn, and whether anything else got in.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct TurnProvenance {
    pub origin: TurnOrigin,
    /// Non-user input was delivered into this turn after it started (§6.3:
    /// a taint, never undone within the turn).
    pub tainted: bool,
    /// The user's message that started the turn (origin `User` only).
    #[serde(skip)]
    pub user_text: Option<String>,
}

impl TurnProvenance {
    fn from_input(input: TurnInput) -> Self {
        let user_text = (input.origin == TurnOrigin::User).then_some(input.text);
        TurnProvenance { origin: input.origin, tainted: false, user_text }
    }
    fn absorb(&mut self, origin: TurnOrigin) {
        if origin != TurnOrigin::User {
            self.tainted = true;
        }
    }
}

struct TurnActivityTrackerInner {
    active_turn: bool,
    exit_code: Option<i32>,
    /// What started the CURRENT turn. Cleared on every idle→active flip that
    /// isn't labelled, so a turn only ever carries a provenance its own start
    /// reported — a stale label can never leak into a later turn.
    provenance: Option<TurnProvenance>,
    /// Told ahead of a turn that starts later — a message queued while the
    /// process spawns. Taken by the next idle→active flip.
    next_turn: Option<TurnProvenance>,
    /// The user-facing turn this block is in, or last was in.
    ledger: Option<TurnLedger>,
    /// Input was written while the current pass ran: the CLI may answer it
    /// in a pass of its own straight after this one (J2).
    input_during_pass: bool,
    /// The CLI reported a finished background task, so it will start a pass
    /// by itself to tell the model (J4). Cleared when any pass starts.
    cli_wake_pending: bool,
    /// Input reached the CLI during the pass that just ended, which it may
    /// answer in a pass of its own straight after (J2). Good only while that
    /// turn settles: a later pass the CLI starts is something else's.
    continuation_expected: bool,
    /// What started that pass: a continuation answers the same input, so it
    /// carries the same provenance (self-quit gate, §6.3).
    continuation_provenance: Option<TurnProvenance>,
}

impl TurnActivityTrackerInner {
    /// idle→active with no label: take the queued hint, if any.
    fn start_unlabelled(&mut self) {
        self.provenance = self.next_turn.take();
    }

    /// Mid-pass input: the turn will expect a next pass when this one ends.
    fn note_input_during_pass(&mut self) {
        self.input_during_pass = true;
        if let Some(l) = self.ledger.as_mut().filter(|l| l.active) {
            l.inputs += 1;
        }
    }

    /// A pass starts (idle→active): join the current turn or open a new one
    /// (turn-model spec §4.3).
    fn begin_pass(&mut self, start: PassStart, now: u64) {
        self.input_during_pass = false;
        self.cli_wake_pending = false;
        self.continuation_expected = false;
        self.continuation_provenance = None;
        let origin = match start {
            PassStart::Input { origin, .. } => Some(origin),
            PassStart::CliWake => Some(TurnOrigin::Automated),
            PassStart::Queued => self.provenance.as_ref().map(|p| p.origin),
        };
        if let Some(ledger) = self.ledger.as_mut() {
            let settling = ledger.settling_at(now);
            let joins = match start {
                // A fresh message the user typed after the agent stopped is a
                // new turn, even inside the settle window (D3); only the
                // pane's flush of a message held during this turn joins it.
                PassStart::Input { origin: TurnOrigin::User, joins_turn } => {
                    joins_turn == Some(ledger.turn_id)
                        && (settling
                            || ledger.ended_at_ms.is_some_and(|e| now.saturating_sub(e) <= HELD_FLUSH_JOIN_MS))
                }
                PassStart::Input { .. } | PassStart::Queued | PassStart::CliWake => settling,
            };
            if joins {
                ledger.passes += 1;
                // A CLI continuation or a queue drain brings no new input.
                ledger.inputs += u32::from(matches!(start, PassStart::Input { .. }));
                ledger.active = true;
                ledger.settle_until_ms = None;
                ledger.ended_at_ms = None;
                ledger.end = None;
                return;
            }
            ledger.close_if_lapsed(now);
        }
        let previous = self.ledger.as_ref().map_or(0, |l| l.turn_id);
        self.ledger = Some(TurnLedger {
            turn_id: now.max(previous + 1),
            origin,
            started_at_ms: now,
            passes: 1,
            active: true,
            inputs: 0,
            counted_passes: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            steps: 0,
            duration_api_ms: 0,
            last_pass_ended_at_ms: None,
            settle_until_ms: None,
            ended_at_ms: None,
            end: None,
        });
    }

    /// The running pass ended (active→idle). The turn waits for a next pass
    /// when one is expected, and is over otherwise.
    fn end_pass(&mut self, now: u64, provenance: Option<TurnProvenance>) {
        let expects_more = self.input_during_pass || self.cli_wake_pending;
        self.continuation_expected = self.input_during_pass;
        self.continuation_provenance = self.input_during_pass.then_some(provenance).flatten();
        self.input_during_pass = false;
        let Some(ledger) = self.ledger.as_mut().filter(|l| l.active) else { return };
        ledger.active = false;
        ledger.last_pass_ended_at_ms = Some(now);
        if expects_more {
            ledger.settle_until_ms = Some(now + TURN_SETTLE_MS);
        } else {
            ledger.ended_at_ms = Some(now);
            ledger.end = Some(TurnEnd::Completed);
        }
    }
}

/// Per-block turn-activity tracker.
pub struct TurnActivityTracker {
    block_id: String,
    inner: Mutex<TurnActivityTrackerInner>,
    publisher: Mutex<Option<TurnLedgerPublisher>>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl TurnActivityTracker {
    pub fn new(block_id: String) -> Self {
        Self::with_clock(block_id, Box::new(agentmux_common::time::now_ms_u64))
    }

    /// A tracker that publishes its turn ledger on `broker`, when there is one.
    pub fn for_block(block_id: String, broker: Option<&Arc<crate::backend::mps::Broker>>) -> Self {
        let tracker = Self::new(block_id.clone());
        if let Some(broker) = broker {
            tracker.set_ledger_publisher(ledger_publisher(Arc::clone(broker), block_id));
        }
        tracker
    }

    fn with_clock(block_id: String, clock: Box<dyn Fn() -> u64 + Send + Sync>) -> Self {
        Self {
            block_id,
            inner: Mutex::new(TurnActivityTrackerInner {
                active_turn: false,
                exit_code: None,
                provenance: None,
                next_turn: None,
                ledger: None,
                input_during_pass: false,
                cli_wake_pending: false,
                continuation_expected: false,
                continuation_provenance: None,
            }),
            publisher: Mutex::new(None),
            clock,
        }
    }

    /// Where ledger changes go (the `agentturn` event). Set once by the
    /// controller that owns this tracker, when it has a broker.
    pub fn set_ledger_publisher(&self, publisher: TurnLedgerPublisher) {
        *self.publisher.lock().unwrap() = Some(publisher);
    }

    /// Send the ledger to the publisher, if it changed. Called after the
    /// tracker's lock is released.
    fn publish_if_changed(&self, before: Option<TurnLedger>, after: Option<TurnLedger>) {
        if before == after {
            return;
        }
        let Some(ledger) = after else { return };
        let publisher = self.publisher.lock().unwrap().clone();
        if let Some(p) = publisher {
            p(&ledger);
        }
    }

    /// Called when a new turn starts (subprocess spawned).
    pub fn set_active_turn(&self, active: bool) {
        if !active {
            return self.end_pass(None);
        }
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        let before = inner.ledger.clone();
        let was_active = inner.active_turn;
        inner.active_turn = true;
        inner.exit_code = None;
        if !was_active {
            inner.start_unlabelled();
            inner.begin_pass(PassStart::Queued, now);
        }
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
        tracing::info!(block_id = %self.block_id, active, "[health] turn_active flip");
    }

    /// Atomically marks a turn active and reports whether one was already in
    /// flight (the pre-call value) — a single lock acquisition, unlike
    /// calling `is_active_turn()` then `set_active_turn(true)` separately.
    /// That two-step form is a check-then-act race: `send_message` (user
    /// input) and `send_user_message` (muxbus delivery) can run concurrently
    /// on the same block, and both reading `false` before either writes
    /// `true` lets both decide to spawn a watchdog — the exact duplicate the
    /// "only re-arm when resuming from idle" logic exists to prevent.
    pub fn mark_turn_active_returning_was_active(&self) -> bool {
        self.mark_turn_active_from(None)
    }

    /// [`Self::mark_turn_active_returning_was_active`], saying who delivered
    /// the input (turn provenance, SPEC_AGENT_SELF_QUIT §6.3):
    /// - starting a turn records `input` as what started it — or, unlabelled,
    ///   whatever was queued for it (else unknown);
    /// - into a running turn, non-user input — or unlabelled input, which
    ///   can't be proven to be the user — taints it.
    pub fn mark_turn_active_from(&self, input: Option<TurnInput>) -> bool {
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        let before = inner.ledger.clone();
        let was_active = inner.active_turn;
        inner.active_turn = true;
        inner.exit_code = None;
        match (was_active, input) {
            (false, Some(i)) => {
                let start = PassStart::Input { origin: i.origin, joins_turn: i.joins_turn };
                inner.provenance = Some(TurnProvenance::from_input(i));
                inner.begin_pass(start, now);
            }
            (false, None) => {
                inner.start_unlabelled();
                inner.begin_pass(PassStart::Queued, now);
            }
            (true, Some(i)) => {
                if let Some(p) = inner.provenance.as_mut() {
                    p.absorb(i.origin);
                }
                inner.note_input_during_pass();
            }
            (true, None) => {
                if let Some(p) = inner.provenance.as_mut() {
                    p.tainted = true;
                }
                inner.note_input_during_pass();
            }
        }
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
        tracing::info!(
            block_id = %self.block_id,
            active = true,
            was_active,
            "[health] turn_active flip"
        );
        was_active
    }

    /// The CLI reported a finished background task (`system/task_notification`).
    /// It tells the model in a pass it starts itself: at once if it is idle,
    /// straight after the running pass otherwise.
    pub fn note_cli_task_notification(&self) {
        self.inner.lock().unwrap().cli_wake_pending = true;
    }

    /// The CLI began a pass with no input from srv (`system/init` while idle).
    /// Marked active only when something explains it (a task notification, or
    /// input that reached it during the last pass), so a stray line can never
    /// leave the agent reported busy with nothing to end it.
    /// Returns whether the pass was marked (the caller then publishes status
    /// and re-arms the heartbeat).
    pub fn mark_turn_active_from_cli(&self) -> bool {
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        // A continuation only while its turn still settles: after that, the
        // input it would have answered is long dealt with (#4492).
        let continuing = inner.continuation_expected && inner.ledger.as_ref().is_some_and(|l| l.settling_at(now));
        if inner.active_turn || !(inner.cli_wake_pending || continuing) {
            return false;
        }
        let before = inner.ledger.clone();
        inner.active_turn = true;
        inner.exit_code = None;
        // Answering input from the last pass: that pass's provenance (taint and
        // all). A task woke it, alone or as well: automated, never the user, so
        // the self-quit gate refuses in it, as for any automated turn.
        let continued = continuing && !inner.cli_wake_pending;
        let carried = inner.continuation_provenance.take().filter(|_| continued);
        inner.provenance = Some(carried.unwrap_or(TurnProvenance {
            origin: TurnOrigin::Automated,
            tainted: false,
            user_text: None,
        }));
        inner.begin_pass(PassStart::CliWake, now);
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
        tracing::info!(block_id = %self.block_id, active = true, "[health] turn_active flip (CLI started a pass)");
        true
    }

    /// The CLI's `result`: the running pass is over. Its figures (when the
    /// frame carries them) go to the turn it belonged to in the same step as
    /// the idle flip, so input that arrives right after can't take them.
    pub fn end_pass(&self, stats: Option<PassStats>) {
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        let before = inner.ledger.clone();
        let was_active = inner.active_turn;
        inner.active_turn = false;
        let ended = inner.provenance.take();
        if let (Some(stats), Some(l)) = (stats, inner.ledger.as_mut().filter(|l| l.active)) {
            l.add_stats(stats);
        }
        if was_active {
            inner.end_pass(now, ended);
        }
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
        tracing::info!(block_id = %self.block_id, active = false, "[health] turn_active flip");
    }

    /// Add a finished pass's `result` figures to the latest turn.
    #[cfg(test)]
    pub fn add_pass_stats(&self, stats: PassStats) {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.ledger.clone();
        if let Some(l) = inner.ledger.as_mut() {
            l.add_stats(stats);
        }
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
    }

    /// The current turn ledger, if any turn has started.
    #[cfg(test)]
    pub fn ledger(&self) -> Option<TurnLedger> {
        self.inner.lock().unwrap().ledger.clone()
    }

    /// Called when the subprocess exits.
    pub fn set_exited(&self, exit_code: i32) {
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        let before = inner.ledger.clone();
        inner.active_turn = false;
        inner.exit_code = Some(exit_code);
        inner.provenance = None;
        inner.next_turn = None;
        inner.input_during_pass = false;
        inner.cli_wake_pending = false;
        inner.continuation_expected = false;
        inner.continuation_provenance = None;
        if let Some(l) = inner.ledger.as_mut().filter(|l| l.ended_at_ms.is_none()) {
            if l.active {
                l.last_pass_ended_at_ms = Some(now);
            }
            l.active = false;
            l.settle_until_ms = None;
            l.ended_at_ms = Some(now);
            l.end = Some(TurnEnd::Exited);
        }
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
        tracing::info!(block_id = %self.block_id, exit_code, "[health] turn_active flip (process exited)");
    }

    /// Whether there's an active turn in progress.
    pub fn is_active_turn(&self) -> bool {
        self.inner.lock().unwrap().active_turn
    }

    /// A message queued now will start a later turn (it waits for the
    /// process to spawn). Two queued before that turn starts: the second, if
    /// not the user's, taints it.
    pub fn hint_next_turn(&self, input: TurnInput) {
        let mut inner = self.inner.lock().unwrap();
        match inner.next_turn.as_mut() {
            None => inner.next_turn = Some(TurnProvenance::from_input(input)),
            Some(p) => p.absorb(input.origin),
        }
    }

    /// An unlabelled message queued for the next turn: it can't be proven to
    /// be the user, so a turn already labelled by an earlier queued message is
    /// tainted. (Queued first, it leaves the turn unknown anyway.)
    pub fn hint_next_turn_unlabelled(&self) {
        if let Some(p) = self.inner.lock().unwrap().next_turn.as_mut() {
            p.tainted = true;
        }
    }

    /// The turn is active because messages already accounted for are being
    /// delivered — the queue drained after a spawn, a retry flush. Not new
    /// input: an idle tracker starts the turn with whatever was queued for it,
    /// and a running turn is left as it is (ReAgent P1 on #3789 — the plain
    /// unlabelled mark tainted every freshly spawned user turn).
    pub fn mark_turn_active_for_queued(&self) -> bool {
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        let before = inner.ledger.clone();
        let was_active = inner.active_turn;
        inner.active_turn = true;
        inner.exit_code = None;
        if !was_active {
            inner.start_unlabelled();
            inner.begin_pass(PassStart::Queued, now);
        }
        let after = inner.ledger.clone();
        drop(inner);
        self.publish_if_changed(before, after);
        tracing::info!(block_id = %self.block_id, active = true, was_active, "[health] turn_active flip (queued delivery)");
        was_active
    }

    /// What started the turn in flight, if one is and it was reported.
    pub fn provenance(&self) -> Option<TurnProvenance> {
        let inner = self.inner.lock().unwrap();
        if inner.active_turn {
            inner.provenance.clone()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_turn_active_returning_was_active_reports_the_pre_call_value() {
        let tracker = TurnActivityTracker::new("test-block".to_string());
        assert!(!tracker.is_active_turn());

        // First call: was idle before this call.
        let was_active = tracker.mark_turn_active_returning_was_active();
        assert!(!was_active, "first call should report idle-before-call");
        assert!(tracker.is_active_turn(), "turn is now active");

        // Second call while already active: reports true (already in flight).
        let was_active_again = tracker.mark_turn_active_returning_was_active();
        assert!(was_active_again, "second call should report already-active");
        assert!(tracker.is_active_turn());
    }

    /// Regression test for the exact race reagent flagged on PR #2005: a
    /// naive `is_active_turn()` read followed by a separate
    /// `set_active_turn(true)` write lets two concurrent callers (send_message
    /// vs. send_user_message on the same block) both observe `false` before
    /// either writes `true`, so both decide to spawn a watchdog.
    /// `mark_turn_active_returning_was_active` closes that window by holding
    /// the lock across both the read and the write — this test simulates the
    /// interleaving directly (no real concurrency needed to prove the
    /// invariant: exactly one of N concurrent-in-spirit calls sees "was
    /// idle").
    #[test]
    fn mark_turn_active_is_atomic_across_repeated_calls() {
        let tracker = TurnActivityTracker::new("test-block".to_string());
        let results: Vec<bool> = (0..5).map(|_| tracker.mark_turn_active_returning_was_active()).collect();
        // Exactly the first call observes "was idle" (false); every
        // subsequent call — however tightly interleaved a real concurrent
        // caller might be — observes "already active" (true), because each
        // read-and-write pair is indivisible under the lock.
        assert_eq!(results, vec![false, true, true, true, true]);
    }

    // ---- turn provenance (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.3) ----

    fn user(text: &str) -> TurnInput {
        TurnInput { origin: TurnOrigin::User, text: text.into(), joins_turn: None }
    }
    fn automated() -> TurnInput {
        TurnInput { origin: TurnOrigin::Automated, text: "jekt".into(), joins_turn: None }
    }

    #[test]
    fn a_turn_the_user_starts_carries_their_text() {
        let t = TurnActivityTracker::new("b".into());
        assert!(!t.mark_turn_active_from(Some(user("finish the PR then quit"))));
        let p = t.provenance().expect("active turn");
        assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
        assert_eq!(p.user_text.as_deref(), Some("finish the PR then quit"));
    }

    #[test]
    fn automated_input_into_a_user_turn_taints_it_for_good() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(user("also this"))); // the user steering their own turn
        assert!(!t.provenance().unwrap().tainted);
        t.mark_turn_active_from(Some(automated()));
        assert!(t.provenance().unwrap().tainted);
        t.mark_turn_active_from(Some(user("more")));
        assert!(t.provenance().unwrap().tainted, "never undone within the turn");
    }

    #[test]
    fn unlabelled_input_counts_as_unknown_never_as_the_user() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_returning_was_active();
        assert_eq!(t.provenance(), None, "an unlabelled start is unknown");
        t.set_active_turn(false);
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_returning_was_active(); // unlabelled input mid-turn
        assert!(t.provenance().unwrap().tainted);
    }

    #[test]
    fn a_label_never_outlives_its_turn() {
        let t = TurnActivityTracker::new("b".into());
        t.mark_turn_active_from(Some(user("go")));
        t.set_active_turn(false);
        assert_eq!(t.provenance(), None, "no turn, no provenance");
        t.set_active_turn(true); // the next turn starts unlabelled
        assert_eq!(t.provenance(), None, "the old user label must not leak into it");
        t.set_exited(0);
        assert_eq!(t.provenance(), None);
    }

    #[test]
    fn a_queued_message_labels_the_turn_it_starts_later() {
        let t = TurnActivityTracker::new("b".into());
        t.hint_next_turn(user("first message after spawn"));
        t.set_active_turn(true); // the spawn starts the turn
        let p = t.provenance().unwrap();
        assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
        t.set_active_turn(false);
        t.hint_next_turn(user("a"));
        t.hint_next_turn(automated());
        t.set_active_turn(true);
        assert!(t.provenance().unwrap().tainted, "a jekt queued alongside taints it");
    }

    #[test]
    fn delivering_the_queue_after_a_spawn_keeps_the_user_s_label() {
        let t = TurnActivityTracker::new("b".into());
        t.hint_next_turn(user("do X then quit"));
        t.set_active_turn(true); // spawn_process
        assert!(t.mark_turn_active_for_queued(), "already active");
        let p = t.provenance().unwrap();
        assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false), "bookkeeping, not new input");

        // Idle, the same mark starts the turn with what was queued for it.
        t.set_active_turn(false);
        t.hint_next_turn(user("again"));
        assert!(!t.mark_turn_active_for_queued());
        assert_eq!(t.provenance().unwrap().user_text.as_deref(), Some("again"));
    }

    #[test]
    fn an_unlabelled_message_queued_behind_the_user_s_taints_that_turn() {
        let t = TurnActivityTracker::new("b".into());
        t.hint_next_turn(user("go"));
        t.hint_next_turn_unlabelled();
        t.set_active_turn(true);
        assert!(t.provenance().unwrap().tainted);
    }

    // ---- turn ledger (SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.3) ----

    use std::sync::atomic::{AtomicU64, Ordering};

    /// A tracker on a clock the test moves, recording what it publishes.
    fn ledger_tracker() -> (TurnActivityTracker, Arc<AtomicU64>, Arc<Mutex<Vec<TurnLedger>>>) {
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let c = Arc::clone(&clock);
        let t = TurnActivityTracker::with_clock("b".into(), Box::new(move || c.load(Ordering::SeqCst)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = Arc::clone(&seen);
        t.set_ledger_publisher(Arc::new(move |l: &TurnLedger| s.lock().unwrap().push(l.clone())));
        (t, clock, seen)
    }

    fn advance(clock: &AtomicU64, ms: u64) {
        clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn jekt() -> TurnInput {
        TurnInput { origin: TurnOrigin::Automated, text: "jekt".into(), joins_turn: None }
    }

    fn stats(output_tokens: u64, steps: u32) -> PassStats {
        PassStats { output_tokens, cost_usd: 0.25, steps, duration_api_ms: 1_000 }
    }

    #[test]
    fn a_single_pass_turn_ends_with_its_pass() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        advance(&clock, 5_000);
        t.set_active_turn(false);
        t.add_pass_stats(stats(300, 2));
        let l = t.ledger().unwrap();
        assert_eq!((l.passes, l.active, l.end), (1, false, Some(TurnEnd::Completed)));
        assert_eq!(l.ended_at_ms, Some(l.started_at_ms + 5_000));
        assert_eq!((l.counted_passes, l.output_tokens, l.steps), (1, 300, 2));
        assert_eq!(l.origin, Some(TurnOrigin::User));
    }

    #[test]
    fn a_jekt_written_mid_pass_and_answered_in_the_next_pass_is_one_turn() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.mark_turn_active_from(Some(jekt())); // J2: arrives mid-pass
        advance(&clock, 10_000);
        t.set_active_turn(false);
        t.add_pass_stats(stats(300, 2));
        let l = t.ledger().unwrap();
        assert_eq!((l.ended_at_ms, l.settle_until_ms.is_some()), (None, true), "waits for the next pass");
        advance(&clock, 50);
        // The CLI answers the jekt in a pass of its own: no input from srv.
        assert!(t.mark_turn_active_from_cli(), "the mid-pass input explains the init");
        let p = t.provenance().unwrap();
        assert_eq!((p.origin, p.tainted), (TurnOrigin::User, true), "the user's pass, tainted by the jekt it answers");
        advance(&clock, 3_000);
        t.set_active_turn(false);
        t.add_pass_stats(stats(100, 1));
        let l = t.ledger().unwrap();
        assert_eq!(l.turn_id, id, "the same turn");
        assert_eq!((l.passes, l.inputs, l.counted_passes, l.output_tokens, l.steps), (2, 1, 2, 400, 3));
        assert_eq!(l.end, Some(TurnEnd::Completed));
        assert_eq!(l.origin, Some(TurnOrigin::User), "the trigger never changes");
    }

    #[test]
    fn a_task_notification_during_a_pass_joins_the_cli_s_own_next_pass() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.note_cli_task_notification(); // the task finished mid-pass
        advance(&clock, 4_000);
        t.set_active_turn(false);
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli(), "init after the notification starts a pass");
        let l = t.ledger().unwrap();
        assert_eq!((l.turn_id, l.passes, l.active), (id, 2, true));
    }

    #[test]
    fn a_task_finishing_after_the_turn_ended_starts_an_automated_turn() {
        // The order Claude Code 2.1.112 was recorded emitting: result, then
        // task_updated / task_notification, then system/init, 8 s later.
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("start a background build")));
        let first = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        assert!(t.ledger().unwrap().end.is_some(), "nothing was pending: the turn is over");
        advance(&clock, 8_000);
        t.note_cli_task_notification();
        assert!(t.mark_turn_active_from_cli());
        let l = t.ledger().unwrap();
        assert_ne!(l.turn_id, first);
        assert_eq!((l.origin, l.passes), (Some(TurnOrigin::Automated), 1));
        assert!(t.is_active_turn());
        assert_eq!(t.provenance().unwrap().origin, TurnOrigin::Automated, "never the user's turn");
    }

    #[test]
    fn an_init_with_no_task_notification_is_not_taken_for_a_pass() {
        let (t, _, _) = ledger_tracker();
        assert!(!t.mark_turn_active_from_cli());
        assert!(!t.is_active_turn(), "a stray init must not leave the agent busy");
        t.mark_turn_active_from(Some(user("go")));
        t.note_cli_task_notification();
        assert!(!t.mark_turn_active_from_cli(), "a pass is already running");
    }

    #[test]
    fn a_settling_turn_ends_at_its_last_pass_when_nothing_follows() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.mark_turn_active_from(Some(jekt()));
        advance(&clock, 1_000);
        t.set_active_turn(false);
        assert!(t.ledger().unwrap().settle_until_ms.is_some());
        advance(&clock, TURN_SETTLE_MS + 1);
        t.mark_turn_active_from(Some(jekt())); // too late to join
        let l = t.ledger().unwrap();
        assert_ne!(l.turn_id, id, "a new turn");
        assert_eq!((l.passes, l.origin), (1, Some(TurnOrigin::Automated)));
    }

    #[test]
    fn a_fresh_message_from_the_user_starts_a_new_turn_even_while_settling() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.mark_turn_active_from(Some(jekt()));
        t.set_active_turn(false);
        advance(&clock, 100);
        t.mark_turn_active_from(Some(user("something new")));
        assert_ne!(t.ledger().unwrap().turn_id, id);
    }

    #[test]
    fn the_pane_s_held_message_joins_the_turn_it_was_held_during() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        advance(&clock, 2_000);
        t.set_active_turn(false); // ended: nothing srv knew of was pending
        advance(&clock, 300);
        let held = TurnInput { origin: TurnOrigin::User, text: "queued while busy".into(), joins_turn: Some(id) };
        t.mark_turn_active_from(Some(held));
        let l = t.ledger().unwrap();
        assert_eq!((l.turn_id, l.passes, l.inputs, l.active), (id, 2, 1, true));
        assert_eq!((l.ended_at_ms, l.end), (None, None), "reopened");
    }

    #[test]
    fn a_held_message_for_a_long_finished_turn_starts_a_new_one() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        advance(&clock, HELD_FLUSH_JOIN_MS + 1);
        let held = TurnInput { origin: TurnOrigin::User, text: "late".into(), joins_turn: Some(id) };
        t.mark_turn_active_from(Some(held));
        assert_ne!(t.ledger().unwrap().turn_id, id);
    }

    #[test]
    fn queued_delivery_joins_only_a_settling_turn() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.mark_turn_active_from(Some(jekt()));
        t.set_active_turn(false);
        advance(&clock, 10);
        t.mark_turn_active_for_queued();
        assert_eq!(t.ledger().unwrap().turn_id, id);
        t.set_active_turn(false); // nothing pending now
        advance(&clock, 10);
        t.mark_turn_active_for_queued();
        assert_ne!(t.ledger().unwrap().turn_id, id);
    }

    #[test]
    fn process_exit_ends_the_turn() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        advance(&clock, 700);
        t.set_exited(1);
        let l = t.ledger().unwrap();
        assert_eq!((l.active, l.end), (false, Some(TurnEnd::Exited)));
        assert_eq!(l.last_pass_ended_at_ms, l.ended_at_ms);
    }

    #[test]
    fn turn_ids_are_unique_even_within_one_millisecond() {
        let (t, _, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("a")));
        let a = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        t.mark_turn_active_from(Some(user("b")));
        assert!(t.ledger().unwrap().turn_id > a);
    }

    #[test]
    fn every_change_is_published_and_nothing_else() {
        let (t, _, seen) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_returning_was_active(); // already active, no ledger change … except inputs
        t.set_active_turn(false);
        t.add_pass_stats(PassStats::default()); // a counted pass, even with nothing in it
        t.set_active_turn(false); // already idle: no change
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4, "start, mid-pass input, end, stats: {seen:#?}");
        assert!(seen[0].active && !seen[2].active);
        assert_eq!(seen[3].counted_passes, 1);
    }

    #[test]
    fn pass_stats_read_a_result_frame() {
        let frame = serde_json::json!({
            "type": "result", "subtype": "success", "num_turns": 2, "duration_api_ms": 9_800,
            "total_cost_usd": 0.0123, "usage": { "input_tokens": 9, "output_tokens": 238 }
        });
        let s = PassStats::from_result_frame(&frame);
        assert_eq!((s.output_tokens, s.steps, s.duration_api_ms), (238, 2, 9_800));
        assert!((s.cost_usd - 0.0123).abs() < 1e-9);
        assert_eq!(PassStats::from_result_frame(&serde_json::json!({"type": "result"})), PassStats::default());
    }

    #[test]
    fn the_ledger_serializes_for_the_pane() {
        let (t, _, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let v = serde_json::to_value(t.ledger().unwrap()).unwrap();
        assert_eq!(v["origin"], "user");
        assert_eq!(v["active"], true);
        assert!(v.get("ended_at_ms").is_none(), "absent, not null");
    }

    /// A pass's figures belong to its own turn, even when input that opens a
    /// new turn arrives right after its `result` (#4492).
    #[test]
    fn a_pass_s_figures_stay_with_its_turn_when_a_new_turn_follows_at_once() {
        let (t, _, seen) = ledger_tracker();
        t.mark_turn_active_from(Some(user("a")));
        let first = t.ledger().unwrap().turn_id;
        t.end_pass(Some(stats(300, 2)));
        t.mark_turn_active_from(Some(user("b"))); // a new turn, at once
        let next = t.ledger().unwrap();
        assert_ne!(next.turn_id, first);
        assert_eq!((next.counted_passes, next.output_tokens), (0, 0), "nothing of the old pass");
        let ended = seen.lock().unwrap().iter().rev().find(|l| l.turn_id == first).cloned().unwrap();
        assert_eq!((ended.counted_passes, ended.output_tokens, ended.steps), (1, 300, 2));
        assert_eq!(ended.end, Some(TurnEnd::Completed));
    }

    /// Input answered inside its own pass leaves no continuation behind: a
    /// task that wakes the CLI minutes later is an automated pass, never the
    /// user's (#4492).
    #[test]
    fn a_late_task_wake_up_never_inherits_an_old_user_pass_s_provenance() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("finish then quit")));
        t.mark_turn_active_from(Some(user("and this too"))); // answered inside the pass
        t.set_active_turn(false);
        advance(&clock, 5 * 60_000);
        t.note_cli_task_notification();
        assert!(t.mark_turn_active_from_cli());
        let p = t.provenance().unwrap();
        assert_eq!((p.origin, p.user_text), (TurnOrigin::Automated, None));
        assert_eq!(t.ledger().unwrap().origin, Some(TurnOrigin::Automated));
    }

    /// Nor does a stale continuation alone explain a later init.
    #[test]
    fn an_expired_continuation_does_not_explain_an_init() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(jekt()));
        t.set_active_turn(false);
        advance(&clock, TURN_SETTLE_MS + 1);
        assert!(!t.mark_turn_active_from_cli(), "no notification, settle window over");
        assert!(!t.is_active_turn());
    }

    /// A pass the CLI starts for both a task and input from the last pass is
    /// the automated one.
    #[test]
    fn a_continuation_that_a_task_also_woke_is_automated() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(user("more")));
        t.note_cli_task_notification();
        t.set_active_turn(false);
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli());
        assert_eq!(t.provenance().unwrap().origin, TurnOrigin::Automated);
    }
}
