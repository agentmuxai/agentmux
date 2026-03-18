// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! History module — discovers and indexes past CLI agent conversations from disk.

pub mod adapter;
pub mod claude_adapter;
pub mod index;

use std::sync::Arc;

use adapter::*;
use claude_adapter::ClaudeHistoryAdapter;
use index::SessionIndex;

/// The history service exposed to the RPC layer.
pub struct HistoryService {
    index: Arc<SessionIndex>,
}

impl HistoryService {
    pub fn new() -> Self {
        let adapters: Vec<Box<dyn HistoryAdapter>> =
            vec![Box::new(ClaudeHistoryAdapter::new())];

        HistoryService {
            index: Arc::new(SessionIndex::new(adapters)),
        }
    }

    /// List sessions with pagination and filters.
    /// Lazy-initializes the index on first call.
    pub fn list(
        &self,
        provider: Option<&str>,
        project: Option<&str>,
        offset: usize,
        limit: usize,
        sort_by: &str,
        sort_dir: &str,
    ) -> serde_json::Value {
        // Lazy init: scan on first request
        if self.index.is_empty() {
            self.index.refresh();
        }

        let (sessions, total, has_more) =
            self.index.list(provider, project, offset, limit, sort_by, sort_dir);

        serde_json::json!({
            "sessions": sessions,
            "total": total,
            "has_more": has_more,
        })
    }

    /// Get full conversation for a session.
    pub fn get(&self, session_id: &str) -> serde_json::Value {
        // Lazy init
        if self.index.is_empty() {
            self.index.refresh();
        }

        match self.index.get_full(session_id) {
            Ok(Some(session)) => serde_json::json!({ "session": session }),
            Ok(None) => serde_json::json!({ "error": "session not found" }),
            Err(e) => serde_json::json!({ "error": format!("{}", e) }),
        }
    }

    /// Re-scan disk and update the index.
    pub fn refresh(&self) -> serde_json::Value {
        let (discovered, updated, new_count) = self.index.refresh();
        serde_json::json!({
            "discovered": discovered,
            "updated": updated,
            "new": new_count,
        })
    }
}
