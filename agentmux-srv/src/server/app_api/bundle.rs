use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_bundle_list(engine, state);
    register_bundle_get(engine, state);
    register_bundle_upsert(engine, state);
    register_bundle_delete(engine, state);
    register_bundle_self_get(engine, state);
    register_bundle_export(engine, state);
    register_bundle_import(engine, state);
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
                struct FileEntry {
                    path: String,
                    content: String,
                }
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
                let mut resolved_requirement_ids: Vec<String> = Vec::new();
                let mut unresolved_requirements: Vec<serde_json::Value> = Vec::new();
                // codex P1, PR #2379 round 4: query each distinct provider
                // ONCE, not once per requirement row -- bundle_import.rs
                // bounds the requirements array length, but even at that
                // bound many rows commonly share a handful of providers
                // (or an attacker can repeat one provider many times), and
                // each identity_list call is a synchronous store query.
                let mut match_count_by_provider: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for requirement in &parsed.requirements {
                    let match_count = match match_count_by_provider.get(requirement.provider.as_str()) {
                        Some(&n) => n,
                        None => {
                            // Codex P2, PR #2379 round 2: account CRUD routes
                            // through id_store (shared/store.db when available), not
                            // wstore (the per-channel database) -- see identity.rs's
                            // own handler registrations. Querying wstore here would
                            // report zero/stale matches for accounts that actually
                            // exist, incorrectly placing requirements in
                            // unresolved_requirements.
                            let n = id_store
                                .identity_list(Some(&requirement.provider))
                                .map_err(|e| format!("bundle.import: account lookup failed: {e}"))?
                                .len();
                            match_count_by_provider.insert(requirement.provider.as_str(), n);
                            n
                        }
                    };
                    if match_count == 1 {
                        resolved_requirement_ids.push(requirement.id.clone());
                    } else {
                        unresolved_requirements.push(json!({
                            "id": requirement.id,
                            "provider": requirement.provider,
                            "env": requirement.env,
                            "match_count": match_count,
                        }));
                    }
                }

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
                    context_files: serde_json::to_string(&parsed.context_files)
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
