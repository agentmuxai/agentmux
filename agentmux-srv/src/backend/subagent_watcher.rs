//! Subagent watcher: monitors Claude Code session directories for subagent
//! JSONL files and broadcasts activity events to WebSocket clients.
//!
//! Claude Code spawns "subagents" via the Task tool. Each subagent writes its
//! conversation to a JSONL file under:
//!   `<claude-config>/projects/<encoded-workspace>/subagents/agent-<id>.jsonl`
//!
//! This module watches those directories and emits:
//!   - `subagent:spawned`   — new subagent JSONL file detected
//!   - `subagent:activity`  — new events appended to a subagent file
//!   - `subagent:completed` — subagent finished (result event seen)

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
    pub session_id: String,
    pub last_event_at: u64,
    pub status: SubagentStatus,
    pub event_count: usize,
    pub model: Option<String>,
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

// ── Internal state ────────────────────────────────────────────────────────

struct SessionWatch {
    subagents: HashMap<String, SubagentState>,
}

struct SubagentState {
    info: SubagentInfo,
    file_offset: u64,
    events: Vec<SubagentEvent>,
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
}

impl SubagentWatcher {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            event_bus,
            sessions: Mutex::new(HashMap::new()),
            watched_agents: Mutex::new(Vec::new()),
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
    pub fn watch_agent(self: &Arc<Self>, agent_id: &str, config_dir: PathBuf) {
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
                                if name.starts_with("agent-") && name.ends_with(".jsonl") {
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
        self.scan_existing_subagents(agent_id, &projects_dir);

        // Spawn async task to process file change notifications
        let self_clone = Arc::clone(self);
        let parent_agent = agent_id.to_string();
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
                    self_clone.process_jsonl_change(&parent_agent, &changed_path);
                }
            }
        });
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

    // ── Internal methods ──────────────────────────────────────────────────

    /// Scan for existing subagent JSONL files in a projects directory.
    fn scan_existing_subagents(&self, parent_agent: &str, projects_dir: &Path) {
        if !projects_dir.exists() {
            return;
        }

        let walker = match std::fs::read_dir(projects_dir) {
            Ok(w) => w,
            Err(_) => return,
        };

        for entry in walker.flatten() {
            let subagents_dir = entry.path().join("subagents");
            if subagents_dir.is_dir() {
                if let Ok(files) = std::fs::read_dir(&subagents_dir) {
                    for file in files.flatten() {
                        let path = file.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with("agent-") && name.ends_with(".jsonl") {
                                self.process_jsonl_change(parent_agent, &path);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Process a changed/new JSONL subagent file. Reads new lines, updates state,
    /// and broadcasts events via EventBus.
    fn process_jsonl_change(&self, parent_agent: &str, jsonl_path: &Path) {
        // Extract agent ID from filename: agent-<id>.jsonl
        let agent_id = match jsonl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("agent-"))
        {
            Some(id) => id.to_string(),
            None => return,
        };

        // Derive session_id from the parent directory structure
        let session_id = jsonl_path
            .parent() // subagents/
            .and_then(|p| p.parent()) // project-encoded-dir/
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

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
                        session_id: session_id.clone(),
                        last_event_at: now_millis(),
                        status: SubagentStatus::Active,
                        event_count: 0,
                        model: None,
                    },
                    file_offset: 0,
                    events: Vec::new(),
                }
            });

            state.file_offset = new_offset;

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
                            "sessionId": session_id,
                            "model": info_snapshot.model,
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
                            "newEvents": new_events.len(),
                            "totalEvents": info_snapshot.event_count,
                            "events": new_events,
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
                            "totalEvents": info_snapshot.event_count,
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
