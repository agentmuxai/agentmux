// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The JSONL/journal file-change state machine: `process_jsonl_change` (the
//! order of its steps, which live in `jsonl_change_steps.rs`) and
//! its dispatch-membership/coalescing plumbing — `flush_pending_dispatch_
//! activity`, `update_dispatch_membership`, `queue_dispatch_updated`,
//! `process_journal_change`, `dispatch_entry`, `refresh_dispatch_info`,
//! `refresh_dispatch_status`, `broadcast_dispatch_updated`.

use std::collections::HashMap;
use std::path::Path;

use super::parse::*;
use super::types::*;
use super::SubagentWatcher;

#[path = "jsonl_change_steps.rs"]
mod steps;

impl SubagentWatcher {
    /// Process a changed/new JSONL subagent file. Reads new lines, updates state,
    /// and broadcasts events via EventBus.
    ///
    /// `live`: `true` only when this call originates from the filesystem
    /// watcher observing a real, in-the-moment change; `false` for a
    /// cold-backfill replay (`scan_subagents_dir`, capped at
    /// `BACKFILL_MAX_FILES`, run on pane reopen / srv restart). Eager
    /// Haiku-naming (see `announce_spawn`) is gated on `live` —
    /// without this, every restart of a long-lived session would re-fire a
    /// naming call for every dispatch replayed from history, repeating the
    /// broadcast-storm incident class in
    /// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md, just for
    /// Haiku spend instead of WS events.
    pub(super) fn process_jsonl_change(&self, parent_agent: &str, parent_block_id: &str, jsonl_path: &Path, live: bool) {
        let Some(change) = steps::Change::of(parent_agent, parent_block_id, jsonl_path, live) else {
            return;
        };
        let Some((new_events, new_offset, meta)) = self.read_new_lines(&change, jsonl_path) else {
            return;
        };
        // Mutex released here — broadcast outside the lock
        let (is_new, info_snapshot, completed) = match self.record_change(&change, jsonl_path, &new_events, new_offset, meta) {
            steps::Recorded::Quiet { id_learned, name_learned } => {
                self.announce_learned(&change, id_learned, name_learned);
                return;
            }
            steps::Recorded::Changed { is_new, info_snapshot, completed, id_learned, name_learned } => {
                self.announce_learned(&change, id_learned, name_learned);
                (is_new, info_snapshot, completed)
            }
        };

        if is_new {
            self.announce_spawn(&change, &info_snapshot);
        }
        if !new_events.is_empty() {
            self.announce_activity(&change, &new_events, &info_snapshot);
        }
        if completed {
            self.announce_completion(&change, &info_snapshot);
        }
        self.announce_dispatch(&change, &info_snapshot, is_new, completed);
    }

    /// Flush every dispatch with buffered activity as one coalesced
    /// `dispatch:activity` broadcast (SPEC §7). Called on
    /// `DISPATCH_ACTIVITY_FLUSH_INTERVAL` by the background task
    /// `spawn` starts. A quiet dispatch (nothing buffered) costs nothing —
    /// the map only ever holds entries for dispatches with real pending
    /// activity, drained on every flush.
    pub(super) fn flush_pending_dispatch_activity(&self) {
        let batch: HashMap<String, PendingDispatchActivity> = {
            let mut pending = self.pending_activity.lock().unwrap();
            std::mem::take(&mut *pending)
        };
        for (dispatch_id, pending) in batch {
            for spawn in &pending.spawned {
                self.broadcast_member_spawned(
                    spawn,
                    &pending.parent_agent,
                    &pending.parent_block_id,
                    &pending.session_id,
                    &dispatch_id,
                );
            }

            if !pending.members.is_empty() {
                let new_events: usize = pending.members.iter().map(|(_, evs)| evs.len()).sum();
                self.event_bus.broadcast_typed::<crate::backend::mps_events::DispatchActivity>(&super::events::DispatchActivityEvent {
                    dispatch_id: dispatch_id.to_string(),
                    parent_agent: pending.parent_agent.to_string(),
                    parent_block_id: pending.parent_block_id.to_string(),
                    session_id: pending.session_id.to_string(),
                    new_events: new_events,
                    members: pending.members.iter().map(|(agent_id, events)| super::events::DispatchMemberEvents { agent_id: agent_id.clone(), events: events.clone() }).collect(),
                });
            }

            for done in &pending.completed {
                self.broadcast_member_completed(
                    done,
                    &pending.parent_agent,
                    &pending.parent_block_id,
                    &pending.session_id,
                    &dispatch_id,
                );
            }

            if let Some(info) = &pending.latest_info {
                self.broadcast_dispatch_updated(info);
            }
        }
    }

    /// Broadcast `subagent:spawned` for one member, immediately for a solo
    /// dispatch or from the coalesced flush for a workflow member.
    fn broadcast_member_spawned(
        &self,
        spawn: &PendingSpawn,
        parent_agent: &str,
        parent_block_id: &str,
        session_id: &str,
        dispatch_id: &str,
    ) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentSpawned>(&super::events::SubagentSpawnedEvent {
            agent_id: spawn.agent_id.to_string(),
            slug: spawn.slug.to_string(),
            parent_agent: parent_agent.to_string(),
            parent_block_id: parent_block_id.to_string(),
            session_id: session_id.to_string(),
            model: spawn.model.clone(),
            dispatch_id: dispatch_id.to_string(),
        });
        tracing::info!(
            agent_id = %spawn.agent_id,
            slug = %spawn.slug,
            parent = %parent_agent,
            parent_block_id = %parent_block_id,
            session_id = %session_id,
            dispatch_id = %dispatch_id,
            "subagent spawned"
        );
    }

    /// Broadcast `subagent:completed` for one member, immediately for a solo
    /// dispatch or from the coalesced flush for a workflow member.
    fn broadcast_member_completed(
        &self,
        done: &PendingCompletion,
        parent_agent: &str,
        parent_block_id: &str,
        session_id: &str,
        dispatch_id: &str,
    ) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentCompleted>(&super::events::SubagentCompletedEvent {
            agent_id: done.agent_id.to_string(),
            parent_agent: Some(parent_agent.to_string()),
            parent_block_id: parent_block_id.to_string(),
            total_events: done.total_events,
            dispatch_id: dispatch_id.to_string(),
        });
        tracing::info!(
            agent_id = %done.agent_id,
            total_events = done.total_events,
            parent_block_id = %parent_block_id,
            session_id = %session_id,
            dispatch_id = %dispatch_id,
            "subagent completed"
        );
    }

    /// Fold a member subagent's lifecycle into its dispatch aggregate and
    /// broadcast `dispatch:updated` — but only when membership actually
    /// changed (a spawn or a completion). `process_jsonl_change` calls this
    /// unconditionally for every workflow member, including plain
    /// text/tool_use/tool_result activity ticks that carry neither flag; a
    /// dispatch with several active members would otherwise broadcast a WS
    /// event on every one of those ticks even though `memberCount`/
    /// `membersDone` never moved. Mirrors the `has_new_records` gate in
    /// `process_journal_change`.
    fn update_dispatch_membership(
        &self,
        workflow_id: &str,
        parent_agent: &str,
        parent_block_id: &str,
        session_id: &str,
        member_spawned: bool,
        member_completed: bool,
    ) {
        if !member_spawned && !member_completed {
            return;
        }

        let info = {
            let mut dispatches = self.dispatches.lock().unwrap();
            let state = Self::dispatch_entry(
                &mut dispatches,
                workflow_id,
                parent_agent,
                parent_block_id,
                session_id,
            );
            if member_spawned {
                state.member_files += 1;
            }
            if member_completed {
                state.members_completed += 1;
            }
            Self::refresh_dispatch_info(state);
            state.info.clone()
        };
        self.queue_dispatch_updated(workflow_id, parent_agent, parent_block_id, session_id, info);
    }

    /// Queue an `AgentDispatch` snapshot for the next coalesced flush instead
    /// of broadcasting `dispatch:updated` immediately — every caller here is
    /// a Workflow-kind dispatch (Solo dispatches broadcast directly via
    /// `broadcast_dispatch_updated`, see `process_jsonl_change`'s solo path).
    fn queue_dispatch_updated(
        &self,
        dispatch_id: &str,
        parent_agent: &str,
        parent_block_id: &str,
        session_id: &str,
        info: AgentDispatch,
    ) {
        let mut pending = self.pending_activity.lock().unwrap();
        let entry = pending
            .entry(dispatch_id.to_string())
            .or_insert_with(|| PendingDispatchActivity::new(parent_agent, parent_block_id, session_id));
        entry.latest_info = Some(info);
    }

    /// Process a changed workflow journal (subagents/workflows/<wf>/journal.jsonl):
    /// tally new `started`/`result` records and broadcast `dispatch:updated`.
    pub(super) fn process_journal_change(
        &self,
        parent_agent: &str,
        parent_block_id: &str,
        journal_path: &Path,
    ) {
        let workflow_id = match parse_workflow_id(journal_path) {
            Some(id) => id,
            None => return,
        };
        let session_id = derive_session_id(journal_path);

        // Read the current offset before locking (file I/O outside the lock).
        let offset = {
            let dispatches = self.dispatches.lock().unwrap();
            dispatches
                .get(&workflow_id)
                .map(|w| w.journal_offset)
                .unwrap_or(0)
        };

        let (started, results, new_offset) = match read_journal_counts(journal_path, offset) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    workflow_id = %workflow_id,
                    error = %e,
                    "failed to read workflow journal"
                );
                return;
            }
        };
        // Nothing new landed (no complete lines since `offset`) — nothing to
        // persist or broadcast.
        if new_offset == offset {
            return;
        }

        let has_new_records = started > 0 || results > 0;
        let info = {
            let mut dispatches = self.dispatches.lock().unwrap();
            let state = Self::dispatch_entry(
                &mut dispatches,
                &workflow_id,
                parent_agent,
                parent_block_id,
                &session_id,
            );
            // Always persist the advanced offset, even when the newly-read
            // lines didn't contain a started/result record — otherwise those
            // bytes get rescanned on every subsequent journal change until a
            // matching record eventually appears elsewhere in the file.
            state.journal_offset = new_offset;
            if has_new_records {
                state.journal_started += started;
                state.journal_results += results;
                Self::refresh_dispatch_info(state);
            }
            state.info.clone()
        };
        // Only queue when the counters actually moved — an offset-only
        // advance (non-started/result lines) has no observable effect on
        // AgentDispatch, so a broadcast would just be noise.
        if has_new_records {
            self.queue_dispatch_updated(&workflow_id, parent_agent, parent_block_id, &session_id, info);
        }
    }

    fn dispatch_entry<'a>(
        dispatches: &'a mut HashMap<String, DispatchState>,
        workflow_id: &str,
        parent_agent: &str,
        parent_block_id: &str,
        session_id: &str,
    ) -> &'a mut DispatchState {
        dispatches
            .entry(workflow_id.to_string())
            .or_insert_with(|| DispatchState {
                info: AgentDispatch {
                    dispatch_id: workflow_id.to_string(),
                    kind: DispatchKind::Workflow,
                    parent_agent: parent_agent.to_string(),
                    parent_block_id: parent_block_id.to_string(),
                    session_id: session_id.to_string(),
                    member_count: 0,
                    members_done: 0,
                    status: DispatchStatus::Running,
                    last_event_at: now_millis(),
                    dispatch_name: None,
                },
                journal_offset: 0,
                journal_started: 0,
                journal_results: 0,
                member_files: 0,
                members_completed: 0,
            })
    }

    /// Recompute public counters from raw journal/member counters. Either
    /// source can lag the other (journal writes vs member file creation), so
    /// take the max of each pair.
    fn refresh_dispatch_info(state: &mut DispatchState) {
        state.info.member_count = state.journal_started.max(state.member_files);
        state.info.members_done = state.journal_results.max(state.members_completed);
        state.info.last_event_at = now_millis();
        // Called only in response to genuinely NEW member evidence (a spawn
        // or a completion just landed) — always allowed to recompute, even
        // overriding a prior Abandoned, mirroring the existing SubAgent-level
        // precedent that a late-arriving Result always wins
        // (`reconcile_stale_subagents_then_late_result_line_ends_completed_
        // not_stuck_abandoned`). Codex P2 on PR #2677: an earlier version of
        // this guard was unconditional (also applied here), permanently
        // stranding a dispatch at Abandoned even once every member had
        // actually finished. See `refresh_dispatch_status`'s own doc comment
        // for why the READ-ONLY path (list_dispatches) still needs the guard.
        Self::recompute_dispatch_status(state);
    }

    /// Counts-complete + 60s quiet ⇒ Completed. There is no timer: the flip
    /// happens lazily at the next event or ListDispatches read. `started ==
    /// results` alone is not terminal — it also holds between phases of a
    /// still-running workflow, hence the quiet window.
    pub(super) fn recompute_dispatch_status(state: &mut DispatchState) {
        let counts_complete = state.info.member_count > 0
            && state.info.members_done >= state.info.member_count;
        let quiet = now_millis().saturating_sub(state.info.last_event_at) > 60_000;
        state.info.status = if counts_complete && quiet {
            DispatchStatus::Completed
        } else {
            DispatchStatus::Running
        };
    }

    /// Read-only variant for `list_dispatches()` — `Abandoned` is terminal
    /// and never recomputed here, since this call has no new evidence
    /// (unlike `refresh_dispatch_info`'s call to `recompute_dispatch_status`
    /// above, triggered by a genuinely new member event). `reconcile_stale_
    /// subagents` is the ONLY writer of `Abandoned`
    /// (SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2),
    /// and `list_dispatches()` calls this on every single read (not just on
    /// new events) — without this guard, the very next `ListDispatches` RPC
    /// after a reconciliation pass would silently overwrite Abandoned back
    /// to Running/Completed based on counts alone, discarding the
    /// reconciliation before any client ever observed it.
    pub(super) fn refresh_dispatch_status(state: &mut DispatchState) {
        if state.info.status == DispatchStatus::Abandoned {
            return;
        }
        Self::recompute_dispatch_status(state);
    }

    /// `subagent:named`: the subagent's display name is set (from the parent's
    /// description, or by Haiku). The dock and the Swarm patch their row from it.
    pub(super) fn broadcast_subagent_named(&self, agent_id: &str, display_name: &str) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentNamed>(&super::events::SubagentNamedEvent {
            agent_id: agent_id.to_string(),
            display_name: display_name.to_string(),
        });
    }

    /// A known subagent's fields changed without a new spawn — today, only its
    /// `tool_use_id` arriving late from the sidecar. Swarm reloads the list on
    /// it so the subagent's background tasks move under its row. Deliberately
    /// NOT a re-sent `subagent:spawned`: the dock's sources add rows on that.
    pub(super) fn broadcast_subagent_updated(&self, agent_id: &str, parent_block_id: &str) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentUpdated>(&super::events::SubagentUpdatedEvent {
            agent_id: agent_id.to_string(),
            parent_block_id: parent_block_id.to_string(),
        });
    }

    pub(super) fn broadcast_dispatch_updated(&self, info: &AgentDispatch) {
        self.event_bus.broadcast_typed::<crate::backend::mps_events::DispatchUpdated>(&super::events::DispatchUpdatedEvent {
            dispatch_id: info.dispatch_id.to_string(),
            kind: info.kind.clone(),
            parent_agent: info.parent_agent.to_string(),
            parent_block_id: info.parent_block_id.to_string(),
            session_id: info.session_id.to_string(),
            member_count: info.member_count,
            members_done: info.members_done,
            status: info.status.clone(),
            dispatch_name: info.dispatch_name.clone(),
        });
    }
}

#[cfg(test)]
#[path = "jsonl_change_tests.rs"]
mod change_tests;
