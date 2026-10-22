// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Former slugs at the store: storage, resolution, reservation
//! (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` Phase 1).

use std::sync::Arc;

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::registry::DefinitionStore;

const NOW: i64 = 1_760_000_000_000;

/// A store with a shared definition store, holding `agents` as
/// `(uid, name, slug)`.
fn store_with(agents: &[(&str, &str, &str)]) -> (tempfile::TempDir, Arc<Store>, Arc<DefinitionStore>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let defs = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
    store.set_def_registry(defs.clone());
    for (uid, name, slug) in agents {
        let mut def = test_agent_def(uid, name, "claude", "host", 1, "");
        def.slug = slug.to_string();
        store.agent_def_insert(&mut def).unwrap();
    }
    (tmp, store, defs)
}

#[test]
fn a_renamed_slug_resolves_to_its_agent_and_its_current_slug() {
    let (_t, store, _defs) = store_with(&[("uid-loap3", "Loap #3", "loap-loap-2-loap-3")]);
    store.test_rename_slug("uid-loap3", "loap-3", NOW);
    let owner = FormerSlugOwner { uid: "uid-loap3".into(), slug: "loap-3".into() };
    for name in ["loap-loap-2-loap-3", "LOAP-LOAP-2-LOAP-3", "  loap-loap-2-loap-3 "] {
        assert_eq!(store.former_slug_owner(name).unwrap(), Some(owner.clone()), "{name}");
    }
    assert_eq!(store.former_slug_owner("loap-3").unwrap(), None, "the current slug is not an alias");
    assert_eq!(store.former_slug_owner("nobody").unwrap(), None);
    assert!(store.former_slug_reserved("Loap-Loap-2-Loap-3"));
    assert!(!store.former_slug_reserved("loap-3"));
}

/// Either copy answers: the shared record alone (a channel that hasn't
/// synced), and this table alone (no shared store).
#[test]
fn either_copy_of_the_former_slugs_answers() {
    let (_t, store, defs) = store_with(&[("uid-naki2", "Naki #2", "naki-naki-2")]);
    let mut rec = defs.get("uid-naki2").unwrap().unwrap();
    rec.data.slug = "naki-2".into();
    rec.data.former_slugs = vec![FormerSlug { slug: "naki-naki-2".into(), retired_at: NOW }];
    std::fs::write(defs.root().join("uid-naki2.json"), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
    assert!(store.agent_former_slugs_local("uid-naki2").unwrap().is_empty());
    let owner = store.former_slug_owner("naki-naki-2").unwrap().unwrap();
    assert_eq!((owner.uid.as_str(), owner.slug.as_str()), ("uid-naki2", "naki-2"));

    let solo = Store::open_in_memory().unwrap();
    let mut def = test_agent_def("uid-solo", "Solo", "claude", "host", 1, "");
    def.slug = "solo-2".into();
    solo.agent_def_insert(&mut def).unwrap();
    solo.agent_former_slugs_record("uid-solo", &[FormerSlug { slug: "Solo-Solo-2".into(), retired_at: NOW }]).unwrap();
    assert_eq!(solo.agent_former_slugs_local("uid-solo").unwrap()[0].slug, "solo-solo-2", "stored lowercased");
    assert_eq!(solo.former_slug_owner("solo-solo-2").unwrap().map(|o| o.slug).as_deref(), Some("solo-2"));
}

/// The two copies stay in step: startup copies the shared list here, and a
/// mirror write carries this table's to the shared record.
#[test]
fn the_shared_record_and_this_table_stay_in_step() {
    let (_t, store, defs) = store_with(&[("uid-a", "Aria", "aria-aria-2"), ("uid-b", "Bo", "bo")]);
    let mut rec = defs.get("uid-a").unwrap().unwrap();
    rec.data.former_slugs = vec![FormerSlug { slug: "aria-old".into(), retired_at: 5 }];
    std::fs::write(defs.root().join("uid-a.json"), serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
    assert_eq!(store.agent_former_slugs_sync_from_shared().unwrap(), 1);
    assert_eq!(store.agent_former_slugs_sync_from_shared().unwrap(), 0, "idempotent");
    assert_eq!(store.agent_former_slugs_local("uid-a").unwrap(), vec![FormerSlug { slug: "aria-old".into(), retired_at: 5 }]);

    store.agent_former_slugs_record("uid-b", &[FormerSlug { slug: "bo-bo-2".into(), retired_at: 9 }]).unwrap();
    let mut def = store.agent_def_get("uid-b").unwrap().unwrap();
    def.description = "edited".into();
    store.agent_def_update(&mut def).unwrap();
    assert_eq!(defs.get("uid-b").unwrap().unwrap().data.former_slugs, vec![FormerSlug { slug: "bo-bo-2".into(), retired_at: 9 }]);
}

/// A name that is some agent's current slug is that agent's, never an alias
/// for the one that used to have it.
#[test]
fn a_current_slug_always_wins_over_a_former_one() {
    let (_t, store, _defs) = store_with(&[("uid-old", "Old", "shared-name"), ("uid-new", "New", "other")]);
    store.test_rename_slug("uid-old", "moved", NOW);
    store.test_rename_slug("uid-new", "shared-name", NOW);
    assert_eq!(store.former_slug_owner("shared-name").unwrap(), None);
    assert_eq!(store.agents_matching_name("shared-name").unwrap().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["uid-new"]);
}

/// The store's name lookup (behind the resolve endpoint and the hold) and the
/// transcript rules' target lookup both find a renamed agent by its former
/// slug, after every live name and current slug.
#[test]
fn name_lookups_fall_back_to_a_former_slug() {
    let (_t, store, _defs) = store_with(&[("uid-loap2", "Loap #2", "loap-loap-2")]);
    store.test_rename_slug("uid-loap2", "loap-2", NOW);
    let matched = store.agents_matching_name("loap-loap-2").unwrap();
    assert_eq!(matched.iter().map(|m| (m.id.as_str(), m.slug.as_str())).collect::<Vec<_>>(), vec![("uid-loap2", "loap-2")]);
    let defs = store.agent_defs_for_target("Loap-Loap-2").unwrap();
    assert_eq!(defs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec!["uid-loap2"]);
    assert!(store.agent_defs_for_target("nobody").unwrap().is_empty());
}

/// Ownership is not aliased: the store's slug-to-uid reads behind
/// `uid_owns_name` and `uid_has_slug` give the current slug only.
#[test]
fn ownership_reads_know_the_current_slug_only() {
    let (_t, store, _defs) = store_with(&[("uid-loap2", "Loap #2", "loap-loap-2")]);
    store.test_rename_slug("uid-loap2", "loap-2", NOW);
    assert!(store.agent_ids_for_slug_folded("loap-loap-2").unwrap().is_empty());
    assert_eq!(store.agent_ids_for_slug_folded("loap-2").unwrap(), vec!["uid-loap2".to_string()]);
    assert_eq!(store.agent_names_by_id("uid-loap2").unwrap().unwrap().slug, "loap-2");
    assert!(store.instance_get_by_slug("loap-loap-2").unwrap().is_none());
}

/// Defining an agent under a former slug's name finds the agent that had it
/// rather than inserting a second one.
#[test]
fn defining_an_agent_under_a_former_slug_finds_its_owner() {
    let (_t, store, _defs) = store_with(&[("uid-loap2", "Loap #2", "loap-loap-2")]);
    store.test_rename_slug("uid-loap2", "loap-2", NOW);
    let mut def = test_agent_def("uid-fresh", "Loap Loap 2", "claude", "host", 2, "");
    def.slug = String::new();
    let found = store.agent_def_find_or_insert(&mut def).unwrap();
    assert_eq!(found.map(|d| d.id).as_deref(), Some("uid-loap2"));
    assert!(store.agent_def_get("uid-fresh").unwrap().is_none());
}

/// Deleting a renamed agent tombstones its former slugs' names from this
/// table too, and removes its rows.
#[test]
fn delete_covers_this_tables_former_slugs() {
    let store = Store::open_in_memory().unwrap();
    let mut def = test_agent_def("uid-del", "Del", "claude", "host", 1, "");
    def.slug = "del-2".into();
    store.agent_def_insert(&mut def).unwrap();
    store.agent_former_slugs_record("uid-del", &[FormerSlug { slug: "del-del-2".into(), retired_at: NOW }]).unwrap();
    store.agent_jekt_key_ensure("del-del-2").unwrap();
    assert!(store.agent_def_delete("uid-del").unwrap());
    assert!(store.agent_jekt_key_load("del-del-2").unwrap().is_none());
    assert!(store.agent_former_slugs_local("uid-del").unwrap().is_empty());
    let tombstones = {
        let conn = store.conn.lock().unwrap();
        crate::backend::storage::agents::key_tombstones_for_tests(&conn)
    };
    assert!(tombstones.contains(&("del-del-2".to_string(), "uid-del".to_string())), "{tombstones:?}");
    assert!(!store.former_slug_reserved("del-del-2"), "a deleted agent's former slug is free, as its slug is");
}

/// The relay's grace subscription reads every former slug with its time.
#[test]
fn every_former_slug_is_listed_with_its_retirement() {
    let (_t, store, _defs) = store_with(&[("uid-a", "A", "a-a-2"), ("uid-b", "B", "b")]);
    store.test_rename_slug("uid-a", "a-2", NOW);
    store.agent_former_slugs_record("uid-b", &[FormerSlug { slug: "b-old".into(), retired_at: 3 }]).unwrap();
    let all = store.agent_former_slugs_all().unwrap();
    assert_eq!(
        all,
        vec![
            ("uid-a".to_string(), FormerSlug { slug: "a-a-2".into(), retired_at: NOW }),
            ("uid-b".to_string(), FormerSlug { slug: "b-old".into(), retired_at: 3 }),
        ]
    );
}

/// A new agent never takes a former slug, from this channel's copy too (the
/// shared copy is `canonical_slug`'s `a_new_slug_avoids_shared_current_and_former_slugs`).
#[test]
fn a_new_agent_never_takes_a_former_slug_this_channel_holds() {
    let store = Store::open_in_memory().unwrap();
    let mut def = test_agent_def("uid-zed", "Zed", "claude", "host", 1, "");
    def.slug = "zed-2".into();
    store.agent_def_insert(&mut def).unwrap();
    store.agent_former_slugs_record("uid-zed", &[FormerSlug { slug: "zed-zed-2".into(), retired_at: NOW }]).unwrap();
    let mut fresh = test_agent_def("uid-fresh", "Fresh", "claude", "host", 2, "");
    fresh.slug = "Zed-Zed-2".into();
    store.agent_def_insert(&mut fresh).unwrap();
    assert_eq!(fresh.slug, "Zed-Zed-2-2");
}
