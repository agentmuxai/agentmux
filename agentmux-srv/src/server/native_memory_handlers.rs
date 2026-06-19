// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Native memory RPCs — read/write the agent's `~/.claude/projects/<sanitized>/memory/`
//! folder that Claude Code uses for autonomous, cross-session fact storage.
//!
//! Three commands:
//!   agent:memory:list       — list *.md files in the memory dir
//!   agent:memory:read_file  — read one file by filename (no path traversal)
//!   agent:memory:write_file — write/create one file atomically (tmp→rename)
//!
//! Spec: docs/specs/SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §7

use std::sync::Arc;
use std::path::PathBuf;

use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_NATIVE_MEMORY_LIST,
    COMMAND_NATIVE_MEMORY_READ_FILE,
    COMMAND_NATIVE_MEMORY_WRITE_FILE,
    CommandNativeMemoryListData,
    CommandNativeMemoryReadFileData,
    CommandNativeMemoryWriteFileData,
    NativeMemoryFileMeta,
    NativeMemoryListResult,
    NativeMemoryReadFileResult,
};

use super::AppState;

/// Compute `~/.claude/projects/<sanitized>/memory/` for the given working directory.
///
/// Mirrors Claude Code's `sessionStoragePortable.ts` algorithm:
/// 1. Replace every non-alphanumeric char with `-`.
/// 2. If the result is longer than 200 chars, truncate at 200 and append a
///    base-36 djb2 hash of the *full* sanitized string as a suffix.
fn memory_dir_for_cwd(working_directory: &str) -> PathBuf {
    let sanitized: String = working_directory
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    let folder_name = if sanitized.len() > 200 {
        let hash = djb2_hash(&sanitized);
        let truncated = &sanitized[..200];
        format!("{truncated}-{}", radix_36(hash))
    } else {
        sanitized
    };

    expand_home_dir_safe("~/.claude/projects")
        .join(folder_name)
        .join("memory")
}

/// Hash matching Claude Code's sessionStoragePortable.ts implementation:
///   seed = 0, multiplier = 31 via `(h << 5) - h + c` with i32 overflow,
///   result = Math.abs(hash) (unsigned_abs).
/// This differs from classic djb2 (seed=5381, multiplier=33).
fn djb2_hash(s: &str) -> u32 {
    let mut hash: i32 = 0;
    for c in s.chars() {
        hash = hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(c as i32);
    }
    hash.unsigned_abs()
}

/// Convert a u32 to a base-36 string (lowercase, same as Number.toString(36) in JS).
fn radix_36(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap_or_else(|_| "0".to_string())
}

/// Validate a filename: alphanumeric + `-_`, must end with `.md`, no path separators.
fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() {
        return Err("filename must not be empty".to_string());
    }
    if !filename.ends_with(".md") {
        return Err(format!("filename must end with .md, got: {filename}"));
    }
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(format!("filename must not contain path separators: {filename}"));
    }
    let stem = &filename[..filename.len() - 3];
    if stem.is_empty() {
        return Err("filename stem must not be empty (.md is not a valid name)".to_string());
    }
    if !stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!(
            "filename stem must be alphanumeric + '-_', got: {stem}"
        ));
    }
    Ok(())
}

/// Extract `metadata.type` from YAML frontmatter.
/// Claude Code memory files nest the type under `metadata:`:
///   metadata:
///     type: user
/// A top-level `type:` key is NOT the correct field.
fn parse_frontmatter_type(content: &str) -> Option<String> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let rest = content.strip_prefix("---")?.trim_start_matches('\n');
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let mut in_metadata = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim_end();
        if trimmed == "metadata:" {
            in_metadata = true;
            continue;
        }
        if in_metadata {
            // A non-indented, non-empty line exits the metadata block.
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                break;
            }
            if let Some(val) = line.trim_start().strip_prefix("type:") {
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

pub fn register_native_memory_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore_list = state.wstore.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_LIST,
        Box::new(move |data, _ctx| {
            let wstore = wstore_list.clone();
            Box::pin(async move {
                let cmd: CommandNativeMemoryListData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:memory:list: {e}"))?;

                let agent = wstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:list: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:list: agent {} not found", cmd.agent_id))?;

                // No working directory → no memory path (avoid mapping to the shared
                // ~/.claude/projects/memory/ directory — reagent P1 on PR #1588).
                if agent.working_directory.is_empty() {
                    return Ok(Some(serde_json::to_value(NativeMemoryListResult { files: vec![] }).map_err(|e| e.to_string())?));
                }

                let memory_dir = memory_dir_for_cwd(&agent.working_directory);

                if !memory_dir.exists() {
                    return Ok(Some(serde_json::to_value(NativeMemoryListResult { files: vec![] }).map_err(|e| e.to_string())?));
                }

                let mut files: Vec<NativeMemoryFileMeta> = Vec::new();
                let entries = std::fs::read_dir(&memory_dir)
                    .map_err(|e| format!("agent:memory:list: read_dir: {e}"))?;

                for entry in entries {
                    let entry = entry.map_err(|e| format!("agent:memory:list: read_dir entry: {e}"))?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !name.ends_with(".md") {
                        continue;
                    }
                    // Reject symlinks — entry.file_type() does NOT follow symlinks.
                    let file_type = entry
                        .file_type()
                        .map_err(|e| format!("agent:memory:list: file_type {name}: {e}"))?;
                    if !file_type.is_file() {
                        continue;
                    }
                    // The file may be deleted after read_dir but before metadata —
                    // skip the entry on NotFound rather than aborting the whole listing.
                    let meta = match entry.metadata() {
                        Ok(m) => m,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(format!("agent:memory:list: metadata {name}: {e}")),
                    };
                    let size_bytes = meta.len();
                    let modified_at = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);

                    // Read up to 512 bytes for frontmatter type parsing.
                    // Take + read_to_end loops internally to fill the buffer —
                    // a single read() call may return fewer bytes on the first try.
                    let preview_content = {
                        use std::io::Read;
                        std::fs::File::open(entry.path())
                            .map(|f| {
                                let mut buf = Vec::with_capacity(512);
                                f.take(512).read_to_end(&mut buf).ok();
                                String::from_utf8_lossy(&buf).into_owned()
                            })
                            .unwrap_or_default()
                    };
                    let metadata_type = parse_frontmatter_type(&preview_content);
                    let is_index = name == "MEMORY.md";

                    files.push(NativeMemoryFileMeta {
                        filename: name,
                        is_index,
                        metadata_type,
                        size_bytes,
                        modified_at,
                    });
                }

                // MEMORY.md first, then alphabetical
                files.sort_by(|a, b| {
                    b.is_index.cmp(&a.is_index).then(a.filename.cmp(&b.filename))
                });

                Ok(Some(serde_json::to_value(NativeMemoryListResult { files }).map_err(|e| e.to_string())?))
            })
        }),
    );

    let wstore_read = state.wstore.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_READ_FILE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_read.clone();
            Box::pin(async move {
                let cmd: CommandNativeMemoryReadFileData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:memory:read_file: {e}"))?;

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:read_file: {e}"))?;

                let agent = wstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:read_file: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:read_file: agent {} not found", cmd.agent_id))?;

                if agent.working_directory.is_empty() {
                    return Err(format!("agent:memory:read_file: agent {} has no configured working directory", cmd.agent_id));
                }

                let path = memory_dir_for_cwd(&agent.working_directory).join(&cmd.filename);
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("agent:memory:read_file: {}: {e}", cmd.filename))?;

                Ok(Some(serde_json::to_value(NativeMemoryReadFileResult { content }).map_err(|e| e.to_string())?))
            })
        }),
    );

    let wstore_write = state.wstore.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_write.clone();
            Box::pin(async move {
                let cmd: CommandNativeMemoryWriteFileData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:memory:write_file: {e}"))?;

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:write_file: {e}"))?;

                const MAX_CONTENT_BYTES: usize = 10 * 1024 * 1024; // 10 MiB
                if cmd.content.len() > MAX_CONTENT_BYTES {
                    return Err(format!(
                        "agent:memory:write_file: content too large ({} bytes, max {})",
                        cmd.content.len(),
                        MAX_CONTENT_BYTES,
                    ));
                }

                let agent = wstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:write_file: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:write_file: agent {} not found", cmd.agent_id))?;

                if agent.working_directory.is_empty() {
                    return Err(format!("agent:memory:write_file: agent {} has no configured working directory", cmd.agent_id));
                }

                let dir = memory_dir_for_cwd(&agent.working_directory);
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:write_file: mkdir: {e}"))?;

                let dest = dir.join(&cmd.filename);
                // Per-write UUID suffix prevents concurrent writes to the same
                // filename from sharing a tmp path and silently corrupting each
                // other's content (reagent P1 on PR #1588).
                let tmp = dir.join(format!(".{}.{}.tmp", cmd.filename, uuid::Uuid::new_v4()));

                std::fs::write(&tmp, &cmd.content)
                    .map_err(|e| format!("agent:memory:write_file: write tmp: {e}"))?;
                if let Err(e) = std::fs::rename(&tmp, &dest) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:write_file: rename: {e}"));
                }

                tracing::info!(
                    agent_id = %cmd.agent_id,
                    filename = %cmd.filename,
                    bytes = cmd.content.len(),
                    "agent:memory:write_file"
                );
                Ok(None)
            })
        }),
    );
}
