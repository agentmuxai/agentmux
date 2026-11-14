// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which folder each agent on a block was started in, so a browser upload can be
//! checked against it (`server/browser_uploads.rs`) and the spawn paths can record
//! it without reaching up into the server.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn launch_workspaces() -> &'static Mutex<HashMap<String, PathBuf>> {
    static W: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Remember the workspace the agent on `block_id` was started in: the
/// `AGENTMUX_AGENT_WORKDIR` srv just put in its spawn env (host agents only;
/// a container agent has none). Called by every spawn path right after
/// `spawn_env::block_identity::carry_agent_workdir_env`.
///
/// Uploads are checked against this record, not the block's `cmd:cwd`: any
/// client with the instance auth key can rewrite that meta, so an agent
/// could otherwise point its "workspace" at `/` and upload anything.
pub fn record_launch_workspace(block_id: &str, env_vars: &HashMap<String, String>) {
    // A block is one or the other: a host launch ends any container record.
    container_workspaces().lock().unwrap_or_else(|p| p.into_inner()).remove(block_id);
    let mut map = launch_workspaces().lock().unwrap_or_else(|p| p.into_inner());
    match env_vars.get("AGENTMUX_AGENT_WORKDIR").filter(|w| !w.trim().is_empty()) {
        Some(w) => {
            map.insert(block_id.to_string(), PathBuf::from(w));
        }
        None => {
            map.remove(block_id);
        }
    }
}

/// The workspace the agent on `block_id` was launched in, if srv started it
/// as a host agent.
pub fn agent_workspace(block_id: &str) -> Option<PathBuf> {
    launch_workspaces().lock().unwrap_or_else(|p| p.into_inner()).get(block_id).cloned()
}

fn container_workspaces() -> &'static Mutex<HashMap<String, PathBuf>> {
    static W: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Remember the host folder mounted at `/workspace` in the container of the
/// agent on `block_id`, as srv started it: the mount's source, not the block's
/// `cmd:cwd`, which any client can rewrite after the launch. Called where the
/// container's mounts are decided.
pub fn record_container_workspace(block_id: &str, host_dir: Option<&str>) {
    launch_workspaces().lock().unwrap_or_else(|p| p.into_inner()).remove(block_id);
    let mut map = container_workspaces().lock().unwrap_or_else(|p| p.into_inner());
    match host_dir.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            map.insert(block_id.to_string(), PathBuf::from(d));
        }
        None => {
            map.remove(block_id);
        }
    }
}

/// The host folder behind `/workspace` for the container agent on `block_id`.
pub fn container_workspace(block_id: &str) -> Option<PathBuf> {
    container_workspaces().lock().unwrap_or_else(|p| p.into_inner()).get(block_id).cloned()
}
