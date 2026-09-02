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
    COMMAND_NATIVE_MEMORY_DIFF,
    COMMAND_NATIVE_MEMORY_HISTORY,
    COMMAND_NATIVE_MEMORY_LIST,
    COMMAND_NATIVE_MEMORY_READ_FILE,
    COMMAND_NATIVE_MEMORY_REVERT,
    COMMAND_NATIVE_MEMORY_WRITE_FILE,
    CommandNativeMemoryDiffData,
    CommandNativeMemoryHistoryData,
    CommandNativeMemoryListData,
    CommandNativeMemoryReadFileData,
    CommandNativeMemoryRevertData,
    CommandNativeMemoryWriteFileData,
    NativeMemoryDiffResult,
    NativeMemoryFileMeta,
    NativeMemoryHistoryResult,
    NativeMemoryListResult,
    NativeMemoryReadFileResult,
    NativeMemoryRevertResult,
    NativeMemoryVersionMeta,
};
use crate::backend::storage::NativeMemoryVersion;

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
/// its stored env blob to find `CLAUDE_CONFIG_DIR`. Returns an error only if
/// the agent cannot be resolved at all — an instance row with a blank
/// `working_directory` falls through to the registry rather than failing (see
/// the inline note below; that short-circuit was a real bug).
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
        // Only trust the instance row when it actually carries a working
        // directory. An empty one is NOT an error and must NOT short-circuit:
        // `agent.open` substitutes a default (`~/.agentmux/agents/<slug>`)
        // whenever `working_directory` is blank, so a blank row describes an
        // agent that is nonetheless running — and writing memories — in a
        // real directory. The registry fallback below is what knows that
        // real directory (`source_agents_base` + `working_dir`).
        //
        // Returning Err here instead made Armory → Memory → Personal and the
        // MemoryList MCP tool fail with "agent <x> has no working directory"
        // for every such agent, while its memory files sat on disk perfectly
        // intact — the common case, since `working_directory` is blank by
        // default. See SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md.
        if !instance.working_directory.is_empty() {
            let config_dir = wstore
                .agent_content_get(&instance.id, "env")
                .ok()
                .flatten()
                .map(|c| parse_claude_config_dir(&c.content))
                .unwrap_or_default();
            return Ok(memory_dir_for_cwd(&config_dir, &instance.working_directory));
        }
        // Resolve through the DEFINITION, not the instance row: `db_agents`
        // stores `instance_name` as its own column, distinct from the
        // definition's `name`, and it is empty for a definition-only row —
        // so reading the name off `instance` here silently yielded "" and
        // fell through to a not-found. `instance.id` IS the definition id in
        // this consolidated table (see the note above).
        if let Ok(Some(def)) = wstore.agent_def_get(&instance.id) {
            if let Some(dir) =
                memory_dir_for_blank_working_dir(wstore, &def.id, &def.name, &def.slug)
            {
                return Ok(dir);
            }
        }
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

/// Resolve `agent_id` — the agent SLUG, per the App-API convention (see
/// [`memory_dir_for_agent`]'s own doc comment) — to the same stable,
/// canonical identifier (`AgentDefinition.id` / `db_agents.id`) the
/// WebSocket RPC surface (`agent:memory:write_file` et al., which receives
/// this id directly from the caller and resolves it via `agent_def_get`)
/// already keys `db_agent_native_memory_versions` by.
///
/// reagent P1: without this, a version written through this App-API/MCP
/// surface (previously slug-keyed, verbatim) lived in a disjoint keyspace
/// from one written through the WS RPC surface for the exact same logical
/// agent whenever `slug != id` — a version written via the `MemoryWrite`
/// MCP tool was invisible to a WS-RPC-based `MemoryHistory`/`MemoryDiff`/
/// `MemoryRevert` call and vice versa, silently defeating the point of
/// having version history at all.
///
/// Mirrors `memory_dir_for_agent`'s own slug → instance → registry
/// resolution order, but returns the id instead of a filesystem path:
/// - Primary path (`instance_get_by_slug`): `db_agents` is the
///   consolidated definition+instance table (Phase 3a) — its own query
///   selects `id, id AS def_id`, i.e. the row's `id` already IS the
///   definition id in this model, not a separate instance-only identity.
/// - Registry fallback (a live agent not yet persisted to `db_agents`):
///   the registry record's `definition_id` field is the one that matches
///   `agent_def_get`'s namespace — its sibling `instance_id` is a
///   different, launch-scoped identity, not what `write_file` keys by.
pub(crate) fn resolve_agent_uuid(
    wstore: &crate::backend::storage::store::Store,
    agent_id: &str,
) -> Result<String, String> {
    if let Some(instance) = wstore
        .instance_get_by_slug(agent_id)
        .map_err(|e| format!("resolve_agent_uuid: store: {e}"))?
    {
        return Ok(instance.id);
    }
    find_active_registry_record_by_slug(agent_id)
        .map(|rec| rec.data.definition_id)
        .ok_or_else(|| format!("resolve_agent_uuid: agent {agent_id} not found"))
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
    memory_dir_for_registry_record(&rec)
}

/// Resolve the memory dir for an agent whose persisted `working_directory`
/// is blank — the DEFAULT state, so this is the common path, not an edge.
///
/// Two stages, in this order:
///
/// 1. **A registry record bound to this exact definition.** The registry
///    records the working dir an agent was really launched with, which beats
///    re-deriving it. `definition_id` is required to match: the plain
///    slug lookup keys off `derive_slug(instance_name)` alone, so two agents
///    whose display names slugify identically collide — and resolving the
///    WRONG agent's memory dir would let list/read/write operations touch
///    another agent's files (Codex P1, PR #2901).
/// 2. **The derived default**, `default_agent_working_dir(name)` — the same
///    path `agent.open` itself substitutes for a blank field. Needed because
///    `agent.open` does NOT create a registry record, so stage 1 misses
///    entirely for a freshly defined agent, which is exactly the case this
///    whole fix targets (Codex P1, PR #2901).
/// Takes the three identity fields explicitly rather than a struct: the two
/// callers hold different types (`AgentInstance` from `instance_get_by_slug`,
/// `AgentDefinition` from the by-id path) which don't share these field
/// names. `definition_id` is the id the registry's own `definition_id`
/// records — for `db_agents`, the consolidated definition+instance table,
/// the instance row's `id` already IS that id (see `memory_dir_for_agent`'s
/// own note).
fn memory_dir_for_blank_working_dir(
    wstore: &crate::backend::storage::store::Store,
    definition_id: &str,
    agent_name: &str,
    slug: &str,
) -> Option<std::path::PathBuf> {
    if !slug.is_empty() {
        if let Some(rec) = find_active_registry_record_by_slug(slug)
            .filter(|r| r.data.definition_id == definition_id)
        {
            if let Some(dir) = memory_dir_for_registry_record(&rec) {
                return Some(dir);
            }
        }
    }
    if agent_name.is_empty() {
        return None;
    }
    let config_dir = wstore
        .agent_content_get(definition_id, "env")
        .ok()
        .flatten()
        .map(|c| parse_claude_config_dir(&c.content))
        .unwrap_or_default();
    let work_dir = crate::backend::storage::agents::default_agent_working_dir(agent_name);
    Some(memory_dir_for_cwd(&config_dir, &work_dir))
}

/// Reconstruct one registry record's absolute memory dir directly (no slug
/// lookup) — shared by [`memory_dir_from_registry`] (single record, found by
/// slug) and [`list_all_memory_targets`] (every active record, for the
/// fs-watch drift detector's enumeration).
///
/// Reconstructs the absolute working directory: `source_agents_base` joined
/// with the relative `working_dir`. Legacy (v1/v2) records without a base
/// fall back to the current channel's agents dir (`AGENTMUX_AGENTS_DIR`),
/// matching the registry's own pre-P0.4 reconstruction rule.
fn memory_dir_for_registry_record(rec: &crate::registry::NamedAgentRecord) -> Option<std::path::PathBuf> {
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

/// Every agent with a resolvable memory directory right now — both agents
/// persisted to `db_agents` and live, registry-only agents that haven't
/// been (or never will be) written back there. Deduped by canonical agent
/// id; a `db_agents` row wins over its own registry record when both exist
/// (the row is the more authoritative, complete source, and matches what
/// `agent:memory:*` RPCs already key by).
///
/// reagent P1 on PR #2675: the fs-watch drift detector's enumeration
/// (`reconciliation_sweep_once`/`refresh_subscriptions` in
/// `native_memory_drift.rs`) originally used only `wstore.agent_def_list()`
/// — the same gap [`memory_dir_for_agent`] itself already had to work
/// around for the App-API surface (see its own doc comment / issue #1836):
/// a live agent spawned but not yet persisted to `db_agents` has no row
/// there at all, so the detector silently skipped it, contradicting the
/// spec's (§4.5) "every agent with an active session" contract.
pub(crate) fn list_all_memory_targets(
    wstore: &crate::backend::storage::store::Store,
) -> Vec<(String, std::path::PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    let mut targets = Vec::new();

    if let Ok(agents) = wstore.agent_def_list() {
        for agent in &agents {
            if let Some(dir) = memory_dir_for_agent_by_id(wstore, agent) {
                if seen.insert(agent.id.clone()) {
                    targets.push((agent.id.clone(), dir));
                }
            }
        }
    }

    if let Some(registry_dir) = crate::registry::resolve_shared_registry_dir() {
        if let Ok(registry) = crate::registry::Registry::open(registry_dir) {
            if let Ok(records) = registry.list_active() {
                for rec in &records {
                    let agent_id = rec.data.definition_id.clone();
                    if agent_id.is_empty() || !seen.insert(agent_id.clone()) {
                        continue;
                    }
                    if let Some(dir) = memory_dir_for_registry_record(rec) {
                        targets.push((agent_id, dir));
                    }
                }
            }
        }
    }

    targets
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
/// — the same read-then-upsert-if-changed logic `agent:memory:list`
/// performs inline (compare each live file's size+mtime against the
/// mirror, and upsert only when it's actually changed). This is a NEW,
/// standalone function with the same shape, not a shared implementation
/// `list`'s handler was refactored to call — reagent P2, PR #2527
/// (second round): `list`'s own inline copy (this file, `~agent:memory:
/// list`'s handler) still exists separately and has already diverged in
/// one detail (it hard-errors on a `file_type()` failure; this function
/// silently skips the entry instead). Deduplicating the two into one
/// shared implementation is a legitimate follow-up, not done here to
/// keep this PR's diff scoped to what ABF v0.2 §2.3 actually needs.
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

        // reagent P2, PR #2527 (second round): checked BEFORE the
        // unchanged-since-last-mirror short-circuit below — an oversized
        // file that hasn't changed since it was last mirrored still
        // exports/imports its truncated content on every subsequent
        // call, so the warning must fire every time too, not just on the
        // one call that actually re-reads and re-upserts it.
        if size_bytes > MAX_MEMORY_FILE_BYTES {
            truncated_files.push(name.clone());
        }

        let unchanged_since_last_mirror = mirrored_meta.get(&name) == Some(&(size_bytes as i64, modified_at));
        if unchanged_since_last_mirror {
            continue;
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
    // Same blank-working_directory fallthrough as `memory_dir_for_agent`
    // (see its own note) — a blank field is "this row can't answer", not
    // "there is no memory dir". `agent.open` substitutes a default whenever
    // it's blank, so such an agent still has memories on disk in a directory
    // only the registry knows.
    //
    // Short-circuiting to None here was worse than the sibling's early Err,
    // because both callers read None as a benign "no memory dir" rather than
    // a failure (ReAgent P1, PR #2901):
    //   - `bundle.rs`'s export_for_agent silently exports an EMPTY memory
    //     file list, losing the agent's memories from the bundle;
    //   - `bundle.rs`'s import_for_agent silently skips the live-fs mirror
    //     refresh that exists specifically to avoid overwriting unmirrored
    //     memory (reagent P0, PR #2527) — i.e. the blank-workdir case
    //     bypassed a data-loss guard.
    // Both for the common case, since `working_directory` is blank by default.
    if !agent.working_directory.is_empty() {
        let config_dir = wstore
            .agent_content_get(&agent.id, "env")
            .ok()
            .flatten()
            .map(|c| parse_claude_config_dir(&c.content))
            .unwrap_or_default();
        return Some(memory_dir_for_cwd(&config_dir, &agent.working_directory));
    }
    memory_dir_for_blank_working_dir(wstore, &agent.id, &agent.name, &agent.slug)
}

fn version_summary_to_meta(v: crate::backend::storage::NativeMemoryVersionSummary) -> NativeMemoryVersionMeta {
    NativeMemoryVersionMeta {
        id: v.id,
        content_hash: v.content_hash,
        parent_version_id: v.parent_version_id,
        source: v.source,
        source_detail: v.source_detail,
        session_id: v.session_id,
        created_at: v.created_at,
    }
}

fn version_to_meta(v: &NativeMemoryVersion) -> NativeMemoryVersionMeta {
    NativeMemoryVersionMeta {
        id: v.id.clone(),
        content_hash: v.content_hash.clone(),
        parent_version_id: v.parent_version_id.clone(),
        source: v.source.clone(),
        source_detail: v.source_detail.clone(),
        session_id: v.session_id.clone(),
        created_at: v.created_at,
    }
}

/// Cap on the LCS table's cell COUNT (from_lines.len() * to_lines.len()),
/// not on either side's line count independently — reagent P1: the
/// original per-side-only cap (20,000 lines each) bounded neither
/// dimension against the other, so two files each under that cap could
/// still produce a ~3.2GB `usize` table (20_000 * 20_000 * 8 bytes) in the
/// shared agentmux-srv process. 4,000,000 cells keeps the table under
/// ~32MB (`* size_of::<usize>()`) regardless of how the two side lengths
/// are distributed — generous for memory files (markdown notes, not logs)
/// while still bounded for any combination of sizes.
const MAX_DIFF_CELLS: usize = 4_000_000;

/// A minimal unified-diff-style line comparison: longest-common-subsequence
/// based, output lines prefixed `"  "` (context), `"- "` (removed, `from`
/// only), or `"+ "` (added, `to` only). No `@@` hunk headers or context
/// trimming in v1 — every line is included, which is fine for memory files.
pub(crate) fn line_diff(from: &str, to: &str) -> String {
    let from_lines: Vec<&str> = from.lines().collect();
    let to_lines: Vec<&str> = to.lines().collect();

    if from_lines.len().saturating_mul(to_lines.len()) > MAX_DIFF_CELLS {
        return format!(
            "(diff omitted: {} x {} lines exceeds the {MAX_DIFF_CELLS}-cell comparison cap)",
            from_lines.len(),
            to_lines.len(),
        );
    }

    let n = from_lines.len();
    let m = to_lines.len();
    let cols = m + 1;
    // A single flat allocation, not `n + 1` separate `Vec<usize>` rows —
    // reagent P2: MAX_DIFF_CELLS bounds the cell COUNT (n * m), but a
    // maximally lopsided diff (e.g. ~4,000,000 short lines vs. 1 line)
    // stays under that cap while `vec![vec![...]; n + 1]` would still
    // perform ~4,000,001 individual heap allocations — one per row — whose
    // allocator overhead and allocation-count latency dwarf the actual
    // cell-data cost the cap was meant to bound. lcs[i][j] (length of the
    // longest common subsequence of from_lines[i..] and to_lines[j..])
    // lives at flat index i * cols + j.
    let mut lcs = vec![0usize; (n + 1) * cols];
    let idx = |i: usize, j: usize| i * cols + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[idx(i, j)] = if from_lines[i] == to_lines[j] {
                lcs[idx(i + 1, j + 1)] + 1
            } else {
                lcs[idx(i + 1, j)].max(lcs[idx(i, j + 1)])
            };
        }
    }

    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if from_lines[i] == to_lines[j] {
            out.push_str("  ");
            out.push_str(from_lines[i]);
            out.push('\n');
            i += 1;
            j += 1;
        } else if lcs[idx(i + 1, j)] >= lcs[idx(i, j + 1)] {
            out.push_str("- ");
            out.push_str(from_lines[i]);
            out.push('\n');
            i += 1;
        } else {
            out.push_str("+ ");
            out.push_str(to_lines[j]);
            out.push('\n');
            j += 1;
        }
    }
    while i < n {
        out.push_str("- ");
        out.push_str(from_lines[i]);
        out.push('\n');
        i += 1;
    }
    while j < m {
        out.push_str("+ ");
        out.push_str(to_lines[j]);
        out.push('\n');
        j += 1;
    }
    out
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

                // A blank working_directory is the DEFAULT state, not a
                // reason to short-circuit: `agent.open` substitutes a real
                // directory whenever this field is blank, so a blank row
                // still describes an agent with real memories on disk. The
                // old inline "blank → Ok(files: [])" here was a duplicate,
                // un-synced copy of the exact blank-workdir bug
                // SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md
                // (#2901) already fixed once in `memory_dir_for_agent_by_id`
                // itself — that fix never propagated to this handler (or to
                // read_file/write_file/revert below), so it silently kept
                // reporting "no memories" for every agent #2901 was
                // supposed to have fixed. See
                // SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02.md.
                let memory_dir = memory_dir_for_agent_by_id(&wstore, &agent).ok_or_else(|| {
                    format!("agent:memory:list: agent {} has no resolvable memory directory", cmd.agent_id)
                })?;

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

                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let path = memory_dir_for_agent_by_id(&wstore, &agent)
                    .ok_or_else(|| {
                        format!("agent:memory:read_file: agent {} has no resolvable memory directory", cmd.agent_id)
                    })?
                    .join(&cmd.filename);

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

                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let dir = memory_dir_for_agent_by_id(&wstore, &agent).ok_or_else(|| {
                    format!("agent:memory:write_file: agent {} has no resolvable memory directory", cmd.agent_id)
                })?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:write_file: mkdir: {e}"))?;

                // Version history — recorded BEFORE the live-file write below,
                // not after. reagent P1: the drift detector's fast path
                // (native_memory_drift.rs) subscribes to fs-watch events on
                // this same directory; if the version row were inserted AFTER
                // the write (as an earlier revision of this handler did), a
                // fs-watch event for the write below can be processed before
                // this version exists, see a hash that doesn't match anything
                // recorded yet, and log this legitimate RPC write as a
                // spurious "external_fs_write". Recording the version first
                // establishes a real happens-before: the file-modify event
                // that write can possibly generate cannot fire until the
                // write below actually executes, by which point this version
                // already exists to match against. Non-fatal on failure — a
                // durability/review layer on top of the write, not the write
                // itself (mirrors the mirror-upsert failure handling below).
                let (version_source, version_detail) = match &cmd.provenance {
                    Some(p) => (p.source.as_str(), p.detail.to_string()),
                    None => ("agent_inferred", "{}".to_string()),
                };
                if let Err(e) = id_store.agent_native_memory_version_insert(
                    &agent.id,
                    &cmd.filename,
                    &cmd.content,
                    version_source,
                    &version_detail,
                    "",
                ) {
                    tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:write_file: version insert failed (non-fatal)");
                }

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

    let wstore_history = state.wstore.clone();
    let id_store_history = state.id_store.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_history.clone();
            let id_store = id_store_history.clone();
            Box::pin(async move {
                let cmd: CommandNativeMemoryHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:memory:history: {e}"))?;

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:history: {e}"))?;

                // Resolve to the same canonical agent.id write_file keys
                // by, for the same reason app_api::memory_history_impl
                // does (reagent P1) — cmd.agent_id is expected to already
                // be that id for this surface (the frontend passes
                // AgentDefinition.id), but resolving explicitly here
                // rather than trusting it verbatim matches write_file's
                // own validation and closes the gap if that assumption
                // ever stops holding for some caller.
                let agent = wstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:history: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:history: agent {} not found", cmd.agent_id))?;

                let versions = id_store
                    .agent_native_memory_version_list(&agent.id, &cmd.filename)
                    .map_err(|e| format!("agent:memory:history: store: {e}"))?
                    .into_iter()
                    .map(version_summary_to_meta)
                    .collect();

                Ok(Some(serde_json::to_value(NativeMemoryHistoryResult { versions }).map_err(|e| e.to_string())?))
            })
        }),
    );

    let wstore_diff = state.wstore.clone();
    let id_store_diff = state.id_store.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_DIFF,
        Box::new(move |data, _ctx| {
            let wstore = wstore_diff.clone();
            let id_store = id_store_diff.clone();
            Box::pin(async move {
                let cmd: CommandNativeMemoryDiffData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:memory:diff: {e}"))?;

                // Resolve to the same canonical agent.id write_file keys by
                // — see the identical comment on the history handler above.
                let agent = wstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:diff: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:diff: agent {} not found", cmd.agent_id))?;

                let from = id_store
                    .agent_native_memory_version_get(&cmd.from_version_id)
                    .map_err(|e| format!("agent:memory:diff: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:diff: version {} not found", cmd.from_version_id))?;
                let to = id_store
                    .agent_native_memory_version_get(&cmd.to_version_id)
                    .map_err(|e| format!("agent:memory:diff: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:diff: version {} not found", cmd.to_version_id))?;
                // reagent P1: unlike list/read/write/history/revert, this
                // handler has no other agent-scoping — every caller shares
                // one instance-wide X-AuthKey, so without this check any
                // caller could read any other agent's memory content by
                // version id.
                if from.agent_id != agent.id || to.agent_id != agent.id {
                    return Err(format!(
                        "agent:memory:diff: one or both versions do not belong to {}",
                        cmd.agent_id
                    ));
                }
                // reagent P2: ownership alone isn't enough — from/to must
                // also be versions of the SAME file, or the "diff" is a
                // meaningless line-by-line comparison of two unrelated
                // files with no error to signal that.
                if from.filename != to.filename {
                    return Err(format!(
                        "agent:memory:diff: from_version_id and to_version_id are versions of different files ({} vs {})",
                        from.filename, to.filename
                    ));
                }

                let diff = line_diff(&from.content, &to.content);
                Ok(Some(serde_json::to_value(NativeMemoryDiffResult { diff }).map_err(|e| e.to_string())?))
            })
        }),
    );

    let wstore_revert = state.wstore.clone();
    let id_store_revert = state.id_store.clone();
    engine.register_handler(
        COMMAND_NATIVE_MEMORY_REVERT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_revert.clone();
            let id_store = id_store_revert.clone();
            Box::pin(async move {
                let cmd: CommandNativeMemoryRevertData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:memory:revert: {e}"))?;

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:revert: {e}"))?;

                // Resolve to the same canonical agent.id write_file keys by
                // — see the identical comment on the history handler above.
                // Resolved BEFORE the ownership check below (not after, as
                // an earlier revision of this handler did) so that check
                // compares against the same id the version was actually
                // stored under, not the raw, possibly-different cmd.agent_id.
                let agent = wstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:revert: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:revert: agent {} not found", cmd.agent_id))?;

                let target = id_store
                    .agent_native_memory_version_get(&cmd.target_version_id)
                    .map_err(|e| format!("agent:memory:revert: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:revert: version {} not found", cmd.target_version_id))?;
                if target.agent_id != agent.id || target.filename != cmd.filename {
                    return Err(format!(
                        "agent:memory:revert: version {} does not belong to {}/{}",
                        cmd.target_version_id, cmd.agent_id, cmd.filename
                    ));
                }

                // Revert is implemented as a NEW write through the same
                // path as agent:memory:write_file (live file + mirror +
                // version), not a rewrite of history — this is the
                // git-revert-not-git-reset guarantee from the spec's §4.3.
                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let dir = memory_dir_for_agent_by_id(&wstore, &agent).ok_or_else(|| {
                    format!("agent:memory:revert: agent {} has no resolvable memory directory", cmd.agent_id)
                })?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:revert: mkdir: {e}"))?;

                // Version recorded BEFORE the live-file write below — same
                // fs-watch-race rationale as agent:memory:write_file's own
                // handler above (reagent P1). Unlike that handler, a failure
                // here IS fatal: the RPC's whole contract is "return the new
                // version," so silently reverting the file but not returning
                // a version would leave the caller with no way to know what
                // they just reverted to.
                let detail = serde_json::json!({ "reverted_to": cmd.target_version_id }).to_string();
                let new_version = id_store
                    .agent_native_memory_version_insert(&agent.id, &cmd.filename, &target.content, "revert", &detail, "")
                    .map_err(|e| format!("agent:memory:revert: version insert: {e}"))?;

                let dest = dir.join(&cmd.filename);
                let tmp = dir.join(format!(".{}.{}.tmp", cmd.filename, uuid::Uuid::new_v4()));
                if let Err(e) = std::fs::write(&tmp, &target.content) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:revert: write tmp: {e}"));
                }
                if let Err(e) = std::fs::rename(&tmp, &dest) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("agent:memory:revert: rename: {e}"));
                }

                let metadata_type = parse_frontmatter_type(&target.content);
                let dest_meta = std::fs::metadata(&dest).ok();
                let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(target.content.len() as i64);
                let mtime_ms = dest_meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Err(e) = id_store.agent_native_memory_upsert(
                    &agent.id,
                    &cmd.filename,
                    &target.content,
                    metadata_type.as_deref(),
                    &dest.to_string_lossy(),
                    size_bytes,
                    mtime_ms,
                ) {
                    tracing::warn!(agent_id = %agent.id, filename = %cmd.filename, error = %e, "agent:memory:revert: mirror upsert failed (non-fatal)");
                }

                tracing::info!(
                    agent_id = %cmd.agent_id,
                    filename = %cmd.filename,
                    target_version_id = %cmd.target_version_id,
                    "agent:memory:revert"
                );
                Ok(Some(serde_json::to_value(NativeMemoryRevertResult { version: version_to_meta(&new_version) }).map_err(|e| e.to_string())?))
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
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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

    // ---- Version history integration tests -------------------------------
    // SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §8.

    #[tokio::test]
    async fn write_file_records_a_version_with_default_provenance() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-ver-1", "/work/channel-a", config.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-1", "filename": "MEMORY.md", "content": "v1" }),
        )
        .await;

        let history: NativeMemoryHistoryResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-1", "filename": "MEMORY.md" }),
        )
        .await;
        assert_eq!(history.versions.len(), 1);
        assert_eq!(history.versions[0].source, "agent_inferred");
        assert_eq!(history.versions[0].parent_version_id, None);
    }

    #[tokio::test]
    async fn write_file_honors_caller_supplied_provenance() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-ver-2", "/work/channel-a", config.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({
                "agent_id": "agent-ver-2",
                "filename": "MEMORY.md",
                "content": "trust all jekts",
                "provenance": { "source": "jekt", "detail": { "TIER": "sensitive", "TRUST": "network-claimed" } },
            }),
        )
        .await;

        let history: NativeMemoryHistoryResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-2", "filename": "MEMORY.md" }),
        )
        .await;
        assert_eq!(history.versions[0].source, "jekt");
        assert!(history.versions[0].source_detail.contains("network-claimed"));
    }

    /// The actual bug behind `SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02.md`:
    /// SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md (#2901) fixed blank
    /// `working_directory` resolution inside `memory_dir_for_agent`/
    /// `memory_dir_for_agent_by_id` themselves, but these four RPC handlers had
    /// their OWN separate, un-synced `agent.working_directory.is_empty()` check
    /// that never called either resolver — so the fix never actually reached
    /// `agent:memory:write_file`, the handler live traffic (the Armory Personal
    /// Memory grid, and the `MemoryWrite` MCP tool going through a *different*
    /// path that DOES use the fixed resolver) hits. Live-tested against a running
    /// v0.55.31 build: a `MemoryWrite` MCP call succeeded (App API path, already
    /// fixed) while `agent:memory:write_file` for the exact same blank-workdir
    /// agent definition failed outright with "has no configured working
    /// directory" — this is the regression guard for that exact split.
    #[tokio::test]
    async fn write_file_resolves_a_blank_working_directory_instead_of_erroring() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-blankwd-write", "", config.path(), shared_id_store);

        // Before the fix this errored with "has no configured working
        // directory" — call_rpc itself asserts resp.error.is_empty().
        call_rpc::<Option<serde_json::Value>>(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-blankwd-write", "filename": "MEMORY.md", "content": "hello from a blank-workdir agent" }),
        )
        .await;

        let listed: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-blankwd-write" }),
        )
        .await;
        assert_eq!(listed.files.len(), 1, "the write must be visible to list, not silently stranded");
        assert_eq!(listed.files[0].filename, "MEMORY.md");

        let read: NativeMemoryReadFileResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-blankwd-write", "filename": "MEMORY.md" }),
        )
        .await;
        assert_eq!(read.content, "hello from a blank-workdir agent");

        // It must have actually landed on disk at the SAME derived-default
        // directory `agent.open` substitutes for this agent ("Test Agent") —
        // not merely round-tripped through some other consistent-with-itself
        // location. The sibling test below plants a file directly at this
        // path and proves `list` finds it independently of this write path.
        let default_dir = crate::backend::storage::agents::default_agent_working_dir("Test Agent");
        let expected_path = memory_dir_for_cwd(&config.path().display().to_string(), &default_dir).join("MEMORY.md");
        assert!(expected_path.is_file(), "expected the write to land at {expected_path:?}");
    }

    /// list's OLD behavior for a blank working_directory was to silently
    /// return an empty file list rather than error — indistinguishable in
    /// the Armory grid from "this agent genuinely has no memories" (the
    /// exact trap SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md's
    /// four-state card design exists to avoid). This proves list now finds
    /// files that were written directly to the derived-default directory —
    /// i.e. files that existed on disk all along, that list was previously
    /// silently failing to find, not files this test manufactures via
    /// write_file (which now shares the same fixed resolver and would trivially
    /// "work" even if list's OWN resolution were still broken).
    #[tokio::test]
    async fn list_finds_files_already_on_disk_at_the_derived_default_dir_for_a_blank_workdir_agent() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();

        let default_dir = crate::backend::storage::agents::default_agent_working_dir("Test Agent");
        let memory_dir = memory_dir_for_cwd(&config.path().display().to_string(), &default_dir);
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("PRE_EXISTING.md"), "written directly to disk, not via this RPC").unwrap();

        let (engine, mut rx) = build_channel_state("agent-blankwd-preexisting", "", config.path(), shared_id_store);
        let listed: NativeMemoryListResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_NATIVE_MEMORY_LIST,
            serde_json::json!({ "agent_id": "agent-blankwd-preexisting" }),
        )
        .await;
        assert_eq!(
            listed.files.len(),
            1,
            "list must find a file that genuinely exists on disk at the derived-default dir, not report empty"
        );
        assert_eq!(listed.files[0].filename, "PRE_EXISTING.md");
    }

    #[tokio::test]
    async fn revert_resolves_a_blank_working_directory_instead_of_erroring() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-blankwd-revert", "", config.path(), shared_id_store);

        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-blankwd-revert", "filename": "MEMORY.md", "content": "v1" }),
        ).await;
        let history: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-blankwd-revert", "filename": "MEMORY.md" }),
        ).await;
        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-blankwd-revert", "filename": "MEMORY.md", "content": "v2" }),
        ).await;

        // Before the fix this errored with "has no configured working
        // directory" — call_rpc itself asserts resp.error.is_empty().
        call_rpc::<serde_json::Value>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_REVERT,
            serde_json::json!({
                "agent_id": "agent-blankwd-revert",
                "filename": "MEMORY.md",
                "target_version_id": history.versions[0].id,
            }),
        ).await;

        let read: NativeMemoryReadFileResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-blankwd-revert", "filename": "MEMORY.md" }),
        ).await;
        assert_eq!(read.content, "v1", "revert must have restored v1's content on disk");
    }

    #[tokio::test]
    async fn history_lists_newest_first_with_parent_chain() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-ver-3", "/work/channel-a", config.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-3", "filename": "MEMORY.md", "content": "v1" }),
        ).await;
        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-3", "filename": "MEMORY.md", "content": "v2" }),
        ).await;

        let history: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-3", "filename": "MEMORY.md" }),
        ).await;
        assert_eq!(history.versions.len(), 2);
        assert_eq!(history.versions[0].parent_version_id, Some(history.versions[1].id.clone()));
    }

    #[tokio::test]
    async fn diff_shows_added_and_removed_lines() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-ver-4", "/work/channel-a", config.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-4", "filename": "MEMORY.md", "content": "line a\nline b" }),
        ).await;
        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-4", "filename": "MEMORY.md", "content": "line a\nline c" }),
        ).await;

        let history: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-4", "filename": "MEMORY.md" }),
        ).await;
        let (newest, oldest) = (&history.versions[0], &history.versions[1]);

        let diff: NativeMemoryDiffResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_DIFF,
            serde_json::json!({ "agent_id": "agent-ver-4", "from_version_id": oldest.id, "to_version_id": newest.id }),
        ).await;
        assert!(diff.diff.contains("  line a"), "unexpected diff: {}", diff.diff);
        assert!(diff.diff.contains("- line b"), "unexpected diff: {}", diff.diff);
        assert!(diff.diff.contains("+ line c"), "unexpected diff: {}", diff.diff);
    }

    #[tokio::test]
    async fn diff_errors_for_an_unknown_version_id() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-ver-5", "/work/channel-a", config.path(), shared_id_store.clone());

        let err = call_rpc_expect_error(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_DIFF,
            serde_json::json!({ "agent_id": "agent-ver-5", "from_version_id": "nope", "to_version_id": "also-nope" }),
        ).await;
        assert!(err.contains("not found"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn diff_rejects_a_version_from_a_different_agent() {
        // reagent P1: from/to must both belong to the calling agent_id —
        // this is the regression test for that fix.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config_a = tempfile::tempdir().unwrap();
        let config_b = tempfile::tempdir().unwrap();
        let (engine_a, mut rx_a) = build_channel_state("agent-diff-a", "/work/channel-a", config_a.path(), shared_id_store.clone());
        let (engine_b, mut rx_b) = build_channel_state("agent-diff-b", "/work/channel-b", config_b.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-diff-a", "filename": "MEMORY.md", "content": "v1" }),
        ).await;
        call_rpc::<Option<serde_json::Value>>(
            &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-diff-a", "filename": "MEMORY.md", "content": "v2" }),
        ).await;
        let history_a: NativeMemoryHistoryResult = call_rpc(
            &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-diff-a", "filename": "MEMORY.md" }),
        ).await;

        let err = call_rpc_expect_error(
            &engine_b, &mut rx_b, COMMAND_NATIVE_MEMORY_DIFF,
            serde_json::json!({
                "agent_id": "agent-diff-b",
                "from_version_id": history_a.versions[1].id,
                "to_version_id": history_a.versions[0].id,
            }),
        ).await;
        assert!(err.contains("do not belong to"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn diff_rejects_two_versions_of_different_files() {
        // reagent P2: ownership alone isn't enough — from/to must also be
        // versions of the SAME file, or the "diff" is a meaningless
        // line-by-line comparison of two unrelated files.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-diff-files", "/work/channel-a", config.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-diff-files", "filename": "a.md", "content": "content a" }),
        ).await;
        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-diff-files", "filename": "b.md", "content": "content b" }),
        ).await;
        let history_a: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-diff-files", "filename": "a.md" }),
        ).await;
        let history_b: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-diff-files", "filename": "b.md" }),
        ).await;

        let err = call_rpc_expect_error(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_DIFF,
            serde_json::json!({
                "agent_id": "agent-diff-files",
                "from_version_id": history_a.versions[0].id,
                "to_version_id": history_b.versions[0].id,
            }),
        ).await;
        assert!(err.contains("different files"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn revert_writes_a_new_version_and_restores_live_content_without_deleting_history() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config = tempfile::tempdir().unwrap();
        let (engine, mut rx) = build_channel_state("agent-ver-6", "/work/channel-a", config.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md", "content": "good content" }),
        ).await;
        call_rpc::<Option<serde_json::Value>>(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md", "content": "fabricated content" }),
        ).await;

        let history_before: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md" }),
        ).await;
        assert_eq!(history_before.versions.len(), 2);
        let good_version_id = history_before.versions[1].id.clone(); // oldest = "good content"

        let revert: NativeMemoryRevertResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_REVERT,
            serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md", "target_version_id": good_version_id }),
        ).await;
        assert_eq!(revert.version.source, "revert");

        // Live content (and mirror) must now read "good content" again.
        let read: NativeMemoryReadFileResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_READ_FILE,
            serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md" }),
        ).await;
        assert_eq!(read.content, "good content");

        // History must now have 3 rows (append-only — the fabricated
        // version is still there, just no longer latest), not 2.
        let history_after: NativeMemoryHistoryResult = call_rpc(
            &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md" }),
        ).await;
        assert_eq!(history_after.versions.len(), 3, "revert must never delete or rewrite prior versions");
    }

    #[tokio::test]
    async fn revert_rejects_a_version_belonging_to_a_different_agent() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
        let config_a = tempfile::tempdir().unwrap();
        let config_b = tempfile::tempdir().unwrap();
        let (engine_a, mut rx_a) = build_channel_state("agent-ver-7a", "/work/channel-a", config_a.path(), shared_id_store.clone());
        let (engine_b, mut rx_b) = build_channel_state("agent-ver-7b", "/work/channel-b", config_b.path(), shared_id_store.clone());

        call_rpc::<Option<serde_json::Value>>(
            &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_WRITE_FILE,
            serde_json::json!({ "agent_id": "agent-ver-7a", "filename": "MEMORY.md", "content": "agent a's content" }),
        ).await;
        let history_a: NativeMemoryHistoryResult = call_rpc(
            &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_HISTORY,
            serde_json::json!({ "agent_id": "agent-ver-7a", "filename": "MEMORY.md" }),
        ).await;

        let err = call_rpc_expect_error(
            &engine_b, &mut rx_b, COMMAND_NATIVE_MEMORY_REVERT,
            serde_json::json!({ "agent_id": "agent-ver-7b", "filename": "MEMORY.md", "target_version_id": history_a.versions[0].id }),
        ).await;
        assert!(err.contains("does not belong to"), "unexpected error: {err}");
    }

    #[test]
    fn line_diff_marks_context_removed_and_added_lines() {
        let diff = line_diff("a\nb\nc", "a\nx\nc");
        assert_eq!(diff, "  a\n- b\n+ x\n  c\n");
    }

    #[test]
    fn line_diff_handles_identical_content() {
        assert_eq!(line_diff("same", "same"), "  same\n");
    }

    // reagent P2 on PR #2674 (re-review): line_diff was rewritten from
    // vec![vec![0usize; m + 1]; n + 1] (n+1 separate heap allocations) to a
    // single flat Vec indexed manually via `i * cols + j` — a lopsided
    // shape (one side much longer than the other) exercises exactly the
    // index arithmetic that refactor could get wrong, so cover it with an
    // asymmetric case in both directions rather than only the roughly
    // square cases above.
    #[test]
    fn line_diff_handles_a_lopsided_shape_in_both_directions() {
        assert_eq!(line_diff("a\nb\nc\nd\ne", "c"), "- a\n- b\n  c\n- d\n- e\n");
        assert_eq!(line_diff("c", "a\nb\nc\nd\ne"), "+ a\n+ b\n  c\n+ d\n+ e\n");
    }

    // reagent P1 on PR #2675: the fs-watch drift detector's enumeration
    // originally used only `wstore.agent_def_list()`, which misses a live
    // agent spawned but never (or not yet) persisted to `db_agents` —
    // contradicting spec §4.5's "every agent with an active session".
    #[tokio::test]
    async fn list_all_memory_targets_includes_registry_only_live_agents() {
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
                    instance_id: "inst-live-only".to_string(),
                    instance_name: "LiveOnly".to_string(),
                    definition_id: "def-live-only".to_string(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    working_dir: "live-only-proj".to_string(),
                    source_agents_base: Some(tmp.path().join("agents").to_string_lossy().to_string()),
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let state = crate::server::tests::test_state();
        let targets = list_all_memory_targets(&state.wstore);
        assert!(
            targets.iter().any(|(id, _)| id == "def-live-only"),
            "a registry-only agent with no db_agents row must still be enumerated: {targets:?}"
        );

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
    }

    // A db_agents row is the more authoritative, complete source for an
    // agent that has one — its own memory dir resolution must win over a
    // registry-reconstructed guess for the same logical agent, and the
    // agent must be enumerated exactly once, not twice.
    #[tokio::test]
    async fn list_all_memory_targets_dedupes_an_agent_present_in_both_db_agents_and_the_registry() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

        let state = crate::server::tests::test_state();
        let config_dir = tempfile::tempdir().unwrap();
        let mut def = crate::backend::storage::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: "dup-agent".to_string(),
            slug: "dup-agent".to_string(),
            name: "Test".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: "/work/dup-agent".to_string(),
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        state.wstore.agent_def_insert(&mut def).unwrap();
        state
            .wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: "dup-agent".to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", config_dir.path().display()),
                updated_at: 0,
            })
            .unwrap();
        let expected_dir = memory_dir_for_agent_by_id(&state.wstore, &def).unwrap();

        let registry_dir = tmp.path().join("agents").join("registry");
        let registry = crate::registry::Registry::open(registry_dir).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 1,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-dup".to_string(),
                    instance_name: "dup-agent".to_string(),
                    definition_id: "dup-agent".to_string(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    // Deliberately a different working dir than db_agents'
                    // own row — proves the db_agents-derived entry wins
                    // rather than being silently overwritten.
                    working_dir: "different-registry-guess".to_string(),
                    source_agents_base: Some(tmp.path().join("agents").to_string_lossy().to_string()),
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let targets = list_all_memory_targets(&state.wstore);
        let matches: Vec<_> = targets.iter().filter(|(id, _)| id == "dup-agent").collect();
        assert_eq!(matches.len(), 1, "must not enumerate the same agent twice: {targets:?}");
        assert_eq!(matches[0].1, expected_dir, "the db_agents-derived dir must win over the registry's");

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


    /// The non-blank path is unaffected: a real working_directory still
    /// resolves straight from the instance row, without consulting the
    /// registry at all.
    #[test]
    fn non_blank_working_directory_still_resolves_from_the_instance_row() {
        let wstore = Store::open_in_memory().unwrap();
        let mut def = agent_def("realwd-agent", "/work/proj");
        wstore.agent_def_insert(&mut def).unwrap();

        let dir = memory_dir_for_agent(&wstore, "realwd-agent").unwrap();
        let comps: Vec<String> = dir
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let want_tail = ["projects", "-work-proj", "memory"];
        let tail = &comps[comps.len() - want_tail.len()..];
        assert_eq!(tail, want_tail, "unexpected memory dir tail: {comps:?}");
    }

    /// A blank `working_directory` — the DEFAULT for a newly defined agent —
    /// must resolve to the same path `agent.open` itself substitutes, not
    /// fail. Before the fix `memory_dir_for_agent` returned
    /// "agent <x> has no working directory" and Armory → Memory → Personal
    /// (and the MemoryList MCP tool) were broken for the common case, while
    /// the agent's memory files sat on disk intact. Reproduced live:
    ///   memory/list failed: HTTP 500 — "memory: agent manoz has no working directory"
    ///
    /// With no registry record present, stage 2 (the derived default) is what
    /// must answer — the case Codex P1 flagged, since `agent.open` does not
    /// create a registry record.
    #[test]
    fn blank_working_directory_resolves_to_the_same_default_agent_open_substitutes() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let wstore = Store::open_in_memory().unwrap();
        let mut def = agent_def("blankwd-agent", "");
        def.name = "Blank WD Agent".to_string();
        wstore.agent_def_insert(&mut def).unwrap();

        let dir = memory_dir_for_agent(&wstore, "blankwd-agent")
            .expect("a blank working_directory must resolve, not error");

        // default_agent_working_dir("Blank WD Agent") -> ~/.agentmux/agents/blank-wd-agent,
        // which memory_dir_for_cwd sanitizes to "---agentmux-agents-blank-wd-agent"
        // (the leading `~`, `/` and `.` each become their own dash).
        let comps: Vec<String> = dir
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let want_tail = ["projects", "---agentmux-agents-blank-wd-agent", "memory"];
        let tail = &comps[comps.len() - want_tail.len()..];
        assert_eq!(tail, want_tail, "unexpected memory dir tail: {comps:?}");
    }

    /// The shared helper must stay byte-identical to `agent.open`'s own
    /// substitution — they are the same path by contract, and drift between
    /// them is precisely what broke Personal Memory.
    #[test]
    fn default_agent_working_dir_matches_agent_opens_inline_derivation() {
        for name in ["Manoz", "Blank WD Agent", "Wei_Zhang-2", "Zurich Nome"] {
            let inline: String = name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                .collect();
            assert_eq!(
                crate::backend::storage::agents::default_agent_working_dir(name),
                format!("~/.agentmux/agents/{inline}"),
                "drifted from agent_open's derivation for {name:?}",
            );
        }
    }

    /// Codex P1: the registry stage must be bound to the agent's own
    /// definition. `find_active_registry_record_by_slug` matches on
    /// `derive_slug(instance_name)` alone, so two agents whose display names
    /// slugify the same collide — resolving the WRONG agent's memory dir
    /// would let list/read/write touch another agent's files. A record whose
    /// `definition_id` doesn't match must be ignored, falling through to the
    /// derived default instead.
    #[test]
    fn a_registry_record_for_a_different_definition_is_never_used() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

        let registry =
            crate::registry::Registry::open(tmp.path().join("agents").join("registry")).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 1,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-other".to_string(),
                    // Slugifies to the same slug as our agent below...
                    instance_name: "Collide Agent".to_string(),
                    // ...but belongs to a DIFFERENT definition.
                    definition_id: "def-somebody-else".to_string(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    working_dir: "somebody-elses-dir".to_string(),
                    source_agents_base: Some(tmp.path().to_string_lossy().to_string()),
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let wstore = Store::open_in_memory().unwrap();
        let mut def = agent_def("collide-agent", "");
        def.name = "Collide Agent".to_string();
        wstore.agent_def_insert(&mut def).unwrap();

        let dir = memory_dir_for_agent(&wstore, "collide-agent").unwrap();
        let as_str = dir.to_string_lossy().to_string();
        assert!(
            !as_str.contains("somebody-elses-dir"),
            "must not resolve another definition's memory dir: {as_str}",
        );
        assert!(
            as_str.contains("collide-agent"),
            "expected the derived default for this agent: {as_str}",
        );

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
    }
}
