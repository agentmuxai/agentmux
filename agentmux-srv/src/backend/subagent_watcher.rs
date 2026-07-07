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
    pub last_event_at: u64,
    pub status: SubagentStatus,
    pub event_count: usize,
    pub model: Option<String>,
    /// Some("wf_<id>") when this subagent runs inside a Workflow tool run
    /// (JSONL under `subagents/workflows/<id>/`); None for direct subagents.
    pub workflow_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubagentStatus {
    Active,
    Completed,
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

        // Set up filesystem watcher
        let tx_clone = tx.clone();
        let watched_dir = if projects_dir.exists() {
            projects_dir.clone()
        } else {
            // Watch parent (config_dir) until projects/ appears
            config_dir.clone()
        };

        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(event) => {
                    let dominated = matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    );
                    if dominated {
                        for path in event.paths {
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

        // Scan for any existing subagent files
        self.scan_existing_subagents(agent_id, parent_block_id, &projects_dir);

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

    // ── Internal methods ──────────────────────────────────────────────────

    /// Scan for existing subagent JSONL files in a projects directory.
    fn scan_existing_subagents(&self, parent_agent: &str, parent_block_id: &str, projects_dir: &Path) {
        if !projects_dir.exists() {
            return;
        }

        let walker = match std::fs::read_dir(projects_dir) {
            Ok(w) => w,
            Err(_) => return,
        };

        for entry in walker.flatten() {
            // subagents/ sits directly under the project dir, or one level
            // deeper under a session dir (projects/<ws>/<session>/subagents).
            let mut candidates = vec![entry.path().join("subagents")];
            if let Ok(children) = std::fs::read_dir(entry.path()) {
                for child in children.flatten() {
                    candidates.push(child.path().join("subagents"));
                }
            }
            for subagents_dir in candidates {
                if subagents_dir.is_dir() {
                    self.scan_subagents_dir(parent_agent, parent_block_id, &subagents_dir);
                }
            }
        }
    }

    /// Process agent-*.jsonl directly in `dir`, plus workflow runs under
    /// `dir/workflows/<wf>/` (member agent files + journal).
    fn scan_subagents_dir(&self, parent_agent: &str, parent_block_id: &str, dir: &Path) {
        if let Ok(files) = std::fs::read_dir(dir) {
            for file in files.flatten() {
                let path = file.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("agent-") && name.ends_with(".jsonl") {
                        self.process_jsonl_change(parent_agent, parent_block_id, &path);
                    }
                }
            }
        }
        let workflows_dir = dir.join("workflows");
        if let Ok(runs) = std::fs::read_dir(&workflows_dir) {
            for run in runs.flatten() {
                if let Ok(files) = std::fs::read_dir(run.path()) {
                    for file in files.flatten() {
                        let path = file.path();
                        match path.file_name().and_then(|n| n.to_str()) {
                            Some(name)
                                if name.starts_with("agent-") && name.ends_with(".jsonl") =>
                            {
                                self.process_jsonl_change(parent_agent, parent_block_id, &path);
                            }
                            Some("journal.jsonl") => {
                                self.process_journal_change(parent_agent, parent_block_id, &path);
                            }
                            _ => {}
                        }
                    }
                }
            }
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
            let state = session.subagents.entry(agent_id.clone()).or_insert_with(|| {
                SubagentState {
                    info: SubagentInfo {
                        agent_id: agent_id.clone(),
                        slug: String::new(),
                        jsonl_path: jsonl_path.to_string_lossy().to_string(),
                        parent_agent: parent_agent.to_string(),
                        parent_block_id: parent_block_id.to_string(),
                        session_id: session_id.clone(),
                        last_event_at: now_millis(),
                        status: SubagentStatus::Active,
                        event_count: 0,
                        model: None,
                        workflow_id: None,
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

            // Check last event for result type (completion)
            if let Some(last) = new_events.last() {
                if matches!(&last.event_type, SubagentEventType::Text { content } if content == "Subagent completed") {
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
            Some(SubagentEventType::Text { content })
        }
        _ => None,
    }
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

/// Derive the Claude Code config directory for a host agent.
pub fn derive_claude_config_dir(agent_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let config_dir = home
        .join(".config")
        .join(format!("claude-{}", agent_id.to_lowercase()));
    Some(config_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_watcher() -> SubagentWatcher {
        SubagentWatcher::new(Arc::new(EventBus::new()))
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
                last_event_at: 0,
                status: SubagentStatus::Active,
                event_count: 0,
                model: None,
                workflow_id: None,
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
