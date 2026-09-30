// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase CPD-5 — host-side saga command dispatch + idempotency LRU.
//
// After CPD-1+2+3 merged, the launcher dispatches `IssueCmd::Host`
// saga commands carrying `saga_id` over the launcher → host pipe.
// CPD-3 made the wire live; this module makes host processing
// idempotent so a launcher retry (e.g. after host pipe reconnect
// drains the launcher's `pending_buffer`) does NOT re-execute the
// same command — instead the host re-emits the same `Report*` reply
// it produced the first time.
//
// **Why not just dedupe at the launcher?** The launcher already does
// best-effort dedupe via single-flight per saga, but the buffer →
// reconnect → drain path can legitimately re-deliver a command if
// the host crashed mid-processing (the launcher has no way to know
// whether the host saw the original send). Defense-in-depth: launcher
// avoids resends when it can; host absorbs the rest with this LRU.
//
// **Key:** `(saga_id, CommandKind)`. `saga_id` alone is not enough —
// a future feature could legitimately issue multiple distinct host
// commands inside one saga (e.g. a saga that both reaps panes AND
// drains the pool, each with a different `Command` variant). Keying
// on `(saga_id, kind)` lets the LRU hold one entry per (saga, action
// type) pair without collisions.
//
// **Bound:** 256 entries. Saga rate is human-driven (window opens /
// closes); 256 covers far more than any realistic concurrent-saga
// burst. Drop-oldest on overflow.
//
// **Test coverage:** see `#[cfg(test)] mod tests` at the bottom —
// covers (a) duplicate command re-emits same Report without re-action,
// (b) LRU eviction at the 257th distinct entry preserves recency
// ordering.

use std::collections::VecDeque;
use std::sync::Arc;

use agentmux_common::ipc::Command;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

/// Discriminator for the host-bound saga command variants. Tracks
/// the three commands the host actually consumes today
/// (`SpawnPoolWindow`, `ReapPanes`, `DrainPoolIfLast`); future host-
/// bound commands add a variant here.
///
/// Used as half of the LRU key `(saga_id, CommandKind)` so the same
/// `saga_id` can carry multiple distinct host actions (a saga that
/// reaps panes AND drains pool, each with a different Command kind)
/// without collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    SpawnPoolWindow,
    ReapPanes,
    DrainPoolIfLast,
}

impl CommandKind {
    /// Extract the discriminator from a host-bound saga `Command`.
    /// Returns `None` for commands that aren't host-bound saga
    /// payloads (e.g. `Report*` Commands flowing in the OTHER
    /// direction, or `Register` / `Ping`).
    pub fn from_command(cmd: &Command) -> Option<Self> {
        match cmd {
            Command::SpawnPoolWindow { .. } => Some(Self::SpawnPoolWindow),
            Command::ReapPanes { .. } => Some(Self::ReapPanes),
            Command::DrainPoolIfLast { .. } => Some(Self::DrainPoolIfLast),
            _ => None,
        }
    }
}

/// Maximum number of `(saga_id, kind)` entries held in the LRU.
/// Spec §3.7: bound 256 — far above any realistic concurrent saga
/// count.
pub const SAGA_LRU_CAP: usize = 256;

/// Idempotency LRU keyed by `(saga_id, CommandKind)`. Stores the
/// resulting `Report*` Command so a duplicate dispatch re-emits the
/// same reply payload without re-running the action.
///
/// Implementation: simple `VecDeque` of `(key, report)` pairs. Front
/// is oldest, back is most-recent. Linear scan on lookup is fine at
/// `cap=256`: the entire scan is a few microseconds and only happens
/// once per saga command (a human-rate event).
pub struct SagaIdempotencyLru {
    entries: VecDeque<((u64, CommandKind), Command)>,
    cap: usize,
}

impl SagaIdempotencyLru {
    pub fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(cap),
            cap,
        }
    }

    pub fn with_default_cap() -> Self {
        Self::new(SAGA_LRU_CAP)
    }

    /// Look up the cached Report for `(saga_id, kind)`. On hit,
    /// promotes the entry to most-recent (back of the deque) and
    /// returns a clone of the cached Report. On miss, returns None.
    pub fn get(&mut self, saga_id: u64, kind: CommandKind) -> Option<Command> {
        let key = (saga_id, kind);
        let pos = self.entries.iter().position(|(k, _)| *k == key)?;
        // Promote to most-recent: remove + push to back.
        let (k, v) = self.entries.remove(pos)?;
        let cloned = v.clone();
        self.entries.push_back((k, v));
        Some(cloned)
    }

    /// Cache the Report for `(saga_id, kind)`. If at capacity, drops
    /// the oldest entry (front). If a duplicate key exists, updates
    /// in place + promotes (defensive — caller usually hits via
    /// `get` first; this branch covers a race where two duplicate
    /// commands raced through `get` with both seeing miss).
    pub fn insert(&mut self, saga_id: u64, kind: CommandKind, report: Command) {
        let key = (saga_id, kind);
        if let Some(pos) = self.entries.iter().position(|(k, _)| *k == key) {
            // Defensive duplicate — replace + promote.
            self.entries.remove(pos);
        } else if self.entries.len() >= self.cap {
            // Drop-oldest on overflow.
            self.entries.pop_front();
        }
        self.entries.push_back((key, report));
    }

    /// Return the saga_id of the oldest entry, or None if empty.
    /// Used in tests to verify drop-oldest semantics.
    #[cfg(test)]
    pub fn oldest_saga_id(&self) -> Option<u64> {
        self.entries.front().map(|(k, _)| k.0)
    }
}

impl Default for SagaIdempotencyLru {
    fn default() -> Self {
        Self::with_default_cap()
    }
}

/// Outcome of `dispatch_host_command`: tells the caller whether the
/// dispatch was a fresh execution (action ran, Report was inserted
/// + sent) or a duplicate (Report was re-sent from cache, action
/// did NOT re-run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// First-time dispatch: action executed, cached Report sent.
    Fresh,
    /// Duplicate dispatch: cached Report re-emitted, no action.
    Duplicate,
    /// Command had no `saga_id` (or `saga_id == 0`, the
    /// "no-saga" sentinel) — the LRU is bypassed and the action
    /// runs unconditionally. This branch is mostly defensive: the
    /// launcher's CPD-3 wiring always issues with a real saga_id,
    /// but a stray legacy / forward-compat dispatch could hit this.
    NoSagaBypass,
    /// Command kind isn't a known host-bound saga payload (e.g.
    /// the reader received a `Report*` echo that should never have
    /// been sent down to the host). Logged at warn; no action, no
    /// reply.
    Unrecognized,
}

/// Dispatcher trait — abstracted so tests can inject a fake action
/// runner without spawning real CEF windows. Production code uses
/// `LiveActionRunner` which calls into `commands::window_pool` /
/// host close-path code.
pub trait SagaActionRunner: Send + Sync {
    /// Run the `SpawnPoolWindow` action. Returns the resulting
    /// pool window label (the host normally synthesizes a
    /// `window-pool-<uuid>` label; for the saga reply we report
    /// "pending" so the launcher reducer can correlate the
    /// follow-up `ReportPoolWindowAdded` organic event by saga_id).
    /// In production the spawn is fire-and-forget on the UI thread
    /// — the actual label is reported via the existing organic
    /// `report_pool_window_added` path. The saga's `Report` here
    /// carries an empty/sentinel label indicating "spawn requested,
    /// pool will fill asynchronously."
    fn spawn_pool_window(&self) -> String;

    /// Run the `ReapPanes` action for the named window. In current
    /// architecture the host's `on_before_close` already drains
    /// panes synchronously when a window closes, so this is a
    /// best-effort acknowledge — the saga relies on the organic
    /// `Event::PanesReaped` (via `report_panes_reaped` in
    /// `client.rs`) for the real signal. The Report this returns
    /// is a saga-correlated echo so the saga's `expected_saga_id`
    /// filter matches.
    fn reap_panes(&self, label: &str);

    /// Run the `DrainPoolIfLast` action for the named window.
    /// Returns true if the host considers `label` to have been the
    /// last user-visible window (i.e. a drain WOULD be triggered).
    /// Like `reap_panes`, the host's existing `on_before_close`
    /// already does this decision inline; the saga's command path
    /// is a re-issue / confirmation channel.
    fn drain_pool_if_last(&self, label: &str) -> bool;
}

/// Dispatch a host-bound saga `Command`: check the LRU, run the
/// action if not cached, build the corresponding `Report*`,
/// cache it, and send via `reply_tx`. Returns the outcome so callers
/// can log / count fresh vs. duplicate dispatch.
///
/// `lru` is a shared `Arc<Mutex<...>>` so the read-loop task and
/// any future direct-dispatch path share a single cache.
pub fn dispatch_host_command<R: SagaActionRunner>(
    cmd: &Command,
    runner: &R,
    lru: &Arc<Mutex<SagaIdempotencyLru>>,
    reply_tx: &UnboundedSender<Command>,
) -> DispatchOutcome {
    let kind = match CommandKind::from_command(cmd) {
        Some(k) => k,
        None => {
            tracing::warn!(
                "[saga-dispatch] received non-host-bound command on host pipe: {:?}",
                cmd
            );
            return DispatchOutcome::Unrecognized;
        }
    };

    // Extract saga_id from the command. For each variant the field
    // name is `saga_id: u64`. `0` is the "no saga" sentinel per
    // CPD-1 spec — bypass the LRU in that case (the action just
    // runs without dedupe; useful for legacy / forward-compat
    // launchers that stamp `0`).
    let saga_id = match cmd {
        Command::SpawnPoolWindow { saga_id } => *saga_id,
        Command::ReapPanes { saga_id, .. } => *saga_id,
        Command::DrainPoolIfLast { saga_id, .. } => *saga_id,
        _ => unreachable!("kind matched but variant didn't — schema drift"),
    };

    if saga_id == 0 {
        // Legacy / no-saga path: run action, build Report with
        // saga_id = None (organic), send. No LRU touch.
        let report = build_and_run_report(cmd, kind, runner, None);
        let _ = reply_tx.send(report);
        return DispatchOutcome::NoSagaBypass;
    }

    // Hot path — saga_id present. Check LRU.
    {
        let mut guard = lru.lock();
        if let Some(cached_report) = guard.get(saga_id, kind) {
            tracing::info!(
                "[saga-dispatch] duplicate saga command (saga_id={}, kind={:?}) — re-emitting cached report",
                saga_id, kind
            );
            // Send the cached report; do NOT re-run the action.
            let _ = reply_tx.send(cached_report);
            return DispatchOutcome::Duplicate;
        }
    }

    // Miss — run action, build Report, cache, send. We hold no
    // lock across the action call (action may post UI tasks /
    // touch CEF state).
    let report = build_and_run_report(cmd, kind, runner, Some(saga_id));
    {
        let mut guard = lru.lock();
        guard.insert(saga_id, kind, report.clone());
    }
    let _ = reply_tx.send(report);
    DispatchOutcome::Fresh
}

/// Run the action for `cmd` and synthesize the corresponding
/// `Report*` Command. `saga_id_for_report` is what the Report's
/// echo field carries — `Some(N)` when the dispatch was saga-driven,
/// `None` for the no-saga bypass path.
fn build_and_run_report<R: SagaActionRunner>(
    cmd: &Command,
    kind: CommandKind,
    runner: &R,
    saga_id_for_report: Option<u64>,
) -> Command {
    match (cmd, kind) {
        (Command::SpawnPoolWindow { .. }, CommandKind::SpawnPoolWindow) => {
            let label = runner.spawn_pool_window();
            Command::ReportPoolWindowAdded {
                label,
                saga_id: saga_id_for_report,
            }
        }
        (Command::ReapPanes { label, .. }, CommandKind::ReapPanes) => {
            runner.reap_panes(label);
            Command::ReportPanesReaped {
                label: label.clone(),
                saga_id: saga_id_for_report,
            }
        }
        (Command::DrainPoolIfLast { label, .. }, CommandKind::DrainPoolIfLast) => {
            let was_last = runner.drain_pool_if_last(label);
            Command::ReportPoolDrainDecision {
                label: label.clone(),
                was_last,
                saga_id: saga_id_for_report,
            }
        }
        _ => unreachable!("kind/cmd mismatch — guarded by from_command()"),
    }
}

// ── Production action runner ──────────────────────────────────────
//
// Wraps real host code paths. Kept thin so the test runner can
// substitute deterministic stubs without pulling in CEF.

// (A1.2 — gate removed; body is platform-neutral and the Unix IPC
// client now also needs it. The original gate was defensive because
// the only caller was Windows-only.)
pub struct LiveActionRunner {
    pub state: Arc<crate::state::AppState>,
}

impl SagaActionRunner for LiveActionRunner {
    fn spawn_pool_window(&self) -> String {
        // Fire the real spawn. The host's existing
        // `report_pool_window_added` organic path will report the
        // freshly minted label; the saga-correlated reply this
        // dispatcher emits carries an empty label as a sentinel —
        // the launcher reducer matches by saga_id, not label.
        crate::commands::window_pool::spawn_pool_window(&self.state);
        String::new()
    }

    fn reap_panes(&self, label: &str) {
        // Host's `on_before_close` is the canonical pane-reaper.
        // A saga-issued `ReapPanes` for a window whose close is
        // already in flight is a redundant nudge — log and rely
        // on the organic `report_panes_reaped` for the real signal.
        // Future expansion (forced-close-from-saga) can hook here.
        tracing::info!(
            "[saga-dispatch] ReapPanes saga command for label={} — acknowledged (host close-path is canonical reaper)",
            label
        );
    }

    fn drain_pool_if_last(&self, label: &str) -> bool {
        // Compute the same condition `on_before_close` uses to
        // decide drain. MUST mirror that gate exactly: same pool
        // inventory (unpromoted ∪ queue), same atomic-snapshot
        // discipline. A two-lock variant or unpromoted-only check
        // here lets a queued pool window inflate `user_count` and
        // suppress the drain when the user actually did close
        // their last visible window.
        let (pool_inventory, browsers) = self.state.user_visibility_snapshot();
        let labels: Vec<String> = browsers.into_iter().map(|(l, _)| l).collect();
        let user_count = labels
            .iter()
            .filter(|k| !pool_inventory.contains(k.as_str()) && !k.starts_with("browser-pane-"))
            .count();
        // `was_last` semantics: closing window is the last user-
        // visible window. Caller's `label` should be subtracted —
        // but at saga-dispatch time the close hasn't happened yet
        // (or has just happened); count of 0 OR count of 1 with
        // the closing window in the set both indicate "last."
        let label_present = labels.iter().any(|k| k == label);
        user_count == 0 || (user_count == 1 && label_present)
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    /// Test runner: counts how many times each action runs so
    /// duplicate-dispatch tests can verify idempotency.
    struct CountingRunner {
        spawn_calls: AtomicUsize,
        reap_calls: AtomicUsize,
        drain_calls: AtomicUsize,
        drain_returns_was_last: bool,
    }

    impl CountingRunner {
        fn new() -> Self {
            Self {
                spawn_calls: AtomicUsize::new(0),
                reap_calls: AtomicUsize::new(0),
                drain_calls: AtomicUsize::new(0),
                drain_returns_was_last: false,
            }
        }
    }

    impl SagaActionRunner for CountingRunner {
        fn spawn_pool_window(&self) -> String {
            let n = self.spawn_calls.fetch_add(1, Ordering::SeqCst);
            format!("window-pool-test-{}", n)
        }

        fn reap_panes(&self, _label: &str) {
            self.reap_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn drain_pool_if_last(&self, _label: &str) -> bool {
            self.drain_calls.fetch_add(1, Ordering::SeqCst);
            self.drain_returns_was_last
        }
    }

    fn drain_replies(rx: &mut mpsc::UnboundedReceiver<Command>) -> Vec<Command> {
        let mut out = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            out.push(cmd);
        }
        out
    }

    #[test]
    fn duplicate_spawn_pool_window_does_not_respawn_but_reemits_report() {
        let runner = CountingRunner::new();
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, mut rx) = mpsc::unbounded_channel();

        let cmd = Command::SpawnPoolWindow { saga_id: 42 };

        // First dispatch — fresh.
        let outcome1 = dispatch_host_command(&cmd, &runner, &lru, &tx);
        assert_eq!(outcome1, DispatchOutcome::Fresh);
        assert_eq!(runner.spawn_calls.load(Ordering::SeqCst), 1);

        // Second dispatch with same saga_id — duplicate; should
        // NOT re-spawn, but SHOULD re-emit the same Report.
        let outcome2 = dispatch_host_command(&cmd, &runner, &lru, &tx);
        assert_eq!(outcome2, DispatchOutcome::Duplicate);
        assert_eq!(
            runner.spawn_calls.load(Ordering::SeqCst),
            1,
            "spawn must not re-execute on duplicate"
        );

        let replies = drain_replies(&mut rx);
        assert_eq!(replies.len(), 2, "expected one reply per dispatch");
        // Both replies must serialize to byte-identical JSON
        // (Command itself isn't PartialEq because it carries
        // some non-comparable nested types).
        let j0 = serde_json::to_string(&replies[0]).unwrap();
        let j1 = serde_json::to_string(&replies[1]).unwrap();
        assert_eq!(j0, j1, "duplicate dispatch must re-emit identical Report");
        match &replies[0] {
            Command::ReportPoolWindowAdded { label, saga_id } => {
                assert_eq!(label, "window-pool-test-0");
                assert_eq!(*saga_id, Some(42));
            }
            other => panic!("unexpected reply: {:?}", other),
        }
    }

    #[test]
    fn duplicate_reap_panes_does_not_rerun_but_reemits_report() {
        let runner = CountingRunner::new();
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, mut rx) = mpsc::unbounded_channel();

        let cmd = Command::ReapPanes {
            label: "win-1".to_string(),
            saga_id: 7,
        };

        assert_eq!(
            dispatch_host_command(&cmd, &runner, &lru, &tx),
            DispatchOutcome::Fresh
        );
        assert_eq!(
            dispatch_host_command(&cmd, &runner, &lru, &tx),
            DispatchOutcome::Duplicate
        );
        assert_eq!(runner.reap_calls.load(Ordering::SeqCst), 1);

        let replies = drain_replies(&mut rx);
        assert_eq!(replies.len(), 2);
        let j0 = serde_json::to_string(&replies[0]).unwrap();
        let j1 = serde_json::to_string(&replies[1]).unwrap();
        assert_eq!(j0, j1);
    }

    #[test]
    fn duplicate_drain_pool_if_last_reuses_was_last_decision() {
        let mut runner = CountingRunner::new();
        runner.drain_returns_was_last = true;
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, mut rx) = mpsc::unbounded_channel();

        let cmd = Command::DrainPoolIfLast {
            label: "win-1".to_string(),
            saga_id: 99,
        };

        assert_eq!(
            dispatch_host_command(&cmd, &runner, &lru, &tx),
            DispatchOutcome::Fresh
        );
        assert_eq!(
            dispatch_host_command(&cmd, &runner, &lru, &tx),
            DispatchOutcome::Duplicate
        );
        assert_eq!(runner.drain_calls.load(Ordering::SeqCst), 1);

        let replies = drain_replies(&mut rx);
        assert_eq!(replies.len(), 2);
        // Even though the runner's `drain_returns_was_last` was
        // captured at first call, the second reply must echo the
        // same `was_last` (defense against runner state changing
        // mid-flight — the LRU MUST hold the original decision).
        match (&replies[0], &replies[1]) {
            (
                Command::ReportPoolDrainDecision {
                    was_last: a,
                    saga_id: id_a,
                    ..
                },
                Command::ReportPoolDrainDecision {
                    was_last: b,
                    saga_id: id_b,
                    ..
                },
            ) => {
                assert_eq!(a, b);
                assert_eq!(*a, true);
                assert_eq!(id_a, id_b);
                assert_eq!(*id_a, Some(99));
            }
            other => panic!("unexpected reply pair: {:?}", other),
        }
    }

    #[test]
    fn lru_evicts_oldest_at_capacity() {
        // Use a small cap to keep test runtime cheap.
        let cap = 4;
        let mut lru = SagaIdempotencyLru::new(cap);
        for i in 0..cap as u64 {
            lru.insert(
                i,
                CommandKind::SpawnPoolWindow,
                Command::ReportPoolWindowAdded {
                    label: format!("w{}", i),
                    saga_id: Some(i),
                },
            );
        }
        assert_eq!(lru.entries.len(), cap);
        assert_eq!(lru.oldest_saga_id(), Some(0));

        // Insert one more — saga_id=0 should be evicted (oldest).
        lru.insert(
            cap as u64,
            CommandKind::SpawnPoolWindow,
            Command::ReportPoolWindowAdded {
                label: format!("w{}", cap),
                saga_id: Some(cap as u64),
            },
        );
        assert_eq!(lru.entries.len(), cap);
        assert_eq!(lru.oldest_saga_id(), Some(1));
        assert!(lru.get(0, CommandKind::SpawnPoolWindow).is_none());
        assert!(lru.get(cap as u64, CommandKind::SpawnPoolWindow).is_some());
    }

    #[test]
    fn lru_eviction_at_257th_distinct_command() {
        // Per spec §3.7: bound 256.
        let runner = CountingRunner::new();
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, _rx) = mpsc::unbounded_channel();

        // Fill to capacity (256 distinct saga_ids).
        for i in 1..=SAGA_LRU_CAP as u64 {
            let cmd = Command::SpawnPoolWindow { saga_id: i };
            let outcome = dispatch_host_command(&cmd, &runner, &lru, &tx);
            assert_eq!(outcome, DispatchOutcome::Fresh);
        }
        assert_eq!(lru.lock().entries.len(), SAGA_LRU_CAP);

        // 257th distinct command — accepted; oldest (saga_id=1) evicted.
        let cmd = Command::SpawnPoolWindow {
            saga_id: SAGA_LRU_CAP as u64 + 1,
        };
        let outcome = dispatch_host_command(&cmd, &runner, &lru, &tx);
        assert_eq!(outcome, DispatchOutcome::Fresh);
        assert_eq!(lru.lock().entries.len(), SAGA_LRU_CAP);

        // Verify saga_id=1 is gone (replaying it now would be a
        // fresh call, not a duplicate).
        let replay = Command::SpawnPoolWindow { saga_id: 1 };
        let outcome = dispatch_host_command(&replay, &runner, &lru, &tx);
        assert_eq!(
            outcome,
            DispatchOutcome::Fresh,
            "evicted entry must NOT be served from cache"
        );

        // Verify saga_id=257 is still cached (recent).
        let replay = Command::SpawnPoolWindow {
            saga_id: SAGA_LRU_CAP as u64 + 1,
        };
        let outcome = dispatch_host_command(&replay, &runner, &lru, &tx);
        assert_eq!(outcome, DispatchOutcome::Duplicate);
    }

    #[test]
    fn distinct_kinds_with_same_saga_id_dont_collide() {
        let runner = CountingRunner::new();
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, _rx) = mpsc::unbounded_channel();

        // Same saga_id, different kinds — both should be Fresh.
        let spawn = Command::SpawnPoolWindow { saga_id: 5 };
        let reap = Command::ReapPanes {
            label: "x".to_string(),
            saga_id: 5,
        };
        assert_eq!(
            dispatch_host_command(&spawn, &runner, &lru, &tx),
            DispatchOutcome::Fresh
        );
        assert_eq!(
            dispatch_host_command(&reap, &runner, &lru, &tx),
            DispatchOutcome::Fresh
        );

        // Re-issue both — both Duplicate.
        assert_eq!(
            dispatch_host_command(&spawn, &runner, &lru, &tx),
            DispatchOutcome::Duplicate
        );
        assert_eq!(
            dispatch_host_command(&reap, &runner, &lru, &tx),
            DispatchOutcome::Duplicate
        );
    }

    #[test]
    fn saga_id_zero_bypasses_lru() {
        let runner = CountingRunner::new();
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, mut rx) = mpsc::unbounded_channel();

        // saga_id=0 is the "no saga" sentinel; LRU must not track.
        let cmd = Command::SpawnPoolWindow { saga_id: 0 };
        for _ in 0..3 {
            let outcome = dispatch_host_command(&cmd, &runner, &lru, &tx);
            assert_eq!(outcome, DispatchOutcome::NoSagaBypass);
        }
        assert_eq!(runner.spawn_calls.load(Ordering::SeqCst), 3);
        assert_eq!(lru.lock().entries.len(), 0);

        let replies = drain_replies(&mut rx);
        assert_eq!(replies.len(), 3);
        for r in &replies {
            match r {
                Command::ReportPoolWindowAdded { saga_id, .. } => {
                    assert_eq!(*saga_id, None, "no-saga bypass must report None");
                }
                _ => panic!("unexpected reply"),
            }
        }
    }

    #[test]
    fn unrecognized_command_is_logged_and_dropped() {
        let runner = CountingRunner::new();
        let lru = Arc::new(Mutex::new(SagaIdempotencyLru::with_default_cap()));
        let (tx, mut rx) = mpsc::unbounded_channel();

        // A `Report*` Command flowing the wrong way (host → host
        // is nonsense; the launcher sends Reports? — no, Reports
        // are host → launcher only) — verify we don't try to act
        // on it.
        let bogus = Command::ReportPanesReaped {
            label: "x".to_string(),
            saga_id: Some(1),
        };
        let outcome = dispatch_host_command(&bogus, &runner, &lru, &tx);
        assert_eq!(outcome, DispatchOutcome::Unrecognized);
        assert_eq!(runner.reap_calls.load(Ordering::SeqCst), 0);
        assert!(drain_replies(&mut rx).is_empty());
    }

    #[test]
    fn lru_get_promotes_to_most_recent() {
        let mut lru = SagaIdempotencyLru::new(3);
        for i in 1..=3u64 {
            lru.insert(
                i,
                CommandKind::SpawnPoolWindow,
                Command::ReportPoolWindowAdded {
                    label: format!("w{}", i),
                    saga_id: Some(i),
                },
            );
        }
        assert_eq!(lru.oldest_saga_id(), Some(1));
        // Touch saga_id=1 — it should move to most-recent.
        let _ = lru.get(1, CommandKind::SpawnPoolWindow);
        assert_eq!(lru.oldest_saga_id(), Some(2));
        // Insert a new entry; saga_id=2 (now oldest) gets evicted,
        // not saga_id=1 (which we just touched).
        lru.insert(
            4,
            CommandKind::SpawnPoolWindow,
            Command::ReportPoolWindowAdded {
                label: "w4".to_string(),
                saga_id: Some(4),
            },
        );
        assert_eq!(lru.oldest_saga_id(), Some(3));
        assert!(lru.get(2, CommandKind::SpawnPoolWindow).is_none());
        assert!(lru.get(1, CommandKind::SpawnPoolWindow).is_some());
    }
}
