// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `process_jsonl_change`: who a changed transcript belongs to, what is read from it, how the
//! subagent's state is brought up to date under the lock, and what is announced once the lock is released.

use super::*;

/// Which transcript changed, and whose subagent it is.
pub(super) struct Change<'a> {
    parent_agent: &'a str,
    parent_block_id: &'a str,
    live: bool,
    agent_id: String,
    session_id: String,
    workflow_id: Option<String>,
    dispatch_id: String,
}

impl<'a> Change<'a> {
    /// `None` when the file is not named `agent-<id>.jsonl`.
    pub(super) fn of(parent_agent: &'a str, parent_block_id: &'a str, jsonl_path: &Path, live: bool) -> Option<Self> {
        // Extract agent ID from filename: agent-<id>.jsonl
        let agent_id = jsonl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("agent-"))?
            .to_string();

        // Session id = the directory containing subagents/. Workflow member
        // files are nested deeper (subagents/workflows/<wf>/agent-*.jsonl),
        // so walk ancestors instead of assuming a fixed depth.
        let session_id = derive_session_id(jsonl_path);
        let workflow_id = parse_workflow_id(jsonl_path);
        // Every SubAgent has a real dispatch_id now — a solo Task-tool call
        // gets a synthesized one (SPEC §5), not just workflow members.
        let dispatch_id = workflow_id.clone().unwrap_or_else(|| solo_dispatch_id(&agent_id));
        Some(Change { parent_agent, parent_block_id, live, agent_id, session_id, workflow_id, dispatch_id })
    }
}

/// What recording a change found: nothing to announce but a late sidecar, or a subagent that changed.
// Built once per change and matched at once: the size gap is not worth a box.
#[allow(clippy::large_enum_variant)]
pub(super) enum Recorded {
    Quiet { id_learned: bool, name_learned: Option<String> },
    Changed { is_new: bool, info_snapshot: SubAgent, completed: bool, id_learned: bool, name_learned: Option<String> },
}

impl SubagentWatcher {
    /// The lines added to the transcript since the last read, or `None` when it cannot be read.
    pub(super) fn read_new_lines(&self, change: &Change, jsonl_path: &Path) -> Option<(Vec<SubagentEvent>, u64, Option<JsonlMeta>)> {
        // Read the current offset before locking (so file I/O is outside the lock)
        let current_offset = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .get(&change.session_id)
                .and_then(|s| s.subagents.get(&change.agent_id))
                .map(|s| s.file_offset)
                .unwrap_or(0)
        };

        // Do file I/O outside the mutex lock
        let (new_events, new_offset, meta) = match read_jsonl_from_offset(jsonl_path, current_offset) {
            Ok(result) => result,
            Err(e) => {
                tracing::debug!(
                    agent_id = %change.agent_id,
                    error = %e,
                    "failed to read subagent JSONL"
                );
                return None;
            }
        };
        Some((new_events, new_offset, meta))
    }

    /// Bring the subagent's state up to date under the lock, and say what changed.
    pub(super) fn record_change(
        &self,
        change: &Change,
        jsonl_path: &Path,
        new_events: &[SubagentEvent],
        new_offset: u64,
        meta: Option<JsonlMeta>,
    ) -> Recorded {
        // Now lock and update state
            let mut sessions = self.sessions.lock().unwrap();
            let session = sessions
                .entry(change.session_id.clone())
                .or_insert_with(|| SessionWatch {
                    subagents: HashMap::new(),
                });

            let is_new = !session.subagents.contains_key(&change.agent_id);
        note_reparented(session, change);
        let initial_status = initial_status(change);
        let initial_spawned_at = initial_spawned_at(new_events);
        let state = session
            .subagents
            .entry(change.agent_id.clone())
            .or_insert_with(|| new_state(change, jsonl_path, initial_status, initial_spawned_at));
        let (id_learned, name_learned) = learn_sidecar(state, jsonl_path, is_new);
        observe(state, change, new_offset, meta);

        if new_events.is_empty() && !is_new {
            return Recorded::Quiet { id_learned, name_learned };
        }

        let completed = take_events(state, new_events);
        let info_snapshot = state.info.clone();
        Recorded::Changed { is_new, info_snapshot, completed, id_learned, name_learned }
    }

    /// The late sidecar's announcements: the id an already-known subagent gained, and its name.
    pub(super) fn announce_learned(&self, change: &Change, id_learned: bool, name_learned: Option<String>) {
        if id_learned {
            self.broadcast_subagent_updated(&change.agent_id, change.parent_block_id);
        }
        if let Some(name) = name_learned {
            self.broadcast_subagent_named(&change.agent_id, &name);
        }
    }

    /// A new subagent: eager naming for a live one, then its spawn (buffered for a workflow member).
    pub(super) fn announce_spawn(&self, change: &Change, info_snapshot: &SubAgent) {
        // Eager per-dispatch Haiku naming (issue:
        // SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19). Gated on
        // `live` (never during backfill replay) and on `naming_triggered`
        // atomically claiming this dispatch_id — for a Workflow dispatch,
        // every member's is_new fires with the SAME dispatch_id, so only
        // the first member to reach this point wins the claim; a Solo
        // dispatch's dispatch_id is unique to its one member, so it
        // always wins its own claim exactly once.
        if change.live && self.naming_triggered.lock().unwrap().insert(change.dispatch_id.clone()) {
            self.trigger_eager_naming(change.dispatch_id.clone(), change.agent_id.clone(), change.workflow_id.is_some());
        }
        let spawn = PendingSpawn {
            agent_id: info_snapshot.agent_id.clone(),
            slug: info_snapshot.slug.clone(),
            model: info_snapshot.model.clone(),
        };
        if change.workflow_id.is_some() {
            let mut pending = self.pending_activity.lock().unwrap();
            let entry = pending
                .entry(change.dispatch_id.clone())
                .or_insert_with(|| PendingDispatchActivity::new(change.parent_agent, change.parent_block_id, &change.session_id));
            entry.spawned.push(spawn);
        } else {
            self.broadcast_member_spawned(
                &spawn,
                change.parent_agent,
                change.parent_block_id,
                &change.session_id,
                &info_snapshot.dispatch_id,
            );
        }
    }

    /// The events just read: buffered for a workflow member, announced at once for a solo subagent.
    pub(super) fn announce_activity(&self, change: &Change, new_events: &[SubagentEvent], info_snapshot: &SubAgent) {
        if change.workflow_id.is_some() {
            // Workflow-kind member: buffer for the next coalesced
            // dispatch:activity flush instead of broadcasting per-member
            // (SPEC §7) — a large dispatch's activity ticks are the
            // exact mechanism behind the crash-storm broadcast volume in
            // docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md.
            let mut pending = self.pending_activity.lock().unwrap();
            let entry = pending
                .entry(change.dispatch_id.clone())
                .or_insert_with(|| PendingDispatchActivity::new(change.parent_agent, change.parent_block_id, &change.session_id));
            entry.members.push((change.agent_id.clone(), new_events.to_vec()));
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
                    .entry(change.dispatch_id.clone())
                    .or_insert_with(|| PendingDispatchActivity::new(change.parent_agent, change.parent_block_id, &change.session_id));
                entry.members.push((change.agent_id.clone(), new_events.to_vec()));
            }
            self.event_bus.broadcast_typed::<crate::backend::mps_events::SubagentActivity>(&super::super::events::SubagentActivityEvent {
                agent_id: change.agent_id.to_string(),
                parent_agent: change.parent_agent.to_string(),
                parent_block_id: change.parent_block_id.to_string(),
                new_events: new_events.len(),
                total_events: info_snapshot.event_count,
                events: new_events.to_vec(),
                dispatch_id: info_snapshot.dispatch_id.to_string(),
            });
        }
    }

    /// The subagent's completion: buffered for a workflow member, announced at once for a solo subagent.
    pub(super) fn announce_completion(&self, change: &Change, info_snapshot: &SubAgent) {
        let done = PendingCompletion {
            agent_id: change.agent_id.clone(),
            total_events: info_snapshot.event_count,
        };
        if change.workflow_id.is_some() {
            let mut pending = self.pending_activity.lock().unwrap();
            let entry = pending
                .entry(change.dispatch_id.clone())
                .or_insert_with(|| PendingDispatchActivity::new(change.parent_agent, change.parent_block_id, &change.session_id));
            entry.completed.push(done);
        } else {
            self.broadcast_member_completed(
                &done,
                change.parent_agent,
                change.parent_block_id,
                &change.session_id,
                &info_snapshot.dispatch_id,
            );
        }
    }

    /// The dispatch the subagent belongs to: its membership for a workflow, the synthesized one for a solo subagent.
    pub(super) fn announce_dispatch(&self, change: &Change, info_snapshot: &SubAgent, is_new: bool, completed: bool) {
        if let Some(wf_id) = &change.workflow_id {
            self.update_dispatch_membership(
                wf_id,
                change.parent_agent,
                change.parent_block_id,
                &change.session_id,
                is_new,
                completed,
            );
        } else if is_new || completed {
            // Solo dispatch: no tracked DispatchState to update — just
            // broadcast the freshly-synthesized AgentDispatch so dispatch:
            // updated fires for both kinds (SPEC §5), not just Workflow.
            if let Some(dispatch) = Self::solo_dispatch(info_snapshot) {
                self.broadcast_dispatch_updated(&dispatch);
            }
        }
    }
}

/// Log a subagent re-observed under another parent block than the one it was first seen under.
fn note_reparented(session: &SessionWatch, change: &Change) {
    // A subagent's parent_block_id is stamped once, at first
    // discovery (`new_state`), and never updated on later re-scans of the
    // same session — see the entry's doc comment. If this session is
    // being (re)scanned under a DIFFERENT block_id than the one this
    // subagent was originally bound to (e.g. the pane was reopened
    // under a new block), the subagent stays attributed to its
    // original, now possibly-stale parent_block_id. That's a direct
    // candidate mechanism for a subagent silently falling outside
    // the block reconcile_stale_subagents/buildTree() expect it
    // under — log it so it's visible without needing to reproduce.
    if let Some(existing) = session.subagents.get(&change.agent_id) {
        if existing.info.parent_block_id != change.parent_block_id {
            // reagent (PR #2143 round 1, P1): info!, not debug! — this
            // is the single most likely diagnostic trail for the
            // duplication bug this PR exists to help find; at debug!
            // it would be silently dropped by the default production
            // filter and never actually appear when it matters.
            tracing::info!(
                agent_id = %change.agent_id,
                session_id = %change.session_id,
                existing_parent_block_id = %existing.info.parent_block_id,
                rescanned_parent_block_id = %change.parent_block_id,
                "subagent re-observed under a different parent_block_id than its original — parent_block_id NOT updated"
            );
        }
    }
}

/// The status a new subagent is inserted with: `Active` for a live spawn, inferred from the parent turn for a replay.
fn initial_status(change: &Change) -> SubAgentStatus {
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
    if change.live {
        SubAgentStatus::Active
    } else {
        match crate::backend::blockcontroller::get_block_controller_status(change.parent_block_id)
            .map(|s| s.turn_active)
        {
            Some(false) => SubAgentStatus::Abandoned,
            _ => SubAgentStatus::Active,
        }
    }
}

/// The spawn time of a new subagent: its first event on disk, else now.
fn initial_spawned_at(new_events: &[SubagentEvent]) -> u64 {
    // Real spawn time for a REPLAYED file, not replay time. The
    // events were just parsed from a transcript that may be months
    // old; stamping `now_millis()` made every backfilled subagent
    // claim to have started at pane-open, which both misreported
    // elapsed time and corrupted the dock's newest-first ordering.
    // Falls back to `now_millis()` only when the file yielded no
    // parseable event at all (a genuinely new/empty transcript,
    // where "now" is the honest answer). See
    // docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md.
    new_events
        .iter()
        .map(|e| e.timestamp)
        .min()
        .unwrap_or_else(now_millis)
}

/// The state a subagent is first recorded with.
fn new_state(change: &Change, jsonl_path: &Path, initial_status: SubAgentStatus, initial_spawned_at: u64) -> SubagentState {
    SubagentState {
        info: SubAgent {
            agent_id: change.agent_id.clone(),
            slug: String::new(),
            jsonl_path: jsonl_path.to_string_lossy().to_string(),
            parent_agent: change.parent_agent.to_string(),
            parent_block_id: change.parent_block_id.to_string(),
            session_id: change.session_id.clone(),
            spawned_at: initial_spawned_at,
            last_event_at: initial_spawned_at,
            status: initial_status,
            event_count: 0,
            model: None,
            dispatch_id: change.dispatch_id.clone(),
            // The parent's own short description of the task
            // (the Agent tool's `description`), from the same
            // sidecar as the tool_use_id: a name with no model call,
            // shown before anyone opens the row. Haiku names a
            // subagent only when there is none (workflow members).
            display_name: super::super::completion::description_for(jsonl_path),
            spawned_from_agent_id: None,
            tool_use_id: super::super::completion::tool_use_id_for(jsonl_path),
        },
        file_offset: 0,
        events: Vec::new(),
    }
}

/// Pick up the sidecar's tool-use id and description once it is readable: whether each was just learned.
fn learn_sidecar(state: &mut SubagentState, jsonl_path: &Path, is_new: bool) -> (bool, Option<String>) {
    // The CLI can write the `.meta.json` sidecar a moment after the
    // transcript itself; keep trying until it is readable. The watcher
    // also routes the sidecar's own write here (`build_watch`), so a
    // sidecar that lands after the transcript's last line is still
    // picked up. `id_learned`: an already-known subagent just gained
    // its id, which the spawn broadcast never carried.
    let id_learned = if state.info.tool_use_id.is_none() {
        state.info.tool_use_id = super::super::completion::tool_use_id_for(jsonl_path);
        !is_new && state.info.tool_use_id.is_some()
    } else {
        false
    };
    // The description arrives with the same sidecar, so it can be learned
    // as late as the id. `Some` for an already-known subagent that just
    // gained its name, which `announce_learned` broadcasts as `subagent:named`.
    let name_learned = if state.info.display_name.is_none() {
        state.info.display_name = super::super::completion::description_for(jsonl_path);
        state.info.display_name.clone().filter(|_| !is_new)
    } else {
        None
    };
    (id_learned, name_learned)
}

/// Apply what the read found about the subagent itself: its status, offset, dispatch and first-line metadata.
fn observe(state: &mut SubagentState, change: &Change, new_offset: u64, meta: Option<JsonlMeta>) {
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
    // in fact finished, its own `result` event sets `Completed` (`take_events`);
    // if the parent turn really is idle, the next reconcile pass
    // re-abandons it.
    if change.live && state.info.status == SubAgentStatus::Abandoned {
        state.info.status = SubAgentStatus::Active;
    }

    state.file_offset = new_offset;
    state.info.dispatch_id = change.dispatch_id.clone();

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
}

/// Append the new events (keeping the newest) and say whether the last one completed the subagent.
fn take_events(state: &mut SubagentState, new_events: &[SubagentEvent]) -> bool {
    // Process events
    let mut completed = false;
    for event in new_events {
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
    // workflow-coalescing flush in `announce_completion`.
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
    completed
}
