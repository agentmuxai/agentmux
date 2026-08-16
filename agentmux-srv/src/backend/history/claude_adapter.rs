// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Claude Code history adapter.
//! Scans for session JSONL files across both the user's global Claude homes and
//! the AgentMux-isolated homes that current agents actually write to:
//!   - ~/.claude/projects/ and ~/.config/claude-*/projects/ (global / legacy)
//!   - <AGENTMUX_SHARED_DIR>/providers/claude/projects/ (default isolated home)
//!   - <AGENTMUX_SHARED_DIR>/identities/<bundle_id>/claude/projects/ (per-identity, global)
//!   - <home>/channels/*/identities/*/claude/projects/ and
//!     <home>/dev/*/(*/)?identities/*/claude/projects/ (per-channel isolated —
//!     Step 4 of docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md.
//!     Step 3 links these forward to the global location, but history written
//!     before that fix landed, or for a bundle whose link creation failed
//!     (best-effort, can fail), still has real data only under these
//!     per-channel paths — scanned here so it stays discoverable rather than
//!     requiring a manual filesystem search.)

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::adapter::*;

pub struct ClaudeHistoryAdapter {
    /// All base directories to scan for project folders. Deduplicated by
    /// canonical (symlink/junction-resolved) path at construction time —
    /// after Step 3 above, a per-channel isolated `projects/` dir may be a
    /// junction pointing at the exact same physical location as one of the
    /// global `<shared>/...` entries, and scanning both would surface every
    /// session twice.
    base_dirs: Vec<PathBuf>,
}

impl ClaudeHistoryAdapter {
    /// Push `path` onto `dirs` iff it's a real directory AND its
    /// canonical (symlink/junction-resolved) form hasn't already been
    /// pushed — the dedup key, not `path` itself, since two different
    /// `base_dirs` entries can be different paths to the same physical
    /// directory (a per-channel isolated `projects/` junction and the
    /// global location it points at). Falls back to the literal path
    /// when canonicalization fails (e.g. a dangling/broken link) rather
    /// than silently dropping it.
    fn push_deduped_dir(dirs: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
        if !path.is_dir() {
            return;
        }
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(key) {
            dirs.push(path);
        }
    }

    pub fn new() -> Self {
        let mut base_dirs = Vec::new();
        let mut seen_canonical: HashSet<PathBuf> = HashSet::new();

        if let Some(home) = dirs::home_dir() {
            // User's personal (non-isolated) Claude sessions
            Self::push_deduped_dir(&mut base_dirs, &mut seen_canonical, home.join(".claude").join("projects"));

            // Legacy multi-account convention: ~/.config/claude-*/projects/
            let config_dir = home.join(".config");
            if config_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&config_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with("claude-") {
                            Self::push_deduped_dir(&mut base_dirs, &mut seen_canonical, entry.path().join("projects"));
                        }
                    }
                }
            }
        }

        // AgentMux-ISOLATED Claude homes — where agents spawned by current builds
        // actually write (AgentMux sets CLAUDE_CONFIG_DIR here at spawn time).
        // Without these, the history browse misses every AgentMux agent
        // conversation. See docs/specs/SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md.
        //   <shared>/providers/claude/projects/              (default, account-wide)
        //   <shared>/identities/<bundle_id>/claude/projects/ (per-identity bundles, global)
        // `AGENTMUX_SHARED_DIR` is exported by the launcher; fall back to
        // ~/.agentmux/shared so discovery still works in plain/test contexts.
        let shared_dir = std::env::var_os("AGENTMUX_SHARED_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".agentmux").join("shared")));
        if let Some(shared) = &shared_dir {
            Self::push_deduped_dir(&mut base_dirs, &mut seen_canonical, shared.join("providers").join("claude").join("projects"));
            if let Ok(entries) = fs::read_dir(shared.join("identities")) {
                for entry in entries.flatten() {
                    Self::push_deduped_dir(&mut base_dirs, &mut seen_canonical, entry.path().join("claude").join("projects"));
                }
            }
        }

        // Per-channel ISOLATED identity dirs (SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
        // §4.4) — `<home>/channels/*/identities/*/claude/projects` and
        // `<home>/dev/*/identities/*/claude/projects` (or, when a dev clone
        // id is in play, one level deeper: `<home>/dev/*/*/identities/*/...`).
        // `home_dir` is re-derived the same way `DataPaths::from_env()`
        // itself derives it (AGENTMUX_HOME_OVERRIDE or the OS home dir),
        // not read from an env var AgentMux doesn't currently export for
        // this purpose.
        if let Some(paths) = agentmux_common::DataPaths::from_env() {
            for root_name in ["channels", "dev"] {
                Self::scan_isolated_identities_under(
                    &paths.home_dir.join(root_name),
                    &mut base_dirs,
                    &mut seen_canonical,
                    2,
                );
            }
        }

        ClaudeHistoryAdapter { base_dirs }
    }

    /// Find every `identities/` directory up to `max_depth` levels under
    /// `root`, and for each, every `<bundle_id>/claude/projects` that
    /// exists. Bounded, not an unbounded recursive walk — channel/dev
    /// directory layouts are shallow by construction
    /// (`channels/<slug>/identities/`, `dev/<branch>/identities/` or
    /// `dev/<branch>/<clone_id>/identities/`), so depth 2 covers every
    /// known layout without risking an expensive filesystem walk if
    /// something unexpected is nested underneath.
    fn scan_isolated_identities_under(
        root: &Path,
        base_dirs: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        max_depth: u8,
    ) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let identities = path.join("identities");
            if identities.is_dir() {
                if let Ok(id_entries) = fs::read_dir(&identities) {
                    for id_entry in id_entries.flatten() {
                        Self::push_deduped_dir(base_dirs, seen, id_entry.path().join("claude").join("projects"));
                    }
                }
            }
            if max_depth > 1 {
                Self::scan_isolated_identities_under(&path, base_dirs, seen, max_depth - 1);
            }
        }
    }

    /// Count subagent JSONL files in a session's subagents/ directory.
    fn count_subagents(session_dir: &Path) -> u32 {
        let subagents_dir = session_dir.join("subagents");
        if !subagents_dir.is_dir() {
            return 0;
        }
        fs::read_dir(&subagents_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        let name = e.file_name();
                        let s = name.to_string_lossy();
                        s.starts_with("agent-") && s.ends_with(".jsonl")
                    })
                    .count() as u32
            })
            .unwrap_or(0)
    }

    /// Decode a project directory name back to a path.
    /// e.g., "C--Users-asafe--claw-agentx-workspace" → "C:/Users/asafe/.claw/agentx-workspace"
    /// This is lossy — real hyphens are indistinguishable from path separators.
    fn decode_project_path(encoded: &str) -> String {
        // Best-effort: replace leading drive pattern and path separators
        let mut result = encoded.to_string();
        // Restore drive letter colon: "C-" at start → "C:"
        if result.len() >= 2 && result.as_bytes()[1] == b'-' && result.as_bytes()[0].is_ascii_uppercase() {
            result = format!("{}:{}", &result[..1], &result[2..]);
        }
        // Replace remaining hyphens with forward slashes
        result = result.replace('-', "/");
        result
    }
}

impl HistoryAdapter for ClaudeHistoryAdapter {
    fn provider(&self) -> &str {
        "claude"
    }

    fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
        let mut files = Vec::new();

        for base_dir in &self.base_dirs {
            let entries = match fs::read_dir(base_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for project_entry in entries.flatten() {
                let project_path = project_entry.path();
                if !project_path.is_dir() {
                    // Top-level .jsonl files (session files at project root level)
                    if project_path.extension().map_or(false, |e| e == "jsonl") {
                        if let Ok(meta) = project_path.metadata() {
                            let mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            files.push(DiscoveredFile {
                                file_path: project_path.to_string_lossy().into(),
                                mtime_ms: mtime,
                            });
                        }
                    }
                    continue;
                }

                // Scan for .jsonl files inside project directories
                // These are session directories that may also contain subagents/
                let dir_entries = match fs::read_dir(&project_path) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for file_entry in dir_entries.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().map_or(false, |e| e == "jsonl") {
                        // Skip subagent files — those are children of sessions
                        if file_path
                            .parent()
                            .and_then(|p| p.file_name())
                            .map_or(false, |n| n == "subagents")
                        {
                            continue;
                        }
                        if let Ok(meta) = file_path.metadata() {
                            let mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            files.push(DiscoveredFile {
                                file_path: file_path.to_string_lossy().into(),
                                mtime_ms: mtime,
                            });
                        }
                    }
                }
            }
        }

        files.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms));
        Ok(files)
    }

    fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError> {
        let path = Path::new(file_path);
        let file = fs::File::open(path)?;
        let file_size = file.metadata()?.len();
        let reader = BufReader::new(file);

        let mut first_user_msg = String::new();
        let mut model = "unknown".to_string();
        let mut slug = String::new();
        let mut cwd = String::new();
        let mut git_branch = String::new();
        let mut entry_count = 0u32;
        let mut total_tokens: u64 = 0;
        let mut first_timestamp: i64 = 0;
        let mut last_timestamp: i64 = 0;
        let mut session_id = String::new();

        // Extract session_id from filename (stem)
        if let Some(stem) = path.file_stem() {
            session_id = stem.to_string_lossy().into();
        }

        let mut lines_iter = reader.lines();
        let mut found_all_meta = false;

        while let Some(Ok(line)) = lines_iter.next() {
            if line.trim().is_empty() {
                continue;
            }

            let entry: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            entry_count += 1;

            // Extract timestamp
            if let Some(ts_str) = entry.get("timestamp").and_then(|v| v.as_str()) {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                    let ts = dt.timestamp_millis();
                    if first_timestamp == 0 {
                        first_timestamp = ts;
                    }
                    last_timestamp = ts;
                }
            }

            // Extract session slug
            if slug.is_empty() {
                if let Some(s) = entry.get("slug").and_then(|v| v.as_str()) {
                    slug = s.to_string();
                }
            }

            // Extract session ID from entry if available
            if session_id.is_empty() {
                if let Some(s) = entry.get("sessionId").and_then(|v| v.as_str()) {
                    session_id = s.to_string();
                }
            }

            // Extract cwd
            if cwd.is_empty() {
                if let Some(c) = entry.get("cwd").and_then(|v| v.as_str()) {
                    cwd = c.to_string();
                }
            }

            // Extract git branch
            if git_branch.is_empty() {
                if let Some(b) = entry.get("gitBranch").and_then(|v| v.as_str()) {
                    git_branch = b.to_string();
                }
            }

            let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

            // Extract model from first assistant entry
            if model == "unknown" && entry_type == "assistant" {
                if let Some(m) = entry.pointer("/message/model").and_then(|v| v.as_str()) {
                    model = m.to_string();
                }
                // Accumulate tokens
                if let Some(usage) = entry.pointer("/message/usage") {
                    if let Some(out) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        total_tokens += out;
                    }
                }
            } else if entry_type == "assistant" {
                // Still accumulate tokens for non-first assistant entries
                if let Some(usage) = entry.pointer("/message/usage") {
                    if let Some(out) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        total_tokens += out;
                    }
                }
            }

            // Extract first user message for preview
            if first_user_msg.is_empty() && entry_type == "user" {
                if let Some(content) = entry.pointer("/message/content") {
                    if let Some(text) = content.as_str() {
                        first_user_msg = text.chars().take(200).collect();
                    } else if let Some(arr) = content.as_array() {
                        // Content can be an array of content blocks
                        for block in arr {
                            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                    first_user_msg = text.chars().take(200).collect();
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Early exit: once we have all metadata fields, count remaining lines cheaply
            if !first_user_msg.is_empty()
                && model != "unknown"
                && !cwd.is_empty()
                && !slug.is_empty()
            {
                found_all_meta = true;
                break;
            }
        }

        // Count remaining lines without parsing JSON (fast)
        if found_all_meta {
            for remaining_line in lines_iter {
                if let Ok(line) = remaining_line {
                    if !line.trim().is_empty() {
                        entry_count += 1;
                    }
                }
            }
        }

        if entry_count == 0 {
            return Ok(None);
        }

        // Fallback: decode project path from parent directory name
        if cwd.is_empty() {
            if let Some(parent_name) = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
            {
                cwd = Self::decode_project_path(&parent_name);
            }
        }

        // Count subagents
        let subagent_count = if let Some(parent) = path.parent() {
            let session_dir = parent.join(&session_id);
            Self::count_subagents(&session_dir)
        } else {
            0
        };

        let file_meta = fs::metadata(file_path)?;
        let modified_at = file_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(last_timestamp);

        Ok(Some(SessionMeta {
            session_id,
            file_path: file_path.to_string(),
            provider: "claude".to_string(),
            model,
            slug,
            working_directory: cwd,
            created_at: first_timestamp,
            modified_at,
            message_count: entry_count,
            first_user_message: first_user_msg,
            file_size_bytes: file_size,
            git_branch,
            total_tokens,
            subagent_count,
        }))
    }

    fn parse_file(&self, file_path: &str) -> Result<Option<HistorySession>, HistoryError> {
        // First extract meta
        let meta = match self.extract_meta(file_path)? {
            Some(m) => m,
            None => return Ok(None),
        };

        let file = fs::File::open(file_path)?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }

            let entry: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");

            // Extract timestamp
            let timestamp = entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis())
                .unwrap_or(0);

            if entry_type == "user" {
                let content = if let Some(msg) = entry.pointer("/message/content") {
                    if let Some(text) = msg.as_str() {
                        text.to_string()
                    } else if let Some(arr) = msg.as_array() {
                        arr.iter()
                            .filter_map(|block| {
                                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                                    block.get("text").and_then(|v| v.as_str()).map(String::from)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                if !content.is_empty() {
                    messages.push(HistoryMessage {
                        role: "user".to_string(),
                        content,
                        timestamp,
                        tool_uses: vec![],
                    });
                }
            } else if entry_type == "assistant" {
                let mut text_parts = Vec::new();
                let mut tool_uses = Vec::new();

                if let Some(content_arr) = entry.pointer("/message/content").and_then(|v| v.as_array()) {
                    for block in content_arr {
                        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                    text_parts.push(text.to_string());
                                }
                            }
                            "tool_use" => {
                                let name = block
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                // Summarize first argument
                                let arg_summary = if let Some(input) = block.get("input") {
                                    if let Some(obj) = input.as_object() {
                                        // Take first key-value pair as summary
                                        obj.iter()
                                            .next()
                                            .map(|(k, v)| {
                                                let val_str = if let Some(s) = v.as_str() {
                                                    s.chars().take(100).collect::<String>()
                                                } else {
                                                    v.to_string().chars().take(100).collect::<String>()
                                                };
                                                format!("{}: {}", k, val_str)
                                            })
                                            .unwrap_or_default()
                                    } else {
                                        String::new()
                                    }
                                } else {
                                    String::new()
                                };
                                tool_uses.push(ToolUseSummary {
                                    name,
                                    argument_summary: arg_summary,
                                });
                            }
                            // Skip "thinking" blocks — they're internal reasoning
                            _ => {}
                        }
                    }
                }

                let content = text_parts.join("\n");
                if !content.is_empty() || !tool_uses.is_empty() {
                    messages.push(HistoryMessage {
                        role: "assistant".to_string(),
                        content,
                        timestamp,
                        tool_uses,
                    });
                }
            }
        }

        Ok(Some(HistorySession { meta, messages }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn push_deduped_dir_skips_a_path_that_is_not_a_real_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::push_deduped_dir(&mut dirs, &mut seen, tmp.path().join("does-not-exist"));
        assert!(dirs.is_empty());
    }

    #[test]
    fn push_deduped_dir_pushes_a_real_directory_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("real");
        std::fs::create_dir_all(&dir).unwrap();
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::push_deduped_dir(&mut dirs, &mut seen, dir.clone());
        ClaudeHistoryAdapter::push_deduped_dir(&mut dirs, &mut seen, dir);
        assert_eq!(dirs.len(), 1, "calling twice with the identical path must not duplicate it");
    }

    // The actual motivating case: after Step 3
    // (SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md §4.1),
    // an isolated bundle's projects/ is a junction pointing at the exact
    // global path this adapter also scans directly — without dedup by
    // canonical path, every session under it would appear twice.
    #[cfg(windows)]
    #[test]
    fn push_deduped_dir_treats_a_junction_and_its_target_as_the_same_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("global").join("projects");
        let link = tmp.path().join("isolated").join("projects");
        agentmux_common::ensure_history_link(&link, &target).unwrap();

        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::push_deduped_dir(&mut dirs, &mut seen, target.clone());
        ClaudeHistoryAdapter::push_deduped_dir(&mut dirs, &mut seen, link);
        assert_eq!(
            dirs.len(),
            1,
            "the junction and its target must be recognized as the same physical location, not scanned twice"
        );
    }

    #[test]
    fn scan_isolated_identities_finds_a_channel_scoped_projects_dir_at_depth_one() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = tmp
            .path()
            .join("local-main-abc123")
            .join("identities")
            .join("bundle-1")
            .join("claude")
            .join("projects");
        std::fs::create_dir_all(&projects).unwrap();

        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::scan_isolated_identities_under(tmp.path(), &mut dirs, &mut seen, 2);
        assert_eq!(dirs, vec![projects]);
    }

    #[test]
    fn scan_isolated_identities_finds_a_dev_clone_scoped_projects_dir_at_depth_two() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = tmp
            .path()
            .join("main")
            .join("a1b2c3d4")
            .join("identities")
            .join("bundle-2")
            .join("claude")
            .join("projects");
        std::fs::create_dir_all(&projects).unwrap();

        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::scan_isolated_identities_under(tmp.path(), &mut dirs, &mut seen, 2);
        assert_eq!(dirs, vec![projects]);
    }

    #[test]
    fn scan_isolated_identities_does_not_descend_past_max_depth() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Three levels deep -- depth 2 must not find this.
        let projects = tmp
            .path()
            .join("main")
            .join("a1b2c3d4")
            .join("extra-nesting")
            .join("identities")
            .join("bundle-3")
            .join("claude")
            .join("projects");
        std::fs::create_dir_all(&projects).unwrap();

        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::scan_isolated_identities_under(tmp.path(), &mut dirs, &mut seen, 2);
        assert!(dirs.is_empty(), "must not walk past the bounded depth");
    }

    #[test]
    fn scan_isolated_identities_ignores_directories_with_no_identities_subdir() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("some-channel").join("data")).unwrap();

        let mut dirs = Vec::new();
        let mut seen = HashSet::new();
        ClaudeHistoryAdapter::scan_isolated_identities_under(tmp.path(), &mut dirs, &mut seen, 2);
        assert!(dirs.is_empty());
    }
}
