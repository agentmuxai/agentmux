// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session/directory backfill scanning and stale-subagent reconciliation:
//! `scan_session_subagents` (pane-reopen backfill entry point),
//! `reconcile_stale_subagents` (Active -> Abandoned correction once a
//! parent's turn is confirmed idle), `broadcast_subagents_abandoned`, and
//! `scan_subagents_dir` (the capped cold-backfill file walk).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::parse::file_mtime;
use super::types::*;
use super::SubagentWatcher;
use crate::backend::eventbus::{WSEventType, WS_EVENT_RPC};

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
                // production EnvFilter (agentmux-srv/src/main.rs, "agentmuxsrv=
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
    /// §2, mechanism 4). Fixed by treating `None` as "retry once, don't
    /// give up silently" (`retry_reconcile_once`) instead of a terminal
    /// no-op — still conservative (a genuine `Some(true)` never retries;
    /// only the single ambiguous case does, and only once).
    /// Called from two places as of SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_
    /// RETIRE_2026_07_20 Phase A: `scan_session_subagents` (reopen/backfill,
    /// unchanged) and `blockcontroller::persistent`'s turn-end hook (live —
    /// closes docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md
    /// Open Question 1, which deliberately deferred real-time wiring).
    /// `pub(crate)` so the live call site (a different module) can reach it.
    /// Thin entry point — always allows the one `None`-case retry described
    /// above. The retry itself calls `reconcile_stale_subagents_impl`
    /// directly with `allow_retry: false`, so a still-`None` second attempt
    /// gives up rather than chaining into an unbounded retry loop.
    pub(crate) fn reconcile_stale_subagents(&self, parent_block_id: &str, session_id: &str) {
        self.reconcile_stale_subagents_impl(parent_block_id, session_id, true);
    }

    // pub(super), not private — `tests.rs` (a sibling module, not a
    // descendant of `scan`) needs to call this directly with
    // `allow_retry: false` to test the exhausted-retry path without
    // depending on a real spawned watcher's tokio::spawn actually firing.
    pub(super) fn reconcile_stale_subagents_impl(&self, parent_block_id: &str, session_id: &str, allow_retry: bool) {
        let turn_active = crate::backend::blockcontroller::get_block_controller_status(parent_block_id)
            .map(|s| s.turn_active);
        match turn_active {
            Some(true) => {
                tracing::info!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    "reconcile_stale_subagents: parent turn active — nothing to reconcile"
                );
                return;
            }
            None if allow_retry => {
                tracing::info!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    "reconcile_stale_subagents: controller not yet registered — retrying once, not skipping silently"
                );
                self.retry_reconcile_once(parent_block_id, session_id);
                return;
            }
            None => {
                tracing::info!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    "reconcile_stale_subagents: controller still not registered after retry — giving up (bounded to one retry)"
                );
                return;
            }
            Some(false) => {} // confirmed idle — proceed below
        }

        // Scoped so the `sessions` lock is released before broadcasting (or
        // touching `self.dispatches`, a separate mutex — never held at the
        // same time as `sessions` anywhere in this function, to avoid any
        // lock-ordering deadlock risk against other call sites) below —
        // `broadcast_subagents_abandoned` doesn't need it, and this call
        // site can now run live (Phase A), not just at reopen, so holding
        // the lock any longer than the mutation itself is unnecessary
        // contention against the live filesystem watcher.
        //
        // Alongside `reconciled_agent_ids`, also snapshot every member's
        // (dispatch_id, status) for this block — not just the ones just
        // reconciled — so the Workflow-dispatch aggregation pass below has
        // the full picture per dispatch_id, not just this pass's deltas.
        let (reconciled_agent_ids, member_statuses_by_dispatch): (
            Vec<String>,
            HashMap<String, Vec<SubAgentStatus>>,
        ) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(session) = sessions.get_mut(session_id) else { return };
            // A session's subagent map can hold entries from a DIFFERENT block —
            // the watcher dedupes purely by agent_id (see watch_agent's doc
            // comment), so two blocks that both ran the same underlying Claude
            // session id (e.g. one reattached to a session another block also
            // touched) can have subagents from both mixed into one
            // `SessionWatch`. We only have a confirmed-idle read for THIS one
            // `parent_block_id` — a sibling block's subagent could easily still
            // be genuinely active, so only reconcile entries this block itself
            // owns (mirrors unwatch_agent's own parent-scoped filter). Reagent
            // P1 on PR #2131.
            let mut reconciled_agent_ids = Vec::new();
            let mut member_statuses_by_dispatch: HashMap<String, Vec<SubAgentStatus>> =
                std::collections::HashMap::new();
            for state in session.subagents.values_mut() {
                if state.info.parent_block_id != parent_block_id {
                    continue;
                }
                if state.info.status == SubAgentStatus::Active {
                    state.info.status = SubAgentStatus::Abandoned;
                    reconciled_agent_ids.push(state.info.agent_id.clone());
                    // Every field a NAME-based grouping/dedup bug needs to
                    // reconstruct offline: which subagent, which dispatch,
                    // which display_name it had already resolved (grouping is
                    // keyed on this), and which block/session it's bound to.
                    tracing::info!(
                        agent_id = %state.info.agent_id,
                        parent_block_id = %parent_block_id,
                        session_id = %session_id,
                        dispatch_id = %state.info.dispatch_id,
                        display_name = ?state.info.display_name,
                        slug = %state.info.slug,
                        "subagent reconciled: active -> abandoned (parent turn ended)"
                    );
                }
                member_statuses_by_dispatch
                    .entry(state.info.dispatch_id.clone())
                    .or_default()
                    .push(state.info.status.clone());
            }
            (reconciled_agent_ids, member_statuses_by_dispatch)
        };
        if !reconciled_agent_ids.is_empty() {
            tracing::info!(
                parent_block_id = %parent_block_id,
                session_id = %session_id,
                reconciled = reconciled_agent_ids.len(),
                "reconcile_stale_subagents: pass complete"
            );
            self.broadcast_subagents_abandoned(parent_block_id, &reconciled_agent_ids);
        }

        // Propagate to the owning Workflow-kind `AgentDispatch` aggregate —
        // a Solo dispatch needs no separate step (its status is synthesized
        // directly from its one member's status, see `solo_dispatch`).
        // SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2:
        // Abandoned iff every member is Completed|Abandoned and at least one
        // is Abandoned — otherwise leave the existing counts-based Running/
        // Completed status (set by `refresh_dispatch_status`) alone.
        let abandoned_dispatch_infos: Vec<AgentDispatch> = {
            let mut dispatches = self.dispatches.lock().unwrap();
            let mut updated = Vec::new();
            for (dispatch_id, statuses) in &member_statuses_by_dispatch {
                if dispatch_id.starts_with("solo:") {
                    continue;
                }
                let Some(state) = dispatches.get_mut(dispatch_id) else { continue };
                // reagent P2 on PR #2677: `statuses` only reflects members
                // currently visible in `session.subagents` — a member whose
                // JSONL file the filesystem watcher hasn't picked up yet
                // (an async notify/debounce lag racing this exact
                // reconciliation pass) is invisible here, so `all_done`
                // could be true against an INCOMPLETE member set. Cross-
                // check against the dispatch's own authoritative
                // `member_count` (tracked separately via journal_started/
                // member_files) — only trust `all_done` when we've actually
                // seen status for every member the dispatch itself believes
                // exist. Under-counting here is safe (skip this round, the
                // next new-evidence event or reconciliation pass reruns
                // this check with a fuller picture) — over-counting would
                // risk abandoning a dispatch with a member reconciliation
                // hasn't even observed yet.
                if statuses.len() < state.info.member_count {
                    continue;
                }
                let all_done = statuses
                    .iter()
                    .all(|s| matches!(s, SubAgentStatus::Completed | SubAgentStatus::Abandoned));
                let any_abandoned = statuses.iter().any(|s| *s == SubAgentStatus::Abandoned);
                if !(all_done && any_abandoned) {
                    continue;
                }
                if state.info.status != DispatchStatus::Abandoned {
                    state.info.status = DispatchStatus::Abandoned;
                    tracing::info!(
                        dispatch_id = %dispatch_id,
                        parent_block_id = %parent_block_id,
                        session_id = %session_id,
                        member_count = statuses.len(),
                        "dispatch reconciled: -> abandoned (all members done, at least one abandoned)"
                    );
                    updated.push(state.info.clone());
                }
            }
            updated
        };
        for info in &abandoned_dispatch_infos {
            self.broadcast_dispatch_updated(info);
        }
    }

    /// Bounded one-shot retry for `reconcile_stale_subagents`'s `None`
    /// (controller-not-yet-registered) case — see that function's doc
    /// comment. Exactly one retry, not a loop: if the controller genuinely
    /// never registers (e.g. the block was deleted in the meantime), a
    /// single delayed re-check is enough to stop treating "unknown" as a
    /// permanent no-op without risking an unbounded retry chain chasing a
    /// block that's never coming back. 2s is long enough to clear the
    /// observed registration race without meaningfully delaying the
    /// correction a user would notice.
    ///
    /// Mirrors `trigger_eager_naming`'s `self_ref` upgrade-to-`Arc` pattern
    /// for spawning a task that outlives this sync call; silently no-ops
    /// for a bare `new()` watcher (most unit tests), same "untracked ->
    /// safe no-op" convention as that method.
    fn retry_reconcile_once(&self, parent_block_id: &str, session_id: &str) {
        let Some(watcher) = self.self_ref.lock().unwrap().as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        let parent_block_id = parent_block_id.to_string();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            // allow_retry: false — this IS the one retry; a second `None`
            // here gives up rather than spawning another.
            watcher.reconcile_stale_subagents_impl(&parent_block_id, &session_id, false);
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
    /// The frontend `waveEventSubscribe({ eventType: "subagent:abandoned",
    /// ... })` listener lands in the immediately-following PR (Phase B,
    /// #2235 — SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20),
    /// not this one. Broadcasting with no subscriber yet is inert, not
    /// harmful: this PR's actual goal (subagents resolving to `Completed`/
    /// `Abandoned` correctly in the backend, live, not just at reopen) is
    /// already fully achieved without it — the missing piece until #2235
    /// merges is only "push the correction to an already-open Swarm pane
    /// immediately" (an already-open pane still picks up the correction on
    /// its next unrelated reload in the meantime).
    fn broadcast_subagents_abandoned(&self, parent_block_id: &str, agent_ids: &[String]) {
        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(json!({
                "command": "eventrecv",
                "data": {
                    "event": "subagent:abandoned",
                    "data": {
                        "parentBlockId": parent_block_id,
                        "agentIds": agent_ids,
                    }
                }
            })),
        };
        self.event_bus.broadcast_event(&event);
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
