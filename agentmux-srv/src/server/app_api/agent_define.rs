use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_define(engine, state);
}

/// Infer a provider slug from a model name prefix.
/// Only maps prefixes that correspond to a registered provider slug.
/// Callers must still validate the result via `providers::get_provider`.
pub(super) fn infer_provider_from_model(model: &str) -> String {
    let m = model.to_lowercase();
    if m.starts_with("claude") {
        "claude".to_string()
    } else if m.starts_with("gemini") {
        "gemini".to_string()
    } else if m.starts_with("codex") {
        "codex".to_string()
    } else if m.starts_with("qwen") {
        "qwen".to_string()
    } else if m.starts_with("kimi") {
        "kimi".to_string()
    } else {
        // Unknown prefix — return as-is; get_provider will reject it with
        // a "cannot infer provider" error so callers know to set provider explicitly.
        model.to_string()
    }
}

/// Returns `(stub_id, newly_inserted)`. `newly_inserted = false` when the
/// UNIQUE constraint fires (stub already existed); callers use this to avoid
/// broadcasting `agents:changed` on no-op calls.
pub(super) fn make_stub_idempotent(
    wstore: &crate::backend::storage::store::Store,
    def_id: &str,
    name: &str,
    now: i64,
) -> Result<(String, bool), String> {
    let stub_id = format!("si-{}", def_id.replace('-', ""));
    let inst = AgentInstance {
        id: stub_id.clone(),
        definition_id: def_id.to_string(),
        parent_instance_id: String::new(),
        block_id: String::new(),
        session_id: String::new(),
        status: "stopped".to_string(),
        github_context: String::new(),
        started_at: now,
        ended_at: 0,
        created_at: now,
        identity_id: String::new(),
        memory_id: String::new(),
        instance_name: name.to_string(),
        working_directory: String::new(),
        display_hidden: false,
    };
    match wstore.instance_create(&inst) {
        Ok(_) => Ok((stub_id, true)),
        Err(e) if e.to_string().contains("UNIQUE constraint") => {
            Ok((stub_id, false)) // stub already existed — idempotent
        }
        Err(e) => Err(format!("agent.define: create stub instance: {e}")),
    }
}

/// Core logic for the `agent.define` command, shared by the WebSocket RPC
/// handler and the HTTP service dispatch (`("agent", "define")` in service.rs).
/// Persist `system_prompt` and `env` content blobs for a freshly created or
/// updated agent definition.  Errors are logged but not propagated — the
/// definition row is already committed and the caller has already published
/// `agents:changed`, so a content-write failure must not abort the response.
pub(super) fn persist_define_content(
    wstore: &Store,
    agent_id: &str,
    cmd: &CommandAgentDefineData,
    now: i64,
) {
    if let Some(prompt) = &cmd.system_prompt {
        if !prompt.is_empty() {
            if let Err(e) = wstore.agent_content_set(&AgentContent {
                agent_id: agent_id.to_string(),
                content_type: "agentmd".to_string(),
                content: prompt.clone(),
                updated_at: now,
            }) {
                tracing::warn!(agent_id, err = %e, "agent.define: failed to persist system_prompt (non-fatal)");
            }
        }
    }
    if let Some(env_map) = &cmd.env {
        if !env_map.is_empty() {
            let content = env_map.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("\n");
            if let Err(e) = wstore.agent_content_set(&AgentContent {
                agent_id: agent_id.to_string(),
                content_type: "env".to_string(),
                content,
                updated_at: now,
            }) {
                tracing::warn!(agent_id, err = %e, "agent.define: failed to persist env (non-fatal)");
            }
        }
    }
}

fn register_agent_define(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();

    engine.register_handler(
        COMMAND_AGENT_DEFINE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandAgentDefineData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.define: {e}"))?;
                agent_define_core(wstore, broker, cmd).await
                    .map(|r| Some(serde_json::to_value(&r).unwrap()))
            })
        }),
    );
}
