// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bundle.import_for_agent`: apply an ABF to an existing agent definition,
//! serialised per agent by `bundle_import_for_agent_lock`.

use super::*;

#[derive(serde::Deserialize, Default)]
pub(super) struct ImportForAgentReq {
    pub(crate) agent_id: String,
    #[serde(default)]
    pub(crate) file_path: Option<String>,
    #[serde(default)]
    pub(crate) zip_base64: Option<String>,
    #[serde(default)]
    pub(crate) files: Option<Vec<FileEntry>>,
}

/// `bundle.import_for_agent` — ABF v0.2 §2.3. The only import path that
/// processes a `components.memory` key; the generic `bundle.import`
/// explicitly skips it (see `parse_bundle_import_with_budget`'s own
/// components.memory guard). Unlike `bundle.import`/`bundle.import.commit`,
/// this is a single-step import — no preview phase — since a
/// memory-bearing import has its own unconditional safety net instead:
/// the target agent must have ZERO existing `db_agent_native_memory` rows,
/// or the whole import is rejected before any write happens. Simpler than
/// skip/rename/replace conflict resolution, and sufficient for v0.2 (see
/// the spec's revision note for why merge semantics are an explicit
/// follow-up, not designed here). Extracted from the RPC closure into a
/// directly-callable, directly-testable function, matching
/// `bundle_import_preview_impl`'s pattern.
pub(super) async fn bundle_import_for_agent_impl(
    id_store: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    req: ImportForAgentReq,
) -> Result<serde_json::Value, String> {
    use crate::backend::bundle_import as bi;

    let agent = mstore
        .agent_def_get(&req.agent_id)
        .map_err(|e| format!("bundle.import_for_agent: {e}"))?
        .ok_or_else(|| format!("bundle.import_for_agent: no agent with id {}", req.agent_id))?;

    // reagent P2, PR #2527: serialize the whole "check zero rows, then
    // write" sequence per agent — without this, two concurrent
    // bundle.import_for_agent calls for the same agent can both pass the
    // zero-rows check before either writes, both proceeding and defeating
    // the "can't destroy existing memory" invariant. Held until the
    // function returns (guard drops at every exit path), mirroring
    // agent_open.rs's AGENT_OPEN_LOCKS precedent — same same-process-only
    // scope note applies (can't see a genuinely different AgentMux
    // instance/channel racing the same agent_id).
    let import_lock = bundle_import_for_agent_lock(&agent.id);
    let _import_guard = import_lock.lock().await;

    // reagent P0, PR #2527: the "zero existing memory" guard below only
    // consults db_agent_native_memory (the mirror) — a live file that was
    // written autonomously but never viewed through the Stash Bundle tab
    // is NOT in the mirror yet, so the check would pass and the write
    // loop further down would silently overwrite it via fs::rename. Same
    // class of bug bundle.export_for_agent already had to guard against
    // (see refresh_memory_mirror_from_live_fs's own doc comment) — the
    // fix is the same: refresh from the live FS before trusting the
    // mirror's row count. A missing/unresolvable working directory is not
    // an error here (nothing to refresh from); the emptiness check below
    // still runs against whatever the mirror already has.
    // Only a verified directory is refreshed from: an unverified guess can be
    // another agent's folder, and refreshing from it would write that
    // agent's files into this one's mirror (SPEC_MEMORY_FOLLOWS_THE_AGENT
    // §2.1.2).
    let resolved = crate::server::native_memory_handlers::resolve_memory_dir_by_id(mstore, &agent);
    let verified = resolved.as_ref().is_some_and(|r| {
        r.provenance != crate::server::native_memory_handlers::MemoryDirProvenance::Unverified
    });
    let memory_dir = resolved.map(|r| r.path);
    let mut warnings: Vec<String> = Vec::new();
    if let Some(dir) = memory_dir.as_ref().filter(|_| verified) {
        if let Err(e) = crate::server::native_memory_handlers::refresh_memory_mirror_from_live_fs(&agent.id, dir, id_store) {
            warnings.push(format!("native memory refresh failed (checking mirror as-is): {e}"));
        }
    }

    // Reject BEFORE any parsing/decoding work — a target with existing
    // memory is a hard stop, not something worth spending
    // decompression/parse cost on first.
    let existing = id_store
        .agent_native_memory_list_meta(&agent.id)
        .map_err(|e| format!("bundle.import_for_agent: {e}"))?;
    if !existing.is_empty() {
        return Err(format!(
            "bundle.import_for_agent: agent {} already has {} native memory file(s) — import into an agent with no existing memory, or clear it first",
            req.agent_id,
            existing.len()
        ));
    }

    let resolved = resolve_import_input(req.file_path, req.zip_base64, req.files, bi::WarningBudget::unbounded())
        .map_err(|e| format!("bundle.import_for_agent: {e}"))?;
    let parsed = bi::parse_bundle_import(&resolved.files)
        .map_err(|e| format!("bundle.import_for_agent: {e}"))?;

    warnings.extend(resolved.intake_warnings);
    // reagent P1, PR #2527: parse_bundle_import unconditionally warns
    // that components.memory is "present but ignored" (correct for
    // bundle.import/.preview/.commit, which really do ignore it) — but
    // THIS function handles memory itself, further down. Filtering the
    // exact shared-constant string out here (rather than duplicating a
    // literal that could drift) stops every successful memory-bearing
    // import from falsely claiming its memory was ignored.
    warnings.extend(parsed.warnings.iter().filter(|w| w.as_str() != bi::MEMORY_COMPONENT_IGNORED_WARNING).cloned());

    // reagent P2, PR #2527: this RPC exists specifically to transfer
    // memory — if the manifest actually has a memory component but this
    // agent has no resolvable working directory (and therefore no memory
    // dir to write into), failing fast here (before any skill/bundle
    // writes) is correct; silently succeeding with
    // memory_files_written: 0 would drop the one thing the caller asked
    // for. A bundle with NO memory component is unaffected — that's a
    // normal bundle.import_for_agent call that just happens not to need
    // a memory dir.
    let manifest_memory_paths = resolved_memory_paths(&resolved.files);
    if !manifest_memory_paths.is_empty() && memory_dir.is_none() {
        return Err(format!(
            "bundle.import_for_agent: agent {} has no working directory configured; cannot import its memory",
            req.agent_id
        ));
    }
    // Importing memory writes files, so the directory must be verified, not
    // a blank-working-dir guess (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2).
    let memory_dir = if manifest_memory_paths.is_empty() {
        memory_dir
    } else {
        Some(
            crate::server::native_memory_handlers::memory_dir_for_write_by_id(mstore, &agent)
                .map_err(|e| format!("bundle.import_for_agent: {e}"))?,
        )
    };

    // Normal bundle components, exactly as bundle.import creates them —
    // a memory-bearing import still creates a reusable bundle row
    // alongside the agent-scoped memory write below.
    let bundle_id = uuid::Uuid::new_v4().to_string();
    // Mirrors bundle.import's own skill-creation block exactly (global,
    // not bound to any agent; skill_upsert_unique_global is the real API
    // — a genuine name conflict is a per-skill warning, any other error
    // aborts the whole RPC).
    let now = now_ms();
    let mut imported_skill_ids: Vec<String> = Vec::new();
    // reagent P2, PR #2527: mirrors bundle.import's own rollback_skills
    // closure — without it, a later failure (the bundle upsert below)
    // leaves already-created global skill rows orphaned with no way for
    // the caller to know cleanup may be needed.
    let rollback_skills = |ids: &[String]| -> Vec<String> {
        ids.iter()
            .filter_map(|id| mstore.skill_delete(identity_store, id).err().map(|e| format!("{id}: {e}")))
            .collect()
    };
    for skill in &parsed.skills {
        let row = crate::backend::storage::Skill {
            id: uuid::Uuid::new_v4().to_string(),
            name: skill.slug.clone(),
            trigger: skill.slug.clone(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: skill.description.clone(),
            content: skill.content.clone(),
            is_global: true,
            created_at: now,
            updated_at: now,
        };
        match identity_store.skill_upsert_unique_global(&row) {
            Ok(()) => imported_skill_ids.push(row.id),
            Err(e) if e.to_string().contains("already exists") => {
                warnings.push(format!("skill \"{}\" already exists; skipped", skill.slug));
            }
            Err(e) => {
                let rollback_errors = rollback_skills(&imported_skill_ids);
                let mut msg = format!("bundle.import_for_agent: failed to create skill \"{}\": {e}", skill.slug);
                if !rollback_errors.is_empty() {
                    msg.push_str(&format!(
                        "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                        rollback_errors.len(),
                        rollback_errors.join("; ")
                    ));
                }
                return Err(msg);
            }
        }
    }
    let memory = crate::backend::storage::store::Bundle {
        id: bundle_id.clone(),
        name: parsed.name,
        description: parsed.description,
        is_blank: false,
        is_global: false,
        provider: parsed.provider,
        model: parsed.model,
        instructions: parsed.instructions,
        instructions_by_provider: serde_json::to_string(&parsed.instructions_by_provider)
            .unwrap_or_else(|_| "{}".to_string()),
        context_files: serde_json::to_string(
            &parsed.context_files.iter().map(|cf| json!({"path": cf.path, "content": cf.content})).collect::<Vec<_>>(),
        ).unwrap_or_else(|_| "[]".to_string()),
        mcp_servers: serde_json::to_string(
            &parsed.mcp_servers.iter().map(|m| m.config.clone()).collect::<Vec<_>>(),
        ).unwrap_or_else(|_| "[]".to_string()),
        skills: serde_json::to_string(&imported_skill_ids).unwrap_or_else(|_| "[]".to_string()),
        sort_order: 0,
        created_at: now,
        updated_at: now,
        is_system: false,
    };
    if let Err(e) = id_store.bundle_upsert(&memory) {
        let rollback_errors = rollback_skills(&imported_skill_ids);
        let mut msg = format!("bundle.import_for_agent: {e}");
        if !rollback_errors.is_empty() {
            msg.push_str(&format!(
                "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                rollback_errors.len(),
                rollback_errors.join("; ")
            ));
        }
        return Err(msg);
    }

    // The bundle row now exists, so its components can be bound. Ref tables
    // are authoritative — without this the imported bundle would export empty
    // and its MCP servers would never reach a spawned agent.
    warnings.extend(bind_imported_components(
        mstore,
        id_store,
        identity_store,
        &bundle_id,
        &imported_skill_ids,
        &parsed.mcp_servers.iter().map(|m| m.config.clone()).collect::<Vec<_>>(),
    ));

    // Bundle files: write through the SAME dual-write path
    // agent:memory:write_file uses (live FS via memory_dir_for_cwd, then
    // the mirror) — writing only to db_agent_native_memory would leave
    // this content visible in Stash while invisible to the actual
    // running agent (see the spec's revision note).
    let mut memory_files_written = 0usize;
    if let Some(memory_dir) = &memory_dir {
        if let Err(e) = std::fs::create_dir_all(memory_dir) {
            warnings.push(format!("native memory: mkdir failed: {e}"));
        }
    }
    if let Some(memory_dir) = memory_dir {
        for path in manifest_memory_paths {
            // reagent P1, PR #2527 (third round): every skip in this loop
            // must warn — this RPC's whole reason to exist is transferring
            // memory, and a silent drop here (bundle_id/skills already
            // created, memory_files_written silently undercounting)
            // directly contradicts that. Matches the sibling
            // invalid-filename branch just below, which already warned.
            let Some(filename) = path.strip_prefix("memory/") else {
                warnings.push(format!("{path}: components.memory path is not under memory/; skipped"));
                continue;
            };
            if crate::server::native_memory_handlers::validate_memory_filename(filename).is_err() {
                warnings.push(format!("memory/{filename}: not a valid memory filename; skipped"));
                continue;
            }
            let Some(content) = resolved.files.iter().find(|f| f.path == path).map(|f| f.content.clone()) else {
                warnings.push(format!("{path}: referenced in components.memory but not found among the bundle's files; skipped"));
                continue;
            };
            let dest = memory_dir.join(filename);
            let tmp = memory_dir.join(format!(".{filename}.{}.tmp", uuid::Uuid::new_v4()));
            let write_result = std::fs::write(&tmp, &content)
                .and_then(|_| std::fs::rename(&tmp, &dest));
            if let Err(e) = write_result {
                let _ = std::fs::remove_file(&tmp);
                warnings.push(format!("memory/{filename}: write failed: {e}"));
                continue;
            }
            let dest_meta = std::fs::metadata(&dest).ok();
            let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(content.len() as i64);
            let mtime_ms = dest_meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let metadata_type = crate::server::native_memory_handlers::parse_memory_frontmatter_type(&content);
            if let Err(e) = id_store.agent_native_memory_upsert(
                &agent.id,
                filename,
                &content,
                metadata_type.as_deref(),
                &dest.to_string_lossy(),
                size_bytes,
                mtime_ms,
            ) {
                warnings.push(format!("memory/{filename}: mirror upsert failed: {e}"));
            } else {
                memory_files_written += 1;
            }
        }
    }

    Ok(json!({
        "bundle_id": bundle_id,
        "memory_files_written": memory_files_written,
        "skipped_skills": parsed.skipped_skills,
        "warnings": warnings,
    }))
}

pub(super) fn register_bundle_import_for_agent(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let mstore = state.mstore.clone();
    engine.register_handler(
        COMMAND_BUNDLE_IMPORT_FOR_AGENT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let mstore = mstore.clone();
            Box::pin(async move {
                let req: ImportForAgentReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.import_for_agent: {e}"))?;
                bundle_import_for_agent_impl(&id_store, &identity_store, &mstore, req).await.map(Some)
            })
        }),
    );
}

/// Every `memory/*` path listed under `components.memory` in a parsed
/// import's `armory.json` — mirrors how `components.instructions`/
/// `components.skills` are read elsewhere in this file, kept local to
/// `bundle.import_for_agent` since no other handler needs it.
pub(super) fn resolved_memory_paths(files: &[crate::backend::bundle_import::BundleImportFile]) -> Vec<String> {
    let Some(manifest_file) = files.iter().find(|f| f.path == "armory.json") else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest_file.content) else {
        return Vec::new();
    };
    manifest["components"]["memory"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}
