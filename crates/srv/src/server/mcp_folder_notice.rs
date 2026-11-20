// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pane row that names the MCP servers a host agent is not given.
//!
//! In a folder a container agent can write, a host agent starts only the servers AgentMux writes
//! (`backend::mcp_folder_rule`). The folder's own `.mcp.json` may list others: they are not started, and the
//! pane says which, once for each set of names in a run of srv. Nothing is dropped without a word.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::backend::obj::{meta_get_string, Block};

/// What each pane was last told: its block id to the names.
static TOLD: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// Whether `names` is news to the pane `block_id` (and records that it has been told).
fn is_news(block_id: &str, names: &[String]) -> bool {
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    let told = told.get_or_insert_with(HashMap::new);
    if told.get(block_id).is_some_and(|last| last == names) {
        return false;
    }
    told.insert(block_id.to_string(), names.to_vec());
    true
}

/// The servers the host agent on a pane with `meta` is not given: those of its folder's `.mcp.json` that its
/// own config lacks. Empty for a container agent and for a folder the rule does not hold for. Reads the disk.
fn left_out_of(meta: &crate::backend::obj::MetaMapType) -> Vec<String> {
    let Some(folder) = crate::backend::mcp_folder_rule::folder_of(meta) else { return Vec::new() };
    let ours = std::fs::read_to_string(crate::backend::mcp_config_file::host_config_path(&folder)).ok();
    crate::backend::mcp_folder_rule::left_out(&folder, ours.as_deref())
}

/// Before a host agent's turn starts its CLI: tell the pane which servers of its folder's own file it is not
/// given, when there are any and the pane has not been told of exactly these.
pub(crate) async fn tell(
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block: &Block,
) {
    let meta = block.meta.clone();
    let names = tokio::task::spawn_blocking(move || left_out_of(&meta)).await.unwrap_or_default();
    if names.is_empty() || !is_news(&block.oid, &names) {
        return;
    }
    tracing::warn!(block_id = %block.oid, count = names.len(), "MCP servers of the folder's own file are not given to this host agent: a container agent can write the folder");
    let provider = meta_get_string(&block.meta, "agentProvider", "claude");
    let frame = crate::backend::cli_notice::mcp_left_out_frame(&provider, &block.oid, &names);
    crate::backend::cli_notice::append_to_pane(broker, filestore, mstore, &block.oid, &frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pane_is_told_once_for_each_set_of_names() {
        let names = |list: &[&str]| list.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert!(is_news("block-told-a", &names(&["one"])));
        assert!(!is_news("block-told-a", &names(&["one"])), "the same names again are not news");
        assert!(is_news("block-told-a", &names(&["one", "two"])), "another server is");
        assert!(is_news("block-told-b", &names(&["one"])), "each pane is told for itself");
    }

    #[test]
    fn the_servers_left_out_are_those_the_agents_own_config_lacks() {
        let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = folder.path().to_string_lossy().into_owned();
        let pane = |mode: &str| -> crate::backend::obj::MetaMapType {
            [("agentMode", json!(mode)), ("agentProvider", json!("claude")), ("cmd:cwd", json!(dir))].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        };
        std::fs::write(folder.path().join(".mcp.json"), r#"{"mcpServers":{"agentmux":{"command":"x"},"theirs":{"command":"y"}}}"#).unwrap();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            assert!(left_out_of(&pane("host")).is_empty(), "a folder no container agent has to do with: nothing is left out");
            crate::backend::mcp_folder_rule::mark(&dir);
            assert_eq!(left_out_of(&pane("host")), vec!["agentmux".to_string(), "theirs".to_string()], "no config of its own yet");
            let own = crate::backend::mcp_config_file::host_config_path(&dir);
            assert!(!own.starts_with(folder.path()), "the agent's config is outside the folder");
            crate::backend::mcp_config_file::write_relocated(&own, r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"}}}"#).unwrap();
            assert_eq!(left_out_of(&pane("host")), vec!["theirs".to_string()]);
            assert!(left_out_of(&pane("container")).is_empty(), "the container agent itself is not a host agent");
        });
    }
}
