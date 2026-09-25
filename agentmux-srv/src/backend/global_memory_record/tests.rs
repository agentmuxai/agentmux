// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, Instant};

use super::*;
use crate::backend::storage::store::Bundle;

const SCOPE: &str = "shared";

fn bundle(id: &str, name: &str, instructions: &str, global: bool) -> Bundle {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": name, "instructions": instructions, "is_global": global,
        "mcp_servers": "[]", "skills": "[]",
    }))
    .unwrap()
}

fn head_body(fs: &FileStore, id: &str) -> Option<(String, String)> {
    let sha = heads(fs, SCOPE).unwrap().entries.get(id)?.sha256.clone()?;
    body(fs, SCOPE, &sha).unwrap()
}

#[test]
fn an_entry_is_recorded_with_its_writer_once_and_again_when_it_changes() {
    let fs = FileStore::open_in_memory().unwrap();
    let store = Store::open_in_memory().unwrap();
    store.bundle_upsert_with_version(&bundle("g1", "Rules", "be kind", true), "armory-ui", "", "human", "").unwrap();
    assert!(sync_entry(&fs, &store, SCOPE, "g1", None).unwrap());
    assert!(!sync_entry(&fs, &store, SCOPE, "g1", None).unwrap(), "unchanged: recorded once");
    assert_eq!(head_body(&fs, "g1"), Some(("Rules".into(), "be kind".into())));
    let h = history(&fs, SCOPE, "g1").unwrap();
    assert_eq!((h[0].source.as_str(), h[0].written_by.as_str()), ("human", "armory-ui"));

    store.bundle_upsert_with_version(&bundle("g1", "Rules", "be kinder", true), "agent-x", "uid-x", "agent", "").unwrap();
    assert!(sync_entry(&fs, &store, SCOPE, "g1", None).unwrap());
    let h = history(&fs, SCOPE, "g1").unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(h[1].parent.as_deref(), Some(h[0].version.as_str()));
    assert_eq!(h[1].written_by_uid, "uid-x");
}

/// Removing from Global Memory (a demote, as the Armory's Remove does, or a
/// delete) is a tombstone; a bundle that was never global is not recorded.
#[test]
fn leaving_global_memory_is_a_tombstone_and_other_bundles_are_ignored() {
    let fs = FileStore::open_in_memory().unwrap();
    let store = Store::open_in_memory().unwrap();
    store.bundle_upsert(&bundle("g1", "A", "a", true)).unwrap();
    store.bundle_upsert(&bundle("g2", "B", "b", true)).unwrap();
    store.bundle_upsert(&bundle("p1", "Personal", "p", false)).unwrap();
    for id in ["g1", "g2", "p1"] {
        sync_entry(&fs, &store, SCOPE, id, None).unwrap();
    }
    assert!(!heads(&fs, SCOPE).unwrap().entries.contains_key("p1"));

    store.bundle_upsert(&bundle("g1", "A", "a", false)).unwrap();
    store.bundle_delete("g2").unwrap();
    assert!(sync_entry(&fs, &store, SCOPE, "g1", None).unwrap());
    assert!(sync_entry(&fs, &store, SCOPE, "g2", None).unwrap());
    let h = heads(&fs, SCOPE).unwrap();
    assert!(h.entries["g1"].sha256.is_none() && h.entries["g2"].sha256.is_none());
    assert!(!sync_entry(&fs, &store, SCOPE, "g2", None).unwrap(), "one tombstone");
    assert_eq!(history(&fs, SCOPE, "g1").unwrap().len(), 2, "its history stays");
}

#[test]
fn the_order_is_recorded_when_it_changes() {
    let fs = FileStore::open_in_memory().unwrap();
    let store = Store::open_in_memory().unwrap();
    store.bundle_upsert(&bundle("g1", "A", "a", true)).unwrap();
    store.bundle_upsert(&bundle("g2", "B", "b", true)).unwrap();
    assert!(sync_order(&fs, &store, SCOPE).unwrap());
    assert!(!sync_order(&fs, &store, SCOPE).unwrap());
    store.bundle_reorder(&["g2".into(), "g1".into()]).unwrap();
    assert!(sync_order(&fs, &store, SCOPE).unwrap());
    assert_eq!(heads(&fs, SCOPE).unwrap().order, vec!["g2".to_string(), "g1".to_string()]);
}

/// The start-up comparison: everything the first time as the baseline;
/// later, what changed without the record (an older build) as legacy-build.
#[test]
fn the_startup_comparison_records_a_baseline_then_what_older_builds_changed() {
    let fs = FileStore::open_in_memory().unwrap();
    let store = Store::open_in_memory().unwrap();
    store.bundle_upsert(&bundle("g1", "A", "a", true)).unwrap();
    store.bundle_upsert(&bundle("g2", "B", "b", true)).unwrap();
    assert_eq!(sync_all(&fs, &store, SCOPE).unwrap(), 2);
    assert_eq!(history(&fs, SCOPE, "g1").unwrap()[0].source, "baseline");
    assert_eq!(sync_all(&fs, &store, SCOPE).unwrap(), 0, "quiet when nothing changed");

    store.bundle_upsert(&bundle("g1", "A", "edited by an older build", true)).unwrap();
    store.bundle_delete("g2").unwrap();
    assert_eq!(sync_all(&fs, &store, SCOPE).unwrap(), 2);
    assert_eq!(history(&fs, SCOPE, "g1").unwrap()[1].source, "legacy-build");
    assert!(heads(&fs, SCOPE).unwrap().entries["g2"].sha256.is_none());
}

/// Write-through: every Global Memory write method tells the observer, and
/// the worker records it without the caller waiting.
#[test]
fn writes_through_the_store_are_recorded_by_the_worker() {
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    let store = Arc::new(Store::open_in_memory().unwrap());
    let _recorder = attach(fs.clone(), store.clone());
    let wait_for = |pred: &dyn Fn() -> bool| {
        let until = Instant::now() + Duration::from_secs(10);
        while !pred() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(pred());
    };
    let scope = scope();
    store.bundle_upsert_with_version(&bundle("g1", "Rules", "x", true), "armory-ui", "", "human", "").unwrap();
    wait_for(&|| heads(&fs, &scope).unwrap().entries.get("g1").is_some_and(|h| h.sha256.is_some()));
    store.bundle_upsert(&bundle("g2", "More", "y", true)).unwrap();
    store.bundle_reorder(&["g2".into(), "g1".into()]).unwrap();
    wait_for(&|| heads(&fs, &scope).unwrap().order == vec!["g2".to_string(), "g1".to_string()]);
    store.bundle_delete("g1").unwrap();
    wait_for(&|| heads(&fs, &scope).unwrap().entries.get("g1").is_some_and(|h| h.sha256.is_none()));
}

// ── Import into an isolated channel ─────────────────────────────────────

/// The shared record, as a stable channel's Global Memory left it.
fn shared_record(fs: &FileStore) {
    let stable = Store::open_in_memory().unwrap();
    stable.bundle_upsert(&bundle("g-rules", "Rules", "be kind", true)).unwrap();
    stable.bundle_upsert(&bundle("g-style", "Style", "short sentences", true)).unwrap();
    let mut system = bundle("operator-config-x", "System", "from AgentMux", true);
    system.is_system = true;
    stable.bundle_upsert_system(&system).unwrap();
    stable.bundle_reorder(&["g-style".into(), "g-rules".into()]).unwrap();
    sync_all(fs, &stable, "shared").unwrap();
}

#[test]
fn an_isolated_channel_is_offered_the_entries_it_lacks_and_imports_them_in_order() {
    let fs = FileStore::open_in_memory().unwrap();
    shared_record(&fs);
    let channel = Store::open_in_memory().unwrap();
    let offer = import_sources_in(&fs, &channel, "channel:test").unwrap();
    assert_eq!(offer.sources.len(), 1);
    assert_eq!(offer.sources[0].scope, "shared");
    let names: Vec<&str> = offer.sources[0].missing.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Style", "Rules"], "in the source's order, without system entries");

    let r = import(&fs, &channel, &offer.list_id, 0).unwrap();
    assert_eq!(r, ImportReport { added: 2, renamed: 0 });
    let here: Vec<(String, String)> =
        channel.bundle_list_global().unwrap().into_iter().map(|b| (b.id, b.instructions)).collect();
    assert_eq!(
        here,
        vec![("g-style".into(), "short sentences".into()), ("g-rules".into(), "be kind".into())],
        "ids and order kept"
    );
    let v = channel.bundle_version_list("g-rules").unwrap();
    assert_eq!((v[0].source.as_str(), v[0].source_detail.as_str()), ("import", "from shared"));
    assert!(import_sources_in(&fs, &channel, "channel:test").unwrap().sources.is_empty(), "nothing left to offer");
}

/// An entry here is never overwritten; a clashing name is taken with the
/// id's short form.
#[test]
fn import_never_overwrites_and_renames_on_a_name_clash() {
    let fs = FileStore::open_in_memory().unwrap();
    shared_record(&fs);
    let channel = Store::open_in_memory().unwrap();
    channel.bundle_upsert(&bundle("g-style", "Style", "this channel's own", true)).unwrap();
    channel.bundle_upsert(&bundle("local-rules", "Rules", "a different entry, same name", true)).unwrap();
    let offer = import_sources_in(&fs, &channel, "channel:test").unwrap();
    assert_eq!(offer.sources[0].missing.len(), 1, "g-style is already here");
    let r = import(&fs, &channel, &offer.list_id, 0).unwrap();
    assert_eq!(r, ImportReport { added: 1, renamed: 1 });
    assert_eq!(channel.bundle_get("g-style").unwrap().unwrap().instructions, "this channel's own");
    assert_eq!(channel.bundle_get("g-rules").unwrap().unwrap().name, "Rules (g-rules)");
    assert_eq!(channel.bundle_get("local-rules").unwrap().unwrap().instructions, "a different entry, same name");
}

#[test]
fn a_shared_channel_is_offered_nothing() {
    let fs = FileStore::open_in_memory().unwrap();
    shared_record(&fs);
    let other = Store::open_in_memory().unwrap();
    other.bundle_upsert(&bundle("g-x", "X", "x", true)).unwrap();
    sync_all(&fs, &other, "channel:other").unwrap();
    let channel = Store::open_in_memory().unwrap();
    assert!(import_sources_in(&fs, &channel, "shared").unwrap().sources.is_empty());
    assert_eq!(import_sources_in(&fs, &channel, "channel:test").unwrap().sources.len(), 2, "shared first, then channel:other");
}
