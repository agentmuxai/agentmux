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

/// Compute `$CLAUDE_CONFIG_DIR/projects/<sanitized>/memory/` for the given
/// working directory and Claude config dir.
///
/// `claude_config_dir` is the value of `CLAUDE_CONFIG_DIR` from the agent's
/// stored env blob. When empty, falls back to
/// `~/.agentmux/shared/providers/claude/` — the default isolated home that
/// `app_api.rs` sets at agent spawn time. We never write to the global
/// `~/.claude/projects/` because AgentMux always sets `CLAUDE_CONFIG_DIR`.
///
/// Sanitization mirrors Claude Code's `sessionStoragePortable.ts`:
/// 1. Replace every non-alphanumeric char with `-`.
/// 2. If the result is longer than 200 chars, truncate at 200 and append a
///    base-36 hash of the *raw* working_directory (before sanitization).
fn memory_dir_for_cwd(claude_config_dir: &str, working_directory: &str) -> PathBuf {
    let sanitized: String = working_directory
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    let folder_name = if sanitized.len() > 200 {
        let hash = djb2_hash(working_directory);
        let truncated = &sanitized[..200];
        format!("{truncated}-{}", radix_36(hash))
    } else {
        sanitized
    };

    let base = if claude_config_dir.is_empty() {
        expand_home_dir_safe("~/.agentmux/shared/providers/claude")
    } else {
        expand_home_dir_safe(claude_config_dir)
    };
    base.join("projects").join(folder_name).join("memory")
}

/// Extract `CLAUDE_CONFIG_DIR` from a `KEY=VALUE\n…` env blob.
fn parse_claude_config_dir(env_blob: &str) -> String {
    for line in env_blob.lines() {
        if let Some(val) = line.strip_prefix("CLAUDE_CONFIG_DIR=") {
            return val.to_string();
        }
    }
    String::new()
}

/// Resolve the memory directory for `agent_id`. Reads the agent definition and
/// its stored env blob to find `CLAUDE_CONFIG_DIR`. Returns an error if the
/// agent is not found or has no working directory.
///
/// Shared by `native_memory_handlers` and the `memory.*` App API handlers.
pub(crate) fn memory_dir_for_agent(
    wstore: &crate::backend::storage::store::Store,
    agent_id: &str,
) -> Result<std::path::PathBuf, String> {
    // agent_id arriving from App API is the agent slug (AGENTMUX_AGENT_ID /
    // bus:register id), not a UUID and not the literal display name — use
    // instance_get_by_slug (agent_def_get queries by UUID and would always
    // return None here; instance_get_by_name matches the display name, a
    // different namespace — see that function's own doc comment).
    if let Some(instance) = wstore
        .instance_get_by_slug(agent_id)
        .map_err(|e| format!("memory: store: {e}"))?
    {
        if instance.working_directory.is_empty() {
            return Err(format!("memory: agent {agent_id} has no working directory"));
        }
        let config_dir = wstore
            .agent_content_get(&instance.id, "env")
            .ok()
            .flatten()
            .map(|c| parse_claude_config_dir(&c.content))
            .unwrap_or_default();
        return Ok(memory_dir_for_cwd(&config_dir, &instance.working_directory));
    }

    // No persisted db_agents instance row. Running agents are tracked in the
    // global named-agent registry, not db_agents (launching an agent does not
    // create an instance row there), so the slug lookup above misses every live
    // agent. Fall back to the registry, which records the slug → working_dir +
    // identity binding needed to locate the agent's isolated memory dir.
    // See issue #1836.
    memory_dir_from_registry(agent_id)
        .ok_or_else(|| format!("memory: agent {agent_id} not found"))
}

/// Find the global named-agent registry's active record for `agent_id` —
/// the `AGENTMUX_AGENT_ID` routing slug (`derive_slug(display_name)`),
/// NOT the record's own `instance_name` (which keeps the original
/// display casing, e.g. "AgentY"). Matching by raw string equality here
/// used to mean this — and every App-API self-lookup endpoint that falls
/// back to it (`memory.*`, and `bundle.self.get` once it's wired to use
/// this too) — silently 404'd for any agent whose name wasn't already
/// all-lowercase. `derive_slug` on both sides makes the comparison
/// consistent with how the slug was actually derived in the first place.
/// Shared so callers outside this module (e.g. `app_api::mod::
/// bundle_self_get_impl`) don't reimplement the same lookup.
pub(crate) fn find_active_registry_record_by_slug(
    agent_id: &str,
) -> Option<crate::registry::NamedAgentRecord> {
    let registry_dir = crate::registry::resolve_shared_registry_dir()?;
    let registry = crate::registry::Registry::open(registry_dir).ok()?;
    let queried_slug = crate::backend::storage::store::derive_slug(agent_id);
    registry
        .list_active()
        .ok()?
        .into_iter()
        .find(|r| crate::backend::storage::store::derive_slug(&r.data.instance_name) == queried_slug)
}

/// Resolve a memory dir for `agent_id` from the global named-agent registry.
///
/// Each live agent is recorded by `instance_name` with a `working_dir` relative
/// to `source_agents_base` (the channel/dev instance it lives in) and an
/// `identity_id` that determines its `CLAUDE_CONFIG_DIR` root. Returns `None`
/// when the registry is unavailable or no active record matches the slug.
fn memory_dir_from_registry(agent_id: &str) -> Option<std::path::PathBuf> {
    let rec = find_active_registry_record_by_slug(agent_id)?;

    // Reconstruct the absolute working directory: source_agents_base joined with
    // the relative working_dir. Legacy (v1/v2) records without a base fall back
    // to the current channel's agents dir (AGENTMUX_AGENTS_DIR), matching the
    // registry's own pre-P0.4 reconstruction rule.
    let base = rec
        .data
        .source_agents_base
        .clone()
        .or_else(|| std::env::var("AGENTMUX_AGENTS_DIR").ok())?;
    let working_directory = std::path::Path::new(&base)
        .join(&rec.data.working_dir)
        .to_string_lossy()
        .to_string();

    let config_dir = claude_config_dir_for_identity(rec.data.identity_id.as_deref());
    Some(memory_dir_for_cwd(&config_dir, &working_directory))
}

/// Compute the `CLAUDE_CONFIG_DIR` root for an agent bound to `identity_id`,
/// mirroring the spawn-time isolated-home layout (`OAuthConfigDir`):
///   - unbound / "default" → `<shared>/providers/claude` (the default home;
///     returned as an empty string so `memory_dir_for_cwd` applies its own
///     identical fallback)
///   - a per-identity bundle → `<shared>/identities/<id>/claude`
fn claude_config_dir_for_identity(identity_id: Option<&str>) -> String {
    match identity_id {
        Some(id) if !id.is_empty() && id != "default" => {
            let shared = std::env::var_os("AGENTMUX_SHARED_DIR")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".agentmux").join("shared")));
            match shared {
                Some(s) => s
                    .join("identities")
                    .join(id)
                    .join("claude")
                    .to_string_lossy()
                    .to_string(),
                None => String::new(),
            }
        }
        _ => String::new(),
    }
}

/// Validate a memory filename.
pub(crate) fn validate_memory_filename(filename: &str) -> Result<(), String> {
    validate_filename(filename)
}

/// Parse `metadata.type` from YAML frontmatter (re-exported for App API).
pub(crate) fn parse_memory_frontmatter_type(content: &str) -> Option<String> {
    parse_frontmatter_type(content)
}

/// Hash matching Claude Code's sessionStoragePortable.ts implementation.
/// JS `charCodeAt()` iterates UTF-16 code units (two surrogates per non-BMP char);
/// Rust `chars()` iterates Unicode scalar values — they diverge for emoji/non-BMP.
/// `encode_utf16()` produces the same UTF-16 unit stream as JS, so the hashes match.
fn djb2_hash(s: &str) -> u32 {
    let mut hash: i32 = 0;
    for unit in s.encode_utf16() {
        hash = hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(unit as i32);
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
    // Tmp path is ".{filename}.{uuid}.tmp" (+42 chars); cap stem at 200 to stay
    // well under the 255-byte filesystem limit and avoid ENAMETOOLONG.
    if stem.len() > 200 {
        return Err(format!("filename stem too long ({} chars, max 200)", stem.len()));
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

                let config_dir = wstore
                    .agent_content_get(&agent.id, "env")
                    .ok().flatten()
                    .map(|c| parse_claude_config_dir(&c.content))
                    .unwrap_or_default();
                let memory_dir = memory_dir_for_cwd(&config_dir, &agent.working_directory);

                let mut files: Vec<NativeMemoryFileMeta> = Vec::new();
                // Treat both "dir doesn't exist" and the TOCTOU case where the dir
                // is deleted between exists() and read_dir() as an empty result.
                let entries = match std::fs::read_dir(&memory_dir) {
                    Ok(e) => e,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(Some(serde_json::to_value(NativeMemoryListResult { files: vec![] }).map_err(|e| e.to_string())?));
                    }
                    Err(e) => return Err(format!("agent:memory:list: read_dir: {e}")),
                };

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

                let config_dir = wstore
                    .agent_content_get(&agent.id, "env")
                    .ok().flatten()
                    .map(|c| parse_claude_config_dir(&c.content))
                    .unwrap_or_default();
                let path = memory_dir_for_cwd(&config_dir, &agent.working_directory).join(&cmd.filename);
                // Reject symlinks — consistent with list handler's file_type check.
                let file_type = std::fs::symlink_metadata(&path)
                    .map_err(|e| format!("agent:memory:read_file: {}: {e}", cmd.filename))?
                    .file_type();
                if !file_type.is_file() {
                    return Err(format!("agent:memory:read_file: {} is not a regular file", cmd.filename));
                }
                // Cap at 10 MiB; use read_to_end + from_utf8_lossy so a boundary
                // mid-UTF-8 sequence doesn't surface as InvalidData.
                const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
                let mut buf = Vec::new();
                std::fs::File::open(&path)
                    .and_then(|f| {
                        use std::io::Read;
                        f.take(MAX_READ_BYTES).read_to_end(&mut buf)
                    })
                    .map_err(|e| format!("agent:memory:read_file: {}: {e}", cmd.filename))?;
                let content = String::from_utf8_lossy(&buf).into_owned();

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

                let config_dir = wstore
                    .agent_content_get(&agent.id, "env")
                    .ok().flatten()
                    .map(|c| parse_claude_config_dir(&c.content))
                    .unwrap_or_default();
                let dir = memory_dir_for_cwd(&config_dir, &agent.working_directory);
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:write_file: mkdir: {e}"))?;

                let dest = dir.join(&cmd.filename);
                // Per-write UUID suffix prevents concurrent writes to the same
                // filename from sharing a tmp path and silently corrupting each
                // other's content (reagent P1 on PR #1588).
                let tmp = dir.join(format!(".{}.{}.tmp", cmd.filename, uuid::Uuid::new_v4()));

                // Clean up tmp on both write failure (partial file) and rename failure.
                if let Err(e) = std::fs::write(&tmp, &cmd.content) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:write_file: write tmp: {e}"));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    // Process-global env access (AGENTMUX_SHARED_DIR / AGENTMUX_HOME_OVERRIDE
    // both feed registry::paths::resolve_global_shared_root) — a module-local
    // lock only serializes tests within THIS file; registry::paths's own
    // test module touches the same resolution path and already uses this
    // crate-wide lock for exactly that reason (see test_support.rs's doc
    // comment). Reusing it here avoids reintroducing the cross-module race
    // it was built to prevent.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    #[test]
    fn config_dir_for_default_identity_is_empty() {
        // Unbound / "default" agents use the shared default home; we return an
        // empty string so memory_dir_for_cwd applies its own providers/claude
        // fallback rather than duplicating it here.
        assert_eq!(claude_config_dir_for_identity(None), "");
        assert_eq!(claude_config_dir_for_identity(Some("")), "");
        assert_eq!(claude_config_dir_for_identity(Some("default")), "");
    }

    #[test]
    fn config_dir_for_bound_identity_points_at_bundle() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        std::env::set_var("AGENTMUX_SHARED_DIR", "/home/u/.agentmux/shared");

        let got = claude_config_dir_for_identity(Some("bundle-x"));
        let want = PathBuf::from("/home/u/.agentmux/shared")
            .join("identities")
            .join("bundle-x")
            .join("claude")
            .to_string_lossy()
            .to_string();
        assert_eq!(got, want);

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
    }

    // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md follow-up: confirmed
    // live — a real agent named "AgentY" (routing slug "agenty") got
    // "memory: agent agenty not found" from `MemoryList`, because this
    // lookup used to compare the slug against the registry's own
    // display-cased `instance_name` with raw string equality.
    #[test]
    fn find_active_registry_record_by_slug_resolves_a_mixed_case_display_name() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

        let registry_dir = tmp.path().join("agents").join("registry");
        let registry = crate::registry::Registry::open(registry_dir).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 1,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-agenty".to_string(),
                    instance_name: "AgentY".to_string(),
                    definition_id: "def-agenty".to_string(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    working_dir: "agenty-0629j".to_string(),
                    source_agents_base: None,
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let found = find_active_registry_record_by_slug("agenty");
        assert!(found.is_some(), "must resolve via the slug-normalized fallback");
        assert_eq!(found.unwrap().data.instance_name, "AgentY");

        let not_found = find_active_registry_record_by_slug("someone-else");
        assert!(not_found.is_none(), "an unrelated slug must not match by coincidence");

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
    }

    #[test]
    fn memory_dir_for_cwd_default_root_matches_spawn_layout() {
        // Empty config dir → the default isolated home under shared, with the
        // working dir sanitized the same way Claude Code encodes project dirs.
        // Assert on path components so mixed separators (Windows) don't matter.
        let dir = memory_dir_for_cwd("", "/work/proj");
        let comps: Vec<String> = dir
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let want_tail = ["providers", "claude", "projects", "-work-proj", "memory"];
        let tail = &comps[comps.len() - want_tail.len()..];
        assert_eq!(tail, want_tail, "unexpected memory dir tail: {comps:?}");
    }
}
