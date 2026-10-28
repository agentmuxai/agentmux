// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's LAN key is the host's: a channel switch, or two channels on one host, keep one key.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::params;

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::store::Store;

const ROW_MS: i64 = 1_700_000_000_000; // before any key minted by a test run

fn host_table() -> (tempfile::TempDir, Arc<HostLanKeys>) {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(HostLanKeys::open(&dir.path().join("identity-store.db")).unwrap());
    (dir, host)
}

/// A channel store (its own `objects.db`), on the host table when given one.
fn channel(host: Option<&Arc<HostLanKeys>>) -> Store {
    let store = Store::open_in_memory().unwrap();
    if let Some(host) = host {
        store.set_host_lan_keys(host.clone());
    }
    store
}

/// What this channel itself holds under `name`, without asking the host.
fn own_row(store: &Store, name: &str) -> Option<AgentLanKeypair> {
    let conn = store.conn.lock().unwrap();
    conn.query_row(
        "SELECT public_key, private_key FROM db_agent_lan_keys WHERE agent_id = ?1",
        params![name],
        |r| Ok(AgentLanKeypair { public_key: r.get(0)?, private_key: r.get(1)? }),
    )
    .ok()
}

fn row(store: &Store, uid: &str, slug: &str) {
    let mut def = test_agent_def(uid, slug, "claude", "agent", ROW_MS, "");
    def.slug = slug.to_string();
    store.agent_def_insert_local_only(&mut def, None).unwrap();
    let conn = store.conn.lock().unwrap();
    conn.execute("UPDATE db_agents SET slug = ?2, created_at = ?3 WHERE id = ?1", params![uid, slug, ROW_MS]).unwrap();
}

#[test]
fn a_channel_switch_keeps_the_agents_key() {
    let (_dir, host) = host_table();
    let stable = channel(Some(&host));
    let first = stable.agent_lan_key_ensure("maop").unwrap();
    // The agent now runs under another channel's store: a store that never saw it.
    let local_build = channel(Some(&host));
    assert!(own_row(&local_build, "maop").is_none());
    let second = local_build.agent_lan_key_ensure("Maop").unwrap();
    assert_eq!(first, second, "the same key, not a fresh one");
    assert_eq!(own_row(&local_build, "maop"), Some(first.clone()), "and this channel's row follows it");
    assert_eq!(local_build.agent_lan_public_key_load("maop").unwrap(), Some(first.public_key.clone()));
    assert_eq!(local_build.agent_lan_key_load("maop").unwrap(), Some(first));
}

#[test]
fn two_channels_on_one_host_sign_the_same() {
    let (_dir, host) = host_table();
    let (a, b) = (channel(Some(&host)), channel(Some(&host)));
    let from_a = a.agent_lan_key_ensure("clare").unwrap();
    let from_b = b.agent_lan_key_ensure("clare").unwrap();
    let sig = agentmux_common::jekt_sign::sign_lan_jekt(
        &BASE64.decode(&from_a.private_key).unwrap(),
        "msg-1",
        "clare",
        "lazo",
        1_000,
        "hello",
    )
    .unwrap();
    assert!(
        agentmux_common::jekt_sign::verify_lan_jekt(
            &BASE64.decode(&from_b.public_key).unwrap(),
            "msg-1",
            "clare",
            "lazo",
            1_000,
            "hello",
            &sig
        ),
        "what one channel signs, the key the other publishes verifies"
    );
}

#[test]
fn an_old_per_channel_key_migrates_and_the_first_channel_to_run_keeps_its_own() {
    // Two channels minted a key each, before there was a host table.
    let (a, b) = (channel(None), channel(None));
    let key_a = a.agent_lan_key_ensure("maop").unwrap();
    let key_b = b.agent_lan_key_ensure("maop").unwrap();
    assert_ne!(key_a, key_b);
    // The build with the host table arrives. Channel A runs first: its key is published, and kept.
    let (_dir, host) = host_table();
    a.set_host_lan_keys(host.clone());
    b.set_host_lan_keys(host.clone());
    assert_eq!(a.agent_lan_key_ensure("maop").unwrap(), key_a, "no host key yet: the channel's own is kept");
    // Channel B finds a host key, and adopts it over its own: never a second key for the agent.
    assert_eq!(b.agent_lan_key_ensure("maop").unwrap(), key_a);
    assert_eq!(own_row(&b, "maop"), Some(key_a.clone()));
    assert_eq!(host.claim(HostKeyKind::Name, "maop", None).unwrap().map(|k| k.pair), Some(key_a));
}

#[test]
fn a_rotation_reaches_every_channel() {
    let (_dir, host) = host_table();
    let (a, b) = (channel(Some(&host)), channel(Some(&host)));
    let old = a.agent_lan_key_ensure("opaz").unwrap();
    assert_eq!(b.agent_lan_key_ensure("opaz").unwrap(), old);
    assert!(a.agent_lan_key_rotate("opaz").unwrap());
    let new = a.agent_lan_key_load("opaz").unwrap().unwrap();
    assert_ne!(new, old);
    assert_eq!(b.agent_lan_key_ensure("opaz").unwrap(), new, "the other channel signs with the new key");
}

#[test]
fn without_a_host_table_each_channel_keeps_its_own_keys() {
    let (a, b) = (channel(None), channel(None));
    assert_ne!(a.agent_lan_key_ensure("maop").unwrap(), b.agent_lan_key_ensure("maop").unwrap());
}

#[test]
fn a_host_table_that_fails_leaves_the_channels_own_key() {
    let (dir, host) = host_table();
    let store = channel(Some(&host));
    let own = store.agent_lan_key_ensure("maop").unwrap();
    // The table goes: the connection fails, and a later call finds no way to open it either.
    {
        let mut slot = host.conn.lock().unwrap();
        *slot = None;
    }
    drop(dir);
    assert_eq!(store.agent_lan_key_ensure("maop").unwrap(), own);
}

#[test]
fn the_uid_key_is_the_hosts_too() {
    let (_dir, host) = host_table();
    let (a, b) = (channel(Some(&host)), channel(Some(&host)));
    row(&a, "uid-maop", "maop");
    row(&b, "uid-maop", "maop");
    let (keys_a, _) = a.agent_uid_keys_ensure("uid-maop").unwrap().unwrap();
    let (keys_b, _) = b.agent_uid_keys_ensure("uid-maop").unwrap().unwrap();
    assert_eq!(keys_a.lan, keys_b.lan, "the same v2 LAN key in both channels");
    assert_ne!(keys_a.wan, keys_b.wan, "WAN keys are still the channel's");
    assert_eq!(b.agent_uid_lan_public_key_load("uid-maop").unwrap(), Some(keys_a.lan.public_key));
}

#[test]
fn the_uid_key_copies_the_hosts_name_key() {
    let (_dir, host) = host_table();
    let (a, b) = (channel(Some(&host)), channel(Some(&host)));
    let name_key = a.agent_lan_key_ensure("maop").unwrap();
    // B never saw the agent's name key; it has the row, and takes the host's key from the name.
    row(&b, "uid-maop", "maop");
    let (keys, _) = b.agent_uid_keys_ensure("uid-maop").unwrap().unwrap();
    assert_eq!(keys.lan.public_key, name_key.public_key, "the UID key is the name key, as it is on one channel");
}

#[test]
fn deleting_an_agent_drops_the_hosts_keys_and_a_channel_that_still_has_it_republishes() {
    let (_dir, host) = host_table();
    let (a, b) = (channel(Some(&host)), channel(Some(&host)));
    row(&a, "uid-zed", "zed");
    row(&b, "uid-zed", "zed");
    let key = a.agent_lan_key_ensure("zed").unwrap();
    a.agent_uid_keys_ensure("uid-zed").unwrap().unwrap();
    assert_eq!(b.agent_lan_key_ensure("zed").unwrap(), key);
    a.agent_def_delete("uid-zed").unwrap();
    assert!(host.claim(HostKeyKind::Name, "zed", None).unwrap().is_none());
    assert!(host.claim(HostKeyKind::Uid, "uid-zed", None).unwrap().is_none());
    // B still has the agent: its key stays, and is the host's again.
    assert_eq!(b.agent_lan_key_ensure("zed").unwrap(), key);
    assert_eq!(host.claim(HostKeyKind::Name, "zed", None).unwrap().map(|k| k.pair), Some(key));
}
