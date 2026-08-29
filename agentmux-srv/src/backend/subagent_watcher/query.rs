// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Query/naming API: `list_active`/`list_dispatches`/`solo_dispatch`/
//! `get_history`/`get_info`/`set_display_name`/`set_dispatch_name`/
//! `trigger_eager_naming` — the read-mostly surface RPC dispatch calls into,
//! plus the eager Haiku-naming trigger fired from `jsonl::process_jsonl_change`.

use serde_json::json;

use super::types::*;
use super::SubagentWatcher;
use crate::backend::eventbus::{WSEventType, WS_EVENT_RPC};

impl SubagentWatcher {
    /// List all subagents across all sessions (sync — safe to call from RPC dispatch).
    pub fn list_active(&self) -> Vec<SubAgent> {
        let sessions = self.sessions.lock().unwrap();
        let mut result = Vec::new();
        for session in sessions.values() {
            for state in session.subagents.values() {
                result.push(state.info.clone());
            }
        }
        result.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
        result
    }

    /// List all tracked dispatches — both kinds (sync — safe to call from
    /// RPC dispatch). Workflow-kind dispatches come from tracked
    /// `DispatchState`; Solo-kind dispatches have no persistent state of
    /// their own and are synthesized 1:1 from every loose `SubAgent` (SPEC
    /// §5/§8) — this is what gives every solo Task-tool call a real,
    /// backend-issued container it never had before.
    pub fn list_dispatches(&self) -> Vec<AgentDispatch> {
        let mut result: Vec<AgentDispatch> = {
            let mut dispatches = self.dispatches.lock().unwrap();
            dispatches
                .values_mut()
                .map(|state| {
                    Self::refresh_dispatch_status(state);
                    state.info.clone()
                })
                .collect()
        };
        let sessions = self.sessions.lock().unwrap();
        for session in sessions.values() {
            for state in session.subagents.values() {
                if let Some(solo) = Self::solo_dispatch(&state.info) {
                    result.push(solo);
                }
            }
        }
        result.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
        result
    }

    /// Synthesize the `AgentDispatch` for a loose (non-workflow) `SubAgent`.
    /// `None` for a workflow member — those are represented by their
    /// tracked `DispatchState` instead, not synthesized per-call.
    pub(super) fn solo_dispatch(sub: &SubAgent) -> Option<AgentDispatch> {
        if !sub.dispatch_id.starts_with("solo:") {
            return None;
        }
        // A solo dispatch's status mirrors its one member's directly — the
        // member IS the dispatch, no separate aggregation to compute. Kept
        // as three distinct arms (not `done as usize` collapsing Completed/
        // Abandoned together) so a dead dispatch reads as Abandoned, not
        // ambiguously "Completed," in the UI. See
        // docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2.
        let (status, done) = match sub.status {
            SubAgentStatus::Completed => (DispatchStatus::Completed, true),
            SubAgentStatus::Abandoned => (DispatchStatus::Abandoned, true),
            SubAgentStatus::Active => (DispatchStatus::Running, false),
        };
        Some(AgentDispatch {
            dispatch_id: sub.dispatch_id.clone(),
            kind: DispatchKind::Solo,
            parent_agent: sub.parent_agent.clone(),
            parent_block_id: sub.parent_block_id.clone(),
            session_id: sub.session_id.clone(),
            member_count: 1,
            members_done: done as usize,
            status,
            last_event_at: sub.last_event_at,
            // A solo dispatch's name IS its one member's display_name — no
            // separate dispatch_name storage needed for the Solo kind (only
            // Workflow-kind DispatchState carries its own dispatch_name).
            dispatch_name: sub.display_name.clone(),
        })
    }

    /// Get recent events for a specific subagent (sync — safe to call from RPC dispatch).
    pub fn get_history(&self, agent_id: &str, limit: usize) -> Vec<SubagentEvent> {
        let sessions = self.sessions.lock().unwrap();
        for session in sessions.values() {
            if let Some(state) = session.subagents.get(agent_id) {
                let events = &state.events;
                let start = events.len().saturating_sub(limit);
                return events[start..].to_vec();
            }
        }
        Vec::new()
    }

    /// Get info for a single subagent (sync — safe to call from RPC dispatch).
    /// Targeted counterpart of `list_active()` — a subagent pane reopened
    /// after its `subagent:spawned` event already fired needs its info once,
    /// at mount; scanning + cloning every active subagent across every
    /// session for that is wasteful when the caller already knows the id.
    pub fn get_info(&self, agent_id: &str) -> Option<SubAgent> {
        let sessions = self.sessions.lock().unwrap();
        for session in sessions.values() {
            if let Some(state) = session.subagents.get(agent_id) {
                return Some(state.info.clone());
            }
        }
        None
    }

    /// Set a subagent's Haiku-generated display name and broadcast
    /// `subagent:named` so every client watching this session picks up the
    /// result, not just the one that triggered the naming call. Returns
    /// `false` if the subagent isn't tracked (e.g. it aged out between the
    /// RPC firing and resolving) — the caller should treat that as a no-op,
    /// not an error.
    pub fn set_display_name(&self, agent_id: &str, display_name: &str) -> bool {
        // Captured alongside the mutation itself (not re-looked-up after
        // unlocking) — this is the exact moment a NAME-based grouping key is
        // born, so the fields that decide which group it lands in
        // (dispatch_id, parent_block_id) need to be logged from the same
        // locked read that set it, not a racy re-fetch.
        let mut found_context: Option<(String, String, String)> = None;
        let found = {
            let mut sessions = self.sessions.lock().unwrap();
            let mut found = false;
            for session in sessions.values_mut() {
                if let Some(state) = session.subagents.get_mut(agent_id) {
                    state.info.display_name = Some(display_name.to_string());
                    found_context = Some((
                        state.info.parent_block_id.clone(),
                        state.info.session_id.clone(),
                        state.info.dispatch_id.clone(),
                    ));
                    found = true;
                    break;
                }
            }
            found
        };
        // Mutex released here — broadcast outside the lock

        if let Some((parent_block_id, session_id, dispatch_id)) = &found_context {
            tracing::info!(
                agent_id = %agent_id,
                display_name = %display_name,
                parent_block_id = %parent_block_id,
                session_id = %session_id,
                dispatch_id = %dispatch_id,
                "subagent display_name resolved"
            );
        }

        if found {
            let named_event = WSEventType {
                eventtype: WS_EVENT_RPC.to_string(),
                oref: String::new(),
                data: Some(json!({
                    "command": "eventrecv",
                    "data": {
                        "event": "subagent:named",
                        "data": {
                            "agentId": agent_id,
                            "displayName": display_name,
                        }
                    }
                })),
            };
            self.event_bus.broadcast_event(&named_event);
        }
        found
    }

    /// Set a Workflow-kind dispatch's Haiku-generated name and broadcast
    /// `dispatch:updated` so every client watching this session picks up the
    /// result. Mirrors `set_display_name` one level up — see that method's
    /// doc comment for the "found=false is a safe no-op" convention. Only
    /// Workflow-kind dispatches store a `dispatch_name` here; a Solo
    /// dispatch's name IS its one member's `display_name` (`solo_dispatch`
    /// reads it straight through), so `set_display_name` already covers the
    /// Solo case — this method is never called for a `solo:` dispatch_id.
    pub fn set_dispatch_name(&self, dispatch_id: &str, name: &str) -> bool {
        let info = {
            let mut dispatches = self.dispatches.lock().unwrap();
            match dispatches.get_mut(dispatch_id) {
                Some(state) => {
                    state.info.dispatch_name = Some(name.to_string());
                    Some(state.info.clone())
                }
                None => None,
            }
        };
        // Mutex released here — broadcast outside the lock

        let Some(info) = info else { return false };
        tracing::info!(
            dispatch_id = %dispatch_id,
            dispatch_name = %name,
            parent_block_id = %info.parent_block_id,
            session_id = %info.session_id,
            "dispatch display_name resolved"
        );
        self.broadcast_dispatch_updated(&info);
        true
    }

    /// Test-only accessor for `naming_triggered` — lets tests assert the
    /// dedup gate claimed (or didn't claim) a given dispatch_id without
    /// needing a real `Arc<Self>`/spawned task (see `trigger_eager_naming`'s
    /// doc comment below for why `naming_triggered_contains` alone, not the
    /// full eager-naming flow, is what unit tests can exercise).
    #[cfg(test)]
    pub(crate) fn naming_triggered_contains(&self, dispatch_id: &str) -> bool {
        self.naming_triggered.lock().unwrap().contains(dispatch_id)
    }

    /// Fire the one eager Haiku-naming call for a dispatch's first-observed
    /// live spawn (issue: SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19).
    /// Caller (`process_jsonl_change`) has already atomically claimed
    /// `dispatch_id` in `naming_triggered` before calling this — this method
    /// itself does no further dedup.
    ///
    /// Upgrades `self_ref` to a real `Arc<Self>` to move into the spawned
    /// task; silently no-ops if this watcher was built via bare `new()`
    /// (most unit tests) rather than `spawn()` — there is no Arc to upgrade
    /// to in that case, matching this module's "untracked -> safe no-op"
    /// convention rather than panicking.
    pub(super) fn trigger_eager_naming(&self, dispatch_id: String, first_member_agent_id: String, is_workflow: bool) {
        let Some(watcher) = self.self_ref.lock().unwrap().as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        tokio::spawn(async move {
            if is_workflow {
                crate::server::app_api::session::generate_dispatch_name(
                    &watcher.wstore,
                    &watcher,
                    &dispatch_id,
                    &first_member_agent_id,
                    crate::server::app_api::session::pull_call_semaphore(),
                ).await;
            } else {
                crate::server::app_api::session::generate_subagent_name(
                    &watcher.wstore,
                    &watcher,
                    &first_member_agent_id,
                    crate::server::app_api::session::pull_call_semaphore(),
                ).await;
            }
        });
    }

    /// Select up to `limit` currently-unnamed subagents/dispatches for the
    /// bounded backfill-naming pass (`resolve_unnamed_backlog`), most
    /// recently-active first, and atomically claim each selected item in
    /// `naming_triggered` before returning it — reusing the exact dedup
    /// structure the live eager-naming path already uses
    /// (`jsonl::process_jsonl_change`), so this can never double-name
    /// something, race with a live spawn of the same dispatch, or need a
    /// second `HashSet`. Because the claim is permanent, calling this again
    /// (a second Swarm-pane-open before an earlier batch finishes, or after
    /// it drains) never re-selects anything already claimed — this is what
    /// makes `resolve_unnamed_backlog` safe to fire on every pane open with
    /// no extra debounce.
    ///
    /// Deliberately never holds `sessions`/`dispatches` and `naming_triggered`
    /// locked at the same time — mirrors `process_jsonl_change`'s own
    /// convention (see its "Mutex released here" comment).
    pub(super) fn select_unnamed_backlog(&self, limit: usize) -> Vec<BacklogNamingItem> {
        let mut candidates: Vec<(u64, BacklogNamingItem)> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .values()
                .flat_map(|s| s.subagents.values())
                .filter(|state| {
                    state.info.display_name.is_none() && state.info.dispatch_id.starts_with("solo:")
                })
                .map(|state| {
                    (
                        state.info.last_event_at,
                        BacklogNamingItem::Solo { agent_id: state.info.agent_id.clone() },
                    )
                })
                .collect()
        };

        let unnamed_workflows: Vec<(String, u64)> = {
            let dispatches = self.dispatches.lock().unwrap();
            dispatches
                .values()
                .map(|state| &state.info)
                .filter(|info| info.kind == DispatchKind::Workflow && info.dispatch_name.is_none())
                .map(|info| (info.dispatch_id.clone(), info.last_event_at))
                .collect()
        };

        if !unnamed_workflows.is_empty() {
            let sessions = self.sessions.lock().unwrap();
            for (dispatch_id, last_event_at) in unnamed_workflows {
                let representative = sessions
                    .values()
                    .flat_map(|s| s.subagents.values())
                    .find(|state| state.info.dispatch_id == dispatch_id)
                    .map(|state| state.info.agent_id.clone());
                if let Some(representative_agent_id) = representative {
                    candidates.push((
                        last_event_at,
                        BacklogNamingItem::Workflow { dispatch_id, representative_agent_id },
                    ));
                }
            }
        }

        candidates.sort_by(|a, b| b.0.cmp(&a.0));

        let mut naming_triggered = self.naming_triggered.lock().unwrap();
        let mut selected = Vec::with_capacity(limit.min(candidates.len()));
        for (_, item) in candidates {
            if selected.len() >= limit {
                break;
            }
            if naming_triggered.insert(item.dispatch_id()) {
                selected.push(item);
            }
        }
        selected
    }

    /// Bounded, rate-limited burst that resolves names for whatever's
    /// currently unnamed in the backfilled/historical backlog
    /// (`select_unnamed_backlog`) — fired only via the
    /// `("subagent", "ResolveUnnamedBacklog")` RPC, itself only ever called
    /// from `SwarmViewModel`'s constructor (i.e. a human actually opening
    /// the Swarm pane), never from the headless per-agent-pane backfill
    /// scan — that path stays exactly as `live`-gated as before (see
    /// `process_jsonl_change`'s doc comment and
    /// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md).
    ///
    /// Deliberately separate from `trigger_eager_naming`'s live path: uses
    /// its own `backlog_naming_semaphore()` (cap 1), not the shared
    /// `pull_call_semaphore()` every live user-facing ambient caller
    /// contends for.
    pub(crate) async fn resolve_unnamed_backlog(self: std::sync::Arc<Self>) {
        let items = self.select_unnamed_backlog(BACKLOG_NAMING_BATCH_LIMIT);
        for item in items {
            let watcher = std::sync::Arc::clone(&self);
            tokio::spawn(async move {
                match item {
                    BacklogNamingItem::Solo { agent_id } => {
                        crate::server::app_api::session::generate_subagent_name(
                            &watcher.wstore,
                            &watcher,
                            &agent_id,
                            crate::server::app_api::session::backlog_naming_semaphore(),
                        )
                        .await;
                    }
                    BacklogNamingItem::Workflow { dispatch_id, representative_agent_id } => {
                        crate::server::app_api::session::generate_dispatch_name(
                            &watcher.wstore,
                            &watcher,
                            &dispatch_id,
                            &representative_agent_id,
                            crate::server::app_api::session::backlog_naming_semaphore(),
                        )
                        .await;
                    }
                }
            });
        }
    }
}
