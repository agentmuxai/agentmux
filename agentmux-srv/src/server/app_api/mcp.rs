// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App API handlers for the v1 standalone MCP Server primitive.
//! mcp.list / mcp.get / mcp.upsert / mcp.delete / mcp.bind / mcp.unbind

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_mcp_list(engine, state);
    register_mcp_get(engine, state);
    register_mcp_upsert(engine, state);
    register_mcp_delete(engine, state);
    register_mcp_bind(engine, state);
    register_mcp_unbind(engine, state);
}

fn register_mcp_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_MCP_LIST,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("mcp.list: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                let servers = wstore.mcp_server_list(&req.agent_id)
                    .map_err(|e| format!("mcp.list: {e}"))?;
                Ok(Some(serde_json::to_value(&servers).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_mcp_get(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_MCP_GET,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("mcp.get: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                if !wstore.mcp_server_is_accessible_to(&req.agent_id, &req.id)
                    .map_err(|e| format!("mcp.get: {e}"))?
                {
                    return Err("FORBIDDEN: MCP server not accessible to this agent".to_string());
                }
                let server = wstore.mcp_server_get(&req.id)
                    .map_err(|e| format!("mcp.get: {e}"))?;
                Ok(Some(serde_json::to_value(&server).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_mcp_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_MCP_UPSERT,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req {
                    agent_id: String,
                    #[serde(default)] id: String,
                    name: String,
                    #[serde(default = "default_transport")] transport: String,
                    #[serde(default = "default_config")] config: String,
                }
                fn default_transport() -> String { "stdio".to_string() }
                fn default_config() -> String { "{}".to_string() }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("mcp.upsert: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;

                match serde_json::from_str::<serde_json::Value>(&req.config) {
                    Ok(serde_json::Value::Object(_)) => {}
                    Ok(_) => return Err("mcp.upsert: config must be a JSON object".to_string()),
                    Err(_) => return Err("mcp.upsert: config must be valid JSON".to_string()),
                }
                if req.name == "agentmux" {
                    return Err("FORBIDDEN: 'agentmux' is a reserved MCP server name".to_string());
                }

                let now = now_ms();
                // created_at is preserved across updates (ON CONFLICT keeps the
                // original), so the returned struct never reports a timestamp the
                // DB won't persist. New rows default to `now`.
                let (id, created_at) = if req.id.is_empty() {
                    (uuid::Uuid::new_v4().to_string(), now)
                } else {
                    if !wstore.mcp_server_is_bound_to(&req.agent_id, &req.id)
                        .map_err(|e| format!("mcp.upsert: {e}"))?
                    {
                        return Err("FORBIDDEN: MCP server not bound to this agent".to_string());
                    }
                    let existing = wstore.mcp_server_get(&req.id)
                        .map_err(|e| format!("mcp.upsert: {e}"))?;
                    match existing {
                        Some(s) if s.is_global => {
                            return Err("FORBIDDEN: cannot mutate a global MCP server".to_string());
                        }
                        Some(s) => (req.id.clone(), s.created_at),
                        None => (req.id.clone(), now),
                    }
                };

                // Reject a duplicate name already bound to this agent (excluding
                // the row being updated, so a rename-to-self is allowed).
                let bound = wstore.mcp_server_list(&req.agent_id)
                    .map_err(|e| format!("mcp.upsert: {e}"))?;
                if bound.iter().any(|s| s.name == req.name && s.id != id) {
                    return Err(format!(
                        "mcp.upsert: server name '{}' already bound to this agent",
                        req.name
                    ));
                }

                let server = crate::backend::storage::McpServer {
                    id: id.clone(),
                    name: req.name,
                    transport: req.transport,
                    config: req.config,
                    is_global: false,
                    created_at,
                    updated_at: now,
                };
                wstore.mcp_server_upsert(&server)
                    .map_err(|e| format!("mcp.upsert: {e}"))?;
                if req.id.is_empty() {
                    wstore.mcp_server_bind(&req.agent_id, &id)
                        .map_err(|e| format!("mcp.upsert: bind: {e}"))?;
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "mcp:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                Ok(Some(serde_json::to_value(&server).map_err(|e| e.to_string())?))
            })
        }),
    );
}

fn register_mcp_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_MCP_DELETE,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("mcp.delete: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                if !wstore.mcp_server_is_bound_to(&req.agent_id, &req.id)
                    .map_err(|e| format!("mcp.delete: {e}"))?
                {
                    return Err("FORBIDDEN: MCP server not bound to this agent".to_string());
                }
                if let Some(existing) = wstore.mcp_server_get(&req.id)
                    .map_err(|e| format!("mcp.delete: {e}"))?
                {
                    if existing.is_global {
                        return Err("FORBIDDEN: cannot delete a global MCP server".to_string());
                    }
                }
                let deleted = wstore.mcp_server_delete(&req.id)
                    .map_err(|e| format!("mcp.delete: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "mcp:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }
                Ok(Some(json!({ "deleted": deleted })))
            })
        }),
    );
}

fn register_mcp_bind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_MCP_BIND,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, mcp_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("mcp.bind: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                // Only global servers (or ones already bound) may be bound, so an
                // agent can't escalate to read another agent's server config.
                match wstore.mcp_server_get(&req.mcp_id)
                    .map_err(|e| format!("mcp.bind: {e}"))?
                {
                    None => return Err("mcp.bind: MCP server not found".to_string()),
                    Some(s) if !s.is_global => {
                        if !wstore.mcp_server_is_bound_to(&req.agent_id, &req.mcp_id)
                            .map_err(|e| format!("mcp.bind: {e}"))?
                        {
                            return Err("FORBIDDEN: can only bind global MCP servers".to_string());
                        }
                    }
                    Some(_) => {}
                }
                wstore.mcp_server_bind(&req.agent_id, &req.mcp_id)
                    .map_err(|e| format!("mcp.bind: {e}"))?;
                Ok(Some(json!({ "bound": true })))
            })
        }),
    );
}

fn register_mcp_unbind(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_MCP_UNBIND,
        Box::new(move |data, ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, mcp_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("mcp.unbind: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                let unbound = wstore.mcp_server_unbind(&req.agent_id, &req.mcp_id)
                    .map_err(|e| format!("mcp.unbind: {e}"))?;
                Ok(Some(json!({ "unbound": unbound })))
            })
        }),
    );
}
