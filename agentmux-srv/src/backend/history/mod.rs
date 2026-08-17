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

    /// Construct directly from a `SessionIndex` — used by tests that need
    /// to inject a mock adapter instead of `ClaudeHistoryAdapter::new()`'s
    /// real filesystem scan. `pub(crate)` (not private) so tests in other
    /// modules (e.g. `app_api::bundle`'s `bundle.export_for_agent_with_history`
    /// tests) can build an isolated `HistoryService` too, instead of
    /// depending on `AppState::history_service`'s real filesystem scan.
    #[cfg(test)]
    pub(crate) fn from_index(index: SessionIndex) -> Self {
        HistoryService { index: Arc::new(index) }
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

    /// Typed core of `list_for_agent` — the JSON-returning method wraps
    /// this. Exists as its own function so other backend code (e.g.
    /// `bundle.rs`'s `bundle.export_for_agent_with_history`) can get this
    /// agent's `SessionMeta` list directly, without a JSON
    /// serialize/deserialize round-trip through the RPC-facing shape.
    ///
    /// `force_refresh`: reagentx P1 on PR #2613 — the lazy
    /// refresh-only-if-empty behavior (shared with `list`/`get`, fine for
    /// an interactive browse where slight staleness is an acceptable
    /// trade for speed) is wrong for a caller claiming completeness, like
    /// `bundle.export_for_agent_with_history`: once the index has been
    /// populated once (by ANY prior call, interactive or otherwise), a
    /// session created since then would silently be missing from an
    /// export that reports itself as the full record. `list_for_agent`
    /// (the interactive RPC) passes `false`, preserving its existing
    /// speed/freshness trade-off; the export path passes `true`.
    pub fn sessions_for_agent(
        &self,
        store: &crate::backend::storage::store::Store,
        agent_id: &str,
        offset: usize,
        limit: usize,
        sort_by: &str,
        sort_dir: &str,
        force_refresh: bool,
    ) -> Result<(Vec<SessionMeta>, u32, bool), String> {
        if force_refresh || self.index.is_empty() {
            self.index.refresh();
        }

        let links = store
            .agent_identity_list_for_agent(agent_id)
            .map_err(|e| format!("failed to resolve agent's linked identities: {e}"))?;
        if links.is_empty() {
            return Ok((Vec::new(), 0, false));
        }

        let mut merged: Vec<SessionMeta> = Vec::new();
        for link in &links {
            let (sessions, _total, _has_more) =
                self.index.list_for_identity(&link.account_id, 0, usize::MAX, sort_by, sort_dir);
            merged.extend(sessions);
        }
        let mut refs: Vec<&SessionMeta> = merged.iter().collect();
        index::SessionIndex::sort_sessions(&mut refs, sort_by, sort_dir);

        let total = refs.len() as u32;
        let has_more = offset + limit < refs.len();
        let page: Vec<SessionMeta> = refs.into_iter().skip(offset).take(limit).cloned().collect();
        Ok((page, total, has_more))
    }

    /// This agent's own sessions — the actual "fast Conversation History
    /// lookup" protocol §4.4 asks for, resolving `agent_id` to its bound
    /// identity bundle(s) (`Store::agent_identity_list_for_agent`, the
    /// same `db_agent_identity_links` table `identity_auth_dirs.rs`
    /// already keys off) and querying `SessionIndex::list_for_identity`'s
    /// O(sessions for this identity) HashMap-backed path instead of
    /// `list()`'s O(total sessions on disk) scan. An agent normally has
    /// at most one linked account per provider, but this merges across
    /// however many exist rather than assuming exactly one.
    pub fn list_for_agent(
        &self,
        store: &crate::backend::storage::store::Store,
        agent_id: &str,
        offset: usize,
        limit: usize,
        sort_by: &str,
        sort_dir: &str,
    ) -> serde_json::Value {
        let (page, total, has_more) = match self.sessions_for_agent(store, agent_id, offset, limit, sort_by, sort_dir, false) {
            Ok(r) => r,
            Err(e) => return serde_json::json!({ "error": e }),
        };

        serde_json::json!({
            "sessions": page,
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

    /// Delete a single session's on-disk transcript and drop it from the index.
    pub fn delete(&self, session_id: &str) -> serde_json::Value {
        if self.index.is_empty() {
            self.index.refresh();
        }
        match self.index.delete(session_id) {
            Ok(true) => serde_json::json!({ "deleted": true }),
            Ok(false) => serde_json::json!({ "deleted": false, "error": "session not found" }),
            Err(e) => serde_json::json!({ "deleted": false, "error": format!("{}", e) }),
        }
    }

    /// Bulk-clear sessions matching the optional provider/project filter.
    pub fn clear(&self, provider: Option<&str>, project: Option<&str>) -> serde_json::Value {
        if self.index.is_empty() {
            self.index.refresh();
        }
        let deleted = self.index.clear(provider, project);
        serde_json::json!({ "deleted": deleted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    /// Adapter that "discovers" caller-supplied files, tagging each with a
    /// fixed identity_id -- just enough to exercise the agent_id ->
    /// identity_id -> sessions chain end to end.
    struct MockAdapter {
        files: Vec<DiscoveredFile>,
        identity_id: String,
    }
    impl HistoryAdapter for MockAdapter {
        fn provider(&self) -> &str {
            "mock"
        }
        fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
            Ok(self.files.iter().map(|f| DiscoveredFile { file_path: f.file_path.clone(), mtime_ms: f.mtime_ms }).collect())
        }
        fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError> {
            let id = std::path::Path::new(file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(Some(SessionMeta {
                session_id: id,
                file_path: file_path.to_string(),
                provider: "mock".to_string(),
                model: String::new(),
                slug: String::new(),
                working_directory: "/proj".to_string(),
                created_at: 0,
                modified_at: 0,
                message_count: 0,
                first_user_message: String::new(),
                file_size_bytes: 0,
                git_branch: String::new(),
                total_tokens: 0,
                subagent_count: 0,
                identity_id: self.identity_id.clone(),
            }))
        }
        fn parse_file(&self, _: &str) -> Result<Option<HistorySession>, HistoryError> {
            Ok(None)
        }
    }

    fn write_session(dir: &std::path::Path, id: &str) -> String {
        let f = dir.join(format!("{id}.jsonl"));
        std::fs::write(&f, b"{}").unwrap();
        f.to_string_lossy().into_owned()
    }

    // docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
    // §4.4: the actual end-to-end feature -- agent_id resolves through a
    // real db_agent_identity_links row (not a mocked lookup) to the
    // sessions found under that link's account_id.
    #[test]
    fn list_for_agent_resolves_through_a_real_identity_link_to_the_right_sessions() {
        let dir = std::env::temp_dir().join(format!("amux-hist-svc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = write_session(&dir, "mine");
        let someone_elses = write_session(&dir, "someone-elses");

        let index = SessionIndex::with_isolated_roots(
            vec![
                Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: mine, mtime_ms: 1 }],
                    identity_id: "acct-mine".to_string(),
                }),
                Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: someone_elses, mtime_ms: 1 }],
                    identity_id: "acct-someone-else".to_string(),
                }),
            ],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "agent-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();
        store
            .identity_upsert(&crate::backend::storage::store::IdentityAccount {
                id: "acct-mine".to_string(),
                name: "claude-acct-mine".to_string(),
                provider: "claude".to_string(),
                kind: "pat".to_string(),
                display_name: String::new(),
                secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: String::new() },
                context: serde_json::json!({}),
                status: "unknown".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.agent_identity_link("agent-1", "acct-mine", "claude").unwrap();

        let result = service.list_for_agent(&store, "agent-1", 0, 10, "created_at", "desc");
        assert_eq!(result["total"], 1);
        assert_eq!(result["sessions"][0]["session_id"], "mine");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_for_agent_returns_empty_for_an_agent_with_no_linked_identity() {
        let service = HistoryService::from_index(SessionIndex::with_isolated_roots(vec![], vec![]));
        let store = Store::open_in_memory().unwrap();
        let result = service.list_for_agent(&store, "agent-with-no-links", 0, 10, "created_at", "desc");
        assert_eq!(result["total"], 0);
        assert_eq!(result["sessions"].as_array().unwrap().len(), 0);
    }

    fn insert_test_agent_and_link(store: &Store, agent_id: &str, account_id: &str) {
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: agent_id.to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();
        store
            .identity_upsert(&crate::backend::storage::store::IdentityAccount {
                id: account_id.to_string(),
                name: format!("claude-{account_id}"),
                provider: "claude".to_string(),
                kind: "pat".to_string(),
                display_name: String::new(),
                secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: String::new() },
                context: serde_json::json!({}),
                status: "unknown".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.agent_identity_link(agent_id, account_id, "claude").unwrap();
    }

    /// Unlike `MockAdapter` (a fixed file list captured at construction),
    /// this re-scans a real directory on every `discover_files()` call --
    /// needed to exercise `refresh()`'s actual re-scan behavior, not just
    /// the in-memory index it populates.
    struct DynamicMockAdapter {
        dir: std::path::PathBuf,
        identity_id: String,
    }
    impl HistoryAdapter for DynamicMockAdapter {
        fn provider(&self) -> &str {
            "mock"
        }
        fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
            let entries = std::fs::read_dir(&self.dir).map_err(|e| HistoryError::Other(e.to_string()))?;
            Ok(entries
                .flatten()
                .map(|e| DiscoveredFile { file_path: e.path().to_string_lossy().into_owned(), mtime_ms: 0 })
                .collect())
        }
        fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError> {
            let id = std::path::Path::new(file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(Some(SessionMeta {
                session_id: id,
                file_path: file_path.to_string(),
                provider: "mock".to_string(),
                model: String::new(),
                slug: String::new(),
                working_directory: "/proj".to_string(),
                created_at: 0,
                modified_at: 0,
                message_count: 0,
                first_user_message: String::new(),
                file_size_bytes: 0,
                git_branch: String::new(),
                total_tokens: 0,
                subagent_count: 0,
                identity_id: self.identity_id.clone(),
            }))
        }
        fn parse_file(&self, _: &str) -> Result<Option<HistorySession>, HistoryError> {
            Ok(None)
        }
    }

    // reagentx P1 on PR #2613: sessions_for_agent's lazy
    // refresh-only-if-empty behavior is wrong for a caller claiming
    // completeness -- a session created after the index was first
    // populated (by ANY prior call) must still show up when
    // force_refresh=true, and must NOT show up when force_refresh=false
    // (proving the two modes are genuinely different, not that refresh
    // just always happens to run).
    #[test]
    fn force_refresh_true_picks_up_a_session_created_after_first_population_false_does_not() {
        let dir = std::env::temp_dir().join(format!("amux-hist-force-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_session(&dir, "first");

        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(DynamicMockAdapter { dir: dir.clone(), identity_id: "acct-mine".to_string() })],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);
        let store = Store::open_in_memory().unwrap();
        insert_test_agent_and_link(&store, "agent-1", "acct-mine");

        // First call populates the index (empty -> refresh runs regardless
        // of force_refresh).
        let (first_pass, total1, _) = service.sessions_for_agent(&store, "agent-1", 0, 10, "created_at", "desc", false).unwrap();
        assert_eq!(total1, 1);
        assert_eq!(first_pass[0].session_id, "first");

        // A new session appears on disk after that first population.
        write_session(&dir, "second");

        let (stale, total_stale, _) =
            service.sessions_for_agent(&store, "agent-1", 0, 10, "created_at", "desc", false).unwrap();
        assert_eq!(total_stale, 1, "force_refresh=false must NOT see the new session (index already non-empty)");

        let (fresh, total_fresh, _) =
            service.sessions_for_agent(&store, "agent-1", 0, 10, "created_at", "desc", true).unwrap();
        assert_eq!(total_fresh, 2, "force_refresh=true must see the new session");
        assert!(fresh.iter().any(|s| s.session_id == "second"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
