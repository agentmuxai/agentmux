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

/// At most this many inputs are listed on a turn (`TurnLedger::absorbed`);
/// `inputs` keeps counting past it.
pub const ABSORBED_CAP: usize = 20;

/// What kind of thing started, or joined, a turn (turn-model spec §5.2).
/// Display data next to [`TurnOrigin`], which stays the security class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// The human, in this pane.
    User,
    /// The human, by Swarm broadcast.
    Broadcast,
    /// A jekt from another agent.
    Agent,
    /// A jekt from a service (`*-consumer`: GitHub review notices, …).
    Service,
    /// A scheduled delivery (cron, a loop).
    Schedule,
    /// The CLI woke for a background task it had started.
    Task,
    /// AgentMux on its own behalf (memory reinjection, a side question).
    System,
}

/// What started, or joined, a turn: its kind, and who or what, for display.
/// Built by srv from what it wrote itself (the jekt marker it composed, the
/// CLI's task notification), never from message body text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct TurnTrigger {
    pub kind: TriggerKind,
    /// The sender (a jekt's `FROM`), or the task's summary. Absent for the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl TurnTrigger {
    fn of(kind: TriggerKind, from: Option<&str>) -> Self {
        TurnTrigger { kind, from: from.map(str::to_string) }
    }

    /// Classify an input by its origin and the envelope srv put on it.
    pub fn from_input(origin: TurnOrigin, text: &str) -> Self {
        match origin {
            // srv rewrites a broadcast marker found inside any jekt, so only a
            // user-origin input can carry a real one.
            TurnOrigin::User if text.contains("[BROADCAST:FROM=user VIA=swarm") => Self::of(TriggerKind::Broadcast, None),
            TurnOrigin::User => Self::of(TriggerKind::User, None),
            TurnOrigin::System => Self::of(TriggerKind::System, None),
            TurnOrigin::Automated => match jekt_sender(text) {
                Some(from @ ("cron" | "loop")) => Self::of(TriggerKind::Schedule, Some(from)),
                Some(from) if from.ends_with("-consumer") => Self::of(TriggerKind::Service, Some(from)),
                from => Self::of(TriggerKind::Agent, from),
            },
        }
    }

    /// The CLI woke for a finished background task; `summary` is the
    /// notification's own (`Background command "…" completed (exit code 0)`).
    pub fn task(summary: Option<&str>) -> Self {
        let summary = summary.map(str::trim).filter(|s| !s.is_empty()).map(|s| {
            if s.chars().count() > 160 {
                format!("{}…", s.chars().take(159).collect::<String>())
            } else {
                s.to_string()
            }
        });
        TurnTrigger { kind: TriggerKind::Task, from: summary }
    }
}

/// The `FROM=` of the `[JEKT:…]` marker srv puts at the head of every
/// automated delivery (cron fires as `FROM=cron`). The input may be the
/// plain text or the stream-json line carrying it, so the marker is looked
/// for anywhere; only automated input is read this way.
fn jekt_sender(text: &str) -> Option<&str> {
    let tag = &text[text.find("[JEKT:")? + "[JEKT:".len()..];
    let tag = &tag[..tag.find(']')?];
    tag.split_whitespace().find_map(|t| t.strip_prefix("FROM=")).filter(|s| !s.is_empty())
}

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
    /// Bumped on every change, under the tracker's lock. Publishes run after
    /// the lock is released, so two can land out of order; the pane keeps the
    /// highest `seq` (#4492).
    pub seq: u64,
    /// What started the turn; `None` when its first pass was unlabelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<TurnOrigin>,
    /// The same, for display: what kind of thing, and who.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TurnTrigger>,
    /// The labelled inputs that arrived after the turn started, in order,
    /// up to [`ABSORBED_CAP`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub absorbed: Vec<TurnTrigger>,
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
    fn absorb(&mut self, trigger: Option<TurnTrigger>) {
        if let Some(t) = trigger.filter(|_| self.absorbed.len() < ABSORBED_CAP) {
            self.absorbed.push(t);
        }
    }

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
///
/// A settling ledger is also published again, closed, when its window lapses
/// with nothing newer published in between (`closed_when_lapsed`). The
/// tracker closes it only lazily, at the next pass, and a consumer like the
/// notification router needs to hear that the turn is over.
pub fn ledger_publisher(broker: Arc<crate::backend::mps::Broker>, block_id: String) -> TurnLedgerPublisher {
    let latest_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let publish = Arc::new(move |ledger: &TurnLedger| {
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
    });
    Arc::new(move |ledger: &TurnLedger| {
        latest_seq.store(ledger.seq, std::sync::atomic::Ordering::SeqCst);
        publish(ledger);
        if let (Some(closed), Ok(handle)) = (closed_when_lapsed(ledger), tokio::runtime::Handle::try_current()) {
            let wait = ledger.settle_until_ms.unwrap_or(0).saturating_sub(agentmux_common::time::now_ms_u64()) + 50;
            let (latest_seq, publish) = (Arc::clone(&latest_seq), Arc::clone(&publish));
            handle.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                if latest_seq.load(std::sync::atomic::Ordering::SeqCst) == closed.seq {
                    publish(&closed);
                }
            });
        }
    })
}

/// The change from one published ledger to the next, in words, for the
/// `[turn]` log: `None` when nothing worth a line changed (stats, `seq`).
pub fn ledger_transition(before: Option<&TurnLedger>, after: Option<&TurnLedger>) -> Option<&'static str> {
    let a = after?;
    let Some(b) = before.filter(|b| b.turn_id == a.turn_id) else {
        return Some("opened");
    };
    if a.passes > b.passes {
        return Some("next pass joined the turn");
    }
    if a.inputs > b.inputs {
        return Some("input joined the running pass");
    }
    if a.end.is_some() && b.end.is_none() {
        return Some(match a.end {
            Some(TurnEnd::Exited) => "ended: the process exited",
            _ => "ended",
        });
    }
    if a.settle_until_ms.is_some() && b.settle_until_ms.is_none() {
        return Some("pass ended, settling for a next one");
    }
    None
}

/// A settling ledger as it reads once its window lapsed with no next pass:
/// ended at its last pass. `None` for a ledger that isn't settling. Same `seq`:
/// it is the same state, only now known to be over.
pub fn closed_when_lapsed(ledger: &TurnLedger) -> Option<TurnLedger> {
    if ledger.active || ledger.ended_at_ms.is_some() || ledger.settle_until_ms.is_none() {
        return None;
    }
    let mut closed = ledger.clone();
    closed.ended_at_ms = closed.last_pass_ended_at_ms;
    closed.end = Some(TurnEnd::Completed);
    closed.settle_until_ms = None;
    Some(closed)
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
    /// The turn a held message being sent now should join, and when srv was
    /// told: for controllers whose pass starts carry no input (`hint_join`).
    pending_join: Option<(u64, u64)>,
    /// The last `TurnLedger::seq` stamped, across turns.
    ledger_seq: u64,
    /// The trigger of a message queued for the next turn (`hint_next_turn`).
    next_turn_trigger: Option<TurnTrigger>,
    /// The task the CLI last reported finished, for the pass it wakes for.
    pending_task_trigger: Option<TurnTrigger>,
}

impl TurnActivityTrackerInner {
    /// The ledger to publish after a change: stamped with the next `seq` when
    /// it differs from `before`.
    fn stamp(&mut self, before: &Option<TurnLedger>) -> Option<TurnLedger> {
        if self.ledger != *before {
            self.ledger_seq += 1;
            if let Some(l) = self.ledger.as_mut() {
                l.seq = self.ledger_seq;
            }
        }
        self.ledger.clone()
    }

    /// idle→active with no label: take the queued hint, if any.
    fn start_unlabelled(&mut self) {
        self.provenance = self.next_turn.take();
    }

    /// Mid-pass input: the turn will expect a next pass when this one ends.
    fn note_input_during_pass(&mut self, trigger: Option<TurnTrigger>) {
        self.input_during_pass = true;
        // A join hint for a message that reached a running pass is spent: it
        // must not make the next fresh message join this turn.
        self.pending_join = None;
        if let Some(l) = self.ledger.as_mut().filter(|l| l.active) {
            l.inputs += 1;
            l.absorb(trigger);
        }
    }

    /// A pass starts (idle→active): join the current turn or open a new one
    /// (turn-model spec §4.3).
    fn begin_pass(&mut self, start: PassStart, trigger: Option<TurnTrigger>, now: u64) {
        self.input_during_pass = false;
        self.cli_wake_pending = false;
        self.continuation_expected = false;
        self.continuation_provenance = None;
        // A join hint is good once, and only briefly: it names a message being
        // sent right now.
        let hinted = self
            .pending_join
            .take()
            .filter(|(_, at)| now.saturating_sub(*at) <= HELD_FLUSH_JOIN_MS)
            .map(|(id, _)| id);
        let trigger = match start {
            PassStart::Queued => trigger.or_else(|| self.next_turn_trigger.take()),
            PassStart::CliWake => self.pending_task_trigger.take().or(trigger),
            PassStart::Input { .. } => trigger,
        };
        // A task that lost the race to this pass is not what a later pass
        // answers: only the CLI's own wake for it may carry it (#4503).
        self.pending_task_trigger = None;
        let origin = match start {
            PassStart::Input { origin, .. } => Some(origin),
            PassStart::CliWake => Some(TurnOrigin::Automated),
            PassStart::Queued => self.provenance.as_ref().map(|p| p.origin),
        };
        if let Some(ledger) = self.ledger.as_mut() {
            let settling = ledger.settling_at(now);
            // Ended, or done settling with no next pass (nothing writes the end
            // then): it ended at its last pass, as the pane reads it (#4492).
            let ended_at = ledger
                .ended_at_ms
                .or_else(|| (!ledger.active && !settling).then_some(ledger.last_pass_ended_at_ms).flatten());
            let recently_ended = ended_at.is_some_and(|e| now.saturating_sub(e) <= HELD_FLUSH_JOIN_MS);
            let joins = match start {
                // A fresh message the user typed after the agent stopped is a
                // new turn, even inside the settle window (D3); only the
                // pane's flush of a message held during this turn joins it.
                PassStart::Input { origin: TurnOrigin::User, joins_turn } => {
                    joins_turn == Some(ledger.turn_id) && (settling || recently_ended)
                }
                PassStart::Queued if hinted == Some(ledger.turn_id) => settling || recently_ended,
                PassStart::Input { .. } | PassStart::Queued | PassStart::CliWake => settling,
            };
            if joins {
                ledger.passes += 1;
                // New input joined: a message, or a finished task the CLI woke
                // for (#4503). A plain continuation or a queue drain brings none.
                let task_wake = matches!(start, PassStart::CliWake) && trigger.is_some();
                if matches!(start, PassStart::Input { .. }) || task_wake {
                    ledger.inputs += 1;
                    ledger.absorb(trigger);
                }
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
            seq: 0,
            origin,
            // A pass the CLI started with no task to name has no trigger: never
            // a sender made up from its origin (#4503).
            trigger: trigger.or_else(|| {
                (!matches!(start, PassStart::CliWake)).then_some(origin).flatten().map(|o| TurnTrigger::from_input(o, ""))
            }),
            absorbed: Vec::new(),
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
    /// Taken before `inner` is released and held through the publish, so
    /// ledgers go out in `seq` order. The broker keeps the last one for
    /// replay, so an older one published last would be what a new subscriber
    /// gets (#4492). Lock order: `inner` → `publish_order`.
    publish_order: Mutex<()>,
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
                pending_join: None,
                ledger_seq: 0,
                next_turn_trigger: None,
                pending_task_trigger: None,
            }),
            publisher: Mutex::new(None),
            publish_order: Mutex::new(()),
            clock,
        }
    }

    /// Where ledger changes go (the `agentturn` event). Set once by the
    /// controller that owns this tracker, when it has a broker.
    pub fn set_ledger_publisher(&self, publisher: TurnLedgerPublisher) {
        *self.publisher.lock().unwrap() = Some(publisher);
    }

    /// Release `inner` and publish the ledger if it changed, in `seq` order,
    /// logging the change as a `[turn]` line (`muxlog`'s answer to "why did
    /// my timer reset": turn-model spec §7).
    fn publish_in_order(
        &self,
        inner: std::sync::MutexGuard<'_, TurnActivityTrackerInner>,
        before: Option<TurnLedger>,
        after: Option<TurnLedger>,
    ) {
        if before == after {
            return;
        }
        let _order = self.publish_order.lock().unwrap();
        drop(inner);
        if let (Some(what), Some(l)) = (ledger_transition(before.as_ref(), after.as_ref()), after.as_ref()) {
            tracing::info!(
                block_id = %self.block_id,
                turn_id = l.turn_id,
                passes = l.passes,
                inputs = l.inputs,
                trigger = ?l.trigger.as_ref().map(|t| t.kind),
                "[turn] {what}"
            );
        }
        self.publish_if_changed(before, after);
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
            inner.begin_pass(PassStart::Queued, None, now);
        }
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
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
                let trigger = TurnTrigger::from_input(i.origin, &i.text);
                inner.provenance = Some(TurnProvenance::from_input(i));
                inner.begin_pass(start, Some(trigger), now);
            }
            (false, None) => {
                inner.start_unlabelled();
                inner.begin_pass(PassStart::Queued, None, now);
            }
            (true, Some(i)) => {
                if let Some(p) = inner.provenance.as_mut() {
                    p.absorb(i.origin);
                }
                inner.note_input_during_pass(Some(TurnTrigger::from_input(i.origin, &i.text)));
            }
            (true, None) => {
                if let Some(p) = inner.provenance.as_mut() {
                    p.tainted = true;
                }
                inner.note_input_during_pass(None);
            }
        }
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
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
    pub fn note_cli_task_notification(&self, summary: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        inner.cli_wake_pending = true;
        inner.pending_task_trigger = Some(TurnTrigger::task(summary));
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
        inner.begin_pass(PassStart::CliWake, None, now);
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
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
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
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
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
    }

    /// A message the pane held while turn `turn_id` ran is about to be sent
    /// through a controller whose pass start carries no input (ACP, App Server,
    /// one-shot subprocess): the pass it starts joins that turn, as
    /// `TurnInput::joins_turn` does for the persistent controller (§4.3, J3).
    pub fn hint_join(&self, turn_id: u64) {
        let now = (self.clock)();
        self.inner.lock().unwrap().pending_join = Some((turn_id, now));
    }

    /// Drop a join hint whose dispatch failed: no pass will consume it, and
    /// a later message must not.
    pub fn clear_join_hint(&self) {
        self.inner.lock().unwrap().pending_join = None;
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
        inner.pending_join = None;
        inner.next_turn_trigger = None;
        inner.pending_task_trigger = None;
        if let Some(l) = inner.ledger.as_mut().filter(|l| l.ended_at_ms.is_none()) {
            if l.active {
                l.last_pass_ended_at_ms = Some(now);
            }
            l.active = false;
            l.settle_until_ms = None;
            l.ended_at_ms = Some(now);
            l.end = Some(TurnEnd::Exited);
        }
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
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
        let now = (self.clock)();
        let mut inner = self.inner.lock().unwrap();
        // A held message queued behind a spawn keeps its join for the pass the
        // spawn starts (#4492).
        if let Some(turn_id) = input.joins_turn {
            inner.pending_join = Some((turn_id, now));
        }
        if inner.next_turn.is_none() {
            inner.next_turn_trigger = Some(TurnTrigger::from_input(input.origin, &input.text));
        }
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
            inner.begin_pass(PassStart::Queued, None, now);
        }
        let after = inner.stamp(&before);
        self.publish_in_order(inner, before, after);
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
        t.note_cli_task_notification(None); // the task finished mid-pass
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
        t.note_cli_task_notification(None);
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
        t.note_cli_task_notification(None);
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
        t.note_cli_task_notification(None);
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
        t.note_cli_task_notification(None);
        t.set_active_turn(false);
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli());
        assert_eq!(t.provenance().unwrap().origin, TurnOrigin::Automated);
    }

    /// ACP, App Server and one-shot subprocess passes start with no input:
    /// the pane's held message joins its turn through the hint (#4492).
    #[test]
    fn a_held_message_joins_its_turn_through_a_controller_without_labelled_input() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_returning_was_active();
        let id = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        advance(&clock, 300);
        t.hint_join(id);
        t.mark_turn_active_returning_was_active(); // the controller's own mark
        let l = t.ledger().unwrap();
        assert_eq!((l.turn_id, l.passes), (id, 2));
    }

    #[test]
    fn a_join_hint_is_good_once_and_only_briefly() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_returning_was_active();
        let id = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        t.hint_join(id);
        advance(&clock, HELD_FLUSH_JOIN_MS + 1);
        t.mark_turn_active_returning_was_active();
        assert_ne!(t.ledger().unwrap().turn_id, id, "stale hint");

        let id2 = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        t.hint_join(id2);
        t.mark_turn_active_returning_was_active(); // joins, using the hint up
        t.set_active_turn(false);
        advance(&clock, 10);
        t.mark_turn_active_returning_was_active();
        assert_ne!(t.ledger().unwrap().turn_id, id2, "the hint was used once");
    }

    #[test]
    fn a_hint_for_another_turn_joins_nothing() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_returning_was_active();
        let id = t.ledger().unwrap().turn_id;
        t.set_active_turn(false);
        advance(&clock, 10);
        t.hint_join(id + 999);
        t.mark_turn_active_returning_was_active();
        assert_ne!(t.ledger().unwrap().turn_id, id);
    }

    /// Every published ledger carries a higher `seq` than the one before it,
    /// across turns, so the pane can drop one that lands late (#4492).
    #[test]
    fn every_published_ledger_has_a_higher_seq() {
        let (t, clock, seen) = ledger_tracker();
        t.mark_turn_active_from(Some(user("a")));
        t.mark_turn_active_from(Some(jekt()));
        t.end_pass(Some(stats(10, 1)));
        advance(&clock, TURN_SETTLE_MS + 1);
        t.mark_turn_active_from(Some(user("b")));
        let seqs: Vec<u64> = seen.lock().unwrap().iter().map(|l| l.seq).collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
        assert_eq!(seqs.first(), Some(&1));
    }

    /// A hint for a message that reached a running pass is spent there: the
    /// next fresh message starts its own turn (#4492).
    #[test]
    fn a_hint_spent_on_a_running_pass_never_joins_a_later_fresh_message() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.hint_join(id);
        t.mark_turn_active_from(Some(user("held, delivered mid-pass")));
        t.end_pass(None);
        advance(&clock, TURN_SETTLE_MS + 1);
        t.mark_turn_active_returning_was_active();
        assert_ne!(t.ledger().unwrap().turn_id, id);
    }

    /// A settle window that lapsed with no next pass counts as the turn's end
    /// for a held message's join, as the pane reads it (#4492).
    #[test]
    fn a_held_message_joins_a_turn_whose_settle_window_lapsed() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.mark_turn_active_from(Some(jekt())); // answered inside the pass
        t.end_pass(None);
        advance(&clock, TURN_SETTLE_MS + 3_000); // lapsed, nothing wrote the end
        let held = TurnInput { origin: TurnOrigin::User, text: "held".into(), joins_turn: Some(id) };
        t.mark_turn_active_from(Some(held));
        assert_eq!(t.ledger().unwrap().turn_id, id);
    }

    /// A held message queued behind a spawn keeps its join (#4492).
    #[test]
    fn a_held_message_queued_behind_a_spawn_keeps_its_join() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        let id = t.ledger().unwrap().turn_id;
        t.end_pass(None);
        advance(&clock, 200);
        t.hint_next_turn(TurnInput { origin: TurnOrigin::User, text: "held".into(), joins_turn: Some(id) });
        t.set_active_turn(true); // the spawn starts the queued pass
        assert_eq!(t.ledger().unwrap().turn_id, id);
    }

    #[test]
    fn a_cleared_join_hint_joins_nothing() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_returning_was_active();
        let id = t.ledger().unwrap().turn_id;
        t.end_pass(None);
        t.hint_join(id);
        t.clear_join_hint(); // the dispatch failed
        advance(&clock, 100);
        t.mark_turn_active_returning_was_active();
        assert_ne!(t.ledger().unwrap().turn_id, id);
    }

    // ---- turn triggers (turn-model spec §5.2) ----

    fn jekt_from(from: &str) -> TurnInput {
        TurnInput {
            origin: TurnOrigin::Automated,
            text: format!("[JEKT:FROM={from} TO=agent5 TIER=coord DELIVERY=wan TRUST=network-claimed]\n───\nhello\n[/JEKT]"),
            joins_turn: None,
        }
    }

    fn trig(kind: TriggerKind, from: Option<&str>) -> TurnTrigger {
        TurnTrigger { kind, from: from.map(str::to_string) }
    }

    #[test]
    fn an_automated_input_is_named_by_the_jekt_marker_srv_put_on_it() {
        let of = |i: TurnInput| TurnTrigger::from_input(i.origin, &i.text);
        assert_eq!(of(jekt_from("agentx")), trig(TriggerKind::Agent, Some("agentx")));
        assert_eq!(of(jekt_from("cron")), trig(TriggerKind::Schedule, Some("cron")));
        assert_eq!(of(jekt_from("github-consumer")), trig(TriggerKind::Service, Some("github-consumer")));
        // The same marker inside the stream-json line that carries it.
        let line = serde_json::json!({"type": "user", "message": {"role": "user", "content": jekt_from("korp").text}});
        assert_eq!(TurnTrigger::from_input(TurnOrigin::Automated, &line.to_string()), trig(TriggerKind::Agent, Some("korp")));
        // No marker: still automated, sender unknown.
        assert_eq!(TurnTrigger::from_input(TurnOrigin::Automated, "nudge"), trig(TriggerKind::Agent, None));
    }

    #[test]
    fn the_user_s_own_words_are_never_read_as_a_jekt() {
        let quoted = "what does [JEKT:FROM=cron TIER=coord] mean?";
        assert_eq!(TurnTrigger::from_input(TurnOrigin::User, quoted), trig(TriggerKind::User, None));
        let broadcast = "[BROADCAST:FROM=user VIA=swarm TO=Agent5 RECIPIENTS=7 MSGID=x TS=1]\nhow's it going";
        assert_eq!(TurnTrigger::from_input(TurnOrigin::User, broadcast), trig(TriggerKind::Broadcast, None));
        assert_eq!(TurnTrigger::from_input(TurnOrigin::System, "reinjection"), trig(TriggerKind::System, None));
    }

    #[test]
    fn a_task_summary_is_kept_and_a_long_one_shortened() {
        assert_eq!(TurnTrigger::task(Some("  npm test completed ")), trig(TriggerKind::Task, Some("npm test completed")));
        assert_eq!(TurnTrigger::task(None), trig(TriggerKind::Task, None));
        let long = TurnTrigger::task(Some(&"x".repeat(400))).from.unwrap();
        assert_eq!(long.chars().count(), 160);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_turn_names_its_trigger_and_lists_what_joined_it() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(jekt_from("agentx")));
        t.mark_turn_active_from(Some(jekt_from("github-consumer"))); // mid-pass
        advance(&clock, 1_000);
        t.set_active_turn(false);
        advance(&clock, 10);
        let held = TurnInput { origin: TurnOrigin::User, text: "also this".into(), joins_turn: Some(t.ledger().unwrap().turn_id) };
        t.mark_turn_active_from(Some(held));
        let l = t.ledger().unwrap();
        assert_eq!(l.trigger, Some(trig(TriggerKind::Agent, Some("agentx"))));
        assert_eq!(l.absorbed, vec![trig(TriggerKind::Service, Some("github-consumer")), trig(TriggerKind::User, None)]);
        assert_eq!(l.inputs, 2);
    }

    #[test]
    fn a_task_wake_up_is_named_by_the_task() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("start the build")));
        t.set_active_turn(false);
        advance(&clock, 30_000);
        t.note_cli_task_notification(Some("Background command \"npm run build\" completed (exit code 0)"));
        assert!(t.mark_turn_active_from_cli());
        assert_eq!(
            t.ledger().unwrap().trigger,
            Some(trig(TriggerKind::Task, Some("Background command \"npm run build\" completed (exit code 0)")))
        );
    }

    #[test]
    fn a_message_queued_at_spawn_names_the_turn_it_starts() {
        let (t, _, _) = ledger_tracker();
        t.hint_next_turn(jekt_from("cron"));
        t.set_active_turn(true); // the spawn
        assert_eq!(t.ledger().unwrap().trigger, Some(trig(TriggerKind::Schedule, Some("cron"))));
    }

    #[test]
    fn absorbed_inputs_are_capped_but_still_counted() {
        let (t, _, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        for _ in 0..(ABSORBED_CAP + 5) {
            t.mark_turn_active_from(Some(jekt_from("agentx")));
        }
        let l = t.ledger().unwrap();
        assert_eq!((l.absorbed.len(), l.inputs as usize), (ABSORBED_CAP, ABSORBED_CAP + 5));
    }

    #[test]
    fn the_trigger_serializes_for_the_pane() {
        let (t, _, _) = ledger_tracker();
        t.mark_turn_active_from(Some(jekt_from("agentx")));
        let v = serde_json::to_value(t.ledger().unwrap()).unwrap();
        assert_eq!(v["trigger"], serde_json::json!({"kind": "agent", "from": "agentx"}));
        assert!(v.get("absorbed").is_none(), "empty: absent");
    }

    /// A task the CLI wakes for inside a turn is listed as what joined it (#4503).
    #[test]
    fn a_task_wake_that_joins_a_turn_is_recorded() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.note_cli_task_notification(Some("Background command \"npm test\" completed (exit code 0)"));
        t.end_pass(None);
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli());
        let l = t.ledger().unwrap();
        assert_eq!((l.passes, l.inputs), (2, 1));
        assert_eq!(l.trigger, Some(trig(TriggerKind::User, None)), "the turn is still the user's");
        assert_eq!(l.absorbed.first().map(|t| t.kind), Some(TriggerKind::Task));
    }

    /// A plain continuation (input answered in its own pass) adds no input.
    #[test]
    fn a_plain_continuation_adds_no_input() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(user("more")));
        t.end_pass(None);
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli());
        assert_eq!(t.ledger().unwrap().inputs, 1, "only the mid-pass message");
    }

    /// A task notification that loses the race to a user pass is not taken
    /// for a later plain continuation (#4503).
    #[test]
    fn a_task_that_lost_the_race_is_not_credited_to_a_later_continuation() {
        let (t, clock, _) = ledger_tracker();
        t.note_cli_task_notification(Some("done")); // while idle
        t.mark_turn_active_from(Some(user("go"))); // the user's pass wins
        t.mark_turn_active_from(Some(user("more"))); // answered in its own pass
        t.end_pass(None);
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli()); // a plain continuation
        let l = t.ledger().unwrap();
        assert_eq!(l.inputs, 1, "only the mid-pass message");
        assert!(l.absorbed.iter().all(|a| a.kind != TriggerKind::Task));
    }

    /// A CLI-started pass with no task notification gets no made-up sender.
    #[test]
    fn a_cli_pass_with_no_task_has_no_trigger() {
        let mut inner = TurnActivityTrackerInner {
            active_turn: true,
            exit_code: None,
            provenance: None,
            next_turn: None,
            ledger: None,
            input_during_pass: false,
            cli_wake_pending: false,
            continuation_expected: false,
            continuation_provenance: None,
            pending_join: None,
            ledger_seq: 0,
            next_turn_trigger: None,
            pending_task_trigger: None,
        };
        inner.begin_pass(PassStart::CliWake, None, 1_000);
        assert_eq!(inner.ledger.unwrap().trigger, None);
    }

    /// A settling turn reads, once lapsed, as ended at its last pass.
    #[test]
    fn a_lapsed_settling_ledger_closes_at_its_last_pass() {
        let (t, clock, _) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(jekt()));
        advance(&clock, 500);
        t.end_pass(None);
        let settling = t.ledger().unwrap();
        let closed = closed_when_lapsed(&settling).expect("settling");
        assert_eq!(closed.ended_at_ms, settling.last_pass_ended_at_ms);
        assert_eq!((closed.end, closed.settle_until_ms, closed.seq), (Some(TurnEnd::Completed), None, settling.seq));
        assert_eq!(closed_when_lapsed(&closed), None, "already closed");
        t.mark_turn_active_from(Some(user("next")));
        assert_eq!(closed_when_lapsed(&t.ledger().unwrap()), None, "running");
    }

    /// Each kind of ledger change has its own `[turn]` line.
    #[test]
    fn ledger_changes_are_named_for_the_log() {
        let (t, clock, seen) = ledger_tracker();
        t.mark_turn_active_from(Some(user("go")));
        t.mark_turn_active_from(Some(jekt()));
        t.end_pass(Some(stats(10, 1)));
        advance(&clock, 20);
        assert!(t.mark_turn_active_from_cli());
        t.end_pass(None);
        let seen = seen.lock().unwrap();
        let named: Vec<_> = std::iter::once(ledger_transition(None, seen.first()))
            .chain(seen.windows(2).map(|w| ledger_transition(Some(&w[0]), Some(&w[1]))))
            .flatten()
            .collect();
        assert_eq!(
            named,
            vec![
                "opened",
                "input joined the running pass",
                "pass ended, settling for a next one",
                "next pass joined the turn",
                "ended",
            ]
        );
    }
}
