// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! History module — discovers and indexes past CLI agent conversations from disk.

pub mod adapter;
pub mod claude_adapter;
pub mod index;
pub(crate) mod record;

use std::sync::Arc;

use adapter::*;
use claude_adapter::ClaudeHistoryAdapter;
use index::SessionIndex;

/// The global named-agent registry record for `agent_id`, for agents that have
/// no `db_agents` row — launching an agent does not create one, so a live agent
/// commonly exists only here.
///
/// Fetched at most once per `sessions_for_agent` call and used for BOTH
/// fallbacks the record can answer (definition id and working directory).
/// It used to return just the working directory, which meant the record's
/// `definition_id` — sitting right there — went unread, and identity-link
/// resolution fell back to the raw slug instead (reagentx P1 on PR #3480,
/// re-review).
///
/// The lookup itself comes from `backend::agent_registry_lookup`, which owns
/// the only copy of the slug-matching and path-reconstruction rules; this
/// function previously re-implemented both inline, which reagentx P2 on the
/// same PR flagged against `native_memory_handlers`' copy, since a later fix
/// to either one would silently miss the other.
fn registry_record(agent_id: &str) -> Option<crate::registry::NamedAgentRecord> {
    crate::backend::agent_registry_lookup::find_active_record_by_slug(agent_id)
}

/// Whose history a lookup is about: the definition id its identity links
/// are keyed by, and the working directory its transcripts record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryOwner {
    pub definition_id: String,
    pub working_directory: Option<String>,
}

impl HistoryOwner {
    /// Resolve an App API slug (the pre-M4c-2 path, and the one an
    /// Unattributed caller keeps).
    pub fn from_slug(store: &crate::backend::storage::store::Store, agent_id: &str) -> Self {
        // `agent_id` arrives as the SLUG (`AGENTMUX_AGENT_ID`) from every App
        // API caller, but `db_agent_identity_links.agent_id` stores the
        // DEFINITION id — so querying that table with the slug matched zero
        // rows every time, and the early return on "no links" then reported a
        // confident, empty history for agents whose transcripts were sitting
        // on disk. A caller that already holds a definition id passes through
        // unchanged. Same resolution `app_api::resolve_agent_definition_id`
        // already documents ("listing links by slug always returns empty").
        let instance = store.instance_get_by_slug(agent_id).ok().flatten();
        let definition_from_db = instance.as_ref().map(|i| {
            if i.definition_id.is_empty() { i.id.clone() } else { i.definition_id.clone() }
        });
        let working_dir_from_db = instance
            .as_ref()
            .map(|i| i.working_directory.clone())
            .filter(|w| !w.is_empty());

        // One registry read, shared by both fallbacks below, and skipped
        // entirely when the `db_agents` row already answered both questions.
        let record = if definition_from_db.is_none() || working_dir_from_db.is_none() {
            registry_record(agent_id)
        } else {
            None
        };

        // Without the registry step, a registry-only agent (no `db_agents`
        // row — the common case) fell through to the raw slug here, and the
        // link table is keyed by definition id, so the link query matched
        // nothing and the agent's identity-bound sessions silently vanished.
        // Same third fallback `app_api::resolve_agent_definition_id` already
        // has, for the same reason. The raw slug remains the last resort: a
        // caller that already holds a definition id passes through unchanged.
        let definition_id = definition_from_db
            .or_else(|| {
                record
                    .as_ref()
                    .map(|r| r.data.definition_id.clone())
                    .filter(|d| !d.is_empty())
            })
            .unwrap_or_else(|| agent_id.to_string());

        // The ambient-credentials fallback below (`sessions_for_owner`) needs
        // the working directory: the row's, else the registry record's.
        let working_directory = working_dir_from_db.or_else(|| {
            record
                .as_ref()
                .and_then(crate::backend::agent_registry_lookup::working_dir_from_record)
        });
        Self { definition_id, working_directory }
    }

    /// An agent's own row — identity M4c-2c, an attributed caller. Its
    /// working directory is the row's, else its registry record's found by
    /// the row's slug **and** id, never by slug alone.
    pub fn of_row(row: &crate::backend::storage::AgentDefinition) -> Self {
        let working_directory = Some(row.working_directory.clone())
            .filter(|w| !w.is_empty())
            .or_else(|| {
                crate::backend::agent_registry_lookup::find_active_record_by_slug_and_definition(
                    &row.slug, &row.id,
                )
                .as_ref()
                .and_then(crate::backend::agent_registry_lookup::working_dir_from_record)
            });
        Self { definition_id: row.id.clone(), working_directory }
    }
}

/// Account ids linked to more than one agent (`db_agent_identity_links`).
fn accounts_linked_to_several_agents(
    store: &crate::backend::storage::store::Store,
) -> Result<std::collections::HashSet<String>, String> {
    let mut agents_by_account: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for link in store
        .agent_identity_list_all()
        .map_err(|e| format!("failed to read identity links: {e}"))?
    {
        agents_by_account.entry(link.account_id).or_default().insert(link.agent_id);
    }
    Ok(agents_by_account
        .into_iter()
        .filter(|(_, agents)| agents.len() > 1)
        .map(|(account, _)| account)
        .collect())
}

/// A search's answer while the first index build after srv start is running.
pub const HISTORY_INDEX_BUILDING: &str = "history index is still being built after AgentMux started \
     (it reads every transcript once) — this is not an empty history; retry in a minute";

/// Why a history search could not answer.
#[derive(Debug)]
pub enum HistorySearchError {
    /// The first index build after srv start is still running. Retryable,
    /// and never to be read as "no history".
    IndexBuilding,
    /// Anything else, e.g. the store failing to list the owner's accounts.
    Failed(String),
}

impl std::fmt::Display for HistorySearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistorySearchError::IndexBuilding => f.write_str(HISTORY_INDEX_BUILDING),
            HistorySearchError::Failed(e) => f.write_str(e),
        }
    }
}

/// What a search knows about a session only AgentMux's record holds.
fn record_meta(session: &record::RecordSession) -> SessionMeta {
    SessionMeta {
        session_id: session.session_id.clone(),
        file_path: session
            .ranges
            .first()
            .map(|(zone, _, _)| format!("agentmux-record:{zone}"))
            .unwrap_or_default(),
        provider: "claude".to_string(),
        model: String::new(),
        slug: String::new(),
        working_directory: session.cwd.clone().unwrap_or_default(),
        created_at: session.first_ms,
        modified_at: session.last_ms,
        message_count: 0,
        first_user_message: String::new(),
        file_size_bytes: session.ranges.iter().map(|(_, start, end)| (end - start).max(0) as u64).sum(),
        git_branch: String::new(),
        total_tokens: 0,
        subagent_count: 0,
        identity_id: String::new(),
        starts_undated: session.first_ms == 0,
    }
}

/// SearchHistory's `since`/`until` in milliseconds. They're documented as
/// Unix seconds, but a hit's own `timestamp` is in milliseconds and is the
/// natural value to pass back; a value this large is already milliseconds
/// (as seconds it would be the year 5138).
pub fn unix_time_to_ms(value: i64) -> i64 {
    const ALREADY_MS: i64 = 100_000_000_000;
    if value.abs() >= ALREADY_MS {
        value
    } else {
        value.saturating_mul(1000)
    }
}

/// The history service exposed to the RPC layer.
pub struct HistoryService {
    index: Arc<SessionIndex>,
    /// AgentMux's own record of each agent's conversations — which sessions
    /// are whose, and the content of those whose transcript is gone.
    records: record::AgentRecords,
}

impl HistoryService {
    pub fn new() -> Self {
        let adapters: Vec<Box<dyn HistoryAdapter>> =
            vec![Box::new(ClaudeHistoryAdapter::new())];

        HistoryService {
            index: Arc::new(SessionIndex::new(adapters)),
            records: record::AgentRecords::global(),
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
        Self::from_index_and_records(index, record::AgentRecords::new(None))
    }

    #[cfg(test)]
    pub(crate) fn from_index_and_records(index: SessionIndex, records: record::AgentRecords) -> Self {
        HistoryService { index: Arc::new(index), records }
    }

    /// Build the index off-thread at srv start, so the first search doesn't
    /// pay for parsing every transcript on the machine inside the MCP's 10 s
    /// timeout (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.5).
    pub fn warm_in_background(&self) -> std::thread::JoinHandle<()> {
        let index = self.index.clone();
        std::thread::Builder::new()
            .name("history-index-warm".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let (discovered, _, _) = index.refresh();
                tracing::info!(
                    discovered,
                    duration_ms = started.elapsed().as_millis() as u64,
                    "history: index built"
                );
            })
            .expect("spawn history-index-warm thread")
    }

    /// Build the index if it never has been. Blocking; for callers without a
    /// tight deadline.
    fn ensure_built(&self) {
        if self.index.refreshed_at_ms() == 0 {
            self.index.refresh();
        }
    }

    /// Bring the index up to date before a search, so that "no hits, complete"
    /// can mean nothing was there: every session on disk when the search began
    /// is in the snapshot it reads. Reusing a recent snapshot (it used to be
    /// reused for 30 s, or whenever another refresh was running) let a session
    /// written in that window go unseen while the answer claimed to be
    /// complete (Codex P1 on #3693).
    ///
    /// An incremental refresh re-lists the transcript directories and
    /// re-parses only files whose mtime moved: 51,671 files stat'd in 2.7 s on
    /// one heavy host (2026-09-24), far less on most. A refresh already
    /// running is waited for and then followed by this search's own (unless
    /// another search's refresh began after this one did), since it may have
    /// started before this search's sessions were written. Only the
    /// very first build answers "still building" instead of waiting: it reads
    /// every transcript once and would outlast the MCP's 10 s timeout.
    /// Concurrent searches share refreshes ([`SessionIndex::refresh_covering_now`]).
    fn refresh_for_search(&self) -> Result<(), HistorySearchError> {
        if self.index.refreshed_at_ms() == 0 {
            return match self.index.try_refresh() {
                Some(_) => Ok(()),
                None => Err(HistorySearchError::IndexBuilding),
            };
        }
        self.index.refresh_covering_now();
        Ok(())
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
        self.ensure_built();

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
        let owner = HistoryOwner::from_slug(store, agent_id);
        self.sessions_for_owner(store, &owner, offset, limit, sort_by, sort_dir, force_refresh)
    }

    /// [`Self::sessions_for_agent`] for an owner already resolved — identity
    /// M4c-2c: an attributed caller's own row (`HistoryOwner::of_row`),
    /// never a slug another agent may share.
    pub fn sessions_for_owner(
        &self,
        store: &crate::backend::storage::store::Store,
        owner: &HistoryOwner,
        offset: usize,
        limit: usize,
        sort_by: &str,
        sort_dir: &str,
        force_refresh: bool,
    ) -> Result<(Vec<SessionMeta>, u32, bool), String> {
        if force_refresh {
            self.index.refresh();
        } else {
            self.ensure_built();
        }

        let links = store
            .agent_identity_list_for_agent(&owner.definition_id)
            .map_err(|e| format!("failed to resolve agent's linked identities: {e}"))?;

        // An account linked to several agents holds all of their transcripts
        // under one identity bundle, so its sessions are not all this
        // owner's: `SearchHistory` would return another agent's conversation,
        // the ungoverned disclosure `search_for_agent`'s doc rules out. From
        // such an account keep only sessions in the owner's own working
        // directory; with no known directory, keep none (counted) rather than
        // guess. An account only this owner uses keeps every session, so an
        // agent whose working directory moved keeps its older history.
        let shared_accounts = accounts_linked_to_several_agents(store)?;
        let own_dir = owner
            .working_directory
            .as_deref()
            .map(index::normalize_working_dir);
        let mut merged: Vec<SessionMeta> = Vec::new();
        for link in &links {
            let (sessions, _total, _has_more) =
                self.index.list_for_identity(&link.account_id, 0, usize::MAX, sort_by, sort_dir);
            if !shared_accounts.contains(&link.account_id) {
                merged.extend(sessions);
                continue;
            }
            match &own_dir {
                Some(dir) => merged.extend(
                    sessions
                        .into_iter()
                        .filter(|s| &index::normalize_working_dir(&s.working_directory) == dir),
                ),
                None => crate::backend::agent_resolve::record_uid_fallback(
                    "history.shared_account_without_working_dir",
                ),
            }
        }

        // An agent on ambient credentials has no link row at all — its
        // registry record's `identity_id` reads "default" while its
        // transcripts are written under a channel identity bundle nothing
        // points at — so the identity lookup above finds nothing for the
        // common case. The working directory is recorded inside the
        // transcript (`cwd`) and on the agent's own row, so it resolves the
        // sessions the identity bundle cannot.
        if let Some(dir) = &owner.working_directory {
            merged.extend(self.index.list_for_working_directory(dir));
        }

        // The two sources legitimately overlap for an agent that is both
        // identity-bound and running in its own directory.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        merged.retain(|s| seen.insert(s.session_id.clone()));

        let mut refs: Vec<&SessionMeta> = merged.iter().collect();
        index::SessionIndex::sort_sessions(&mut refs, sort_by, sort_dir);

        let total = refs.len() as u32;
        let has_more = offset + limit < refs.len();
        let page: Vec<SessionMeta> = refs.into_iter().skip(offset).take(limit).cloned().collect();
        Ok((page, total, has_more))
    }

    /// Search this agent's OWN conversation history.
    ///
    /// `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md`. Closes the gap where an
    /// agent's memory of what it did is bounded by its current context window
    /// while its actual actions are not — so a compaction or session reset
    /// leaves it confidently misreporting its own past, with nothing marking
    /// the boundary. Questions like "did I already send that message" are
    /// answerable from disk; before this they were not answerable by the agent
    /// being asked.
    ///
    /// **Own history only.** Searching another agent's transcript is already a
    /// governed act — `conversation_visibility` plus the `transcript_request`
    /// tier rules decide whether one agent may read another's content. A
    /// search verb that read other agents' sessions directly would be a
    /// second, ungoverned disclosure path around that machinery, and a more
    /// dangerous one for looking like an ordinary read tool. Cross-agent
    /// search must route *through* `transcript_request`, as its own phase.
    ///
    /// `opts.since_ms`/`until_ms` drop whole sessions on indexed metadata
    /// **before** any file is opened, so a narrow time window costs nothing on
    /// irrelevant sessions, and then filter the messages of the sessions that
    /// remain. `opts.max_sessions` bounds how many of those are parsed, newest
    /// first; the rest are counted, not dropped.
    pub fn search_for_agent(
        &self,
        store: &crate::backend::storage::store::Store,
        owner: &HistoryOwner,
        opts: &index::HistorySearchOptions,
    ) -> Result<index::HistorySearchOutcome, HistorySearchError> {
        self.refresh_for_search()?;

        // Whose sessions these are comes from AgentMux's own record, keyed by
        // the agent's UID (`owner.definition_id` is the caller's row id): the
        // same in every channel and version and across account switches. A
        // session there is searched in its provider transcript when that
        // still exists, and in the record when it doesn't.
        let (ledger, record_problems) = self.records.sessions_of(&owner.definition_id);
        let in_ledger: std::collections::HashSet<String> =
            ledger.iter().map(|s| s.session_id.clone()).collect();
        let mut candidates: Vec<index::SearchCandidate> = Vec::with_capacity(ledger.len());
        let mut record_only = 0u32;
        for session in ledger {
            let candidate = match self.index.get_meta(&session.session_id) {
                Some(meta) => index::SearchCandidate {
                    meta,
                    source: index::Source::Transcript,
                    attribution: index::Attribution::AgentRecord,
                },
                None => {
                    record_only += 1;
                    index::SearchCandidate {
                        meta: record_meta(&session),
                        source: index::Source::Record(session),
                        attribution: index::Attribution::AgentRecord,
                    }
                }
            };
            candidates.push(candidate);
        }

        // Sessions found only by working directory or account. When the agent
        // has a record they're left out (counted): none of them is in it, so
        // they're another agent's in a shared folder or account, or older
        // than the record (none on the host measured, 2026-09-24). An agent
        // with no record at all has nothing better, so it keeps them.
        let (by_folder, _total, _has_more) = self
            .sessions_for_owner(store, owner, 0, usize::MAX, "modified_at", "desc", false)
            .map_err(HistorySearchError::Failed)?;
        let inferred: Vec<SessionMeta> =
            by_folder.into_iter().filter(|s| !in_ledger.contains(&s.session_id)).collect();
        let mut inferred_excluded = 0u32;
        if in_ledger.is_empty() || opts.include_inferred {
            candidates.extend(inferred.into_iter().map(|meta| index::SearchCandidate {
                meta,
                source: index::Source::Transcript,
                attribution: index::Attribution::Inferred,
            }));
        } else {
            inferred_excluded = inferred.len() as u32;
        }

        // A session last written before `since` holds nothing after it, and
        // one started after `until` holds nothing before it. An unknown time
        // — no timestamp at all, or an undated record before the first dated
        // one — can't rule a session out.
        let mut in_window: Vec<index::SearchCandidate> = candidates
            .into_iter()
            .filter(|c| opts.since_ms.is_none_or(|since| c.meta.modified_at == 0 || c.meta.modified_at >= since))
            .filter(|c| {
                opts.until_ms.is_none_or(|until| {
                    c.meta.created_at == 0 || c.meta.starts_undated || c.meta.created_at <= until
                })
            })
            .collect();
        in_window.sort_by(|a, b| b.meta.modified_at.cmp(&a.meta.modified_at));

        let mut outcome = index::search_candidates(&in_window, opts, &|c| self.load(c));
        outcome.ledger_sessions = in_ledger.len() as u32;
        outcome.record_only_sessions = record_only;
        outcome.inferred_sessions_excluded = inferred_excluded;
        let mut problems = self.index.discovery_problems();
        problems.extend(record_problems);
        outcome.add_discovery_errors(problems);
        Ok(outcome)
    }

    /// A candidate's conversation, from wherever it lives.
    fn load(&self, candidate: &index::SearchCandidate) -> Result<Option<HistorySession>, HistoryError> {
        match &candidate.source {
            index::Source::Transcript => self.index.get_full(&candidate.meta.session_id),
            index::Source::Record(session) if session.ranges.is_empty() => Err(HistoryError::Other(
                "in the agent's segment log, but neither a provider transcript nor AgentMux's record holds it"
                    .into(),
            )),
            index::Source::Record(session) => {
                let (messages, skipped_records) = self.records.read(session)?;
                Ok(Some(HistorySession { meta: candidate.meta.clone(), messages, skipped_records }))
            }
        }
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
        self.ensure_built();

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
        self.ensure_built();
        match self.index.delete(session_id) {
            Ok(true) => serde_json::json!({ "deleted": true }),
            Ok(false) => serde_json::json!({ "deleted": false, "error": "session not found" }),
            Err(e) => serde_json::json!({ "deleted": false, "error": format!("{}", e) }),
        }
    }

    /// Bulk-clear sessions matching the optional provider/project filter.
    pub fn clear(&self, provider: Option<&str>, project: Option<&str>) -> serde_json::Value {
        self.ensure_built();
        let deleted = self.index.clear(provider, project);
        serde_json::json!({ "deleted": deleted })
    }
}

/// Whose sessions a search reads, from AgentMux's own record (PR 2 of the
/// history-robustness plan).
#[cfg(test)]
mod agent_record_search_tests {
    use super::record::fixtures::{append, assistant, init, store, zone, UID};
    use super::*;

    const CWD: &str = "/work/agent-a";

    /// A service over a real Claude adapter rooted at `shared`, and the
    /// agent's record in `records`.
    fn service(shared: &std::path::Path, records: &Arc<crate::backend::storage::filestore::FileStore>) -> HistoryService {
        HistoryService::from_index_and_records(
            SessionIndex::with_isolated_roots(
                vec![Box::new(ClaudeHistoryAdapter::with_roots(None, Some(shared.to_path_buf()), None))],
                vec![],
            ),
            record::AgentRecords::new(Some(records.clone())),
        )
    }

    fn transcript(shared: &std::path::Path, sid: &str, text: &str) {
        let project = shared.join("providers").join("claude").join("projects").join("-work-agent-a");
        std::fs::create_dir_all(&project).unwrap();
        let line = format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","model":"m","content":[{{"type":"text","text":"{text}"}}]}},"timestamp":"2026-09-24T10:00:00Z","cwd":"{CWD}"}}"#
        );
        std::fs::write(project.join(format!("{sid}.jsonl")), format!("{line}\n")).unwrap();
    }

    fn search(svc: &HistoryService, query: &str, include_inferred: bool) -> index::HistorySearchOutcome {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let owner = HistoryOwner { definition_id: UID.into(), working_directory: Some(CWD.into()) };
        let opts = index::HistorySearchOptions { query: query.into(), limit: 50, include_inferred, ..Default::default() };
        svc.search_for_agent(&store, &owner, &opts).unwrap()
    }

    /// 39% of the sessions on the host measured have no provider transcript
    /// left (Claude's cleanup, a deleted account folder). They're still in
    /// AgentMux's record, and found there.
    #[test]
    fn a_session_whose_transcript_is_gone_is_found_in_the_agents_record() {
        let tmp = tempfile::TempDir::new().unwrap();
        let records = store();
        append(&records, &zone(), &[init("s-gone", CWD), assistant("s-gone", "only the record remembers this")]);
        let out = search(&service(tmp.path(), &records), "record remembers", false);
        assert_eq!(out.hits.len(), 1, "{out:?}");
        assert_eq!(out.hits[0].source, "agentmux_record");
        assert_eq!(out.hits[0].attribution, "agent_record");
        assert_eq!(out.hits[0].session_id, "s-gone");
        assert_eq!((out.ledger_sessions, out.record_only_sessions), (1, 1));
        assert!(out.complete, "{out:?}");
    }

    /// A session whose transcript still exists is read from the transcript,
    /// once — not again from the record.
    #[test]
    fn a_session_with_a_transcript_is_searched_there_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let records = store();
        append(&records, &zone(), &[init("s1", CWD), assistant("s1", "words in both places")]);
        transcript(tmp.path(), "s1", "words in both places");
        let out = search(&service(tmp.path(), &records), "both places", false);
        assert_eq!(out.hits.len(), 1, "{out:?}");
        assert_eq!(out.hits[0].source, "provider_transcript");
        assert_eq!((out.ledger_sessions, out.record_only_sessions), (1, 0));
    }

    /// Found by folder but not in the agent's record: another agent's session
    /// in a shared folder, or one older than the record. Left out and counted
    /// — `include_inferred` searches it, labelled.
    #[test]
    fn a_session_found_only_by_folder_is_left_out_when_the_agent_has_a_record() {
        let tmp = tempfile::TempDir::new().unwrap();
        let records = store();
        append(&records, &zone(), &[init("s1", CWD), assistant("s1", "mine")]);
        transcript(tmp.path(), "s-other", "someone else wrote this");
        let svc = service(tmp.path(), &records);

        let out = search(&svc, "someone else", false);
        assert!(out.hits.is_empty(), "{out:?}");
        assert_eq!(out.inferred_sessions_excluded, 1);

        let out = search(&svc, "someone else", true);
        assert_eq!(out.hits.len(), 1, "{out:?}");
        assert_eq!(out.hits[0].attribution, "inferred");
    }

    /// An agent with no record at all (never ran on a build that keeps one)
    /// has nothing better than its folder, so it keeps that.
    #[test]
    fn an_agent_without_a_record_keeps_the_folder_match() {
        let tmp = tempfile::TempDir::new().unwrap();
        transcript(tmp.path(), "s-folder", "found by folder");
        let out = search(&service(tmp.path(), &store()), "by folder", false);
        assert_eq!(out.hits.len(), 1, "{out:?}");
        assert_eq!(out.hits[0].attribution, "inferred");
        assert_eq!(out.ledger_sessions, 0);
    }

    /// A session the segment log names but nothing holds any more can't be
    /// searched, and the answer says so.
    #[test]
    fn a_logged_session_with_no_content_anywhere_makes_the_answer_incomplete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let records = store();
        crate::backend::continuity_segments::record_start(
            &records,
            crate::backend::continuity_segments::Start {
                segment_id: String::new(),
                agent_uid: UID.into(),
                definition_id: Some(UID.into()),
                provider: "claude".into(),
                config_dir: None,
                provider_session_id: Some("s-lost".into()),
                cwd: CWD.into(),
                channel: "test".into(),
                agentmux_version: "0.0.0".into(),
                block_id: "b".into(),
                zone: None,
                byte_start: None,
                started_at_ms: 1_000,
                continuity_rung: crate::backend::continuity_segments::Rung::Fresh,
                predecessor_segment_id: None,
                lease_epoch: None,
            },
        )
        .unwrap();
        let out = search(&service(tmp.path(), &records), "anything", false);
        assert!(!out.complete, "{out:?}");
        assert_eq!(out.incomplete_reasons, vec!["unreadable_sessions"]);
        assert_eq!(out.sessions_unreadable[0].session_id, "s-lost");
    }
}

#[cfg(test)]
mod search_freshness_tests {
    use super::*;

    fn empty_service() -> HistoryService {
        HistoryService::from_index(SessionIndex::with_isolated_roots(vec![], vec![]))
    }

    /// The MCP gives a search 10 s; a cold build over every transcript on
    /// the machine takes far longer. While it runs, a search must answer at
    /// once — and say the index is building, never "no history".
    #[test]
    fn a_search_during_the_first_build_says_so_instead_of_waiting() {
        let svc = empty_service();
        let _building = svc.index.refresh_lock.lock().unwrap();
        let err = svc.refresh_for_search().unwrap_err();
        assert!(matches!(err, HistorySearchError::IndexBuilding), "{err}");
        assert!(err.to_string().contains("still being built"), "{err}");
    }

    #[test]
    fn since_and_until_accept_seconds_or_milliseconds() {
        // Documented as seconds …
        assert_eq!(unix_time_to_ms(1_790_280_000), 1_790_280_000_000);
        // … but a hit's own timestamp is milliseconds, and passing it back
        // must not be multiplied into the year 58,000.
        assert_eq!(unix_time_to_ms(1_790_280_000_123), 1_790_280_000_123);
        assert_eq!(unix_time_to_ms(0), 0);
    }

    #[test]
    fn a_search_on_a_never_built_idle_index_builds_it() {
        let svc = empty_service();
        assert!(svc.refresh_for_search().is_ok());
        assert!(svc.index.refreshed_at_ms() > 0);
    }

    fn real_service(shared: &std::path::Path) -> HistoryService {
        HistoryService::from_index(SessionIndex::with_isolated_roots(
            vec![Box::new(ClaudeHistoryAdapter::with_roots(None, Some(shared.to_path_buf()), None))],
            vec![],
        ))
    }

    fn owner_in(dir: &str) -> HistoryOwner {
        HistoryOwner { definition_id: "agent-a".into(), working_directory: Some(dir.into()) }
    }

    /// A transcript whose first message carries no timestamp: its session
    /// start (the first dated record) is after `until`, but the undated
    /// message may not be, and the message-level window keeps undated
    /// messages. Dropping the whole session on its start made the two
    /// disagree and could answer "complete, no hits" (Codex P2 on #3693).
    #[test]
    fn a_session_starting_with_an_undated_message_stays_in_the_until_window() {
        let tmp = tempfile::TempDir::new().unwrap();
        let shared = tmp.path().join("shared");
        let project = shared.join("providers").join("claude").join("projects").join("-work-a");
        std::fs::create_dir_all(&project).unwrap();
        let undated = r#"{"type":"user","message":{"role":"user","content":"undated question"},"cwd":"/work/a"}"#;
        let dated = r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":[{"type":"text","text":"answered"}]},"timestamp":"2026-09-24T12:00:00Z","cwd":"/work/a"}"#;
        std::fs::write(project.join("s.jsonl"), format!("{undated}\n{dated}\n")).unwrap();
        let svc = real_service(&shared);
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let until = chrono::DateTime::parse_from_rfc3339("2026-09-24T11:00:00Z").unwrap().timestamp_millis();
        let opts = index::HistorySearchOptions {
            query: "undated question".into(),
            limit: 50,
            until_ms: Some(until),
            ..Default::default()
        };
        let out = svc.search_for_agent(&store, &owner_in("/work/a"), &opts).unwrap();
        assert_eq!(out.hits.len(), 1, "{out:?}");
    }

    /// Discovery that couldn't read somewhere may have missed the session
    /// being searched for, so the answer isn't complete (Codex P1 on #3693).
    #[test]
    fn a_search_over_a_snapshot_with_discovery_problems_is_not_complete() {
        struct Unreadable;
        impl HistoryAdapter for Unreadable {
            fn provider(&self) -> &str {
                "mock"
            }
            fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
                Ok(Vec::new())
            }
            fn discover(&self) -> Result<Discovery, HistoryError> {
                Ok(Discovery { files: Vec::new(), problems: vec!["/x/projects: Access is denied".into()] })
            }
            fn extract_meta(&self, _: &str) -> Result<Option<SessionMeta>, HistoryError> {
                Ok(None)
            }
            fn parse_file(&self, _: &str) -> Result<Option<HistorySession>, HistoryError> {
                Ok(None)
            }
        }
        let svc = HistoryService::from_index(SessionIndex::with_isolated_roots(vec![Box::new(Unreadable)], vec![]));
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let opts = index::HistorySearchOptions { query: "anything".into(), limit: 50, ..Default::default() };
        let out = svc.search_for_agent(&store, &owner_in("/work/a"), &opts).unwrap();
        assert!(out.hits.is_empty());
        assert!(!out.complete, "{out:?}");
        assert_eq!(out.incomplete_reasons, vec!["discovery_errors"]);
        assert!(out.discovery_errors.iter().any(|p| p.contains("Access is denied")), "{out:?}");
    }

    /// A session written moments after one search is found by the next. A
    /// snapshot reused because it was "fresh enough" (under 30 s old, or
    /// another refresh running) answered `complete: true` without it — the
    /// false negative this whole tool exists to prevent (Codex P1 on #3693).
    #[test]
    fn a_session_written_right_after_a_search_is_found_by_the_next_search() {
        let tmp = tempfile::TempDir::new().unwrap();
        let shared = tmp.path().join("shared");
        let project = shared.join("providers").join("claude").join("projects").join("-work-a");
        std::fs::create_dir_all(&project).unwrap();
        let svc = HistoryService::from_index(SessionIndex::with_isolated_roots(
            vec![Box::new(ClaudeHistoryAdapter::with_roots(None, Some(shared.clone()), None))],
            vec![],
        ));
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let owner = HistoryOwner { definition_id: "agent-a".into(), working_directory: Some("/work/a".into()) };
        let opts = index::HistorySearchOptions { query: "deploy".into(), limit: 50, ..Default::default() };

        let first = svc.search_for_agent(&store, &owner, &opts).unwrap();
        assert!(first.hits.is_empty() && first.complete, "{first:?}");

        let line = r#"{"type":"user","message":{"role":"user","content":"deploy now"},"timestamp":"2026-09-24T10:00:00Z","cwd":"/work/a"}"#;
        std::fs::write(project.join("written-after.jsonl"), format!("{line}\n")).unwrap();
        let second = svc.search_for_agent(&store, &owner, &opts).unwrap();
        assert_eq!(second.hits.len(), 1, "{second:?}");
        assert!(second.complete, "{second:?}");
    }

    #[test]
    fn warm_in_background_builds_the_index() {
        let svc = empty_service();
        svc.warm_in_background().join().unwrap();
        assert!(svc.index.refreshed_at_ms() > 0);
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
        /// Defaults to "/proj" via [`MockAdapter::new`]; set explicitly by the
        /// working-directory resolution tests.
        working_directory: String,
    }

    impl MockAdapter {
        fn new(files: Vec<DiscoveredFile>, identity_id: &str) -> Self {
            MockAdapter {
                files,
                identity_id: identity_id.to_string(),
                working_directory: "/proj".to_string(),
            }
        }
        fn in_dir(files: Vec<DiscoveredFile>, identity_id: &str, working_directory: &str) -> Self {
            MockAdapter {
                files,
                identity_id: identity_id.to_string(),
                working_directory: working_directory.to_string(),
            }
        }
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
                working_directory: self.working_directory.clone(),
                created_at: 0,
                modified_at: 0,
                message_count: 0,
                first_user_message: String::new(),
                file_size_bytes: 0,
                git_branch: String::new(),
                total_tokens: 0,
                subagent_count: 0,
                identity_id: self.identity_id.clone(),
                starts_undated: false,
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
                Box::new(MockAdapter::new(
                    vec![DiscoveredFile { file_path: mine, mtime_ms: 1 }],
                    "acct-mine",
                )),
                Box::new(MockAdapter::new(
                    vec![DiscoveredFile { file_path: someone_elses, mtime_ms: 1 }],
                    "acct-someone-else",
                )),
            ],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::store::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
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
    fn list_for_agent_returns_empty_when_nothing_on_disk_matches_the_agent() {
        let service = HistoryService::from_index(SessionIndex::with_isolated_roots(vec![], vec![]));
        let store = Store::open_in_memory().unwrap();
        let result = service.list_for_agent(&store, "agent-with-no-links", 0, 10, "created_at", "desc");
        assert_eq!(result["total"], 0);
        assert_eq!(result["sessions"].as_array().unwrap().len(), 0);
    }

    // ── Agent → session resolution
    // (docs/specs/SPEC_CROSS_CHANNEL_AGENT_HISTORY_RESOLUTION_2026_09_21.md) ──

    /// Insert an agent whose slug differs from its id — i.e. every real
    /// agent, since the id is a UUID and the slug is human-readable.
    fn insert_agent(store: &Store, id: &str, name: &str, slug: &str, working_directory: &str) {
        let mut def = crate::backend::storage::store::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: slug.to_string(),
            name: name.to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: working_directory.to_string(),
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
    }

    /// The dominant real-world failure: an agent on ambient credentials has
    /// NO `db_agent_identity_links` row (its registry record's `identity_id`
    /// reads "default"), so identity-bundle resolution finds nothing — while
    /// its transcripts sit on disk, recording the directory it ran in. Before
    /// the working-directory index this returned a confident, empty history.
    #[test]
    fn sessions_resolve_by_working_directory_when_the_agent_has_no_identity_link() {
        let dir = std::env::temp_dir().join(format!("amux-hist-wd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = write_session(&dir, "mine-wd");
        let theirs = write_session(&dir, "theirs-wd");

        let index = SessionIndex::with_isolated_roots(
            vec![
                Box::new(MockAdapter::in_dir(
                    vec![DiscoveredFile { file_path: mine, mtime_ms: 1 }],
                    "",
                    "/agents/agenty-0629j",
                )),
                Box::new(MockAdapter::in_dir(
                    vec![DiscoveredFile { file_path: theirs, mtime_ms: 1 }],
                    "",
                    "/agents/somebody-else",
                )),
            ],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        insert_agent(&store, "def-uuid-agenty", "AgentY", "agenty", "/agents/agenty-0629j");
        // Deliberately NO agent_identity_link: that is the whole point.

        let (sessions, total, _) = service
            .sessions_for_agent(&store, "agenty", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(total, 1, "the agent's own session must be found without any identity link");
        assert_eq!(sessions[0].session_id, "mine-wd");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// One account linked to two agents holds both agents' transcripts. Each
    /// agent's history is its own working directory's sessions from it —
    /// never the other agent's — while an account only one agent uses keeps
    /// every session, including one from an earlier working directory.
    #[test]
    fn a_shared_account_contributes_only_the_owners_own_directory() {
        let dir = std::env::temp_dir().join(format!("amux-hist-shared-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let y = write_session(&dir, "shared-y");
        let z = write_session(&dir, "shared-z");
        let y_old = write_session(&dir, "solo-y-old-dir");

        let index = SessionIndex::with_isolated_roots(
            vec![
                Box::new(MockAdapter::in_dir(vec![DiscoveredFile { file_path: y, mtime_ms: 1 }], "acct-shared", "/agents/y")),
                Box::new(MockAdapter::in_dir(vec![DiscoveredFile { file_path: z, mtime_ms: 1 }], "acct-shared", "/agents/z")),
                Box::new(MockAdapter::in_dir(vec![DiscoveredFile { file_path: y_old, mtime_ms: 1 }], "acct-solo", "/agents/y-before-move")),
            ],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);
        let store = Store::open_in_memory().unwrap();
        insert_agent(&store, "def-y", "AgentY", "agenty", "/agents/y");
        insert_agent(&store, "def-z", "AgentZ", "agentz", "/agents/z");
        for acct in ["acct-shared", "acct-solo"] {
            store
                .identity_upsert(&crate::backend::storage::store::IdentityAccount {
                    id: acct.to_string(),
                    name: format!("claude-{acct}"),
                    provider: "claude".to_string(),
                    kind: "oauth".to_string(),
                    display_name: String::new(),
                    secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: String::new() },
                    context: serde_json::json!({}),
                    status: "unknown".to_string(),
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
        }
        store.agent_identity_link("def-y", "acct-shared", "claude").unwrap();
        store.agent_identity_link("def-z", "acct-shared", "claude").unwrap();
        store.agent_identity_link("def-y", "acct-solo", "codex").unwrap();

        let ids = |slug: &str| {
            let (sessions, _, _) = service
                .sessions_for_agent(&store, slug, 0, 10, "created_at", "desc", true)
                .unwrap();
            let mut ids: Vec<String> = sessions.into_iter().map(|s| s.session_id).collect();
            ids.sort();
            ids
        };
        assert_eq!(ids("agenty"), ["shared-y", "solo-y-old-dir"], "never AgentZ's session");
        assert_eq!(ids("agentz"), ["shared-z"], "never AgentY's sessions");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Callers pass the SLUG (`AGENTMUX_AGENT_ID`), but
    /// `db_agent_identity_links.agent_id` stores the DEFINITION id — so the
    /// link lookup has to resolve the slug first or it matches zero rows.
    #[test]
    fn identity_links_resolve_from_the_slug_not_only_the_definition_id() {
        let dir = std::env::temp_dir().join(format!("amux-hist-slug-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let linked = write_session(&dir, "linked-session");

        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter::in_dir(
                vec![DiscoveredFile { file_path: linked, mtime_ms: 1 }],
                "acct-mine",
                // A directory the agent is NOT in, so only the identity link
                // can account for this hit.
                "/somewhere/else",
            ))],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        insert_agent(&store, "def-uuid-agenty", "AgentY", "agenty", "/agents/agenty-0629j");
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
        // The link is keyed by DEFINITION id, as production writes it.
        store.agent_identity_link("def-uuid-agenty", "acct-mine", "claude").unwrap();

        // The caller passes the slug, which is all an agent knows about itself.
        let (sessions, total, _) = service
            .sessions_for_agent(&store, "agenty", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(total, 1, "the slug must resolve to the definition id the link is stored under");
        assert_eq!(sessions[0].session_id, "linked-session");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `list`'s `project` filter matches on `contains`; working-directory
    /// resolution must not, or an agent in `/agents/foo` silently claims
    /// every session belonging to `/agents/foo-2`.
    #[test]
    fn working_directory_matching_is_exact_not_a_prefix() {
        let dir = std::env::temp_dir().join(format!("amux-hist-exact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let neighbour = write_session(&dir, "neighbour-session");

        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter::in_dir(
                vec![DiscoveredFile { file_path: neighbour, mtime_ms: 1 }],
                "",
                "/agents/foo-2",
            ))],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        insert_agent(&store, "def-uuid-foo", "Foo", "foo", "/agents/foo");

        let (_, total, _) = service
            .sessions_for_agent(&store, "foo", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(total, 0, "/agents/foo must not claim /agents/foo-2's sessions");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A trailing separator or different slash style describes the same
    /// directory and must resolve to the same sessions.
    #[test]
    fn working_directory_matching_normalizes_separators_and_trailing_slash() {
        let dir = std::env::temp_dir().join(format!("amux-hist-norm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = write_session(&dir, "norm-session");

        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter::in_dir(
                vec![DiscoveredFile { file_path: mine, mtime_ms: 1 }],
                "",
                r"C:\work\agents\bar",
            ))],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        insert_agent(&store, "def-uuid-bar", "Bar", "bar", "C:/work/agents/bar/");

        let (_, total, _) = service
            .sessions_for_agent(&store, "bar", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(total, 1, "the same directory written differently must still match");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The registry fallback — `registry_record` plus
    /// `agent_registry_lookup::working_dir_from_record` — is the path for a
    /// live agent with NO `db_agents` row, which is the common case (launching
    /// an agent does not create one). Every other test here inserts a real row,
    /// so `instance_get_by_slug` always succeeds and this path never runs;
    /// a regression in slug derivation or `source_agents_base` handling would
    /// ship undetected. reagentx P2 on PR #3480.
    #[test]
    fn sessions_resolve_via_the_registry_when_the_agent_has_no_db_row() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

        // The registry stores working_dir RELATIVE to source_agents_base.
        let base = tmp.path().join("agentsbase");
        let resolved_dir = base.join("agenty-0629j").to_string_lossy().to_string();

        let registry = crate::registry::Registry::open(tmp.path().join("agents").join("registry")).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 3,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-agenty".to_string(),
                    instance_name: "AgentY".to_string(),
                    definition_id: "def-agenty".to_string(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    working_dir: "agenty-0629j".to_string(),
                    source_agents_base: Some(base.to_string_lossy().to_string()),
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let dir = std::env::temp_dir().join(format!("amux-hist-reg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = write_session(&dir, "registry-session");
        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter::in_dir(
                vec![DiscoveredFile { file_path: mine, mtime_ms: 1 }],
                "",
                &resolved_dir,
            ))],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        // Deliberately EMPTY store: no db_agents row, so resolution has to come
        // from the registry record above or not at all.
        let store = Store::open_in_memory().unwrap();

        // The display name is "AgentY"; callers pass the derived slug.
        let (sessions, total, _) = service
            .sessions_for_agent(&store, "agenty", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(total, 1, "registry fallback must resolve the working directory");
        assert_eq!(sessions[0].session_id, "registry-session");

        // An unrelated agent must not inherit it by coincidence.
        let (_, other_total, _) = service
            .sessions_for_agent(&store, "someone-else", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(other_total, 0, "an unrelated slug must not match");

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A registry-only agent that ALSO has a real identity link — the gap
    /// reagentx P1 flagged on PR #3480 (re-review). The registry test above
    /// proves the working-directory fallback, but its record has no identity,
    /// so the identity-link half was never exercised without a `db_agents`
    /// row.
    ///
    /// The failure it guards is quiet: with no instance row, `definition_id`
    /// fell back to the raw SLUG, and `db_agent_identity_links.agent_id`
    /// stores a DEFINITION id — so the link lookup matched zero rows and the
    /// agent's identity-bound sessions vanished. That is Bug A, reproduced for
    /// exactly the case the PR's own docs call the common one.
    ///
    /// The session here is deliberately in a directory the registry record
    /// does NOT point at, so the working-directory fallback cannot mask a
    /// regression by finding it a second way.
    #[test]
    fn registry_only_agent_still_resolves_its_identity_linked_sessions() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

        let base = tmp.path().join("agentsbase");
        let registry =
            crate::registry::Registry::open(tmp.path().join("agents").join("registry")).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 3,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-linked".to_string(),
                    instance_name: "AgentY".to_string(),
                    definition_id: "def-agenty".to_string(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    working_dir: "agenty-0629j".to_string(),
                    source_agents_base: Some(base.to_string_lossy().to_string()),
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let dir = std::env::temp_dir().join(format!("amux-hist-reglink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = write_session(&dir, "identity-linked-session");
        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter::in_dir(
                vec![DiscoveredFile { file_path: mine, mtime_ms: 1 }],
                "acct-mine",
                "/somewhere/the/registry/does/not/point",
            ))],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        // A definition + identity link, but NO `db_agents` instance row — so
        // `instance_get_by_slug("agenty")` finds nothing and the definition id
        // can only come from the registry record above.
        let store = Store::open_in_memory().unwrap();
        insert_test_agent_and_link(&store, "def-agenty", "acct-mine");

        let (sessions, total, _) = service
            .sessions_for_agent(&store, "agenty", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(
            total, 1,
            "a registry-only agent's identity-linked sessions must resolve; \
             the slug is not a definition id"
        );
        assert_eq!(sessions[0].session_id, "identity-linked-session");

        // The registry record must not hand its definition id to a different
        // agent — resolution stays scoped to the matching slug.
        let (_, other_total, _) = service
            .sessions_for_agent(&store, "someone-else", 0, 10, "created_at", "desc", true)
            .unwrap();
        assert_eq!(other_total, 0, "an unrelated slug must not inherit the link");

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two live agents whose display names normalize to the SAME slug must not
    /// be able to read each other's history. reagentx P1 on PR #3480
    /// (third review).
    ///
    /// Registry records are keyed by `instance_id`, and nothing enforces a
    /// unique `instance_name` across them — so `derive_slug("AgentY")` and
    /// `derive_slug("AGENTY")` both yield `agenty`. The lookup used to take
    /// the *first* match in whatever order the registry happened to list, then
    /// hand that record's `definition_id` and `working_dir` to the caller. For
    /// this path that means one agent's conversation history silently
    /// resolving into another agent's `sessions_for_agent` call.
    ///
    /// `native_memory_handlers` already guards the same lookup with
    /// `.filter(|r| r.data.definition_id == definition_id)`, but that guard is
    /// unavailable here: `definition_id` is precisely what this path is trying
    /// to resolve. So the lookup itself must refuse to guess.
    ///
    /// The assertion is deliberately "not the other agent's session" rather
    /// than an exact count — failing closed (0 sessions) is the required
    /// behaviour, but the bug being prevented is *leakage*, so that is what is
    /// asserted.
    #[test]
    fn colliding_slugs_never_resolve_into_another_agents_history() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

        let base = tmp.path().join("agentsbase");
        let registry =
            crate::registry::Registry::open(tmp.path().join("agents").join("registry")).unwrap();

        let mut rec = |instance_id: &str, name: &str, def: &str, dir: &str| {
            registry
                .upsert(&crate::registry::NamedAgentRecord {
                    schema_version: 3,
                    data: crate::registry::NamedAgentRecordV1 {
                        instance_id: instance_id.to_string(),
                        instance_name: name.to_string(),
                        definition_id: def.to_string(),
                        identity_id: None,
                        memory_id: None,
                        session_id: None,
                        working_dir: dir.to_string(),
                        source_agents_base: Some(base.to_string_lossy().to_string()),
                        created_at_ms: 1,
                        last_launched_at_ms: 1,
                        created_by_version: "test".to_string(),
                        last_launched_by_version: "test".to_string(),
                    },
                })
                .unwrap();
        };
        // Both normalize to the slug `agenty`.
        rec("inst-a", "AgentY", "def-a", "dir-a");
        rec("inst-b", "AGENTY", "def-b", "dir-b");

        // A session under EACH colliding record, so the assertion does not
        // depend on which one `list_active()` happens to return first. Whoever
        // the caller really is, at most one of these is theirs — and the slug
        // alone cannot say which — so resolving either one is a leak.
        let dir = std::env::temp_dir().join(format!("amux-hist-collide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sess_a = write_session(&dir, "agent-a-private-session");
        let sess_b = write_session(&dir, "agent-b-private-session");
        let index = SessionIndex::with_isolated_roots(
            vec![
                Box::new(MockAdapter::in_dir(
                    vec![DiscoveredFile { file_path: sess_a, mtime_ms: 1 }],
                    "acct-a",
                    &base.join("dir-a").to_string_lossy().to_string(),
                )),
                Box::new(MockAdapter::in_dir(
                    vec![DiscoveredFile { file_path: sess_b, mtime_ms: 1 }],
                    "acct-b",
                    &base.join("dir-b").to_string_lossy().to_string(),
                )),
            ],
            vec![dir.clone()],
        );
        let service = HistoryService::from_index(index);

        let store = Store::open_in_memory().unwrap();
        insert_test_agent_and_link(&store, "def-a", "acct-a");
        insert_test_agent_and_link(&store, "def-b", "acct-b");

        let (sessions, total, _) = service
            .sessions_for_agent(&store, "agenty", 0, 10, "created_at", "desc", true)
            .unwrap();

        assert_eq!(
            total,
            0,
            "an ambiguous slug must resolve to NO history rather than guessing \
             an agent; got {:?}",
            sessions.iter().map(|s| &s.session_id).collect::<Vec<_>>()
        );

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    fn insert_test_agent_and_link(store: &Store, agent_id: &str, account_id: &str) {
        let mut def = crate::backend::storage::store::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
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
                starts_undated: false,
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
