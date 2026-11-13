// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session/directory backfill scanning and stale-subagent reconciliation:
//! `scan_session_subagents` (pane-reopen backfill entry point),
//! `reconcile_stale_subagents` (Active -> Abandoned correction once a
//! parent's turn is confirmed idle), `broadcast_subagents_abandoned`, and
//! `scan_subagents_dir` (the capped cold-backfill file walk).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::completion;
use super::parse::file_mtime;
use super::types::*;
use super::SubagentWatcher;

/// How many times `reconcile_stale_subagents` retries when the parent
/// block's controller isn't registered yet, before giving up and leaving
/// the entries as-is. See that function's own doc comment (2026-09-02
/// update) for why a single 2s attempt proved too short in practice —
/// `MAX_RECONCILE_RETRIES * RECONCILE_RETRY_INTERVAL_MS` (~15s) is a
/// realistic bound for a real controller to finish spawning under load,
/// while still failing open rather than chasing a block forever.
const MAX_RECONCILE_RETRIES: u32 = 5;
const RECONCILE_RETRY_INTERVAL_MS: u64 = 3_000;

/// Publish a `mps::EVENT_SUBAGENT_BACKFILL_STATUS` ping, scoped to this
/// pane's own block id. No-op if `self.broker` was never wired (tests, or a
/// `SubagentWatcher` built via bare `new()`) -- see the `broker` field's own
/// doc comment in `mod.rs`. See
/// docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md
/// section 5.
fn publish_backfill_status(watcher: &SubagentWatcher, parent_block_id: &str, status: &str) {
    let Some(broker) = watcher.broker.lock().unwrap().clone() else { return };
    broker.publish_event::<crate::backend::mps_events::SubagentBackfillStatus>(
        &super::events::SubagentBackfillStatusEvent { status: status.to_string() },
        vec![format!("block:{}", parent_block_id)],
        2,
    );
}

impl SubagentWatcher {
    /// Backfill subagents that already existed before this pane (re)opened,
    /// scoped to exactly the ONE session being resumed — not this agent
    /// identity's entire history. Called from `handle_reactive_register`
    /// when the block being registered already has a persisted
    /// `agent:sessionid` (i.e. it's resuming a prior conversation, not
    /// starting fresh); a brand-new session has nothing to backfill.
    ///
    /// Searches only the top level of `config_dir/projects/*` for a child
    /// directory literally named `session_id` (the nested-per-session
    /// layout observed in practice: `projects/<ws>/<session-uuid>/subagents/`)
    /// — cheap, since an agent identity typically has few distinct project
    /// dirs. Older/flat-layout installs where `subagents/` sits directly
    /// under the project dir with no session-level folder have no way to
    /// isolate one session's subagents from another's at the filesystem
    /// level; in that case this intentionally finds nothing rather than
    /// falling back to scanning everything.
    pub fn scan_session_subagents(
        &self,
        parent_agent: &str,
        parent_block_id: &str,
        config_dir: &Path,
        session_id: &str,
    ) {
        // "started"/"done" wrap the whole call, regardless of which of
        // `scan_session_subagents_inner`'s several return paths is actually
        // taken (nothing found at all, or found-and-processed) -- see
        // `publish_backfill_status`'s own doc comment. A pane whose block
        // has no persisted session id never calls this at all (see the
        // caller in `server/reactive.rs`), so it never sees either status
        // and its `ready()` gate is unaffected by this signal.
        //
        // reagentx P2 (PR #2781): generation-gated "done" -- see
        // `backfill_generation`'s own doc comment in `mod.rs`. "started" is
        // always published unconditionally; a stale/superseded call's own
        // "started" is harmless (the gate just stays closed either way).
        let my_generation = {
            let mut gens = self.backfill_generation.lock().unwrap();
            let next = gens.get(parent_block_id).copied().unwrap_or(0) + 1;
            gens.insert(parent_block_id.to_string(), next);
            next
        };
        publish_backfill_status(self, parent_block_id, "started");
        self.scan_session_subagents_inner(parent_agent, parent_block_id, config_dir, session_id);
        if self.is_backfill_generation_current(parent_block_id, my_generation) {
            publish_backfill_status(self, parent_block_id, "done");
        }
    }

    /// Whether `generation` (captured at the start of one
    /// `scan_session_subagents` call) is still the most recent generation
    /// recorded for `parent_block_id` — i.e. no NEWER overlapping call has
    /// started since. See `backfill_generation`'s own doc comment in
    /// `mod.rs`.
    pub(super) fn is_backfill_generation_current(&self, parent_block_id: &str, generation: u64) -> bool {
        self.backfill_generation.lock().unwrap().get(parent_block_id).copied() == Some(generation)
    }

    fn scan_session_subagents_inner(
        &self,
        parent_agent: &str,
        parent_block_id: &str,
        config_dir: &Path,
        session_id: &str,
    ) {
        let projects_dir = config_dir.join("projects");
        let Ok(walker) = std::fs::read_dir(&projects_dir) else { return };

        for entry in walker.flatten() {
            let session_dir = entry.path().join(session_id);
            let subagents_dir = session_dir.join("subagents");
            if subagents_dir.is_dir() {
                // Correlates a pane (re)open with the session it's backfilling —
                // needed to tell "this session was backfilled once" apart from
                // "this session was backfilled repeatedly under different
                // parent_block_ids" (a subagent's parent_block_id is fixed at
                // first discovery, see process_jsonl_change; a mismatch here is
                // the mechanism a NAME/grouping-dedup bug would leave a trail in).
                //
                // reagent (PR #2143 round 1): info!, not debug! — the default
                // production EnvFilter (crates/srv/src/main.rs, "agentmuxsrv=
                // info,info") drops debug-level lines unless RUST_LOG=debug is
                // already set, which would make this diagnostic invisible in a
                // normally-running srv, defeating the point of adding it.
                tracing::info!(
                    agent = %parent_agent,
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    dir = %subagents_dir.display(),
                    "backfilling session subagents on pane (re)open"
                );
                self.scan_subagents_dir(parent_agent, parent_block_id, &subagents_dir);
                self.reconcile_stale_subagents(parent_block_id, session_id);
                return; // session ids are unique — no need to keep scanning
            }
        }
    }

    /// After a full session backfill scan, downgrade any subagent that's
    /// still `Active` to `Abandoned` if its parent block's turn is not
    /// currently active. A subagent runs inside its parent's own CLI
    /// process — a Task-tool call is synchronous within the parent's turn —
    /// so once the parent's turn has ended, any subagent file lacking a
    /// terminal `Result` line was interrupted (crashed, killed, or the
    /// app/srv restarted mid-task), not still running. This call site is
    /// reached from `scan_session_subagents`, itself only invoked when a
    /// block re-registers with a pre-existing session id (`reactive.rs`) —
    /// i.e. a freshly-spawned controller whose turn genuinely hasn't
    /// started yet, so every subagent found on disk predates this process
    /// and is unambiguously history, not in-flight.
    ///
    /// Only reconciles on a *confirmed-idle* read (`Some(false)`).
    /// `Some(true)` (confirmed active) skips with no follow-up — correct,
    /// nothing to reconcile. `None` ("no controller registered yet") used
    /// to be folded into the same "assume active, don't touch it" bucket as
    /// confirmed-active, permanently — a previously-flagged, unconfirmed
    /// race (`SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`
    /// §5 Open Question 2) where this call site, reached from
    /// `scan_session_subagents` at pane-reopen backfill time, can run
    /// before the freshly-spawned controller has registered in
    /// `CONTROLLER_REGISTRY`. Left unresolved, an entry hitting that race
    /// stayed `Active`-looking forever, since nothing else ever revisits it
    /// (see `SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md`
    /// §2, mechanism 4). Fixed by treating `None` as "retry, don't give up
    /// silently" (`retry_reconcile_once`) instead of a terminal no-op —
    /// still conservative (a genuine `Some(true)` never retries; only the
    /// ambiguous case does, and only up to `MAX_RECONCILE_RETRIES` times).
    ///
    /// 2026-09-02 (docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md):
    /// the original version of this fix allowed exactly ONE 2s retry before
    /// giving up permanently. Confirmed live on a real agent whose CLI
    /// controller never registered at all (an expired provider login — the
    /// process either never spawns or spawns and exits before reaching
    /// `register_controller`): the single 2s retry window is frequently too
    /// short to distinguish "controller is still starting up, wait" from
    /// "controller is never coming" — for the FORMER case this left the
    /// pane's Activity Dock showing genuinely-dead subagents as `Active`
    /// indefinitely (no live turn-end ever fires to retry later either),
    /// and for the LATTER case a couple of seconds is too optimistic a
    /// bound for how long a real controller can legitimately take to spawn
    /// and register under load. Extended to a bounded retry loop
    /// (`MAX_RECONCILE_RETRIES` attempts, `RECONCILE_RETRY_INTERVAL_MS`
    /// apart) — still fails open in bounded time (~`MAX_RECONCILE_RETRIES *
    /// RECONCILE_RETRY_INTERVAL_MS`), just with a realistic window instead
    /// of a single throw-away check.
    ///
    /// Called from two places as of SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_
    /// RETIRE_2026_07_20 Phase A: `scan_session_subagents` (reopen/backfill,
    /// unchanged) and `blockcontroller::persistent`'s turn-end hook (live —
    /// closes docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md
    /// Open Question 1, which deliberately deferred real-time wiring).
    /// `pub(crate)` so the live call site (a different module) can reach it.
    /// Thin entry point — always starts the retry budget at
    /// `MAX_RECONCILE_RETRIES`.
    pub(crate) fn reconcile_stale_subagents(&self, parent_block_id: &str, session_id: &str) {
        self.reconcile_stale_subagents_impl(parent_block_id, session_id, MAX_RECONCILE_RETRIES);
    }

    // pub(super), not private — `tests.rs` (a sibling module, not a
    // descendant of `scan`) needs to call this directly with
    // `retries_remaining: 0` to test the exhausted-retry path without
    // depending on a real spawned watcher's tokio::spawn actually firing.
    pub(super) fn reconcile_stale_subagents_impl(&self, parent_block_id: &str, session_id: &str, retries_remaining: u32) {
        if !self.parent_turn_is_confirmed_idle(parent_block_id, session_id, retries_remaining) {
            return;
        }
        let resolved = self.resolve_active_members(parent_block_id, session_id);
        let Some(reconciled) = self.apply_resolved_statuses(parent_block_id, session_id, &resolved) else { return };
        self.announce_reconciled(parent_block_id, session_id, &reconciled);
        self.abandon_finished_dispatches(parent_block_id, session_id, &reconciled.statuses_by_dispatch);
    }

    /// Bounded retry for `reconcile_stale_subagents`'s `None`
    /// (controller-not-yet-registered) case — see that function's doc
    /// comment for why a single 2s attempt was too short in practice.
    /// `retries_remaining` counts DOWN — the loop terminates because each
    /// hop passes a strictly smaller budget, never because of a separate
    /// flag; a block whose controller genuinely never registers (e.g. it
    /// was deleted, or its provider login never resolves) still gets a
    /// bounded, not unbounded, chase.
    ///
    /// Mirrors `trigger_eager_naming`'s `self_ref` upgrade-to-`Arc` pattern
    /// for spawning a task that outlives this sync call; silently no-ops
    /// for a bare `new()` watcher (most unit tests), same "untracked ->
    /// safe no-op" convention as that method.
    /// The parent block is mid-close, or no longer exists — either way no
    /// controller for it will ever register again. A failed store read is
    /// "unknown", not "closed", so it keeps the bounded retry.
    /// SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md (graceful close
    /// removes the controller before the block record).
    pub(super) fn parent_block_closed(&self, parent_block_id: &str) -> bool {
        crate::backend::blockcontroller::is_closing(parent_block_id)
            || matches!(self.mstore.get::<crate::backend::obj::Block>(parent_block_id), Ok(None))
    }

    pub(super) fn retry_reconcile_once(&self, parent_block_id: &str, session_id: &str, retries_remaining: u32) {
        let Some(watcher) = self.self_ref.lock().unwrap().as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        let parent_block_id = parent_block_id.to_string();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(RECONCILE_RETRY_INTERVAL_MS)).await;
            watcher.reconcile_stale_subagents_impl(&parent_block_id, &session_id, retries_remaining);
        });
    }

    /// One batched broadcast per reconciliation pass (not one per
    /// subagent) — a pass can reconcile many subagents at once (e.g. a
    /// workflow with dozens of members whose parent turn just ended), and
    /// the frontend only needs "something changed, reload" (`swarm-model.ts`'s
    /// `scheduleLoadSubagents`), not per-agent granularity. Mirrors the
    /// batch-not-spam precedent `dispatch:activity` already established in
    /// this file for the same reason.
    ///
    /// The frontend `muxEventSubscribe({ eventType: "subagent:abandoned",
    /// ... })` listener lands in the immediately-following PR (Phase B,
    /// #2235 — SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20),
    /// not this one. Broadcasting with no subscriber yet is inert, not
    /// harmful: this PR's actual goal (subagents resolving to `Completed`/
    /// `Abandoned` correctly in the backend, live, not just at reopen) is
    /// already fully achieved without it — the missing piece until #2235
    /// merges is only "push the correction to an already-open Swarm pane
    /// immediately" (an already-open pane still picks up the correction on
    /// its next unrelated reload in the meantime).
    pub(super) fn broadcast_subagents_abandoned(&self, parent_block_id: &str, agent_ids: &[String]) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentAbandoned>(&super::events::SubagentAbandonedEvent {
            parent_block_id: parent_block_id.to_string(),
            agent_ids: agent_ids.to_vec(),
        });
    }

    /// The `Completed` counterpart to `broadcast_subagents_abandoned` above,
    /// for members reconciliation resolved as having actually returned (see
    /// `completion.rs`). Emits the same `subagent:completed` event `jsonl.rs`
    /// already sends when it observes a completion live, so a client needs no
    /// new handling for the reconcile path — it just stops being told
    /// "abandoned" for dispatches that returned.
    ///
    /// Per-agent rather than batched, unlike its `Abandoned` sibling: the
    /// payload carries per-member fields (`totalEvents`, `dispatchId`) that a
    /// batched shape has nowhere to put, and matching `jsonl.rs`'s existing
    /// event exactly is worth more here than saving a broadcast — a reconcile
    /// pass typically resolves a handful of completions, not the dozens the
    /// batching rationale above was written for.
    ///
    /// `parentAgent` is omitted (the live emitter includes it): reconcile is
    /// keyed on the block, and no consumer reads that field.
    pub(super) fn broadcast_subagent_completed(
        &self,
        parent_block_id: &str,
        agent_id: &str,
        dispatch_id: &str,
        total_events: usize,
    ) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentCompleted>(&super::events::SubagentCompletedEvent {
            agent_id: agent_id.to_string(),
            parent_agent: None,
            parent_block_id: parent_block_id.to_string(),
            total_events: total_events,
            dispatch_id: dispatch_id.to_string(),
        });
    }

    /// Process agent-*.jsonl directly in `dir`, plus workflow runs under
    /// `dir/workflows/<wf>/` (member agent files + journal).
    ///
    /// A pane's `subagents/` directory accumulates forever — every Task-tool
    /// and Workflow-tool run this session has ever made leaves its
    /// `agent-*.jsonl` files behind. Because `is_new` (`process_jsonl_change`)
    /// is keyed off the in-memory `sessions` map, which starts empty on every
    /// srv process (restart or fresh start), an unbounded scan here replays
    /// the session's ENTIRE history — full-file read + parse + WS broadcast
    /// per file — on every pane reopen or srv restart. On a heavily-used pane
    /// this is a genuine crash trigger, not just wasted work: a live incident
    /// (see docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md) hit
    /// 1,000+ replayed files across three back-to-back srv restarts in under
    /// 10 seconds, on a machine already near its commit ceiling — the
    /// resulting allocation/broadcast storm is a plausible contributor to
    /// each restart re-crashing before the launcher's 3-strikes budget gave
    /// up and killed the whole app.
    ///
    /// Bound the replay to the `BACKFILL_MAX_FILES` most-recently-modified
    /// agent files (by mtime) — recent activity is what the Swarm pane's
    /// reopen backfill exists to show; a months-old workflow run replaying on
    /// every restart serves no one. Skipped files are still on disk and
    /// still show up via `list_active`'s normal file-count telemetry if
    /// something later reads them directly — this only bounds the *push*
    /// (broadcast) side of a cold backfill.
    fn scan_subagents_dir(&self, parent_agent: &str, parent_block_id: &str, dir: &Path) {
        let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();

        if let Ok(files) = std::fs::read_dir(dir) {
            for file in files.flatten() {
                let path = file.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("agent-") && name.ends_with(".jsonl") {
                        candidates.push((file_mtime(&path), path));
                    }
                }
            }
        }

        let workflows_dir = dir.join("workflows");
        let mut journals: Vec<PathBuf> = Vec::new();
        if let Ok(runs) = std::fs::read_dir(&workflows_dir) {
            for run in runs.flatten() {
                if let Ok(files) = std::fs::read_dir(run.path()) {
                    for file in files.flatten() {
                        let path = file.path();
                        match path.file_name().and_then(|n| n.to_str()) {
                            Some(name)
                                if name.starts_with("agent-") && name.ends_with(".jsonl") =>
                            {
                                candidates.push((file_mtime(&path), path));
                            }
                            // Journals are one small file per run (not one per
                            // member agent) — always process them so
                            // `workflow:updated`/run status stay accurate
                            // regardless of the member-file cap below.
                            Some("journal.jsonl") => journals.push(path),
                            _ => {}
                        }
                    }
                }
            }
        }

        let total = candidates.len();
        candidates.sort_by(|a, b| b.0.cmp(&a.0)); // newest mtime first
        let skipped = total.saturating_sub(BACKFILL_MAX_FILES);
        if skipped > 0 {
            tracing::info!(
                agent = %parent_agent,
                parent_block_id = %parent_block_id,
                dir = %dir.display(),
                total,
                skipped,
                cap = BACKFILL_MAX_FILES,
                "scan_subagents_dir: capping cold-backfill replay to the most recent files"
            );
        }
        for (_, path) in candidates.into_iter().take(BACKFILL_MAX_FILES) {
            // live=false: this is a cold-backfill replay (pane reopen / srv
            // restart), not a genuinely-live spawn — must never trigger eager
            // Haiku naming (see process_jsonl_change's `live` param doc).
            self.process_jsonl_change(parent_agent, parent_block_id, &path, false);
        }
        for path in journals {
            self.process_journal_change(parent_agent, parent_block_id, &path);
        }
    }
}
