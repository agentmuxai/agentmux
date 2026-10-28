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
//! The copies of those keys that are kept per agent UID (`..._by_uid`) are removed as well: a UID copy of an
//! exposed key would still verify, and the next use copies the new name-keyed key, or mints one.
//!
//! The WAN key too: an earlier version injected it into the same file, and the channel-wide `wan.db` key was
//! imported from that one. Both rows are removed, so the next signing mints a new key and srv's normal
//! publish puts it in the directory.
//!
//! Only an agent that has keys gets new ones (none is minted). Idempotent: a second run replaces them
//! again, which is harmless. An agent's key rows are keyed by its slug. A LAN peer that already pinned
//! an agent's old LAN key keeps flagging that agent's LAN messages until it clears the pin.

use std::sync::Arc;

use crate::backend::storage::agent_lan_keys::AgentLanKeypair;
use crate::backend::storage::host_lan_keys::{HostKey, HostKeyKind, HostLanKeys};
use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{self, WanIdentityStore};

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
        // `wan.db` is shared by every version of the channel. Without a path to it, or when it can't be
        // opened, the WAN key is left alone (and so is the legacy copy it would be imported from again).
        let wan_path = wan_identity::resolve_wan_identity_path();
        let wan = match &wan_path {
            Some(path) if path.exists() => match WanIdentityStore::open(path) {
                Ok(wan) => Wan::Store(wan),
                Err(e) => {
                    tracing::warn!(error = %e, "rotate_container_agent_keys: wan.db not opened; WAN keys not replaced");
                    Wan::Unavailable
                }
            },
            Some(_) => Wan::NoneYet,
            None => Wan::Unavailable,
        };
        // The host's LAN keys (shared by every channel): one that is still the exposed key goes with it.
        let host = crate::registry::resolve_identity_store_path().filter(|p| p.exists()).and_then(|p| match HostLanKeys::open(&p) {
            Ok(host) => Some(host),
            Err(e) => {
                tracing::warn!(error = %e, "rotate_container_agent_keys: the host's LAN key table did not open");
                None
            }
        });
        let rotated = rotate_container_keys(&store, &wan, host.as_ref()).map_err(MigrationError)?;
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

/// The channel's WAN keys, as far as this migration can reach them.
enum Wan {
    Store(WanIdentityStore),
    /// No `wan.db` yet: only the legacy copy a later import would use exists.
    NoneYet,
    Unavailable,
}

/// The `(id, slug)` of the agents that run, or ran, as container agents: those defined as one, and those a
/// pane still records as running in one (the store keeps no history of a definition's type, so an agent
/// switched to host since is found only by its pane).
fn container_agents(store: &Store) -> Result<Vec<(String, String)>, String> {
    let mut slugs: Vec<(String, String)> = Vec::new();
    for def in store.agent_def_list().map_err(|e| format!("rotate_container_agent_keys: read agents: {e}"))? {
        if def.agent_type == "container" {
            slugs.push((def.id, def.slug));
        }
    }
    let conn = store.conn().lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT data FROM db_block")
        .map_err(|e| format!("rotate_container_agent_keys: read db_block: {e}"))?;
    let datas: Vec<Vec<u8>> = stmt
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|e| format!("rotate_container_agent_keys: read db_block: {e}"))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);
    drop(conn);
    for data in datas {
        let Ok(block) = serde_json::from_slice::<serde_json::Value>(&data) else { continue };
        let meta = &block["meta"];
        if meta["agentMode"].as_str() != Some("container") {
            continue;
        }
        if let Some(def) = meta["agentId"].as_str().and_then(|id| store.agent_def_get(id).ok().flatten()) {
            slugs.push((def.id, def.slug));
        }
    }
    slugs.retain(|(_, slug)| !slug.trim().is_empty());
    slugs.sort();
    slugs.dedup();
    Ok(slugs)
}

/// A new LAN keypair, as the stores mint them.
fn fresh_lan_pair() -> AgentLanKeypair {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let mut seed = [0u8; 32];
    seed[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    seed[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let (public, private) = agentmux_common::jekt_sign::generate_lan_keypair(seed);
    AgentLanKeypair { public_key: BASE64.encode(public), private_key: BASE64.encode(private) }
}

/// Replace the keys of each container agent; how many agents had any.
///
/// `host` is the host's LAN key table. A key there that is still the exposed one is replaced by a new
/// one, so the next lookup cannot adopt it back; a key another channel put there is not the exposed one
/// and stays, so no other channel's key changes.
fn rotate_container_keys(store: &Store, wan: &Wan, host: Option<&HostLanKeys>) -> Result<usize, String> {
    let mut rotated = 0;
    for (id, slug) in container_agents(store)? {
        let old_name = store.agent_lan_key_load(&slug).map_err(|e| format!("rotate_container_agent_keys: LAN key of {slug}: {e}"))?;
        let old_uid = store.agent_uid_lan_public_key_load(&id).map_err(|e| format!("rotate_container_agent_keys: UID LAN key of {slug}: {e}"))?;
        // The host's keys first, with new keys made here: a failure leaves this channel untouched, so a
        // retry still finds the exposed keys to match. (Replacing them afterwards lost them on a retry.)
        // The channel's own rotation below makes keys of its own; once the host table is attached the
        // channel follows the host's. An empty host slot is never left for a channel that still holds the
        // exposed key to fill.
        if let Some(host) = host {
            let now = agentmux_common::time::now_secs();
            if let Some(old) = &old_name {
                let new = HostKey { pair: fresh_lan_pair(), created_at: now };
                host.supersede(HostKeyKind::Name, &slug.to_lowercase(), &old.public_key, Some(&new))
                    .map_err(|e| format!("rotate_container_agent_keys: host LAN key of {slug}: {e}"))?;
            }
            if let Some(old) = &old_uid {
                let new = HostKey { pair: fresh_lan_pair(), created_at: now };
                host.supersede(HostKeyKind::Uid, &id, old, Some(&new))
                    .map_err(|e| format!("rotate_container_agent_keys: host UID LAN key of {slug}: {e}"))?;
            }
        }
        let uid_copies = store
            .agent_uid_keys_delete(&id)
            .map_err(|e| format!("rotate_container_agent_keys: UID keys of {slug}: {e}"))?;
        let wan_removed = match wan {
            Wan::Store(wan) => {
                let current = wan
                    .agent_keys_delete(std::slice::from_ref(&slug))
                    .map_err(|e| format!("rotate_container_agent_keys: WAN key of {slug}: {e}"))?;
                // After the `wan.db` row, which would otherwise be imported from this one again.
                let legacy = store
                    .agent_wan_key_delete(&slug)
                    .map_err(|e| format!("rotate_container_agent_keys: legacy WAN key of {slug}: {e}"))?;
                current > 0 || legacy
            }
            Wan::NoneYet => store
                .agent_wan_key_delete(&slug)
                .map_err(|e| format!("rotate_container_agent_keys: legacy WAN key of {slug}: {e}"))?,
            Wan::Unavailable => false,
        };
        let jekt = store
            .agent_jekt_key_rotate(&slug)
            .map_err(|e| format!("rotate_container_agent_keys: jekt key of {slug}: {e}"))?;
        let lan = store
            .agent_lan_key_rotate(&slug)
            .map_err(|e| format!("rotate_container_agent_keys: LAN key of {slug}: {e}"))?;
        rotated += usize::from(jekt || lan || wan_removed || uid_copies);
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

        assert_eq!(rotate_container_keys(&store, &Wan::Unavailable, None).unwrap(), 1);

        assert_ne!(store.agent_jekt_key_load(&boxed).unwrap().unwrap(), boxed_jekt);
        assert_ne!(store.agent_lan_key_load(&boxed).unwrap().unwrap().private_key, boxed_lan.private_key);
        assert_eq!(store.agent_jekt_key_load(&host).unwrap().unwrap(), host_jekt, "a host agent keeps its keys");
        assert_eq!(store.agent_lan_key_load(&host).unwrap().unwrap(), host_lan);
    }

    #[test]
    fn the_per_uid_copies_of_a_container_agents_keys_are_replaced_too() {
        let store = Store::open_in_memory().unwrap();
        let boxed = agent(&store, "Boxed", "container");
        let host = agent(&store, "Hosty", "host");
        let id_of = |slug: &str| store.agent_def_list().unwrap().into_iter().find(|d| d.slug == slug).unwrap().id;
        let (boxed_id, host_id) = (id_of(&boxed), id_of(&host));
        let boxed_before = store.agent_uid_keys_ensure(&boxed_id).unwrap().unwrap().0;
        let host_before = store.agent_uid_keys_ensure(&host_id).unwrap().unwrap().0;

        assert_eq!(rotate_container_keys(&store, &Wan::Unavailable, None).unwrap(), 1);

        let boxed_after = store.agent_uid_keys_ensure(&boxed_id).unwrap().unwrap().0;
        assert_ne!(boxed_after.lan.private_key, boxed_before.lan.private_key, "the exposed UID key no longer verifies");
        assert_ne!(boxed_after.wan.private_key, boxed_before.wan.private_key);
        assert_eq!(store.agent_uid_keys_ensure(&host_id).unwrap().unwrap().0.lan.private_key, host_before.lan.private_key);
    }

    #[test]
    fn an_agent_switched_to_host_is_found_by_the_pane_that_ran_it_as_a_container() {
        use crate::backend::obj::Block;
        let store = Store::open_in_memory().unwrap();
        let switched = agent(&store, "Switched", "host");
        let (id, other) = (store.agent_def_list().unwrap().iter().find(|d| d.slug == switched).unwrap().id.clone(), agent(&store, "Other", "host"));
        store.agent_jekt_key_ensure(&switched).unwrap();
        let before = store.agent_jekt_key_ensure(&other).unwrap();
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        block.meta = serde_json::json!({ "agentMode": "container", "agentId": id }).as_object().unwrap().clone().into_iter().collect();
        store.insert(&mut block).unwrap();

        assert_eq!(rotate_container_keys(&store, &Wan::Unavailable, None).unwrap(), 1);
        assert_eq!(store.agent_jekt_key_load(&other).unwrap().unwrap(), before, "an agent no pane ran as a container is left alone");
    }

    #[test]
    fn a_container_agents_wan_key_and_the_legacy_copy_it_came_from_are_replaced() {
        let store = Store::open_in_memory().unwrap();
        let (dir, wan) = crate::backend::storage::wan_identity::attach_temp_wan_identity(&store);
        let wan_path = dir.path().join("wan.db");
        let boxed = agent(&store, "Boxed", "container");
        let host = agent(&store, "Hosty", "host");
        let legacy = store.agent_wan_key_ensure(&boxed).unwrap();
        store.agent_wan_key_ensure(&host).unwrap();
        let exposed = store.channel_wan_key(&wan, &boxed).unwrap();
        assert_eq!(exposed.private_key, legacy.private_key, "wan.db took the legacy key");
        let host_before = store.channel_wan_key(&wan, &host).unwrap();

        let reopened = Wan::Store(WanIdentityStore::open(&wan_path).unwrap());
        assert_eq!(rotate_container_keys(&store, &reopened, None).unwrap(), 1);

        let fresh = store.channel_wan_key(&wan, &boxed).unwrap();
        assert_ne!(fresh.private_key, exposed.private_key, "a new key, not the exposed one imported again");
        assert!(store.agent_wan_key_load(&boxed).unwrap().is_none());
        assert_eq!(store.channel_wan_key(&wan, &host).unwrap().private_key, host_before.private_key);
    }

    #[test]
    fn without_a_wan_db_yet_the_legacy_copy_is_removed_so_it_is_never_imported() {
        let store = Store::open_in_memory().unwrap();
        let boxed = agent(&store, "Boxed", "container");
        store.agent_wan_key_ensure(&boxed).unwrap();
        assert_eq!(rotate_container_keys(&store, &Wan::NoneYet, None).unwrap(), 1);
        assert!(store.agent_wan_key_load(&boxed).unwrap().is_none());
    }

    #[test]
    fn a_container_agent_with_no_keys_gets_none_minted() {
        let store = Store::open_in_memory().unwrap();
        let boxed = agent(&store, "Boxed", "container");
        assert_eq!(rotate_container_keys(&store, &Wan::Unavailable, None).unwrap(), 0);
        assert!(store.agent_jekt_key_load(&boxed).unwrap().is_none());
        assert!(store.agent_lan_key_load(&boxed).unwrap().is_none());
    }

    fn host_table() -> (tempfile::TempDir, HostLanKeys) {
        let dir = tempfile::tempdir().unwrap();
        let host = HostLanKeys::open(&dir.path().join("identity-store.db")).unwrap();
        (dir, host)
    }

    fn host_key(host: &HostLanKeys, kind: HostKeyKind, subject: &str) -> Option<HostKey> {
        host.claim(kind, subject, None).unwrap()
    }

    #[test]
    fn an_exposed_key_the_host_holds_is_replaced_there_too() {
        let store = Store::open_in_memory().unwrap();
        let (_dir, host) = host_table();
        let boxed = agent(&store, "Boxed", "container");
        let id = store.agent_def_list().unwrap().into_iter().find(|d| d.slug == boxed).unwrap().id;
        let exposed = store.agent_lan_key_ensure(&boxed).unwrap();
        let uid_key = store.agent_uid_keys_ensure(&id).unwrap().unwrap().0.lan;
        let exposed_uid_public = uid_key.public_key.clone();
        let now = agentmux_common::time::now_secs();
        host.replace(HostKeyKind::Name, &boxed, &HostKey { pair: exposed.clone(), created_at: now }).unwrap();
        let uid_pair = AgentLanKeypair { public_key: uid_key.public_key, private_key: uid_key.private_key };
        host.replace(HostKeyKind::Uid, &id, &HostKey { pair: uid_pair, created_at: now }).unwrap();

        assert_eq!(rotate_container_keys(&store, &Wan::Unavailable, Some(&host)).unwrap(), 1);

        let fresh = store.agent_lan_key_load(&boxed).unwrap().unwrap();
        assert_ne!(fresh, exposed);
        let on_host = host_key(&host, HostKeyKind::Name, &boxed).expect("the slot is filled");
        assert_ne!(on_host.pair, exposed, "the exposed key is off the host");
        let uid_on_host = host_key(&host, HostKeyKind::Uid, &id).expect("the slot is filled, not left empty for a stale channel to reclaim");
        assert_ne!(uid_on_host.pair.public_key, exposed_uid_public, "and not the exposed UID key either");
    }

    #[test]
    fn a_host_that_cannot_be_updated_fails_the_migration_before_anything_is_rotated() {
        let store = Store::open_in_memory().unwrap();
        let (dir, host) = host_table();
        let boxed = agent(&store, "Boxed", "container");
        let exposed = store.agent_lan_key_ensure(&boxed).unwrap();
        let now = agentmux_common::time::now_secs();
        host.replace(HostKeyKind::Name, &boxed, &HostKey { pair: exposed.clone(), created_at: now }).unwrap();
        host.disconnect();
        drop(dir);

        assert!(rotate_container_keys(&store, &Wan::Unavailable, Some(&host)).is_err());
        assert_eq!(store.agent_lan_key_load(&boxed).unwrap().unwrap(), exposed, "nothing was rotated, so a retry still finds the exposed key");
    }

    #[test]
    fn a_key_another_channel_put_on_the_host_is_not_the_exposed_one_and_stays() {
        let store = Store::open_in_memory().unwrap();
        let (_dir, host) = host_table();
        let boxed = agent(&store, "Boxed", "container");
        store.agent_lan_key_ensure(&boxed).unwrap();
        let other = Store::open_in_memory().unwrap().agent_lan_key_ensure(&boxed).unwrap();
        let now = agentmux_common::time::now_secs();
        host.replace(HostKeyKind::Name, &boxed, &HostKey { pair: other.clone(), created_at: now }).unwrap();

        rotate_container_keys(&store, &Wan::Unavailable, Some(&host)).unwrap();

        assert_eq!(host_key(&host, HostKeyKind::Name, &boxed).unwrap().pair, other, "no other channel's key changes");
    }
}
