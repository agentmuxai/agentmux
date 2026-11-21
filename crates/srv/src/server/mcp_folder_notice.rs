// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pane row that says what of its folder a host agent is not given.
//!
//! In a folder a container agent can write, a host agent takes its MCP servers, settings, instructions and
//! skills from AgentMux alone (`backend::mcp_folder_rule`, `backend::host_agent_files`). The folder may hold
//! its own: they are not loaded, and the pane says what, by kind and count (the servers by name), once for
//! each state of the folder in a run of srv. Nothing is dropped without a word.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::backend::host_agent_files::LeftOut;
use crate::backend::obj::{meta_get_string, Block};

/// What a host agent in such a folder is not given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NotGiven {
    /// The servers of the folder's own `.mcp.json` that the agent's config lacks.
    servers: Vec<String>,
    /// The folder's other files, by kind.
    folder: LeftOut,
    /// The agent's own servers left out of its config: their command has no path outside the folder.
    no_path: Vec<String>,
}

/// What each pane was last told: its block id to what it was not given.
static TOLD: Mutex<Option<HashMap<String, NotGiven>>> = Mutex::new(None);

/// Whether `what` is news to the pane `block_id` (and records that it has been told).
fn is_news(block_id: &str, what: &NotGiven) -> bool {
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    let told = told.get_or_insert_with(HashMap::new);
    if told.get(block_id).is_some_and(|last| last == what) {
        return false;
    }
    told.insert(block_id.to_string(), what.clone());
    true
}

/// What the host agent on a pane with `meta` is not given of its folder, or `None` for a container agent and
/// for a folder the rule does not hold for. Reads the disk.
fn not_given(meta: &crate::backend::obj::MetaMapType) -> Option<NotGiven> {
    let folder = crate::backend::mcp_folder_rule::folder_of(meta)?;
    let ours = std::fs::read_to_string(crate::backend::mcp_config_file::host_config_path(&folder)).ok();
    Some(NotGiven {
        servers: crate::backend::mcp_folder_rule::left_out(&folder, ours.as_deref()),
        folder: crate::backend::host_agent_files::left_out(&folder),
        no_path: crate::backend::host_agent_files::dir_of(&folder).map(|dir| crate::backend::host_agent_files::servers_with_no_path(&dir)).unwrap_or_default(),
    })
}

/// Before a host agent's turn starts its CLI in such a folder: tell the pane what of the folder it is not
/// given, and what is off there, unless the pane has been told exactly this.
pub(crate) async fn tell(
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block: &Block,
) {
    let meta = block.meta.clone();
    let Some(what) = tokio::task::spawn_blocking(move || not_given(&meta)).await.ok().flatten() else { return };
    if !is_news(&block.oid, &what) {
        return;
    }
    tracing::warn!(block_id = %block.oid, servers = what.servers.len(), folder = ?what.folder, "this host agent takes nothing of its folder's own configuration: a container agent can write the folder");
    let provider = meta_get_string(&block.meta, "agentProvider", "claude");
    let frame = crate::backend::cli_notice::mcp_left_out_frame(&provider, &block.oid, &what.servers, &what.folder, &what.no_path);
    crate::backend::cli_notice::append_to_pane(broker, filestore, mstore, &block.oid, &frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pane_is_told_once_for_each_state_of_the_folder() {
        let with = |servers: &[&str], skills: usize| NotGiven {
            servers: servers.iter().map(|n| n.to_string()).collect(),
            folder: LeftOut { skills, ..LeftOut::default() },
            no_path: Vec::new(),
        };
        assert!(is_news("block-told-a", &with(&["one"], 0)));
        assert!(!is_news("block-told-a", &with(&["one"], 0)), "the same again is not news");
        assert!(is_news("block-told-a", &with(&["one", "two"], 0)), "another server is");
        assert!(is_news("block-told-a", &with(&["one", "two"], 1)), "and so is a skill added to the folder");
        assert!(is_news("block-told-b", &with(&["one"], 0)), "each pane is told for itself");
        assert!(is_news("block-told-c", &NotGiven::default()), "a folder with nothing of its own is still told what is off there");
    }

    #[test]
    fn what_is_not_given_is_the_folders_own_and_only_where_the_rule_holds() {
        let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = folder.path().to_string_lossy().into_owned();
        let pane = |mode: &str| -> crate::backend::obj::MetaMapType {
            [("agentMode", json!(mode)), ("agentProvider", json!("claude")), ("cmd:cwd", json!(dir))].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        };
        std::fs::write(folder.path().join(".mcp.json"), r#"{"mcpServers":{"agentmux":{"command":"x"},"theirs":{"command":"y"}}}"#).unwrap();
        std::fs::create_dir_all(folder.path().join(".claude")).unwrap();
        std::fs::write(folder.path().join(".claude").join("settings.local.json"), "{}").unwrap();
        std::fs::write(folder.path().join("CLAUDE.md"), "the folder's own").unwrap();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            assert_eq!(not_given(&pane("host")), None, "a folder no container agent has to do with: nothing is said");
            crate::backend::mcp_folder_rule::mark(&dir);
            let first = not_given(&pane("host")).unwrap();
            assert_eq!(first.servers, vec!["agentmux".to_string(), "theirs".to_string()], "no config of its own yet");
            assert_eq!(first.folder, LeftOut { settings: 1, instructions: 1, ..LeftOut::default() });
            let own = crate::backend::mcp_config_file::host_config_path(&dir);
            assert!(!own.starts_with(folder.path()), "the agent's config is outside the folder");
            crate::backend::mcp_config_file::write_relocated(&own, r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"}}}"#).unwrap();
            // A file just written can be unreadable for a moment on Windows (a scanner holds it): ask again.
            let mut after = not_given(&pane("host")).unwrap();
            for _ in 0..20 {
                if after.servers == vec!["theirs".to_string()] {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                after = not_given(&pane("host")).unwrap();
            }
            assert_eq!(after.servers, vec!["theirs".to_string()], "own config {}: {:?}", own.display(), std::fs::read_to_string(&own).map(|text| text.len()));
            assert_eq!(not_given(&pane("container")), None, "the container agent itself is not a host agent");
        });
    }
}
