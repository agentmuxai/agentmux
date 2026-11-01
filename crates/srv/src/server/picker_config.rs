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
