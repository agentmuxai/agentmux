// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A terminal-tier agent (a provider whose CLI runs in a term pane) starts
//! from its block's own `cmd:env`, written when the pane was opened. After a
//! rename that snapshot names the old id, so the spawn takes the agent's
//! canonical identity over it, as every other agent spawn does
//! (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.3, `agent_handlers::identity_env`),
//! and realigns the workspace `.mcp.json`.

use crate::backend::obj::MetaMapType;

use super::controller::ShellController;

impl ShellController {
    /// `block_meta` with its `cmd:env` carrying the agent's canonical
    /// identity, when this block shows an agent whose row has a slug.
    /// Anything else (a plain terminal, an agent's PtyShell child, a block
    /// with no store) gets its meta unchanged.
    pub(super) fn with_agent_identity(&self, block_meta: &MetaMapType) -> MetaMapType {
        let mut meta = block_meta.clone();
        let Some(mstore) = &self.mstore else {
            return meta;
        };
        let Some(slug) = crate::server::agent_handlers::identity_env::block_agent_slug(mstore, &self.block_id) else {
            return meta;
        };
        let mut env = super::super::cmd_env_of(block_meta);
        crate::server::agent_handlers::identity_env::carry_slug_identity_env(&mut env, &slug);
        meta.insert(super::super::META_KEY_CMD_ENV.to_string(), serde_json::json!(env));
        let meta_str = |key, default| crate::backend::obj::meta_get_string(block_meta, key, default);
        let (cwd, mode) = (meta_str(super::super::META_KEY_CMD_CWD, ""), meta_str("agentMode", "host"));
        crate::backend::mcp_json_identity::realign_workspace_mcp_json(mstore, &cwd, &mode, &slug);
        meta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;
    use std::sync::Arc;

    fn block(store: &Store, id: &str, agent_id: Option<&str>, env: serde_json::Value) -> MetaMapType {
        let mut meta = MetaMapType::new();
        if let Some(agent) = agent_id {
            meta.insert("agentId".to_string(), serde_json::json!(agent));
        }
        meta.insert("cmd:env".to_string(), env);
        let mut b = crate::backend::obj::Block { oid: id.to_string(), meta: meta.clone(), ..Default::default() };
        store.insert(&mut b).unwrap();
        meta
    }

    fn controller(store: &Arc<Store>, block_id: &str) -> ShellController {
        ShellController::new("cmd".into(), "tab".into(), block_id.into(), None, None, Some(store.clone()), None, String::new())
    }

    /// P0.3 for a terminal-tier agent: its pane's persisted `cmd:env` names
    /// the old id; the spawn's names the row's slug. A plain terminal's is
    /// left as it is.
    #[test]
    fn a_terminal_agent_pane_spawns_with_its_current_identity() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut def = crate::backend::storage::agents::test_agent_def("uid-term", "Term Agent", "claude", "host", 1, "");
        def.slug = "term-agent".to_string();
        store.agent_def_insert(&mut def).unwrap();
        let stale = serde_json::json!({ "AGENTMUX_AGENT_ID": "old-id", "AGENTMUX_AGENT_SLUG": "old-id", "KEEP": "1" });
        let agent_meta = block(&store, "block-term-agent", Some("uid-term"), stale.clone());
        let plain_meta = block(&store, "block-plain", None, stale.clone());

        let env = crate::backend::blockcontroller::cmd_env_of(&controller(&store, "block-term-agent").with_agent_identity(&agent_meta));
        for key in ["AGENTMUX_AGENT_ID", "AGENTMUX_AGENT_SLUG", "MUXBUS_AGENT_ID"] {
            assert_eq!(env[key], "term-agent", "{key}");
        }
        assert_eq!(env["KEEP"], "1");

        let plain = controller(&store, "block-plain").with_agent_identity(&plain_meta);
        assert_eq!(plain, plain_meta, "a plain terminal's meta is unchanged");
    }
}
