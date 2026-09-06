// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The JSONL/journal file-change state machine: `process_jsonl_change` (one
//! continuous state machine, moved whole rather than decomposed further) and
//! its dispatch-membership/coalescing plumbing — `flush_pending_dispatch_
//! activity`, `update_dispatch_membership`, `queue_dispatch_updated`,
//! `process_journal_change`, `dispatch_entry`, `refresh_dispatch_info`,
//! `refresh_dispatch_status`, `broadcast_dispatch_updated`.

use std::collections::HashMap;
use std::path::Path;

use serde_json::json;

use super::parse::*;
use super::types::*;
use super::SubagentWatcher;
use crate::backend::eventbus::{WSEventType, WS_EVENT_RPC};

impl SubagentWatcher {
    /// Process a changed/new JSONL subagent file. Reads new lines, updates state,
    /// and broadcasts events via EventBus.
    ///
    /// `live`: `true` only when this call originates from the filesystem
    /// watcher observing a real, in-the-moment change; `false` for a
    /// cold-backfill replay (`scan_subagents_dir`, capped at
    /// `BACKFILL_MAX_FILES`, run on pane reopen / srv restart). Eager
    /// Haiku-naming (see the `is_new` handling below) is gated on `live` —
    /// without this, every restart of a long-lived session would re-fire a
    /// naming call for every dispatch replayed from history, repeating the
    /// broadcast-storm incident class in
    /// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md, just for
    /// Haiku spend instead of WS events.
    pub(super) fn process_jsonl_change(&self, parent_agent: &str, parent_block_id: &str, jsonl_path: &Path, live: bool) {
        // Extract agent ID from filename: agent-<id>.jsonl
        let agent_id = match jsonl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("agent-"))
        {
            Some(id) => id.to_string(),
            None => return,
        };

        // Session id = the directory containing subagents/. Workflow member
        // files are nested deeper (subagents/workflows/<wf>/agent-*.jsonl),
        // so walk ancestors instead of assuming a fixed depth.
        let session_id = derive_session_id(jsonl_path);
        let workflow_id = parse_workflow_id(jsonl_path);
        // Every SubAgent has a real dispatch_id now — a solo Task-tool call
        // gets a synthesized one (SPEC §5), not just workflow members.
        let dispatch_id = workflow_id.clone().unwrap_or_else(|| solo_dispatch_id(&agent_id));

        // Read the current offset before locking (so file I/O is outside the lock)
        let current_offset = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .get(&session_id)
                .and_then(|s| s.subagents.get(&agent_id))
                .map(|s| s.file_offset)
                .unwrap_or(0)
        };

        // Do file I/O outside the mutex lock
        let (new_events, new_offset, meta) = match read_jsonl_from_offset(jsonl_path, current_offset) {
            Ok(result) => result,
            Err(e) => {
                tracing::debug!(
                    agent_id = %agent_id,
                    error = %e,
                    "failed to read subagent JSONL"
                );
                return;
            }
        };

        // Now lock and update state
        let (is_new, info_snapshot, completed) = {
            let mut sessions = self.sessions.lock().unwrap();
            let session = sessions
                .entry(session_id.clone())
                .or_insert_with(|| SessionWatch {
                    subagents: HashMap::new(),
                });

            let is_new = !session.subagents.contains_key(&agent_id);
            // A subagent's parent_block_id is stamped once, at first
            // discovery (below), and never updated on later re-scans of the
            // same session — see the entry's doc comment. If this session is
            // being (re)scanned under a DIFFERENT block_id than the one this
            // subagent was originally bound to (e.g. the pane was reopened
            // under a new block), the subagent stays attributed to its
            // original, now possibly-stale parent_block_id. That's a direct
            // candidate mechanism for a subagent silently falling outside
            // the block reconcile_stale_subagents/buildTree() expect it
            // under — log it so it's visible without needing to reproduce.
            if let Some(existing) = session.subagents.get(&agent_id) {
                if existing.info.parent_block_id != parent_block_id {
                    // reagent (PR #2143 round 1, P1): info!, not debug! — this
                    // is the single most likely diagnostic trail for the
                    // duplication bug this PR exists to help find; at debug!
                    // it would be silently dropped by the default production
                    // filter and never actually appear when it matters.
                    tracing::info!(
                        agent_id = %agent_id,
                        session_id = %session_id,
                        existing_parent_block_id = %existing.info.parent_block_id,
                        rescanned_parent_block_id = %parent_block_id,
                        "subagent re-observed under a different parent_block_id than its original — parent_block_id NOT updated"
                    );
                }
            }
            // A replayed spawn (`live == false`) describes a subagent whose
            // transcript already existed on disk when this pane opened.
            // Inserting it as `Active` asserts something the replay cannot
            // know, and `reconcile_stale_subagents` (scan.rs) then retracts it
            // moments later — up to 200 spurious abandon broadcasts per reopen,
            // and a window where `subagent.ListActive` reports rows the backend
            // is about to disown. See
            // docs/reports/REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md §2.
            //
            // Decided here using the SAME predicate reconcile uses, not a
            // second independently-invented one — a confirmed-idle parent turn
            // means nothing on disk can still be running. Two deliberate
            // fallbacks to `Active`:
            //   - `live` always wins. `turn_active` can lag a genuine spawn,
            //     and mislabelling a real subagent would break live rows.
            //   - An unregistered controller is UNKNOWN, not idle.
            //     `scan_session_subagents` can run before the controller
            //     registers, so this keeps reconcile's own retry path as the
            //     authority rather than inferring `Abandoned` from absent
            //     information.
            let initial_status = if live {
                SubAgentStatus::Active
            } else {
                match crate::backend::blockcontroller::get_block_controller_status(parent_block_id)
                    .map(|s| s.turn_active)
                {
                    Some(false) => SubAgentStatus::Abandoned,
                    _ => SubAgentStatus::Active,
                }
            };
            // Real spawn time for a REPLAYED file, not replay time. The
            // events were just parsed from a transcript that may be months
            // old; stamping `now_millis()` made every backfilled subagent
            // claim to have started at pane-open, which both misreported
            // elapsed time and corrupted the dock's newest-first ordering.
            // Falls back to `now_millis()` only when the file yielded no
            // parseable event at all (a genuinely new/empty transcript,
            // where "now" is the honest answer). See
            // docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md.
            let initial_spawned_at = new_events
                .iter()
                .map(|e| e.timestamp)
                .min()
                .unwrap_or_else(now_millis);
            let state = session.subagents.entry(agent_id.clone()).or_insert_with(|| {
                SubagentState {
                    info: SubAgent {
                        agent_id: agent_id.clone(),
                        slug: String::new(),
                        jsonl_path: jsonl_path.to_string_lossy().to_string(),
                        parent_agent: parent_agent.to_string(),
                        parent_block_id: parent_block_id.to_string(),
                        session_id: session_id.clone(),
                        spawned_at: initial_spawned_at,
                        last_event_at: initial_spawned_at,
                        status: initial_status,
                        event_count: 0,
                        model: None,
                        dispatch_id: dispatch_id.clone(),
                        display_name: None,
                        spawned_from_agent_id: None,
                    },
                    file_offset: 0,
                    events: Vec::new(),
                }
            });

            // A live observation outranks a replay's inference.
            //
            // codex P1 on PR #2837: if a backfill scan races ahead of the
            // filesystem watcher on a newly-created transcript, it can read the
            // documented-lagging `turn_active == false` and insert the entry as
            // `Abandoned`. The `live == true` call that follows would otherwise
            // be stuck with that: `or_insert_with` doesn't run for an existing
            // entry, ordinary events never revive a status, and
            // `reconcile_stale_subagents` only ever downgrades `Active` ->
            // `Abandoned`. A genuinely running subagent would read as abandoned
            // until a `result` line happened to arrive.
            //
            // `Abandoned` is always an inference; watching the file change is a
            // direct observation, so the observation wins. This can't strand a
            // wrong answer in the other direction either: if the subagent has
            // in fact finished, its own `result` event sets `Completed` below;
            // if the parent turn really is idle, the next reconcile pass
            // re-abandons it.
            if live && state.info.status == SubAgentStatus::Abandoned {
                state.info.status = SubAgentStatus::Active;
            }

            state.file_offset = new_offset;
            state.info.dispatch_id = dispatch_id.clone();

            // Update metadata from first line if we got it
            if let Some(m) = meta {
                if !m.slug.is_empty() {
                    state.info.slug = m.slug;
                }
                if let Some(model) = m.model {
                    state.info.model = Some(model);
                }
                if m.parent_uuid.is_some() {
                    state.info.spawned_from_agent_id = m.parent_uuid;
                }
            }

            if new_events.is_empty() && !is_new {
                return;
            }

            // Process events
            let mut completed = false;
            for event in &new_events {
                state.info.event_count += 1;
                state.info.last_event_at = event.timestamp;
                state.events.push(event.clone());
            }
            // Trim to the cap, oldest-first — event_count above already
            // recorded the true cumulative total before this truncation.
            if state.events.len() > MAX_SUBAGENT_EVENTS {
                let excess = state.events.len() - MAX_SUBAGENT_EVENTS;
                state.events.drain(..excess);
            }

            // Completion by terminal `Result` line.
            //
            // ⚠ THIS NEVER FIRES ON ANY TRANSCRIPT AGENTMUX CURRENTLY WRITES.
            // No `"type":"result"` line exists in the `entrypoint: sdk-cli`
            // format — 0 of 11 subagent transcripts on the investigated
            // machine, across CLI 2.1.198 and 2.1.247, and none in the parent
            // session files either. Every completed subagent transcript ends
            // on an `assistant` message. Evidence:
            // docs/reports/REPORT_SUBAGENT_COMPLETION_NEVER_DETECTED_2026_09_05.md.
            //
            // Real completion is established by `completion.rs` instead,
            // correlating the PARENT's `tool_result` for the dispatch's
            // `tool_use_id` (#3007). If you are here because completions look
            // wrong, that is the code path to read — not this one.
            //
            // Kept rather than deleted: it is not incorrect, only unreachable
            // in a transcript format we don't control, and it would resume
            // being the fastest completion signal (immediate, rather than
            // waiting for the parent turn to end) if such lines ever appear.
            // Deleting it would also strand `PendingCompletion` and the
            // workflow-coalescing flush below.
            //
            // The history is worth keeping visible, because it is how the bug
            // survived: #2283 replaced a placeholder-text match its own
            // comment described as "almost never fired" with this discriminant
            // check, which fires *never*. Both fail identically from outside —
            // rows that simply never complete — so the regression read as the
            // pre-existing flakiness.
            if let Some(last) = new_events.last() {
                if matches!(&last.event_type, SubagentEventType::Result { .. }) {
                    completed = true;
                    state.info.status = SubAgentStatus::Completed;
                }
            }

            let info_snapshot = state.info.clone();
            (is_new, info_snapshot, completed)
        };
        // Mutex released here — broadcast outside the lock

        if is_new {
            // Eager per-dispatch Haiku naming (issue:
            // SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19). Gated on
            // `live` (never during backfill replay) and on `naming_triggered`
            // atomically claiming this dispatch_id — for a Workflow dispatch,
            // every member's is_new fires with the SAME dispatch_id, so only
            // the first member to reach this point wins the claim; a Solo
            // dispatch's dispatch_id is unique to its one member, so it
            // always wins its own claim exactly once.
            if live && self.naming_triggered.lock().unwrap().insert(dispatch_id.clone()) {
                self.trigger_eager_naming(dispatch_id.clone(), agent_id.clone(), workflow_id.is_some());
            }
            if workflow_id.is_some() {
                let mut pending = self.pending_activity.lock().unwrap();
                let entry = pending
                    .entry(dispatch_id.clone())
                    .or_insert_with(|| PendingDispatchActivity::new(parent_agent, parent_block_id, &session_id));
                entry.spawned.push(PendingSpawn {
                    agent_id: info_snapshot.agent_id.clone(),
                    slug: info_snapshot.slug.clone(),
                    model: info_snapshot.model.clone(),
                });
            } else {
                let spawned_event = WSEventType {
                    eventtype: WS_EVENT_RPC.to_string(),
                    oref: String::new(),
                    data: Some(json!({
                        "command": "eventrecv",
                        "data": {
                            "event": "subagent:spawned",
                            "data": {
                                "agentId": info_snapshot.agent_id,
                                "slug": info_snapshot.slug,
                                "parentAgent": parent_agent,
                                "parentBlockId": parent_block_id,
                                "sessionId": session_id,
                                "model": info_snapshot.model,
                                "dispatchId": info_snapshot.dispatch_id,
                            }
                        }
                    })),
                };
                self.event_bus.broadcast_event(&spawned_event);
                tracing::info!(
                    agent_id = %agent_id,
                    slug = %info_snapshot.slug,
                    parent = %parent_agent,
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    dispatch_id = %info_snapshot.dispatch_id,
                    "subagent spawned"
                );
            }
        }

        if !new_events.is_empty() {
            if workflow_id.is_some() {
                // Workflow-kind member: buffer for the next coalesced
                // dispatch:activity flush instead of broadcasting per-member
                // (SPEC §7) — a large dispatch's activity ticks are the
                // exact mechanism behind the crash-storm broadcast volume in
                // docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md.
                let mut pending = self.pending_activity.lock().unwrap();
                let entry = pending
                    .entry(dispatch_id.clone())
                    .or_insert_with(|| PendingDispatchActivity::new(parent_agent, parent_block_id, &session_id));
                entry.members.push((agent_id.clone(), new_events.clone()));
            } else {
                // Solo dispatch: exactly one member, no coalescing benefit for
                // this event itself — broadcast subagent:activity immediately,
                // same as before the redesign (existing consumers, e.g. the
                // agent-pane activity dock, still rely on this per-member
                // stream). ADDITIONALLY queue into pending_activity so
                // dispatch:activity also flows for solo dispatch_ids — Phase B
                // of SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19's
                // unified concatenated feed needs this for solo rows, which
                // dispatch:activity never covered before. Deliberate dual
                // emission, not a replacement — see that spec's Phase A notes
                // for why the existing subagent:activity stream is kept as-is.
                {
                    let mut pending = self.pending_activity.lock().unwrap();
                    let entry = pending
                        .entry(dispatch_id.clone())
                        .or_insert_with(|| PendingDispatchActivity::new(parent_agent, parent_block_id, &session_id));
                    entry.members.push((agent_id.clone(), new_events.clone()));
                }
                let activity_event = WSEventType {
                    eventtype: WS_EVENT_RPC.to_string(),
                    oref: String::new(),
                    data: Some(json!({
                        "command": "eventrecv",
                        "data": {
                            "event": "subagent:activity",
                            "data": {
                                "agentId": agent_id,
                                "parentAgent": parent_agent,
                                "parentBlockId": parent_block_id,
                                "newEvents": new_events.len(),
                                "totalEvents": info_snapshot.event_count,
                                "events": new_events,
                                "dispatchId": info_snapshot.dispatch_id,
                            }
                        }
                    })),
                };
                self.event_bus.broadcast_event(&activity_event);
            }
        }

        if completed {
            if workflow_id.is_some() {
                let mut pending = self.pending_activity.lock().unwrap();
                let entry = pending
                    .entry(dispatch_id.clone())
                    .or_insert_with(|| PendingDispatchActivity::new(parent_agent, parent_block_id, &session_id));
                entry.completed.push(PendingCompletion {
                    agent_id: agent_id.clone(),
                    total_events: info_snapshot.event_count,
                });
            } else {
                let completed_event = WSEventType {
                    eventtype: WS_EVENT_RPC.to_string(),
                    oref: String::new(),
                    data: Some(json!({
                        "command": "eventrecv",
                        "data": {
                            "event": "subagent:completed",
                            "data": {
                                "agentId": agent_id,
                                "parentAgent": parent_agent,
                                "parentBlockId": parent_block_id,
                                "totalEvents": info_snapshot.event_count,
                                "dispatchId": info_snapshot.dispatch_id,
                            }
                        }
                    })),
                };
                self.event_bus.broadcast_event(&completed_event);
                tracing::info!(
                    agent_id = %agent_id,
                    total_events = info_snapshot.event_count,
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    dispatch_id = %info_snapshot.dispatch_id,
                    "subagent completed"
                );
            }
        }

        if let Some(wf_id) = workflow_id {
            self.update_dispatch_membership(
                &wf_id,
                parent_agent,
                parent_block_id,
                &session_id,
                is_new,
                completed,
            );
        } else if is_new || completed {
            // Solo dispatch: no tracked DispatchState to update — just
            // broadcast the freshly-synthesized AgentDispatch so dispatch:
            // updated fires for both kinds (SPEC §5), not just Workflow.
            if let Some(dispatch) = Self::solo_dispatch(&info_snapshot) {
                self.broadcast_dispatch_updated(&dispatch);
            }
        }
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
                let spawned_event = WSEventType {
                    eventtype: WS_EVENT_RPC.to_string(),
                    oref: String::new(),
                    data: Some(json!({
                        "command": "eventrecv",
                        "data": {
                            "event": "subagent:spawned",
                            "data": {
                                "agentId": spawn.agent_id,
                                "slug": spawn.slug,
                                "parentAgent": pending.parent_agent,
                                "parentBlockId": pending.parent_block_id,
                                "sessionId": pending.session_id,
                                "model": spawn.model,
                                "dispatchId": dispatch_id,
                            }
                        }
                    })),
                };
                self.event_bus.broadcast_event(&spawned_event);
                tracing::info!(
                    agent_id = %spawn.agent_id,
                    slug = %spawn.slug,
                    parent = %pending.parent_agent,
                    parent_block_id = %pending.parent_block_id,
                    session_id = %pending.session_id,
                    dispatch_id = %dispatch_id,
                    "subagent spawned"
                );
            }

            if !pending.members.is_empty() {
                let new_events: usize = pending.members.iter().map(|(_, evs)| evs.len()).sum();
                let event = WSEventType {
                    eventtype: WS_EVENT_RPC.to_string(),
                    oref: String::new(),
                    data: Some(json!({
                        "command": "eventrecv",
                        "data": {
                            "event": "dispatch:activity",
                            "data": {
                                "dispatchId": dispatch_id,
                                "parentAgent": pending.parent_agent,
                                "parentBlockId": pending.parent_block_id,
                                "sessionId": pending.session_id,
                                "newEvents": new_events,
                                "members": pending.members.iter().map(|(agent_id, events)| {
                                    json!({ "agentId": agent_id, "events": events })
                                }).collect::<Vec<_>>(),
                            }
                        }
                    })),
                };
                self.event_bus.broadcast_event(&event);
            }

            for done in &pending.completed {
                let completed_event = WSEventType {
                    eventtype: WS_EVENT_RPC.to_string(),
                    oref: String::new(),
                    data: Some(json!({
                        "command": "eventrecv",
                        "data": {
                            "event": "subagent:completed",
                            "data": {
                                "agentId": done.agent_id,
                                "parentAgent": pending.parent_agent,
                                "parentBlockId": pending.parent_block_id,
                                "totalEvents": done.total_events,
                                "dispatchId": dispatch_id,
                            }
                        }
                    })),
                };
                self.event_bus.broadcast_event(&completed_event);
                tracing::info!(
                    agent_id = %done.agent_id,
                    total_events = done.total_events,
                    parent_block_id = %pending.parent_block_id,
                    session_id = %pending.session_id,
                    dispatch_id = %dispatch_id,
                    "subagent completed"
                );
            }

            if let Some(info) = &pending.latest_info {
                self.broadcast_dispatch_updated(info);
            }
        }
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

    pub(super) fn broadcast_dispatch_updated(&self, info: &AgentDispatch) {
        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(json!({
                "command": "eventrecv",
                "data": {
                    "event": "dispatch:updated",
                    "data": {
                        "dispatchId": info.dispatch_id,
                        "kind": info.kind,
                        "parentAgent": info.parent_agent,
                        "parentBlockId": info.parent_block_id,
                        "sessionId": info.session_id,
                        "memberCount": info.member_count,
                        "membersDone": info.members_done,
                        "status": info.status,
                        "dispatchName": info.dispatch_name,
                    }
                }
            })),
        };
        self.event_bus.broadcast_event(&event);
    }
}
