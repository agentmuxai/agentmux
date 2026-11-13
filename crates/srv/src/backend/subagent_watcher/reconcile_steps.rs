// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `reconcile_stale_subagents_impl` (scan.rs), one function each:
//! the idle gate, resolving what each `Active` member became, applying that under
//! the `sessions` lock, announcing it, and propagating it to Workflow dispatches.
//!
//! Lock order: `sessions` and `dispatches` are never held at the same time, and
//! neither is held across file IO or a broadcast.

use std::collections::HashMap;
use std::path::PathBuf;

use super::completion;
use super::types::*;
use super::SubagentWatcher;

/// What one reconcile pass changed, for the announce and dispatch steps.
pub(super) struct Reconciled {
    /// Members that went `Active` -> `Abandoned`.
    pub(super) abandoned_ids: Vec<String>,
    /// Members that went `Active` -> `Completed`: (agent_id, dispatch_id, event_count).
    pub(super) completed: Vec<(String, String, usize)>,
    /// Every member of this block, by dispatch, after the pass (not just the changed ones).
    pub(super) statuses_by_dispatch: HashMap<String, Vec<SubAgentStatus>>,
}

impl SubagentWatcher {
    /// True when the parent's turn is confirmed idle and the pass should go on. Otherwise
    /// it has said why not (and queued the retry, when one is due) and the pass is over.
    pub(super) fn parent_turn_is_confirmed_idle(
        &self,
        parent_block_id: &str,
        session_id: &str,
        retries_remaining: u32,
    ) -> bool {
        let turn_active = crate::backend::blockcontroller::get_block_controller_status(parent_block_id)
            .map(|s| s.turn_active);
        match turn_active {
            Some(true) => {
                tracing::info!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    "reconcile_stale_subagents: parent turn active — nothing to reconcile"
                );
                false
            }
            // The pane was closed: its controller left the registry on
            // purpose and will never register again. Retrying (~15s of
            // "not yet registered" lines after every mid-turn close) would
            // only chase a block that's gone.
            None if self.parent_block_closed(parent_block_id) => {
                tracing::debug!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    "reconcile_stale_subagents: parent block closed — nothing to reconcile"
                );
                false
            }
            None if retries_remaining > 0 => {
                tracing::info!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    retries_remaining,
                    "reconcile_stale_subagents: controller not yet registered — retrying, not giving up silently"
                );
                self.retry_reconcile_once(parent_block_id, session_id, retries_remaining - 1);
                false
            }
            None => {
                tracing::info!(
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    "reconcile_stale_subagents: controller still not registered after all retries — giving up (bounded)"
                );
                false
            }
            Some(false) => true, // confirmed idle — proceed
        }
    }

    /// What each of this block's still-`Active` members became: `Completed` when the
    /// PARENT recorded a `tool_result` for its `tool_use_id`, else `Abandoned`.
    ///
    /// AgentMux transcripts carry no `"type":"result"` line, so a missing one proves
    /// nothing (`completion.rs`, docs/reports/REPORT_SUBAGENT_COMPLETION_NEVER_DETECTED_2026_09_05.md).
    /// Resolved before taking the `sessions` lock and in two phases, because a parent
    /// transcript is routinely tens of MB: read the candidates under a short lock, do the
    /// file IO unlocked, apply afterwards (which re-checks `Active`).
    pub(super) fn resolve_active_members(
        &self,
        parent_block_id: &str,
        session_id: &str,
    ) -> HashMap<String, SubAgentStatus> {
        let candidates: Vec<(String, PathBuf)> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .get(session_id)
                .map(|session| {
                    session
                        .subagents
                        .values()
                        .filter(|s| {
                            s.info.parent_block_id == parent_block_id
                                && s.info.status == SubAgentStatus::Active
                        })
                        .map(|s| {
                            (s.info.agent_id.clone(), PathBuf::from(&s.info.jsonl_path))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        // Each candidate resolves its OWN parent transcript, with reads cached per
        // distinct path: Workflow-tool members sit one level deeper
        // (`subagents/workflows/<run-id>/`), and `candidates` comes from a HashMap in no
        // fixed order, so sharing one member's resolution would abandon Solo members
        // non-deterministically (#3007).
        let mut results_by_parent: HashMap<PathBuf, HashMap<String, bool>> = HashMap::new();
        candidates
            .into_iter()
            .map(|(agent_id, path)| {
                let status = match completion::parent_transcript_for(&path) {
                    Some(parent) => {
                        let results = results_by_parent
                            .entry(parent.clone())
                            .or_insert_with(|| completion::parent_tool_results_at(&parent));
                        let tool_use_id = completion::tool_use_id_for(&path);
                        completion::terminal_status(tool_use_id.as_deref(), results)
                    }
                    // No `subagents` ancestor — unrecognised layout.
                    // Fall back to the historical inference.
                    None => SubAgentStatus::Abandoned,
                };
                (agent_id, status)
            })
            .collect()
    }

    /// Write the resolved statuses onto this block's `Active` members, under the
    /// `sessions` lock. `None` when the session is not tracked.
    ///
    /// A session's subagent map can hold entries from a DIFFERENT block (the watcher
    /// dedupes by agent_id alone), and only this one `parent_block_id` was read as idle,
    /// so a sibling block's subagent is left alone (#2131).
    pub(super) fn apply_resolved_statuses(
        &self,
        parent_block_id: &str,
        session_id: &str,
        resolved_statuses: &HashMap<String, SubAgentStatus>,
    ) -> Option<Reconciled> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id)?;
        let mut reconciled_agent_ids = Vec::new();
        let mut completed_members: Vec<(String, String, usize)> = Vec::new();
        let mut member_statuses_by_dispatch: HashMap<String, Vec<SubAgentStatus>> =
            std::collections::HashMap::new();
        for state in session.subagents.values_mut() {
            if state.info.parent_block_id != parent_block_id {
                continue;
            }
            if state.info.status == SubAgentStatus::Active {
                // Absent a resolution (no sidecar, unreadable parent, an
                // entry that appeared after the snapshot above) fall back
                // to the historical `Abandoned` — never optimistically
                // complete something we could not correlate.
                let resolved = resolved_statuses
                    .get(&state.info.agent_id)
                    .cloned()
                    .unwrap_or(SubAgentStatus::Abandoned);
                state.info.status = resolved.clone();
                if resolved == SubAgentStatus::Completed {
                    tracing::info!(
                        agent_id = %state.info.agent_id,
                        parent_block_id = %parent_block_id,
                        session_id = %session_id,
                        dispatch_id = %state.info.dispatch_id,
                        "subagent reconciled: active -> completed (parent recorded a tool_result)"
                    );
                    completed_members.push((
                        state.info.agent_id.clone(),
                        state.info.dispatch_id.clone(),
                        state.info.event_count,
                    ));
                    member_statuses_by_dispatch
                        .entry(state.info.dispatch_id.clone())
                        .or_default()
                        .push(state.info.status.clone());
                    continue;
                }
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
        Some(Reconciled {
            abandoned_ids: reconciled_agent_ids,
            completed: completed_members,
            statuses_by_dispatch: member_statuses_by_dispatch,
        })
    }

    /// Push the corrections the same way the abandoned ones are pushed — an
    /// already-open Swarm pane otherwise keeps showing these as running until some
    /// unrelated reload happens to refresh it.
    pub(super) fn announce_reconciled(&self, parent_block_id: &str, session_id: &str, reconciled: &Reconciled) {
        for (agent_id, dispatch_id, total_events) in &reconciled.completed {
            self.broadcast_subagent_completed(
                parent_block_id,
                agent_id,
                dispatch_id,
                *total_events,
            );
        }
        if !reconciled.abandoned_ids.is_empty() {
            tracing::info!(
                parent_block_id = %parent_block_id,
                session_id = %session_id,
                reconciled = reconciled.abandoned_ids.len(),
                "reconcile_stale_subagents: pass complete"
            );
            self.broadcast_subagents_abandoned(parent_block_id, &reconciled.abandoned_ids);
        }
    }

    /// Propagate to the owning Workflow-kind `AgentDispatch` aggregate — a Solo dispatch
    /// needs no step (its status is synthesized from its one member's status).
    /// SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2: Abandoned iff
    /// every member is Completed|Abandoned and at least one is Abandoned — otherwise the
    /// counts-based Running/Completed status (`refresh_dispatch_status`) is left alone.
    pub(super) fn abandon_finished_dispatches(
        &self,
        parent_block_id: &str,
        session_id: &str,
        member_statuses_by_dispatch: &HashMap<String, Vec<SubAgentStatus>>,
    ) {
        let abandoned_dispatch_infos: Vec<AgentDispatch> = {
            let mut dispatches = self.dispatches.lock().unwrap();
            let mut updated = Vec::new();
            for (dispatch_id, statuses) in member_statuses_by_dispatch {
                if dispatch_id.starts_with("solo:") {
                    continue;
                }
                let Some(state) = dispatches.get_mut(dispatch_id) else { continue };
                // `statuses` only covers members visible in `session.subagents`; one whose
                // JSONL the filesystem watcher has not picked up yet (notify/debounce lag)
                // is missing, so `all_done` could hold for an INCOMPLETE set. Trust it only
                // once every member the dispatch itself counts has a status. Skipping is
                // safe (the next pass rechecks); abandoning early is not (#2677).
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
}
