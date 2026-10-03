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
    COMMAND_NATIVE_MEMORY_ADOPTION_LIST,
    COMMAND_NATIVE_MEMORY_CLAIMS,
    COMMAND_NATIVE_MEMORY_DIFF,
    COMMAND_NATIVE_MEMORY_HISTORY,
    COMMAND_NATIVE_MEMORY_LIST,
    COMMAND_NATIVE_MEMORY_READ_FILE,
    COMMAND_NATIVE_MEMORY_REVERT,
    COMMAND_NATIVE_MEMORY_WRITE_FILE,
    CommandNativeMemoryAdoptionListData,
    CommandNativeMemoryClaimsData,
    CommandNativeMemoryDiffData,
    CommandNativeMemoryHistoryData,
    CommandNativeMemoryListData,
    CommandNativeMemoryReadFileData,
    CommandNativeMemoryRevertData,
    CommandNativeMemoryWriteFileData,
    NativeMemoryAdoptionListResult,
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
/// `app_api/agent_open.rs` sets at agent spawn time. We never write to the global
/// `~/.claude/projects/` because AgentMux always sets `CLAUDE_CONFIG_DIR`.
///
/// The project folder is named by the CLI's own rules: after the directory
/// memory is keyed by ([`crate::backend::claude_layout::memory_project_root`]
/// — the repository's main checkout, for a subdirectory or a linked
/// worktree), sanitized by [`crate::backend::claude_layout::project_dir_name`].
pub(crate) fn memory_dir_for_cwd(claude_config_dir: &str, working_directory: &str) -> PathBuf {
    // Claude names the project folder from the absolute cwd, so `~` must be
    // expanded first — a spawn segment records the block's `cmd:cwd`
    // unexpanded (`~/.agentmux/agents/<slug>` for an agent.open default),
    // and `--agentmux-agents-…` is a folder Claude never writes (#3603).
    let working_directory = expand_home_dir_safe(working_directory);
    let root = crate::backend::claude_layout::memory_project_root(&working_directory);
    let folder_name = crate::backend::claude_layout::project_dir_name(&root.to_string_lossy());

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

/// The identity stores (`id_store`, `identity_store`), attached once at boot,
/// so memory resolution can find the account an agent is linked to — the
/// directory its spawn actually runs Claude in. Its callers hold only the
/// channel store. Unattached (tests): the linked-account step is skipped.
static IDENTITY_STORES: std::sync::OnceLock<(
    std::sync::Arc<crate::backend::storage::store::Store>,
    std::sync::Arc<crate::backend::storage::store::Store>,
)> = std::sync::OnceLock::new();

pub(crate) fn attach_identity_stores(
    id_store: std::sync::Arc<crate::backend::storage::store::Store>,
    identity_store: std::sync::Arc<crate::backend::storage::store::Store>,
) {
    let _ = IDENTITY_STORES.set((id_store, identity_store));
}

/// The Claude config dir an agent runs in, as its spawn decides it: the
/// config dir of the Claude OAuth account it is linked to — the spawn sets
/// `CLAUDE_CONFIG_DIR` to it over any env value (`resolver::inject`) — else
/// its env override, else `""`, the shared default. #3603: an agent with no
/// env override and a linked account resolved to the shared default, so
/// `MemoryList` listed nothing while its memories sat in the account's dir.
fn agent_claude_config_dir(mstore: &crate::backend::storage::store::Store, agent_id: &str) -> String {
    let linked = IDENTITY_STORES.get().and_then(|(id_store, identity_store)| {
        // Only an agent with a binding — its own, or its template's (the
        // spawn's fallback) — reaches the resolver, which logs a warning for
        // an unbound one; memory is read every sweep.
        crate::identity::resolver::agent_has_identity_binding(mstore, identity_store, agent_id)
            .then(|| {
                crate::identity::resolver::resolve_bound_claude_config_dir_for_agent(
                    mstore, id_store, identity_store, agent_id,
                )
            })
            .flatten()
    });
    if let Some(dir) = linked {
        return dir.to_string_lossy().into_owned();
    }
    mstore
        .agent_content_get(agent_id, "env")
        .ok()
        .flatten()
        .map(|c| parse_claude_config_dir(&c.content))
        .unwrap_or_default()
}

/// Resolve the memory directory for `agent_id`. Reads the agent definition and
/// its stored env blob to find `CLAUDE_CONFIG_DIR`. Returns an error only if
/// the agent cannot be resolved at all — an instance row with a blank
/// `working_directory` falls through to the registry rather than failing (see
/// the inline note below; that short-circuit was a real bug).
///
/// Shared by `native_memory_handlers` and the `memory.*` App API handlers.
pub(crate) fn memory_dir_for_agent(
    mstore: &crate::backend::storage::store::Store,
    agent_id: &str,
) -> Result<std::path::PathBuf, String> {
    // agent_id arriving from App API is the agent slug (AGENTMUX_AGENT_ID /
    // bus:register id), not a UUID and not the literal display name — use
    // instance_get_by_slug (agent_def_get queries by UUID and would always
    // return None here; instance_get_by_name matches the display name, a
    // different namespace — see that function's own doc comment).
    if let Some(instance) = mstore
        .instance_get_by_slug(agent_id)
        .map_err(|e| format!("memory: store: {e}"))?
    {
        // The agent's own latest spawn knows its directory for certain
        // (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2).
        if let Ok(Some(def)) = mstore.agent_def_get(&instance.id) {
            let gfs = crate::backend::agent_session::global_transcript_store().map(|a| a.as_ref());
            if let Some(dir) = memory_dir_from_latest_segment(gfs, &def.id) {
                return Ok(dir);
            }
        }
        // Only trust the instance row when it actually carries a working
        // directory. An empty one is NOT an error and must NOT short-circuit:
        // `agent.open` substitutes a default (`~/.agentmux/agents/<slug>`)
        // whenever `working_directory` is blank, so a blank row describes an
        // agent that is nonetheless running — and writing memories — in a
        // real directory. The registry fallback below is what knows that
        // real directory (`source_agents_base` + `working_dir`).
        //
        // Returning Err here instead made Armory → Bundle → Personal and the
        // MemoryList MCP tool fail with "agent <x> has no working directory"
        // for every such agent, while its memory files sat on disk perfectly
        // intact — the common case, since `working_directory` is blank by
        // default. See SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md.
        if !instance.working_directory.is_empty() {
            let config_dir = agent_claude_config_dir(mstore, &instance.id);
            return Ok(memory_dir_for_cwd(&config_dir, &instance.working_directory));
        }
        // Resolve through the DEFINITION, not the instance row: `db_agents`
        // stores `instance_name` as its own column, distinct from the
        // definition's `name`, and it is empty for a definition-only row —
        // so reading the name off `instance` here silently yielded "" and
        // fell through to a not-found. `instance.id` IS the definition id in
        // this consolidated table (see the note above).
        if let Ok(Some(def)) = mstore.agent_def_get(&instance.id) {
            if let Some(dir) =
                memory_dir_for_blank_working_dir(mstore, &def.id, &def.name, &def.slug)
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
    mstore: &crate::backend::storage::store::Store,
    agent_id: &str,
) -> Result<String, String> {
    crate::backend::agent_resolve::resolve_agent_id(mstore, agent_id)
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
    // Thin alias. The implementation moved to
    // `backend::agent_registry_lookup` so `backend::history` could share it
    // rather than keep a second copy of the slug-matching rules (reagentx P2,
    // PR #3480). Kept as a named function here because this module's callers
    // and its own tests refer to it, and the name says what it is at those
    // call sites.
    crate::backend::agent_registry_lookup::find_active_record_by_slug(agent_id)
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
    mstore: &crate::backend::storage::store::Store,
    definition_id: &str,
    agent_name: &str,
    slug: &str,
) -> Option<std::path::PathBuf> {
    if !slug.is_empty() {
        // Disambiguates by definition_id *inside* the lookup rather than
        // filtering after it. Same result when the slug is unique, but under a
        // collision the slug-only lookup now refuses to guess and returns None
        // (reagentx P1 on #3480) — this caller knows exactly which agent it
        // means, so it can still resolve.
        if let Some(rec) =
            crate::backend::agent_registry_lookup::find_active_record_by_slug_and_definition(
                slug,
                definition_id,
            )
        {
            if let Some(dir) = memory_dir_for_registry_record(&rec) {
                return Some(dir);
            }
        }
    }
    if agent_name.is_empty() {
        return None;
    }
    let config_dir = agent_claude_config_dir(mstore, definition_id);
    // Claude records the absolute cwd, so the default `~/.agentmux/agents/…`
    // must be expanded before it is turned into a project-folder name — the
    // unexpanded form named a folder (`--agentmux-agents-…`) Claude never
    // writes (#3603 review).
    let work_dir = crate::backend::base::expand_home_dir_safe(
        &crate::backend::storage::agents::default_agent_working_dir(agent_name),
    );
    Some(memory_dir_for_cwd(&config_dir, &work_dir.to_string_lossy()))
}

/// Reconstruct one registry record's absolute memory dir directly (no slug
/// lookup) — shared by [`memory_dir_from_registry`] (single record, found by
/// slug) and [`list_all_memory_targets`] (every active record, for the
/// fs-watch drift detector's enumeration).
///
/// The working directory comes from
/// [`crate::backend::agent_registry_lookup::working_dir_from_record`].
fn memory_dir_for_registry_record(rec: &crate::registry::NamedAgentRecord) -> Option<std::path::PathBuf> {
    let working_directory = crate::backend::agent_registry_lookup::working_dir_from_record(rec)?;

    let config_dir = claude_config_dir_for_identity(rec.data.identity_id.as_deref());
    Some(memory_dir_for_cwd(&config_dir, &working_directory))
}

/// Every agent whose memory directory is verified — found from its own
/// spawn or its explicit working directory — for the drift detector and the
/// first-sight backfill (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2,
/// phase M1).
///
/// Unverified guesses (the registry record, the name-derived default for a
/// blank working directory) are left out: a stale registry `identity_id`
/// once made this list attribute one agent's files to another. A directory
/// two agents resolve to is left out too — its files can't be attributed to
/// either — until the memory record's claims decide it (phase M3).
pub(crate) fn list_all_memory_targets(
    mstore: &crate::backend::storage::store::Store,
) -> Vec<(String, std::path::PathBuf)> {
    let gfs = crate::backend::agent_session::global_transcript_store().map(|a| a.as_ref());
    list_memory_targets_with(mstore, gfs)
}

fn list_memory_targets_with(
    mstore: &crate::backend::storage::store::Store,
    segments_fs: Option<&crate::backend::storage::filestore::FileStore>,
) -> Vec<(String, std::path::PathBuf)> {
    let Ok(agents) = mstore.agent_def_list() else {
        return Vec::new();
    };
    let mut targets: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for agent in &agents {
        if !seen.insert(agent.id.clone()) {
            continue;
        }
        if let Some(r) = resolve_memory_dir_with(mstore, agent, segments_fs) {
            if r.provenance != MemoryDirProvenance::Unverified {
                targets.push((agent.id.clone(), r.path));
            }
        }
    }
    let key = |p: &std::path::PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
    let mut owners: std::collections::HashMap<std::path::PathBuf, usize> = std::collections::HashMap::new();
    for (_, dir) in &targets {
        *owners.entry(key(dir)).or_default() += 1;
    }
    targets.retain(|(agent_id, dir)| {
        let shared = owners.get(&key(dir)).copied().unwrap_or(0) > 1;
        if shared {
            tracing::debug!(agent_id, dir = %dir.display(), "memory: directory resolved by more than one agent; not attributed");
        }
        !shared
    });
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

/// Validate a filename: alphanumeric + `-_`, must end with `.md`, no path separators.
pub(crate) fn validate_filename(filename: &str) -> Result<(), String> {
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
/// only sync the mirror on a Stash Bundle tab open, so exporting straight
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
    mstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
) -> Option<std::path::PathBuf> {
    resolve_memory_dir_by_id(mstore, agent).map(|r| r.path)
}

/// How an agent's memory directory was found
/// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, phase M1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryDirProvenance {
    /// The config dir and working dir the agent's own latest spawn used,
    /// from its continuity segment.
    Spawn,
    /// The agent's explicit working directory under its linked account.
    WorkingDirectory,
    /// A blank working directory with no spawn on record: the registry
    /// record or the name-derived default. A guess — the stale registry
    /// `identity_id` behind it once read another agent's files — so it is
    /// read-only and never swept for drift.
    Unverified,
}

pub(crate) struct ResolvedMemoryDir {
    pub(crate) path: std::path::PathBuf,
    pub(crate) provenance: MemoryDirProvenance,
}

/// The memory dir of the agent's latest Claude spawn, from its continuity
/// segments in the global transcript store.
fn memory_dir_from_latest_segment(
    fs: Option<&crate::backend::storage::filestore::FileStore>,
    agent_uid: &str,
) -> Option<std::path::PathBuf> {
    let latest = crate::backend::continuity_segments::segments(fs?, agent_uid)
        .into_iter()
        .filter(|s| s.start.provider == "claude" && !s.start.cwd.is_empty())
        .max_by_key(|s| s.start.started_at_ms)?;
    Some(memory_dir_for_cwd(latest.start.config_dir.as_deref().unwrap_or(""), &latest.start.cwd))
}

/// Resolve an agent's memory dir with how it was found: its latest spawn
/// first, then its explicit working directory, then the blank-working-dir
/// guess (read-only).
pub(crate) fn resolve_memory_dir_by_id(
    mstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
) -> Option<ResolvedMemoryDir> {
    let gfs = crate::backend::agent_session::global_transcript_store().map(|a| a.as_ref());
    resolve_memory_dir_with(mstore, agent, gfs)
}

fn resolve_memory_dir_with(
    mstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
    segments_fs: Option<&crate::backend::storage::filestore::FileStore>,
) -> Option<ResolvedMemoryDir> {
    if let Some(path) = memory_dir_from_latest_segment(segments_fs, &agent.id) {
        return Some(ResolvedMemoryDir { path, provenance: MemoryDirProvenance::Spawn });
    }
    legacy_memory_dir_by_id(mstore, agent).map(|(path, provenance)| ResolvedMemoryDir { path, provenance })
}

/// The memory dir of `agent_uid`'s latest spawn, if one is on record — for a
/// caller that knows the agent's id but has no local row for it.
pub(crate) fn memory_dir_from_spawn(agent_uid: &str) -> Option<std::path::PathBuf> {
    let gfs = crate::backend::agent_session::global_transcript_store().map(|a| a.as_ref());
    memory_dir_from_latest_segment(gfs, agent_uid)
}

/// The memory dir an AgentMux write may go to: never an unverified guess.
pub(crate) fn memory_dir_for_write_by_id(
    mstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
) -> Result<std::path::PathBuf, String> {
    match resolve_memory_dir_by_id(mstore, agent) {
        Some(r) if r.provenance != MemoryDirProvenance::Unverified => Ok(r.path),
        Some(_) => Err(format!(
            "agent {} has no verified memory directory yet (no working directory and no spawn on record); \
             it becomes writable after the agent's next launch",
            agent.id
        )),
        None => Err(format!("agent {} has no resolvable memory directory", agent.id)),
    }
}

/// Today's resolution without spawn records: explicit working directory,
/// else the blank-working-dir guess.
fn legacy_memory_dir_by_id(
    mstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
) -> Option<(std::path::PathBuf, MemoryDirProvenance)> {
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
        let config_dir = agent_claude_config_dir(mstore, &agent.id);
        return Some((
            memory_dir_for_cwd(&config_dir, &agent.working_directory),
            MemoryDirProvenance::WorkingDirectory,
        ));
    }
    memory_dir_for_blank_working_dir(mstore, &agent.id, &agent.name, &agent.slug)
        .map(|dir| (dir, MemoryDirProvenance::Unverified))
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

/// The content `agent:memory:read_file` would return for `path` right now —
/// live FS first (lossy UTF-8, capped at [`MAX_MEMORY_FILE_BYTES`]), the
/// durable mirror only when the live file is genuinely absent — WITHOUT
/// that handler's mirror-upsert side effect. `Ok(None)` means the file
/// exists in neither place. Used by `agent:memory:write_file`'s
/// `base_sha256` check, which must hash exactly what the caller's draft was
/// based on (the frontend hashes the `read_file` response), or every
/// conditional save of a large or non-UTF-8 file would look like a conflict.
fn current_memory_content(
    id_store: &crate::backend::storage::store::Store,
    agent_id: &str,
    path: &std::path::Path,
    filename: &str,
) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => {
            let mut buf = Vec::new();
            std::fs::File::open(path)
                .and_then(|f| {
                    use std::io::Read;
                    f.take(MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf)
                })
                .map_err(|e| format!("{filename}: {e}"))?;
            Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
        }
        Ok(_) => Err(format!("{filename} is not a regular file")),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{filename}: {e}")),
        Err(_) => id_store
            .agent_native_memory_read(agent_id, filename)
            .map_err(|e| format!("{filename}: mirror lookup failed: {e}")),
    }
}

/// The optimistic-concurrency check behind `base_sha256`
/// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4): `Ok` when `current`
/// still hashes to `base`, otherwise an error starting `conflict:` — the
/// marker the frontend keys its "changed since you started editing" banner
/// on. A file that no longer exists is a conflict too: the draft's base is
/// gone, and silently re-creating it would undo someone else's delete.
fn check_memory_base_sha256(filename: &str, base: &str, current: Option<&str>) -> Result<(), String> {
    match current {
        None => Err(format!(
            "conflict: {filename} no longer exists (your edit was based on {base}); not written"
        )),
        Some(content) => {
            let current_hash = crate::backend::storage::agent_native_memory_versions::content_hash(content);
            if current_hash.eq_ignore_ascii_case(base) {
                Ok(())
            } else {
                Err(format!(
                    "conflict: {filename} changed since your edit began (base {base}, current {current_hash}); not written"
                ))
            }
        }
    }
}

pub fn register_native_memory_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore_list = state.mstore.clone();
    let id_store_list = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_LIST,
        move |cmd: CommandNativeMemoryListData, _ctx| {
            let mstore = mstore_list.clone();
            let id_store = id_store_list.clone();
            async move {

                let agent = mstore
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
                let resolved = resolve_memory_dir_by_id(&mstore, &agent).ok_or_else(|| {
                    format!("agent:memory:list: agent {} has no resolvable memory directory", cmd.agent_id)
                })?;
                let unverified = resolved.provenance == MemoryDirProvenance::Unverified;
                let memory_dir = resolved.path;

                // Existing mirror metadata (no content) for this agent, keyed by
                // filename — lets the loop below skip the expensive full-content
                // read+upsert for a file that hasn't changed since it was last
                // mirrored, instead of doing it unconditionally on every list
                // call (reagent P1 on PR #2459: `list` fires on every Stash
                // Bundle tab open/refresh, so an unconditional full read + SQLite
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

                Ok(NativeMemoryListResult { files, unverified })
            }
        },
    );

    let mstore_read = state.mstore.clone();
    let id_store_read = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_READ_FILE,
        move |cmd: CommandNativeMemoryReadFileData, _ctx| {
            let mstore = mstore_read.clone();
            let id_store = id_store_read.clone();
            async move {

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:read_file: {e}"))?;

                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:read_file: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:read_file: agent {} not found", cmd.agent_id))?;

                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let path = memory_dir_for_agent_by_id(&mstore, &agent)
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

                Ok(NativeMemoryReadFileResult { content })
            }
        },
    );

    let mstore_write = state.mstore.clone();
    let id_store_write = state.id_store.clone();
    let broker_write = state.broker.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        move |cmd: CommandNativeMemoryWriteFileData, _ctx| {
            let mstore = mstore_write.clone();
            let id_store = id_store_write.clone();
            let broker = broker_write.clone();
            async move {

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

                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:write_file: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:write_file: agent {} not found", cmd.agent_id))?;

                // See the identical comment on the list handler above — a
                // blank working_directory is not "no memory dir".
                let dir = memory_dir_for_write_by_id(&mstore, &agent)
                    .map_err(|e| format!("agent:memory:write_file: {e}"))?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("agent:memory:write_file: mkdir: {e}"))?;

                // Conditional save (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
                // §2.4): a caller that says which content its draft started
                // from gets a refusal, not an overwrite, when that content
                // has since moved. Checked before the version row below so a
                // refused write leaves no trace in history. Not atomic with
                // the rename further down — a write landing in between still
                // wins — but it closes the window from minutes (a human
                // editing) to microseconds, which is the case this exists for.
                if let Some(base) = cmd.base_sha256.as_deref() {
                    let current = current_memory_content(&id_store, &agent.id, &dir.join(&cmd.filename), &cmd.filename)
                        .map_err(|e| format!("agent:memory:write_file: {e}"))?;
                    check_memory_base_sha256(&cmd.filename, base, current.as_deref())
                        .map_err(|e| format!("agent:memory:write_file: {e}"))?;
                }

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
                // And into the agent's memory record, likewise first
                // (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1).
                crate::backend::memory_reconcile::record_agentmux_write(
                    &agent.id,
                    &dir,
                    &cmd.filename,
                    cmd.content.as_bytes(),
                    version_source,
                    &version_detail,
                );

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

                // Reactive Armory updates (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md):
                // same event, same `agent:memory:changed:{canonical_uuid}`
                // naming convention `app_api::mod`'s write/revert already use
                // for the MemoryWrite MCP tool's own path — keying by
                // `agent.id` (the canonical UUID this whole file's other
                // handlers already key by, per `resolve_agent_uuid`'s own
                // doc comment on why a slug-keyed event here would silently
                // miss frontend subscribers that only ever have the UUID).
                broker.publish(crate::backend::mps::MuxEvent {
                    event: format!("agent:memory:changed:{}", agent.id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                tracing::info!(
                    agent_id = %cmd.agent_id,
                    filename = %cmd.filename,
                    bytes = cmd.content.len(),
                    "agent:memory:write_file"
                );
                // `register_typed` always sends a `data` field, so this `()`
                // puts `"data": null` on the wire where the old untyped
                // `Ok(None)` omitted the key entirely. All three frontend
                // callers (agent-native-memory-model.ts's draft `save` and
                // `createFile`, NativeMemoryFileView.tsx's draft `save`) await
                // and discard the result, and no other crate invokes this
                // command, so the difference is unobservable.
                Ok(())
            }
        },
    );

    // Offers only: adopting a listed folder goes through the host's
    // confirmation window (`service/memory_adopt.rs`), never this RPC — every
    // agent holds the key this one is called with.
    let mstore_adoption = state.mstore.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_ADOPTION_LIST,
        move |cmd: CommandNativeMemoryAdoptionListData, _ctx| {
            let mstore = mstore_adoption.clone();
            async move {
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:adoption_list: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:adoption_list: agent {} not found", cmd.agent_id))?;
                let Some(fs) = crate::backend::agent_session::global_transcript_store() else {
                    return Ok(NativeMemoryAdoptionListResult { list: None });
                };
                let list = tokio::task::spawn_blocking(move || crate::backend::memory_adopt::list(fs, &mstore, &agent))
                    .await
                    .map_err(|e| format!("agent:memory:adoption_list: {e}"))?
                    .map_err(|e| format!("agent:memory:adoption_list: {e}"))?;
                Ok(NativeMemoryAdoptionListResult { list })
            }
        },
    );

    // Offers only, like the adoption list: releasing goes through the host.
    let mstore_claims = state.mstore.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_CLAIMS,
        move |cmd: CommandNativeMemoryClaimsData, _ctx| {
            let mstore = mstore_claims.clone();
            async move {
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:claims: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:claims: agent {} not found", cmd.agent_id))?;
                let fs = crate::backend::agent_session::global_transcript_store()
                    .ok_or_else(|| "agent:memory:claims: memory record store unavailable".to_string())?;
                tokio::task::spawn_blocking(move || crate::backend::memory_release::list(fs, &agent.id))
                    .await
                    .map_err(|e| format!("agent:memory:claims: {e}"))?
                    .map_err(|e| format!("agent:memory:claims: {e}"))
            }
        },
    );

    let mstore_history = state.mstore.clone();
    let id_store_history = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_HISTORY,
        move |cmd: CommandNativeMemoryHistoryData, _ctx| {
            let mstore = mstore_history.clone();
            let id_store = id_store_history.clone();
            async move {

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
                let agent = mstore
                    .agent_def_get(&cmd.agent_id)
                    .map_err(|e| format!("agent:memory:history: store: {e}"))?
                    .ok_or_else(|| format!("agent:memory:history: agent {} not found", cmd.agent_id))?;

                let versions = id_store
                    .agent_native_memory_version_list(&agent.id, &cmd.filename)
                    .map_err(|e| format!("agent:memory:history: store: {e}"))?
                    .into_iter()
                    .map(version_summary_to_meta)
                    .collect();

                Ok(NativeMemoryHistoryResult { versions })
            }
        },
    );

    let mstore_diff = state.mstore.clone();
    let id_store_diff = state.id_store.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_DIFF,
        move |cmd: CommandNativeMemoryDiffData, _ctx| {
            let mstore = mstore_diff.clone();
            let id_store = id_store_diff.clone();
            async move {

                // Resolve to the same canonical agent.id write_file keys by
                // — see the identical comment on the history handler above.
                let agent = mstore
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
                Ok(NativeMemoryDiffResult { diff })
            }
        },
    );

    let mstore_revert = state.mstore.clone();
    let id_store_revert = state.id_store.clone();
    let broker_revert = state.broker.clone();
    engine.register_typed(
        COMMAND_NATIVE_MEMORY_REVERT,
        move |cmd: CommandNativeMemoryRevertData, _ctx| {
            let mstore = mstore_revert.clone();
            let id_store = id_store_revert.clone();
            let broker = broker_revert.clone();
            async move {

                validate_filename(&cmd.filename)
                    .map_err(|e| format!("agent:memory:revert: {e}"))?;

                // Resolve to the same canonical agent.id write_file keys by
                // — see the identical comment on the history handler above.
                // Resolved BEFORE the ownership check below (not after, as
                // an earlier revision of this handler did) so that check
                // compares against the same id the version was actually
                // stored under, not the raw, possibly-different cmd.agent_id.
                let agent = mstore
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
                let dir = memory_dir_for_write_by_id(&mstore, &agent).map_err(|e| {
                    format!("agent:memory:revert: {e}")
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
                crate::backend::memory_reconcile::record_agentmux_write(
                    &agent.id,
                    &dir,
                    &cmd.filename,
                    target.content.as_bytes(),
                    "revert",
                    &detail,
                );

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

                // See the identical comment on write_file's own handler above.
                broker.publish(crate::backend::mps::MuxEvent {
                    event: format!("agent:memory:changed:{}", agent.id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                tracing::info!(
                    agent_id = %cmd.agent_id,
                    filename = %cmd.filename,
                    target_version_id = %cmd.target_version_id,
                    "agent:memory:revert"
                );
                Ok(NativeMemoryRevertResult { version: version_to_meta(&new_version) })
            }
        },
    );
}

// Request-shape tests for the six `agent:memory:*` commands.
//
// These exist because NOTHING else catches a Req/payload mismatch: `tsc` only
// checks the frontend against the GENERATED types, and
// `scripts/check-rpc-bindings.sh` only checks that a generated type exists for
// each command — neither one ever deserializes a real payload into the Rust
// struct. A `Req` that cannot parse what the stub sends compiles, typechecks,
// passes the binding gate, and then fails on every call at runtime. That is
// exactly the bug found on `bookmarks.list` (`Req = ()` rejects `{}`), so each
// command below is pinned to the literal JSON its call site actually sends.
#[cfg(test)]
#[path = "tests/native_memory_handlers/req_shape_tests.rs"]
mod req_shape_tests;

#[cfg(test)]
#[path = "tests/native_memory_handlers/tests.rs"]
mod tests;
