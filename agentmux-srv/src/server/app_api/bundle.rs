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
    register_bundle_export_for_agent_with_history(engine, state);
    register_bundle_import_for_agent(engine, state);
    register_bundle_validate(engine, state);
    register_agent_project_instructions(engine, state);
}

/// Raw `[{path, content}]` entry shape, shared by `bundle.import`'s `files`
/// input and the Phase 3 `bundle.import.preview`/`.commit` handlers.
#[derive(serde::Deserialize, Default)]
struct FileEntry {
    path: String,
    content: String,
}

/// Normalize a `bundle.upsert` request body into the shape the `Bundle` struct
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

// The deprecated `preset.*` aliases (Phase 2's one-release compat window)
// were retired as part of the Armory Bundle Format (ABF) UI-alignment pass —
// `bundle.*` is the only surface now; see the ABF branding sweep this
// change shipped alongside.

fn register_bundle_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |_data, _ctx| {
            let state = state.clone();
            Box::pin(async move { Ok(Some(bundle_list_impl(&state).await?)) })
        })
    };
    engine.register_handler(COMMAND_BUNDLE_LIST, make(state.clone()));
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
}

fn register_bundle_validate(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let handler: crate::backend::rpc::engine::CommandHandler = Box::new(move |data, _ctx| {
        let wstore = wstore.clone();
        Box::pin(async move { Ok(Some(bundle_validate_impl(&wstore, data)?)) })
    });
    engine.register_handler(COMMAND_BUNDLE_VALIDATE, handler);
}

/// Reject a `bundle.upsert` write that would change `provider`/`model` on
/// an existing bundle that already has them set —
/// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.2. These are
/// readonly-once-set: an ABF's portability guarantee (it's self-describing
/// about what it needs to run) depends on them never silently changing
/// after creation, and the UI-only disabling in `memory-manager.tsx` is
/// advisory, not a guarantee — this is the actual enforcement.
///
/// `existing = None` (fresh insert) or an existing row with `provider`/
/// `model` still empty (legacy row awaiting backfill, or was raced to
/// create without them) always allows the write — that's how a bundle's
/// provider/model get set the FIRST time. Once non-empty, they're locked.
///
/// Pure — no I/O — directly unit-testable without spinning up an
/// `AppState`, mirroring `agent_open.rs`'s `resolve_vendor_env_override`.
fn check_provider_model_immutable(existing: Option<&Bundle>, incoming: &Bundle) -> Result<(), String> {
    let Some(existing) = existing else { return Ok(()) };
    if !existing.provider.is_empty() && existing.provider != incoming.provider {
        return Err(format!(
            "FORBIDDEN: bundle {} provider is readonly once set (has '{}', got '{}')",
            existing.id, existing.provider, incoming.provider
        ));
    }
    if !existing.model.is_empty() && existing.model != incoming.model {
        return Err(format!(
            "FORBIDDEN: bundle {} model is readonly once set (has '{}', got '{}')",
            existing.id, existing.model, incoming.model
        ));
    }
    Ok(())
}

#[cfg(test)]
mod check_provider_model_immutable_tests {
    use super::*;

    fn memory(id: &str, provider: &str, model: &str) -> Bundle {
        Bundle {
            id: id.to_string(),
            name: "T".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: provider.to_string(),
            model: model.to_string(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        }
    }

    #[test]
    fn allows_a_fresh_insert_to_set_provider_and_model() {
        let incoming = memory("b1", "claude", "anthropic");
        assert!(check_provider_model_immutable(None, &incoming).is_ok());
    }

    #[test]
    fn allows_setting_provider_and_model_the_first_time_on_an_existing_empty_row() {
        // Legacy row awaiting backfill, or a bundle created before this
        // field existed — first write that populates them must succeed.
        let existing = memory("b1", "", "");
        let incoming = memory("b1", "claude", "anthropic");
        assert!(check_provider_model_immutable(Some(&existing), &incoming).is_ok());
    }

    #[test]
    fn allows_an_unrelated_field_edit_that_resends_the_same_provider_and_model() {
        let existing = memory("b1", "claude", "anthropic");
        let incoming = memory("b1", "claude", "anthropic");
        assert!(check_provider_model_immutable(Some(&existing), &incoming).is_ok());
    }

    #[test]
    fn rejects_changing_provider_once_set() {
        let existing = memory("b1", "claude", "anthropic");
        let incoming = memory("b1", "codex", "anthropic");
        let err = check_provider_model_immutable(Some(&existing), &incoming).unwrap_err();
        assert!(err.contains("FORBIDDEN"));
        assert!(err.contains("provider"));
    }

    #[test]
    fn rejects_changing_model_once_set() {
        let existing = memory("b1", "claude", "anthropic");
        let incoming = memory("b1", "claude", "custom");
        let err = check_provider_model_immutable(Some(&existing), &incoming).unwrap_err();
        assert!(err.contains("FORBIDDEN"));
        assert!(err.contains("model"));
    }
}

fn register_bundle_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: &AppState| -> crate::backend::rpc::engine::CommandHandler {
        let id_store = state.id_store.clone();
        let broker = state.broker.clone();
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Bundle =
                    serde_json::from_value(normalize_bundle_upsert_input(data))
                        .map_err(|e| format!("bundle.upsert: {e}"))?;

                // S4: guard on the target id, not the caller-supplied is_blank flag
                // (which defaults false and can be omitted to bypass is_blank check).
                if memory.id == "blank" || memory.id.starts_with("seed-") || memory.is_blank {
                    return Err("FORBIDDEN: cannot mutate a protected bundle".to_string());
                }
                // Guard existing global bundles: an agent must not be able to demote or
                // corrupt a shared global bundle it doesn't own by supplying its id.
                if !memory.id.is_empty() {
                    if let Some(existing) = id_store.bundle_get(&memory.id)
                        .map_err(|e| format!("bundle.upsert: {e}"))?
                    {
                        if existing.is_global {
                            return Err("FORBIDDEN: cannot mutate a global bundle".to_string());
                        }
                        // provider/model are readonly once set — the actual
                        // enforcement behind the portability guarantee (the
                        // bundle editor's disabled-input UI is advisory
                        // only). See check_provider_model_immutable's doc
                        // comment.
                        check_provider_model_immutable(Some(&existing), &memory)?;
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

                id_store.bundle_upsert(&memory)
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
}

fn register_bundle_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: &AppState| -> crate::backend::rpc::engine::CommandHandler {
        let id_store = state.id_store.clone();
        let wstore = state.wstore.clone();
        let broker = state.broker.clone();
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.delete: {e}"))?;

                if req.id == "blank" {
                    return Err("FORBIDDEN: cannot delete a seeded bundle".to_string());
                }

                match id_store.bundle_delete(&req.id) {
                    Ok(deleted) => {
                        if deleted {
                            purge_bundle_component_refs(&wstore, &req.id);
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
}

/// Resolve the agent definition behind an S1-authenticated `agent_id`.
///
/// **`check_s1` authenticates a SLUG, not a UUID.** `RpcContext.agent_id` is
/// "slug of the authenticated agent from bus:register"
/// (`rpc_types/misc.rs:221`), while `agent_def_get` queries `db_agents` by its
/// UUID primary key — so calling it with the authenticated value returns
/// `None` for every real caller (ReAgent P0, PR #3156).
///
/// Delegates to `resolve_agent_definition_id`, the existing resolver for
/// exactly this, rather than hand-rolling the lookup: it already carries the
/// registry-only fallback a previous review round forced in, for a live agent
/// that never created a local `db_agents` row. A fresh two-tier lookup here
/// would have that hole again.
fn resolve_agent_for_s1(
    state: &AppState,
    agent_id: &str,
) -> Result<crate::backend::storage::AgentDefinition, String> {
    let def_id = resolve_agent_definition_id(state, agent_id)?;
    state
        .wstore
        .agent_def_get(&def_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no agent with slug or id {agent_id}"))
}

/// `agent.project_instructions` — what this agent will actually read.
///
/// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Until
/// now nothing could answer this: AgentMux knew what it wrote and recorded a
/// single boolean about anything it found already there (§2.4).
///
/// Agent-scoped, so `check_s1` applies — an agent's working directory contents
/// are not window-scoped data the way a bundle is.
///
/// **Read-only, and there is no write counterpart on purpose.** A foreign
/// instruction file belongs to the repository;
/// `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists to keep AgentMux
/// out of it, and this must not become a way around that.
fn register_agent_project_instructions(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let make = |state: AppState| -> crate::backend::rpc::engine::CommandHandler {
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req {
                    agent_id: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("agent.project_instructions: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;

                let agent = resolve_agent_for_s1(&state, &req.agent_id)
                    .map_err(|e| format!("agent.project_instructions: {e}"))?;

                // The directory the agent will actually run in: a blank
                // working_directory means the per-agent default and a `~`
                // path is expanded at launch, so the stored value alone
                // describes a directory the agent never uses (Codex, #3156).
                let work_dir = crate::backend::project_instructions::effective_working_dir(
                    &agent.working_directory,
                    &agent.name,
                );
                let files = crate::backend::project_instructions::resolve_project_instructions(
                    &agent.provider,
                    &work_dir,
                );

                // Compare against what was recorded at the last launch, so a
                // repository file that changed underneath is visible rather
                // than merely present. Observations are written at
                // `agent.open` (`observe_project_instructions`) — this RPC
                // stays a pure read, which is why a file edited since the last
                // launch reports `modified` every time it is called until the
                // agent is next opened.
                let previous = state
                    .wstore
                    .project_instructions_list(&agent.id)
                    .unwrap_or_default();
                let mut enriched: Vec<serde_json::Value> = files
                    .iter()
                    .map(|f| {
                        let prev = previous.iter().find(|p| p.path == f.path);
                        let change = crate::backend::storage::project_instructions::classify_change(
                            prev,
                            f.exists,
                            &f.content_hash,
                        );
                        let mut v = serde_json::to_value(f).unwrap_or_default();
                        if let Some(obj) = v.as_object_mut() {
                            obj.insert("change".to_string(), json!(change));
                            obj.insert(
                                "last_observed_at".to_string(),
                                json!(prev.map(|p| p.observed_at)),
                            );
                        }
                        v
                    })
                    .collect();

                // A path the resolver no longer returns at all — a scanned
                // `.github/instructions/*.instructions.md` deleted since the
                // last launch — would otherwise just vanish from the response,
                // never reported as `removed`, and recreating it later would
                // read as `unchanged` against the stale row (Codex, PR #3162).
                // The declared paths always come back (absent ones included),
                // so this only ever fires for dynamically scanned files.
                for prev in previous.iter().filter(|p| !files.iter().any(|f| f.path == p.path)) {
                    let change = crate::backend::storage::project_instructions::classify_change(
                        Some(prev),
                        false,
                        "",
                    );
                    enriched.push(json!({
                        "path": prev.path,
                        "exists": false,
                        "size_bytes": 0,
                        "content_hash": "",
                        "owner": prev.owner,
                        "content": "",
                        "truncated": false,
                        "error": serde_json::Value::Null,
                        "change": change,
                        "last_observed_at": prev.observed_at,
                    }));
                }

                Ok(Some(json!({
                    "provider": agent.provider,
                    "working_directory": work_dir,
                    "files": enriched,
                })))
            })
        })
    };
    engine.register_handler(COMMAND_AGENT_PROJECT_INSTRUCTIONS, make(state.clone()));
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
#[derive(serde::Deserialize, Default)]
struct ExportReq {
    id: String,
    #[serde(default)]
    format: String,
}

fn register_bundle_export(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                let req: ExportReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export: {e}"))?;
                bundle_export_impl(&id_store, &wstore, req).map(Some)
            })
        }),
    );
}

/// The agent-less `bundle.export`, extracted from its handler closure so the
/// warning contract in §5.1 can be asserted at the RPC boundary rather than
/// only on its helper — matching `bundle_export_for_agent_impl` and
/// `bundle_export_for_agent_with_history_impl`, which were already shaped this
/// way for the same reason.
fn bundle_export_impl(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    req: ExportReq,
) -> Result<serde_json::Value, String> {
    let bundle = id_store
        .bundle_get(&req.id)
        .map_err(|e| format!("bundle.export: {e}"))?
        .ok_or_else(|| format!("bundle.export: no bundle with id {}", req.id))?;

    // Components come from the ref tables, which are authoritative for what a
    // bundle contains -- see resolve_bundle_components.
    let components = resolve_bundle_components(wstore, &bundle.id)
        .map_err(|e| format!("bundle.export: {e}"))?;
    let mut handler_warnings = components.warnings;

    // `missing_skill_ids` predates the ref tables: the inline column stored
    // bare ids, so an id whose skill row had been deleted had to be resolved
    // and reported (Codex P2, PR #2325). A ref cannot be dangling the same
    // way -- `managed_list` joins through the catalog, and `skill_delete`
    // purges refs -- so this is now always empty. Kept in the response because
    // it is part of the RPC's shape and callers may still read it.
    let missing_skill_ids: Vec<String> = Vec::new();

    let export = crate::backend::bundle_export::export_bundle(
        &bundle,
        &components.skills,
        &components.mcp_entries,
    );

    let mut all_warnings = export.warnings.clone();
    all_warnings.append(&mut handler_warnings);

    // Export/import symmetry -- see MEMORY_NOT_EXPORTED_WARNING.
    if bound_agent_has_native_memory(id_store, wstore, &bundle.id) {
        all_warnings.push(MEMORY_NOT_EXPORTED_WARNING.to_string());
    }

    if req.format == "zip" {
        let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
            .map_err(|e| format!("bundle.export: {e}"))?;
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

/// Drop a deleted bundle's component refs.
///
/// Called from **both** delete paths — `bundle.delete` here and `deletememory`
/// in `agent_handlers/bundle.rs`, which is the one the Armory actually calls.
/// Fixing only the first left the product path orphaning refs (Codex, PR
/// #3153), so this exists to make "both" mean one implementation.
///
/// `bundle_delete` runs on the store that owns `db_bundles`, which has no ref
/// tables — they sit beside the catalog tables they key into, in the other
/// database, which is also why no foreign key reaches across
/// (`migrations.rs:721-741`). So the purge cannot live inside it and has to be
/// driven from a handler, where both stores are in scope.
///
/// Best-effort by design: the bundle row is already gone by the time this
/// runs, so a failure here must not turn a successful delete into an error.
pub(crate) fn purge_bundle_component_refs(
    wstore: &crate::backend::storage::store::Store,
    bundle_id: &str,
) {
    match wstore.bundle_unbind_all_components(bundle_id) {
        Ok((skills, mcp)) if skills + mcp > 0 => {
            tracing::info!(bundle_id, skills, mcp, "bundle delete: purged component refs")
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(
            bundle_id, error = %e,
            "bundle delete: component refs left behind"
        ),
    }
}

/// Bind an imported bundle's components into the ref tables.
///
/// Phase 0b. The ref tables are authoritative for what a bundle contains, so
/// an import that only wrote the inline columns would produce a bundle that
/// exports empty and, for MCP servers, never materialises at spawn either —
/// the `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` "inert at runtime" state
/// the ref tables exist to end.
///
/// **Must run after `bundle_upsert`.** Both bind paths check the bundle exists
/// in `id_store` first (`storage/managed.rs:291`) and refuse otherwise.
///
/// Returns warnings rather than failing: by this point the bundle row is
/// already committed, so aborting would leave a half-imported bundle behind. A
/// component that could not be bound is reported and the import continues.
fn bind_imported_components(
    wstore: &crate::backend::storage::store::Store,
    id_store: &crate::backend::storage::store::Store,
    bundle_id: &str,
    imported_skill_ids: &[String],
    mcp_configs: &[serde_json::Value],
) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();

    for skill_id in imported_skill_ids {
        if let Err(e) = wstore.bundle_skill_bind(id_store, bundle_id, skill_id) {
            warnings.push(format!(
                "skills: imported skill {skill_id} could not be bound to the bundle ({e}) — it will not be exported or reach the agent"
            ));
        }
    }

    // Same shared path the m0030 backfill uses, so a server that arrives by
    // import and one recovered from an old inline column end up identical,
    // including duplicate-name handling.
    let (_created, mcp_warnings) =
        wstore.bundle_mcp_bind_inline_entries(id_store, bundle_id, mcp_configs, now_ms());
    warnings.extend(mcp_warnings);

    warnings
}

/// A bundle's components, resolved from the tables that are authoritative for
/// them.
pub(super) struct ResolvedComponents {
    pub(super) skills: Vec<crate::backend::storage::Skill>,
    /// Exporter entry shape: each server's config object with its `name`
    /// alongside, which is what `bundle_export::redact_mcp_entry` reads.
    pub(super) mcp_entries: Vec<serde_json::Value>,
    pub(super) warnings: Vec<String>,
}

/// Resolve what a bundle actually contains, from `db_bundle_skills_ref` /
/// `db_bundle_mcp_ref`.
///
/// Phase 0b of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Before
/// this, export read the inline `bundle.skills` / `bundle.mcp_servers` columns
/// while agent launch read the ref tables (`app_api/agent_open.rs:793`, `:812`),
/// and nothing kept the two in step — so a skill bound in the Armory ran at
/// launch but exported as an empty `skills/` directory, with no warning. One
/// resolver, used by every export path, is what stops that recurring.
///
/// **Scope note:** this is deliberately narrower than `effective_skills` /
/// `effective_mcp_servers`, which additionally union an *agent's* own binds,
/// globals and the legacy blob. Those answer "what does this agent run with";
/// this answers "what is in this bundle", which is what an ABF describes. The
/// two are not meant to be equal, only to agree about the bundle's own
/// contribution.
///
/// `managed_list` returns the whole catalog with a `bound_to_bundle` flag
/// rather than just the bound rows, hence the filter; and because it joins
/// through the catalog table, a ref whose catalog row has been deleted
/// resolves to nothing rather than reporting itself. `skill_delete` /
/// `mcp_server_delete` both purge refs (`storage/managed.rs:232`), so that
/// state is not reachable through the app — but the FK that would enforce it
/// is inert, since `PRAGMA foreign_keys` is only ever set ON in tests.
pub(super) fn resolve_bundle_components(
    wstore: &crate::backend::storage::store::Store,
    bundle_id: &str,
) -> Result<ResolvedComponents, String> {
    let skills: Vec<crate::backend::storage::Skill> = wstore
        .bundle_skill_list(bundle_id)
        .map_err(|e| format!("failed to resolve bundle skills: {e}"))?
        .into_iter()
        .filter(|item| item.bound_to_bundle)
        .map(|item| item.skill)
        .collect();

    let mut warnings: Vec<String> = Vec::new();
    let mut mcp_entries: Vec<serde_json::Value> = Vec::new();
    for item in wstore
        .bundle_mcp_list(bundle_id)
        .map_err(|e| format!("failed to resolve bundle MCP servers: {e}"))?
        .into_iter()
        .filter(|item| item.bound_to_bundle)
    {
        let server = item.server;
        // `config` is the same JSON object the launch path writes into
        // `.mcp.json` keyed by `server.name` (`agent_config.rs`
        // build_mcp_config_from_refs). The exporter wants that object with the
        // name inline, so the row's name is authoritative over any `name` the
        // config blob happens to carry.
        match serde_json::from_str::<serde_json::Value>(&server.config) {
            Ok(serde_json::Value::Object(mut obj)) => {
                obj.insert("name".to_string(), json!(server.name));
                mcp_entries.push(serde_json::Value::Object(obj));
            }
            Ok(_) => warnings.push(format!(
                "mcp server {}: config is not a JSON object — skipped",
                server.name
            )),
            Err(e) => warnings.push(format!(
                "mcp server {}: invalid config JSON ({e}) — skipped",
                server.name
            )),
        }
    }

    Ok(ResolvedComponents { skills, mcp_entries, warnings })
}

/// Warning the agent-less `bundle.export` pushes when the bundle it is
/// exporting is bound to an agent that actually has native memory.
///
/// Mirror of [`crate::backend::bundle_import::MEMORY_COMPONENT_IGNORED_WARNING`]
/// on the import side, and a constant for the same reason that one is: the push
/// site and the test that pins the wording must not drift apart.
///
/// `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §3.1/§5.1. The
/// asymmetry worth fixing was never that `bundle.export` omits memory — a
/// bundle detached from any agent has no memory to carry, and omitting it is
/// correct. It is that import *announces* the omission and export did not,
/// while every `components` key is optional (`bundle_export.rs:586-606`), so a
/// missing `memory` key is indistinguishable from "this bundle had none".
pub(crate) const MEMORY_NOT_EXPORTED_WARNING: &str =
    "memory: this bundle is bound to an agent with native memory, which bundle.export does not carry — use bundle.export_for_agent to include it";

/// Does any agent bound to `bundle_id` have mirrored native memory?
///
/// **Both bindings count.** A bundle is reachable two ways, and they are
/// different columns: `AgentDefinition.memory_id` is the agent's own dedicated
/// bundle (stored in `db_agents.default_memory_id`), while
/// `AgentInstance.memory_id` is one *launch* deliberately pointed at some other
/// bundle (`storage/agents.rs:153-163`). Checking only the definition would
/// miss exactly the case the operator is most likely to hit — exporting the
/// bundle a running instance was launched with. Native memory is keyed on the
/// definition id either way (`build_export_for_agent` resolves the agent with
/// `agent_def_get` before reading memory), so the instance path resolves
/// through `definition_id`.
///
/// Reads the `db_agent_native_memory` mirror and deliberately does NOT call
/// `refresh_memory_mirror_from_live_fs` first, unlike `build_export_for_agent`
/// (which must, because it is about to *copy* the files —
/// `app_api/bundle.rs:558`). Refreshing writes to the store, and an agent-less
/// export has no business mutating per-agent state as a side effect of
/// producing a warning.
///
/// **Known limitation — a cross-channel binding is invisible here, and cannot
/// currently be made visible.** Both lookups resolve the binding from
/// channel-local SQLite: `instance_list` reads this channel's rows, and
/// `agent_def_list`'s global overlay only preserves `memory_id` when a local
/// row exists (`storage/agents.rs:462`). An agent created in another channel
/// has no local row, so it comes back with an empty `memory_id` — because
/// `DefinitionRecordV1` does not carry the field at all
/// (`storage/def_registry_mirror.rs:128-133`). The bundle and the memory mirror
/// are global; only the binding between them is not. So exporting a shared
/// bundle from a channel other than the one its agent was created in will not
/// warn, which is exactly the portability case the warning is for (Codex, PR
/// #3147).
///
/// Closing it means adding `memory_id` to the registry wire format, which is
/// already a tracked gap for bigger reasons than this warning — the same
/// missing field makes a cross-channel reopen start unbound and limits m0021's
/// backfill to local SQLite. Tracked in #3148; deliberately not widened here,
/// because warning whenever the binding is merely *unknown* would fire on
/// unrelated bundles and train operators to ignore it.
///
/// The other cost is under-warning on a stale mirror: an agent whose memory
/// exists on disk but was never mirrored is missed. That is the residual gap
/// `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` already documents and
/// accepts for the mirror generally. Both costs are one-directional and this
/// warning is advisory — it changes no bytes in the archive.
fn bound_agent_has_native_memory(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    bundle_id: &str,
) -> bool {
    // A blank bundle_id is the "blank singleton" sentinel that every unbound
    // agent carries — treating it as a binding would warn on nearly every
    // export.
    if bundle_id.is_empty() {
        return false;
    }

    let has_memory = |agent_id: &str| {
        id_store
            .agent_native_memory_list_meta(agent_id)
            .map(|rows| !rows.is_empty())
            .unwrap_or(false)
    };

    // Best-effort throughout: a store error here must not fail an export that
    // is otherwise fine. Neither table is read by the export itself.
    let bound_by_definition = wstore
        .agent_def_list()
        .map(|agents| {
            agents
                .iter()
                .filter(|a| a.memory_id == bundle_id)
                .any(|a| has_memory(&a.id))
        })
        .unwrap_or(false);
    if bound_by_definition {
        return true;
    }

    wstore
        .instance_list(None, None)
        .map(|instances| {
            instances
                .iter()
                .filter(|i| i.memory_id == bundle_id)
                .any(|i| has_memory(&i.definition_id))
        })
        .unwrap_or(false)
}

/// Register `entries` under `components.<key>` in the export's `armory.json`.
///
/// Takes JSON values rather than paths: most components are a list of archive
/// paths, but `projectInstructions` carries an object per file, because a bare
/// path could not express the `owner` discriminator that keeps import from
/// installing somebody else's instructions.
///
/// The manifest is built as an inline `json!` literal in `bundle_export.rs`
/// with no struct behind it (`bundle_export.rs:615`), so every agent-aware
/// component has to reopen and rewrite it. Shared by the memory splice below
/// and the history splice, which otherwise duplicated the find-parse-rewrite
/// dance byte for byte.
///
/// No-ops on an empty list: `components` keys are all optional, and writing
/// `"history": []` would assert "this archive has an empty history" where
/// absence already says "none".
fn set_manifest_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    key: &str,
    entries: Vec<serde_json::Value>,
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let manifest_idx = export
        .files
        .iter()
        .position(|f| f.path == "armory.json")
        .ok_or_else(|| "bundle export is missing armory.json".to_string())?;
    let mut manifest: serde_json::Value = serde_json::from_str(&export.files[manifest_idx].content)
        .map_err(|e| format!("armory.json: {e}"))?;
    manifest["components"][key] = json!(entries);
    export.files[manifest_idx].content =
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Register the transcript files `bundle.export_for_agent_with_history` already
/// wrote into the archive under `components.history`.
///
/// `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §3.5: the files were
/// pushed into `export.files` (`app_api/bundle.rs:797`) and counted in the RPC
/// response, but the manifest was never touched after the memory splice — so a
/// consumer reading `components.*` to learn what an archive holds could not see
/// history at all. The paths are caller-supplied rather than re-derived here so
/// this cannot drift from what was actually written.
fn splice_history_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    history_paths: Vec<String>,
) -> Result<(), String> {
    set_manifest_component(export, "history", history_paths.into_iter().map(|p| json!(p)).collect())
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
/// Carry the agent's project instructions into the export as
/// `components.projectInstructions`.
///
/// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Files
/// land under `instructions/project/<sanitized-path>`, and each manifest entry
/// carries its original `path`, a `contentHash`, and an `owner`.
///
/// **`owner` is the load-bearing field.** It is what stops this from becoming a
/// way to install one repository's instructions into another: an importer that
/// treated a `foreign` entry as content to write would be doing precisely what
/// `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists to prevent.
/// Import surfaces these and never applies them (spec §5.2), so an operator who
/// wants them acts deliberately.
///
/// Only files that exist and were readable are carried — there is nothing to
/// put in an archive otherwise. Their absence from the manifest is therefore
/// not a claim that the source agent had none; `agent.project_instructions` is
/// where the fuller picture, including absent and unreadable files, lives.
///
/// "Readable" is decided by `error`/`truncated`, NOT by whether the content is
/// empty (Codex P2, PR #3163). A real, readable, zero-byte `CLAUDE.md` is a
/// fact about the source agent — dropping it would make the export
/// indistinguishable from one where the file was absent, which is the exact
/// kind of quiet lie this component exists to remove.
///
/// Paths go through the same sanitizer the exporter uses for context files, so
/// a scanned path can never write outside the archive's own tree.
fn splice_project_instructions_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    files: &[crate::backend::project_instructions::ProjectInstructionFile],
) -> Result<(), String> {
    use crate::backend::project_instructions::InstructionOwner;
    use std::collections::HashSet;

    let mut entries: Vec<serde_json::Value> = Vec::new();
    // Case-INSENSITIVE, for the same reason the context-file loop in
    // `bundle_export.rs` is (reagent P2, PR #2333): a case-sensitive source
    // filesystem can hold `.github/instructions/A.md` and `.../a.md`, and the
    // scan in `project_instructions.rs` will find both — but they collide on
    // extraction on Windows/macOS, where one snapshot silently overwrites the
    // other. Scanned directories make this reachable here in a way it isn't
    // for the fixed registry paths (Codex P2, PR #3163). The written path
    // keeps its original case; only the check is folded.
    let mut used_paths: HashSet<String> = HashSet::new();
    for file in files {
        if !file.exists || file.error.is_some() || file.truncated {
            continue;
        }
        let Some(safe) = crate::backend::bundle_export::sanitize_context_relative_path(&file.path)
        else {
            // Warn rather than dropping in silence — a backup tool that loses
            // an input without saying so defeats its own purpose (reagent P1,
            // PR #2333, on the context-file equivalent).
            export.warnings.push(format!(
                "projectInstructions: \"{}\" could not be sanitized into an \
                 archive-relative path; skipped",
                file.path
            ));
            continue;
        };
        let out_path = format!("instructions/project/{safe}");
        if !used_paths.insert(out_path.to_lowercase()) {
            export.warnings.push(format!(
                "projectInstructions: \"{}\" normalizes to the same path as an \
                 earlier entry ({out_path}); skipped to avoid overwriting it",
                file.path
            ));
            continue;
        }
        export.files.push(crate::backend::bundle_export::BundleExportFile {
            path: out_path.clone(),
            content: file.content.clone(),
        });
        entries.push(json!({
            "path": file.path,
            "file": out_path,
            "contentHash": file.content_hash,
            "owner": match file.owner {
                InstructionOwner::Agentmux => "agentmux",
                InstructionOwner::Foreign => "foreign",
            },
        }));
    }
    set_manifest_component(export, "projectInstructions", entries)
}

fn splice_memory_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    memory_files: &[(String, String)],
) -> Result<(), String> {
    if memory_files.is_empty() {
        return Ok(());
    }
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
    set_manifest_component(export, "memory", manifest_memory.into_iter().map(|p| json!(p)).collect())
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
/// Shared by `bundle_export_for_agent_impl` and
/// `bundle_export_for_agent_with_history_impl`: resolve the bundle + agent,
/// build the base `BundleExport`, splice in native memory. Returns the
/// export, the resolved agent (the history variant needs its id), combined
/// warnings, and missing skill ids. Everything after this point (whether to
/// zip, whether to also append history files) is caller-specific.
async fn build_export_for_agent(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    bundle_id: &str,
    agent_id: &str,
    err_prefix: &str,
) -> Result<(
    crate::backend::bundle_export::BundleExport,
    crate::backend::storage::store::AgentDefinition,
    Vec<String>,
    Vec<String>,
), String> {
    let bundle = id_store
        .bundle_get(bundle_id)
        .map_err(|e| format!("{err_prefix}: {e}"))?
        .ok_or_else(|| format!("{err_prefix}: no bundle with id {bundle_id}"))?;
    let agent = wstore
        .agent_def_get(agent_id)
        .map_err(|e| format!("{err_prefix}: {e}"))?
        .ok_or_else(|| format!("{err_prefix}: no agent with id {agent_id}"))?;

    // Same resolver as the agent-less path, so the two cannot disagree about
    // what the bundle contains.
    let components = resolve_bundle_components(wstore, &bundle.id)
        .map_err(|e| format!("{err_prefix}: {e}"))?;
    let mut handler_warnings = components.warnings;
    // Always empty now — see the note at the agent-less export path.
    let missing_skill_ids: Vec<String> = Vec::new();

    let mut export = crate::backend::bundle_export::export_bundle(
        &bundle,
        &components.skills,
        &components.mcp_entries,
    );

    // Refresh the mirror from the live FS, then read every mirrored
    // file's content — this agent's memory, freshest as of right now,
    // not as of whenever its Stash Bundle tab was last opened.
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
    splice_memory_component(&mut export, &memory_files).map_err(|e| format!("{err_prefix}: {e}"))?;

    // Project instructions — carried as a record of what the source agent was
    // reading, not as content to install. The working directory is resolved
    // the same way launch resolves it, so the export describes the directory
    // the agent actually runs in.
    let work_dir = crate::backend::project_instructions::effective_working_dir(
        &agent.working_directory,
        &agent.name,
    );
    let instructions = crate::backend::project_instructions::resolve_project_instructions(
        &agent.provider,
        &work_dir,
    );
    splice_project_instructions_component(&mut export, &instructions)
        .map_err(|e| format!("{err_prefix}: {e}"))?;

    let mut all_warnings = export.warnings.clone();
    all_warnings.append(&mut handler_warnings);

    Ok((export, agent, all_warnings, missing_skill_ids))
}

async fn bundle_export_for_agent_impl(
    id_store: &crate::backend::storage::store::Store,
    wstore: &crate::backend::storage::store::Store,
    req: ExportForAgentReq,
) -> Result<serde_json::Value, String> {
    let (export, _agent, all_warnings, missing_skill_ids) =
        build_export_for_agent(id_store, wstore, &req.bundle_id, &req.agent_id, "bundle.export_for_agent").await?;

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

#[derive(serde::Deserialize)]
struct ExportForAgentWithHistoryReq {
    bundle_id: String,
    agent_id: String,
}

/// Per-transcript size ceiling for `bundle.export_for_agent_with_history`.
/// reagentx P1 on PR #2613: reading every session fully into a `String`
/// with no bound could exhaust server memory for the multi-GB histories
/// this feature exists to migrate. Unlike `MAX_ABF_FILE_SIZE_BYTES`
/// (context files, truncated with a warning when oversized — fine, since
/// a partial context file is still useful), a transcript is skipped
/// ENTIRELY rather than truncated: JSONL requires whole lines, and a
/// truncated mid-record file could fail to parse (or silently misparse)
/// on the receiving end, which is worse than the session being visibly
/// absent with a warning explaining why.
const MAX_HISTORY_SESSION_FILE_SIZE_BYTES: u64 = 200 * 1024 * 1024;

/// Aggregate ceiling across every session included in one
/// `bundle.export_for_agent_with_history` call. reagentx P1, third review
/// round: a per-file cap alone doesn't bound an agent with many sessions
/// each just under it — this is the analogous total cap the sibling
/// import path already enforces (`MAX_TOTAL_UNCOMPRESSED_BYTES`,
/// `bundle_import.rs`), sized larger since export's job is specifically
/// to carry as much real history as reasonably fits, not to validate a
/// small hand-authored bundle. Sessions are processed in
/// `sessions_for_agent`'s existing `modified_at desc` order, so hitting
/// this cap keeps the most recent conversations and cuts off the oldest
/// — the same priority a human moving "my history" to a new machine would
/// want. Unlike import's reject-the-whole-thing-on-overflow behavior,
/// export keeps its existing best-effort philosophy: stop including
/// further sessions and say so in a warning, never fail the call outright.
const MAX_TOTAL_HISTORY_BYTES: u64 = 500 * 1024 * 1024;

/// Given per-session sizes already in priority order (most-recent-first,
/// matching `sessions_for_agent`'s `modified_at desc` sort), decide how
/// many fit within `total_cap`. Returns `(count_included, stopped_at_cap)`.
/// Pure — no I/O — deliberately extracted so this decision is testable
/// with tiny numbers instead of needing real `MAX_TOTAL_HISTORY_BYTES`-
/// scale files on disk.
fn sessions_within_total_budget(sizes: &[u64], total_cap: u64) -> (usize, bool) {
    let mut total = 0u64;
    for (i, &size) in sizes.iter().enumerate() {
        if total + size > total_cap {
            return (i, true);
        }
        total += size;
    }
    (sizes.len(), false)
}

/// `bundle.export_for_agent_with_history` —
/// `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
/// §3.3. Deliberately a SEPARATE, explicit, opt-in action from
/// `bundle.export_for_agent` — see `COMMAND_BUNDLE_EXPORT_FOR_AGENT_WITH_HISTORY`'s
/// doc comment for why conversation transcripts must never ride along in
/// the default (shareable, reusable-across-agents) bundle export. Builds
/// the identical base export, then appends every session
/// `HistoryService::sessions_for_agent` finds for this agent under
/// `history/<provider>/<session_id>.jsonl`. Always returns a zip — a
/// multi-file, potentially large archive has no sensible inline-JSON
/// shape, unlike the plain export's optional format. A transcript that
/// fails to read (deleted, permission issue, or over
/// `MAX_HISTORY_SESSION_FILE_SIZE_BYTES`) is warned about and skipped,
/// never fails the whole export — same best-effort philosophy as the
/// memory-splicing step above.
///
/// reagentx P1 on PR #2613, all addressed:
/// - `sessions_for_agent(..., force_refresh: true)` — this export claims
///   completeness ("included N of N known sessions"), so it can't rely on
///   the interactive lazy-refresh-if-empty behavior; a session created
///   since the index was first populated (by ANY prior call) must not be
///   silently missing.
/// - `sessions_for_agent` itself runs inside its own `spawn_blocking` —
///   `force_refresh: true` means it calls `SessionIndex::refresh()`,
///   which synchronously walks and parses every session file for every
///   provider on disk. A second review round caught that only the LATER
///   file-read/zip/encode step (below) had been moved off the async
///   worker thread; this earlier, more expensive full-index-rebuild step
///   was still running inline on it. Takes `Arc<Store>`/
///   `Arc<HistoryService>` (not bare refs) specifically so a clone can
///   cross into this closure — `build_export_for_agent` above still
///   receives them by reference (`Arc` derefs to `&Store` for free), no
///   signature change needed there.
/// - the actual file reads + zip + base64 encode run inside their own
///   `spawn_blocking` — synchronous, potentially slow I/O and CPU work
///   that must not block the async RPC worker thread either. Only the
///   already-resolved, owned `export`/`sessions` data crosses into that
///   closure.
/// - `MAX_TOTAL_HISTORY_BYTES` — a third review round caught that the
///   per-session cap alone doesn't bound an agent with MANY sessions each
///   just under it; the sibling import path enforces an analogous total
///   cap (`MAX_TOTAL_UNCOMPRESSED_BYTES`). See `sessions_within_total_budget`.
async fn bundle_export_for_agent_with_history_impl(
    id_store: Arc<crate::backend::storage::store::Store>,
    identity_store: Arc<crate::backend::storage::store::Store>,
    wstore: &crate::backend::storage::store::Store,
    history_service: Arc<crate::backend::history::HistoryService>,
    req: ExportForAgentWithHistoryReq,
) -> Result<serde_json::Value, String> {
    let (export, _agent, all_warnings, missing_skill_ids) = build_export_for_agent(
        &id_store,
        wstore,
        &req.bundle_id,
        &req.agent_id,
        "bundle.export_for_agent_with_history",
    )
    .await?;

    let agent_id = req.agent_id.clone();
    let (sessions, session_count, _has_more) = {
        // identity_store, not id_store — agent-identity links now live in
        // the permanently-global identity store
        // (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md); reagentx P1 review on
        // PR #2632. build_export_for_agent above still reads the BUNDLE via
        // id_store — bundle routing isn't part of this fix's scope.
        let identity_store = identity_store.clone();
        let history_service = history_service.clone();
        tokio::task::spawn_blocking(move || {
            history_service.sessions_for_agent(&identity_store, &agent_id, 0, usize::MAX, "modified_at", "desc", true)
        })
        .await
        .map_err(|e| format!("bundle.export_for_agent_with_history: blocking task panicked: {e}"))?
        .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?
    };

    tokio::task::spawn_blocking(move || {
        let mut export = export;
        let mut all_warnings = all_warnings;

        // Pass 1: stat every session, drop per-file-oversized ones. No
        // reads yet — just sizes, so the pure budget decision below (pass
        // 2) can be tested with tiny numbers instead of real files.
        let mut sized: Vec<(&crate::backend::history::adapter::SessionMeta, u64)> = Vec::new();
        for session in &sessions {
            match std::fs::metadata(&session.file_path) {
                Ok(meta) if meta.len() > MAX_HISTORY_SESSION_FILE_SIZE_BYTES => {
                    all_warnings.push(format!(
                        "history: skipped session {} ({}) — {} bytes exceeds the {} byte per-session limit",
                        session.session_id, session.file_path, meta.len(), MAX_HISTORY_SESSION_FILE_SIZE_BYTES
                    ));
                }
                Ok(meta) => sized.push((session, meta.len())),
                Err(e) => all_warnings.push(format!(
                    "history: failed to read session {} ({}): {e}",
                    session.session_id, session.file_path
                )),
            }
        }

        // Pass 2: how many of the (already most-recent-first-ordered)
        // remaining sessions fit within the aggregate cap.
        let sizes: Vec<u64> = sized.iter().map(|(_, size)| *size).collect();
        let sized_count = sized.len();
        let (fit_count, stopped_at_total_cap) = sessions_within_total_budget(&sizes, MAX_TOTAL_HISTORY_BYTES);
        if stopped_at_total_cap {
            let total: u64 = sizes[..fit_count].iter().sum();
            all_warnings.push(format!(
                "history: stopped after {total} bytes (limit {MAX_TOTAL_HISTORY_BYTES}) — {} additional session(s) omitted, oldest first",
                sized_count - fit_count
            ));
        }

        // Pass 3: actually read + include only what fit.
        let mut included = 0u32;
        let mut history_paths: Vec<String> = Vec::new();
        for (session, _size) in sized.into_iter().take(fit_count) {
            match std::fs::read_to_string(&session.file_path) {
                Ok(content) => {
                    let out_path =
                        format!("history/{}/{}.jsonl", session.provider, session.session_id);
                    export.files.push(crate::backend::bundle_export::BundleExportFile {
                        path: out_path.clone(),
                        content,
                    });
                    history_paths.push(out_path);
                    included += 1;
                }
                Err(e) => all_warnings.push(format!(
                    "history: failed to read session {} ({}): {e}",
                    session.session_id, session.file_path
                )),
            }
        }
        all_warnings.push(format!("history: included {included} of {session_count} known session(s)"));

        // §3.5: the files above are in the archive; say so in the manifest too.
        // Must happen before zipping — armory.json is zipped from export.files.
        splice_history_component(&mut export, history_paths)
            .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?;

        let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
            .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?;
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
        Ok(json!({
            "root_slug": export.root_slug,
            "skipped_skills": export.skipped_skills,
            "warnings": all_warnings,
            "missing_skill_ids": missing_skill_ids,
            "history_session_count": included,
            "zip_base64": encoded,
        }))
    })
    .await
    .map_err(|e| format!("bundle.export_for_agent_with_history: blocking task panicked: {e}"))?
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

fn register_bundle_export_for_agent_with_history(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let wstore = state.wstore.clone();
    let history_service = state.history_service.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT_FOR_AGENT_WITH_HISTORY,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let wstore = wstore.clone();
            let history_service = history_service.clone();
            Box::pin(async move {
                let req: ExportForAgentWithHistoryReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?;
                bundle_export_for_agent_with_history_impl(id_store, identity_store, &wstore, history_service, req)
                    .await
                    .map(Some)
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
    // written autonomously but never viewed through the Stash Bundle tab
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
        wstore,
        id_store,
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

                let memory = Bundle {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: parsed.name,
                    description: parsed.description,
                    is_blank: false,
                    // Never imported as global — an import must not
                    // silently start injecting into every agent's
                    // CLAUDE.md without explicit user action (spec §4.4).
                    is_global: false,
                    provider: parsed.provider,
                    model: parsed.model,
                    instructions: parsed.instructions,
                    // ABF v0.2 §2.2: every variant is stored verbatim, no
                    // merge decision made at import time.
                    instructions_by_provider: serde_json::to_string(&parsed.instructions_by_provider)
                        .unwrap_or_else(|_| "{}".to_string()),
                    // Phase 3 spec §3.0: ImportedContextFile gained a
                    // stable `id` selection key alongside `path`/`content`
                    // -- project it away before persisting, matching
                    // `Bundle.context_files`'s existing documented
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
                    // raw MCP config every consumer of `Bundle.mcp_servers`
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
                    is_system: false,
                };
                if let Err(e) = id_store.bundle_upsert(&memory) {
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

                // Ref tables are authoritative — bind after the bundle row
                // exists, or this import exports empty and its servers never
                // reach a spawned agent.
                warnings.extend(bind_imported_components(
                    &wstore,
                    &id_store,
                    &memory.id,
                    &imported_skill_ids,
                    &parsed.mcp_servers.iter().map(|m| m.config.clone()).collect::<Vec<_>>(),
                ));

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
/// Per-entry content cap for `project_instructions` in an import preview.
///
/// Deliberately far below `MAX_INSTRUCTIONS_PREVIEW_CHARS` (50k): that cap
/// applies to ONE field, whereas a preview can carry up to
/// `MAX_IMPORTED_PROJECT_INSTRUCTIONS` (200) of these, so the same number
/// would put a 10 MB ceiling on a single response. 4k is enough to recognize a
/// file and see how it opens; the untruncated bytes are in the archive itself.
const MAX_PROJECT_INSTRUCTION_PREVIEW_CHARS: usize = 4_000;

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

    // Bundle name collision -- soft, informational (§2: bundle_upsert
    // has no name uniqueness constraint, so this never blocks).
    let existing_names: std::collections::HashSet<String> = id_store
        .bundle_list()
        .map_err(|e| format!("bundle.import.preview: {e}"))?
        .into_iter()
        .map(|b| b.name)
        .collect();
    let name_collision = existing_names.contains(&parsed.name);

    // Phase 3, Codex P1 on PR #3163. Read-only: this is what the SOURCE agent
    // was reading, and `bundle.import.commit` has no counterpart field — the
    // entries exist to be looked at, never to be written into the importing
    // repository (spec §5.2). Each content is bounded the same way
    // `instructions_preview` is, since a snapshot can be an arbitrarily large
    // repository file and the response carries up to
    // MAX_IMPORTED_PROJECT_INSTRUCTIONS of them.
    let project_instructions_json: Vec<serde_json::Value> = parsed
        .project_instructions
        .iter()
        .map(|pi| {
            let total_chars = pi.content.chars().count();
            let truncated = total_chars > MAX_PROJECT_INSTRUCTION_PREVIEW_CHARS;
            let preview: String = if truncated {
                pi.content.chars().take(MAX_PROJECT_INSTRUCTION_PREVIEW_CHARS).collect()
            } else {
                pi.content.clone()
            };
            json!({
                "path": bounded_display(&pi.path),
                "file": bounded_display(&pi.file),
                "content_hash": bounded_display(&pi.content_hash),
                "owner": bounded_display(&pi.owner),
                "content_preview": preview,
                "content_truncated": truncated,
                "content_total_chars": total_chars,
            })
        })
        .collect();

    let (warnings, warnings_truncated) = bound_warnings_for_response(all_warnings);

    Ok(json!({
        "project_instructions": project_instructions_json,
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

                let memory = Bundle {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: bundle_name,
                    description: parsed.description,
                    is_blank: false,
                    is_global: false,
                    // Like `description` above (and unlike instructions/
                    // context/mcp), provider/model are structural identity
                    // fields, not opt-in components — there's no
                    // include_provider toggle in the preview/commit UI, so
                    // these always carry straight through when present.
                    provider: parsed.provider,
                    model: parsed.model,
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
                    is_system: false,
                };
                if let Err(e) = id_store.bundle_upsert(&memory) {
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

                // Ref tables are authoritative — bind after the bundle row
                // exists. Only the servers the user actually selected.
                warnings.extend(bind_imported_components(
                    &wstore,
                    &id_store,
                    &memory.id,
                    &imported_skill_ids,
                    &selected_mcp_servers.iter().map(|c| (*c).clone()).collect::<Vec<_>>(),
                ));

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
        assert!(state.id_store.bundle_list().unwrap().iter().all(|b| b.name != "test-bundle"));
    }

    #[tokio::test]
    async fn commit_applies_bundle_name_override_not_parsed_name() {
        // codex P2, PR #2381 round 11: bundle_name must actually be
        // substituted for Bundle.name, never silently ignored.
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
        let saved = state.id_store.bundle_get(bundle_id).unwrap().unwrap();
        assert_eq!(saved.name, "Renamed Bundle");
    }

    #[tokio::test]
    async fn commit_bounds_an_oversized_bundle_name_override() {
        // reagentx P2, PR #2382 round 3: unlike parsed.name (bounded at
        // parse time), req.bundle_name had no length cap of its own before
        // being used verbatim as Bundle.name.
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
        let saved = state.id_store.bundle_get(bundle_id).unwrap().unwrap();
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
        let saved = state.id_store.bundle_get(bundle_id).unwrap().unwrap();
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
        let saved = state.id_store.bundle_get(bundle_id).unwrap().unwrap();
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

    fn make_bundle(state: &AppState, id: &str, instructions: &str) -> crate::backend::storage::store::Bundle {
        let bundle = crate::backend::storage::store::Bundle {
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
            is_system: false,
        };
        state.id_store.bundle_upsert(&bundle).unwrap();
        bundle
    }

    /// Bind an MCP server to a bundle through the ref table, the way the
    /// Armory does.
    fn bind_mcp(state: &AppState, bundle_id: &str, name: &str, config: &str) {
        let server = crate::backend::storage::McpServer {
            id: format!("srv-{name}"),
            name: name.to_string(),
            transport: "stdio".to_string(),
            config: config.to_string(),
            is_global: false,
            created_at: 1,
            updated_at: 1,
        };
        state
            .wstore
            .bundle_mcp_upsert_unique(&state.id_store, bundle_id, &server, true)
            .unwrap();
    }

    /// Bind a skill to a bundle through the ref table.
    fn bind_skill(state: &AppState, bundle_id: &str, name: &str) {
        let skill = crate::backend::storage::Skill {
            id: format!("skill-{name}"),
            name: name.to_string(),
            trigger: name.to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: String::new(),
            content: "body".to_string(),
            is_global: false,
            created_at: 1,
            updated_at: 1,
        };
        state
            .wstore
            .bundle_skill_upsert_unique(&state.id_store, bundle_id, &skill, true)
            .unwrap();
    }

    #[tokio::test]
    async fn project_instructions_resolves_the_authenticated_slug_not_a_uuid() {
        // ReAgent P0 on #3156: check_s1 authenticates a SLUG, while
        // agent_def_get queries by UUID — so looking the agent up directly
        // returned None for every real caller. The existing test helper sets
        // id == slug, which could never have caught it, so this one keeps them
        // deliberately different.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();

        let mut def: crate::backend::storage::AgentDefinition =
            serde_json::from_value(serde_json::json!({
                "id": "11111111-2222-3333-4444-555555555555",
                "slug": "agent-slug",
                "name": "Agent Slug",
                "icon": "robot",
                "provider": "claude",
                "description": "",
                "working_directory": work.path().to_str().unwrap(),
                "created_at": 1,
            }))
            .unwrap();
        state.wstore.agent_def_insert(&mut def).unwrap();
        state
            .wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: def.id.clone(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", config_dir.path().display()),
                updated_at: 0,
            })
            .unwrap();

        // The slug is what an authenticated agent actually sends.
        let resolved = resolve_agent_for_s1(&state, "agent-slug")
            .expect("the authenticated slug must resolve");
        assert_eq!(resolved.id, def.id, "must find the definition behind the slug");

        // And the UUID still works, for a caller that legitimately holds one.
        let by_id = resolve_agent_for_s1(&state, &def.id).expect("a definition id must still work");
        assert_eq!(by_id.slug, "agent-slug");
    }

    #[tokio::test]
    async fn export_carries_ref_bound_components_the_inline_columns_never_had() {
        // The Phase 0b regression. Before this, binding a skill or server in
        // the Armory wrote only a ref row while export read only the inline
        // column, so this bundle exported as empty skills/ and mcp/
        // directories with no warning — in a format advertised as a backup.
        let state = test_state();
        let bundle = make_bundle(&state, "bundle-1", "Be helpful.");
        assert_eq!(bundle.skills, "[]", "fixture must leave the inline columns empty");
        assert_eq!(bundle.mcp_servers, "[]");

        bind_skill(&state, "bundle-1", "Deploy");
        bind_mcp(&state, "bundle-1", "github", r#"{"command":"gh-mcp","env":{"GITHUB_TOKEN":"tok"}}"#);

        let result = bundle_export_impl(
            &state.id_store,
            &state.wstore,
            ExportReq { id: "bundle-1".to_string(), format: String::new() },
        )
        .unwrap();

        let paths: Vec<String> = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            paths.iter().any(|p| p.starts_with("skills/")),
            "ref-bound skill must be exported: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == "mcp/github.server.json"),
            "ref-bound MCP server must be exported: {paths:?}"
        );

        // And the secret is still redacted on the way out — resolving from a
        // different source must not bypass the redaction pass.
        let server_file = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "mcp/github.server.json")
            .unwrap();
        let content = server_file["content"].as_str().unwrap();
        assert!(!content.contains("tok"), "secret must not survive export: {content}");
    }

    #[tokio::test]
    async fn deleting_a_bundle_takes_its_component_refs_with_it() {
        // No FK reaches from the ref tables to db_bundles — they live in
        // different physical databases — so nothing else removes these, and a
        // later bundle reusing the id would inherit components nobody chose.
        let state = test_state();
        make_bundle(&state, "bundle-1", "Be helpful.");
        bind_skill(&state, "bundle-1", "Deploy");
        bind_mcp(&state, "bundle-1", "github", r#"{"command":"gh-mcp"}"#);

        let before = resolve_bundle_components(&state.wstore, "bundle-1").unwrap();
        assert_eq!(before.skills.len(), 1);
        assert_eq!(before.mcp_entries.len(), 1);

        assert!(state.id_store.bundle_delete("bundle-1").unwrap());
        // Through the shared helper both delete RPCs call — `bundle.delete`
        // here and `deletememory`, the one the Armory actually uses. Fixing
        // only one left the product path orphaning refs (Codex, PR #3153).
        purge_bundle_component_refs(&state.wstore, "bundle-1");
        let (skills, mcp) = state.wstore.bundle_unbind_all_components("bundle-1").unwrap();
        assert_eq!(
            (skills, mcp), (0, 0),
            "the purge must have already removed both refs"
        );

        // Re-create a bundle with the same id: it must start empty.
        make_bundle(&state, "bundle-1", "Reused id.");
        let after = resolve_bundle_components(&state.wstore, "bundle-1").unwrap();
        assert!(
            after.skills.is_empty() && after.mcp_entries.is_empty(),
            "a reused bundle id must not inherit the old bundle's components"
        );
    }

    #[tokio::test]
    async fn validate_reports_a_malformed_bound_component_instead_of_passing() {
        // Codex on #3153: the resolver drops a bound server whose config will
        // not parse and says so in a warning. Discarding that left validate
        // reporting is_valid for exactly the component it exists to catch —
        // the entry is absent from the resolved list, so nothing downstream
        // could have seen it.
        let state = test_state();
        make_bundle(&state, "bundle-1", "Be helpful.");
        bind_mcp(&state, "bundle-1", "broken", "not json at all");

        let report = crate::server::app_api::bundle_validate_impl(
            &state.wstore,
            json!({ "id": "bundle-1", "name": "Bundle bundle-1" }),
        )
        .unwrap();

        assert_eq!(
            report["is_valid"], json!(false),
            "a bound component that cannot load must not validate clean: {report}"
        );
        let issues = report["issues"].as_array().unwrap();
        assert!(
            issues.iter().any(|i| i["field"] == "mcp_servers"
                && i["message"].as_str().unwrap_or("").contains("broken")),
            "the issue must name the server: {issues:?}"
        );
    }

    #[tokio::test]
    async fn export_carries_project_instructions_with_their_owner() {
        // The last piece of Phase 3: an exported bundle records what the
        // source agent was reading, with enough to tell whose file each was.
        let state = test_state();
        let work = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        std::fs::write(work.path().join("CLAUDE.md"), "# House rules\n\nBe careful.\n").unwrap();

        let result = bundle_export_for_agent_impl(
            &state.id_store,
            &state.wstore,
            ExportForAgentReq {
                bundle_id: "bundle-1".to_string(),
                agent_id: "agent-1".to_string(),
                format: String::new(),
            },
        )
        .await
        .unwrap();

        let files = result["files"].as_array().unwrap();
        let carried = files
            .iter()
            .find(|f| f["path"] == "instructions/project/CLAUDE.md")
            .expect("the repository's own CLAUDE.md must be carried");
        assert!(carried["content"].as_str().unwrap().contains("House rules"));

        let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
        let entries = manifest["components"]["projectInstructions"].as_array().unwrap();
        let entry = entries.iter().find(|e| e["path"] == "CLAUDE.md").unwrap();
        assert_eq!(
            entry["owner"], "foreign",
            "an unmarked repository file is the repository's — this field is what keeps import from installing it"
        );
        assert!(!entry["contentHash"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn export_carries_a_readable_but_empty_instruction_file() {
        // Codex P2 on #3163: skipping on `content.is_empty()` conflated "the
        // file is there and says nothing" with "there is no file", which is
        // the exact ambiguity this component exists to remove. An empty
        // CLAUDE.md is a real fact about what the agent reads.
        let state = test_state();
        let work = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        std::fs::write(work.path().join("CLAUDE.md"), "").unwrap();

        let result = bundle_export_for_agent_impl(
            &state.id_store,
            &state.wstore,
            ExportForAgentReq {
                bundle_id: "bundle-1".to_string(),
                agent_id: "agent-1".to_string(),
                format: String::new(),
            },
        )
        .await
        .unwrap();

        let files = result["files"].as_array().unwrap();
        assert!(
            files.iter().any(|f| f["path"] == "instructions/project/CLAUDE.md"),
            "an empty-but-present CLAUDE.md must still be recorded"
        );
        let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
        let entries = manifest["components"]["projectInstructions"].as_array().unwrap();
        assert!(
            entries.iter().any(|e| e["path"] == "CLAUDE.md"),
            "present-and-empty must be distinguishable from absent, got: {entries:?}"
        );
    }

    /// Build a resolver result by hand. The two conditions below (a
    /// case-only path collision, and unreadable/oversized files) are not
    /// reachable through the filesystem on this repo's own dev platform —
    /// Windows folds case, so the collision cannot be staged at all — so the
    /// splice function is exercised directly rather than not at all.
    fn instruction_file(
        path: &str,
        content: &str,
    ) -> crate::backend::project_instructions::ProjectInstructionFile {
        crate::backend::project_instructions::ProjectInstructionFile {
            path: path.to_string(),
            exists: true,
            size_bytes: content.len() as u64,
            content_hash: "hash".to_string(),
            owner: crate::backend::project_instructions::InstructionOwner::Foreign,
            content: content.to_string(),
            truncated: false,
            error: None,
        }
    }

    fn empty_export() -> crate::backend::bundle_export::BundleExport {
        crate::backend::bundle_export::BundleExport {
            root_slug: "b".to_string(),
            files: vec![crate::backend::bundle_export::BundleExportFile {
                path: "armory.json".to_string(),
                content: "{\"components\":{}}".to_string(),
            }],
            skipped_skills: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn case_only_different_instruction_paths_do_not_overwrite_each_other() {
        // Codex P2 on #3163. Scanned directories (Copilot's
        // `.github/instructions/`) make two paths differing only in case
        // reachable on a case-sensitive source filesystem; extracted on
        // Windows or macOS one silently clobbers the other.
        let mut export = empty_export();
        let files = vec![
            instruction_file(".github/instructions/A.instructions.md", "first"),
            instruction_file(".github/instructions/a.instructions.md", "second"),
        ];
        splice_project_instructions_component(&mut export, &files).unwrap();

        let carried: Vec<_> = export
            .files
            .iter()
            .filter(|f| f.path.to_lowercase().starts_with("instructions/project/"))
            .collect();
        assert_eq!(carried.len(), 1, "the second must be skipped, not written alongside");
        assert!(
            export.warnings.iter().any(|w| w.contains("normalizes to the same path")),
            "the skip must be reported, not silent: {:?}",
            export.warnings
        );
    }

    #[test]
    fn unreadable_and_truncated_instruction_files_are_not_exported() {
        // The other half of the empty-file fix: `content.is_empty()` was doing
        // two jobs, and only one of them was correct. These are the cases that
        // genuinely have nothing to archive.
        let mut export = empty_export();
        let mut unreadable = instruction_file("CLAUDE.md", "");
        unreadable.error = Some("permission denied".to_string());
        let mut oversized = instruction_file("AGENTS.md", "");
        oversized.truncated = true;
        let mut absent = instruction_file("GEMINI.md", "");
        absent.exists = false;

        splice_project_instructions_component(&mut export, &[unreadable, oversized, absent])
            .unwrap();

        assert!(
            !export.files.iter().any(|f| f.path.starts_with("instructions/project/")),
            "nothing readable means nothing to carry"
        );
    }

    #[tokio::test]
    async fn a_scanned_file_deleted_after_launch_is_reported_removed() {
        // Codex on #3162: the resolver stops returning a scanned path once the
        // file is gone, so iterating current files only made it vanish from
        // the response instead of reporting `removed` — and the stale row
        // meant recreating it would later read as `unchanged`.
        use crate::backend::storage::project_instructions::{
            classify_change, InstructionChange, ProjectInstructionObservation,
        };

        let state = test_state();
        let work = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());

        // Recorded at a previous launch, and since deleted from disk.
        state
            .wstore
            .project_instructions_record(
                "agent-1",
                &[ProjectInstructionObservation {
                    path: ".github/instructions/gone.instructions.md".to_string(),
                    content_hash: "h1".to_string(),
                    size_bytes: 3,
                    owner: "foreign".to_string(),
                    existed: true,
                    observed_at: 1,
                }],
            )
            .unwrap();

        let current = crate::backend::project_instructions::resolve_project_instructions(
            "copilot",
            work.path().to_str().unwrap(),
        );
        assert!(
            !current.iter().any(|f| f.path.ends_with("gone.instructions.md")),
            "precondition: the resolver no longer returns a deleted scanned file"
        );

        let previous = state.wstore.project_instructions_list("agent-1").unwrap();
        let orphan = previous
            .iter()
            .find(|p| p.path.ends_with("gone.instructions.md"))
            .unwrap();
        assert_eq!(
            classify_change(Some(orphan), false, ""),
            InstructionChange::Removed,
            "a prior observation with no current file is a removal"
        );
    }

    #[tokio::test]
    async fn a_foreign_instruction_file_changing_between_launches_is_visible() {
        // The point of tracking: a repository's own CLAUDE.md can be the
        // majority of what an agent is told, and until now nothing recorded
        // enough about one to notice it had changed.
        use crate::backend::storage::project_instructions::InstructionChange;

        let state = test_state();
        let work = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());
        let agent = state.wstore.agent_def_get("agent-1").unwrap().unwrap();

        std::fs::write(work.path().join("CLAUDE.md"), "original rules").unwrap();

        // First launch: nothing recorded yet.
        let first = crate::backend::project_instructions::resolve_project_instructions(
            &agent.provider,
            work.path().to_str().unwrap(),
        );
        let claude = first.iter().find(|f| f.path == "CLAUDE.md").unwrap();
        assert_eq!(
            crate::backend::storage::project_instructions::classify_change(
                None, claude.exists, &claude.content_hash
            ),
            InstructionChange::FirstSeen
        );
        crate::server::app_api::agent_open::observe_project_instructions(
            &state.wstore,
            &agent,
            work.path().to_str().unwrap(),
        );

        // The repository changes underneath.
        std::fs::write(work.path().join("CLAUDE.md"), "somebody edited this").unwrap();

        let second = crate::backend::project_instructions::resolve_project_instructions(
            &agent.provider,
            work.path().to_str().unwrap(),
        );
        let claude = second.iter().find(|f| f.path == "CLAUDE.md").unwrap();
        let recorded = state.wstore.project_instructions_list("agent-1").unwrap();
        let prev = recorded.iter().find(|p| p.path == "CLAUDE.md");
        assert_eq!(
            crate::backend::storage::project_instructions::classify_change(
                prev, claude.exists, &claude.content_hash
            ),
            InstructionChange::Modified,
            "an edit between launches must be detectable"
        );

        // Observing again settles it.
        crate::server::app_api::agent_open::observe_project_instructions(
            &state.wstore,
            &agent,
            work.path().to_str().unwrap(),
        );
        let recorded = state.wstore.project_instructions_list("agent-1").unwrap();
        let prev = recorded.iter().find(|p| p.path == "CLAUDE.md");
        assert_eq!(
            crate::backend::storage::project_instructions::classify_change(
                prev, claude.exists, &claude.content_hash
            ),
            InstructionChange::Unchanged
        );
    }

    #[tokio::test]
    async fn a_bound_mcp_server_with_unparseable_config_warns_instead_of_vanishing() {
        // The guarantee that moved here from bundle_export.rs when the
        // renderer stopped parsing MCP JSON: malformed component data warns,
        // it does not silently disappear.
        let state = test_state();
        make_bundle(&state, "bundle-1", "Be helpful.");
        bind_mcp(&state, "bundle-1", "broken", "not json at all");

        let components = resolve_bundle_components(&state.wstore, "bundle-1").unwrap();
        assert!(components.mcp_entries.is_empty(), "unparseable config must not be exported");
        assert!(
            components.warnings.iter().any(|w| w.contains("broken") && w.contains("invalid config")),
            "expected a warning naming the server, got: {:?}",
            components.warnings
        );
    }

    #[tokio::test]
    async fn resolving_ignores_catalog_rows_this_bundle_is_not_bound_to() {
        // `managed_list` returns globals alongside bound rows; only the bound
        // ones are this bundle's contents.
        let state = test_state();
        make_bundle(&state, "bundle-1", "Be helpful.");
        make_bundle(&state, "bundle-2", "Other.");
        bind_mcp(&state, "bundle-2", "elsewhere", r#"{"command":"x"}"#);

        let components = resolve_bundle_components(&state.wstore, "bundle-1").unwrap();
        assert!(
            components.mcp_entries.is_empty(),
            "another bundle's server must not leak in: {:?}",
            components.mcp_entries
        );
        assert!(components.skills.is_empty());
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

    /// Point an agent DEFINITION's own bundle at `bundle_id`.
    ///
    /// Uses the dedicated setter, not `agent_def_update`: that UPDATE
    /// deliberately does not list `default_memory_id` among its columns
    /// (`storage/agents.rs:1188-1197`), so mutating `AgentDefinition.memory_id`
    /// and calling it would silently no-op.
    fn bind_definition_to_bundle(state: &AppState, agent_id: &str, bundle_id: &str) {
        assert!(
            state
                .wstore
                .agent_def_set_memory_id_if_empty(agent_id, bundle_id)
                .unwrap(),
            "test setup: binding {agent_id} to {bundle_id} must apply"
        );
    }

    #[tokio::test]
    async fn agent_less_export_warns_when_a_bound_agent_has_memory() {
        // SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md §3.1/§5.1:
        // import announces a dropped memory component; export must too.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        bind_definition_to_bundle(&state, "agent-1", "bundle-1");
        state
            .id_store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "a fact", None, "/x", 6, 0)
            .unwrap();

        assert!(
            bound_agent_has_native_memory(&state.id_store, &state.wstore, "bundle-1"),
            "definition-bound agent with memory must be detected"
        );

        // The contract that matters is at the RPC boundary, not the helper.
        let result = bundle_export_impl(
            &state.id_store,
            &state.wstore,
            ExportReq { id: "bundle-1".to_string(), format: String::new() },
        )
        .unwrap();
        let warnings = result["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w == MEMORY_NOT_EXPORTED_WARNING),
            "bundle.export must announce the memory it is not carrying: {warnings:?}"
        );
        // ...and still not carry it: this path has no agent to read from.
        let files = result["files"].as_array().unwrap();
        assert!(!files.iter().any(|f| f["path"].as_str().unwrap_or("").starts_with("memory/")));
    }

    #[tokio::test]
    async fn agent_less_export_is_silent_without_memory_or_without_a_binding() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        make_bundle(&state, "bundle-2", "Other.");
        bind_definition_to_bundle(&state, "agent-1", "bundle-1");

        // Bound, but the agent has no memory at all.
        assert!(!bound_agent_has_native_memory(&state.id_store, &state.wstore, "bundle-1"));

        state
            .id_store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "a fact", None, "/x", 6, 0)
            .unwrap();

        // Now it has memory — but bundle-2 is bound to nobody, so exporting
        // bundle-2 loses nothing and must stay quiet.
        assert!(!bound_agent_has_native_memory(&state.id_store, &state.wstore, "bundle-2"));

        // The blank-singleton sentinel must never warn, or nearly every
        // export would.
        assert!(!bound_agent_has_native_memory(&state.id_store, &state.wstore, ""));

        // And the RPC stays quiet for the unbound bundle.
        let result = bundle_export_impl(
            &state.id_store,
            &state.wstore,
            ExportReq { id: "bundle-2".to_string(), format: String::new() },
        )
        .unwrap();
        let warnings = result["warnings"].as_array().unwrap();
        assert!(
            !warnings.iter().any(|w| w == MEMORY_NOT_EXPORTED_WARNING),
            "exporting a bundle nobody is bound to loses nothing: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn agent_less_export_warns_for_an_instance_scoped_binding_too() {
        // The definition's own bundle and a launch's bundle are different
        // columns; a launch pointed at another bundle still carries the
        // definition's memory.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        make_bundle(&state, "bundle-2", "Other.");
        bind_definition_to_bundle(&state, "agent-1", "bundle-1");
        state
            .id_store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "a fact", None, "/x", 6, 0)
            .unwrap();

        let inst: crate::backend::storage::AgentInstance =
            serde_json::from_value(serde_json::json!({
                "id": "inst-1",
                "definition_id": "agent-1",
                "memory_id": "bundle-2",
                "status": "running",
                "started_at": 1,
                "created_at": 1,
            }))
            .unwrap();
        state.wstore.instance_create(&inst).unwrap();

        assert!(
            bound_agent_has_native_memory(&state.id_store, &state.wstore, "bundle-2"),
            "a launch pointed at bundle-2 still carries agent-1's memory"
        );
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

    /// An ABF carrying one skill and one MCP server.
    fn abf_files_with_components() -> Vec<FileEntry> {
        let manifest = serde_json::json!({
            "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
            "name": "imported-bundle",
            "version": "0.1.0",
            "description": "",
            "components": {
                "instructions": { "default": ["instructions/AGENTS.md"] },
                "skills": ["skills/deploy"],
                "mcpServers": ["mcp/github.server.json"],
            },
            "metadata": {},
        });
        vec![
            FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
            FileEntry { path: "instructions/AGENTS.md".to_string(), content: "Be helpful.".to_string() },
            FileEntry {
                path: "skills/deploy/SKILL.md".to_string(),
                // Frontmatter values are JSON-quoted — that is what
                // `render_skill_md` writes and what `parse_skill_md` requires.
                content: "---\nname: \"deploy\"\ndescription: \"Ship it\"\n---\n\nSteps.".to_string(),
            },
            FileEntry {
                path: "mcp/github.server.json".to_string(),
                content: r#"{"name":"github","command":"gh-mcp","env":{"GITHUB_TOKEN":"${GITHUB_TOKEN}"}}"#.to_string(),
            },
        ]
    }

    #[tokio::test]
    async fn an_imported_bundle_exports_the_components_it_arrived_with() {
        // Codex P1 on #3152: once export reads only the ref tables, an import
        // that writes only the inline columns produces a bundle that exports
        // empty — and whose MCP servers never reach a spawned agent, since
        // launch reads the refs too. The round trip is the contract.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        let result = bundle_import_for_agent_impl(
            &state.id_store,
            &state.wstore,
            ImportForAgentReq {
                agent_id: "agent-1".to_string(),
                file_path: None,
                zip_base64: None,
                files: Some(abf_files_with_components()),
            },
        )
        .await
        .unwrap();

        let bundle_id = result["bundle_id"]
            .as_str()
            .or_else(|| result["id"].as_str())
            .expect("import must report the bundle it created")
            .to_string();

        let components = resolve_bundle_components(&state.wstore, &bundle_id).unwrap();
        assert_eq!(
            components.skills.len(),
            1,
            "imported skill must be bound to the bundle. import result: {result}"
        );
        assert_eq!(
            components.mcp_entries.len(),
            1,
            "imported MCP server must be bound to the bundle: {:?}",
            components.warnings
        );

        // And it round-trips out again.
        let exported = bundle_export_impl(
            &state.id_store,
            &state.wstore,
            ExportReq { id: bundle_id, format: String::new() },
        )
        .unwrap();
        let paths: Vec<String> = exported["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(paths.iter().any(|p| p.starts_with("skills/")), "got {paths:?}");
        assert!(paths.iter().any(|p| p == "mcp/github.server.json"), "got {paths:?}");
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
    async fn import_for_agent_fails_fast_when_agent_has_no_resolvable_memory_dir_but_bundle_has_memory() {
        // reagent P2, PR #2527: must fail BEFORE creating any skill/bundle
        // rows, not silently succeed with memory_files_written: 0 -- this
        // RPC exists specifically to transfer memory.
        //
        // The trigger narrowed in PR #2901: a blank `working_directory` alone
        // no longer reaches this guard, because it now resolves to the same
        // default `agent.open` substitutes (~/.agentmux/agents/<name-slug>) --
        // which serves this guard's OWN stated purpose better than erroring
        // did, since the memory is written where the agent will actually read
        // it instead of being refused. The guard remains for the genuinely
        // unresolvable case, which needs a blank NAME too (no name => no
        // derivable default), exercised here.
        let state = test_state();
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "agent-no-workdir",
            "slug": "agent-no-workdir",
            "name": "",
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
        assert!(state.id_store.bundle_list().unwrap().iter().all(|b| b.is_blank));
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
    async fn export_for_agent_repeats_the_truncation_warning_on_an_unchanged_oversized_file() {
        // reagent P2, PR #2527 (second round): the truncation check used
        // to run only inside the "changed since last mirror" branch, so
        // a SECOND export of the same still-oversized, unchanged file
        // silently stopped warning even though the exported content was
        // still truncated every time.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");

        let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        let oversized = "x".repeat(10 * 1024 * 1024 + 1);
        std::fs::write(memory_dir.join("MEMORY.md"), &oversized).unwrap();

        // First export mirrors it (and warns).
        let _ = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        }).await.unwrap();

        // Second export: file on disk is byte-for-byte unchanged (same
        // size+mtime), so the mirror-refresh's "unchanged" fast path
        // applies — the warning must still fire.
        let result = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        }).await.unwrap();
        let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(
            warnings.iter().any(|w| w.contains("MEMORY.md") && w.contains("truncated")),
            "truncation warning must repeat on a second export of the same unchanged oversized file, got: {warnings:?}"
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
        let bundle = state.id_store.bundle_get(bundle_id).unwrap().unwrap();
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
        let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(
            warnings.iter().any(|w| w.contains("not a valid memory filename")),
            "got: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn import_for_agent_warns_when_a_declared_memory_path_has_no_matching_content() {
        // reagent P1, PR #2527 (third round): a components.memory entry
        // with no matching file among the bundle's contents used to
        // silently continue with no warning, undercounting
        // memory_files_written with zero signal to the caller — this RPC
        // exists specifically to transfer memory, so every skip must warn.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());

        let manifest = serde_json::json!({
            "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
            "name": "imported-bundle",
            "version": "0.1.0",
            "description": "",
            "components": { "memory": ["memory/MISSING.md"] },
            "metadata": {},
        });
        // Deliberately no "memory/MISSING.md" entry in files.
        let files = vec![
            FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
        ];
        let result = bundle_import_for_agent_impl(&state.id_store, &state.wstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(files),
        }).await.unwrap();
        assert_eq!(result["memory_files_written"], 0);
        let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(
            warnings.iter().any(|w| w.contains("MISSING.md") && w.contains("not found")),
            "got: {warnings:?}"
        );
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

    // ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.3/§7.5 step 6:
    // provider/model round-trip through export -> import, same as the
    // memory-files round-trip above.
    #[tokio::test]
    async fn export_then_import_carries_provider_and_model_through() {
        let state = test_state();
        let config_a = tempfile::tempdir().unwrap();
        let config_b = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-a", "/work/a", config_a.path());
        make_agent(&state, "agent-b", "/work/b", config_b.path());
        let mut bundle = make_bundle(&state, "bundle-src", "Shared instructions.");
        bundle.provider = "claude".to_string();
        bundle.model = "anthropic".to_string();
        state.id_store.bundle_upsert(&bundle).unwrap();

        let exported = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-src".to_string(),
            agent_id: "agent-a".to_string(),
            format: String::new(),
        }).await.unwrap();

        let manifest_file = exported["files"].as_array().unwrap().iter()
            .find(|f| f["path"] == "armory.json")
            .expect("armory.json must be present");
        let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
        assert_eq!(manifest["provider"], "claude");
        assert_eq!(manifest["model"], "anthropic");

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
        let new_bundle_id = imported["bundle_id"].as_str().expect("import response must include bundle_id");
        let new_bundle = state.id_store.bundle_get(new_bundle_id).unwrap().unwrap();
        assert_eq!(new_bundle.provider, "claude");
        assert_eq!(new_bundle.model, "anthropic");
    }

    #[tokio::test]
    async fn export_of_an_unbound_bundle_omits_provider_and_model_rather_than_exporting_empty_strings() {
        let state = test_state();
        let config_a = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-a", "/work/a", config_a.path());
        make_bundle(&state, "bundle-unbound", "Shared instructions."); // provider/model left empty

        let exported = bundle_export_for_agent_impl(&state.id_store, &state.wstore, ExportForAgentReq {
            bundle_id: "bundle-unbound".to_string(),
            agent_id: "agent-a".to_string(),
            format: String::new(),
        }).await.unwrap();

        let manifest_file = exported["files"].as_array().unwrap().iter()
            .find(|f| f["path"] == "armory.json")
            .expect("armory.json must be present");
        let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
        assert!(manifest["provider"].is_null(), "unset provider must not export as an empty string");
        assert!(manifest["model"].is_null(), "unset model must not export as an empty string");
    }

    // docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
    // §3.3 — bundle.export_for_agent_with_history.

    mod with_history_tests {
        use super::*;
        use crate::backend::history::adapter::{DiscoveredFile, HistoryAdapter, HistoryError, HistorySession, SessionMeta};
        use crate::backend::history::index::SessionIndex;
        use crate::backend::history::HistoryService;

        /// Tags every discovered file with a fixed identity_id -- enough to
        /// exercise the agent_id -> identity_id -> sessions chain without a
        /// real filesystem scan.
        struct MockAdapter {
            files: Vec<DiscoveredFile>,
            identity_id: String,
        }
        impl HistoryAdapter for MockAdapter {
            fn provider(&self) -> &str {
                "mock"
            }
            fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
                Ok(self.files.iter().map(|f| DiscoveredFile { file_path: f.file_path.clone(), mtime_ms: f.mtime_ms }).collect())
            }
            fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError> {
                let id = std::path::Path::new(file_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                Ok(Some(SessionMeta {
                    session_id: id,
                    file_path: file_path.to_string(),
                    provider: "mock".to_string(),
                    model: String::new(),
                    slug: String::new(),
                    working_directory: "/proj".to_string(),
                    created_at: 0,
                    modified_at: 0,
                    message_count: 0,
                    first_user_message: String::new(),
                    file_size_bytes: 0,
                    git_branch: String::new(),
                    total_tokens: 0,
                    subagent_count: 0,
                    identity_id: self.identity_id.clone(),
                }))
            }
            fn parse_file(&self, _: &str) -> Result<Option<HistorySession>, HistoryError> {
                Ok(None)
            }
        }

        fn link_agent_to_identity(state: &AppState, agent_id: &str, account_id: &str) {
            state
                .id_store
                .identity_upsert(&crate::backend::storage::store::IdentityAccount {
                    id: account_id.to_string(),
                    name: format!("claude-{account_id}"),
                    provider: "claude".to_string(),
                    kind: "pat".to_string(),
                    display_name: String::new(),
                    secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: String::new() },
                    context: serde_json::json!({}),
                    status: "unknown".to_string(),
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
            state.id_store.agent_identity_link(agent_id, account_id, "claude").unwrap();
        }

        fn unzip_paths(zip_base64: &str) -> Vec<String> {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(zip_base64).unwrap();
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect()
        }

        /// The `armory.json` manifest out of an exported zip.
        fn unzip_manifest(zip_base64: &str) -> serde_json::Value {
            use base64::Engine as _;
            use std::io::Read as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(zip_base64).unwrap();
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let name = (0..archive.len())
                .map(|i| archive.by_index(i).unwrap().name().to_string())
                .find(|n| n.ends_with("armory.json"))
                .expect("export must contain armory.json");
            let mut content = String::new();
            archive.by_name(&name).unwrap().read_to_string(&mut content).unwrap();
            serde_json::from_str(&content).unwrap()
        }

        #[tokio::test]
        async fn includes_the_agents_sessions_alongside_the_normal_bundle_files() {
            let state = test_state();
            let config_dir = tempfile::tempdir().unwrap();
            make_agent(&state, "agent-1", "/work/proj", config_dir.path());
            make_bundle(&state, "bundle-1", "Be helpful.");
            link_agent_to_identity(&state, "agent-1", "acct-mine");

            let session_dir = tempfile::tempdir().unwrap();
            let session_path = session_dir.path().join("sess-1.jsonl");
            std::fs::write(&session_path, "{\"role\":\"user\"}\n").unwrap();

            let index = SessionIndex::with_isolated_roots(
                vec![Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: session_path.to_string_lossy().into_owned(), mtime_ms: 1 }],
                    identity_id: "acct-mine".to_string(),
                })],
                vec![session_dir.path().to_path_buf()],
            );
            let history_service = HistoryService::from_index(index);

            let result = bundle_export_for_agent_with_history_impl(
                state.id_store.clone(),
                state.identity_store.clone(),
                &state.wstore,
                std::sync::Arc::new(history_service),
                ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
            )
            .await
            .unwrap();

            assert_eq!(result["history_session_count"], 1);
            let paths = unzip_paths(result["zip_base64"].as_str().unwrap());
            assert!(paths.iter().any(|p| p.ends_with("instructions/AGENTS.md")), "base bundle files must still be present: {paths:?}");
            assert!(paths.iter().any(|p| p.ends_with("history/mock/sess-1.jsonl")), "session must be included under history/: {paths:?}");

            // SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md §3.5: the
            // files above were in the archive but invisible to anything reading
            // components.* to learn what the archive holds.
            let manifest = unzip_manifest(result["zip_base64"].as_str().unwrap());
            assert_eq!(
                manifest["components"]["history"],
                json!(["history/mock/sess-1.jsonl"]),
                "history must be registered in the manifest, not just zipped"
            );
        }

        #[tokio::test]
        async fn omits_the_history_component_when_no_session_was_included() {
            // Absence already means "none"; an empty array would assert
            // "this archive has an empty history", which is a different claim.
            let state = test_state();
            let config_dir = tempfile::tempdir().unwrap();
            make_agent(&state, "agent-1", "/work/proj", config_dir.path());
            make_bundle(&state, "bundle-1", "Be helpful.");
            // No identity link -> no sessions discovered.

            let index = SessionIndex::with_isolated_roots(vec![], vec![]);
            let history_service = HistoryService::from_index(index);

            let result = bundle_export_for_agent_with_history_impl(
                state.id_store.clone(),
                state.identity_store.clone(),
                &state.wstore,
                std::sync::Arc::new(history_service),
                ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
            )
            .await
            .unwrap();

            assert_eq!(result["history_session_count"], 0);
            let manifest = unzip_manifest(result["zip_base64"].as_str().unwrap());
            assert!(
                manifest["components"].get("history").is_none(),
                "no sessions must leave the history component absent: {}",
                manifest["components"]
            );
        }

        #[tokio::test]
        async fn succeeds_with_zero_sessions_when_the_agent_has_no_linked_identity() {
            let state = test_state();
            let config_dir = tempfile::tempdir().unwrap();
            make_agent(&state, "agent-1", "/work/proj", config_dir.path());
            make_bundle(&state, "bundle-1", "Be helpful.");
            // No link_agent_to_identity call -- agent has no bound account.

            let history_service = HistoryService::from_index(SessionIndex::with_isolated_roots(vec![], vec![]));

            let result = bundle_export_for_agent_with_history_impl(
                state.id_store.clone(),
                state.identity_store.clone(),
                &state.wstore,
                std::sync::Arc::new(history_service),
                ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
            )
            .await
            .unwrap();

            assert_eq!(result["history_session_count"], 0);
            let paths = unzip_paths(result["zip_base64"].as_str().unwrap());
            assert!(!paths.iter().any(|p| p.contains("history/")), "no history/ entries when nothing is linked: {paths:?}");
        }

        #[tokio::test]
        async fn warns_and_continues_when_a_transcript_file_cannot_be_read() {
            let state = test_state();
            let config_dir = tempfile::tempdir().unwrap();
            make_agent(&state, "agent-1", "/work/proj", config_dir.path());
            make_bundle(&state, "bundle-1", "Be helpful.");
            link_agent_to_identity(&state, "agent-1", "acct-mine");

            // Points at a file that will never exist on disk.
            let missing_path = std::env::temp_dir().join("amx-nonexistent-session-xyz.jsonl");
            let index = SessionIndex::with_isolated_roots(
                vec![Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: missing_path.to_string_lossy().into_owned(), mtime_ms: 1 }],
                    identity_id: "acct-mine".to_string(),
                })],
                vec![],
            );
            // Note: refresh() would normally skip a file it can't stat via
            // discover_files' own is_dir/exists checks in the real adapter,
            // but MockAdapter unconditionally reports it as discovered so
            // extract_meta -- and therefore the export's own read -- is what
            // actually exercises the missing-file path here.
            let history_service = HistoryService::from_index(index);

            let result = bundle_export_for_agent_with_history_impl(
                state.id_store.clone(),
                state.identity_store.clone(),
                &state.wstore,
                std::sync::Arc::new(history_service),
                ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
            )
            .await
            .unwrap();

            assert_eq!(result["history_session_count"], 0, "unreadable session must not count as included");
            let warnings = result["warnings"].as_array().unwrap();
            assert!(
                warnings.iter().any(|w| w.as_str().unwrap_or("").contains("failed to read session")),
                "must warn about the unreadable session: {warnings:?}"
            );
        }

        // reagentx P1 on PR #2613: an oversized transcript must be skipped
        // (never truncated -- a partial JSONL file can fail or misparse on
        // the receiving end) and warned about, not silently included or
        // read into memory unbounded.
        #[tokio::test]
        async fn skips_and_warns_on_a_transcript_over_the_size_limit() {
            let state = test_state();
            let config_dir = tempfile::tempdir().unwrap();
            make_agent(&state, "agent-1", "/work/proj", config_dir.path());
            make_bundle(&state, "bundle-1", "Be helpful.");
            link_agent_to_identity(&state, "agent-1", "acct-mine");

            let session_dir = tempfile::tempdir().unwrap();
            let oversized_path = session_dir.path().join("sess-huge.jsonl");
            {
                // Sparse file -- claims MAX_HISTORY_SESSION_FILE_SIZE_BYTES + 1
                // bytes via metadata without actually writing/allocating
                // that much, matching read_abf_file_path's own sparse-file
                // test technique above.
                let file = std::fs::File::create(&oversized_path).unwrap();
                file.set_len(MAX_HISTORY_SESSION_FILE_SIZE_BYTES + 1).unwrap();
            }

            let index = SessionIndex::with_isolated_roots(
                vec![Box::new(MockAdapter {
                    files: vec![DiscoveredFile { file_path: oversized_path.to_string_lossy().into_owned(), mtime_ms: 1 }],
                    identity_id: "acct-mine".to_string(),
                })],
                vec![session_dir.path().to_path_buf()],
            );
            let history_service = HistoryService::from_index(index);

            let result = bundle_export_for_agent_with_history_impl(
                state.id_store.clone(),
                state.identity_store.clone(),
                &state.wstore,
                std::sync::Arc::new(history_service),
                ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
            )
            .await
            .unwrap();

            assert_eq!(result["history_session_count"], 0, "oversized session must not count as included");
            let paths = unzip_paths(result["zip_base64"].as_str().unwrap());
            assert!(!paths.iter().any(|p| p.contains("sess-huge")), "oversized session must not be in the zip: {paths:?}");
            let warnings = result["warnings"].as_array().unwrap();
            assert!(
                warnings.iter().any(|w| w.as_str().unwrap_or("").contains("exceeds") && w.as_str().unwrap_or("").contains("sess-huge")),
                "must warn about the size limit: {warnings:?}"
            );
        }

        // reagentx P1, third review round: a per-session cap alone doesn't
        // bound an agent with many sessions each just under it. These
        // exercise the pure sessions_within_total_budget decision directly
        // with tiny numbers -- MAX_TOTAL_HISTORY_BYTES is 500 MiB, real
        // enough files to trigger it would make this test slow and wasteful
        // for no extra confidence over testing the same logic with small
        // inputs.
        #[test]
        fn total_budget_includes_everything_when_under_the_cap() {
            let (count, stopped) = sessions_within_total_budget(&[10, 10, 10], 100);
            assert_eq!(count, 3);
            assert!(!stopped);
        }

        #[test]
        fn total_budget_stops_at_the_first_session_that_would_exceed_the_cap() {
            // Most-recent-first order: [10, 10, 10] with cap 25 -- the
            // third one (cumulative 30) doesn't fit, so only the first two
            // (most recent) are included.
            let (count, stopped) = sessions_within_total_budget(&[10, 10, 10], 25);
            assert_eq!(count, 2, "must include the two most-recent sessions, not just any two that individually fit");
            assert!(stopped);
        }

        #[test]
        fn total_budget_includes_nothing_when_the_first_session_alone_exceeds_the_cap() {
            let (count, stopped) = sessions_within_total_budget(&[50, 10], 25);
            assert_eq!(count, 0);
            assert!(stopped);
        }

        #[test]
        fn total_budget_handles_an_empty_list() {
            let (count, stopped) = sessions_within_total_budget(&[], 100);
            assert_eq!(count, 0);
            assert!(!stopped);
        }

        #[test]
        fn total_budget_includes_a_session_that_exactly_fills_the_remaining_cap() {
            // 10 + 15 == 25 exactly -- must NOT be treated as exceeding.
            let (count, stopped) = sessions_within_total_budget(&[10, 15], 25);
            assert_eq!(count, 2);
            assert!(!stopped);
        }
    }
}
