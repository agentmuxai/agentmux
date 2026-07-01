use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_preset_list(engine, state);
    register_preset_get(engine, state);
    register_preset_upsert(engine, state);
    register_preset_delete(engine, state);
    register_preset_self_get(engine, state);
}

/// Normalize a `preset.upsert` request body into the shape the `Memory` struct
/// deserializes from, so the App API accepts the request exactly as documented
/// in the spec:
///   - `id` may be omitted to create (the struct has no serde default), so an
///     absent or null `id` is filled with an empty string (the handler then
///     mints a UUID).
///   - `context_files` / `mcp_servers` / `skills` are JSON-encoded array strings
///     on the struct, but the spec shows them as JSON arrays. Array values are
///     re-encoded to their JSON string form; values already given as strings
///     pass through untouched.
pub(super) fn normalize_preset_upsert_input(mut data: serde_json::Value) -> serde_json::Value {
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

fn register_preset_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_PRESET_LIST,
        Box::new(move |_data, _ctx| {
            let state = state.clone();
            Box::pin(async move { Ok(Some(preset_list_impl(&state).await?)) })
        }),
    );
}

fn register_preset_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_PRESET_GET,
        Box::new(move |data, _ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                struct Req {
                    #[serde(default)] id: String,
                    #[serde(default)] name: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("preset.get: {e}"))?;
                Ok(Some(preset_get_impl(&state, &req.id, &req.name).await?))
            })
        }),
    );
}

fn register_preset_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_PRESET_UPSERT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Memory =
                    serde_json::from_value(normalize_preset_upsert_input(data))
                        .map_err(|e| format!("preset.upsert: {e}"))?;

                // S4: guard on the target id, not the caller-supplied is_blank flag
                // (which defaults false and can be omitted to bypass is_blank check).
                if memory.id == "blank" || memory.id.starts_with("seed-") || memory.is_blank {
                    return Err("FORBIDDEN: cannot mutate a protected preset".to_string());
                }
                // Guard existing global presets: an agent must not be able to demote or
                // corrupt a shared global brain bundle it doesn't own by supplying its id.
                if !memory.id.is_empty() {
                    if let Some(existing) = id_store.bundle_memory_get(&memory.id)
                        .map_err(|e| format!("preset.upsert: {e}"))?
                    {
                        if existing.is_global {
                            return Err("FORBIDDEN: cannot mutate a global preset".to_string());
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
                    .map_err(|e| format!("preset.upsert: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(serde_json::to_value(&memory).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_preset_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_PRESET_DELETE,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("preset.delete: {e}"))?;

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
                    Err(e) => Err(format!("preset.delete: {e}")),
                }
            })
        }),
    );
}

fn register_preset_self_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_PRESET_SELF_GET,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("preset.self.get: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                Ok(Some(preset_self_get_impl(&state, &req.agent_id).await?))
            })
        }),
    );
}
