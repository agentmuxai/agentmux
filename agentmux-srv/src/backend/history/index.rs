// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory session index built from adapter discovery.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::adapter::*;

/// Max characters of matched text returned per hit.
///
/// A single history message can be hundreds of KB, and this tool must never be
/// the reason a caller blows its own context window — the problem it exists to
/// solve is an agent not being able to audit itself, which is made worse, not
/// better, by an answer too large to read. Not theoretical: the
/// `GetAgentTranscript` call that motivated
/// `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md` returned 112 KB and had to be
/// spilled to a file.
const SNIPPET_MAX_CHARS: usize = 400;
/// Characters of leading context kept before the match inside a snippet.
const SNIPPET_LEAD_CHARS: usize = 120;

/// Query parameters for [`SessionIndex::search_sessions`].
#[derive(Debug, Clone, Default)]
pub struct HistorySearchOptions {
    /// Case-insensitive substring. Matched against message content, tool
    /// names, and tool argument summaries.
    pub query: String,
    /// Restrict to `"user"` or `"assistant"` messages.
    pub role: Option<String>,
    /// Only match tool calls whose name equals this (case-insensitive).
    ///
    /// Its own parameter rather than folding into `query` because "find where
    /// I *called* SendMessage" is a different question from "find where I
    /// wrote the word SendMessage", and only the structural answer settles an
    /// audit.
    pub tool: Option<String>,
    /// Max hits to return before reporting `truncated`.
    pub limit: usize,
}

/// One match.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistorySearchHit {
    pub session_id: String,
    pub file_path: String,
    pub timestamp: i64,
    pub role: String,
    pub snippet: String,
    /// `Some(name)` when the hit is a tool call rather than message prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// Result of a bounded search.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistorySearchOutcome {
    pub hits: Vec<HistorySearchHit>,
    /// Sessions actually opened and parsed.
    pub sessions_scanned: u32,
    /// Candidates offered to the search (after the caller's own filtering).
    pub total_sessions: u32,
    /// True when the scan stopped on `limit` rather than exhausting every
    /// candidate — see `search_sessions`' doc comment for why this must never
    /// be conflated with "no matches".
    pub truncated: bool,
}

/// Case-insensitive substring search returning a byte offset into the
/// **original** haystack, not into a lowercased copy of it.
///
/// The naive version of this (`haystack.to_lowercase().find(needle)`, then
/// slice `haystack` at the result) is wrong, and wrong in a way that can
/// panic: lowercasing is not length-preserving. Turkish `İ` (U+0130, 2 bytes)
/// lowercases to `i̇` (3 bytes), so every such character before the match
/// shifts the two strings out of alignment — slicing the original at an offset
/// derived from the lowercased copy can land mid-character and panic, or
/// silently return a misaligned snippet. Caught by reagentx P1 on PR #3321.
///
/// So the lowercase mapping is built explicitly, recording which original byte
/// each lowercased byte came from.
fn find_case_insensitive(haystack: &str, needle_lower: &str) -> Option<usize> {
    if needle_lower.is_empty() {
        return None;
    }
    let mut lowered = String::with_capacity(haystack.len());
    // lowered-byte-offset -> original-byte-offset
    let mut origin: Vec<usize> = Vec::with_capacity(haystack.len() + 1);
    for (orig_idx, ch) in haystack.char_indices() {
        for lc in ch.to_lowercase() {
            let mut buf = [0u8; 4];
            let encoded = lc.encode_utf8(&mut buf);
            for _ in 0..encoded.len() {
                origin.push(orig_idx);
            }
            lowered.push_str(encoded);
        }
    }
    origin.push(haystack.len());
    let at = lowered.find(needle_lower)?;
    // Always a char boundary in the original: every entry is the start index
    // of the char that produced it.
    origin.get(at).copied()
}

/// Bound a matched region to something a caller can actually read, keeping a
/// little context before the match so the hit is interpretable.
fn snippet_around(haystack: &str, match_at: usize) -> String {
    let start = haystack[..match_at]
        .char_indices()
        .rev()
        .take(SNIPPET_LEAD_CHARS)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(match_at);
    let tail: String = haystack[start..].chars().take(SNIPPET_MAX_CHARS).collect();
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&tail);
    if start + tail.len() < haystack.len() {
        out.push('…');
    }
    out
}

/// Every match within one message: its prose, plus each tool call.
fn match_message(
    meta: &SessionMeta,
    msg: &HistoryMessage,
    needle: &str,
    opts: &HistorySearchOptions,
) -> Vec<HistorySearchHit> {
    let mut hits = Vec::new();

    // Tool-call matches. When `tool` is set the search is structural: the
    // filter is the tool NAME, and an empty query then means "every call to
    // it" rather than "no matches".
    //
    // Every snippet offset below is resolved against the exact string the
    // snippet is cut from. Deriving an offset from one string and slicing
    // another — e.g. finding in `"{name} {args}"` and slicing `args` — puts
    // the window in the wrong place, and for a long argument summary silently
    // returns evidence that does not contain the match. A hit whose snippet
    // omits the match is worse than no hit: it looks like verification.
    // Both were reagentx P1s on PR #3321.
    for tu in &msg.tool_uses {
        if let Some(want) = opts.tool.as_deref() {
            if !tu.name.eq_ignore_ascii_case(want) {
                continue;
            }
            // Empty query + tool filter = "every call to this tool", so the
            // start of the arguments is the right window.
            let at = if needle.is_empty() {
                Some(0)
            } else {
                find_case_insensitive(&tu.argument_summary, needle)
            };
            if let Some(at) = at {
                hits.push(HistorySearchHit {
                    session_id: meta.session_id.clone(),
                    file_path: meta.file_path.clone(),
                    timestamp: msg.timestamp,
                    role: msg.role.clone(),
                    snippet: snippet_around(&tu.argument_summary, at),
                    tool_name: Some(tu.name.clone()),
                });
            }
            continue;
        }
        if needle.is_empty() {
            continue;
        }
        // No tool filter: the query may match the tool's NAME or its
        // arguments. Searched separately so the snippet offset always belongs
        // to `argument_summary`, the string actually being cut.
        let at = if find_case_insensitive(&tu.name, needle).is_some() {
            Some(0)
        } else {
            find_case_insensitive(&tu.argument_summary, needle)
        };
        if let Some(at) = at {
            hits.push(HistorySearchHit {
                session_id: meta.session_id.clone(),
                file_path: meta.file_path.clone(),
                timestamp: msg.timestamp,
                role: msg.role.clone(),
                snippet: snippet_around(&tu.argument_summary, at),
                tool_name: Some(tu.name.clone()),
            });
        }
    }

    // Prose match — skipped entirely when the caller asked a structural
    // tool-only question.
    if opts.tool.is_none() && !needle.is_empty() {
        if let Some(at) = find_case_insensitive(&msg.content, needle) {
            hits.push(HistorySearchHit {
                session_id: meta.session_id.clone(),
                file_path: meta.file_path.clone(),
                timestamp: msg.timestamp,
                role: msg.role.clone(),
                snippet: snippet_around(&msg.content, at),
                tool_name: None,
            });
        }
    }

    hits
}

/// AgentMux-ISOLATED provider-home roots under which delete/clear is permitted:
/// `<shared>/providers/` and `<shared>/identities/`. Anything outside these
/// (the user's personal `~/.claude` / `~/.config/claude-*`) is OFF-LIMITS so a
/// "clear all" can never nuke transcripts AgentMux didn't create.
fn default_isolated_roots() -> Vec<PathBuf> {
    let shared = std::env::var_os("AGENTMUX_SHARED_DIR")
        .map(PathBuf::from)
        .or_else(|| Some(crate::backend::base::get_mux_data_dir().join("shared")));
    match shared {
        Some(s) => vec![s.join("providers"), s.join("identities")],
        None => Vec::new(),
    }
}

/// In-memory index of discovered sessions.
pub struct SessionIndex {
    /// session_id -> SessionMeta
    sessions: Mutex<HashMap<String, SessionMeta>>,
    /// identity_id -> session_ids (only non-empty `SessionMeta::identity_id`
    /// values are indexed here). Maintained alongside `sessions` so "this
    /// identity's sessions" is an O(k) HashMap lookup + small per-identity
    /// sort, not an O(total sessions) scan of everything on disk — see
    /// `list_for_identity` and
    /// `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
    /// §4.4.
    by_identity: Mutex<HashMap<String, Vec<String>>>,
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
            by_identity: Mutex::new(HashMap::new()),
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
        let mut new_by_identity: HashMap<String, Vec<String>> = HashMap::new();

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
                        if !meta.identity_id.is_empty() {
                            new_by_identity
                                .entry(meta.identity_id.clone())
                                .or_default()
                                .push(meta.session_id.clone());
                        }
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
        *self.by_identity.lock().unwrap() = new_by_identity;

        (discovered, updated, new_count)
    }

    /// This identity's sessions, sorted/paginated the same way `list` is —
    /// but via the `by_identity` index instead of scanning every session on
    /// disk. `identity_id` is a bundle/account id (`SessionMeta::identity_id`
    /// — see its own doc comment); resolving an `agent_id` to one is the
    /// caller's job (`Store::agent_identity_list_for_agent`), not this
    /// index's — a bundle is a filesystem fact, which agent(s) claim it is
    /// a database fact, and this module has no Store access by design (it's
    /// pure discovery/indexing over the filesystem).
    pub fn list_for_identity(
        &self,
        identity_id: &str,
        offset: usize,
        limit: usize,
        sort_by: &str,
        sort_dir: &str,
    ) -> (Vec<SessionMeta>, u32, bool) {
        let by_identity = self.by_identity.lock().unwrap();
        let Some(session_ids) = by_identity.get(identity_id) else {
            return (Vec::new(), 0, false);
        };
        let sessions = self.sessions.lock().unwrap();
        let mut filtered: Vec<&SessionMeta> =
            session_ids.iter().filter_map(|id| sessions.get(id)).collect();
        Self::sort_sessions(&mut filtered, sort_by, sort_dir);

        let total = filtered.len() as u32;
        let has_more = offset + limit < filtered.len();
        let page: Vec<SessionMeta> = filtered.into_iter().skip(offset).take(limit).cloned().collect();
        (page, total, has_more)
    }

    /// Shared sort logic between `list` (scans everything) and
    /// `list_for_identity` (scans one identity's sessions) — extracted so
    /// the two can't silently diverge on sort semantics.
    pub(crate) fn sort_sessions(filtered: &mut [&SessionMeta], sort_by: &str, sort_dir: &str) {
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

        Self::sort_sessions(&mut filtered, sort_by, sort_dir);

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

    /// Search message content and tool calls across a specific, already-chosen
    /// set of sessions.
    ///
    /// `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md` §3.1/§3.2. The caller picks
    /// the candidate sessions (normally `HistoryService::search_for_agent`,
    /// which resolves them from the agent's linked identities and applies the
    /// `since`/`until` window) — this function only scans what it is handed,
    /// so the expensive `get_full` parse never touches a session the caller
    /// already ruled out on cheap indexed metadata.
    ///
    /// Bounded deliberately: it stops at `limit` hits and reports whether it
    /// stopped early. **That signal is load-bearing.** A search that silently
    /// truncates and returns nothing reproduces the exact failure this whole
    /// feature exists to prevent — an agent concluding "I never did that" from
    /// an incomplete audit — so "no matches" and "ran out of budget" must never
    /// look the same to a caller.
    pub fn search_sessions(
        &self,
        candidates: &[SessionMeta],
        opts: &HistorySearchOptions,
    ) -> HistorySearchOutcome {
        let needle = opts.query.to_lowercase();
        let mut hits: Vec<HistorySearchHit> = Vec::new();
        let mut sessions_scanned = 0u32;
        let mut truncated = false;

        for meta in candidates {
            if hits.len() >= opts.limit {
                // More candidates remained that were never opened.
                truncated = true;
                break;
            }
            sessions_scanned += 1;
            // A session that fails to parse is skipped, not fatal: one
            // malformed file must not make the whole audit unanswerable.
            let Ok(Some(session)) = self.get_full(&meta.session_id) else {
                continue;
            };
            for msg in &session.messages {
                if let Some(role) = opts.role.as_deref() {
                    if msg.role != role {
                        continue;
                    }
                }
                if hits.len() >= opts.limit {
                    truncated = true;
                    break;
                }
                for hit in match_message(meta, msg, &needle, opts) {
                    if hits.len() >= opts.limit {
                        truncated = true;
                        break;
                    }
                    hits.push(hit);
                }
            }
        }

        HistorySearchOutcome {
            hits,
            sessions_scanned,
            total_sessions: candidates.len() as u32,
            truncated,
        }
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
        identity_id: String,
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
                identity_id: self.identity_id.clone(),
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
                identity_id: String::new(),
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
                identity_id: String::new(),
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
                identity_id: String::new(),
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

    // docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
    // §4.4: list_for_identity is the actual "fast lookup" this whole step
    // exists for — an O(sessions for this identity) HashMap-backed lookup
    // instead of list()'s O(total sessions on disk) scan.

    #[test]
    fn list_for_identity_returns_only_that_identitys_sessions() {
        let dir = std::env::temp_dir().join(format!("amux-hist-idx-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a1 = write_session(&dir, "a1");
        let a2 = write_session(&dir, "a2");
        let b1 = write_session(&dir, "b1");
        let unattributed = write_session(&dir, "unattributed");

        let idx = SessionIndex::with_isolated_roots(
            vec![
                Box::new(MockAdapter {
                    files: vec![
                        DiscoveredFile { file_path: a1, mtime_ms: 1 },
                        DiscoveredFile { file_path: a2, mtime_ms: 2 },
                    ],
                    provider: "mock".into(),
                    working_directory: "/proj".into(),
                    identity_id: "identity-a".into(),
                }),
                Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: b1, mtime_ms: 1 }],
                    provider: "mock".into(),
                    working_directory: "/proj".into(),
                    identity_id: "identity-b".into(),
                }),
                Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: unattributed, mtime_ms: 1 }],
                    provider: "mock".into(),
                    working_directory: "/proj".into(),
                    identity_id: String::new(),
                }),
            ],
            vec![dir.clone()],
        );
        idx.refresh();

        let (sessions_a, total_a, _) = idx.list_for_identity("identity-a", 0, 10, "created_at", "desc");
        assert_eq!(total_a, 2);
        let ids_a: std::collections::HashSet<_> = sessions_a.iter().map(|s| s.session_id.clone()).collect();
        assert_eq!(ids_a, std::collections::HashSet::from(["a1".to_string(), "a2".to_string()]));

        let (sessions_b, total_b, _) = idx.list_for_identity("identity-b", 0, 10, "created_at", "desc");
        assert_eq!(total_b, 1);
        assert_eq!(sessions_b[0].session_id, "b1");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_for_identity_returns_empty_for_an_unknown_identity() {
        let idx = SessionIndex::with_isolated_roots(vec![], vec![]);
        idx.refresh();
        let (sessions, total, has_more) = idx.list_for_identity("no-such-identity", 0, 10, "created_at", "desc");
        assert!(sessions.is_empty());
        assert_eq!(total, 0);
        assert!(!has_more);
    }

    #[test]
    fn list_for_identity_paginates_and_respects_sort_direction() {
        let dir = std::env::temp_dir().join(format!("amux-hist-idx-page-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let files: Vec<DiscoveredFile> = (0..3)
            .map(|i| DiscoveredFile { file_path: write_session(&dir, &format!("s{i}")), mtime_ms: i })
            .collect();

        let idx = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files,
                provider: "mock".into(),
                working_directory: "/proj".into(),
                identity_id: "identity-a".into(),
            })],
            vec![dir.clone()],
        );
        idx.refresh();

        let (page1, total, has_more) = idx.list_for_identity("identity-a", 0, 2, "created_at", "desc");
        assert_eq!(total, 3);
        assert_eq!(page1.len(), 2);
        assert!(has_more);
        let (page2, _, has_more2) = idx.list_for_identity("identity-a", 2, 2, "created_at", "desc");
        assert_eq!(page2.len(), 1);
        assert!(!has_more2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_rebuilds_the_identity_index_from_scratch_each_time() {
        // A session that's re-discovered under a DIFFERENT identity on a
        // second refresh (e.g. its bundle got re-linked) must not leave a
        // stale entry under the old identity_id.
        let dir = std::env::temp_dir().join(format!("amux-hist-idx-rebuild-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let fp = write_session(&dir, "moved");

        let idx = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: fp.clone(), mtime_ms: 1 }],
                provider: "mock".into(),
                working_directory: "/proj".into(),
                identity_id: "identity-old".into(),
            })],
            vec![dir.clone()],
        );
        idx.refresh();
        assert_eq!(idx.list_for_identity("identity-old", 0, 10, "created_at", "desc").1, 1);

        let idx2 = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: fp, mtime_ms: 1 }],
                provider: "mock".into(),
                working_directory: "/proj".into(),
                identity_id: "identity-new".into(),
            })],
            vec![dir.clone()],
        );
        idx2.refresh();
        assert_eq!(idx2.list_for_identity("identity-old", 0, 10, "created_at", "desc").1, 0, "stale identity must not linger");
        assert_eq!(idx2.list_for_identity("identity-new", 0, 10, "created_at", "desc").1, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    // ── History search (SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md) ──

    /// Adapter serving canned messages, recording which files were actually
    /// opened — the recording is what lets a test prove the search never
    /// parses a session the caller already excluded.
    struct SearchMockAdapter {
        sessions: HashMap<String, Vec<HistoryMessage>>,
        parsed: Mutex<Vec<String>>,
    }

    impl HistoryAdapter for SearchMockAdapter {
        fn provider(&self) -> &str {
            "mock"
        }
        fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
            Ok(self
                .sessions
                .keys()
                .map(|id| DiscoveredFile { file_path: format!("/tmp/{id}.jsonl"), mtime_ms: 0 })
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
                provider: "mock".into(),
                model: String::new(),
                slug: String::new(),
                working_directory: "/proj".into(),
                created_at: 0,
                modified_at: 0,
                message_count: 0,
                first_user_message: String::new(),
                file_size_bytes: 0,
                git_branch: String::new(),
                total_tokens: 0,
                subagent_count: 0,
                identity_id: String::new(),
            }))
        }
        fn parse_file(&self, file_path: &str) -> Result<Option<HistorySession>, HistoryError> {
            self.parsed.lock().unwrap().push(file_path.to_string());
            let id = PathBuf::from(file_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(messages) = self.sessions.get(&id) else { return Ok(None) };
            let meta = self.extract_meta(file_path)?.unwrap();
            Ok(Some(HistorySession { meta, messages: messages.clone() }))
        }
    }

    fn msg(role: &str, content: &str, tools: Vec<(&str, &str)>) -> HistoryMessage {
        HistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: 100,
            tool_uses: tools
                .into_iter()
                .map(|(n, a)| ToolUseSummary { name: n.to_string(), argument_summary: a.to_string() })
                .collect(),
        }
    }

    fn search_index(sessions: Vec<(&str, Vec<HistoryMessage>)>) -> SessionIndex {
        let map: HashMap<String, Vec<HistoryMessage>> =
            sessions.into_iter().map(|(id, m)| (id.to_string(), m)).collect();
        let idx = SessionIndex::new(vec![Box::new(SearchMockAdapter {
            sessions: map,
            parsed: Mutex::new(Vec::new()),
        })]);
        idx.refresh();
        idx
    }

    fn opts(query: &str) -> HistorySearchOptions {
        HistorySearchOptions { query: query.into(), role: None, tool: None, limit: 50 }
    }

    #[test]
    fn search_matches_message_content() {
        let idx = search_index(vec![("s1", vec![msg("assistant", "I merged the WAN signing PR", vec![])])]);
        let cands = vec![idx.get_meta("s1").unwrap()];
        let out = idx.search_sessions(&cands, &opts("wan signing"));
        assert_eq!(out.hits.len(), 1);
        assert!(out.hits[0].snippet.contains("WAN signing"), "snippet carries the match");
        assert!(!out.truncated);
    }

    #[test]
    fn search_matches_a_tool_call_structurally_not_just_prose() {
        // The question that motivated this whole feature: "did I call
        // SendMessage to Agent4" must be answerable even when no prose in the
        // transcript mentions it.
        let args = r#"{"to":"Agent4","message":"following up"}"#;
        let idx = search_index(vec![(
            "s1",
            vec![msg("assistant", "sure, sending now", vec![("SendMessage", args)])],
        )]);
        let cands = vec![idx.get_meta("s1").unwrap()];

        let mut o = opts("");
        o.tool = Some("sendmessage".into()); // case-insensitive
        let out = idx.search_sessions(&cands, &o);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].tool_name.as_deref(), Some("SendMessage"));
        assert!(out.hits[0].snippet.contains("Agent4"));

        // And filtering by argument content within that tool.
        let mut o2 = opts("agent4");
        o2.tool = Some("SendMessage".into());
        assert_eq!(idx.search_sessions(&cands, &o2).hits.len(), 1);
        let mut o3 = opts("agent9");
        o3.tool = Some("SendMessage".into());
        assert_eq!(idx.search_sessions(&cands, &o3).hits.len(), 0);
    }

    #[test]
    fn an_exhausted_budget_is_distinguishable_from_no_matches() {
        // The single most important property here. A search that silently
        // stops early and returns nothing recreates the exact failure this
        // feature exists to prevent: an agent concluding "I never did that"
        // from an incomplete audit.
        let many: Vec<HistoryMessage> =
            (0..10).map(|i| msg("assistant", &format!("hit number {i}"), vec![])).collect();
        let idx = search_index(vec![("s1", many)]);
        let cands = vec![idx.get_meta("s1").unwrap()];

        let mut limited = opts("hit number");
        limited.limit = 3;
        let out = idx.search_sessions(&cands, &limited);
        assert_eq!(out.hits.len(), 3);
        assert!(out.truncated, "stopping on budget must be reported");

        let absent = idx.search_sessions(&cands, &opts("never appears anywhere"));
        assert!(absent.hits.is_empty());
        assert!(!absent.truncated, "genuinely-no-matches must NOT look truncated");
    }

    #[test]
    fn search_only_opens_the_sessions_it_is_handed() {
        // What makes the since/until window cheap: excluded candidates are
        // never parsed, because the caller filters on indexed metadata first.
        let idx = search_index(vec![
            ("keep", vec![msg("assistant", "findme", vec![])]),
            ("skip", vec![msg("assistant", "findme", vec![])]),
        ]);
        let cands = vec![idx.get_meta("keep").unwrap()];
        let out = idx.search_sessions(&cands, &opts("findme"));
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.sessions_scanned, 1);
        assert_eq!(out.total_sessions, 1);
        assert_eq!(out.hits[0].session_id, "keep");
    }

    #[test]
    fn role_filter_narrows_to_one_side_of_the_conversation() {
        let idx = search_index(vec![(
            "s1",
            vec![msg("user", "deploy it", vec![]), msg("assistant", "deploy it? confirming first", vec![])],
        )]);
        let cands = vec![idx.get_meta("s1").unwrap()];
        let mut o = opts("deploy it");
        o.role = Some("user".into());
        let out = idx.search_sessions(&cands, &o);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].role, "user");
    }

    #[test]
    fn a_snippet_is_bounded_even_for_a_pathologically_large_message() {
        // This tool must never be the reason a caller blows its own context.
        let huge = format!("{}NEEDLE{}", "x".repeat(50_000), "y".repeat(50_000));
        let idx = search_index(vec![("s1", vec![msg("assistant", &huge, vec![])])]);
        let cands = vec![idx.get_meta("s1").unwrap()];
        let out = idx.search_sessions(&cands, &opts("needle"));
        assert_eq!(out.hits.len(), 1);
        assert!(out.hits[0].snippet.chars().count() <= SNIPPET_MAX_CHARS + 2, "snippet stays bounded");
        assert!(out.hits[0].snippet.contains("NEEDLE"), "and still shows the match");
    }

    // ── Snippet/offset correctness (reagentx P1s on PR #3321) ──
    //
    // All three of these fail against the original implementation. They are
    // about the same underlying mistake in three places: an offset computed
    // against one string and then used to slice a different one.

    #[test]
    fn a_tool_hit_snippet_contains_the_match_even_far_into_a_long_argument() {
        // The snippet used to be cut from offset 0 whenever a `tool` filter
        // was set, so a match past SNIPPET_MAX_CHARS was reported as a hit
        // whose evidence did not contain it. A hit whose snippet omits the
        // match is worse than no hit — it looks like verification.
        let args = format!("{}NEEDLE_TARGET{}", "p".repeat(3_000), "q".repeat(100));
        let idx = search_index(vec![("s1", vec![msg("assistant", "sent", vec![("SendMessage", &args)])])]);
        let cands = vec![idx.get_meta("s1").unwrap()];

        let mut o = opts("needle_target");
        o.tool = Some("SendMessage".into());
        let out = idx.search_sessions(&cands, &o);
        assert_eq!(out.hits.len(), 1);
        assert!(
            out.hits[0].snippet.contains("NEEDLE_TARGET"),
            "snippet must show the match, not the start of the arguments: {}",
            out.hits[0].snippet
        );
    }

    #[test]
    fn an_untooled_tool_hit_offsets_against_the_arguments_not_name_plus_arguments() {
        // Previously the offset came from `format!("{name} {args}")` but was
        // used to slice `args` alone, so the window was shifted by
        // name.len()+1 — and for a long name effectively clamped to the end.
        let args = format!("{}FIND_ME{}", "z".repeat(1_000), "w".repeat(1_000));
        let idx = search_index(vec![(
            "s1",
            vec![msg("assistant", "no prose match here", vec![("SomeVeryLongToolNameIndeed", &args)])],
        )]);
        let cands = vec![idx.get_meta("s1").unwrap()];

        let out = idx.search_sessions(&cands, &opts("find_me"));
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].tool_name.as_deref(), Some("SomeVeryLongToolNameIndeed"));
        assert!(
            out.hits[0].snippet.contains("FIND_ME"),
            "snippet window must be centred on the real match: {}",
            out.hits[0].snippet
        );
    }

    #[test]
    fn matching_is_safe_when_lowercasing_changes_byte_length() {
        // Lowercasing is not length-preserving: 'İ' (U+0130, 2 bytes) becomes
        // 'i̇' (3 bytes). Finding in a lowercased copy and slicing the
        // original therefore drifts by one byte per such character, which can
        // slice mid-character and panic. This must neither panic nor
        // mis-window.
        let content = format!("{}NEEDLE tail", "İ".repeat(200));
        let idx = search_index(vec![("s1", vec![msg("assistant", &content, vec![])])]);
        let cands = vec![idx.get_meta("s1").unwrap()];

        let out = idx.search_sessions(&cands, &opts("needle"));
        assert_eq!(out.hits.len(), 1);
        assert!(
            out.hits[0].snippet.contains("NEEDLE"),
            "snippet must still contain the match: {}",
            out.hits[0].snippet
        );
    }

    #[test]
    fn case_insensitive_find_reports_offsets_into_the_original_string() {
        // Direct unit coverage of the helper the three fixes above rely on.
        assert_eq!(find_case_insensitive("Hello World", "world"), Some(6));
        assert_eq!(find_case_insensitive("abc", "zzz"), None);
        assert_eq!(find_case_insensitive("abc", ""), None);

        // 'İ' is 2 bytes and lowercases to 3, so a naive lowercased-offset
        // would report 3 here instead of the correct 2.
        let s = "İX";
        assert_eq!(find_case_insensitive(s, "x"), Some(2));
        assert!(s.is_char_boundary(find_case_insensitive(s, "x").unwrap()));
    }
}
