use super::*;

/// Per-agent async lock serializing `bundle.import_for_agent`'s "check
/// zero existing memory rows, then write" sequence — ABF v0.2 §2.3,
/// reagent P2 on PR #2527. Mirrors `agent_open.rs`'s `AGENT_OPEN_LOCKS`
/// precedent exactly, including its scope note: this only serializes
/// calls handled by THIS process, not a genuinely different AgentMux
/// instance/channel racing the same agent_id.
static BUNDLE_IMPORT_FOR_AGENT_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn bundle_import_for_agent_lock(agent_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = BUNDLE_IMPORT_FOR_AGENT_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks
        .entry(agent_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_bundle_list(engine, state);
    register_bundle_get(engine, state);
    register_bundle_upsert(engine, state);
    register_bundle_delete(engine, state);
    register_bundle_self_get(engine, state);
    register_bundle_export(engine, state);
    register_bundle_import(engine, state);
    register_bundle_import_preview(engine, state);
    register_bundle_import_commit(engine, state);
    register_bundle_export_for_agent(engine, state);
    register_bundle_import_for_agent(engine, state);
}

/// Raw `[{path, content}]` entry shape, shared by `bundle.import`'s `files`
/// input and the Phase 3 `bundle.import.preview`/`.commit` handlers.
#[derive(serde::Deserialize, Default)]
struct FileEntry {
    path: String,
    content: String,
}

/// Normalize a `bundle.upsert` request body into the shape the `Memory` struct
/// deserializes from, so the App API accepts the request exactly as documented
/// in the spec:
///   - `id` may be omitted to create (the struct has no serde default), so an
///     absent or null `id` is filled with an empty string (the handler then
///     mints a UUID).
///   - `context_files` / `mcp_servers` / `skills` are JSON-encoded array strings
///     on the struct, but the spec shows them as JSON arrays. Array values are
///     re-encoded to their JSON string form; values already given as strings
///     pass through untouched.
pub(super) fn normalize_bundle_upsert_input(mut data: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = data {
        match map.get("id") {
            Some(serde_json::Value::String(_)) => {}
            _ => {
                map.insert("id".to_string(), serde_json::Value::String(String::new()));
            }
        }
        for key in ["context_files", "mcp_servers", "skills"] {
            if let Some(v) = map.get(key) {
                if v.is_array() {
                    let encoded =
                        serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string());
                    map.insert(key.to_string(), serde_json::Value::String(encoded));
                }
            }
        }
    }
    data
}

// Each handler is registered under BOTH the new `bundle.*` command and the
// deprecated `preset.*` alias (one-release compat window, spec Phase 2). The
// alias forwards to the same logic; remove it in Phase 4. The two
// `register_handler` calls per fn pass explicit command constants (not a loop
// variable) so the rpc-contract extractor can resolve every registered name.

fn register_bundle_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |_data, _ctx| {
            let state = state.clone();
            Box::pin(async move { Ok(Some(bundle_list_impl(&state).await?)) })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_LIST, make(state.clone()));
    engine.register_handler(COMMAND_PRESET_LIST, make(state.clone()));
}

fn register_bundle_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |data, _ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                struct Req {
                    #[serde(default)] id: String,
                    #[serde(default)] name: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.get: {e}"))?;
                Ok(Some(bundle_get_impl(&state, &req.id, &req.name).await?))
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_GET, make(state.clone()));
    engine.register_handler(COMMAND_PRESET_GET, make(state.clone()));
}

fn register_bundle_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: &AppState| -> crate::backend::rpc::engine::CommandHandler {
        let id_store = state.id_store.clone();
        let broker = state.broker.clone();
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Memory =
                    serde_json::from_value(normalize_bundle_upsert_input(data))
                        .map_err(|e| format!("bundle.upsert: {e}"))?;

                // S4: guard on the target id, not the caller-supplied is_blank flag
                // (which defaults false and can be omitted to bypass is_blank check).
                if memory.id == "blank" || memory.id.starts_with("seed-") || memory.is_blank {
                    return Err("FORBIDDEN: cannot mutate a protected bundle".to_string());
                }
                // Guard existing global bundles: an agent must not be able to demote or
                // corrupt a shared global brain bundle it doesn't own by supplying its id.
                if !memory.id.is_empty() {
                    if let Some(existing) = id_store.bundle_memory_get(&memory.id)
                        .map_err(|e| format!("bundle.upsert: {e}"))?
                    {
                        if existing.is_global {
                            return Err("FORBIDDEN: cannot mutate a global bundle".to_string());
                        }
                    }
                }
                if memory.id.is_empty() {
                    memory.id = uuid::Uuid::new_v4().to_string();
                }
                // S4a: strip caller-supplied escalation fields.
                memory.is_global = false;
                memory.sort_order = 0;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if memory.created_at == 0 { memory.created_at = now; }
                memory.updated_at = now;

                id_store.bundle_memory_upsert(&memory)
                    .map_err(|e| format!("bundle.upsert: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(serde_json::to_value(&memory).map_err(|e| e.to_string())?))
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_UPSERT, make(state));
    engine.register_handler(COMMAND_PRESET_UPSERT, make(state));
}

fn register_bundle_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: &AppState| -> crate::backend::rpc::engine::CommandHandler {
        let id_store = state.id_store.clone();
        let broker = state.broker.clone();
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.delete: {e}"))?;

                if req.id == "blank" {
                    return Err("FORBIDDEN: cannot delete a seeded bundle".to_string());
                }

                match id_store.bundle_memory_delete(&req.id) {
                    Ok(deleted) => {
                        if deleted {
                            broker.publish(crate::backend::wps::WaveEvent {
                                event: "memories:changed".to_string(),
                                scopes: vec![], sender: String::new(), persist: 0, data: None,
                            });
                        }
                        Ok(Some(json!({ "deleted": deleted })))
                    }
                    Err(crate::backend::storage::error::StoreError::Other(msg))
                        if msg.contains("seed") || msg.contains("seeded") =>
                    {
                        Err(format!("FORBIDDEN: cannot delete a seeded bundle"))
                    }
                    Err(e) => Err(format!("bundle.delete: {e}")),
                }
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_DELETE, make(state));
    engine.register_handler(COMMAND_PRESET_DELETE, make(state));
}

fn register_bundle_self_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.self.get: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                Ok(Some(bundle_self_get_impl(&state, &req.agent_id).await?))
            })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_SELF_GET, make(state.clone()));
    engine.register_handler(COMMAND_PRESET_SELF_GET, make(state.clone()));
}

/// `bundle.export` — Armory Bundle Format (ABF) exporter, Phase 1 of
/// docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md /
/// https://docs.agentmux.ai/abf/. Window-scoped (no `check_s1`) like the
/// rest of `bundle.*` — bundles aren't agent-specific. Reads the bundle
/// from `id_store` and resolves its referenced skill ids against `wstore`
/// (skills live in a different Store instance than bundles — see
/// `Store::skill_get` callers elsewhere in this codebase), then hands both
/// to the pure `bundle_export::export_bundle`. `format: "zip"` returns a
/// base64-encoded archive; anything else (including omitted) returns the
/// raw file list for the caller to write out itself.
fn register_bundle_export(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                struct Req {
                    id: String,
                    #[serde(default)]
                    format: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export: {e}"))?;

                let bundle = id_store
                    .bundle_memory_get(&req.id)
                    .map_err(|e| format!("bundle.export: {e}"))?
                    .ok_or_else(|| format!("bundle.export: no bundle with id {}", req.id))?;

                // Same silent-data-loss pattern already fixed for
                // context_files/mcp_servers in bundle_export.rs -- a
                // malformed bundle.skills value must warn, not just
                // vanish, but a genuinely blank one is not an error
                // (reagent P1, PR #2333). Shares the same helper so all
                // three fields behave identically.
                let mut handler_warnings: Vec<String> = Vec::new();
                let skill_ids: Vec<String> = crate::backend::bundle_export::parse_json_field_or_warn(
                    &bundle.skills,
                    "skills",
                    &mut handler_warnings,
                );
                // Distinguish a genuine DB error from a legitimately deleted
                // skill id: `.ok().flatten()` previously collapsed both to
                // "absent", so a locked/damaged store silently reported a
                // successful export missing content instead of failing loudly
                // -- unsafe for the advertised backup use case (Codex P2, PR
                // #2325). A lookup error now fails the whole export; a
                // missing row (Ok(None), i.e. actually deleted) is skipped
                // and reported back in `missing_skill_ids`.
                let mut skills: Vec<crate::backend::storage::Skill> = Vec::new();
                let mut missing_skill_ids: Vec<String> = Vec::new();
                for id in &skill_ids {
                    match wstore.skill_get(id) {
                        Ok(Some(skill)) => skills.push(skill),
                        Ok(None) => missing_skill_ids.push(id.clone()),
                        Err(e) => {
                            return Err(format!("bundle.export: failed to look up skill {id}: {e}"));
                        }
                    }
                }

                let export = crate::backend::bundle_export::export_bundle(&bundle, &skills);

                let mut all_warnings = export.warnings.clone();
                all_warnings.append(&mut handler_warnings);

                if req.format == "zip" {
                    let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
                        .map_err(|e| format!("bundle.export: {e}"))?;
                    use base64::Engine as _;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
                    return Ok(Some(json!({
                        "root_slug": export.root_slug,
                        "skipped_skills": export.skipped_skills,
                        "warnings": all_warnings,
                        "missing_skill_ids": missing_skill_ids,
                        "zip_base64": encoded,
                    })));
                }

                let mut result = serde_json::to_value(&export).map_err(|e| e.to_string())?;
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("missing_skill_ids".to_string(), json!(missing_skill_ids));
                    obj.insert("warnings".to_string(), json!(all_warnings));
                }
                Ok(Some(result))
            })
        }),
    );
}

/// Splice native-memory files into an already-built bundle export's
/// `armory.json` manifest and files list, adding `components.memory` (ABF
/// v0.2 §2.3). Kept OUTSIDE `bundle_export.rs` deliberately — that
/// module's `export_bundle()` is scoped to a bundle's own components
/// (instructions/skills/MCP/accounts) with no concept of "agent" or
/// native memory at all; memory is agent-scoped, not bundle-scoped, so
/// splicing it in here (the RPC-handler layer, which already resolves
/// other agent-scoped data like skill rows) keeps that module's
/// documented scope intact rather than growing it a fifth, unrelated
/// component category. A no-op when `memory_files` is empty — matches the
/// existing omit-empty-components convention used elsewhere in the
/// manifest.
fn splice_memory_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    memory_files: &[(String, String)],
) -> Result<(), String> {
    if memory_files.is_empty() {
        return Ok(());
    }
    let manifest_idx = export
        .files
        .iter()
        .position(|f| f.path == "armory.json")
        .ok_or_else(|| "bundle export is missing armory.json".to_string())?;
    let mut manifest: serde_json::Value = serde_json::from_str(&export.files[manifest_idx].content)
        .map_err(|e| format!("armory.json: {e}"))?;
    let mut manifest_memory: Vec<String> = Vec::new();
    for (filename, content) in memory_files {
        // Filenames here always came from db_agent_native_memory, which
        // only ever accepts app-validated names (validate_filename in
        // native_memory_handlers.rs: alphanumeric + "-_", ends ".md", no
        // separators) — unlike instructions_by_provider's keys (§2.2),
        // there's no untrusted-manifest path that could smuggle an unsafe
        // segment in here, so no additional sanitization is needed.
        let out_path = format!("memory/{filename}");
        export.files.push(crate::backend::bundle_export::BundleExportFile {
            path: out_path.clone(),
            content: content.clone(),
        });
        manifest_memory.push(out_path);
    }
    manifest["components"]["memory"] = json!(manifest_memory);
    export.files[manifest_idx].content =
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    Ok(())
}

/// `bundle.export_for_agent` — ABF v0.2 §2.3. The only export path that
/// carries native memory: pulls a bundle's normal components (identical to
/// `bundle.export`) plus a snapshot of ONE agent's
/// `db_agent_native_memory` rows, refreshed from the live filesystem
/// first (see [`native_memory_handlers::refresh_memory_mirror_from_live_fs`]'s
/// doc comment for why that refresh is required, not optional). `bundle.
/// export` stays agent-less and memory-less — bundles are reusable across
/// many agents by design, so memory (inherently per-agent) needs its own,
/// explicitly-scoped entry point rather than an implicit "whichever agent
/// happens to be attached" guess.
#[derive(serde::Deserialize, Default)]
struct ExportForAgentReq {
    bundle_id: String,
    agent_id: String,
    #[serde(default)]
    format: String,
}

/// `bundle.export_for_agent` — ABF v0.2 §2.3. The only export path that
/// carries native memory: pulls a bundle's normal components (identical to
/// `bundle.export`) plus a snapshot of ONE agent's
/// `db_agent_native_memory` rows, refreshed from the live filesystem
/// first (see [`native_memory_handlers::refresh_memory_mirror_from_live_fs`]'s
/// doc comment for why that refresh is required, not optional). `bundle.
/// export` stays agent-less and memory-less — bundles are reusable across
/// many agents by design, so memory (inherently per-agent) needs its own,
/// explicitly-scoped entry point rather than an implicit "whichever agent
/// happens to be attached" guess. Extracted from the RPC closure into a
/// directly-callable, directly-testable function, matching
/// `bundle_import_preview_impl`'s pattern.
async fn bundle_export_for_agent_impl(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    req: ExportForAgentReq,
) -> Result<serde_json::Value, String> {
    let bundle = id_store
        .bundle_memory_get(&req.bundle_id)
        .map_err(|e| format!("bundle.export_for_agent: {e}"))?
        .ok_or_else(|| format!("bundle.export_for_agent: no bundle with id {}", req.bundle_id))?;
    let agent = wstore
        .agent_def_get(&req.agent_id)
        .map_err(|e| format!("bundle.export_for_agent: {e}"))?
        .ok_or_else(|| format!("bundle.export_for_agent: no agent with id {}", req.agent_id))?;

    let mut handler_warnings: Vec<String> = Vec::new();
    let skill_ids: Vec<String> = crate::backend::bundle_export::parse_json_field_or_warn(
        &bundle.skills,
        "skills",
        &mut handler_warnings,
    );
    let mut skills: Vec<crate::backend::storage::Skill> = Vec::new();
    let mut missing_skill_ids: Vec<String> = Vec::new();
    for id in &skill_ids {
        match wstore.skill_get(id) {
            Ok(Some(skill)) => skills.push(skill),
            Ok(None) => missing_skill_ids.push(id.clone()),
            Err(e) => {
                return Err(format!("bundle.export_for_agent: failed to look up skill {id}: {e}"));
            }
        }
    }

    let mut export = crate::backend::bundle_export::export_bundle(&bundle, &skills);

    // Refresh the mirror from the live FS, then read every mirrored
    // file's content — this agent's memory, freshest as of right now,
    // not as of whenever its Stash Memory tab was last opened.
    let mut memory_files: Vec<(String, String)> = Vec::new();
    if let Some(memory_dir) =
        crate::server::native_memory_handlers::memory_dir_for_agent_by_id(wstore, &agent)
    {
        match crate::server::native_memory_handlers::refresh_memory_mirror_from_live_fs(
            &agent.id,
            &memory_dir,
            id_store,
        ) {
            Ok(truncated) => {
                // reagent P2, PR #2527: surface truncation instead of
                // silently exporting/importing a partial file.
                for filename in truncated {
                    handler_warnings.push(format!(
                        "{filename}: exceeds the native-memory size limit and was truncated; the exported copy is incomplete"
                    ));
                }
            }
            Err(e) => handler_warnings.push(format!("native memory refresh failed (exporting mirror as-is): {e}")),
        }
    }
    match id_store.agent_native_memory_list_meta(&agent.id) {
        Ok(rows) => {
            for row in rows {
                match id_store.agent_native_memory_read(&agent.id, &row.filename) {
                    Ok(Some(content)) => memory_files.push((row.filename, content)),
                    Ok(None) => {} // deleted between list_meta and read — skip, not an error
                    Err(e) => handler_warnings.push(format!(
                        "native memory: failed to read {}: {e}",
                        row.filename
                    )),
                }
            }
        }
        Err(e) => handler_warnings.push(format!("native memory: failed to list: {e}")),
    }
    splice_memory_component(&mut export, &memory_files)
        .map_err(|e| format!("bundle.export_for_agent: {e}"))?;

    let mut all_warnings = export.warnings.clone();
    all_warnings.append(&mut handler_warnings);

    if req.format == "zip" {
        let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
            .map_err(|e| format!("bundle.export_for_agent: {e}"))?;
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
        return Ok(json!({
            "root_slug": export.root_slug,
            "skipped_skills": export.skipped_skills,
            "warnings": all_warnings,
            "missing_skill_ids": missing_skill_ids,
            "zip_base64": encoded,
        }));
    }

    let mut result = serde_json::to_value(&export).map_err(|e| e.to_string())?;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("missing_skill_ids".to_string(), json!(missing_skill_ids));
        obj.insert("warnings".to_string(), json!(all_warnings));
    }
    Ok(result)
}

fn register_bundle_export_for_agent(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT_FOR_AGENT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                let req: ExportForAgentReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export_for_agent: {e}"))?;
                bundle_export_for_agent_impl(&id_store, &wstore, req).await.map(Some)
            })
        }),
    );
}

#[derive(serde::Deserialize, Default)]
struct ImportForAgentReq {
    agent_id: String,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    zip_base64: Option<String>,
    #[serde(default)]
    files: Option<Vec<FileEntry>>,
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
async fn bundle_import_for_agent_impl(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    req: ImportForAgentReq,
) -> Result<serde_json::Value, String> {
    use crate::backend::bundle_import as bi;

    let agent = wstore
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
    // written autonomously but never viewed through the Stash Memory tab
    // is NOT in the mirror yet, so the check would pass and the write
    // loop further down would silently overwrite it via fs::rename. Same
    // class of bug bundle.export_for_agent already had to guard against
    // (see refresh_memory_mirror_from_live_fs's own doc comment) — the
    // fix is the same: refresh from the live FS before trusting the
    // mirror's row count. A missing/unresolvable working directory is not
    // an error here (nothing to refresh from); the emptiness check below
    // still runs against whatever the mirror already has.
    let memory_dir = crate::server::native_memory_handlers::memory_dir_for_agent_by_id(wstore, &agent);
    let mut warnings: Vec<String> = Vec::new();
    if let Some(dir) = &memory_dir {
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
            .filter_map(|id| wstore.skill_delete(id).err().map(|e| format!("{id}: {e}")))
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
        match wstore.skill_upsert_unique_global(&row) {
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
    let memory = crate::backend::storage::store::Memory {
        id: bundle_id.clone(),
        name: parsed.name,
        description: parsed.description,
        is_blank: false,
        is_global: false,
        provider: String::new(),
        model: String::new(),
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
    };
    if let Err(e) = id_store.bundle_memory_upsert(&memory) {
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

    // Memory files: write through the SAME dual-write path
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
            let Some(filename) = path.strip_prefix("memory/") else { continue };
            if crate::server::native_memory_handlers::validate_memory_filename(filename).is_err() {
                warnings.push(format!("memory/{filename}: not a valid memory filename; skipped"));
                continue;
            }
            let Some(content) = resolved.files.iter().find(|f| f.path == path).map(|f| f.content.clone()) else {
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

fn register_bundle_import_for_agent(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_BUNDLE_IMPORT_FOR_AGENT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                let req: ImportForAgentReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.import_for_agent: {e}"))?;
                bundle_import_for_agent_impl(&id_store, &wstore, req).await.map(Some)
            })
        }),
    );
}

/// Every `memory/*` path listed under `components.memory` in a parsed
/// import's `armory.json` — mirrors how `components.instructions`/
/// `components.skills` are read elsewhere in this file, kept local to
/// `bundle.import_for_agent` since no other handler needs it.
fn resolved_memory_paths(files: &[crate::backend::bundle_import::BundleImportFile]) -> Vec<String> {
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

/// Read-only account-requirement resolution (spec §4.5) — one identity
/// lookup per DISTINCT provider, not per requirement row (codex P1, PR
/// #2379 round 4: `bundle_import.rs` bounds the requirements array length,
/// but even at that bound many rows commonly share a handful of
/// providers). Extracted here (Phase 3 spec §3.1's account-resolution
/// implementation note) so the existing `bundle.import` route and the new
/// `bundle.import.preview`/`.commit` handlers share the exact same
/// resolution logic rather than duplicating it in a second place where it
/// could drift.
/// Full, untruncated values — callers apply the shared bounded-display
/// projection ([`bounded_display`]) to `id`/`provider`/`env` before ever
/// serializing these into an RPC response (Phase 3 spec §3.1, round 11).
struct RequirementResolution {
    id: String,
    provider: String,
    env: String,
    match_count: usize,
}

impl RequirementResolution {
    fn resolved(&self) -> bool {
        self.match_count == 1
    }
}

fn resolve_account_requirements(
    id_store: &crate::backend::storage::store::Store,
    requirements: &[crate::backend::bundle_import::AccountRequirement],
) -> Result<Vec<RequirementResolution>, String> {
    let mut results: Vec<RequirementResolution> = Vec::new();
    let mut match_count_by_provider: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for requirement in requirements {
        let match_count = match match_count_by_provider.get(requirement.provider.as_str()) {
            Some(&n) => n,
            None => {
                // Codex P2, PR #2379 round 2: account CRUD routes through
                // id_store (shared/store.db when available), not wstore
                // (the per-channel database).
                let n = id_store
                    .identity_list(Some(&requirement.provider))
                    .map_err(|e| format!("account lookup failed: {e}"))?
                    .len();
                match_count_by_provider.insert(requirement.provider.as_str(), n);
                n
            }
        };
        results.push(RequirementResolution {
            id: requirement.id.clone(),
            provider: requirement.provider.clone(),
            env: requirement.env.clone(),
            match_count,
        });
    }
    Ok(results)
}

/// Shared bounded-display projection (Phase 3 spec, round 12's governing
/// rule) for every "meant to be short" field both `preview` and `commit`
/// echo back: skill slug/description, requirement `id`/`provider`/`env`,
/// context-file `display_path`, bundle `description`, and commit's
/// `skipped_skills`/`resolved_requirement_ids` entries. `bundle.name` is
/// NOT projected through this — it's bounded at parse time instead
/// (round 13), since it's re-submitted verbatim as `bundle_name`.
fn bounded_display(s: &str) -> String {
    crate::backend::bundle_import::truncate_display(s, crate::backend::bundle_import::MAX_DISPLAY_FIELD_CHARS)
}

/// `bundle.import` — ABF importer, Phase 2 of
/// docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md. Window-
/// scoped like `bundle.export` (bundles aren't agent-specific). Accepts
/// EITHER `zip_base64` (a `.abf` archive, mirroring `bundle.export`'s own
/// `zip_base64` response field) OR `files` (a raw `[{path, content}]`
/// list) — exactly one must be present. Hands the bytes to the pure
/// `bundle_import` module for parsing/validation, then owns every Store
/// side-effect: creates the bundle row, creates one global Skill row per
/// parsed skill, and performs READ-ONLY account-requirement lookups
/// (never creates an account, never writes a secret anywhere — see the
/// spec's §4.5 correction for why this stopped short of "resolving"
/// placeholders in place).
fn register_bundle_import(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_BUNDLE_IMPORT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                struct Req {
                    #[serde(default)]
                    zip_base64: Option<String>,
                    #[serde(default)]
                    files: Option<Vec<FileEntry>>,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.import: {e}"))?;

                let (files, mut warnings): (Vec<crate::backend::bundle_import::BundleImportFile>, Vec<String>) =
                    match (req.zip_base64, req.files) {
                        (Some(_), Some(_)) => {
                            return Err(
                                "bundle.import: exactly one of zip_base64/files must be present, got both"
                                    .to_string(),
                            );
                        }
                        (None, None) => {
                            return Err(
                                "bundle.import: exactly one of zip_base64/files must be present, got neither"
                                    .to_string(),
                            );
                        }
                        (Some(b64), None) => {
                            use base64::Engine as _;
                            let zip_bytes = base64::engine::general_purpose::STANDARD
                                .decode(&b64)
                                .map_err(|e| format!("bundle.import: invalid zip_base64: {e}"))?;
                            crate::backend::bundle_import::unzip_bundle_import(&zip_bytes)
                                .map_err(|e| format!("bundle.import: {e}"))?
                        }
                        (None, Some(files)) => {
                            // reagent P1, PR #2379 round 5: the raw `files`
                            // list previously skipped straight to
                            // parse_bundle_import with zero size/count
                            // enforcement, even though the spec treats it as
                            // an equally untrusted alternate ingestion path
                            // to zip_base64 — bypassing every zip-bomb/DoS
                            // defense the zip path enforces.
                            let raw_files: Vec<crate::backend::bundle_import::BundleImportFile> = files
                                .into_iter()
                                .map(|f| crate::backend::bundle_import::BundleImportFile {
                                    path: f.path,
                                    content: f.content,
                                })
                                .collect();
                            crate::backend::bundle_import::enforce_raw_files_caps(raw_files)
                                .map_err(|e| format!("bundle.import: {e}"))?
                        }
                    };

                let parsed = crate::backend::bundle_import::parse_bundle_import(&files)
                    .map_err(|e| format!("bundle.import: {e}"))?;
                warnings.extend(parsed.warnings.clone());

                // Account requirement resolution (§4.5) — read-only lookup
                // by provider, purely informational. Never creates an
                // account, never writes anything derived from a match.
                // Phase 3 spec §3.1: extracted into resolve_account_requirements,
                // shared with the new preview/commit RPC handlers.
                // Today's existing route keeps its response shape exactly
                // as before -- unbounded, matching its own already-shipped
                // behavior. The bounded-display projection below is a
                // Phase 3 preview/commit-only requirement.
                let resolved = resolve_account_requirements(&id_store, &parsed.requirements)
                    .map_err(|e| format!("bundle.import: {e}"))?;
                let resolved_requirement_ids: Vec<String> =
                    resolved.iter().filter(|r| r.resolved()).map(|r| r.id.clone()).collect();
                let unresolved_requirements: Vec<serde_json::Value> = resolved
                    .iter()
                    .filter(|r| !r.resolved())
                    .map(|r| json!({
                        "id": r.id,
                        "provider": r.provider,
                        "env": r.env,
                        "match_count": r.match_count,
                    }))
                    .collect();

                // Skills — global, not bound to any agent (mirrors
                // skill.catalog.upsert's own is_global: true convention for
                // a shared-resource creation path). A genuine name
                // conflict (the ONLY intentional business error
                // `skill_upsert_unique_global` returns —
                // `StoreError::Other` containing "already exists") is a
                // per-skill warning, not an aborted import. Codex P2, PR
                // #2379: any OTHER error (locked/corrupt store, I/O
                // failure) previously hit this same arm and was silently
                // treated as a skip too — turning an infrastructure
                // failure into an apparently-successful lossy import.
                // Only the confirmed-conflict shape is swallowed; anything
                // else aborts the whole RPC.
                // codex P2, PR #2379 round 4: rollback previously discarded
                // every skill_delete error via `let _ = ...`, so if the SAME
                // failing store also rejects the cleanup deletes, previously-
                // inserted global skills could silently survive an RPC that
                // reports failure. No transaction primitive exists on Store
                // today, so full atomicity is out of scope here; surfacing
                // the rollback failure instead of swallowing it is the fix
                // that fits — the caller at least learns cleanup itself may
                // not have fully succeeded.
                let rollback_skills = |ids: &[String]| -> Vec<String> {
                    ids.iter()
                        .filter_map(|id| wstore.skill_delete(id).err().map(|e| format!("{id}: {e}")))
                        .collect()
                };

                let now = now_ms();
                let mut imported_skill_ids: Vec<String> = Vec::new();
                let mut skipped_skills = parsed.skipped_skills.clone();
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
                    match wstore.skill_upsert_unique_global(&row) {
                        Ok(()) => imported_skill_ids.push(row.id),
                        Err(crate::backend::storage::error::StoreError::Other(msg))
                            if msg.contains("already exists") =>
                        {
                            warnings.push(format!("skill \"{}\": {msg}", skill.slug));
                            skipped_skills.push(skill.slug.clone());
                        }
                        Err(e) => {
                            // Codex P2, PR #2379: roll back every skill this
                            // loop already created before propagating —
                            // otherwise an infra failure partway through
                            // leaves orphaned global skill rows (bound to
                            // no bundle, but visible/bindable by any agent)
                            // behind a failed RPC.
                            let rollback_errors = rollback_skills(&imported_skill_ids);
                            let mut msg = format!(
                                "bundle.import: failed to create skill \"{}\": {e}",
                                skill.slug
                            );
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

                let memory = Memory {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: parsed.name,
                    description: parsed.description,
                    is_blank: false,
                    // Never imported as global — an import must not
                    // silently start injecting into every agent's
                    // CLAUDE.md without explicit user action (spec §4.4).
                    is_global: false,
                    provider: String::new(),
                    model: String::new(),
                    instructions: parsed.instructions,
                    // ABF v0.2 §2.2: every variant is stored verbatim, no
                    // merge decision made at import time.
                    instructions_by_provider: serde_json::to_string(&parsed.instructions_by_provider)
                        .unwrap_or_else(|_| "{}".to_string()),
                    // Phase 3 spec §3.0: ImportedContextFile gained a
                    // stable `id` selection key alongside `path`/`content`
                    // -- project it away before persisting, matching
                    // `Memory.context_files`'s existing documented
                    // `[{path, content}]` shape.
                    context_files: serde_json::to_string(
                        &parsed.context_files.iter().map(|cf| json!({"path": cf.path, "content": cf.content})).collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| "[]".to_string()),
                    // Phase 3 spec §3.0, round 2: `parsed.mcp_servers` is now
                    // `Vec<ParsedMcpServer>{source_path, config}` (a stable
                    // selection key alongside the raw config) — every write
                    // site must project to `.config` before serializing, or
                    // this would persist the wrapper object instead of the
                    // raw MCP config every consumer of `Memory.mcp_servers`
                    // expects.
                    mcp_servers: serde_json::to_string(
                        &parsed.mcp_servers.iter().map(|m| &m.config).collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| "[]".to_string()),
                    skills: serde_json::to_string(&imported_skill_ids)
                        .unwrap_or_else(|_| "[]".to_string()),
                    sort_order: 0,
                    created_at: now,
                    updated_at: now,
                };
                if let Err(e) = id_store.bundle_memory_upsert(&memory) {
                    // Codex P2, PR #2379: same rollback as above — the
                    // skills this RPC just created must not survive a
                    // failed bundle creation as orphaned global rows.
                    let rollback_errors = rollback_skills(&imported_skill_ids);
                    let mut msg = format!("bundle.import: {e}");
                    if !rollback_errors.is_empty() {
                        msg.push_str(&format!(
                            "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                            rollback_errors.len(),
                            rollback_errors.join("; ")
                        ));
                    }
                    return Err(msg);
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                if !imported_skill_ids.is_empty() {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }

                Ok(Some(json!({
                    "bundle_id": memory.id,
                    "imported_skill_ids": imported_skill_ids,
                    "skipped_skills": skipped_skills,
                    "resolved_requirement_ids": resolved_requirement_ids,
                    "unresolved_requirements": unresolved_requirements,
                    "warnings": warnings,
                })))
            })
        }),
    );
}

/// On-disk size ceiling for `file_path` input (Phase 3 spec §3.0.5, rounds
/// 4/5) — generous headroom above `MAX_TOTAL_UNCOMPRESSED_BYTES`'s 50 MiB
/// for compression/container overhead and imperfectly-compressed content.
const MAX_ABF_FILE_SIZE_BYTES: u64 = 100 * 1024 * 1024;

/// Reads a `file_path` input server-side (Phase 3 spec §3.0.5). Opens the
/// path with a no-follow mechanism so a symlink fails to open at all
/// rather than being silently resolved to its target (round 6) — one
/// handle, one continuous read, no second path resolution for anything to
/// race against (round 5's TOCTOU fix): metadata comes from that SAME open
/// handle and is checked against `MAX_ABF_FILE_SIZE_BYTES` BEFORE any read
/// (round 4); the actual read is still hard-bounded via `.take(...)`
/// regardless of what the metadata claimed.
fn read_abf_file_path(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("{path}: failed to open: {e}"))?
    };
    #[cfg(windows)]
    let mut file = {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT (0x0020_0000): opens a symlink/
        // junction AS the reparse point itself, rather than transparently
        // following it to its target — the Windows equivalent of Unix's
        // O_NOFOLLOW, in a single atomic CreateFileW call (no separate
        // path resolution for a TOCTOU window to open in). The metadata
        // check immediately below then rejects the handle outright if it
        // turns out to be a reparse point. Windows symlinks require
        // elevated privileges to create by default, narrowing (not
        // eliminating) the practical risk this guards against.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|e| format!("{path}: failed to open: {e}"))?
    };
    #[cfg(not(any(unix, windows)))]
    let mut file = std::fs::File::open(path).map_err(|e| format!("{path}: failed to open: {e}"))?;

    let metadata = file.metadata().map_err(|e| format!("{path}: failed to stat: {e}"))?;
    #[cfg(windows)]
    if metadata.file_type().is_symlink() {
        return Err(format!("{path}: refusing to follow a symlink"));
    }
    if !metadata.is_file() {
        return Err(format!("{path}: not a regular file"));
    }
    if metadata.len() > MAX_ABF_FILE_SIZE_BYTES {
        return Err(format!(
            "{path}: size ({} bytes) exceeds the limit ({MAX_ABF_FILE_SIZE_BYTES} bytes)",
            metadata.len()
        ));
    }

    let mut buf: Vec<u8> = Vec::new();
    let mut limited = (&mut file).take(MAX_ABF_FILE_SIZE_BYTES + 1);
    limited.read_to_end(&mut buf).map_err(|e| format!("{path}: failed to read: {e}"))?;
    if buf.len() as u64 > MAX_ABF_FILE_SIZE_BYTES {
        return Err(format!("{path}: exceeds the size limit while reading"));
    }
    Ok(buf)
}

/// Bounded warning budget shared by `bundle.import.preview`/`.commit`
/// (Phase 3 spec §3.1, round 11) — unlike the existing `bundle.import`
/// route's effectively-unbounded budget, preserved unchanged above.
fn preview_commit_warning_budget() -> crate::backend::bundle_import::WarningBudget {
    crate::backend::bundle_import::WarningBudget::bounded(200, 300)
}

/// Final combined-list bound applied to a preview/commit response's
/// `warnings` (Phase 3 spec §3.1 round 8 / round 9's commit-response
/// generalization) — cheap defense-in-depth over an already
/// per-source-bounded list (each of `resolve_import_input`'s intake
/// warnings and `parse_bundle_import_with_budget`'s own warnings is
/// already individually bounded; concatenating two independently-bounded
/// lists can still exceed either bound alone, so one more pass caps the
/// combined total before it's ever serialized).
fn bound_warnings_for_response(warnings: Vec<String>) -> (Vec<String>, bool) {
    const MAX_COMBINED_WARNINGS: usize = 200;
    if warnings.len() <= MAX_COMBINED_WARNINGS {
        (warnings, false)
    } else {
        let mut kept: Vec<String> = warnings.into_iter().take(MAX_COMBINED_WARNINGS).collect();
        kept.push("... additional warnings not shown".to_string());
        (kept, true)
    }
}

/// The outcome of resolving one of `file_path`/`zip_base64`/`files` into
/// intake-ready files plus the Phase 3 content digest (§3.0.5) — shared by
/// `bundle.import.preview` and `.commit` so both compute the digest
/// identically. The mode tag baked into `content_digest` (round 7) means a
/// plain digest-equality check at commit is sufficient to enforce "same
/// input mode as preview" (round 6) — no separate mode field is needed.
#[derive(Debug)]
struct ResolvedImportInput {
    files: Vec<crate::backend::bundle_import::BundleImportFile>,
    intake_warnings: Vec<String>,
    content_digest: String,
}

fn resolve_import_input(
    file_path: Option<String>,
    zip_base64: Option<String>,
    files: Option<Vec<FileEntry>>,
    warning_budget: crate::backend::bundle_import::WarningBudget,
) -> Result<ResolvedImportInput, String> {
    use crate::backend::bundle_import as bi;
    let provided = [file_path.is_some(), zip_base64.is_some(), files.is_some()]
        .iter()
        .filter(|p| **p)
        .count();
    if provided != 1 {
        return Err(format!(
            "exactly one of file_path/zip_base64/files must be present, got {provided}"
        ));
    }
    if let Some(path) = file_path {
        let raw = read_abf_file_path(&path)?;
        let content_digest = bi::content_digest_raw_bytes(bi::ImportInputMode::FilePath, &raw);
        let (files, intake_warnings) = bi::unzip_bundle_import_with_budget(&raw, warning_budget)?;
        return Ok(ResolvedImportInput { files, intake_warnings, content_digest });
    }
    if let Some(b64) = zip_base64 {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .map_err(|e| format!("invalid zip_base64: {e}"))?;
        let content_digest = bi::content_digest_raw_bytes(bi::ImportInputMode::ZipBase64, &raw);
        let (files, intake_warnings) = bi::unzip_bundle_import_with_budget(&raw, warning_budget)?;
        return Ok(ResolvedImportInput { files, intake_warnings, content_digest });
    }
    let files = files.expect("exactly one input already validated present");
    let raw_files: Vec<bi::BundleImportFile> = files
        .into_iter()
        .map(|f| bi::BundleImportFile { path: f.path, content: f.content })
        .collect();
    let (capped_files, intake_warnings) = bi::enforce_raw_files_caps_with_budget(raw_files, warning_budget)?;
    let content_digest = bi::content_digest_files(&capped_files);
    Ok(ResolvedImportInput { files: capped_files, intake_warnings, content_digest })
}

#[derive(serde::Deserialize, Default)]
struct PreviewReq {
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    zip_base64: Option<String>,
    #[serde(default)]
    files: Option<Vec<FileEntry>>,
}

/// `bundle.import.preview` — Phase 3 of
/// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §3.1. Pure parse plus
/// read-only collision/name-collision lookups — zero Store writes. Window-
/// scoped like the rest of `bundle.*`. Extracted from the RPC closure into
/// a directly-callable, directly-testable function (the closure itself
/// only deserializes the request and forwards).
async fn bundle_import_preview_impl(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    req: PreviewReq,
) -> Result<serde_json::Value, String> {
    use crate::backend::bundle_import as bi;

    let resolved = resolve_import_input(req.file_path, req.zip_base64, req.files, preview_commit_warning_budget())
        .map_err(|e| format!("bundle.import.preview: {e}"))?;

    let parsed = bi::parse_bundle_import_with_budget(&resolved.files, preview_commit_warning_budget())
        .map_err(|e| format!("bundle.import.preview: {e}"))?;

    let mut all_warnings = resolved.intake_warnings;
    all_warnings.extend(parsed.warnings);

    // Skill collision detection (§3.1, two-pass).
    let global_slugs: std::collections::HashSet<String> = wstore
        .skill_list_global()
        .map_err(|e| format!("bundle.import.preview: {e}"))?
        .into_iter()
        .map(|item| item.skill.name)
        .collect();
    let in_bundle_dupes = bi::duplicate_in_bundle_slugs(&parsed.skills);

    let skills_json: Vec<serde_json::Value> = parsed
        .skills
        .iter()
        .map(|skill| {
            let collision = bi::classify_skill_collision(&skill.slug, &global_slugs, &in_bundle_dupes);
            json!({
                "source_dir": skill.source_dir,
                "slug": bounded_display(&skill.slug),
                "description": bounded_display(&skill.description),
                "collision": collision,
            })
        })
        .collect();

    let mcp_servers_json: Vec<serde_json::Value> = parsed
        .mcp_servers
        .iter()
        .map(|m| json!({
            "source_path": m.source_path,
            "display": bi::mcp_server_display(&m.config),
        }))
        .collect();

    let (instructions_preview, instructions_truncated, instructions_total_chars) =
        bi::bounded_instructions_preview(&parsed.instructions);

    let context_files_json: Vec<serde_json::Value> = parsed
        .context_files
        .iter()
        .map(|cf| json!({
            "id": cf.id,
            "display_path": bounded_display(&cf.path),
            "size_bytes": cf.content.len(),
        }))
        .collect();

    let resolved_requirements = resolve_account_requirements(id_store, &parsed.requirements)
        .map_err(|e| format!("bundle.import.preview: {e}"))?;
    let requirements_json: Vec<serde_json::Value> = resolved_requirements
        .iter()
        .map(|r| json!({
            "id": bounded_display(&r.id),
            "provider": bounded_display(&r.provider),
            "env": bounded_display(&r.env),
            "resolved": r.resolved(),
            "match_count": r.match_count,
        }))
        .collect();

    // Bundle name collision -- soft, informational (§2: bundle_memory_upsert
    // has no name uniqueness constraint, so this never blocks).
    let existing_names: std::collections::HashSet<String> = id_store
        .bundle_memory_list()
        .map_err(|e| format!("bundle.import.preview: {e}"))?
        .into_iter()
        .map(|b| b.name)
        .collect();
    let name_collision = existing_names.contains(&parsed.name);

    let (warnings, warnings_truncated) = bound_warnings_for_response(all_warnings);

    Ok(json!({
        "name": parsed.name,
        "description": bounded_display(&parsed.description),
        "instructions_preview": instructions_preview,
        "instructions_truncated": instructions_truncated,
        "instructions_total_chars": instructions_total_chars,
        "context_files": context_files_json,
        "skills": skills_json,
        "mcp_servers": mcp_servers_json,
        "requirements": requirements_json,
        "warnings": warnings,
        "warnings_truncated": warnings_truncated,
        "name_collision": name_collision,
        "content_digest": resolved.content_digest,
    }))
}

fn register_bundle_import_preview(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_BUNDLE_IMPORT_PREVIEW,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                let req: PreviewReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.import.preview: {e}"))?;
                Ok(Some(bundle_import_preview_impl(&id_store, &wstore, req).await?))
            })
        }),
    );
}

#[derive(serde::Deserialize)]
struct SkillSelection {
    source_dir: String,
    #[serde(default)]
    import_as: Option<String>,
}
#[derive(serde::Deserialize, Default)]
struct CommitReq {
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    zip_base64: Option<String>,
    #[serde(default)]
    files: Option<Vec<FileEntry>>,
    #[serde(default)]
    expected_content_digest: String,
    #[serde(default)]
    bundle_name: Option<String>,
    #[serde(default)]
    include_instructions: bool,
    #[serde(default)]
    include_context_files: Vec<usize>,
    #[serde(default)]
    include_skills: Vec<SkillSelection>,
    #[serde(default)]
    include_mcp_servers: Vec<String>,
}

/// `bundle.import.commit` — Phase 3 §3.2. Re-resolves and re-parses the
/// same input fresh (never trusts client-supplied preview data for
/// anything written), rejects on a content-digest mismatch, then writes
/// only the selected items. Shares `bundle.import`'s existing skill-write/
/// rollback logic; the differences are selection filtering, the
/// `bundle_name`/`import_as` overrides, and the digest gate. Extracted
/// from the RPC closure into a directly-callable, directly-testable
/// function (the closure itself only deserializes the request and
/// forwards).
async fn bundle_import_commit_impl(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    broker: &crate::backend::wps::Broker,
    req: CommitReq,
) -> Result<serde_json::Value, String> {
    use crate::backend::bundle_import as bi;

                let resolved = resolve_import_input(req.file_path, req.zip_base64, req.files, preview_commit_warning_budget())
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;

                // §3.0.5: reject BEFORE doing anything else on a mismatch --
                // never a partial import against whatever content was
                // actually given. The mode tag baked into content_digest
                // (round 7) means this single comparison also enforces
                // "same input mode as preview" (round 6) with no separate
                // mode field needed.
                if resolved.content_digest != req.expected_content_digest {
                    return Err(
                        "bundle.import.commit: content changed since preview (digest mismatch) — re-select and preview again"
                            .to_string(),
                    );
                }

                let parsed = bi::parse_bundle_import_with_budget(&resolved.files, preview_commit_warning_budget())
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;

                let mut warnings = resolved.intake_warnings;
                warnings.extend(parsed.warnings.clone());

                // codex P2, PR #2381 round 11: bundle_name must actually be
                // substituted for parsed.name, never silently ignored.
                // reagentx P2, PR #2382 round 3: unlike parsed.name (bounded
                // at parse time), a client-supplied override had no length
                // cap at all -- bound_bundle_name closes that gap the same
                // way for both paths (a no-op when bundle_name is already
                // parsed.name, since that's already within the cap).
                let bundle_name = bi::bound_bundle_name(&req.bundle_name.unwrap_or_else(|| parsed.name.clone()));

                let instructions = if req.include_instructions { parsed.instructions.clone() } else { String::new() };
                // ABF v0.2 §2.2: provider-scoped variants are part of the
                // same "instructions" component, gated by the same
                // include_instructions flag — no separate per-variant
                // selection exists in the Phase 3 preview/commit UI.
                let instructions_by_provider = if req.include_instructions {
                    parsed.instructions_by_provider.clone()
                } else {
                    std::collections::HashMap::new()
                };

                // include_context_files selects by the stable `id` (round
                // 13), never by (truncatable) display_path.
                let selected_context_files: Vec<serde_json::Value> = parsed
                    .context_files
                    .iter()
                    .filter(|cf| req.include_context_files.contains(&cf.id))
                    .map(|cf| json!({ "path": cf.path, "content": cf.content }))
                    .collect();

                // include_mcp_servers selects by source_path (§3.0), never
                // by a JSON "name" field. The write path projects to
                // .config, matching the §3.0 amendment.
                let include_mcp: std::collections::HashSet<&str> =
                    req.include_mcp_servers.iter().map(|s| s.as_str()).collect();
                let selected_mcp_servers: Vec<&serde_json::Value> = parsed
                    .mcp_servers
                    .iter()
                    .filter(|m| include_mcp.contains(m.source_path.as_str()))
                    .map(|m| &m.config)
                    .collect();

                let resolved_reqs = resolve_account_requirements(id_store, &parsed.requirements)
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;
                // codex P2, PR #2381 round 12: commit's response reuses the
                // exact same bounded-display projection preview's
                // equivalent fields use, via the shared bounded_display fn.
                let resolved_requirement_ids: Vec<String> =
                    resolved_reqs.iter().filter(|r| r.resolved()).map(|r| bounded_display(&r.id)).collect();
                let unresolved_requirements: Vec<serde_json::Value> = resolved_reqs
                    .iter()
                    .filter(|r| !r.resolved())
                    .map(|r| json!({
                        "id": bounded_display(&r.id),
                        "provider": bounded_display(&r.provider),
                        "env": bounded_display(&r.env),
                        "match_count": r.match_count,
                    }))
                    .collect();

                // Two-pass skill collision, recomputed server-side --
                // §4.1 point 4's "empty rename on a colliding skill = skip"
                // rule is enforced authoritatively here, not left to a
                // possibly-buggy/malicious client to have honored: without
                // this, a "duplicate_in_bundle" collision (not yet in the
                // global catalog) would let the FIRST of two same-slug
                // skills write successfully under its own slug even with
                // an empty rename, since skill_upsert_unique_global alone
                // wouldn't reject it until the second attempt.
                let global_slugs: std::collections::HashSet<String> = wstore
                    .skill_list_global()
                    .map_err(|e| format!("bundle.import.commit: {e}"))?
                    .into_iter()
                    .map(|item| item.skill.name)
                    .collect();
                let in_bundle_dupes = bi::duplicate_in_bundle_slugs(&parsed.skills);

                let rollback_skills = |ids: &[String]| -> Vec<String> {
                    ids.iter()
                        .filter_map(|id| wstore.skill_delete(id).err().map(|e| format!("{id}: {e}")))
                        .collect()
                };

                let now = now_ms();
                let mut imported_skill_ids: Vec<String> = Vec::new();
                let mut skipped_skills: Vec<String> = Vec::new();

                // reagentx P1, PR #2382 round 3: req.include_skills is
                // client-supplied with no length cap and no dedup by
                // source_dir -- an unbounded/repeated array could otherwise
                // drive an unbounded number of skill_upsert_unique_global
                // Store writes regardless of how many skills the archive
                // itself actually parsed to (parsed.skills is already
                // bounded by MAX_IMPORTED_SKILLS). First occurrence wins per
                // source_dir (matches this module's existing duplicate-
                // reference convention elsewhere), then capped at the same
                // MAX_IMPORTED_SKILLS the parser itself enforces.
                let mut seen_source_dirs: std::collections::HashSet<&str> = std::collections::HashSet::new();
                let deduped_skill_selections: Vec<&SkillSelection> = req
                    .include_skills
                    .iter()
                    .filter(|s| seen_source_dirs.insert(s.source_dir.as_str()))
                    .take(bi::MAX_IMPORTED_SKILLS)
                    .collect();

                for selection in deduped_skill_selections {
                    let Some(skill) = parsed.skills.iter().find(|s| s.source_dir == selection.source_dir) else {
                        continue; // source_dir not present in this parse -- nothing to import
                    };
                    let collision = bi::classify_skill_collision(&skill.slug, &global_slugs, &in_bundle_dupes);
                    let non_empty_rename = selection.import_as.as_deref().filter(|s| !s.is_empty());

                    if collision != "none" && non_empty_rename.is_none() {
                        // §4.1 point 4: never silently sent through with
                        // its original, known-conflicting slug.
                        skipped_skills.push(bounded_display(&skill.slug));
                        continue;
                    }
                    let effective_slug = non_empty_rename.map(|s| s.to_string()).unwrap_or_else(|| skill.slug.clone());

                    let row = crate::backend::storage::Skill {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: effective_slug.clone(),
                        trigger: effective_slug.clone(),
                        skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                        description: skill.description.clone(),
                        content: skill.content.clone(),
                        is_global: true,
                        created_at: now,
                        updated_at: now,
                    };
                    match wstore.skill_upsert_unique_global(&row) {
                        Ok(()) => imported_skill_ids.push(row.id),
                        Err(crate::backend::storage::error::StoreError::Other(msg))
                            if msg.contains("already exists") =>
                        {
                            // codex P2, PR #2382 round 2: effective_slug can be
                            // the caller-supplied import_as, which has no
                            // length bound anywhere before this point (unlike
                            // a parsed skill.slug, implicitly bounded by the
                            // per-entry decompression cap) -- and msg's own
                            // text embeds the identical unbounded value a
                            // second time (StoreError::Other's own format!
                            // interpolates skill.name). Use a fixed message
                            // instead of msg's text (its only informative
                            // content, "already exists", is already implied by
                            // the branch guard) and the shared bounded_display
                            // projection every other warning in this handler
                            // already uses, closing both unbounded paths at
                            // once rather than truncating msg's text in place.
                            warnings.push(format!("skill \"{}\": already exists", bounded_display(&effective_slug)));
                            skipped_skills.push(bounded_display(&effective_slug));
                        }
                        Err(e) => {
                            let rollback_errors = rollback_skills(&imported_skill_ids);
                            let mut msg = format!(
                                "bundle.import.commit: failed to create skill \"{effective_slug}\": {e}"
                            );
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

                let memory = Memory {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: bundle_name,
                    description: parsed.description,
                    is_blank: false,
                    is_global: false,
                    provider: String::new(),
                    model: String::new(),
                    instructions,
                    instructions_by_provider: serde_json::to_string(&instructions_by_provider)
                        .unwrap_or_else(|_| "{}".to_string()),
                    context_files: serde_json::to_string(&selected_context_files)
                        .unwrap_or_else(|_| "[]".to_string()),
                    mcp_servers: serde_json::to_string(&selected_mcp_servers)
                        .unwrap_or_else(|_| "[]".to_string()),
                    skills: serde_json::to_string(&imported_skill_ids)
                        .unwrap_or_else(|_| "[]".to_string()),
                    sort_order: 0,
                    created_at: now,
                    updated_at: now,
                };
                if let Err(e) = id_store.bundle_memory_upsert(&memory) {
                    let rollback_errors = rollback_skills(&imported_skill_ids);
                    let mut msg = format!("bundle.import.commit: {e}");
                    if !rollback_errors.is_empty() {
                        msg.push_str(&format!(
                            "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                            rollback_errors.len(),
                            rollback_errors.join("; ")
                        ));
                    }
                    return Err(msg);
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                if !imported_skill_ids.is_empty() {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }

                // codex P1, PR #2381 round 9: bounded the same as preview's
                // response, not left unbounded like today's bundle.import.
                let (bounded_warnings, warnings_truncated) = bound_warnings_for_response(warnings);

    Ok(json!({
        "bundle_id": memory.id,
        "imported_skill_ids": imported_skill_ids,
        "skipped_skills": skipped_skills,
        "resolved_requirement_ids": resolved_requirement_ids,
        "unresolved_requirements": unresolved_requirements,
        "warnings": bounded_warnings,
        "warnings_truncated": warnings_truncated,
    }))
}

fn register_bundle_import_commit(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_BUNDLE_IMPORT_COMMIT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let req: CommitReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;
                Ok(Some(bundle_import_commit_impl(&id_store, &wstore, &broker, req).await?))
            })
        }),
    );
}

#[cfg(test)]
mod import_preview_commit_tests {
    use super::*;
    use crate::backend::bundle_import as bi;
    use crate::server::tests::test_state;

    fn manifest(components: serde_json::Value) -> String {
        serde_json::to_string(&serde_json::json!({
            "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
            "name": "test-bundle",
            "version": "0.1.0",
            "description": "A test bundle",
            "components": components,
            "metadata": {},
        }))
        .unwrap()
    }

    fn entry(path: &str, content: &str) -> FileEntry {
        FileEntry { path: path.to_string(), content: content.to_string() }
    }

    fn skill_md(name: &str, description: &str, body: &str) -> String {
        crate::backend::agent_config::render_skill_md(name, description, body)
    }

    #[tokio::test]
    async fn preview_returns_parsed_bundle_with_digest() {
        let state = test_state();
        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md", "instructions/context/notes.md"],
                "skills": ["skills/deploy"],
            }))),
            entry("instructions/AGENTS.md", "Be concise."),
            entry("instructions/context/notes.md", "Extra context."),
            entry("skills/deploy/SKILL.md", &skill_md("deploy", "Runs the checklist", "1. Test\n2. Deploy")),
        ];
        let req = PreviewReq { file_path: None, zip_base64: None, files: Some(files) };
        let resp = bundle_import_preview_impl(&state.id_store, &state.wstore, req).await.unwrap();

        assert_eq!(resp["name"], "test-bundle");
        assert_eq!(resp["instructions_preview"], "Be concise.");
        assert_eq!(resp["context_files"][0]["id"], 0);
        assert_eq!(resp["context_files"][0]["display_path"], "notes.md");
        assert_eq!(resp["skills"][0]["source_dir"], "skills/deploy");
        assert_eq!(resp["skills"][0]["slug"], "deploy");
        assert_eq!(resp["skills"][0]["collision"], "none");
        assert!(resp["content_digest"].as_str().unwrap().len() > 0);
        assert_eq!(resp["name_collision"], false);
    }

    #[tokio::test]
    async fn preview_flags_name_conflict_against_existing_global_skill() {
        let state = test_state();
        // Seed an existing global skill named "deploy".
        state
            .wstore
            .skill_upsert_unique_global(&crate::backend::storage::Skill {
                id: "existing-1".to_string(),
                name: "deploy".to_string(),
                trigger: "deploy".to_string(),
                skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                description: "pre-existing".to_string(),
                content: "pre-existing body".to_string(),
                is_global: true,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();

        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
            entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
        ];
        let req = PreviewReq { file_path: None, zip_base64: None, files: Some(files) };
        let resp = bundle_import_preview_impl(&state.id_store, &state.wstore, req).await.unwrap();
        assert_eq!(resp["skills"][0]["collision"], "name_conflict");
    }

    #[tokio::test]
    async fn preview_flags_duplicate_in_bundle_when_two_parsed_skills_share_a_slug() {
        // Phase 3 spec §3.1, codex P1 round 2: two skills within the same
        // bundle sharing a slug that ISN'T yet global must both be flagged,
        // not silently passed as "none".
        let state = test_state();
        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({
                "skills": ["skills/code-review-v2", "skills/code-review-old"],
            }))),
            entry("skills/code-review-v2/SKILL.md", &skill_md("code-review", "new", "body-new")),
            entry("skills/code-review-old/SKILL.md", &skill_md("code-review", "old", "body-old")),
        ];
        let req = PreviewReq { file_path: None, zip_base64: None, files: Some(files) };
        let resp = bundle_import_preview_impl(&state.id_store, &state.wstore, req).await.unwrap();
        let skills = resp["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills.iter().all(|s| s["collision"] == "duplicate_in_bundle"));
    }

    #[tokio::test]
    async fn commit_rejects_on_digest_mismatch_and_writes_nothing() {
        let state = test_state();
        let files = vec![entry("armory.json", &manifest(serde_json::json!({})))];
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: "not-the-real-digest".to_string(),
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![],
            include_mcp_servers: vec![],
        };
        let err = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req)
            .await
            .unwrap_err();
        assert!(err.contains("digest mismatch"));
        assert!(state.id_store.bundle_memory_list().unwrap().iter().all(|b| b.name != "test-bundle"));
    }

    #[tokio::test]
    async fn commit_applies_bundle_name_override_not_parsed_name() {
        // codex P2, PR #2381 round 11: bundle_name must actually be
        // substituted for Memory.name, never silently ignored.
        let state = test_state();
        let files = vec![entry("armory.json", &manifest(serde_json::json!({})))];
        let digest = bi::content_digest_files(&files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect::<Vec<_>>());
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: Some("Renamed Bundle".to_string()),
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![],
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        let bundle_id = resp["bundle_id"].as_str().unwrap();
        let saved = state.id_store.bundle_memory_get(bundle_id).unwrap().unwrap();
        assert_eq!(saved.name, "Renamed Bundle");
    }

    #[tokio::test]
    async fn commit_bounds_an_oversized_bundle_name_override() {
        // reagentx P2, PR #2382 round 3: unlike parsed.name (bounded at
        // parse time), req.bundle_name had no length cap of its own before
        // being used verbatim as Memory.name.
        let state = test_state();
        let files = vec![entry("armory.json", &manifest(serde_json::json!({})))];
        let digest = bi::content_digest_files(&files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect::<Vec<_>>());
        let oversized_name = "n".repeat(bi::MAX_BUNDLE_NAME_CHARS + 500);
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: Some(oversized_name),
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![],
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        let bundle_id = resp["bundle_id"].as_str().unwrap();
        let saved = state.id_store.bundle_memory_get(bundle_id).unwrap().unwrap();
        assert_eq!(saved.name.chars().count(), bi::MAX_BUNDLE_NAME_CHARS);
    }

    #[tokio::test]
    async fn commit_dedupes_repeated_source_dirs_in_include_skills_first_occurrence_wins() {
        // reagentx P1, PR #2382 round 3: a client repeating the same
        // source_dir with a different import_as each time must not drive
        // one Store write per repetition.
        let state = test_state();
        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
            entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);
        let include_skills: Vec<SkillSelection> = (0..50)
            .map(|i| SkillSelection { source_dir: "skills/deploy".to_string(), import_as: Some(format!("deploy-{i}")) })
            .collect();
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills,
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        let imported = resp["imported_skill_ids"].as_array().unwrap();
        assert_eq!(imported.len(), 1, "expected only the first occurrence of the repeated source_dir to be written");
        let saved_skill = state.wstore.skill_get(imported[0].as_str().unwrap()).unwrap().unwrap();
        assert_eq!(saved_skill.name, "deploy-0");
    }

    #[tokio::test]
    async fn commit_caps_include_skills_at_max_imported_skills() {
        // reagentx P1, PR #2382 round 3: an include_skills array longer
        // than MAX_IMPORTED_SKILLS must not drive more than that many
        // skill_upsert_unique_global write attempts. Distinct-but-bogus
        // source_dirs (each a cheap no-op "continue") occupy the first
        // MAX_IMPORTED_SKILLS positions; three genuinely resolvable
        // selections are placed AFTER that boundary. If the cap truncates
        // the selection list itself (not just deduping), those three are
        // silently dropped -- proving the cap applies before resolution,
        // not just as an incidental side effect of the dedup fix.
        let state = test_state();
        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({
                "skills": ["skills/a", "skills/b", "skills/c"],
            }))),
            entry("skills/a/SKILL.md", &skill_md("a", "d", "body")),
            entry("skills/b/SKILL.md", &skill_md("b", "d", "body")),
            entry("skills/c/SKILL.md", &skill_md("c", "d", "body")),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);

        let mut include_skills: Vec<SkillSelection> = (0..bi::MAX_IMPORTED_SKILLS)
            .map(|i| SkillSelection { source_dir: format!("skills/nonexistent-{i}"), import_as: None })
            .collect();
        include_skills.push(SkillSelection { source_dir: "skills/a".to_string(), import_as: None });
        include_skills.push(SkillSelection { source_dir: "skills/b".to_string(), import_as: None });
        include_skills.push(SkillSelection { source_dir: "skills/c".to_string(), import_as: None });

        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills,
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        assert!(
            resp["imported_skill_ids"].as_array().unwrap().is_empty(),
            "the three real selections beyond the MAX_IMPORTED_SKILLS boundary must be dropped by the cap, not imported"
        );
    }

    #[tokio::test]
    async fn commit_selects_context_files_by_id_not_display_path() {
        let state = test_state();
        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({
                "instructions": ["instructions/context/a.md", "instructions/context/b.md"],
            }))),
            entry("instructions/context/a.md", "content A"),
            entry("instructions/context/b.md", "content B"),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);
        // Only select id 1 (b.md) -- verify a.md (id 0) is excluded.
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![1],
            include_skills: vec![],
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        let bundle_id = resp["bundle_id"].as_str().unwrap();
        let saved = state.id_store.bundle_memory_get(bundle_id).unwrap().unwrap();
        assert!(saved.context_files.contains("content B"));
        assert!(!saved.context_files.contains("content A"));
    }

    #[tokio::test]
    async fn commit_skips_colliding_skill_left_with_an_empty_rename() {
        // §4.1 point 4: never silently sent through under its original,
        // known-conflicting slug.
        let state = test_state();
        state
            .wstore
            .skill_upsert_unique_global(&crate::backend::storage::Skill {
                id: "existing-1".to_string(),
                name: "deploy".to_string(),
                trigger: "deploy".to_string(),
                skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                description: "pre-existing".to_string(),
                content: "pre-existing body".to_string(),
                is_global: true,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();

        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
            entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![SkillSelection { source_dir: "skills/deploy".to_string(), import_as: None }],
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        assert!(resp["imported_skill_ids"].as_array().unwrap().is_empty());
        assert_eq!(resp["skipped_skills"][0], "deploy");
    }

    #[tokio::test]
    async fn commit_imports_colliding_skill_under_a_non_empty_rename() {
        let state = test_state();
        state
            .wstore
            .skill_upsert_unique_global(&crate::backend::storage::Skill {
                id: "existing-1".to_string(),
                name: "deploy".to_string(),
                trigger: "deploy".to_string(),
                skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                description: "pre-existing".to_string(),
                content: "pre-existing body".to_string(),
                is_global: true,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();

        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
            entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![SkillSelection {
                source_dir: "skills/deploy".to_string(),
                import_as: Some("deploy-team-x".to_string()),
            }],
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        assert_eq!(resp["imported_skill_ids"].as_array().unwrap().len(), 1);
        let imported_id = resp["imported_skill_ids"][0].as_str().unwrap();
        let saved_skill = state.wstore.skill_get(imported_id).unwrap().unwrap();
        assert_eq!(saved_skill.name, "deploy-team-x");
    }

    #[tokio::test]
    async fn commit_bounds_an_oversized_import_as_in_the_already_exists_warning() {
        // codex P2, PR #2382 round 2: effective_slug (from caller-supplied
        // import_as) has no length bound before reaching the "already
        // exists" warning push -- unlike a parsed skill.slug, which is at
        // least implicitly bounded by the per-entry decompression cap.
        // StoreError::Other's own message text ALSO embeds the identical
        // unbounded value a second time.
        let state = test_state();
        let oversized_name = "n".repeat(bi::MAX_DISPLAY_FIELD_CHARS + 500);
        state
            .wstore
            .skill_upsert_unique_global(&crate::backend::storage::Skill {
                id: "existing-1".to_string(),
                name: "deploy".to_string(),
                trigger: "deploy".to_string(),
                skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                description: "pre-existing".to_string(),
                content: "pre-existing body".to_string(),
                is_global: true,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        state
            .wstore
            .skill_upsert_unique_global(&crate::backend::storage::Skill {
                id: "existing-2".to_string(),
                name: oversized_name.clone(),
                trigger: oversized_name.clone(),
                skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                description: "pre-existing".to_string(),
                content: "pre-existing body".to_string(),
                is_global: true,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();

        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
            entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![SkillSelection {
                source_dir: "skills/deploy".to_string(),
                import_as: Some(oversized_name),
            }],
            include_mcp_servers: vec![],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        assert!(resp["imported_skill_ids"].as_array().unwrap().is_empty());
        let warnings = resp["warnings"].as_array().unwrap();
        assert!(!warnings.is_empty(), "expected an already-exists warning");
        for w in warnings {
            let s = w.as_str().unwrap();
            assert!(
                s.chars().count() <= bi::MAX_DISPLAY_FIELD_CHARS + 3 + 40,
                "warning not bounded ({} chars): {s:?}",
                s.chars().count()
            );
        }
        let skipped = resp["skipped_skills"][0].as_str().unwrap();
        assert!(skipped.chars().count() <= bi::MAX_DISPLAY_FIELD_CHARS + 3);
    }

    #[tokio::test]
    async fn commit_persists_raw_mcp_config_not_the_source_path_wrapper() {
        // Phase 3 spec §3.0, round 2: every write site touching
        // parsed.mcp_servers must project to .config before serializing.
        let state = test_state();
        let files = vec![
            entry("armory.json", &manifest(serde_json::json!({ "mcpServers": ["mcp/github.server.json"] }))),
            entry("mcp/github.server.json", r#"{"command":"npx","args":["-y","gh-mcp"]}"#),
        ];
        let bi_files: Vec<bi::BundleImportFile> =
            files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
        let digest = bi::content_digest_files(&bi_files);
        let req = CommitReq {
            file_path: None,
            zip_base64: None,
            files: Some(files),
            expected_content_digest: digest,
            bundle_name: None,
            include_instructions: false,
            include_context_files: vec![],
            include_skills: vec![],
            include_mcp_servers: vec!["mcp/github.server.json".to_string()],
        };
        let resp = bundle_import_commit_impl(&state.id_store, &state.wstore, &state.broker, req).await.unwrap();
        let bundle_id = resp["bundle_id"].as_str().unwrap();
        let saved = state.id_store.bundle_memory_get(bundle_id).unwrap().unwrap();
        let mcp_servers: serde_json::Value = serde_json::from_str(&saved.mcp_servers).unwrap();
        assert_eq!(mcp_servers[0]["command"], "npx");
        assert!(mcp_servers[0].get("source_path").is_none(), "must not persist the {{source_path, config}} wrapper");
    }

    #[tokio::test]
    async fn read_abf_file_path_rejects_a_missing_path() {
        let err = read_abf_file_path("C:\\definitely\\not\\a\\real\\path.abf").unwrap_err();
        assert!(err.contains("failed to open") || err.contains("failed to stat"));
    }

    #[tokio::test]
    async fn read_abf_file_path_rejects_a_directory() {
        let dir = std::env::temp_dir();
        let err = read_abf_file_path(dir.to_str().unwrap()).unwrap_err();
        assert!(err.contains("not a regular file") || err.contains("failed to open"));
    }

    #[tokio::test]
    async fn read_abf_file_path_reads_a_small_file_correctly() {
        let path = std::env::temp_dir().join(format!("abf-test-{}.bin", std::process::id()));
        std::fs::write(&path, b"hello abf").unwrap();
        let result = read_abf_file_path(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert_eq!(result.unwrap(), b"hello abf".to_vec());
    }

    #[tokio::test]
    async fn read_abf_file_path_rejects_a_file_over_the_size_cap_via_sparse_file() {
        // Verified via a sparse/pre-allocated file (metadata-based rejection,
        // before any read) rather than actually writing 100MB+ to disk.
        let path = std::env::temp_dir().join(format!("abf-test-oversized-{}.bin", std::process::id()));
        {
            let file = std::fs::File::create(&path).unwrap();
            file.set_len(MAX_ABF_FILE_SIZE_BYTES + 1).unwrap();
        }
        let result = read_abf_file_path(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        let err = result.unwrap_err();
        assert!(err.contains("exceeds the limit"), "expected a size-limit error, got: {err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_abf_file_path_rejects_a_symlink() {
        let target = std::env::temp_dir().join(format!("abf-symlink-target-{}.bin", std::process::id()));
        let link = std::env::temp_dir().join(format!("abf-symlink-{}.bin", std::process::id()));
        std::fs::write(&target, b"real content").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let result = read_abf_file_path(link.to_str().unwrap());
        std::fs::remove_file(&target).ok();
        std::fs::remove_file(&link).ok();
        assert!(result.is_err(), "opening a symlink via file_path must fail, not silently follow it");
    }

    #[test]
    fn bound_warnings_for_response_caps_the_combined_list() {
        let many: Vec<String> = (0..500).map(|i| format!("warning {i}")).collect();
        let (bounded, truncated) = bound_warnings_for_response(many);
        assert!(truncated);
        assert!(bounded.len() <= 201);
        assert!(bounded.last().unwrap().contains("not shown"));
    }

    #[test]
    fn bound_warnings_for_response_leaves_a_short_list_untouched() {
        let few = vec!["a".to_string(), "b".to_string()];
        let (bounded, truncated) = bound_warnings_for_response(few.clone());
        assert!(!truncated);
        assert_eq!(bounded, few);
    }

    #[test]
    fn resolve_import_input_rejects_when_zero_or_multiple_inputs_given() {
        let budget = bi::WarningBudget::unbounded();
        let none = resolve_import_input(None, None, None, budget).unwrap_err();
        assert!(none.contains("exactly one"));
        let both = resolve_import_input(Some("x".to_string()), Some("y".to_string()), None, budget).unwrap_err();
        assert!(both.contains("exactly one"));
    }
}

/// ABF v0.2 §2.3 — `bundle.export_for_agent`/`bundle.import_for_agent`.
/// Uses a real temp directory for the native-memory filesystem (these
/// handlers genuinely touch disk, unlike the pure bundle_export.rs/
/// bundle_import.rs modules), mirroring native_memory_handlers.rs's own
/// test fixtures.
#[cfg(test)]
mod export_import_for_agent_tests {
    use super::*;
    use crate::server::tests::test_state;

    /// Insert an AgentDefinition with a real working_directory + a
    /// CLAUDE_CONFIG_DIR env pointing at `config_dir` (must be a per-test
    /// temp dir — an empty/shared value would resolve to the real
    /// ~/.agentmux/shared/providers/claude/, writing test fixtures into
    /// the developer's actual home directory, exactly the trap
    /// native_memory_handlers.rs's own test helper's doc comment warns
    /// about).
    fn make_agent(state: &AppState, id: &str, working_directory: &str, config_dir: &std::path::Path) {
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": id,
            "slug": id,
            "name": id,
            "icon": "robot",
            "provider": "claude",
            "description": "test agent",
            "working_directory": working_directory,
            "created_at": 1,
        }))
        .unwrap();
        state.wstore.agent_def_insert(&mut def).unwrap();
        state
            .wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: id.to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", config_dir.display()),
                updated_at: 0,
            })
            .unwrap();
    }

    fn make_bundle(state: &AppState, id: &str, instructions: &str) -> crate::backend::storage::store::Memory {
        let bundle = crate::backend::storage::store::Memory {
            id: id.to_string(),
            name: format!("Bundle {id}"),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: instructions.to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        state.id_store.bundle_memory_upsert(&bundle).unwrap();
        bundle
    }

    #[tokio::test]
    async fn export_for_agent_includes_normal_components_and_native_memory() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        // Written directly to the live FS, bypassing the mirror entirely —
        // proves the export path's own refresh (not a pre-existing mirror
        // row) is what picks this up.
        let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("MEMORY.md"), "Learned fact.").unwrap();

        let result = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        }).await.unwrap();

        let files = result["files"].as_array().unwrap();
        assert!(files.iter().any(|f| f["path"] == "instructions/AGENTS.md" && f["content"] == "Be helpful."));
        let memory_file = files.iter().find(|f| f["path"] == "memory/MEMORY.md")
            .expect("expected memory/MEMORY.md in the export — live-FS refresh must have picked it up");
        assert_eq!(memory_file["content"], "Learned fact.");

        let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
        assert_eq!(manifest["components"]["memory"], json!(["memory/MEMORY.md"]));

        // The refresh must also have durably mirrored it, not just read it
        // for this one export.
        assert_eq!(
            state.id_store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(),
            Some("Learned fact.".to_string())
        );
    }

    #[tokio::test]
    async fn export_for_agent_omits_memory_component_when_agent_has_none() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        let result = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        }).await.unwrap();

        let files = result["files"].as_array().unwrap();
        assert!(!files.iter().any(|f| f["path"].as_str().unwrap_or("").starts_with("memory/")));
        let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
        assert!(manifest["components"].get("memory").is_none());
    }

    #[tokio::test]
    async fn export_for_agent_errors_for_an_unknown_bundle_or_agent() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        let err = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "no-such-bundle".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        }).await.unwrap_err();
        assert!(err.contains("no bundle"));

        let err = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "no-such-agent".to_string(),
            format: String::new(),
        }).await.unwrap_err();
        assert!(err.contains("no agent"));
    }

    fn abf_files_with_memory(instructions: &str, memory_filename: &str, memory_content: &str) -> Vec<FileEntry> {
        let manifest = serde_json::json!({
            "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
            "name": "imported-bundle",
            "version": "0.1.0",
            "description": "",
            "components": {
                "instructions": { "default": ["instructions/AGENTS.md"] },
                "memory": [format!("memory/{memory_filename}")],
            },
            "metadata": {},
        });
        vec![
            FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
            FileEntry { path: "instructions/AGENTS.md".to_string(), content: instructions.to_string() },
            FileEntry { path: format!("memory/{memory_filename}"), content: memory_content.to_string() },
        ]
    }

    #[tokio::test]
    async fn import_for_agent_rejects_a_target_with_existing_memory() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        state.id_store.agent_native_memory_upsert("agent-1", "MEMORY.md", "already here", None, "/x", 5, 0).unwrap();

        let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Imported fact.");
        let err = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap_err();
        assert!(err.contains("already has"));

        // Rejected up front — the pre-existing row must survive untouched.
        assert_eq!(
            state.id_store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(),
            Some("already here".to_string())
        );
    }

    #[tokio::test]
    async fn import_for_agent_rejects_a_target_with_an_unmirrored_live_memory_file() {
        // reagent P0, PR #2527: a file written directly to the live FS
        // (never viewed through Stash, so never mirrored) must still be
        // detected by the "zero existing memory" guard -- otherwise the
        // write loop would silently overwrite it via fs::rename.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(memory_dir.join("MEMORY.md"), "Never mirrored, but real.").unwrap();
        // Confirm the premise: nothing in the mirror yet.
        assert!(state.id_store.agent_native_memory_list_meta("agent-1").unwrap().is_empty());

        let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Would-be overwrite.");
        let err = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap_err();
        assert!(err.contains("already has"));

        // The live file must survive untouched.
        assert_eq!(
            std::fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap(),
            "Never mirrored, but real."
        );
    }

    #[tokio::test]
    async fn import_for_agent_fails_fast_when_agent_has_no_working_directory_but_bundle_has_memory() {
        // reagent P2, PR #2527: must fail BEFORE creating any skill/bundle
        // rows, not silently succeed with memory_files_written: 0 -- this
        // RPC exists specifically to transfer memory.
        let state = test_state();
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "agent-no-workdir",
            "slug": "agent-no-workdir",
            "name": "agent-no-workdir",
            "icon": "robot",
            "provider": "claude",
            "description": "test agent",
            "working_directory": "",
            "created_at": 1,
        }))
        .unwrap();
        state.wstore.agent_def_insert(&mut def).unwrap();

        let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Some fact.");
        let err = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-no-workdir".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap_err();
        assert!(err.contains("no working directory"));

        // Nothing should have been created.
        assert!(state.id_store.bundle_memory_list().unwrap().iter().all(|b| b.is_blank));
    }

    #[tokio::test]
    async fn export_for_agent_warns_when_a_memory_file_is_truncated() {
        // reagent P2, PR #2527: a file over the native-memory size cap
        // must be flagged, not silently exported partial with no signal.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        // One byte over the 10 MiB cap.
        let oversized = "x".repeat(10 * 1024 * 1024 + 1);
        std::fs::write(memory_dir.join("MEMORY.md"), &oversized).unwrap();

        let result = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        }).await.unwrap();

        let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(
            warnings.iter().any(|w| w.contains("MEMORY.md") && w.contains("truncated")),
            "expected a truncation warning, got: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn import_for_agent_writes_memory_to_both_live_fs_and_mirror() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Imported fact.");
        let result = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap();

        assert_eq!(result["memory_files_written"], 1);
        assert!(result["bundle_id"].as_str().unwrap().len() > 0);

        // Live FS.
        let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
        let on_disk = std::fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap();
        assert_eq!(on_disk, "Imported fact.");

        // Mirror.
        assert_eq!(
            state.id_store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(),
            Some("Imported fact.".to_string())
        );

        // The bundle row itself was also created.
        let bundle_id = result["bundle_id"].as_str().unwrap();
        let bundle = state.id_store.bundle_memory_get(bundle_id).unwrap().unwrap();
        assert_eq!(bundle.instructions, "Be helpful.");
    }

    #[tokio::test]
    async fn import_for_agent_does_not_falsely_claim_memory_was_ignored() {
        // reagent P1, PR #2527: parse_bundle_import's "components.memory:
        // present but ignored" warning is correct for bundle.import/.
        // preview/.commit, but bundle_import_for_agent_impl DOES handle
        // memory (as this same test's success asserts) — it must not
        // also carry that misleading warning into its own response.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Imported fact.");
        let result = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap();

        assert_eq!(result["memory_files_written"], 1);
        let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(
            !warnings.iter().any(|w| w.contains("present but ignored")),
            "memory was actually processed; the 'ignored' warning must not appear: {warnings:?}"
        );
    }

    #[test]
    fn bundle_import_for_agent_lock_is_keyed_by_agent_id() {
        // Direct test of the lock mechanism itself, since the join-based
        // test below can't rigorously prove the lock (as opposed to
        // incidental single-threaded-runtime serialization) is what
        // makes concurrent calls behave — bundle_import_for_agent_impl's
        // body has no other .await points, so it already serializes on a
        // CURRENT_THREAD runtime regardless of the lock. Production runs
        // multi-threaded, where that incidental serialization doesn't
        // apply and the lock is load-bearing.
        let lock_a1 = bundle_import_for_agent_lock("agent-1");
        let lock_a2 = bundle_import_for_agent_lock("agent-1");
        assert!(Arc::ptr_eq(&lock_a1, &lock_a2), "the same agent_id must return the same lock instance");

        let lock_b = bundle_import_for_agent_lock("agent-2");
        assert!(!Arc::ptr_eq(&lock_a1, &lock_b), "different agent_ids must not share a lock");
    }

    #[tokio::test]
    async fn concurrent_imports_for_the_same_agent_do_not_both_succeed() {
        // reagent P2, PR #2527: without the per-agent lock, two concurrent
        // bundle.import_for_agent calls for the same agent could both
        // pass the "zero existing rows" check before either writes. The
        // lock serializes them fully — one must complete (and its memory
        // row must exist) before the other's own zero-rows check runs,
        // so the second is guaranteed to see the first's write and fail.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        let files_a = abf_files_with_memory("A.", "MEMORY.md", "From import A.");
        let files_b = abf_files_with_memory("B.", "MEMORY.md", "From import B.");

        let (result_a, result_b) = tokio::join!(
            bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
                agent_id: "agent-1".to_string(), file_path: None, zip_base64: None, files: Some(files_a),
            }),
            bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
                agent_id: "agent-1".to_string(), file_path: None, zip_base64: None, files: Some(files_b),
            }),
        );

        let outcomes = [result_a.is_ok(), result_b.is_ok()];
        assert_eq!(
            outcomes.iter().filter(|ok| **ok).count(),
            1,
            "exactly one of two concurrent imports for the same agent must succeed, got {outcomes:?}"
        );
        if let Err(e) = if result_a.is_err() { &result_a } else { &result_b } {
            assert!(e.contains("already has"), "the losing import must fail with the existing-memory guard, got: {e}");
        }
    }

    #[tokio::test]
    async fn import_for_agent_rejects_an_unsafe_memory_filename() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        // A manifest referencing a path outside memory/ conventions —
        // validate_memory_filename must reject it, not write it.
        let manifest = serde_json::json!({
            "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
            "name": "imported-bundle",
            "version": "0.1.0",
            "description": "",
            "components": { "memory": ["memory/../escape.md"] },
            "metadata": {},
        });
        let files = vec![
            FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
            FileEntry { path: "memory/../escape.md".to_string(), content: "malicious".to_string() },
        ];
        let result = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap();
        assert_eq!(result["memory_files_written"], 0);
    }

    #[tokio::test]
    async fn round_trip_export_then_import_into_a_fresh_agent_preserves_memory() {
        let state = test_state();
        let config_a = tempfile::tempdir().unwrap();
        let config_b = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-a", "/work/a", config_a.path());
        make_agent(&state, "agent-b", "/work/b", config_b.path());
        make_bundle(&state, "bundle-src", "Shared instructions.");

        let memory_dir_a = config_a.path().join("projects").join("-work-a").join("memory");
        std::fs::create_dir_all(&memory_dir_a).unwrap();
        std::fs::write(memory_dir_a.join("MEMORY.md"), "Agent A's learned fact.").unwrap();

        let exported = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-src".to_string(),
            agent_id: "agent-a".to_string(),
            format: String::new(),
        }).await.unwrap();

        let files: Vec<FileEntry> = exported["files"].as_array().unwrap().iter()
            .map(|f| FileEntry {
                path: f["path"].as_str().unwrap().to_string(),
                content: f["content"].as_str().unwrap().to_string(),
            })
            .collect();

        let imported = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-b".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap();
        assert_eq!(imported["memory_files_written"], 1);

        assert_eq!(
            state.id_store.agent_native_memory_read("agent-b", "MEMORY.md").unwrap(),
            Some("Agent A's learned fact.".to_string())
        );
        let memory_dir_b = config_b.path().join("projects").join("-work-b").join("memory");
        assert_eq!(
            std::fs::read_to_string(memory_dir_b.join("MEMORY.md")).unwrap(),
            "Agent A's learned fact."
        );
    }
}
