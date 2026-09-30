// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Global Memory App API bodies, moved out of app_api/mod.rs unchanged
//! (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md item 9).

use super::*;

/// Caller-supplied provenance for a `globalmemory.write` call — same shape
/// as `MemoryWriteProvenance` above; kept as a distinct type since callers
/// are conceptually different (an agent writing to a workspace-wide list
/// every other agent inherits, not its own private memory) even though the
/// wire shape is identical.
pub(crate) struct GlobalMemoryWriteProvenance<'a> {
    pub source: &'a str,
    pub detail: &'a str,
}

/// Creates a new ordinary Global Memory entry, or updates an existing one
/// by `id`. Backs the `GlobalMemoryWrite` MCP tool.
pub(crate) fn global_memory_write_impl(
    state: &AppState,
    agent_id: &str,
    written_by_uid: &str,
    id: Option<&str>,
    name: &str,
    content: &str,
    provenance: Option<GlobalMemoryWriteProvenance<'_>>,
) -> Result<serde_json::Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("globalmemory.write: name is required".to_string());
    }
    // Generous but bounded — Global Memory entries are meant to be readable
    // instructions text, not arbitrary file storage (memory.write's own
    // 10MB cap is for markdown notes files; this is an order of magnitude
    // tighter since every agent's launch has to parse and inject this).
    const MAX: usize = 1024 * 1024;
    if content.len() > MAX {
        return Err(format!("globalmemory.write: content too large ({} bytes, max {MAX})", content.len()));
    }

    let now = agentmux_common::time::now_ms();
    let is_new = id.map(|v| v.is_empty()).unwrap_or(true);
    let bundle = match id {
        Some(existing_id) if !existing_id.is_empty() => {
            let mut existing = state.id_store.bundle_get(existing_id)
                .map_err(|e| format!("globalmemory.write: {e}"))?
                .ok_or_else(|| format!("globalmemory.write: not found id={existing_id}"))?;
            // Belt-and-suspenders with bundle_upsert's own guard (see this
            // section's top-of-block comment) — refuse here too, before
            // even attempting the write, so the error message is specific
            // to this API rather than bundle_upsert's generic one.
            if existing.is_system {
                return Err(
                    "globalmemory.write: cannot modify a system Global Memory entry through this API"
                        .to_string(),
                );
            }
            // reagent P0, PR #3237: `id` is caller-supplied and bundle ids
            // for EVERY bundle in the system (not just global ones) are
            // already enumerable by any agent via PresetList
            // (bundle_list_impl, unfiltered). Without these two checks, an
            // agent could target any other agent's private preset — or the
            // seeded 'blank' singleton — by id, rename/replace its content,
            // and force it into is_global=true, hijacking it into every
            // agent's launch instructions workspace-wide. Mirrors
            // upsertmemory's own already-fixed blank-singleton guard
            // (agent_handlers/bundle.rs, reagent P1, 2026-05-08) and adds
            // the is_global check that guard alone doesn't cover: this API
            // may only ever EDIT an entry that was already a Global Memory
            // entry BEFORE this call, never silently promote/adopt an
            // unrelated bundle into one.
            if existing.is_blank || existing.id == "blank" {
                return Err("globalmemory.write: cannot mutate the blank Bundle singleton".to_string());
            }
            if !existing.is_global {
                return Err(format!(
                    "globalmemory.write: id={existing_id} is not a Global Memory entry — this API cannot adopt an unrelated bundle, only create a new entry (omit id) or edit an existing Global Memory one"
                ));
            }
            existing.name = name.to_string();
            existing.instructions = content.to_string();
            existing.is_global = true;
            existing.updated_at = now;
            existing
        }
        _ => Bundle {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: String::new(),
            is_blank: false,
            is_global: true,
            provider: String::new(),
            model: String::new(),
            instructions: content.to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            is_system: false,
            created_at: now,
            updated_at: now,
        },
    };

    // `written_by` is the TRUSTED `agent_id` this function received
    // (agentmux-mcp's own AGENTMUX_AGENT_ID, unforgeable from the agent's
    // PTY — see this section's top-of-block comment) — never taken from
    // caller-supplied `source`/`source_detail`, which stay free-form
    // annotation. bundle_upsert_with_version records both, atomically with
    // the bundle write itself (codex P2, PR #3237 — see that method's own
    // doc comment for why two separate calls here would have been unsafe
    // under concurrent writers). `written_by_uid` is the writer's UID from
    // the request's `Caller` (identity M4c-1, spec §6.5.9) — its token,
    // never a body field; `""` when Unattributed.
    let (source, detail) = match &provenance {
        Some(p) => (p.source, p.detail),
        None => ("agent_inferred", "{}"),
    };
    let source_detail = if detail.is_empty() { "{}" } else { detail };
    state.id_store
        .bundle_upsert_with_version(&bundle, agent_id, written_by_uid, source, source_detail)
        .map_err(|e| format!("globalmemory.write: {e}"))?;

    // New entries sort LAST, matching the Armory UI's own "+ New section"
    // behavior (global-bundle-model.ts's saveEdit) — without this, a fresh
    // bundle_id defaults to sort_order=0 and could land ambiguously
    // interleaved with any other 0-sort_order row instead of visibly
    // appended at the end of the order the operator/agent expects.
    if is_new {
        let ordinary_ids: Vec<String> = state.id_store.bundle_list_global()
            .map_err(|e| format!("globalmemory.write: {e}"))?
            .into_iter()
            .filter(|b| !b.is_system && b.id != bundle.id)
            .map(|b| b.id)
            .chain(std::iter::once(bundle.id.clone()))
            .collect();
        if let Err(e) = state.id_store.bundle_reorder(&ordinary_ids) {
            tracing::warn!(agent_id, bundle_id = %bundle.id, error = %e, "globalmemory.write: append-to-order failed (non-fatal)");
        }
    }

    state.broker.publish(crate::backend::mps::MuxEvent {
        event: "memories:changed".to_string(),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });
    tracing::info!(agent_id, bundle_id = %bundle.id, name = %bundle.name, "globalmemory.write");
    Ok(json!({ "id": bundle.id, "name": bundle.name }))
}

/// Ordinary (non-system) Global Memory entries only — id/name/updated_at,
/// no content (mirrors `memory_list_impl`'s summary shape). The `is_system`
/// filter is structural here, not just a response projection. Backs the
/// `GlobalMemoryList` MCP tool.
pub(crate) fn global_memory_list_impl(state: &AppState) -> Result<serde_json::Value, String> {
    let entries: Vec<_> = state.id_store.bundle_list_global()
        .map_err(|e| format!("globalmemory.list: {e}"))?
        .into_iter()
        .filter(|b| !b.is_system)
        .map(|b| json!({ "id": b.id, "name": b.name, "updated_at": b.updated_at }))
        .collect();
    Ok(json!({ "entries": entries }))
}

/// Full content of one ordinary Global Memory entry by id. Refuses a
/// system-tier id, same reasoning as write. Backs the `GlobalMemoryRead`
/// MCP tool.
pub(crate) fn global_memory_read_impl(state: &AppState, id: &str) -> Result<serde_json::Value, String> {
    let bundle = state.id_store.bundle_get(id)
        .map_err(|e| format!("globalmemory.read: {e}"))?
        .ok_or_else(|| format!("globalmemory.read: not found id={id}"))?;
    if bundle.is_system {
        return Err("globalmemory.read: cannot read a system Global Memory entry through this API".to_string());
    }
    if !bundle.is_global {
        return Err(format!("globalmemory.read: {id} is not a Global Memory entry"));
    }
    Ok(json!({ "id": bundle.id, "name": bundle.name, "content": bundle.instructions }))
}

/// Demotes a Global Memory entry (clears `is_global`) — matches the Armory
/// UI's own "Remove" semantics: the bundle row itself survives (still
/// available in the Bundles tab), only its Global Memory membership is
/// cleared. Refuses a system-tier id, the blank singleton, or any id that
/// wasn't ALREADY a Global Memory entry before this call — same ownership
/// guards as `global_memory_write_impl`, for the identical reason: bundle
/// ids are enumerable via `PresetList` regardless of `is_global`. Backs the
/// `GlobalMemoryRemove` MCP tool.
pub(crate) fn global_memory_remove_impl(state: &AppState, id: &str) -> Result<serde_json::Value, String> {
    let mut bundle = state.id_store.bundle_get(id)
        .map_err(|e| format!("globalmemory.remove: {e}"))?
        .ok_or_else(|| format!("globalmemory.remove: not found id={id}"))?;
    if bundle.is_system {
        return Err("globalmemory.remove: cannot remove a system Global Memory entry through this API".to_string());
    }
    // reagent P1, PR #3237 (re-review): same missing-ownership-check bug
    // already fixed for write above, left unfixed here. Bundle ids for
    // every bundle are enumerable via PresetList (unfiltered), so without
    // this an agent could call GlobalMemoryRemove with another agent's
    // private preset id (or 'blank') — is_global is already false so the
    // flag write is a no-op, but bundle_upsert still rewrites the row and
    // bumps updated_at on a bundle this API has no business touching at
    // all. This API may only ever act on an entry that was already Global
    // Memory before the call.
    if bundle.is_blank || bundle.id == "blank" {
        return Err("globalmemory.remove: cannot mutate the blank Bundle singleton".to_string());
    }
    if !bundle.is_global {
        return Err(format!("globalmemory.remove: id={id} is not a Global Memory entry"));
    }
    bundle.is_global = false;
    bundle.updated_at = agentmux_common::time::now_ms();
    state.id_store.bundle_upsert(&bundle).map_err(|e| format!("globalmemory.remove: {e}"))?;
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: "memories:changed".to_string(),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });
    Ok(json!({ "ok": true }))
}

/// A version's metadata as JSON — `id`/`content_hash`/`parent_version_id`/
/// `source`/`source_detail`/`written_by`/`created_at`, no `name`/
/// `instructions`. Takes the five common-field accessors as plain values
/// rather than borrowing a concrete struct so both `BundleVersionSummary`
/// (the `history` list shape) and `BundleVersion` (the full-content shape
/// `revert`'s freshly-inserted row comes back as) can share this one
/// formatter without either type needing to convert into the other.
fn bundle_version_meta_json(
    id: &str,
    content_hash: &str,
    parent_version_id: &Option<String>,
    source: &str,
    source_detail: &str,
    written_by: &str,
    written_by_uid: &str,
    created_at: i64,
) -> serde_json::Value {
    json!({
        "id": id,
        "content_hash": content_hash,
        "parent_version_id": parent_version_id,
        "source": source,
        "source_detail": source_detail,
        "written_by": written_by,
        // Identity M4c-2d (spec §6.5.9): the writer's UID beside its name;
        // empty when the write was Unattributed.
        "written_by_uid": written_by_uid,
        "created_at": created_at,
    })
}

/// Loads `id`'s bundle and refuses it unless it is (a) not the blank
/// singleton, (b) not a system-tier entry, and (c) currently a Global Memory
/// entry (`is_global`) — the exact guard `global_memory_write_impl`/
/// `global_memory_remove_impl` already apply, reused here rather than
/// re-derived so `history`/`diff`/`revert` can never diverge from it. Bundle
/// ids for EVERY bundle in the system (not just global ones) are enumerable
/// via `PresetList` (reagent P0/P1, PR #3237) — without this, an agent could
/// point any of these three new routes at another agent's private preset, or
/// at AgentMux's own system-tier entries, and read/restore content that was
/// never meant to be reachable through this API.
fn load_ordinary_global_bundle(state: &AppState, id: &str, verb: &str) -> Result<Bundle, String> {
    let bundle = state.id_store.bundle_get(id)
        .map_err(|e| format!("globalmemory.{verb}: {e}"))?
        .ok_or_else(|| format!("globalmemory.{verb}: not found id={id}"))?;
    if bundle.is_system {
        return Err(format!(
            "globalmemory.{verb}: cannot {verb} a system Global Memory entry through this API"
        ));
    }
    if bundle.is_blank || bundle.id == "blank" {
        return Err(format!("globalmemory.{verb}: cannot {verb} the blank Bundle singleton"));
    }
    if !bundle.is_global {
        return Err(format!("globalmemory.{verb}: id={id} is not a Global Memory entry"));
    }
    Ok(bundle)
}

/// Version history for one Global Memory entry, newest first — metadata
/// only, no `name`/`instructions` (mirrors `memory_history_impl`'s summary
/// shape and `global_memory_list_impl`'s "no content" posture). Refuses a
/// system-tier, blank, or non-global id via `load_ordinary_global_bundle` —
/// structurally the same exclusion `list`/`read`/`remove` already apply, so
/// there is no path here that can ever surface a system-tier row's history.
/// Backs the `GlobalMemoryHistory` MCP tool.
pub(crate) fn global_memory_history_impl(state: &AppState, id: &str) -> Result<serde_json::Value, String> {
    load_ordinary_global_bundle(state, id, "history")?;
    let versions: Vec<_> = state.id_store.bundle_version_list(id)
        .map_err(|e| format!("globalmemory.history: store: {e}"))?
        .iter()
        .map(|v| bundle_version_meta_json(
            &v.id, &v.content_hash, &v.parent_version_id, &v.source, &v.source_detail, &v.written_by,
            &v.written_by_uid, v.created_at,
        ))
        .collect();
    Ok(json!({ "versions": versions }))
}

/// A line-based diff between two recorded versions of one Global Memory
/// entry (mirrors `memory_diff_impl`). Both versions must belong to `id` —
/// reagent P1 in `memory_diff_impl` fixed the identical "diff across two
/// unrelated owners" gap for native memory; the same check applies here
/// since a `version_id` alone does not otherwise prove which bundle (and
/// therefore whether a system-tier one) it came from. A name change is
/// surfaced as its own two-line entry ahead of the instructions diff, since
/// `content_hash` is computed over `name` + `instructions` together
/// (`bundle_versions.rs`'s own `content_hash` doc comment) — a rename with
/// unchanged body would otherwise produce a "no differences" instructions
/// diff that silently hid the one thing that actually changed. Backs the
/// `GlobalMemoryDiff` MCP tool.
pub(crate) fn global_memory_diff_impl(
    state: &AppState,
    id: &str,
    from_version_id: &str,
    to_version_id: &str,
) -> Result<serde_json::Value, String> {
    load_ordinary_global_bundle(state, id, "diff")?;
    let from = state.id_store.bundle_version_get(from_version_id)
        .map_err(|e| format!("globalmemory.diff: store: {e}"))?
        .ok_or_else(|| format!("globalmemory.diff: version {from_version_id} not found"))?;
    let to = state.id_store.bundle_version_get(to_version_id)
        .map_err(|e| format!("globalmemory.diff: store: {e}"))?
        .ok_or_else(|| format!("globalmemory.diff: version {to_version_id} not found"))?;
    if from.bundle_id != id || to.bundle_id != id {
        return Err(format!("globalmemory.diff: one or both versions do not belong to {id}"));
    }
    Ok(json!({ "diff": bundle_version_diff(&from, &to) }))
}

/// The diff text `global_memory_diff_impl` returns — a `- name:`/`+ name:`
/// pair when the name changed (see that function's doc comment for why),
/// then `line_diff` over the instructions. Shared with the Armory UI's
/// `globalmemory:diff` WebSocket command (`agent_handlers/bundle.rs`) so the
/// MCP tool and the UI can never render the same pair of versions two ways.
pub(crate) fn bundle_version_diff(
    from: &crate::backend::storage::BundleVersion,
    to: &crate::backend::storage::BundleVersion,
) -> String {
    let mut diff = String::new();
    if from.name != to.name {
        diff.push_str(&format!("- name: {}\n+ name: {}\n", from.name, to.name));
    }
    diff.push_str(&crate::server::native_memory_handlers::line_diff(&from.instructions, &to.instructions));
    diff
}

/// Restores a Global Memory entry's live `name`/`instructions` to a prior
/// recorded version — same "revert never rewrites history, it records a new
/// version" semantics as `memory_revert_impl` (`source: "revert"`, chained
/// onto whatever the current latest version is). Reuses
/// `Store::bundle_upsert_with_version` — the SAME primitive
/// `global_memory_write_impl` already uses — so the live row and its new
/// version are written atomically, in one transaction, exactly as that
/// method's own doc comment requires (codex P2, PR #3237). `agent_id` is the
/// TRUSTED caller identity (an agent's real `AGENTMUX_AGENT_ID`, resolved by
/// `agentmux-mcp` from its own env, never caller-suppliable) recorded as
/// `written_by` — never inferred from anything in the request body.
/// `written_by_uid` is recorded beside it: the request's `Caller` UID
/// (identity M4c-1), `""` when Unattributed. Backs the `GlobalMemoryRevert`
/// MCP tool.
pub(crate) fn global_memory_revert_impl(
    state: &AppState,
    agent_id: &str,
    written_by_uid: &str,
    id: &str,
    version_id: &str,
) -> Result<serde_json::Value, String> {
    let mut bundle = load_ordinary_global_bundle(state, id, "revert")?;
    let target = state.id_store.bundle_version_get(version_id)
        .map_err(|e| format!("globalmemory.revert: store: {e}"))?
        .ok_or_else(|| format!("globalmemory.revert: version {version_id} not found"))?;
    if target.bundle_id != id {
        return Err(format!("globalmemory.revert: version {version_id} does not belong to {id}"));
    }

    bundle.name = target.name.clone();
    bundle.instructions = target.instructions.clone();
    bundle.updated_at = agentmux_common::time::now_ms();

    let detail = json!({ "reverted_to": version_id }).to_string();
    let new_version = state.id_store
        .bundle_upsert_with_version(&bundle, agent_id, written_by_uid, "revert", &detail)
        .map_err(|e| format!("globalmemory.revert: {e}"))?
        // `bundle.is_global` is guaranteed true by `load_ordinary_global_bundle`
        // above, and `bundle_upsert_with_version` only ever returns `None` when
        // `is_global` is false — so this branch is unreachable in practice, but
        // failing loudly beats silently reporting success with no new version.
        .ok_or_else(|| "globalmemory.revert: no version was recorded (unexpected: entry is not global)".to_string())?;

    state.broker.publish(crate::backend::mps::MuxEvent {
        event: "memories:changed".to_string(),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });
    tracing::info!(agent_id, bundle_id = %id, version_id, "globalmemory.revert");
    Ok(json!({ "version": bundle_version_meta_json(
        &new_version.id,
        &new_version.content_hash,
        &new_version.parent_version_id,
        &new_version.source,
        &new_version.source_detail,
        &new_version.written_by,
        &new_version.written_by_uid,
        new_version.created_at,
    ) }))
}
