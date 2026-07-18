// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Subagent watcher: monitors Claude Code session directories for subagent
//! JSONL files and broadcasts activity events to WebSocket clients.
//!
//! Claude Code spawns "subagents" via the Task tool. Each subagent writes its
//! conversation to a JSONL file under:
//!   `<claude-config>/projects/<encoded-workspace>/subagents/agent-<id>.jsonl`
//!
//! A Workflow tool run nests its member agents one level deeper, under a
//! per-run directory, and adds a run-level journal:
//!   `<claude-config>/projects/<encoded-workspace>/subagents/workflows/<run-id>/agent-<id>.jsonl`
//!   `<claude-config>/projects/<encoded-workspace>/subagents/workflows/<run-id>/journal.jsonl`
//!
//! This module watches those directories and emits:
//!   - `subagent:spawned`   — new subagent JSONL file detected
//!   - `subagent:activity`  — new events appended to a subagent file
//!   - `subagent:completed` — subagent finished (result event seen)
//!   - `workflow:updated`   — a workflow run's member count or journal
//!     `started`/`result` tally changed (see `WorkflowInfo`)

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

use super::eventbus::{EventBus, WSEventType, WS_EVENT_RPC};

// ── Public types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub agent_id: String,
    pub slug: String,
    pub jsonl_path: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    /// Unix ms when this subagent was first observed (set once, at
    /// creation, never updated) — distinct from `last_event_at`, which
    /// advances on every journal read. Needed by the frontend's activity
    /// dock to render an elapsed timer / sort by spawn recency.
    pub spawned_at: u64,
    pub last_event_at: u64,
    pub status: SubagentStatus,
    pub event_count: usize,
    pub model: Option<String>,
    /// Some("wf_<id>") when this subagent runs inside a Workflow tool run
    /// (JSONL under `subagents/workflows/<id>/`); None for direct subagents.
    pub workflow_id: Option<String>,
    /// Concise Haiku-generated name, set once on-demand when a client first
    /// expands this subagent (see `subagent.GenerateName`). None until then
    /// — callers fall back to `slug`/`agent_id` themselves.
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubagentStatus {
    Active,
    Completed,
    /// The parent block's turn ended without a `Result` line ever appearing
    /// for this subagent — it crashed, was killed, or was interrupted by an
    /// app/srv restart mid-task. Distinct from `Completed`: the subagent
    /// didn't finish, it was cut off. Set only by
    /// `reconcile_stale_subagents`, never by `process_jsonl_change`'s
    /// normal event processing. See
    /// docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md.
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEvent {
    pub agent_id: String,
    pub event_type: SubagentEventType,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubagentEventType {
    Text { content: String },
    ToolUse { name: String, input_summary: String },
    ToolResult { is_error: bool, preview: String },
    Progress { output: String },
    /// A JSONL `"result"`-typed line — the subagent's final output. Kept
    /// distinct from `Text` so completion detection can key off the
    /// discriminant directly instead of matching derived text content
    /// (see `process_jsonl_change`'s completion check).
    Result { content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInfo {
    pub workflow_id: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    /// Agents launched, per the run's journal `started` records (falls back
    /// to the count of member JSONL files seen when the journal lags).
    pub agents_total: usize,
    /// Agents finished, per the journal's `result` records.
    pub agents_done: usize,
    pub status: WorkflowStatus,
    pub last_event_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowStatus {
    Running,
    Completed,
}

// ── Internal state ────────────────────────────────────────────────────────

struct SessionWatch {
    subagents: HashMap<String, SubagentState>,
}

/// Cap on `SubagentState.events` — without it, a long-running subagent (or a
/// long-lived srv process accumulating many subagents) grows this Vec
/// unboundedly; `info.event_count` still tracks the true total separately
/// (mirrors `wps.rs`'s `arr_total_adds` vs. capped `PersistEventWrap.events`).
/// `get_history`'s `limit` is a request ceiling, not a guarantee — this is
/// the hard ceiling on what's retained to serve it from.
const MAX_SUBAGENT_EVENTS: usize = 2048;

/// Cap on how many `agent-*.jsonl` files `scan_subagents_dir` will replay in
/// one cold backfill (pane reopen / srv restart) — see that function's doc
/// comment. 200 comfortably covers a real reopen's "what happened recently"
/// use case while bounding the worst case to a fixed, small cost regardless
/// of how many workflow runs a long-lived session has accumulated.
const BACKFILL_MAX_FILES: usize = 200;

struct SubagentState {
    info: SubagentInfo,
    file_offset: u64,
    events: Vec<SubagentEvent>,
}

struct WorkflowState {
    info: WorkflowInfo,
    journal_offset: u64,
    /// Journal-sourced counters. `agents_total` in the public info is
    /// max(journal_started, member files seen) since either side can lag.
    journal_started: usize,
    journal_results: usize,
    member_files: usize,
    members_completed: usize,
}

#[allow(dead_code)]
struct WatchedAgent {
    agent_id: String,
    config_dir: PathBuf,
    _watcher: RecommendedWatcher,
}

// ── SubagentWatcher ───────────────────────────────────────────────────────

pub struct SubagentWatcher {
    event_bus: Arc<EventBus>,
    sessions: Mutex<HashMap<String, SessionWatch>>,
    watched_agents: Mutex<Vec<WatchedAgent>>,
    workflows: Mutex<HashMap<String, WorkflowState>>,
}

impl SubagentWatcher {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            event_bus,
            sessions: Mutex::new(HashMap::new()),
            watched_agents: Mutex::new(Vec::new()),
            workflows: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new SubagentWatcher and return it wrapped in Arc.
    pub fn spawn(event_bus: Arc<EventBus>) -> Arc<Self> {
        let watcher = Arc::new(Self::new(event_bus));
        tracing::info!("subagent watcher initialized");
        watcher
    }

    /// Start watching a Claude Code agent's session directory for subagent files.
    /// Spawns a background tokio task for debounced file event processing.
    ///
    /// `parent_block_id` is the pane/block that owns this Claude instance (from
    /// the reactive register request). It is stamped onto every emitted subagent
    /// event so the frontend can route the ⚡ panel to the originating pane only,
    /// instead of every agent pane rendering every subagent globally.
    ///
    /// Note: the watcher dedupes by `agent_id`, so if the same agent_id is
    /// registered from two blocks, events carry the first registrant's block id.
    /// That edge case (same instance name in two panes) is rare; the common
    /// leak — a terminal Claude's subagents showing up in unrelated agent panes —
    /// is fully fixed because the terminal block id never matches an agent pane.
    pub fn watch_agent(self: &Arc<Self>, agent_id: &str, parent_block_id: &str, config_dir: PathBuf) {
        // Derive the projects directory where Claude stores session data
        let projects_dir = config_dir.join("projects");
        if !projects_dir.exists() {
            tracing::debug!(
                agent = %agent_id,
                dir = %projects_dir.display(),
                "projects dir does not exist yet, will watch when created"
            );
        }

        // Check if already watching this agent
        {
            let watched = self.watched_agents.lock().unwrap();
            if watched.iter().any(|w| w.agent_id == agent_id) {
                tracing::debug!(agent = %agent_id, "already watching this agent");
                return;
            }
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();

        // Set up filesystem watcher. `watch_agent` is called from the
        // reactive-register handshake, which fires as soon as the CLI's hook
        // reaches AgentMux — well before the CLI itself has necessarily
        // created its `CLAUDE_CONFIG_DIR` on disk (a live trace showed a 47s
        // gap between register and the persistent process actually
        // spawning). Watching a path that doesn't exist yet fails outright
        // with no retry, permanently disabling subagent tracking for that
        // agent's whole session — the observed cause of subagents silently
        // never appearing in Swarm. Fall back to the nearest EXISTING
        // ancestor (typically `~/.config`, effectively always present) so
        // the watch succeeds immediately and the recursive mode picks up
        // `config_dir`/`projects_dir` once the CLI creates them.
        let watched_dir = if projects_dir.exists() {
            projects_dir.clone()
        } else if config_dir.exists() {
            config_dir.clone()
        } else {
            match nearest_existing_ancestor(&config_dir) {
                Some(dir) => {
                    tracing::info!(
                        agent = %agent_id,
                        config_dir = %config_dir.display(),
                        watching = %dir.display(),
                        "config dir does not exist yet — watching nearest existing ancestor instead"
                    );
                    dir
                }
                None => {
                    tracing::warn!(
                        agent = %agent_id,
                        config_dir = %config_dir.display(),
                        "no existing ancestor found for config dir — cannot watch for subagents"
                    );
                    return;
                }
            }
        };

        // Filters events to this agent's config dir regardless of which
        // directory ended up watched — a no-op when watching config_dir or
        // projects_dir directly (both are already under config_dir), but
        // essential when watching a shared ancestor above: without it,
        // every other agent's subagent files under that same ancestor would
        // be misattributed to this agent_id.
        let config_dir_filter = config_dir.clone();
        let tx_clone = tx.clone();

        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(event) => {
                    let dominated = matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    );
                    if dominated {
                        for path in event.paths {
                            if !path.starts_with(&config_dir_filter) {
                                continue;
                            }
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                let is_subagent =
                                    name.starts_with("agent-") && name.ends_with(".jsonl");
                                // Workflow run journals live at
                                // subagents/workflows/<wf>/journal.jsonl and drive
                                // workflow-level progress counters.
                                let is_journal = name == "journal.jsonl";
                                if is_subagent || is_journal {
                                    let _ = tx_clone.send(path);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "subagent filesystem watcher error");
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    agent = %agent_id,
                    error = %e,
                    "failed to create subagent file watcher"
                );
                return;
            }
        };

        if let Err(e) = watcher.watch(&watched_dir, RecursiveMode::Recursive) {
            tracing::warn!(
                agent = %agent_id,
                dir = %watched_dir.display(),
                error = %e,
                "failed to watch directory for subagents"
            );
            return;
        }

        tracing::info!(
            agent = %agent_id,
            dir = %watched_dir.display(),
            "watching for subagent JSONL files"
        );

        // Store the watcher handle to keep it alive
        {
            let mut watched = self.watched_agents.lock().unwrap();
            watched.push(WatchedAgent {
                agent_id: agent_id.to_string(),
                config_dir: config_dir.clone(),
                _watcher: watcher,
            });
        }

        // Deliberately NOT scanning history here. `watch_agent` runs at
        // reactive-register time — before this agent identity's session for
        // *this pane* is known — and `projects_dir` covers every project
        // this agent identity has EVER worked in, across every past
        // session. A blind scan here used to flood the Swarm view with
        // every subagent this identity had ever spawned, in every project,
        // the moment any pane for it was reopened (observed: 20 old
        // sessions, 4-18 subagent files each, all appearing at once). A
        // brand-new session has nothing to backfill — subagents will be
        // picked up live, correctly, as the Task tool spawns them. A
        // RESUMED session's own prior subagents (still legitimately
        // relevant) are scoped in via `scan_session_subagents`, called from
        // `handle_reactive_register` when the block's persisted
        // `agent:sessionid` meta says which exact session this pane is
        // resuming. See
        // docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md.

        // Spawn async task to process file change notifications
        let self_clone = Arc::clone(self);
        let parent_agent = agent_id.to_string();
        let parent_block_id = parent_block_id.to_string();
        tokio::spawn(async move {
            loop {
                let path = match rx.recv().await {
                    Some(p) => p,
                    None => {
                        tracing::info!(
                            agent = %parent_agent,
                            "subagent watcher channel closed"
                        );
                        break;
                    }
                };

                // Debounce: drain additional events within 200ms
                tokio::time::sleep(Duration::from_millis(200)).await;
                let mut paths = vec![path];
                while let Ok(p) = rx.try_recv() {
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }

                for changed_path in paths {
                    let is_journal = changed_path.file_name().and_then(|n| n.to_str())
                        == Some("journal.jsonl");
                    if is_journal {
                        self_clone.process_journal_change(
                            &parent_agent,
                            &parent_block_id,
                            &changed_path,
                        );
                    } else {
                        self_clone.process_jsonl_change(
                            &parent_agent,
                            &parent_block_id,
                            &changed_path,
                        );
                    }
                }
            }
        });
    }

    /// Stop watching an agent: drop its filesystem watcher, which closes the
    /// debounce channel — so the processing task self-terminates on the next
    /// `rx.recv()` returning `None`. Idempotent: a no-op if the agent isn't
    /// currently watched.
    ///
    /// Without this, `watched_agents` was push-only: every distinct agent that
    /// ever ran leaked one OS watch handle + channel + idle task for the rest of
    /// the process lifetime, even after its pane/agent was deleted.
    ///
    /// Also prunes `sessions`: every subagent whose `info.parent_agent` is
    /// this agent (across all sessions — subagents are keyed by session_id,
    /// not by parent), and any session left with no subagents afterward.
    /// The parent agent is gone, so nothing can query this data again — it
    /// was previously left as plain data forever, growing `sessions` by one
    /// entry set per distinct agent that ever ran a subagent.
    pub fn unwatch_agent(&self, agent_id: &str) {
        let mut watched = self.watched_agents.lock().unwrap();
        let before = watched.len();
        watched.retain(|w| w.agent_id != agent_id);
        if watched.len() != before {
            tracing::info!(agent = %agent_id, "stopped watching subagent dir");
        }
        drop(watched);

        let mut sessions = self.sessions.lock().unwrap();
        let mut pruned_subagents = 0usize;
        sessions.retain(|_session_id, session| {
            let before = session.subagents.len();
            session
                .subagents
                .retain(|_agent_id, state| state.info.parent_agent != agent_id);
            pruned_subagents += before - session.subagents.len();
            !session.subagents.is_empty()
        });
        if pruned_subagents > 0 {
            tracing::debug!(
                agent = %agent_id,
                pruned_subagents,
                "pruned subagent session state for unwatched agent"
            );
        }
    }

    /// List all subagents across all sessions (sync — safe to call from RPC dispatch).
    pub fn list_active(&self) -> Vec<SubagentInfo> {
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

    /// List all tracked workflow runs (sync — safe to call from RPC dispatch).
    pub fn list_workflows(&self) -> Vec<WorkflowInfo> {
        let mut workflows = self.workflows.lock().unwrap();
        let mut result: Vec<WorkflowInfo> = workflows
            .values_mut()
            .map(|state| {
                Self::refresh_workflow_status(state);
                state.info.clone()
            })
            .collect();
        result.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
        result
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
    pub fn get_info(&self, agent_id: &str) -> Option<SubagentInfo> {
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
        // (workflow_id, parent_block_id) need to be logged from the same
        // locked read that set it, not a racy re-fetch.
        let mut found_context: Option<(String, String, Option<String>)> = None;
        let found = {
            let mut sessions = self.sessions.lock().unwrap();
            let mut found = false;
            for session in sessions.values_mut() {
                if let Some(state) = session.subagents.get_mut(agent_id) {
                    state.info.display_name = Some(display_name.to_string());
                    found_context = Some((
                        state.info.parent_block_id.clone(),
                        state.info.session_id.clone(),
                        state.info.workflow_id.clone(),
                    ));
                    found = true;
                    break;
                }
            }
            found
        };
        // Mutex released here — broadcast outside the lock

        if let Some((parent_block_id, session_id, workflow_id)) = &found_context {
            tracing::info!(
                agent_id = %agent_id,
                display_name = %display_name,
                parent_block_id = %parent_block_id,
                session_id = %session_id,
                workflow_id = ?workflow_id,
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

    // ── Internal methods ──────────────────────────────────────────────────

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
    /// Only reconciles on a *confirmed-idle* read (`Some(false)`) —
    /// `unwrap_or(true)` treats "no controller registered yet" or any
    /// other uncertainty as "assume active, don't touch it," matching the
    /// same conservative bias `ReconcileTurnActive` uses on the frontend
    /// (only ever promote/correct on positive evidence, never guess).
    /// Scoped to the reopen/backfill path only — see
    /// docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md
    /// Open Question 1 for why real-time (mid-session) reconciliation is a
    /// deliberate fast-follow, not this pass.
    fn reconcile_stale_subagents(&self, parent_block_id: &str, session_id: &str) {
        let parent_turn_active =
            crate::backend::blockcontroller::get_block_controller_status(parent_block_id)
                .map(|s| s.turn_active)
                .unwrap_or(true);
        if parent_turn_active {
            // reagent (PR #2143 round 1): info!, not debug! — see the note on
            // the backfill log above; the default production filter drops
            // debug-level lines, which would make this and the pass-summary
            // log below invisible in a normally-running srv.
            tracing::info!(
                parent_block_id = %parent_block_id,
                session_id = %session_id,
                "reconcile_stale_subagents: parent turn active (or unknown) — nothing to reconcile"
            );
            return;
        }

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
        let mut reconciled = 0usize;
        for state in session.subagents.values_mut() {
            if state.info.parent_block_id != parent_block_id {
                continue;
            }
            if state.info.status == SubagentStatus::Active {
                state.info.status = SubagentStatus::Abandoned;
                reconciled += 1;
                // Every field a NAME-based grouping/dedup bug needs to
                // reconstruct offline: which subagent, which workflow (if
                // any), which display_name it had already resolved (grouping
                // is keyed on this), and which block/session it's bound to.
                tracing::info!(
                    agent_id = %state.info.agent_id,
                    parent_block_id = %parent_block_id,
                    session_id = %session_id,
                    workflow_id = ?state.info.workflow_id,
                    display_name = ?state.info.display_name,
                    slug = %state.info.slug,
                    "subagent reconciled: active -> abandoned (parent turn ended)"
                );
            }
        }
        if reconciled > 0 {
            tracing::info!(
                parent_block_id = %parent_block_id,
                session_id = %session_id,
                reconciled,
                "reconcile_stale_subagents: pass complete"
            );
        }
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
            self.process_jsonl_change(parent_agent, parent_block_id, &path);
        }
        for path in journals {
            self.process_journal_change(parent_agent, parent_block_id, &path);
        }
    }

    /// Process a changed/new JSONL subagent file. Reads new lines, updates state,
    /// and broadcasts events via EventBus.
    fn process_jsonl_change(&self, parent_agent: &str, parent_block_id: &str, jsonl_path: &Path) {
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
            let state = session.subagents.entry(agent_id.clone()).or_insert_with(|| {
                SubagentState {
                    info: SubagentInfo {
                        agent_id: agent_id.clone(),
                        slug: String::new(),
                        jsonl_path: jsonl_path.to_string_lossy().to_string(),
                        parent_agent: parent_agent.to_string(),
                        parent_block_id: parent_block_id.to_string(),
                        session_id: session_id.clone(),
                        spawned_at: now_millis(),
                        last_event_at: now_millis(),
                        status: SubagentStatus::Active,
                        event_count: 0,
                        model: None,
                        workflow_id: None,
                        display_name: None,
                    },
                    file_offset: 0,
                    events: Vec::new(),
                }
            });

            state.file_offset = new_offset;
            state.info.workflow_id = workflow_id.clone();

            // Update metadata from first line if we got it
            if let Some(m) = meta {
                if !m.slug.is_empty() {
                    state.info.slug = m.slug;
                }
                if let Some(model) = m.model {
                    state.info.model = Some(model);
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

            // Check last event for result type (completion). Keyed off the
            // `Result` discriminant itself (a real `"result"`-typed JSONL
            // line), not derived text content — real Claude Code result
            // events populate `result`/`content`, so matching against the
            // "Subagent completed" placeholder (only ever produced when
            // both are absent) almost never fired.
            if let Some(last) = new_events.last() {
                if matches!(&last.event_type, SubagentEventType::Result { .. }) {
                    completed = true;
                    state.info.status = SubagentStatus::Completed;
                }
            }

            let info_snapshot = state.info.clone();
            (is_new, info_snapshot, completed)
        };
        // Mutex released here — broadcast outside the lock

        if is_new {
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
                            "workflowId": info_snapshot.workflow_id,
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
                workflow_id = ?info_snapshot.workflow_id,
                "subagent spawned"
            );
        }

        if !new_events.is_empty() {
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
                            "workflowId": info_snapshot.workflow_id,
                        }
                    }
                })),
            };
            self.event_bus.broadcast_event(&activity_event);
        }

        if completed {
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
                            "workflowId": info_snapshot.workflow_id,
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
                workflow_id = ?info_snapshot.workflow_id,
                "subagent completed"
            );
        }

        if let Some(wf_id) = workflow_id {
            self.update_workflow_membership(
                &wf_id,
                parent_agent,
                parent_block_id,
                &session_id,
                is_new,
                completed,
            );
        }
    }

    /// Fold a member subagent's lifecycle into its workflow aggregate and
    /// broadcast `workflow:updated` — but only when membership actually
    /// changed (a spawn or a completion). `process_jsonl_change` calls this
    /// unconditionally for every workflow member, including plain
    /// text/tool_use/tool_result activity ticks that carry neither flag; a
    /// workflow with several active members would otherwise broadcast a WS
    /// event on every one of those ticks even though `agentsTotal`/
    /// `agentsDone` never moved. Mirrors the `has_new_records` gate in
    /// `process_journal_change`.
    fn update_workflow_membership(
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
            let mut workflows = self.workflows.lock().unwrap();
            let state = Self::workflow_entry(
                &mut workflows,
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
            Self::refresh_workflow_info(state);
            state.info.clone()
        };
        self.broadcast_workflow_updated(&info);
    }

    /// Process a changed workflow journal (subagents/workflows/<wf>/journal.jsonl):
    /// tally new `started`/`result` records and broadcast `workflow:updated`.
    fn process_journal_change(
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
            let workflows = self.workflows.lock().unwrap();
            workflows
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
            let mut workflows = self.workflows.lock().unwrap();
            let state = Self::workflow_entry(
                &mut workflows,
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
                Self::refresh_workflow_info(state);
            }
            state.info.clone()
        };
        // Only broadcast when the counters actually moved — an offset-only
        // advance (non-started/result lines) has no observable effect on
        // WorkflowInfo, so a broadcast would just be noise.
        if has_new_records {
            self.broadcast_workflow_updated(&info);
        }
    }

    fn workflow_entry<'a>(
        workflows: &'a mut HashMap<String, WorkflowState>,
        workflow_id: &str,
        parent_agent: &str,
        parent_block_id: &str,
        session_id: &str,
    ) -> &'a mut WorkflowState {
        workflows
            .entry(workflow_id.to_string())
            .or_insert_with(|| WorkflowState {
                info: WorkflowInfo {
                    workflow_id: workflow_id.to_string(),
                    parent_agent: parent_agent.to_string(),
                    parent_block_id: parent_block_id.to_string(),
                    session_id: session_id.to_string(),
                    agents_total: 0,
                    agents_done: 0,
                    status: WorkflowStatus::Running,
                    last_event_at: now_millis(),
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
    fn refresh_workflow_info(state: &mut WorkflowState) {
        state.info.agents_total = state.journal_started.max(state.member_files);
        state.info.agents_done = state.journal_results.max(state.members_completed);
        state.info.last_event_at = now_millis();
        Self::refresh_workflow_status(state);
    }

    /// Counts-complete + 60s quiet ⇒ Completed. There is no timer: the flip
    /// happens lazily at the next event or ListWorkflows read. `started ==
    /// results` alone is not terminal — it also holds between phases of a
    /// still-running workflow, hence the quiet window.
    fn refresh_workflow_status(state: &mut WorkflowState) {
        let counts_complete = state.info.agents_total > 0
            && state.info.agents_done >= state.info.agents_total;
        let quiet = now_millis().saturating_sub(state.info.last_event_at) > 60_000;
        state.info.status = if counts_complete && quiet {
            WorkflowStatus::Completed
        } else {
            WorkflowStatus::Running
        };
    }

    fn broadcast_workflow_updated(&self, info: &WorkflowInfo) {
        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(json!({
                "command": "eventrecv",
                "data": {
                    "event": "workflow:updated",
                    "data": {
                        "workflowId": info.workflow_id,
                        "parentAgent": info.parent_agent,
                        "parentBlockId": info.parent_block_id,
                        "sessionId": info.session_id,
                        "agentsTotal": info.agents_total,
                        "agentsDone": info.agents_done,
                        "status": info.status,
                    }
                }
            })),
        };
        self.event_bus.broadcast_event(&event);
    }
}

// ── JSONL parsing ─────────────────────────────────────────────────────────

/// Metadata extracted from the first JSONL line (the subagent init record).
struct JsonlMeta {
    slug: String,
    model: Option<String>,
}

/// Read a JSONL file from a byte offset, parsing new subagent events.
/// Returns (events, new_offset, optional_meta).
fn read_jsonl_from_offset(
    path: &Path,
    offset: u64,
) -> Result<(Vec<SubagentEvent>, u64, Option<JsonlMeta>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let file_len = file.metadata().map_err(|e| format!("metadata: {e}"))?.len();

    if file_len <= offset {
        return Ok((Vec::new(), offset, None));
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek: {e}"))?;

    let mut events = Vec::new();
    let mut meta = None;
    let mut current_offset = offset;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        current_offset += line.len() as u64 + 1; // +1 for newline

        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract metadata from init/config lines
        if offset == 0 && meta.is_none() {
            if let Some(slug) = value.get("slug").and_then(|v| v.as_str()) {
                meta = Some(JsonlMeta {
                    slug: slug.to_string(),
                    model: value
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
            if meta.is_none() {
                if let Some(agent_id) = value.get("agentId").and_then(|v| v.as_str()) {
                    meta = Some(JsonlMeta {
                        slug: value
                            .get("slug")
                            .and_then(|v| v.as_str())
                            .unwrap_or(agent_id)
                            .to_string(),
                        model: value
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    });
                }
            }
        }

        let timestamp = value
            .get("timestamp")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(now_millis);

        let event_type = parse_event_type(&value);
        if let Some(et) = event_type {
            let line_agent_id = value
                .get("agentId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            events.push(SubagentEvent {
                agent_id: line_agent_id,
                event_type: et,
                timestamp,
            });
        }
    }

    Ok((events, current_offset, meta))
}

/// Read just the subagent's own initial task prompt from the first JSONL
/// line (a `"type":"user"` init record) — used by `subagent.GenerateName` as
/// the source text for the Haiku naming call. Deliberately bypasses the
/// events cache/offset machinery `read_jsonl_from_offset` uses: naming only
/// ever needs the first line, is called at most once per subagent (cached
/// via `display_name` thereafter), and must work even for a subagent whose
/// events haven't been scanned into `SubagentState` yet.
///
/// `message.content` is either a plain string or an array of content blocks
/// (mirrors the two shapes `parse_event_type`'s "assistant" arm already
/// handles) — both are accepted here.
pub(crate) fn read_task_prompt(jsonl_path: &str) -> Option<String> {
    let file = std::fs::File::open(jsonl_path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;

    let value: serde_json::Value = serde_json::from_str(first_line.trim()).ok()?;
    if value.get("type").and_then(|v| v.as_str()) != Some("user") {
        return None;
    }
    let content = value.get("message")?.get("content")?;

    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }

    if let Some(arr) = content.as_array() {
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
        let joined = texts.join("\n").trim().to_string();
        return (!joined.is_empty()).then_some(joined);
    }

    None
}

/// Parse a JSONL line into a SubagentEventType based on the `type` field.
fn parse_event_type(value: &serde_json::Value) -> Option<SubagentEventType> {
    let event_type = value.get("type").and_then(|v| v.as_str())?;

    match event_type {
        "assistant" => {
            let content = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| {
                    if let Some(arr) = c.as_array() {
                        let texts: Vec<&str> = arr
                            .iter()
                            .filter_map(|block| {
                                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    block.get("text").and_then(|t| t.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if texts.is_empty() {
                            None
                        } else {
                            Some(texts.join("\n"))
                        }
                    } else {
                        c.as_str().map(|s| s.to_string())
                    }
                })
                .unwrap_or_default();
            Some(SubagentEventType::Text { content })
        }
        "tool_use" => {
            let name = value
                .get("name")
                .or_else(|| value.get("tool_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let input_summary = value
                .get("input")
                .map(|v| {
                    let s = v.to_string();
                    if s.len() > 200 {
                        let end = s.char_indices().nth(200).map_or(s.len(), |(i, _)| i);
                        format!("{}...", &s[..end])
                    } else {
                        s
                    }
                })
                .unwrap_or_default();
            Some(SubagentEventType::ToolUse {
                name,
                input_summary,
            })
        }
        "tool_result" => {
            let is_error = value
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let preview = value
                .get("content")
                .or_else(|| value.get("output"))
                .map(|v| {
                    let s = if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    };
                    if s.len() > 500 {
                        let end = s.char_indices().nth(500).map_or(s.len(), |(i, _)| i);
                        format!("{}...", &s[..end])
                    } else {
                        s
                    }
                })
                .unwrap_or_default();
            Some(SubagentEventType::ToolResult { is_error, preview })
        }
        "progress" => {
            let output = value
                .get("output")
                .or_else(|| value.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(SubagentEventType::Progress { output })
        }
        "result" => {
            let content = value
                .get("result")
                .or_else(|| value.get("content"))
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_else(|| "Subagent completed".to_string());
            Some(SubagentEventType::Result { content })
        }
        _ => None,
    }
}

/// Modification time of `path`, or `UNIX_EPOCH` if it can't be read (a
/// vanished/permission-denied file sorts oldest — excluded first by
/// `scan_subagents_dir`'s recency cap rather than crashing the scan).
fn file_mtime(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Walk up `path`'s ancestors (parent, grandparent, ...) and return the
/// first one that already exists on disk. Used by `watch_agent` when the
/// target directory doesn't exist yet — `notify::Watcher::watch` fails
/// outright on a nonexistent path, so this finds the closest directory that
/// can actually be watched right now (a later-created descendant is still
/// picked up, since the watch is recursive).
///
/// Never walks above the user's home directory. Without a floor, a fresh
/// environment where even `~/.config` doesn't exist yet (ephemeral
/// dev/CI/container — this project's own docs describe several such setups)
/// would walk all the way to `$HOME` or further, and
/// `watcher.watch(&watched_dir, RecursiveMode::Recursive)` performs a
/// synchronous, blocking directory walk of whatever it's handed — recursing
/// the entire home directory (or beyond) from inside the async
/// `handle_reactive_register` request handler risks a long stall and, on
/// Linux, exhausting the OS-wide inotify watch-count limit. Returns `None`
/// (giving up on watching rather than risking an unbounded walk) if `path`
/// isn't under the home directory at all, or if no existing ancestor is
/// found within that bound.
fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let floor = dirs::home_dir()?;
    path.ancestors()
        .skip(1)
        .take_while(|p| p.starts_with(&floor))
        .find(|p| p.exists())
        .map(|p| p.to_path_buf())
}

/// Extract the workflow id from a path under `.../subagents/workflows/<id>/...`.
/// Returns None for direct (non-workflow) subagent files.
fn parse_workflow_id(path: &Path) -> Option<String> {
    let comps: Vec<&str> = path.iter().filter_map(|c| c.to_str()).collect();
    comps.windows(3).find_map(|w| {
        (w[0] == "subagents" && w[1] == "workflows" && !w[2].ends_with(".jsonl"))
            .then(|| w[2].to_string())
    })
}

/// Session id = the name of the directory containing `subagents/`. Workflow
/// member files are nested (`subagents/workflows/<wf>/agent-*.jsonl`), so walk
/// ancestors instead of assuming a fixed depth.
fn derive_session_id(path: &Path) -> String {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.file_name().and_then(|n| n.to_str()) == Some("subagents") {
            return d
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }
        dir = d.parent();
    }
    "unknown".to_string()
}

/// Count new `started` / `result` records in a workflow journal from `offset`.
/// Returns (started, results, new_offset).
///
/// Reads line-by-line via `read_until(b'\n', ..)` rather than `BufRead::lines()`
/// so a trailing line with no `\n` yet (the writer racing a partial append) is
/// never counted or consumed: `new_offset` only ever advances past complete,
/// newline-terminated lines, so the next call re-reads that partial line whole
/// once the rest of it lands, instead of seeking mid-line and silently losing
/// the record's leading bytes (and the record itself, since the resulting
/// truncated JSON fails to parse).
fn read_journal_counts(path: &Path, offset: u64) -> Result<(usize, usize, u64), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let file_len = file.metadata().map_err(|e| format!("metadata: {e}"))?.len();
    if file_len <= offset {
        return Ok((0, 0, offset));
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek: {e}"))?;

    let (mut started, mut results) = (0usize, 0usize);
    let mut current_offset = offset;
    loop {
        let mut buf = Vec::new();
        let bytes_read = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("read_until: {e}"))?;
        if bytes_read == 0 {
            break; // EOF
        }
        if !buf.ends_with(b"\n") {
            break; // trailing partial line — leave it unconsumed for next time
        }
        current_offset += bytes_read as u64;

        let line = String::from_utf8_lossy(&buf);
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match value.get("type").and_then(|v| v.as_str()) {
            Some("started") => started += 1,
            Some("result") => results += 1,
            _ => {}
        }
    }
    Ok((started, results, current_offset))
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Utility: encode workspace path like Claude Code does ──────────────────

/// Encode a workspace path the same way Claude Code does for its projects dir.
#[allow(dead_code)]
pub fn encode_workspace_path(workspace_path: &str) -> String {
    workspace_path
        .replace('\\', "-")
        .replace('/', "-")
        .replace(':', "")
}

/// Derive the Claude Code config directory for a host agent. Only matches
/// reality for an agent with an explicit per-identity bundle override —
/// prefer `resolve_claude_config_dir` when the block's meta is available.
pub fn derive_claude_config_dir(agent_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let config_dir = home
        .join(".config")
        .join(format!("claude-{}", agent_id.to_lowercase()));
    Some(config_dir)
}

/// The authoritative Claude Code config directory for a block: the
/// `CLAUDE_CONFIG_DIR` the CLI process was actually launched with, read from
/// the block's own `cmd:env` meta (written by the launch flow before spawn,
/// from the exact same resolution the CLI process itself used — see
/// `agentmux-cef`'s `ensure_auth_dir`). Falls back to
/// `derive_claude_config_dir`'s legacy `~/.config/claude-<agent_id>` guess
/// only when `cmd:env` isn't set yet.
///
/// This distinction matters: `derive_claude_config_dir`'s guess only holds
/// for an agent with an explicit per-identity bundle override. Any agent
/// without one launches under the shared default at
/// `~/.agentmux/shared/providers/claude/`, a completely different path that
/// the guess never matches — silently disabling subagent tracking for that
/// agent forever (confirmed live: repeated "config dir does not exist yet"
/// over 38+ minutes and multiple re-registrations, for an agent that had, in
/// fact, already spawned subagents — just under the real shared path). See
/// docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md.
pub fn resolve_claude_config_dir(
    meta: &crate::backend::obj::MetaMapType,
    agent_id: &str,
) -> Option<PathBuf> {
    meta.get("cmd:env")
        .and_then(|v| v.get("CLAUDE_CONFIG_DIR"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .or_else(|| derive_claude_config_dir(agent_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_watcher() -> SubagentWatcher {
        SubagentWatcher::new(Arc::new(EventBus::new()))
    }

    /// Write a minimal terminated subagent JSONL file with an explicit mtime
    /// (`UNIX_EPOCH + offset_secs`), so backfill-ordering tests don't depend
    /// on real wall-clock write speed / filesystem timestamp resolution.
    fn write_agent_file_with_mtime(path: &Path, offset_secs: u64) {
        use std::io::Write;
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(b"{\"type\":\"result\",\"result\":\"done\"}\n").unwrap();
        f.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(offset_secs))
            .unwrap();
    }

    fn fixture_state(parent_agent: &str, agent_id: &str, session_id: &str) -> SubagentState {
        SubagentState {
            info: SubagentInfo {
                agent_id: agent_id.to_string(),
                slug: String::new(),
                jsonl_path: String::new(),
                parent_agent: parent_agent.to_string(),
                parent_block_id: String::new(),
                session_id: session_id.to_string(),
                spawned_at: 0,
                last_event_at: 0,
                status: SubagentStatus::Active,
                event_count: 0,
                model: None,
                workflow_id: None,
                display_name: None,
            },
            file_offset: 0,
            events: Vec::new(),
        }
    }

    #[test]
    fn unwatch_agent_prunes_only_matching_parent_subagents() {
        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            // Two sessions; session "s1" has subagents from two different
            // parents, session "s2" has a subagent from a third parent.
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
            s1.subagents.insert("sub-b".to_string(), fixture_state("parent-2", "sub-b", "s1"));
            sessions.insert("s1".to_string(), s1);

            let mut s2 = SessionWatch { subagents: HashMap::new() };
            s2.subagents.insert("sub-c".to_string(), fixture_state("parent-1", "sub-c", "s2"));
            sessions.insert("s2".to_string(), s2);
        }

        watcher.unwatch_agent("parent-1");

        let sessions = watcher.sessions.lock().unwrap();
        // s1: parent-1's subagent gone, parent-2's remains.
        let s1 = sessions.get("s1").expect("s1 still has parent-2's subagent, should not be dropped");
        assert!(!s1.subagents.contains_key("sub-a"));
        assert!(s1.subagents.contains_key("sub-b"));
        // s2: its only subagent belonged to parent-1, so the whole session
        // entry is pruned (not left behind as an empty HashMap).
        assert!(!sessions.contains_key("s2"), "session left with zero subagents must be removed, not left empty");
    }

    #[test]
    fn get_info_finds_a_subagent_by_id_without_scanning_the_full_list() {
        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
            s1.subagents.insert("sub-b".to_string(), fixture_state("parent-1", "sub-b", "s1"));
            sessions.insert("s1".to_string(), s1);
        }

        let found = watcher.get_info("sub-b").expect("sub-b should be found");
        assert_eq!(found.agent_id, "sub-b");
        assert_eq!(found.parent_agent, "parent-1");

        assert!(watcher.get_info("never-spawned").is_none());
    }

    #[test]
    fn set_display_name_updates_info_and_reports_found() {
        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
            sessions.insert("s1".to_string(), s1);
        }

        assert!(watcher.set_display_name("sub-a", "Refactor shell module"));
        let info = watcher.get_info("sub-a").expect("sub-a should be found");
        assert_eq!(info.display_name.as_deref(), Some("Refactor shell module"));
    }

    #[test]
    fn set_display_name_on_unknown_agent_is_noop_and_reports_not_found() {
        let watcher = fixture_watcher();
        assert!(!watcher.set_display_name("never-spawned", "Some name"));
    }

    #[test]
    fn read_task_prompt_extracts_plain_string_content_from_first_line() {
        // Pre-existing bug fixed in passing: this and its two sibling tests
        // below all shared one directory keyed on std::process::id() (constant
        // for the whole test binary, not per-test) — under parallel test
        // execution, one test's std::fs::remove_dir_all teardown could race
        // another's still-in-progress create_dir_all/write/read, producing
        // flaky failures unrelated to what each test actually exercises.
        // now_millis() (already used elsewhere in this file for the same
        // per-test-uniqueness purpose) gives each test its own directory.
        let dir = std::env::temp_dir().join(format!("amx-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let jsonl_path = dir.join("agent-prompt-string.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Analyze the shell module\"}}\n\
             {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n",
        )
        .unwrap();

        let prompt = read_task_prompt(jsonl_path.to_str().unwrap());
        assert_eq!(prompt.as_deref(), Some("Analyze the shell module"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_task_prompt_extracts_joined_text_blocks_from_content_array() {
        let dir = std::env::temp_dir().join(format!("amx-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let jsonl_path = dir.join("agent-prompt-array.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Part one\"},{\"type\":\"text\",\"text\":\"Part two\"}]}}\n",
        )
        .unwrap();

        let prompt = read_task_prompt(jsonl_path.to_str().unwrap());
        assert_eq!(prompt.as_deref(), Some("Part one\nPart two"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_task_prompt_returns_none_when_first_line_is_not_a_user_record() {
        let dir = std::env::temp_dir().join(format!("amx-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let jsonl_path = dir.join("agent-prompt-none.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        )
        .unwrap();

        assert!(read_task_prompt(jsonl_path.to_str().unwrap()).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unwatch_agent_on_unknown_agent_is_noop() {
        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            s1.subagents.insert("sub-a".to_string(), fixture_state("parent-1", "sub-a", "s1"));
            sessions.insert("s1".to_string(), s1);
        }

        watcher.unwatch_agent("never-watched");

        let sessions = watcher.sessions.lock().unwrap();
        assert!(sessions.get("s1").unwrap().subagents.contains_key("sub-a"));
    }

    #[test]
    fn subagent_events_are_capped_at_max() {
        let mut state = fixture_state("parent-1", "sub-a", "s1");
        // Simulate what process_jsonl_change's push+trim loop does, without
        // going through real JSONL files.
        for i in 0..(MAX_SUBAGENT_EVENTS + 100) {
            state.info.event_count += 1;
            state.events.push(SubagentEvent {
                agent_id: "sub-a".to_string(),
                event_type: SubagentEventType::Text { content: i.to_string() },
                timestamp: i as u64,
            });
        }
        if state.events.len() > MAX_SUBAGENT_EVENTS {
            let excess = state.events.len() - MAX_SUBAGENT_EVENTS;
            state.events.drain(..excess);
        }

        assert_eq!(state.events.len(), MAX_SUBAGENT_EVENTS);
        // event_count kept the true cumulative total despite truncation.
        assert_eq!(state.info.event_count, MAX_SUBAGENT_EVENTS + 100);
        // Oldest events were dropped — the retained window is the newest ones.
        let SubagentEventType::Text { content } = &state.events[0].event_type else {
            panic!("expected Text event");
        };
        assert_eq!(content, "100"); // first 100 (0..100) were trimmed away
    }

    #[test]
    fn parse_event_type_result_line_with_content() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"type":"result","result":"final answer"}"#).unwrap();
        let parsed = parse_event_type(&value);
        assert!(matches!(
            parsed,
            Some(SubagentEventType::Result { content }) if content == "final answer"
        ));
    }

    #[test]
    fn parse_event_type_result_line_without_content_falls_back() {
        // Real Claude Code result events always populate `result`/`content`;
        // this fallback only exists for malformed/unexpected lines.
        let value: serde_json::Value = serde_json::from_str(r#"{"type":"result"}"#).unwrap();
        let parsed = parse_event_type(&value);
        assert!(matches!(
            parsed,
            Some(SubagentEventType::Result { content }) if content == "Subagent completed"
        ));
    }

    #[test]
    fn process_jsonl_change_marks_completed_on_result_event() {
        let dir = std::env::temp_dir().join(format!("amx-subagent-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let jsonl_path = dir.join("agent-sub-a.jsonl");
        std::fs::write(
            &jsonl_path,
            concat!(
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
                "{\"type\":\"result\",\"result\":\"final answer\"}\n",
            ),
        )
        .unwrap();

        let watcher = fixture_watcher();
        watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path);

        {
            let sessions = watcher.sessions.lock().unwrap();
            let session = sessions.values().next().expect("session recorded");
            let state = session.subagents.get("sub-a").expect("subagent recorded");
            assert_eq!(state.info.status, SubagentStatus::Completed);
            assert!(matches!(
                state.events.last().unwrap().event_type,
                SubagentEventType::Result { .. }
            ));
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn process_jsonl_change_stays_active_without_result_event() {
        let dir = std::env::temp_dir().join(format!("amx-subagent-test-active-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let jsonl_path = dir.join("agent-sub-b.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"still working\"}]}}\n",
        )
        .unwrap();

        let watcher = fixture_watcher();
        watcher.process_jsonl_change("parent-1", "block-1", &jsonl_path);

        {
            let sessions = watcher.sessions.lock().unwrap();
            let session = sessions.values().next().expect("session recorded");
            let state = session.subagents.get("sub-b").expect("subagent recorded");
            assert_eq!(state.info.status, SubagentStatus::Active);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    #[test]
    fn workflow_id_from_nested_member_path() {
        let path = p("projects/ws/sess-1/subagents/workflows/wf_abc123/agent-a1.jsonl");
        assert_eq!(parse_workflow_id(&path), Some("wf_abc123".to_string()));
    }

    #[test]
    fn workflow_id_from_journal_path() {
        let path = p("projects/ws/sess-1/subagents/workflows/wf_abc123/journal.jsonl");
        assert_eq!(parse_workflow_id(&path), Some("wf_abc123".to_string()));
    }

    #[test]
    fn workflow_id_none_for_direct_subagent() {
        let path = p("projects/ws/subagents/agent-a1.jsonl");
        assert_eq!(parse_workflow_id(&path), None);
    }

    #[test]
    fn workflow_id_none_for_stray_file_in_workflows_dir() {
        let path = p("projects/ws/subagents/workflows/agent-a1.jsonl");
        assert_eq!(parse_workflow_id(&path), None);
    }

    #[test]
    fn session_id_flat_layout() {
        let path = p("projects/proj-enc/subagents/agent-a1.jsonl");
        assert_eq!(derive_session_id(&path), "proj-enc");
    }

    #[test]
    fn nearest_existing_ancestor_finds_first_existing_parent() {
        // Must live under the home dir — nearest_existing_ancestor's floor
        // (see its doc comment) would otherwise reject the whole path before
        // ever reaching `dir`. std::env::temp_dir() is NOT reliably under
        // home (e.g. plain /tmp on Linux CI runners), so build the temp path
        // from home_dir() directly.
        let home = dirs::home_dir().expect("test requires a resolvable home dir");
        let dir = home.join(format!("amx-ancestor-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();

        // dir exists; dir/a/b/c does not.
        let missing = dir.join("a").join("b").join("c");
        assert_eq!(nearest_existing_ancestor(&missing), Some(dir.clone()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nearest_existing_ancestor_returns_none_for_a_root_that_does_not_exist() {
        // A path with no ancestors at all (bare filename) has nothing to
        // walk up to — real callers always pass an absolute config dir, but
        // the function must not panic on this input.
        assert_eq!(nearest_existing_ancestor(Path::new("bare-name")), None);
    }

    /// Regression test for reagent's finding on PR #2008: without a floor,
    /// a path whose entire ancestor chain up to and including the home
    /// directory is missing would walk PAST home — risking a
    /// `notify::Watcher::watch` recursive walk of an enormous, unrelated
    /// tree. Must return None instead once the walk would have to cross the
    /// home directory boundary.
    #[test]
    fn nearest_existing_ancestor_never_walks_above_the_home_directory() {
        let home = dirs::home_dir().expect("test requires a resolvable home dir");
        // Every ancestor from `missing` up through (and including) `home`
        // is guaranteed nonexistent (home itself always exists — it's the
        // real user's home dir — so nest deep enough that none of the
        // intermediate synthetic segments exist either).
        let missing = home
            .join("amx-never-created-1")
            .join("amx-never-created-2")
            .join("amx-never-created-3");
        // `home` itself exists, so the walk finds it — proving the floor is
        // inclusive of home, not exclusive.
        assert_eq!(nearest_existing_ancestor(&missing), Some(home));
    }

    /// Regression test for the observed bug: an agent without an explicit
    /// per-identity bundle override launches under the shared default auth
    /// dir (`~/.agentmux/shared/providers/claude/`), not
    /// `derive_claude_config_dir`'s `~/.config/claude-<agent_id>` guess.
    /// `resolve_claude_config_dir` must prefer the block's real `cmd:env`
    /// over that guess whenever it's actually set.
    #[test]
    fn resolve_claude_config_dir_prefers_cmd_env_over_the_legacy_guess() {
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert(
            "cmd:env".to_string(),
            serde_json::json!({ "CLAUDE_CONFIG_DIR": "/agentmux/shared/providers/claude" }),
        );

        let resolved = resolve_claude_config_dir(&meta, "some-agent").unwrap();
        assert_eq!(resolved, PathBuf::from("/agentmux/shared/providers/claude"));
    }

    #[test]
    fn resolve_claude_config_dir_falls_back_to_the_legacy_guess_when_cmd_env_is_absent() {
        let meta = crate::backend::obj::MetaMapType::new();
        let resolved = resolve_claude_config_dir(&meta, "SomeAgent").unwrap();
        assert_eq!(resolved, derive_claude_config_dir("SomeAgent").unwrap());
    }

    #[test]
    fn resolve_claude_config_dir_falls_back_when_cmd_env_lacks_the_key() {
        let mut meta = crate::backend::obj::MetaMapType::new();
        // cmd:env is present but doesn't carry CLAUDE_CONFIG_DIR (e.g. a
        // non-Claude provider, or a race before the key is written).
        meta.insert("cmd:env".to_string(), serde_json::json!({ "OTHER_VAR": "x" }));

        let resolved = resolve_claude_config_dir(&meta, "SomeAgent").unwrap();
        assert_eq!(resolved, derive_claude_config_dir("SomeAgent").unwrap());
    }

    #[tokio::test]
    async fn watch_agent_falls_back_to_nearest_existing_ancestor_when_config_dir_is_missing() {
        // Regression test for the observed bug: watch_agent() is called from
        // the reactive-register handshake, which fires well before the CLI
        // process has created CLAUDE_CONFIG_DIR on disk. Watching a
        // nonexistent path used to fail outright with no retry, permanently
        // disabling subagent tracking for that agent's whole session.
        // Must live under the home dir — nearest_existing_ancestor's floor
        // would otherwise reject this path outright on platforms where
        // std::env::temp_dir() isn't under home (e.g. plain /tmp on Linux
        // CI runners).
        let home = dirs::home_dir().expect("test requires a resolvable home dir");
        let root = home.join(format!("amx-watch-fallback-test-{}", now_millis()));
        std::fs::create_dir_all(&root).unwrap(); // ancestor exists...
        let config_dir = root.join("claude-testagent"); // ...but this does not.
        assert!(!config_dir.exists());

        let watcher = Arc::new(fixture_watcher());
        watcher.watch_agent("test-agent", "block-1", config_dir.clone());

        // watch_agent must have succeeded (registered itself) instead of
        // bailing out — the old behavior returned early on the failed
        // notify::watch() call, before ever reaching this point.
        assert_eq!(watcher.watched_agents.lock().unwrap().len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    /// Regression test for the observed flood: reopening a pane for an
    /// agent identity that has spawned subagents across many past sessions
    /// (in this project) must only backfill the ONE session being resumed,
    /// not every session the identity has ever run.
    #[test]
    fn scan_session_subagents_only_backfills_the_named_session() {
        let config_dir = std::env::temp_dir()
            .join(format!("amx-scan-session-test-{}", now_millis()));
        let target_session = "target-session-uuid";
        let other_session = "other-session-uuid";

        let target_dir = config_dir
            .join("projects")
            .join("ws-enc")
            .join(target_session)
            .join("subagents");
        let other_dir = config_dir
            .join("projects")
            .join("ws-enc")
            .join(other_session)
            .join("subagents");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();

        std::fs::write(
            target_dir.join("agent-wanted.jsonl"),
            "{\"type\":\"result\",\"result\":\"done\"}\n",
        )
        .unwrap();
        std::fs::write(
            other_dir.join("agent-unwanted.jsonl"),
            "{\"type\":\"result\",\"result\":\"done\"}\n",
        )
        .unwrap();

        let watcher = fixture_watcher();
        watcher.scan_session_subagents("parent-1", "block-1", &config_dir, target_session);

        let active = watcher.list_active();
        assert_eq!(active.len(), 1, "only the target session's subagent should be backfilled");
        assert_eq!(active[0].agent_id, "wanted");
        assert_eq!(active[0].session_id, target_session);

        std::fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn scan_session_subagents_is_a_noop_for_an_unknown_session_id() {
        let config_dir = std::env::temp_dir()
            .join(format!("amx-scan-session-unknown-test-{}", now_millis()));
        let existing_dir = config_dir
            .join("projects")
            .join("ws-enc")
            .join("some-other-session")
            .join("subagents");
        std::fs::create_dir_all(&existing_dir).unwrap();
        std::fs::write(
            existing_dir.join("agent-a.jsonl"),
            "{\"type\":\"result\",\"result\":\"done\"}\n",
        )
        .unwrap();

        let watcher = fixture_watcher();
        watcher.scan_session_subagents("parent-1", "block-1", &config_dir, "never-existed");

        assert!(watcher.list_active().is_empty(), "unknown session id must not fall back to scanning everything");

        std::fs::remove_dir_all(&config_dir).ok();
    }

    // ── scan_subagents_dir backfill cap (docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md) ──
    //
    // A long-lived pane's subagents/ directory accumulates forever; without a
    // cap, every cold backfill (pane reopen, srv restart) replays the WHOLE
    // history — a live incident hit 1,000+ replayed files across three
    // back-to-back srv crash-restarts in under 10 seconds. These tests lock
    // in the fix: the cap applies regardless of corpus size, and it always
    // keeps the most RECENT files, not an arbitrary subset.

    #[test]
    fn scan_subagents_dir_caps_cold_backfill_to_the_most_recent_files() {
        let config_dir = std::env::temp_dir()
            .join(format!("amx-scan-backfill-cap-test-{}", now_millis()));
        let session_id = "backfill-cap-session";
        let subagents_dir = config_dir
            .join("projects")
            .join("ws-enc")
            .join(session_id)
            .join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();

        // One more file than the cap, mtimes strictly increasing by index —
        // "newest BACKFILL_MAX_FILES" is unambiguous regardless of directory
        // enumeration order.
        let total = BACKFILL_MAX_FILES + 1;
        for i in 0..total {
            let path = subagents_dir.join(format!("agent-id{i:04}.jsonl"));
            write_agent_file_with_mtime(&path, i as u64);
        }

        let watcher = fixture_watcher();
        watcher.scan_session_subagents("parent-1", "block-1", &config_dir, session_id);

        let active = watcher.list_active();
        assert_eq!(
            active.len(),
            BACKFILL_MAX_FILES,
            "cold backfill must not replay more than the cap regardless of corpus size"
        );
        assert!(
            !active.iter().any(|a| a.agent_id == "id0000"),
            "the single oldest file must be the one dropped"
        );
        assert!(
            active
                .iter()
                .any(|a| a.agent_id == format!("id{:04}", total - 1)),
            "the newest file must always survive the cap"
        );

        std::fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn scan_subagents_dir_processes_workflow_journal_even_beyond_the_member_cap() {
        let config_dir = std::env::temp_dir()
            .join(format!("amx-scan-backfill-journal-test-{}", now_millis()));
        let session_id = "backfill-journal-session";
        let run_dir = config_dir
            .join("projects")
            .join("ws-enc")
            .join(session_id)
            .join("subagents")
            .join("workflows")
            .join("wf_test-run");
        std::fs::create_dir_all(&run_dir).unwrap();

        // More member files than the cap — the cap must still apply here...
        let total = BACKFILL_MAX_FILES + 5;
        for i in 0..total {
            let path = run_dir.join(format!("agent-id{i:04}.jsonl"));
            write_agent_file_with_mtime(&path, i as u64);
        }
        // ...but the run's journal (one small file, not one per member) is
        // always processed regardless — it drives `workflow:updated`/run
        // status, which must stay accurate even when membership is capped.
        std::fs::write(
            run_dir.join("journal.jsonl"),
            "{\"type\":\"started\",\"agent_id\":\"id0000\"}\n",
        )
        .unwrap();

        let watcher = fixture_watcher();
        watcher.scan_session_subagents("parent-1", "block-1", &config_dir, session_id);

        assert_eq!(
            watcher.list_active().len(),
            BACKFILL_MAX_FILES,
            "member files are still capped inside a workflow run"
        );
        let workflows = watcher.list_workflows();
        assert_eq!(workflows.len(), 1, "the run's journal must still be processed");
        assert_eq!(workflows[0].workflow_id, "wf_test-run");

        std::fs::remove_dir_all(&config_dir).ok();
    }

    // ── reconcile_stale_subagents ─────────────────────────────────────────
    //
    // A stub `Controller` so these tests can control what
    // `get_block_controller_status` reports without spinning up a real
    // subprocess. `CONTROLLER_REGISTRY` is process-global (shared across
    // every test in this binary) — each test below registers its stub
    // under a unique, per-test block id (never a literal shared with any
    // other test) so parallel test execution can't cross-contaminate.

    struct StubController {
        block_id: String,
        turn_active: bool,
    }

    impl crate::backend::blockcontroller::Controller for StubController {
        fn start(&self, _: crate::backend::obj::MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self, _: bool, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn get_runtime_status(&self) -> crate::backend::blockcontroller::BlockControllerRuntimeStatus {
            crate::backend::blockcontroller::BlockControllerRuntimeStatus {
                blockid: self.block_id.clone(),
                turn_active: self.turn_active,
                ..Default::default()
            }
        }
        fn send_input(&self, _: crate::backend::blockcontroller::BlockInputUnion, _: Option<u64>) -> Result<(), String> {
            Ok(())
        }
        fn controller_type(&self) -> &str {
            "stub"
        }
        fn block_id(&self) -> &str {
            &self.block_id
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn register_stub_controller(block_id: &str, turn_active: bool) {
        crate::backend::blockcontroller::register_controller(
            block_id,
            Arc::new(StubController { block_id: block_id.to_string(), turn_active }),
        );
    }

    #[test]
    fn reconcile_stale_subagents_downgrades_active_to_abandoned_when_parent_turn_is_confirmed_idle() {
        let block_id = format!("recon-idle-{}", now_millis());
        register_stub_controller(&block_id, false);

        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            let mut state = fixture_state("parent-1", "sub-a", "s1");
            state.info.parent_block_id = block_id.clone();
            s1.subagents.insert("sub-a".to_string(), state);
            sessions.insert("s1".to_string(), s1);
        }

        watcher.reconcile_stale_subagents(&block_id, "s1");

        let info = watcher.get_info("sub-a").expect("sub-a should still exist");
        assert_eq!(info.status, SubagentStatus::Abandoned);
    }

    #[test]
    fn reconcile_stale_subagents_leaves_active_alone_when_parent_turn_is_active() {
        let block_id = format!("recon-active-{}", now_millis());
        register_stub_controller(&block_id, true);

        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            let mut state = fixture_state("parent-1", "sub-a", "s1");
            state.info.parent_block_id = block_id.clone();
            s1.subagents.insert("sub-a".to_string(), state);
            sessions.insert("s1".to_string(), s1);
        }

        watcher.reconcile_stale_subagents(&block_id, "s1");

        let info = watcher.get_info("sub-a").expect("sub-a should still exist");
        assert_eq!(info.status, SubagentStatus::Active, "a genuinely active parent turn must never be reconciled away");
    }

    #[test]
    fn reconcile_stale_subagents_leaves_active_alone_when_no_controller_is_registered() {
        // No register_stub_controller call — block id is guaranteed unique
        // (per-test suffix) so get_block_controller_status returns None.
        // unwrap_or(true) means "uncertain" defaults to "assume active,
        // don't touch it" — the same conservative bias as ReconcileTurnActive.
        let block_id = format!("recon-unregistered-{}", now_millis());

        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            let mut state = fixture_state("parent-1", "sub-a", "s1");
            state.info.parent_block_id = block_id.clone();
            s1.subagents.insert("sub-a".to_string(), state);
            sessions.insert("s1".to_string(), s1);
        }

        watcher.reconcile_stale_subagents(&block_id, "s1");

        let info = watcher.get_info("sub-a").expect("sub-a should still exist");
        assert_eq!(info.status, SubagentStatus::Active);
    }

    #[test]
    fn reconcile_stale_subagents_never_downgrades_an_already_completed_subagent() {
        let block_id = format!("recon-completed-{}", now_millis());
        register_stub_controller(&block_id, false);

        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            let mut state = fixture_state("parent-1", "sub-a", "s1");
            state.info.parent_block_id = block_id.clone();
            state.info.status = SubagentStatus::Completed;
            s1.subagents.insert("sub-a".to_string(), state);
            sessions.insert("s1".to_string(), s1);
        }

        watcher.reconcile_stale_subagents(&block_id, "s1");

        let info = watcher.get_info("sub-a").expect("sub-a should still exist");
        assert_eq!(info.status, SubagentStatus::Completed, "a subagent that genuinely finished must stay Completed, not be downgraded");
    }

    #[test]
    fn reconcile_stale_subagents_never_touches_a_sibling_blocks_subagent_in_the_same_session() {
        // Two blocks can both have subagents recorded under the same
        // session_id (the watcher dedupes purely by agent_id — see
        // watch_agent's doc comment). reconcile_stale_subagents only has a
        // confirmed-idle read for the ONE block it was called with; a
        // sibling block sharing that session_id could still be genuinely
        // active, so its subagent must be left alone. Reagent P1 on #2131.
        let idle_block = format!("recon-sibling-idle-{}", now_millis());
        let active_block = format!("recon-sibling-active-{}", now_millis());
        register_stub_controller(&idle_block, false);
        register_stub_controller(&active_block, true);

        let watcher = fixture_watcher();
        {
            let mut sessions = watcher.sessions.lock().unwrap();
            let mut s1 = SessionWatch { subagents: HashMap::new() };
            let mut owned = fixture_state("parent-1", "sub-owned", "s1");
            owned.info.parent_block_id = idle_block.clone();
            let mut sibling = fixture_state("parent-2", "sub-sibling", "s1");
            sibling.info.parent_block_id = active_block.clone();
            s1.subagents.insert("sub-owned".to_string(), owned);
            s1.subagents.insert("sub-sibling".to_string(), sibling);
            sessions.insert("s1".to_string(), s1);
        }

        watcher.reconcile_stale_subagents(&idle_block, "s1");

        let owned_info = watcher.get_info("sub-owned").expect("sub-owned should still exist");
        assert_eq!(owned_info.status, SubagentStatus::Abandoned, "this block's own subagent should still be reconciled");
        let sibling_info = watcher.get_info("sub-sibling").expect("sub-sibling should still exist");
        assert_eq!(sibling_info.status, SubagentStatus::Active, "a sibling block's subagent must never be reconciled by an unrelated block's idle read");
    }

    /// End-to-end: a subagent JSONL with no terminal `result` line, backfilled
    /// via a real `scan_session_subagents` call while the parent's turn is
    /// confirmed idle, comes out `Abandoned` — not `Active` forever. This is
    /// the exact user-reported symptom (SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md).
    #[test]
    fn scan_session_subagents_reconciles_an_unterminated_file_to_abandoned_when_parent_turn_is_idle() {
        let block_id = format!("recon-scan-{}", now_millis());
        register_stub_controller(&block_id, false);

        let config_dir = std::env::temp_dir()
            .join(format!("amx-scan-reconcile-test-{}", now_millis()));
        let session_id = "target-session-uuid";
        let target_dir = config_dir
            .join("projects")
            .join("ws-enc")
            .join(session_id)
            .join("subagents");
        std::fs::create_dir_all(&target_dir).unwrap();
        // No "type":"result" line — this subagent never got a terminal event,
        // simulating a crash/kill/interrupted-by-restart.
        std::fs::write(
            target_dir.join("agent-crashed.jsonl"),
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"working...\"}]}}\n",
        )
        .unwrap();

        let watcher = fixture_watcher();
        watcher.scan_session_subagents("parent-1", &block_id, &config_dir, session_id);

        let active = watcher.list_active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].agent_id, "crashed");
        assert_eq!(active[0].status, SubagentStatus::Abandoned);

        std::fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn session_id_nested_workflow_layout() {
        let path = p("projects/ws/sess-uuid/subagents/workflows/wf_x/agent-a1.jsonl");
        assert_eq!(derive_session_id(&path), "sess-uuid");
    }

    #[test]
    fn journal_counts_incremental() {
        let dir = std::env::temp_dir().join(format!("amx-journal-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let journal = dir.join("journal.jsonl");

        std::fs::write(
            &journal,
            "{\"type\":\"started\",\"agentId\":\"a1\"}\n{\"type\":\"result\",\"agentId\":\"a1\",\"result\":{}}\n",
        )
        .unwrap();
        let (started, results, offset) = read_journal_counts(&journal, 0).unwrap();
        assert_eq!((started, results), (1, 1));

        // Append two more records; re-read from the saved offset.
        let mut existing = std::fs::read(&journal).unwrap();
        existing.extend_from_slice(
            b"{\"type\":\"started\",\"agentId\":\"a2\"}\n{\"type\":\"started\",\"agentId\":\"a3\"}\n",
        );
        std::fs::write(&journal, existing).unwrap();
        let (started2, results2, _) = read_journal_counts(&journal, offset).unwrap();
        assert_eq!((started2, results2), (2, 0));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Regression test for a race where the journal writer has flushed a
    /// record's bytes but not yet its trailing `\n` (mid-`write!` on a
    /// concurrently-appended file). The unterminated line must be neither
    /// counted nor consumed — `new_offset` should sit exactly at its start —
    /// so the next read picks it up whole once the newline lands, instead of
    /// silently losing the record.
    #[test]
    fn journal_counts_skips_unterminated_trailing_line() {
        let dir = std::env::temp_dir().join(format!("amx-journal-test-partial-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let journal = dir.join("journal.jsonl");

        // One complete record, then a partial record with no trailing newline.
        let first_line = "{\"type\":\"started\",\"agentId\":\"a1\"}\n";
        let partial_line = "{\"type\":\"started\",\"agentId\":\"a2";
        std::fs::write(&journal, format!("{first_line}{partial_line}")).unwrap();

        let (started, results, offset) = read_journal_counts(&journal, 0).unwrap();
        assert_eq!((started, results), (1, 0), "partial trailing line must not be counted");
        assert_eq!(
            offset, first_line.len() as u64,
            "offset must stop at the start of the partial line, not past it"
        );

        // The writer finishes the line; a re-read from the same offset must
        // now see the complete record rather than a truncated/corrupted one.
        let mut existing = std::fs::read(&journal).unwrap();
        existing.extend_from_slice(b"\"}\n");
        std::fs::write(&journal, existing).unwrap();
        let (started2, results2, _) = read_journal_counts(&journal, offset).unwrap();
        assert_eq!((started2, results2), (1, 0), "completed line must be picked up whole, not dropped");

        std::fs::remove_dir_all(&dir).ok();
    }
}
