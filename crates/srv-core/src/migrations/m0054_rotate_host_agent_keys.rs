// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Replace the signing keys of every host agent, once.
//!
//! The start file an agent's MCP server is started from no longer holds a signing value (it holds
//! references, and the server asks srv for its keys with its own token). An earlier version wrote the
//! values into that file, where anything that could read the agent's workspace could read them. A key that
//! has been in such a file is no longer private, so each host agent's jekt and LAN keys, and the WAN key an
//! earlier version also wrote there, are replaced with new ones that are written to no file. A start file
//! written by an older version is rewritten at the agent's next spawn, which also removes the old values.
//!
//! The same replacement as `m0043` makes for container agents, for the agents that migration leaves alone:
//! the per-UID copies, the host's LAN table slot for each key, and the WAN rows, so the next use makes new
//! ones and srv's normal publish puts the new WAN key in the directory.
//!
//! Only an agent that has keys gets new ones (none is minted). Idempotent: a second run replaces them
//! again, which is harmless. A LAN peer that already pinned an agent's old LAN key keeps flagging that
//! agent's LAN messages until it clears the pin.

use std::sync::Arc;

use crate::backend::storage::store::Store;

use super::m0043_rotate_container_agent_keys::{open_host, open_wan, rotate_agent_keys, Wan};
use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0054RotateHostAgentKeys;

impl Migration for M0054RotateHostAgentKeys {
    fn id(&self) -> &'static str { "0054_rotate_host_agent_keys" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Replace the signing keys of host agents, which an earlier start file carried as values"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let store = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("rotate_host_agent_keys: open store: {e}")))?,
        );
        let wan = open_wan();
        let host = open_host()?;
        let rotated = rotate_host_keys(&store, &wan, host.as_ref()).map_err(MigrationError)?;
        if rotated > 0 {
            tracing::info!(rotated, "rotate_host_agent_keys: replaced host agents' signing keys");
        }
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.channel_store_path.exists() {
            return VerifyOutcome::Ok("no channel store".into());
        }
        match Store::open(&ctx.channel_store_path).map(|s| s.agent_def_list()) {
            Ok(Ok(defs)) => VerifyOutcome::Ok(format!("{} agent definition(s) read", defs.len())),
            Ok(Err(e)) | Err(e) => VerifyOutcome::Error(format!("read agent definitions: {e}")),
        }
    }
}

/// The `(id, slug)` of every agent that is not a container agent. A container agent's keys were replaced by
/// `m0043`, and no start file of it has held a value since.
fn host_agents(store: &Store) -> Result<Vec<(String, String)>, String> {
    let mut agents: Vec<(String, String)> = store
        .agent_def_list()
        .map_err(|e| format!("rotate_host_agent_keys: read agents: {e}"))?
        .into_iter()
        .filter(|def| !def.runtime().is_container() && !def.slug.trim().is_empty())
        .map(|def| (def.id, def.slug))
        .collect();
    agents.sort();
    agents.dedup();
    Ok(agents)
}

/// Replace the keys of each host agent; how many agents had any.
fn rotate_host_keys(store: &Store, wan: &Wan, host: Option<&crate::backend::storage::host_lan_keys::HostLanKeys>) -> Result<usize, String> {
    rotate_agent_keys(store, wan, host, host_agents(store)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::AgentDefinition;

    fn agent(store: &Store, name: &str, agent_type: &str) -> String {
        let mut def: AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "name": name, "icon": "", "provider": "claude", "description": "",
            "created_at": 0, "agent_type": agent_type,
        }))
        .unwrap();
        store.agent_def_insert(&mut def).unwrap();
        def.slug
    }

    #[test]
    fn a_host_agents_keys_are_replaced_and_a_container_agents_are_left_to_the_earlier_migration() {
        let store = Store::open_in_memory().unwrap();
        let hosty = agent(&store, "Hosty", "host");
        let boxed = agent(&store, "Boxed", "container");
        let (jekt, lan) = (store.agent_jekt_key_ensure(&hosty).unwrap(), store.agent_lan_key_ensure(&hosty).unwrap());
        let boxed_jekt = store.agent_jekt_key_ensure(&boxed).unwrap();

        assert_eq!(rotate_host_keys(&store, &Wan::Unavailable, None).unwrap(), 1);

        assert_ne!(store.agent_jekt_key_load(&hosty).unwrap().unwrap(), jekt, "a value an old start file held no longer verifies");
        assert_ne!(store.agent_lan_key_load(&hosty).unwrap().unwrap().private_key, lan.private_key);
        assert_eq!(store.agent_jekt_key_load(&boxed).unwrap().unwrap(), boxed_jekt, "this migration leaves a container agent to m0043");
    }

    #[test]
    fn the_per_uid_copies_and_the_wan_rows_of_a_host_agent_are_replaced_too() {
        let store = Store::open_in_memory().unwrap();
        let (dir, wan) = crate::backend::storage::wan_identity::attach_temp_wan_identity(&store);
        let hosty = agent(&store, "Hosty", "host");
        let id = store.agent_def_list().unwrap().into_iter().find(|d| d.slug == hosty).unwrap().id;
        store.agent_jekt_key_ensure(&hosty).unwrap();
        let uid_before = store.agent_uid_keys_ensure(&id).unwrap().unwrap().0;
        let wan_before = store.channel_wan_key(&wan, &hosty).unwrap();

        let reopened = Wan::Store(crate::backend::storage::wan_identity::WanIdentityStore::open(&dir.path().join("wan.db")).unwrap());
        assert_eq!(rotate_host_keys(&store, &reopened, None).unwrap(), 1);

        let uid_after = store.agent_uid_keys_ensure(&id).unwrap().unwrap().0;
        assert_ne!(uid_after.lan.private_key, uid_before.lan.private_key);
        assert_ne!(store.channel_wan_key(&wan, &hosty).unwrap().private_key, wan_before.private_key);
    }

    #[test]
    fn a_host_agent_with_no_keys_gets_none_minted() {
        let store = Store::open_in_memory().unwrap();
        let hosty = agent(&store, "Hosty", "host");
        assert_eq!(rotate_host_keys(&store, &Wan::Unavailable, None).unwrap(), 0);
        assert!(store.agent_jekt_key_load(&hosty).unwrap().is_none());
        assert!(store.agent_lan_key_load(&hosty).unwrap().is_none());
    }
}
