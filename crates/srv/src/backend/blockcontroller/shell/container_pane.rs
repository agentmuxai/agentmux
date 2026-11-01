// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! A container agent's Shell drawer runs in its container, never on the host.
//!
//! The drawer's shell is a sub-block of the agent's pane. When that pane belongs to a container agent
//! (`agentMode` "container"), the shell is `docker exec -it <container> bash` in the agent's workspace
//! mount, so what the person types runs where the agent's own commands run. If docker or the container is
//! not there, the exec fails and says so in the terminal: it never falls back to a host shell.

use portable_pty::CommandBuilder;

use crate::backend::container::{container_of_meta, CONTAINER_WORKSPACE_DIR};
use crate::backend::obj;
use crate::backend::storage::store::Store;

/// The container a shell sub-block belongs in: that of the container agent whose pane is its parent.
pub(super) fn container_for(store: &Store, block_id: &str) -> Option<String> {
    let block = store.get::<obj::Block>(block_id).ok().flatten()?;
    let parent = store.get::<obj::Block>(block.parentoref.strip_prefix("block:")?).ok().flatten()?;
    container_of_meta(&parent.meta)
}

fn exec_args(container: &str) -> Vec<String> {
    ["exec", "-it", "-w", CONTAINER_WORKSPACE_DIR, "-e", "TERM=xterm-256color", "-e", "COLORTERM=truecolor", container, "bash"]
        .map(String::from)
        .to_vec()
}

/// `docker exec` into `container`, run on a PTY like an SSH pane's `ssh`.
pub(super) fn command(container: &str, block_id: &str) -> CommandBuilder {
    tracing::info!(block_id, container, "container shell spawn");
    let mut c = CommandBuilder::new("docker");
    c.args(exec_args(container));
    crate::backend::pane_env::sanitize_pty_command(&mut c);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;

    fn meta(pairs: &[(&str, &str)]) -> MetaMapType {
        pairs.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect()
    }

    #[test]
    fn a_container_agents_pane_names_its_container() {
        let m = meta(&[("agentMode", "container"), ("agentId", "3f2a-agent")]);
        assert_eq!(container_of_meta(&m).as_deref(), Some("agentmux-3f2a-agent"));
    }

    #[test]
    fn a_native_agent_or_a_pane_with_no_agent_gets_no_container() {
        assert_eq!(container_of_meta(&meta(&[("agentMode", "native"), ("agentId", "a")])), None);
        assert_eq!(container_of_meta(&meta(&[("agentId", "a")])), None);
        assert_eq!(container_of_meta(&meta(&[("agentMode", "container")])), None);
    }

    fn block(store: &Store, parent: &str, meta: MetaMapType) -> String {
        let oid = uuid::Uuid::new_v4().to_string();
        let mut b = obj::Block { oid: oid.clone(), parentoref: parent.to_string(), version: 1, runtimeopts: None, stickers: None, meta, subblockids: None };
        store.insert(&mut b).unwrap();
        oid
    }

    /// The drawer's shell is a sub-block of the agent's pane: a container agent's gets the container, a
    /// native agent's and a plain pane's do not.
    #[test]
    fn the_shell_sub_block_of_a_container_agents_pane_runs_in_its_container() {
        let store = Store::open_in_memory().unwrap();
        let container_pane = block(&store, "tab:t1", meta(&[("view", "agent"), ("agentMode", "container"), ("agentId", "a1")]));
        let native_pane = block(&store, "tab:t1", meta(&[("view", "agent"), ("agentMode", "native"), ("agentId", "a2")]));
        let shell_of = |pane: &str| block(&store, &format!("block:{pane}"), meta(&[("view", "term"), ("controller", "shell")]));
        assert_eq!(container_for(&store, &shell_of(&container_pane)).as_deref(), Some("agentmux-a1"));
        assert_eq!(container_for(&store, &shell_of(&native_pane)), None);
        assert_eq!(container_for(&store, &block(&store, "tab:t1", meta(&[("view", "term")]))), None, "a pane in a tab is not a drawer shell");
    }

    #[test]
    fn the_shell_is_an_interactive_exec_in_the_workspace_mount() {
        let args = exec_args("agentmux-x");
        assert_eq!(&args[..4], ["exec", "-it", "-w", "/workspace"]);
        assert_eq!(&args[args.len() - 2..], ["agentmux-x", "bash"], "the container, then the shell, last");
    }
}
