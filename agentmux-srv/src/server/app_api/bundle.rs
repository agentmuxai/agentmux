use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_bundle_list(engine, state);
    register_bundle_get(engine, state);
    register_bundle_upsert(engine, state);
    register_bundle_delete(engine, state);
    register_bundle_self_get(engine, state);
    register_bundle_export(engine, state);
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
