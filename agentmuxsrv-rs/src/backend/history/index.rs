// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory session index built from adapter discovery.

use std::collections::HashMap;
use std::sync::Mutex;

use super::adapter::*;

/// In-memory index of discovered sessions.
pub struct SessionIndex {
    /// session_id -> SessionMeta
    sessions: Mutex<HashMap<String, SessionMeta>>,
    /// Adapters for all registered providers
    adapters: Vec<Box<dyn HistoryAdapter>>,
}

impl SessionIndex {
    pub fn new(adapters: Vec<Box<dyn HistoryAdapter>>) -> Self {
        SessionIndex {
            sessions: Mutex::new(HashMap::new()),
            adapters,
        }
    }

    /// Full scan: discover all files and extract metadata.
    /// Returns (discovered, updated, new) counts.
    pub fn refresh(&self) -> (u32, u32, u32) {
        let mut discovered: u32 = 0;
        let mut updated: u32 = 0;
        let mut new_count: u32 = 0;

        let mut new_sessions: HashMap<String, SessionMeta> = HashMap::new();

        for adapter in &self.adapters {
            let files = match adapter.discover_files() {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(
                        "history: failed to discover {} files: {}",
                        adapter.provider(),
                        e
                    );
                    continue;
                }
            };

            discovered += files.len() as u32;

            for file in &files {
                match adapter.extract_meta(&file.file_path) {
                    Ok(Some(meta)) => {
                        new_sessions.insert(meta.session_id.clone(), meta);
                    }
                    Ok(None) => {} // empty/invalid session
                    Err(e) => {
                        tracing::debug!(
                            "history: failed to extract meta from {}: {}",
                            file.file_path,
                            e
                        );
                    }
                }
            }
        }

        // Compare with existing index
        let mut sessions = self.sessions.lock().unwrap();
        for (id, _meta) in &new_sessions {
            if sessions.contains_key(id) {
                updated += 1;
            } else {
                new_count += 1;
            }
        }

        *sessions = new_sessions;

        (discovered, updated, new_count)
    }

    /// List sessions with pagination and optional filters.
    pub fn list(
        &self,
        provider: Option<&str>,
        project: Option<&str>,
        offset: usize,
        limit: usize,
        sort_by: &str,
        sort_dir: &str,
    ) -> (Vec<SessionMeta>, u32, bool) {
        let sessions = self.sessions.lock().unwrap();

        let mut filtered: Vec<&SessionMeta> = sessions
            .values()
            .filter(|s| {
                if let Some(p) = provider {
                    if s.provider != p {
                        return false;
                    }
                }
                if let Some(proj) = project {
                    if !s.working_directory.contains(proj) {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort
        let desc = sort_dir != "asc";
        match sort_by {
            "created_at" | "created" => {
                filtered.sort_by(|a, b| {
                    if desc {
                        b.created_at.cmp(&a.created_at)
                    } else {
                        a.created_at.cmp(&b.created_at)
                    }
                });
            }
            "messages" => {
                filtered.sort_by(|a, b| {
                    if desc {
                        b.message_count.cmp(&a.message_count)
                    } else {
                        a.message_count.cmp(&b.message_count)
                    }
                });
            }
            "tokens" => {
                filtered.sort_by(|a, b| {
                    if desc {
                        b.total_tokens.cmp(&a.total_tokens)
                    } else {
                        a.total_tokens.cmp(&b.total_tokens)
                    }
                });
            }
            _ => {
                // Default: modified_at desc
                filtered.sort_by(|a, b| {
                    if desc {
                        b.modified_at.cmp(&a.modified_at)
                    } else {
                        a.modified_at.cmp(&b.modified_at)
                    }
                });
            }
        }

        let total = filtered.len() as u32;
        let has_more = offset + limit < filtered.len();
        let page: Vec<SessionMeta> = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        (page, total, has_more)
    }

    /// Get a session by ID — returns just the meta from index.
    pub fn get_meta(&self, session_id: &str) -> Option<SessionMeta> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(session_id).cloned()
    }

    /// Full parse of a session by ID.
    pub fn get_full(&self, session_id: &str) -> Result<Option<HistorySession>, HistoryError> {
        let meta = match self.get_meta(session_id) {
            Some(m) => m,
            None => return Ok(None),
        };

        // Find the adapter for this provider
        for adapter in &self.adapters {
            if adapter.provider() == meta.provider {
                return adapter.parse_file(&meta.file_path);
            }
        }

        Err(HistoryError::Other(format!(
            "no adapter for provider: {}",
            meta.provider
        )))
    }

    /// Check if the index has been populated.
    pub fn is_empty(&self) -> bool {
        self.sessions.lock().unwrap().is_empty()
    }
}
