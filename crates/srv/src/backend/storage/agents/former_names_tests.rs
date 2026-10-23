// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Former display names at the store: resolution, ambiguity, reservation, and
//! the one resolver they share with former slugs.

use std::sync::Arc;

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::registry::{DefinitionStore, FormerSlug};

/// A store with a shared definition store, holding `agents` as
/// `(uid, name, slug)`.
fn store_with(agents: &[(&str, &str, &str)]) -> (tempfile::TempDir, Arc<Store>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let defs = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
    store.set_def_registry(defs);
    for (uid, name, slug) in agents {
        let mut def = test_agent_def(uid, name, "claude", "host", 1, "");
        def.slug = slug.to_string();
        store.agent_def_insert(&mut def).unwrap();
    }
    (tmp, store)
}

/// Rename `uid`'s display name as the tab rename does for a root.
fn rename(store: &Store, uid: &str, name: &str) {
    let mut def = store.agent_def_get(uid).unwrap().unwrap();
    def.name = name.to_string();
    assert!(store.agent_def_update(&mut def).unwrap());
}

/// Record a former display name directly, as a rename path that records by
/// trigger (a fork's branch label) leaves it.
fn record_former_name(store: &Store, uid: &str, name: &str, at: i64) {
    let conn = store.conn.lock().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO db_agent_former_names (agent_id, name, recorded_at) VALUES (?1, ?2, ?3)",
        params![uid, name, at],
    )
    .unwrap();
}

fn owner_uid(store: &Store, name: &str) -> Option<String> {
    store.former_name_owner(name).unwrap().map(|o| o.uid)
}

#[test]
fn a_former_display_name_resolves_to_its_agent_and_its_slug() {
    let (_t, store) = store_with(&[("uid-zed", "Zed Prime", "zed")]);
    assert_eq!(owner_uid(&store, "Zed-Prime"), None, "the current name is not an alias");
    rename(&store, "uid-zed", "Zork");
    let owner = FormerSlugOwner { uid: "uid-zed".into(), slug: "zed".into() };
    for name in ["Zed-Prime", "zed-prime", "Zed Prime", "  ZED PRIME "] {
        assert_eq!(store.former_name_owner(name).unwrap(), Some(owner.clone()), "{name}");
    }
    assert_eq!(owner_uid(&store, "Zork"), None);
    assert_eq!(owner_uid(&store, "zed"), None, "its slug is current");
    let matched = store.agents_matching_name("Zed-Prime").unwrap();
    assert_eq!(matched.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["uid-zed"]);
    let defs = store.agent_defs_for_target("zed-prime").unwrap();
    assert_eq!(defs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec!["uid-zed"]);
}

/// A fork's former label, recorded however the row got it, resolves the same.
#[test]
fn a_recorded_former_label_resolves_too() {
    let (_t, store) = store_with(&[("uid-fork", "Loap #3", "loap-3")]);
    record_former_name(&store, "uid-fork", "Experiment", 5);
    store.former_names_changed();
    assert_eq!(owner_uid(&store, "experiment").as_deref(), Some("uid-fork"));
}

/// A former display name two agents list resolves to neither, and so does one
/// any agent holds now: as its slug, as its display name's route, or as the
/// same agent's current name after a rename back.
#[test]
fn an_ambiguous_or_currently_held_former_name_resolves_none() {
    let (_t, store) = store_with(&[("uid-a", "Twin Name", "a"), ("uid-b", "Bee", "b"), ("uid-c", "Sea", "c")]);
    rename(&store, "uid-a", "Aye");
    rename(&store, "uid-b", "Twin Name");
    rename(&store, "uid-b", "Bee Two");
    assert_eq!(owner_uid(&store, "twin-name"), None, "two agents list it");
    assert!(store.former_name_reserved("Twin Name"));
    assert!(store.agents_matching_name("twin-name").unwrap().is_empty());

    rename(&store, "uid-c", "Sea Old");
    rename(&store, "uid-c", "Sea New");
    assert_eq!(owner_uid(&store, "sea-old").as_deref(), Some("uid-c"));
    let mut taker = test_agent_def("uid-d", "Sea Old", "claude", "host", 2, "");
    taker.slug = "d".into();
    store.agent_def_insert(&mut taker).unwrap();
    assert_eq!(owner_uid(&store, "sea-old"), None, "another agent's current display name wins");

    rename(&store, "uid-c", "Sea New Two");
    assert_eq!(owner_uid(&store, "sea-new").as_deref(), Some("uid-c"));
    rename(&store, "uid-c", "Sea New");
    assert_eq!(owner_uid(&store, "sea-new"), None, "its own current name again");

    record_former_name(&store, "uid-a", "C", 3);
    store.former_names_changed();
    assert!(store.former_name_reserved("c"));
    assert_eq!(owner_uid(&store, "c"), None, "another agent's current slug wins");
}

/// A former display name and another agent's former slug on the same key are
/// ambiguous too: one resolver, one rule.
#[test]
fn a_former_display_name_and_a_former_slug_of_two_agents_collide() {
    let (_t, store) = store_with(&[("uid-a", "Old Gold", "a"), ("uid-b", "Bo", "bo")]);
    rename(&store, "uid-a", "New Gold");
    store.agent_former_slugs_record("uid-b", &[FormerSlug { slug: "old-gold".into(), retired_at: 1 }]).unwrap();
    assert_eq!(owner_uid(&store, "old-gold"), None);
}

/// The reads that authenticate or act know current names only.
#[test]
fn ownership_reads_never_answer_a_former_display_name() {
    let (_t, store) = store_with(&[("uid-zed", "Zed Prime", "zed")]);
    rename(&store, "uid-zed", "Zork");
    assert!(store.agent_ids_for_slug_folded("zed-prime").unwrap().is_empty());
    assert!(store.instance_get_by_slug("zed-prime").unwrap().is_none());
    assert_eq!(store.agent_names_by_id("uid-zed").unwrap().unwrap().slug, "zed");
}

/// No other agent takes a former display name as its slug.
#[test]
fn a_new_slug_avoids_another_agents_former_display_name() {
    let (_t, store) = store_with(&[("uid-zed", "Zed Prime", "zed")]);
    rename(&store, "uid-zed", "Zork");
    let mut fresh = test_agent_def("uid-fresh", "Zed Prime", "claude", "host", 2, "");
    fresh.slug = String::new();
    store.agent_def_insert(&mut fresh).unwrap();
    assert_eq!(fresh.slug, "zed-prime-2");
    assert_eq!(store.former_name_listers("Zed Prime").unwrap().into_iter().collect::<Vec<_>>(), vec!["uid-zed"]);
}

/// The relay's grace subscription lists the former display names that still
/// resolve, with their time; one another agent holds now is left out.
#[test]
fn routable_former_display_names_are_listed_with_their_time() {
    let (_t, store) = store_with(&[("uid-a", "Ant", "a"), ("uid-b", "Bat", "b"), ("uid-c", "Cat", "c")]);
    record_former_name(&store, "uid-a", "Ant Old", 7);
    record_former_name(&store, "uid-b", "Cat", 9);
    assert_eq!(store.agent_former_display_names_routable().unwrap(), vec![("uid-a".to_string(), "ant-old".to_string(), 7)]);
}

fn reads(store: &Store) -> usize {
    store.former_slugs.reads.load(std::sync::atomic::Ordering::SeqCst)
}

/// A name that is no former name costs no store access once the sets are
/// warm, and a rename through this store is seen at once.
#[test]
fn a_name_that_is_no_former_name_never_touches_the_store() {
    let (_t, store) = store_with(&[("uid-zed", "Zed Prime", "zed"), ("uid-o", "Other", "other")]);
    rename(&store, "uid-zed", "Zork");
    assert!(store.former_name_owner("warm-up").unwrap().is_none());
    let warm = reads(&store);
    for name in ["github-consumer", "zed", "zork", "other", "Other"] {
        assert!(store.former_name_owner(name).unwrap().is_none(), "{name}");
    }
    assert_eq!(reads(&store), warm, "no store access for names that are no former name");
    rename(&store, "uid-zed", "Zork Two");
    assert_eq!(owner_uid(&store, "zork").as_deref(), Some("uid-zed"), "seen at once after a rename");
}
