// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! File-based cross-instance agent registry.
//!
//! Each AgentMux instance writes agent registrations to
//! `{data_dir}/agents/{agent_id}.json`. When a local inject fails with
//! "agent not found", the inject handler looks up this registry and
//! HTTP-forwards the request to the owning instance.
//!
//! Lifecycle:
//! - Register: write file (on HTTP register endpoint + shell auto-register)
//! - Unregister: delete file (on HTTP unregister endpoint + process exit)
//! - Cleanup: TTL-based removal of stale files at startup

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::now_unix_millis;

/// One entry per registered agent in the shared data dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub agent_id: String,
    /// Local HTTP URL of the owning AgentMux instance (e.g. http://127.0.0.1:PORT).
    pub local_url: String,
    pub block_id: String,
    /// OS PID of the owning agentmuxsrv-rs process.
    pub pid: u32,
    /// Unix milliseconds of last update.
    pub updated_at: u64,
}

fn agents_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agents")
}

fn agent_path(data_dir: &Path, agent_id: &str) -> PathBuf {
    // Sanitize: only allow alphanumeric, dash, underscore to prevent path traversal.
    let safe: String = agent_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    agents_dir(data_dir).join(format!("{}.json", safe))
}

/// Write (create or update) an agent entry in the shared registry.
pub fn write(data_dir: &Path, agent_id: &str, local_url: &str, block_id: &str) {
    let dir = agents_dir(data_dir);
    let _ = std::fs::create_dir_all(&dir);
    let entry = AgentEntry {
        agent_id: agent_id.to_string(),
        local_url: local_url.to_string(),
        block_id: block_id.to_string(),
        pid: std::process::id(),
        updated_at: now_unix_millis(),
    };
    if let Ok(json) = serde_json::to_string(&entry) {
        let _ = std::fs::write(agent_path(data_dir, agent_id), json);
    }
}

/// Remove an agent entry from the shared registry.
pub fn remove(data_dir: &Path, agent_id: &str) {
    let _ = std::fs::remove_file(agent_path(data_dir, agent_id));
}

/// Look up an agent entry. Returns None if not found or file is malformed.
pub fn lookup(data_dir: &Path, agent_id: &str) -> Option<AgentEntry> {
    let content = std::fs::read_to_string(agent_path(data_dir, agent_id)).ok()?;
    serde_json::from_str(&content).ok()
}

/// Remove stale entries at startup.
///
/// An entry is considered stale if `updated_at` is older than `max_age_ms`.
/// The default is 4 hours — well beyond any reasonable agent session.
/// Entries are also removed if their JSON is malformed.
pub fn cleanup_stale(data_dir: &Path, max_age_ms: u64) {
    let dir = agents_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let cutoff = now_unix_millis().saturating_sub(max_age_ms);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        match serde_json::from_str::<AgentEntry>(&content) {
            Ok(agent) if agent.updated_at >= cutoff => {} // still fresh
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}
