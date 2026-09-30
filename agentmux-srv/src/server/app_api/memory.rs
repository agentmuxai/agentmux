use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_memory_list(engine, state);
    register_memory_read(engine, state);
    register_memory_write(engine, state);
}

fn register_memory_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_MEMORY_LIST,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("memory.list: {e}"))?;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;
                Ok(Some(memory_list_impl(&state, &req.agent_id)?))
            })
        }),
    );
}

fn register_memory_read(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_MEMORY_READ,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, filename: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("memory.read: {e}"))?;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;
                Ok(Some(memory_read_impl(&state, &req.agent_id, &req.filename)?))
            })
        }),
    );
}

fn register_memory_write(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_MEMORY_WRITE,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, filename: String, content: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("memory.write: {e}"))?;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;
                memory_write_impl(&state, &req.agent_id, &req.filename, &req.content, None)?;
                Ok(None)
            })
        }),
    );
}

pub(crate) fn memory_list_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
) -> Result<serde_json::Value, String> {
    let owner = owner.into();
    let memory_dir = owner.dir(&state.mstore).map_err(|e| format!("memory.list: {e}"))?;

    let mut files: Vec<NativeMemoryFileMeta> = Vec::new();
    let entries = match std::fs::read_dir(&memory_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({ "files": [] }));
        }
        Err(e) => return Err(format!("memory.list: read_dir: {e}")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("memory.list: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") { continue; }
        let file_type = entry.file_type()
            .map_err(|e| format!("memory.list: file_type {name}: {e}"))?;
        if !file_type.is_file() { continue; }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("memory.list: metadata {name}: {e}")),
        };
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
        files.push(NativeMemoryFileMeta {
            is_index: name == "MEMORY.md",
            metadata_type: crate::server::native_memory_handlers::parse_memory_frontmatter_type(&preview_content),
            size_bytes: meta.len(),
            modified_at: meta.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            filename: name,
        });
    }
    files.sort_by(|a, b| b.is_index.cmp(&a.is_index).then(a.filename.cmp(&b.filename)));
    serde_json::to_value(NativeMemoryListResult { files, unverified: false }).map_err(|e| e.to_string())
}

pub(crate) fn memory_read_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
    filename: &str,
) -> Result<serde_json::Value, String> {
    let owner = owner.into();
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.read: {e}"))?;
    let path = owner.dir(&state.mstore).map_err(|e| format!("memory.read: {e}"))?.join(filename);

    let file_type = std::fs::symlink_metadata(&path)
        .map_err(|e| format!("memory.read: {filename}: {e}"))?.file_type();
    if !file_type.is_file() {
        return Err(format!("memory.read: {filename} is not a regular file"));
    }
    const MAX: u64 = 10 * 1024 * 1024;
    let mut buf = Vec::new();
    std::fs::File::open(&path)
        .and_then(|f| { use std::io::Read; f.take(MAX).read_to_end(&mut buf) })
        .map_err(|e| format!("memory.read: {filename}: {e}"))?;
    let content = String::from_utf8_lossy(&buf).into_owned();
    serde_json::to_value(NativeMemoryReadFileResult { content }).map_err(|e| e.to_string())
}

/// Caller-supplied provenance for a `memory.write` call — mirrors
/// `NativeMemoryWriteProvenance` (the WebSocket RPC's own wire shape) but
/// kept as plain `&str`s here rather than importing that type, since this
/// impl fn is also called directly from tests without going through the
/// RPC layer at all. See
/// docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.1.
pub(crate) struct MemoryWriteProvenance<'a> {
    pub source: &'a str,
    pub detail: &'a str,
}

pub(crate) fn memory_write_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
    filename: &str,
    content: &str,
    provenance: Option<MemoryWriteProvenance<'_>>,
) -> Result<(), String> {
    let owner = owner.into();
    let agent_id = owner.label();
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.write: {e}"))?;
    const MAX: usize = 10 * 1024 * 1024;
    if content.len() > MAX {
        return Err(format!("memory.write: content too large ({} bytes, max {MAX})", content.len()));
    }
    let dir = owner.dir_for_write(&state.mstore).map_err(|e| format!("memory.write: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("memory.write: mkdir: {e}"))?;

    // Version history recorded BEFORE the live-file write below — reagent
    // P1: closes the same fs-watch race described in
    // native_memory_handlers.rs's write_file handler. This is the path the
    // MemoryWrite MCP tool actually calls in production — instrumenting it
    // matters at least as much as the WebSocket RPC path, since it's what
    // an agent's own MemoryWrite tool call hits. Non-fatal on failure — a
    // durability/review layer on top of the write, not the write itself.
    //
    // Keyed by the RESOLVED canonical id, not the raw `agent_id` (slug)
    // parameter — reagent P1: this surface previously stored versions
    // slug-keyed while the WS RPC surface stores them keyed by the
    // resolved `AgentDefinition.id`, so a version written here was
    // invisible to a WS-RPC-based history/diff/revert call for the same
    // logical agent whenever slug != id. See `resolve_agent_uuid`'s own
    // doc for the full resolution-order rationale.
    //
    // Hard-fails on resolution failure — reagent P2 (re-review): this used
    // to silently fall back to the raw slug via `unwrap_or_else`, while
    // memory_history_impl/memory_diff_impl/memory_revert_impl all hard-fail
    // on the identical call. The owner's directory above already resolved
    // (proving it resolves via at least one lookup path), so a
    // failure here is very likely transient (e.g. a registry-file I/O
    // hiccup) rather than "unknown agent" — but silently keying this
    // version by the raw slug on that failure would reintroduce the exact
    // disjoint-keyspace bug (a write invisible to history/diff/revert) this
    // PR exists to fix. A visible, retriable write failure is strictly
    // safer than a silent data-integrity split.
    let version_agent_id = owner.owner_id(&state.mstore)
        .map_err(|e| format!("memory.write: {e}"))?;
    let (source, detail) = match &provenance {
        Some(p) => (p.source, p.detail),
        None => ("agent_inferred", "{}"),
    };
    if let Err(e) = state.id_store.agent_native_memory_version_insert(&version_agent_id, filename, content, source, detail, "") {
        tracing::warn!(agent_id, filename, error = %e, "memory.write: version insert failed (non-fatal)");
    }
    // And into the agent's memory record, likewise first
    // (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1).
    crate::backend::memory_reconcile::record_agentmux_write(&version_agent_id, &dir, filename, content.as_bytes(), source, detail);

    let dest = dir.join(filename);
    let tmp = dir.join(format!(".{}.{}.tmp", filename, uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&tmp, content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.write: write tmp: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.write: rename: {e}"));
    }

    // reagent P1 on PR #2674 (found on memory_revert_impl, same gap exists
    // here): this App-API path backing the `MemoryWrite` MCP tool never
    // updated the `db_agent_native_memory` mirror row, unlike its WS-RPC
    // sibling `agent:memory:write_file` in native_memory_handlers.rs. Per
    // `read_file`'s own fallback logic, a channel with no live copy of the
    // file falls back to the mirror and treats a stale row as permanent —
    // so a write issued through the MCP tool never propagated cross-channel.
    let metadata_type = crate::server::native_memory_handlers::parse_memory_frontmatter_type(content);
    let dest_meta = std::fs::metadata(&dest).ok();
    let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(content.len() as i64);
    let mtime_ms = dest_meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Err(e) = state.id_store.agent_native_memory_upsert(
        &version_agent_id,
        filename,
        content,
        metadata_type.as_deref(),
        &dest.to_string_lossy(),
        size_bytes,
        mtime_ms,
    ) {
        tracing::warn!(agent_id, filename, error = %e, "memory.write: mirror upsert failed (non-fatal)");
    }

    // Keyed by `version_agent_id` (the resolved canonical UUID), not the raw
    // `agent_id` slug parameter — SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md:
    // the WS RPC surface (native_memory_handlers.rs) publishes this same
    // event keyed by `agent.id`, and a frontend subscriber only ever has
    // that UUID (`AgentDefinition.id`), never the App-API-only slug
    // namespace. Slug-keying here would silently split writes made through
    // this surface (the MemoryWrite MCP tool) into a keyspace no UI
    // subscriber can ever match — the exact class of bug this file's own
    // `resolve_agent_uuid` doc comment already warns about for version
    // storage; the same reasoning applies to this event.
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: format!("agent:memory:changed:{version_agent_id}"),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });
    Ok(())
}

// ---- Global Memory (agent-facing) — SPEC_AGENT_FACING_GLOBAL_MEMORY_API_
// 2026_09_15.md Phase 1. Unlike native memory above (per-agent, filesystem-
// backed), Global Memory is a pure `db_bundles` row an agent can read/write
// through here for the FIRST time — previously only reachable via the
// authenticated WebSocket RPC channel the frontend uses
// (`agent_handlers/bundle.rs`'s `upsertmemory`), which `agentmux-mcp`
// cannot reach at all. Every function here calls the SAME
// `Store::bundle_upsert`/`bundle_get`/`bundle_list_global` the human-facing
// Armory editor already uses — no new storage-layer write path, no new way
// to touch a row this app didn't already know how to touch.
//
// The one invariant every function below shares: NONE of them ever read or
// set `is_system` from caller input, and none of them accept/return a
// system-tier row. This is enforced twice, independently, on purpose:
// `Store::bundle_upsert` itself already refuses to touch an existing
// `is_system=1` row (SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md), and
// separately, the code below never even constructs a request/response that
// could carry `is_system=true` in the first place. A future refactor would
// have to break BOTH independently to let an agent reach the system tier
// through this surface.


pub(crate) fn memory_history_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
    filename: &str,
) -> Result<serde_json::Value, String> {
    let owner = owner.into();
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.history: {e}"))?;
    // See memory_write_impl's own comment — must key by the same resolved
    // canonical id that write used, not the raw slug.
    let version_agent_id = owner.owner_id(&state.mstore)
        .map_err(|e| format!("memory.history: {e}"))?;
    let versions: Vec<crate::backend::rpc_types::NativeMemoryVersionMeta> = state
        .id_store
        .agent_native_memory_version_list(&version_agent_id, filename)
        .map_err(|e| format!("memory.history: store: {e}"))?
        .into_iter()
        .map(|v| crate::backend::rpc_types::NativeMemoryVersionMeta {
            id: v.id,
            content_hash: v.content_hash,
            parent_version_id: v.parent_version_id,
            source: v.source,
            source_detail: v.source_detail,
            session_id: v.session_id,
            created_at: v.created_at,
        })
        .collect();
    serde_json::to_value(crate::backend::rpc_types::NativeMemoryHistoryResult { versions })
        .map_err(|e| e.to_string())
}

pub(crate) fn memory_diff_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
    from_version_id: &str,
    to_version_id: &str,
) -> Result<serde_json::Value, String> {
    let owner = owner.into();
    let agent_id = owner.label();
    // See memory_write_impl's own comment — must compare against the same
    // resolved canonical id that write used, not the raw slug.
    let version_agent_id = owner.owner_id(&state.mstore)
        .map_err(|e| format!("memory.diff: {e}"))?;
    let from = state
        .id_store
        .agent_native_memory_version_get(from_version_id)
        .map_err(|e| format!("memory.diff: store: {e}"))?
        .ok_or_else(|| format!("memory.diff: version {from_version_id} not found"))?;
    let to = state
        .id_store
        .agent_native_memory_version_get(to_version_id)
        .map_err(|e| format!("memory.diff: store: {e}"))?
        .ok_or_else(|| format!("memory.diff: version {to_version_id} not found"))?;
    // reagent P1 — see the identical check in native_memory_handlers.rs's
    // WS RPC handler for the full rationale.
    if from.agent_id != version_agent_id || to.agent_id != version_agent_id {
        return Err(format!("memory.diff: one or both versions do not belong to {agent_id}"));
    }
    // reagent P2 — see the identical check in native_memory_handlers.rs's
    // WS RPC handler for the full rationale.
    if from.filename != to.filename {
        return Err(format!(
            "memory.diff: from_version_id and to_version_id are versions of different files ({} vs {})",
            from.filename, to.filename
        ));
    }
    let diff = crate::server::native_memory_handlers::line_diff(&from.content, &to.content);
    serde_json::to_value(crate::backend::rpc_types::NativeMemoryDiffResult { diff }).map_err(|e| e.to_string())
}

pub(crate) fn memory_revert_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
    filename: &str,
    target_version_id: &str,
) -> Result<serde_json::Value, String> {
    let owner = owner.into();
    let agent_id = owner.label();
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.revert: {e}"))?;

    // See memory_write_impl's own comment — must key/compare against the
    // same resolved canonical id that write used, not the raw slug.
    let version_agent_id = owner.owner_id(&state.mstore)
        .map_err(|e| format!("memory.revert: {e}"))?;

    let target = state
        .id_store
        .agent_native_memory_version_get(target_version_id)
        .map_err(|e| format!("memory.revert: store: {e}"))?
        .ok_or_else(|| format!("memory.revert: version {target_version_id} not found"))?;
    if target.agent_id != version_agent_id || target.filename != filename {
        return Err(format!(
            "memory.revert: version {target_version_id} does not belong to {agent_id}/{filename}"
        ));
    }

    let dir = owner.dir_for_write(&state.mstore).map_err(|e| format!("memory.revert: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("memory.revert: mkdir: {e}"))?;

    // Version recorded BEFORE the live-file write below — same fs-watch-race
    // rationale as memory_write_impl above (reagent P1). Fatal on failure,
    // same reasoning as the WS RPC revert handler: silently reverting the
    // file but failing to record what it was reverted to would leave the
    // caller with no way to know.
    let detail = json!({ "reverted_to": target_version_id }).to_string();
    let new_version = state
        .id_store
        .agent_native_memory_version_insert(&version_agent_id, filename, &target.content, "revert", &detail, "")
        .map_err(|e| format!("memory.revert: version insert: {e}"))?;

    let dest = dir.join(filename);
    let tmp = dir.join(format!(".{}.{}.tmp", filename, uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&tmp, &target.content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.revert: write tmp: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.revert: rename: {e}"));
    }

    // reagent P1 on PR #2674: this App-API path backing the `MemoryRevert`
    // MCP tool reverted the live file but never updated the
    // `db_agent_native_memory` mirror row, unlike its WS-RPC sibling
    // `agent:memory:revert` in native_memory_handlers.rs. Per `read_file`'s
    // own fallback logic, a channel with no live copy of the file falls
    // back to the mirror and treats a stale mirror row as permanent, not
    // briefly stale — so a revert issued through the actual `MemoryRevert`
    // tool silently failed to propagate cross-channel, leaving other
    // channels still showing the pre-revert (fabricated) content
    // indefinitely.
    let metadata_type = crate::server::native_memory_handlers::parse_memory_frontmatter_type(&target.content);
    let dest_meta = std::fs::metadata(&dest).ok();
    let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(target.content.len() as i64);
    let mtime_ms = dest_meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Err(e) = state.id_store.agent_native_memory_upsert(
        &version_agent_id,
        filename,
        &target.content,
        metadata_type.as_deref(),
        &dest.to_string_lossy(),
        size_bytes,
        mtime_ms,
    ) {
        tracing::warn!(agent_id, filename, error = %e, "memory.revert: mirror upsert failed (non-fatal)");
    }

    // See memory_write_impl's own comment on why this is version_agent_id
    // (canonical UUID), not the raw agent_id slug parameter.
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: format!("agent:memory:changed:{version_agent_id}"),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });

    serde_json::to_value(crate::backend::rpc_types::NativeMemoryRevertResult {
        version: crate::backend::rpc_types::NativeMemoryVersionMeta {
            id: new_version.id,
            content_hash: new_version.content_hash,
            parent_version_id: new_version.parent_version_id,
            source: new_version.source,
            source_detail: new_version.source_detail,
            session_id: new_version.session_id,
            created_at: new_version.created_at,
        },
    })
    .map_err(|e| e.to_string())
}
