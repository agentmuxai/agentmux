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
//!
//! All three also write through into `db_agent_native_memory` (via
//! `state.id_store`) — a durable mirror keyed by the stable
//! `AgentDefinition.id`, since the live filesystem path above is
//! channel-relative by design and not the same across channels/instances for
//! the same logical agent. `list`/`read_file` merge the live-FS view with
//! the mirror so a file written from one channel stays visible from another.
//! See docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md.

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

/// Cap for both live-FS reads and mirror upserts — one shared limit so a
/// file that's readable stays within the size the mirror can also store.
const MAX_MEMORY_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Refresh `db_agent_native_memory` from the live filesystem for `agent_id`
/// — the same read-then-upsert-if-changed pass `agent:memory:list` performs
/// inline (compare each live file's size+mtime against the mirror, and
/// upsert only when it's actually changed), extracted so a second caller
/// can trigger the identical refresh without duplicating the walk.
///
/// ABF v0.2 §2.3 (`bundle.export_for_agent`) needs this: `list`/`read_file`
/// only sync the mirror on a Stash Memory tab open, so exporting straight
/// from `db_agent_native_memory` without refreshing first would silently
/// omit anything Claude wrote autonomously since the tab was last opened
/// (or if it was never opened at all) — see
/// `SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`
/// §2.3's second revision note.
///
/// Errors only on a genuine `read_dir` failure (permissions, I/O) — a
/// missing directory (never written, or wiped) is not an error, matching
/// `list`'s own treatment. A per-file upsert failure is logged and
/// swallowed (non-fatal), same as `list`.
///
/// Returns the filenames of any file whose real on-disk size exceeded
/// [`MAX_MEMORY_FILE_BYTES`] — reagent P2, PR #2527: `list`'s original
/// inline version of this logic silently truncates via
/// `take(MAX_MEMORY_FILE_BYTES)` with no signal that it happened, so an
/// export→import round trip through `bundle.export_for_agent` could
/// permanently lose the tail of a large file with no warning anywhere.
/// This function still truncates the same way (the cap itself is
/// unchanged — still generous for any legitimate memory file), but now
/// reports which files it happened to, so a caller that surfaces
/// warnings (like `bundle.export_for_agent`) can tell the user.
pub(crate) fn refresh_memory_mirror_from_live_fs(
    agent_id: &str,
    memory_dir: &std::path::Path,
    id_store: &crate::backend::storage::store::Store,
) -> Result<Vec<String>, String> {
    let mirrored_meta: std::collections::HashMap<String, (i64, i64)> = id_store
        .agent_native_memory_list_meta(agent_id)
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.filename, (row.size_bytes, row.last_seen_mtime_ms)))
        .collect();

    let entries = match std::fs::read_dir(memory_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("refresh_memory_mirror_from_live_fs: read_dir: {e}")),
    };

    let mut truncated_files: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("refresh_memory_mirror_from_live_fs: read_dir entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue, // TOCTOU-deleted between read_dir and here; skip, not fatal.
        };
        if !file_type.is_file() {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size_bytes = meta.len();
        let modified_at = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let unchanged_since_last_mirror = mirrored_meta.get(&name) == Some(&(size_bytes as i64, modified_at));
        if unchanged_since_last_mirror {
            continue;
        }
        if size_bytes > MAX_MEMORY_FILE_BYTES {
            truncated_files.push(name.clone());
        }
        let full_content_read = {
            use std::io::Read;
            std::fs::File::open(entry.path()).and_then(|f| {
                let mut buf = Vec::new();
                f.take(MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf)?;
                Ok(buf)
            })
        };
        match full_content_read {
            Ok(buf) => {
                let full_content = String::from_utf8_lossy(&buf).into_owned();
                let full_metadata_type = parse_frontmatter_type(&full_content);
                if let Err(e) = id_store.agent_native_memory_upsert(
                    agent_id,
                    &name,
                    &full_content,
                    full_metadata_type.as_deref(),
                    &entry.path().to_string_lossy(),
                    size_bytes as i64,
                    modified_at,
                ) {
                    tracing::warn!(agent_id, filename = %name, error = %e, "refresh_memory_mirror_from_live_fs: mirror upsert failed (non-fatal)");
                }
            }
            Err(e) => {
                tracing::warn!(agent_id, filename = %name, error = %e, "refresh_memory_mirror_from_live_fs: full-content read failed, skipping this round (non-fatal)");
            }
        }
    }
    Ok(truncated_files)
}

/// Resolve the live memory directory for an agent identified by its
/// `AgentDefinition.id` (UUID) — the same identifier convention
/// `agent:memory:list`/`read_file`/`write_file` use (as opposed to
/// [`memory_dir_for_agent`]'s slug-based lookup, used by different, App-API
/// callers). Shared by those three handlers and `bundle.export_for_agent`/
/// `bundle.import_for_agent` (ABF v0.2 §2.3) so all five resolve identically.
pub(crate) fn memory_dir_for_agent_by_id(
    wstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
) -> Option<std::path::PathBuf> {
    if agent.working_directory.is_empty() {
        return None;
    }
    let config_dir = wstore
        .agent_content_get(&agent.id, "env")
        .ok()
        .flatten()
        .map(|c| parse_claude_config_dir(&c.content))
        .unwrap_or_default();
    Some(memory_dir_for_cwd(&config_dir, &agent.working_directory))
}

pub fn register_native_memory_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore_list = state.wstore.clone();
    let id_store_list = state.id_store.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_LIST,
        Box::new(move |data, _ctx| {
            let wstore = wstore_list.clone();
            let id_store = id_store_list.clone();
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

                // Existing mirror metadata (no content) for this agent, keyed by
                // filename — lets the loop below skip the expensive full-content
                // read+upsert for a file that hasn't changed since it was last
                // mirrored, instead of doing it unconditionally on every list
                // call (reagent P1 on PR #2459: `list` fires on every Stash
                // Memory tab open/refresh, so an unconditional full read + SQLite
                // write per file would mean synchronous, potentially many-MB
                // disk I/O on a call meant to be a lightweight metadata listing).
                //
                // Compares BOTH size and mtime, not size alone: a same-byte-length
                // edit is common (e.g. correcting a typo) and size-only comparison
                // would silently leave the mirror stale for it. This matters for
                // exactly the case this table exists for — a channel with no live
                // copy of a file relies entirely on the mirror (reagent P1 on
                // PR #2459's first pass: read_file only "always re-reads fresh"
                // on the channel that still HAS a live copy; a channel that never
                // did has nothing to self-correct with, so a stale mirror row
                // there is permanent, not "briefly stale").
                let mirrored_meta: std::collections::HashMap<String, (i64, i64)> = id_store
                    .agent_native_memory_list_meta(&agent.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|row| (row.filename, (row.size_bytes, row.last_seen_mtime_ms)))
                    .collect();

                let mut files: Vec<NativeMemoryFileMeta> = Vec::new();
                let mut live_filenames: std::collections::HashSet<String> = std::collections::HashSet::new();
                // Only a missing directory (never-yet-written, or wiped after this
                // channel last had files) is treated as "no live files" — any
                // other read_dir error (permissions, I/O) propagates, matching the
                // pre-mirror behavior (reagent P2 on PR #2459: silently swallowing
                // every error here would hide a real access problem behind what
                // looks like an empty listing). The mirror merge below still runs
                // regardless, so a wiped live folder doesn't lose durability for
                // files it mirrored previously.
                let entries = match std::fs::read_dir(&memory_dir) {
                    Ok(e) => Some(e),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(format!("agent:memory:list: read_dir: {e}")),
                };

                if let Some(entries) = entries {
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

                        // Read up to 512 bytes for frontmatter type parsing — same
                        // cheap preview the pre-mirror code used. Take + read_to_end
                        // loops internally to fill the buffer.
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

                        let unchanged_since_last_mirror =
                            mirrored_meta.get(&name) == Some(&(size_bytes as i64, modified_at));
                        if !unchanged_since_last_mirror {
                            // A read failure here (permission change, TOCTOU
                            // delete, AV/NFS lock, concurrent editor) must NOT
                            // upsert an empty string — that would overwrite any
                            // previously-durable mirrored content, destroying the
                            // exact cross-channel durability guarantee this table
                            // exists for on the very first transient read hiccup
                            // (reagent P0 on PR #2459). Skip the upsert entirely
                            // this round instead; it retries on the next list().
                            let full_content_read = {
                                use std::io::Read;
                                std::fs::File::open(entry.path()).and_then(|f| {
                                    let mut buf = Vec::new();
                                    f.take(MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf)?;
                                    Ok(buf)
                                })
                            };
                            match full_content_read {
                                Ok(buf) => {
                                    let full_content = String::from_utf8_lossy(&buf).into_owned();
                                    let full_metadata_type = parse_frontmatter_type(&full_content);
                                    if let Err(e) = id_store.agent_native_memory_upsert(
                                        &agent.id,
                                        &name,
                                        &full_content,
                                        full_metadata_type.as_deref(),
                                        &entry.path().to_string_lossy(),
                                        size_bytes as i64,
                                        modified_at,
                                    ) {
                                        tracing::warn!(agent_id = %agent.id, filename = %name, error = %e, "agent:memory:list: mirror upsert failed (non-fatal)");
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(agent_id = %agent.id, filename = %name, error = %e, "agent:memory:list: full-content read failed, skipping mirror upsert this round (non-fatal)");
                                }
                            }
                        }
                        live_filenames.insert(name.clone());

                        files.push(NativeMemoryFileMeta {
                            filename: name,
                            is_index,
                            metadata_type,
                            size_bytes,
                            modified_at,
                        });
                    }
                }

                // Merge in mirror-only files — present in a different channel's
                // write (or the live folder was wiped) but not on this channel's
                // live FS. Served transparently, with no distinguishing treatment.
                match id_store.agent_native_memory_list_meta(&agent.id) {
                    Ok(mirrored) => {
                        for row in mirrored {
                            if live_filenames.contains(&row.filename) {
                                continue;
                            }
                            files.push(NativeMemoryFileMeta {
                                is_index: row.filename == "MEMORY.md",
                                metadata_type: row.metadata_type,
                                size_bytes: row.size_bytes as u64,
                                modified_at: row.last_seen_mtime_ms,
                                filename: row.filename,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(agent_id = %agent.id, error = %e, "agent:memory:list: mirror list failed (non-fatal)");
                    }
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
    let id_store_read = state.id_store.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_READ_FILE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_read.clone();
            let id_store = id_store_read.clone();
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

                // Live FS is the freshest copy when present — Claude may have
                // written moments ago, before this call's mirror upsert even
                // runs. Fall back to the mirror only when the live file is
                // genuinely absent (a different channel's write, or the live
                // folder was wiped) — a live path that exists but fails to
                // read for another reason (permissions, non-regular file)
                // still surfaces as an error rather than silently masking it
                // with stale mirrored content.
                // Distinguish "genuinely absent" (fall back to the mirror) from
                // every other symlink_metadata outcome, matching the
                // pre-durable-sync behavior exactly: a real access error
                // (permissions, I/O) must surface as an error, not silently
                // fall back to possibly-stale mirrored content, and an existing
                // non-regular-file path must be explicitly rejected, not treated
                // as "absent" either (reagent P1 on PR #2459, second pass —
                // collapsing every outcome into "absent" the first time around
                // could serve stale content or a misleading "not found" for a
                // path that actually exists but errored or is the wrong type).
                let content = match std::fs::symlink_metadata(&path) {
                    Ok(live_meta) if live_meta.file_type().is_file() => {
                        let mut buf = Vec::new();
                        std::fs::File::open(&path)
                            .and_then(|f| {
                                use std::io::Read;
                                f.take(MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf)
                            })
                            .map_err(|e| format!("agent:memory:read_file: {}: {e}", cmd.filename))?;
                        let content = String::from_utf8_lossy(&buf).into_owned();

                        let metadata_type = parse_frontmatter_type(&content);
                        let mtime_ms = live_meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        if let Err(e) = id_store.agent_native_memory_upsert(
                            &agent.id,
                            &cmd.filename,
                            &content,
                            metadata_type.as_deref(),
                            &path.to_string_lossy(),
                            live_meta.len() as i64,
                            mtime_ms,
                        ) {
                            tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:read_file: mirror upsert failed (non-fatal)");
                        }
                        content
                    }
                    Ok(_) => {
                        return Err(format!("agent:memory:read_file: {} is not a regular file", cmd.filename));
                    }
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("agent:memory:read_file: {}: {e}", cmd.filename));
                    }
                    Err(_) => match id_store.agent_native_memory_read(&agent.id, &cmd.filename) {
                        Ok(Some(mirrored)) => mirrored,
                        Ok(None) => {
                            return Err(format!("agent:memory:read_file: {}: not found", cmd.filename));
                        }
                        Err(e) => {
                            return Err(format!("agent:memory:read_file: {}: not found on this channel and mirror lookup failed: {e}", cmd.filename));
                        }
                    }
                };

                Ok(Some(serde_json::to_value(NativeMemoryReadFileResult { content }).map_err(|e| e.to_string())?))
            })
        }),
    );

    let wstore_write = state.wstore.clone();
    let id_store_write = state.id_store.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_write.clone();
            let id_store = id_store_write.clone();
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

                let metadata_type = parse_frontmatter_type(&cmd.content);
                // Re-stat the just-written file for its real on-disk size/mtime
                // rather than deriving them from cmd.content here — keeps this
                // in exact agreement with what a subsequent list() will compute,
                // so the size+mtime change check there doesn't spuriously treat
                // this write as "changed again" due to clock/precision drift.
                let dest_meta = std::fs::metadata(&dest).ok();
                let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(cmd.content.len() as i64);
                let mtime_ms = dest_meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Err(e) = id_store.agent_native_memory_upsert(
                    &agent.id,
                    &cmd.filename,
                    &cmd.content,
                    metadata_type.as_deref(),
                    &dest.to_string_lossy(),
                    size_bytes,
                    mtime_ms,
                ) {
                    tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:write_file: mirror upsert failed (non-fatal)");
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

    // ---- Durable sync integration tests ----------------------------------
    // SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md §5: simulate two
    // "channels" against the same agent.id by pointing each AppState's
    // wstore at a different working_directory (so memory_dir_for_cwd
    // resolves two different live paths) while sharing one id_store — the
    // same topology production uses (each channel's own objects.db caches
    // the same global AgentDefinition.id; one shared store.db backs id_store).

    use crate::backend::rpc::engine::WshRpcEngine;
    use crate::backend::storage::store::Store;
    use crate::backend::storage::AgentDefinition;
    use crate::backend::rpc_types::{RpcMessage};

    fn agent_def(id: &str, working_directory: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: id.to_string(),
            name: "Test Agent".to_string(),
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
            agent_type: "host".to_string(),
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
            model_vendor_base_url: String::new(),
        }
    }

    /// Build a channel's AppState: its own per-channel wstore (holding the
    /// agent definition, keyed by the same `agent_id` every channel shares)
    /// plus the given shared `id_store` (the durable mirror).
    /// `claude_config_dir` must be a per-test temp directory — an empty
    /// value would make `memory_dir_for_cwd` fall back to the REAL
    /// `~/.agentmux/shared/providers/claude/`, writing test fixtures into
    /// the developer's actual home directory (caught live: a prior version
    /// of these tests left `-work-channel-a/` behind under the real home).
    fn build_channel_state(
        agent_id: &str,
        working_directory: &str,
        claude_config_dir: &std::path::Path,
        id_store: Arc<Store>,
    ) -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let mut def = agent_def(agent_id, working_directory);
        wstore.agent_def_insert(&mut def).unwrap();
        wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: agent_id.to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", claude_config_dir.display()),
                updated_at: 0,
            })
            .unwrap();

        let mut state = crate::server::tests::test_state();
        state.wstore = wstore.clone();
        state.id_store = id_store;

        let (engine, rx) = WshRpcEngine::new();
        register_native_memory_handlers(&engine, &state);
        (engine, rx)
    }

    async fn call_rpc<T: serde::de::DeserializeOwned>(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> T {
        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            data: Some(data),
            ..Default::default()
        };
        engine.handle_message(msg);
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id, "unexpected response id");
        assert!(resp.error.is_empty(), "handler returned error: {}", resp.error);
        serde_json::from_value(resp.data.unwrap_or(serde_json::Value::Null)).expect("response deserialize")
    }

    async fn call_rpc_expect_error(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> String {
        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            data: Some(data),
            ..Default::default()
        };
        engine.handle_message(msg);
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id);
        assert!(!resp.error.is_empty(), "expected error, got success: {:?}", resp.data);
        resp.error
    }

    #[tokio::test]
    async fn a_file_written_from_one_channel_is_visible_from_another() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config_a = tempfile::tempdir().unwrap();
        let config_b = tempfile::tempdir().unwrap();

        let (engine_a, mut rx_a) = build_channel_state("agent-shared-1", "/work/channel-a", config_a.path(), shared_id_store.clone());
        let (engine_b, mut rx_b) = build_channel_state("agent-shared-1", "/work/channel-b", config_b.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine_a,
            &mut rx_a,
            COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({
                "agent_id": "agent-shared-1",
                "filename": "MEMORY.md",
                "content": "written from channel A",
            }),
        )
        .await;

        // Channel B's live FS never had this file — list must still surface
        // it (via the shared mirror), and read_file must return its content.
        let listed: NativeMemoryListResult = call_rpc(
            &engine_b,
            &mut rx_b,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-shared-1" }),
        )
        .await;
        assert_eq!(listed.files.len(), 1, "channel B must see channel A's mirrored file");
        assert_eq!(listed.files[0].filename, "MEMORY.md");

        let read: NativeMemoryReadFileResult = call_rpc(
            &engine_b,
            &mut rx_b,
            COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-shared-1", "filename": "MEMORY.md" }),
        )
        .await;
        assert_eq!(read.content, "written from channel A");
    }

    #[tokio::test]
    async fn list_reports_the_files_real_mtime_for_a_mirror_only_entry() {
        // reagent P1 on PR #2459 (fifth pass): a mirror-only listing entry
        // (no live copy on this channel) must report the FILE's real
        // last-modified time (last_seen_mtime_ms), not the mirror row's own
        // sync timestamp (updated_at) — those are two different clocks that
        // just happen to be close together right after a write.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();

        // A deliberately old, obviously-not-"just synced" mtime — upsert's
        // own now_ms() for updated_at will always land far later than this.
        const REAL_FILE_MTIME_MS: i64 = 12_345;
        shared_id_store
            .agent_native_memory_upsert("agent-mtime", "MEMORY.md", "content", None, "/elsewhere", 7, REAL_FILE_MTIME_MS)
            .unwrap();

        let (engine, mut rx) = build_channel_state("agent-mtime", "/work/channel-a", config.path(), shared_id_store);

        let listed: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-mtime" }),
        )
        .await;
        assert_eq!(listed.files.len(), 1);
        assert_eq!(
            listed.files[0].modified_at, REAL_FILE_MTIME_MS,
            "mirror-only entries must report the file's real mtime, not the mirror's own sync timestamp"
        );
    }

    #[tokio::test]
    async fn live_fs_content_wins_over_a_stale_mirror_row() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config_a = tempfile::tempdir().unwrap();

        let (engine_a, mut rx_a) = build_channel_state("agent-shared-2", "/work/channel-a", config_a.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine_a,
            &mut rx_a,
            COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-shared-2", "filename": "MEMORY.md", "content": "v1" }),
        )
        .await;
        call_rpc::<Option<serde_json::Value>>(
            &engine_a,
            &mut rx_a,
            COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-shared-2", "filename": "MEMORY.md", "content": "v2 — freshest" }),
        )
        .await;

        // Directly stamp a stale mirror row behind the live FS's back — the
        // read path must still prefer the live file, not this stale row.
        shared_id_store
            .agent_native_memory_upsert("agent-shared-2", "MEMORY.md", "stale mirror content", None, "/nowhere", "stale mirror content".len() as i64, 0)
            .unwrap();

        let read: NativeMemoryReadFileResult = call_rpc(
            &engine_a,
            &mut rx_a,
            COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-shared-2", "filename": "MEMORY.md" }),
        )
        .await;
        assert_eq!(read.content, "v2 — freshest", "live FS must win over a stale mirror row");
    }

    #[tokio::test]
    async fn read_file_errors_when_absent_from_both_live_fs_and_mirror() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config_a = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-shared-3", "/work/channel-a", config_a.path(), shared_id_store);

        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-shared-3", "filename": "MEMORY.md" }),
        )
        .await;
        assert!(err.contains("not found"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn read_file_rejects_a_non_regular_file_instead_of_falling_back_to_the_mirror() {
        // reagent P1 on PR #2459 (second pass): a path that exists but isn't a
        // regular file (e.g. a directory landed at the expected filename) must
        // be rejected explicitly — collapsing it into "absent, fall back to
        // mirror" could serve stale mirrored content for a path that actually
        // exists but is the wrong type.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        // Pre-mirror some content, so a bug that treats this as "absent" would
        // wrongly succeed by serving it instead of erroring.
        shared_id_store
            .agent_native_memory_upsert("agent-wrongtype", "MEMORY.md", "mirrored content", None, "/elsewhere", "mirrored content".len() as i64, 0)
            .unwrap();
        let (engine, mut rx) = build_channel_state("agent-wrongtype", "/work/channel-a", config.path(), shared_id_store);

        let memory_dir = config.path().join("projects").join("-work-channel-a").join("memory");
        std::fs::create_dir_all(memory_dir.join("MEMORY.md")).unwrap(); // a directory, not a file

        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-wrongtype", "filename": "MEMORY.md" }),
        )
        .await;
        assert!(err.contains("not a regular file"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn list_propagates_a_real_read_dir_error_instead_of_treating_it_as_empty() {
        // reagent P2 on PR #2459: `std::fs::read_dir(&memory_dir).ok()` used to
        // swallow every error (permissions, I/O — not just the legitimate
        // "directory doesn't exist yet" case), silently reporting an empty
        // listing instead of surfacing a real access problem. Force a
        // non-NotFound read_dir error by making the "memory" path a regular
        // file instead of a directory.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();

        // memory_dir_for_cwd sanitizes "/work/channel-a" to "-work-channel-a".
        let projects_dir = config.path().join("projects").join("-work-channel-a");
        std::fs::create_dir_all(&projects_dir).unwrap();
        std::fs::write(projects_dir.join("memory"), b"not a directory").unwrap();

        let (engine, mut rx) = build_channel_state("agent-baddir", "/work/channel-a", config.path(), shared_id_store);

        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-baddir" }),
        )
        .await;
        assert!(err.contains("read_dir"), "expected a propagated read_dir error, got: {err}");
    }

    #[tokio::test]
    async fn list_does_not_re_upsert_an_unchanged_file_on_a_second_call() {
        // reagent P1 on PR #2459: list() must not do a full-content read +
        // SQLite write for every file on every call — only for a file whose
        // size differs from what's already mirrored. Two back-to-back list()
        // calls on an untouched file should produce exactly one mirror write.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-unchanged", "/work/channel-a", config.path(), shared_id_store.clone());

        let memory_dir = config.path().join("projects").join("-work-channel-a").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("MEMORY.md"), "stable content").unwrap();

        let _: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-unchanged" }),
        )
        .await;
        let first_updated_at = shared_id_store
            .agent_native_memory_list_meta("agent-unchanged")
            .unwrap()[0]
            .updated_at;

        let _: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-unchanged" }),
        )
        .await;
        let second_updated_at = shared_id_store
            .agent_native_memory_list_meta("agent-unchanged")
            .unwrap()[0]
            .updated_at;

        assert_eq!(
            first_updated_at, second_updated_at,
            "an unchanged file must not be re-upserted into the mirror on a second list() call"
        );
    }

    #[tokio::test]
    async fn list_detects_a_same_size_content_change_via_mtime() {
        // reagent P1 on PR #2459: comparing size alone would miss a same-byte-
        // length edit (e.g. correcting a typo), leaving the mirror serving
        // stale content forever to a channel that never had a live copy of
        // its own to self-correct with. Confirm the mtime check catches it.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-samesize", "/work/channel-a", config.path(), shared_id_store.clone());

        let memory_dir = config.path().join("projects").join("-work-channel-a").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        let file = memory_dir.join("MEMORY.md");
        std::fs::write(&file, "content A!").unwrap();

        let _: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-samesize" }),
        )
        .await;
        assert_eq!(
            shared_id_store.agent_native_memory_read("agent-samesize", "MEMORY.md").unwrap(),
            Some("content A!".to_string())
        );

        // Same byte length, different content, comfortably past mtime resolution.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        std::fs::write(&file, "content B!").unwrap();
        assert_eq!(file.metadata().unwrap().len(), "content A!".len() as u64);

        let _: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-samesize" }),
        )
        .await;
        assert_eq!(
            shared_id_store.agent_native_memory_read("agent-samesize", "MEMORY.md").unwrap(),
            Some("content B!".to_string()),
            "a same-size content change must still be picked up by list() via mtime"
        );
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
