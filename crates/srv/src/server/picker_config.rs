// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `writeagentconfig` (the window's Launch) writes for an agent, before it is written: the files the
//! window built, with what srv adds to them. The catalog's MCP servers go into `.mcp.json`
//! ([`mcp_launch::add_to_files`]), the launch's agent id and signing keys
//! ([`mcp_json_identity::finish_launch`]), and Global Memory and the agent's picked bundles into the startup
//! file ([`inject_global_bundles`]). Pure, so a test can render a launch without a socket or a disk: the
//! parity test (`agent_config_parity_tests`) compares it with `agent.open`'s.

use std::sync::Arc;

use crate::backend::rpc_types::AgentConfigFile;
use crate::backend::storage::store::Store;

use super::app_api::agent_config_render::render_agent_config_files;
use super::editor_handlers::inject_global_bundles;

/// `files` as the window sent them, with what srv adds for `agent_id` launched as `agent_mode`
/// (`host` or `container`). Keys go only when `hands_out_keys` (the window, proven by its host) and the
/// agent is not a container agent.
pub(crate) fn picker_files(
    mut files: Vec<AgentConfigFile>,
    mstore: &Arc<Store>,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    agent_id: Option<&str>,
    agent_mode: &str,
    hands_out_keys: bool,
) -> Vec<AgentConfigFile> {
    // The bundles picked for the agent go into its startup file after Global Memory. An older frontend
    // sends no agent id: then there are none.
    let agent_block = agent_id
        .filter(|id| !id.is_empty())
        .map(|id| {
            crate::backend::storage::format_agent_bundle_block(&crate::backend::storage::agent_picked_bundles(
                mstore, id_store, id,
            ))
        })
        .unwrap_or_default();
    // The agent's catalog servers, as `agent.open` gives them.
    let container = agent_mode == "container";
    crate::backend::mcp_launch::add_to_files(&mut files, mstore, identity_store, agent_id, container);
    // The launch's .mcp.json signs as the definition's agent; the window's launch also carries that
    // agent's signing keys, never a container agent's.
    crate::backend::mcp_json_identity::finish_launch(&mut files, mstore, agent_id, hands_out_keys && !container);
    // The startup file gets the same Global Memory `agent.open` puts in it. This RPC carries no provider
    // id, so a startup-instructions file is recognized by filename against the provider registry.
    for file in &mut files {
        if file.path == "CLAUDE.md" || crate::backend::providers::is_known_startup_instructions_filename(&file.path) {
            file.content = inject_global_bundles(&file.content, id_store, agent_mode, &agent_block);
        }
    }
    files
}

/// The files the window's Launch writes for `agent_id` into `work_dir`, rendered in Rust exactly as `agent.open`
/// renders them ([`render_agent_config_files`]), so the window sends no file content of its own. `instance_name`
/// is the launch's name for the agent (a modal-supplied one), `None` for the agent's own. The launch's folder is
/// its own (allocated or named by the caller), never the fallback folder every same-name launch shares. Whether
/// the agent is a container agent is the definition's (`agent_type`), as in `agent.open`.
pub(crate) fn launch_files(
    mstore: &Store,
    id_store: &Store,
    identity_store: &Store,
    agent_id: &str,
    instance_name: Option<&str>,
    work_dir: &str,
) -> Result<Vec<AgentConfigFile>, String> {
    let agent = mstore
        .agent_def_get(agent_id)
        .map_err(|e| format!("failed to read agent {agent_id}: {e}"))?
        .ok_or_else(|| format!("unknown agent {agent_id}"))?;
    let display_name = instance_name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&agent.name);
    let slug = if agent.slug.is_empty() { agentmux_common::slug::agent_route_name(&agent.name) } else { agent.slug.clone() };
    let files = render_agent_config_files(mstore, id_store, identity_store, &agent, display_name, &slug, work_dir, false);
    Ok(files.into_iter().map(|f| AgentConfigFile { path: f.filename, content: f.content }).collect())
}

/// The files `writeagentconfig` writes into `work_dir`: rendered from the agent's definition when the window asks
/// (`render_from_agent`), otherwise the ones the caller sent with what srv adds to them ([`picker_files`]).
pub(crate) fn files_to_write(
    sent: Vec<AgentConfigFile>,
    mstore: &Arc<Store>,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    cmd: &crate::backend::rpc_types::CommandWriteAgentConfigData,
    agent_mode: &str,
    hands_out_keys: bool,
    work_dir: &str,
) -> Result<Vec<AgentConfigFile>, String> {
    if cmd.render_from_agent && !hands_out_keys {
        return Err("FORBIDDEN: only the AgentMux window renders an agent's launch files".to_string());
    }
    match cmd.agent_id.as_deref().filter(|_| cmd.render_from_agent) {
        Some(agent_id) => launch_files(mstore, id_store, identity_store, agent_id, cmd.instance_name.as_deref(), work_dir),
        None => Ok(picker_files(sent, mstore, id_store, identity_store, cmd.agent_id.as_deref(), agent_mode, hands_out_keys)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(agent_id: &str) -> crate::backend::rpc_types::CommandWriteAgentConfigData {
        serde_json::from_value(serde_json::json!({
            "working_dir": "w", "files": [], "auto_allocate": false, "agent_id": agent_id, "render_from_agent": true
        }))
        .unwrap()
    }

    #[test]
    fn only_the_window_renders_an_agents_launch_files() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let refused = files_to_write(Vec::new(), &store, &store, &store, &cmd("someone"), "host", false, "w");
        assert!(refused.unwrap_err().starts_with("FORBIDDEN"), "an agent connection must not render launch files");
    }

    #[test]
    fn rendering_an_unknown_agent_fails_instead_of_writing_nothing() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let err = files_to_write(Vec::new(), &store, &store, &store, &cmd("no-such-agent"), "host", true, "w").unwrap_err();
        assert!(err.contains("unknown agent"), "{err}");
    }

    #[test]
    fn without_the_flag_the_sent_files_are_written_as_before() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut c = cmd("someone");
        c.render_from_agent = false;
        let sent = vec![AgentConfigFile { path: "notes.txt".into(), content: "x".into() }];
        let files = files_to_write(sent, &store, &store, &store, &c, "host", true, "w").unwrap();
        assert!(files.iter().any(|f| f.path == "notes.txt" && f.content == "x"));
    }
}
