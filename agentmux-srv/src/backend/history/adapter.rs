// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! History adapter trait and shared types.

use serde::{Deserialize, Serialize};

/// Error type for history operations.
#[derive(Debug)]
pub enum HistoryError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Other(String),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::Io(e) => write!(f, "IO error: {}", e),
            HistoryError::Json(e) => write!(f, "JSON error: {}", e),
            HistoryError::Other(s) => write!(f, "{}", s),
        }
    }
}

impl From<std::io::Error> for HistoryError {
    fn from(e: std::io::Error) -> Self {
        HistoryError::Io(e)
    }
}

impl From<serde_json::Error> for HistoryError {
    fn from(e: serde_json::Error) -> Self {
        HistoryError::Json(e)
    }
}

/// A discovered file on disk (path + modification time).
pub struct DiscoveredFile {
    pub file_path: String,
    pub mtime_ms: i64,
}

/// Lightweight metadata for the session list — extracted without full parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub file_path: String,
    pub provider: String,
    pub model: String,
    pub slug: String,
    pub working_directory: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub message_count: u32,
    pub first_user_message: String,
    pub file_size_bytes: u64,
    pub git_branch: String,
    pub total_tokens: u64,
    pub subagent_count: u32,
    /// The identity bundle id (`db_accounts.id` / the `<bundle_id>` path
    /// segment under `identities/`) this session was discovered under, if
    /// any. Empty for sessions found via a non-bundle base_dir (personal
    /// `~/.claude`, the default `<shared>/providers/claude/projects`, or
    /// legacy `~/.config/claude-*`) — those aren't attributable to a
    /// specific identity. Lets `SessionIndex` answer "this agent's
    /// sessions" in O(sessions for this identity) instead of a full scan;
    /// see `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
    /// §4.4. An agent's own id is resolved from this via
    /// `db_agent_identity_links` (`Store::agent_identity_list_for_agent`),
    /// not stored here directly — the same bundle can in principle be
    /// shared, and this field reflects the filesystem fact (which bundle),
    /// not a specific agent's claim on it.
    #[serde(default)]
    pub identity_id: String,
}

/// Full parsed session — produced on demand when user opens a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySession {
    pub meta: SessionMeta,
    pub messages: Vec<HistoryMessage>,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub timestamp: i64,
    pub tool_uses: Vec<ToolUseSummary>,
}

/// Summary of a tool call within an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseSummary {
    pub name: String,
    pub argument_summary: String,
}

/// One implementation per CLI provider.
pub trait HistoryAdapter: Send + Sync {
    /// Provider identifier (e.g., "claude", "codex", "gemini", "kimi", "openclaw", "pi").
    fn provider(&self) -> &str;

    /// Discover all session file paths on disk.
    /// Returns (file_path, mtime_ms) pairs, sorted by mtime descending.
    fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError>;

    /// Extract lightweight metadata without full parsing.
    fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError>;

    /// Parse a single session file into a full HistorySession.
    fn parse_file(&self, file_path: &str) -> Result<Option<HistorySession>, HistoryError>;
}
