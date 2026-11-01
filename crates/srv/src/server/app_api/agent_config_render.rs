// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use crate::backend::storage::store::Store;

/// The files an agent launch writes into its working directory, whichever way it was started: `agent.open`
/// ([`super::agent_open::write_agent_config_files`]) and the window's Launch (`writeagentconfig`, `picker_config::launch_files`)
/// both render them here, so a native and a container agent, and the two entry points, share one builder.
/// `display_name` is what `{{AGENT}}` and `{{AGENT_DISPLAY}}` expand to (a launch's instance name, or the
/// agent's name); `workdir_is_shared` is whether other launches of the agent use the same folder.
pub(crate) fn render_agent_config_files(
    mstore: &Store,
    id_store: &Store,
    identity_store: &Store,
    agent: &crate::backend::storage::AgentDefinition,
    display_name: &str,
    agent_slug: &str,
    work_dir: &str,
    workdir_is_shared: bool,
) -> Vec<crate::backend::agent_config::AgentConfigFile> {
    // The agent's content, with Global Memory and its picked bundles put
    // before its own memory (shared with `memory:launchcost`).
    let mut content_map = crate::server::memory_launch_cost::startup_content_map(mstore, id_store, agent);

    // Disable autonomous memory writes only when using the bare slug-fallback workdir
    // (~/.agentmux/agents/<slug>) — that path is shared across same-name multi-tab
    // launches with no collision resolution, risking concurrent MEMORY.md corruption
    // (upstream issue #29051). Any explicitly set workdir, or one a launch allocated, is isolated and
    // keeps writes on.
    if workdir_is_shared {
        let settings_str = content_map
            .entry("settings".to_string())
            .or_insert_with(|| "{}".to_string());
        match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(settings_str) {
            Ok(mut obj) => {
                obj.entry("autoMemoryEnabled".to_string())
                    .or_insert(json!(false));
                *settings_str =
                    serde_json::to_string(&obj).unwrap_or_else(|_| "{}".to_string());
            }
            Err(e) => {
                tracing::warn!(
                    work_dir = %work_dir,
                    error = %e,
                    "write_agent_config_files: settings JSON unparseable; \
                     autoMemoryEnabled guard skipped — memory writes may be active on shared workdir"
                );
            }
        }
    }

    // v1 skills: globals are always injected; the agent's OWN ref-bound skills
    // are authoritative when present, otherwise fall back to legacy
    // db_agent_skills. See Store::effective_skills for the merge algorithm —
    // shared with the `listagentskills` RPC handler so the frontend's
    // pre-launch skill fetch and this materialization path never diverge.
    let effective_skills = mstore.effective_skills(identity_store, &agent.id);

    let mut config_files = crate::backend::agent_config::build_config_files(
        &content_map,
        &effective_skills,
        display_name,
        &agent.id,
        agent_slug,
        work_dir,
        &agent.provider,
    );

    // v1 MCP: "agentmux" plus the agent's effective servers. The
    // legacy blob's user servers are merged in ONLY when the agent has no own
    // ref-bound servers (so a global server never wipes a legacy-only agent's
    // .mcp.json). When the agent has own refs, those are authoritative.
    // A bundle carries no MCP servers
    // (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md §3.1), so bundles
    // add none.
    let visible_mcp: Vec<crate::backend::storage::McpServer> = mstore.effective_mcp_servers(identity_store, &agent.id, agent.agent_type == "container");
    let has_own_mcp_refs = visible_mcp.iter().any(|server| !server.is_global);
    if !visible_mcp.is_empty() {
        let blob_for_merge = if has_own_mcp_refs {
            None
        } else {
            content_map.get("mcp").map(|s| s.as_str())
        };
        if let Some(mcp_json) = crate::backend::agent_config::build_mcp_config_from_refs(
            &visible_mcp,
            blob_for_merge,
            agent_slug,
            &agent.agent_bus_id,
        ) {
            if let Some(pos) = config_files.iter().position(|f| f.filename == ".mcp.json") {
                config_files[pos].content = mcp_json;
            } else {
                config_files.push(crate::backend::agent_config::AgentConfigFile {
                    filename: ".mcp.json".to_string(),
                    content: mcp_json,
                });
            }
        }
    }

    // Host-tier + LAN-tier jekt sender signing keys
    // (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2,
    // SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1) — inject
    // AGENTMUX_JEKT_KEY / AGENTMUX_LAN_KEY into the agentmux MCP server's
    // own env, right alongside AGENTMUX_AGENT_ID, so the `Message` tool
    // can sign outgoing jekts as this agent. This copy is the fallback: the
    // MCP first fetches its keys from srv (`GET /agentmux/agents/self/keys`).
    // Best-effort: a failure here must never block agent spawn — the agent
    // still launches; only with neither source do its jekts go unsigned.
    // Shared with `WriteAgentConfig`
    // (server/editor_handlers.rs, the "click Launch" path) via
    // `agent_config::inject_jekt_signing_keys_into_mcp_json` — see that
    // function's doc comment for why this must not be duplicated inline
    // again.
    if let Some(file) = config_files.iter_mut().find(|f| f.filename == ".mcp.json") {
        crate::backend::mcp_json_identity::keys_for_agent_open(&mut file.content, &mstore, agent_slug, &agent);
    }
    config_files
}
