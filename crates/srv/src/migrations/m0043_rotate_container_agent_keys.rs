// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Replace the signing keys of every container agent, once.
//!
//! A container agent's workspace is mounted into its container, and an earlier launch wrote that agent's
//! signing keys into the `.mcp.json` there. A launch no longer does, and the next start of the container
//! takes them out of the file; but a key a container could read is no longer private, so each is replaced
//! with a fresh one that is written nowhere a container can see. srv signs and attributes a container
//! agent's messages itself, by its token, so nothing needs the old keys.
//!
//! Only an agent that has keys gets new ones (none is minted). Idempotent: a second run replaces them
//! again, which is harmless. An agent's key rows are keyed by its slug.

use std::sync::Arc;

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0043RotateContainerAgentKeys;

impl Migration for M0043RotateContainerAgentKeys {
    fn id(&self) -> &'static str { "0043_rotate_container_agent_keys" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Replace the signing keys of container agents, whose workspace a container could read them from"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let store = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("rotate_container_agent_keys: open store: {e}")))?,
        );
        let rotated = rotate_container_keys(&store).map_err(MigrationError)?;
        if rotated > 0 {
            tracing::info!(rotated, "rotate_container_agent_keys: replaced container agents' signing keys");
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

/// Replace the keys of each container agent; how many agents had any.
fn rotate_container_keys(store: &Store) -> Result<usize, String> {
    let defs = store.agent_def_list().map_err(|e| format!("rotate_container_agent_keys: read agents: {e}"))?;
    let mut rotated = 0;
    for def in defs.iter().filter(|d| d.agent_type == "container" && !d.slug.trim().is_empty()) {
        let jekt = store
            .agent_jekt_key_rotate(&def.slug)
            .map_err(|e| format!("rotate_container_agent_keys: jekt key of {}: {e}", def.slug))?;
        let lan = store
            .agent_lan_key_rotate(&def.slug)
            .map_err(|e| format!("rotate_container_agent_keys: LAN key of {}: {e}", def.slug))?;
        rotated += usize::from(jekt || lan);
    }
    Ok(rotated)
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
    fn only_container_agents_get_new_keys() {
        let store = Store::open_in_memory().unwrap();
        let boxed = agent(&store, "Boxed", "container");
        let host = agent(&store, "Hosty", "host");
        let (boxed_jekt, host_jekt) = (store.agent_jekt_key_ensure(&boxed).unwrap(), store.agent_jekt_key_ensure(&host).unwrap());
        let (boxed_lan, host_lan) = (store.agent_lan_key_ensure(&boxed).unwrap(), store.agent_lan_key_ensure(&host).unwrap());

        assert_eq!(rotate_container_keys(&store).unwrap(), 1);

        assert_ne!(store.agent_jekt_key_load(&boxed).unwrap().unwrap(), boxed_jekt);
        assert_ne!(store.agent_lan_key_load(&boxed).unwrap().unwrap().private_key, boxed_lan.private_key);
        assert_eq!(store.agent_jekt_key_load(&host).unwrap().unwrap(), host_jekt, "a host agent keeps its keys");
        assert_eq!(store.agent_lan_key_load(&host).unwrap().unwrap(), host_lan);
    }

    #[test]
    fn a_container_agent_with_no_keys_gets_none_minted() {
        let store = Store::open_in_memory().unwrap();
        let boxed = agent(&store, "Boxed", "container");
        assert_eq!(rotate_container_keys(&store).unwrap(), 0);
        assert!(store.agent_jekt_key_load(&boxed).unwrap().is_none());
        assert!(store.agent_lan_key_load(&boxed).unwrap().is_none());
    }
}
