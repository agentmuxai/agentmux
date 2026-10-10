// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity M4d-2: when a UID takes its name's key, and when it gets a fresh one.

use super::*;
use crate::backend::storage::agents::test_agent_def;

/// A row created at `created_ms` with slug `slug`.
fn row(store: &Store, uid: &str, slug: &str, created_ms: i64) {
    let mut def = test_agent_def(uid, slug, "claude", "agent", created_ms, "");
    def.slug = slug.to_string();
    store.agent_def_insert_local_only(&mut def, None).unwrap();
    let conn = store.conn.lock().unwrap();
    conn.execute("UPDATE db_agents SET slug = ?2, created_at = ?3 WHERE id = ?1", params![uid, slug, created_ms]).unwrap();
}

/// A name-keyed LAN and WAN key under `name`, minted at `created_secs`.
fn name_keys(store: &Store, name: &str, created_secs: i64) -> (String, String) {
    let lan = store.agent_lan_key_ensure(name).unwrap();
    let wan = store.agent_wan_key_ensure(name).unwrap();
    let conn = store.conn.lock().unwrap();
    for table in ["db_agent_lan_keys", "db_agent_wan_keys"] {
        conn.execute(&format!("UPDATE {table} SET created_at = ?2 WHERE agent_id = ?1"), params![name.to_lowercase(), created_secs]).unwrap();
    }
    (lan.public_key, wan.public_key)
}

const ROW_MS: i64 = 1_800_000_000_000;
const ROW_SECS: i64 = ROW_MS / 1000;

fn ensure(store: &Store, uid: &str) -> (AgentUidKeys, [UidKeyOrigin; 2]) {
    store.agent_uid_keys_ensure(uid).unwrap().expect("the row exists")
}

#[test]
fn the_owner_of_a_name_key_takes_it_to_its_uid() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", ROW_MS);
    let (lan_pub, wan_pub) = name_keys(&store, "aria", ROW_SECS + 60);
    let (keys, origins) = ensure(&store, "uid-aria");
    assert_eq!(origins, [UidKeyOrigin::Copied; 2]);
    assert_eq!((keys.lan.public_key, keys.wan.public_key), (lan_pub, wan_pub));
}

#[test]
fn a_key_minted_in_the_same_second_as_its_row_is_its_own() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", ROW_MS + 999);
    name_keys(&store, "aria", ROW_SECS);
    assert_eq!(ensure(&store, "uid-aria").1, [UidKeyOrigin::Copied; 2]);
}

#[test]
fn a_key_older_than_the_row_belonged_to_someone_else() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", ROW_MS);
    let (lan_pub, _) = name_keys(&store, "aria", ROW_SECS - 1);
    let (keys, origins) = ensure(&store, "uid-aria");
    assert_eq!(origins, [UidKeyOrigin::Fresh(FreshReason::KeyOlderThanRow); 2]);
    assert_ne!(keys.lan.public_key, lan_pub);
}

#[test]
fn a_tombstoned_name_is_never_carried_to_a_new_uid() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", ROW_MS);
    name_keys(&store, "aria", ROW_SECS + 60);
    {
        let conn = store.conn.lock().unwrap();
        conn.execute("INSERT INTO db_agent_key_tombstones (name, deleted_uid, deleted_at) VALUES ('aria', 'uid-old', 1)", []).unwrap();
    }
    assert_eq!(ensure(&store, "uid-aria").1, [UidKeyOrigin::Fresh(FreshReason::Tombstoned); 2]);
}

#[test]
fn a_slug_two_rows_answer_to_case_insensitively_is_nobody_s() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", ROW_MS);
    row(&store, "uid-aria-caps", "Aria", ROW_MS);
    name_keys(&store, "aria", ROW_SECS + 60);
    assert_eq!(ensure(&store, "uid-aria").1, [UidKeyOrigin::Fresh(FreshReason::SlugShared); 2]);
}

#[test]
fn an_undated_row_cannot_prove_the_key_is_its_own() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", 0);
    name_keys(&store, "aria", ROW_SECS);
    assert_eq!(ensure(&store, "uid-aria").1, [UidKeyOrigin::Fresh(FreshReason::RowUndated); 2]);
}

#[test]
fn no_name_key_means_a_fresh_one_and_a_second_ensure_returns_it() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-aria", "aria", ROW_MS);
    let (first, origins) = ensure(&store, "uid-aria");
    assert_eq!(origins, [UidKeyOrigin::Fresh(FreshReason::NoNameKey); 2]);
    let (again, origins) = ensure(&store, "uid-aria");
    assert_eq!(origins, [UidKeyOrigin::Existing; 2]);
    assert_eq!(again, first);
    assert_ne!(first.lan.public_key, first.wan.public_key, "two tiers, two keys");
}

#[test]
fn two_agents_with_one_name_never_share_a_uid_key() {
    let store = Store::open_in_memory().unwrap();
    row(&store, "uid-a", "aria", ROW_MS);
    row(&store, "uid-b", "aria-2", ROW_MS);
    let a = ensure(&store, "uid-a").0;
    let b = ensure(&store, "uid-b").0;
    assert_ne!(a.lan.public_key, b.lan.public_key);
}

#[test]
fn a_purged_row_gets_no_key_and_its_keys_go_with_it() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.agent_uid_keys_ensure("uid-gone").unwrap(), None, "no row, no key");
    row(&store, "uid-aria", "aria", ROW_MS);
    ensure(&store, "uid-aria");
    assert!(store.agent_uid_lan_public_key_load("uid-aria").unwrap().is_some());
    {
        let conn = store.conn.lock().unwrap();
        crate::backend::storage::agents::purge_agent_dependents_for_tests(&conn, "uid-aria", None).unwrap();
    }
    assert_eq!(store.agent_uid_lan_public_key_load("uid-aria").unwrap(), None);
    let conn = store.conn.lock().unwrap();
    let wan: i64 = conn.query_row("SELECT COUNT(*) FROM db_agent_wan_keys_by_uid WHERE uid = 'uid-aria'", [], |r| r.get(0)).unwrap();
    assert_eq!(wan, 0);
}
