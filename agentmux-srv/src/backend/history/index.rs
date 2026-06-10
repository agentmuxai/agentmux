// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory session index built from adapter discovery.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::adapter::*;

/// AgentMux-ISOLATED provider-home roots under which delete/clear is permitted:
/// `<shared>/providers/` and `<shared>/identities/`. Anything outside these
/// (the user's personal `~/.claude` / `~/.config/claude-*`) is OFF-LIMITS so a
/// "clear all" can never nuke transcripts AgentMux didn't create.
fn default_isolated_roots() -> Vec<PathBuf> {
    let shared = std::env::var_os("AGENTMUX_SHARED_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".agentmux").join("shared")));
    match shared {
        Some(s) => vec![s.join("providers"), s.join("identities")],
        None => Vec::new(),
    }
}

/// In-memory index of discovered sessions.
pub struct SessionIndex {
    /// session_id -> SessionMeta
    sessions: Mutex<HashMap<String, SessionMeta>>,
    /// Adapters for all registered providers
    adapters: Vec<Box<dyn HistoryAdapter>>,
    /// Roots under which destructive ops (delete/clear) are allowed.
    isolated_roots: Vec<PathBuf>,
}

impl SessionIndex {
    pub fn new(adapters: Vec<Box<dyn HistoryAdapter>>) -> Self {
        Self::with_isolated_roots(adapters, default_isolated_roots())
    }

    /// Construct with explicit isolated roots (used by new() and tests).
    pub fn with_isolated_roots(
        adapters: Vec<Box<dyn HistoryAdapter>>,
        isolated_roots: Vec<PathBuf>,
    ) -> Self {
        SessionIndex {
            sessions: Mutex::new(HashMap::new()),
            adapters,
            isolated_roots,
        }
    }

    /// True if `path` lives under an AgentMux-isolated provider home and is
    /// therefore safe to delete. Personal global homes are never isolated.
    fn is_isolated(&self, path: &Path) -> bool {
        self.isolated_roots.iter().any(|r| path.starts_with(r))
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

    /// Delete a session: remove its on-disk transcript (and the sibling
    /// `<session_id>/` subagents dir Claude keeps next to it) and drop it from
    /// the index. Returns Ok(true) if a file was removed, Ok(false) if the
    /// session id is unknown.
    pub fn delete(&self, session_id: &str) -> Result<bool, HistoryError> {
        let meta = match self.get_meta(session_id) {
            Some(m) => m,
            None => return Ok(false),
        };
        let path = Path::new(&meta.file_path);
        // Safety: only delete inside AgentMux-isolated homes — never the user's
        // personal Claude/Codex/... transcripts that the browse also surfaces.
        if !self.is_isolated(path) {
            return Err(HistoryError::Other(format!(
                "refusing to delete '{}' — it lives in your personal {} home, not AgentMux's",
                meta.file_path, meta.provider
            )));
        }
        // Treat an already-removed file (GC'd out-of-band) as success so the
        // stale index entry still gets dropped (otherwise clear-all can't
        // converge and the index stays non-empty).
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        // Claude stores subagent transcripts in a sibling `<session_id>/` dir;
        // remove it too so the clear actually frees the space. Best-effort.
        if let Some(parent) = path.parent() {
            let sidecar = parent.join(session_id);
            if sidecar.is_dir() {
                let _ = std::fs::remove_dir_all(&sidecar);
            }
        }
        self.sessions.lock().unwrap().remove(session_id);
        Ok(true)
    }

    /// Bulk-delete all indexed sessions matching the optional provider/project
    /// filter (no filter = clear everything). Returns the number removed.
    pub fn clear(&self, provider: Option<&str>, project: Option<&str>) -> u32 {
        // Snapshot matching ids while holding the lock, then do fs ops without
        // it (delete() re-locks per id).
        let ids: Vec<String> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .values()
                .filter(|s| provider.map_or(true, |p| s.provider == p))
                .filter(|s| project.map_or(true, |proj| s.working_directory.contains(proj)))
                // Never bulk-delete the user's personal global transcripts.
                .filter(|s| self.is_isolated(Path::new(&s.file_path)))
                .map(|s| s.session_id.clone())
                .collect()
        };
        let mut deleted = 0;
        for id in ids {
            if matches!(self.delete(&id), Ok(true)) {
                deleted += 1;
            }
        }
        deleted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Adapter that "discovers" caller-supplied files and derives the session id
    /// from each file stem. Just enough to populate the index for delete tests.
    struct MockAdapter {
        files: Vec<DiscoveredFile>,
        provider: String,
        working_directory: String,
    }
    impl HistoryAdapter for MockAdapter {
        fn provider(&self) -> &str {
            &self.provider
        }
        fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
            Ok(self
                .files
                .iter()
                .map(|f| DiscoveredFile { file_path: f.file_path.clone(), mtime_ms: f.mtime_ms })
                .collect())
        }
        fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError> {
            let id = PathBuf::from(file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(Some(SessionMeta {
                session_id: id,
                file_path: file_path.to_string(),
                provider: self.provider.clone(),
                model: String::new(),
                slug: String::new(),
                working_directory: self.working_directory.clone(),
                created_at: 0,
                modified_at: 0,
                message_count: 0,
                first_user_message: String::new(),
                file_size_bytes: 0,
                git_branch: String::new(),
                total_tokens: 0,
                subagent_count: 0,
            }))
        }
        fn parse_file(&self, _: &str) -> Result<Option<HistorySession>, HistoryError> {
            Ok(None)
        }
    }

    fn write_session(dir: &std::path::Path, id: &str) -> String {
        let f = dir.join(format!("{id}.jsonl"));
        fs::write(&f, b"{}").unwrap();
        f.to_string_lossy().into_owned()
    }

    #[test]
    fn delete_removes_file_and_index_entry() {
        let dir = std::env::temp_dir().join(format!("amux-hist-del-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let fp = write_session(&dir, "sess-abc");
        // A sibling subagents dir should be removed too.
        let sidecar = dir.join("sess-abc");
        fs::create_dir_all(sidecar.join("subagents")).unwrap();

        let idx = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: fp.clone(), mtime_ms: 0 }],
                provider: "mock".into(),
                working_directory: "/proj".into(),
            })],
            vec![dir.clone()],
        );
        idx.refresh();
        assert!(idx.get_meta("sess-abc").is_some());

        assert!(matches!(idx.delete("sess-abc"), Ok(true)));
        assert!(!std::path::Path::new(&fp).exists(), "transcript removed");
        assert!(!sidecar.exists(), "subagents sidecar removed");
        assert!(idx.get_meta("sess-abc").is_none(), "dropped from index");
        assert!(matches!(idx.delete("sess-abc"), Ok(false)), "second delete is a no-op");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_respects_provider_filter() {
        let dir = std::env::temp_dir().join(format!("amux-hist-clr-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = write_session(&dir, "a");
        let b = write_session(&dir, "b");

        let idx = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![
                    DiscoveredFile { file_path: a.clone(), mtime_ms: 0 },
                    DiscoveredFile { file_path: b.clone(), mtime_ms: 0 },
                ],
                provider: "mock".into(),
                working_directory: "/proj".into(),
            })],
            vec![dir.clone()],
        );
        idx.refresh();

        // Non-matching provider clears nothing; matching clears both.
        assert_eq!(idx.clear(Some("other"), None), 0);
        assert!(std::path::Path::new(&a).exists());
        assert_eq!(idx.clear(Some("mock"), None), 2);
        assert!(!std::path::Path::new(&a).exists() && !std::path::Path::new(&b).exists());
        assert!(idx.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_delete_outside_isolated_roots() {
        // A session that lives OUTSIDE the isolated roots (i.e. the user's
        // personal global home) must never be deletable.
        let dir = std::env::temp_dir().join(format!("amux-hist-guard-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let fp = write_session(&dir, "personal-sess");

        let idx = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: fp.clone(), mtime_ms: 0 }],
                provider: "claude".into(),
                working_directory: "/home/me/.claude".into(),
            })],
            // isolated roots deliberately do NOT include `dir`.
            vec![std::env::temp_dir().join("amux-some-other-isolated-root")],
        );
        idx.refresh();

        assert!(idx.delete("personal-sess").is_err(), "single delete must refuse");
        assert!(std::path::Path::new(&fp).exists(), "file must be intact");
        assert_eq!(idx.clear(None, None), 0, "clear-all must skip it");
        assert!(std::path::Path::new(&fp).exists(), "still intact after clear");

        let _ = fs::remove_dir_all(&dir);
    }
}
