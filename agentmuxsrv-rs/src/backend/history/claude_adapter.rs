// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Claude Code history adapter.
//! Scans ~/.claude/projects/ and ~/.config/claude-*/projects/ for session JSONL files.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::adapter::*;

pub struct ClaudeHistoryAdapter {
    /// All base directories to scan for project folders.
    base_dirs: Vec<PathBuf>,
}

impl ClaudeHistoryAdapter {
    pub fn new() -> Self {
        let mut base_dirs = Vec::new();

        if let Some(home) = dirs::home_dir() {
            // User's personal Claude sessions
            let personal = home.join(".claude").join("projects");
            if personal.is_dir() {
                base_dirs.push(personal);
            }

            // AgentMux agent sessions: ~/.config/claude-*/projects/
            let config_dir = home.join(".config");
            if config_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&config_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with("claude-") {
                            let projects = entry.path().join("projects");
                            if projects.is_dir() {
                                base_dirs.push(projects);
                            }
                        }
                    }
                }
            }
        }

        ClaudeHistoryAdapter { base_dirs }
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
