// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::bundle_scope::{SCOPE_FLEET, SCOPE_MACHINE};

const REMOTE: Actor<'static> = Actor { label: "loap@charlie", uid: "uid-loap" };

fn store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store.bundle_scope_ensure().unwrap();
    store
}

fn entry(id: &str, text: &str, updated_at: i64) -> FleetEntry {
    FleetEntry {
        id: id.to_string(),
        name: format!("Rules {id}"),
        description: String::new(),
        instructions: text.to_string(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        updated_at,
    }
}

fn bundle(id: &str, text: &str, global: bool) -> Bundle {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": format!("Rules {id}"), "instructions": text, "is_global": global,
        "mcp_servers": "[]", "skills": "[]", "updated_at": 100,
    }))
    .unwrap()
}

fn put(store: &Store, e: &FleetEntry) -> Outcome {
    apply_put(store, e, REMOTE).unwrap()
}

#[test]
fn a_new_entry_is_created_with_its_origin_time_and_attributed_to_the_remote_agent() {
    let store = store();
    store.bundle_upsert(&bundle("local-1", "mine", true)).unwrap();
    assert_eq!(put(&store, &entry("f1", "shared rule", 5_000)), Outcome::Applied(Applied::Created));

    let b = store.bundle_get("f1").unwrap().unwrap();
    assert!(b.is_global && !b.is_system);
    assert_eq!((b.instructions.as_str(), b.updated_at), ("shared rule", 5_000));

    let v = store.bundle_version_latest("f1").unwrap().unwrap();
    assert_eq!((v.source.as_str(), v.written_by.as_str(), v.written_by_uid.as_str()), (FLEET_SOURCE, "loap@charlie", "uid-loap"));

    let order: Vec<String> = store.bundle_list_global().unwrap().into_iter().map(|b| b.id).collect();
    assert_eq!(order.last().map(String::as_str), Some("f1"), "a new entry goes last: {order:?}");
    assert_eq!(store.bundle_scope_get("f1").unwrap(), SCOPE_FLEET, "an entry that arrived follows the fleet");
}

#[test]
fn the_newer_write_wins_and_an_older_or_identical_one_changes_nothing() {
    let store = store();
    put(&store, &entry("f1", "v1", 1_000));
    assert_eq!(put(&store, &entry("f1", "v2", 2_000)), Outcome::Applied(Applied::Updated));
    assert_eq!(store.bundle_get("f1").unwrap().unwrap().instructions, "v2");

    let versions = store.bundle_version_list("f1").unwrap().len();
    assert_eq!(put(&store, &entry("f1", "v1", 1_500)), Outcome::Skipped(Skipped::Older));
    assert_eq!(put(&store, &entry("f1", "v2", 9_000)), Outcome::Skipped(Skipped::Same));
    let b = store.bundle_get("f1").unwrap().unwrap();
    assert_eq!((b.instructions.as_str(), b.updated_at), ("v2", 2_000));
    assert_eq!(store.bundle_version_list("f1").unwrap().len(), versions, "nothing was recorded");
}

#[test]
fn two_machines_that_wrote_in_the_same_millisecond_agree_on_the_same_winner() {
    let (a, b) = (store(), store());
    let (ea, eb) = (entry("f1", "from a", 7_000), entry("f1", "from b", 7_000));
    put(&a, &ea);
    put(&b, &eb);
    put(&a, &eb);
    put(&b, &ea);
    let (ba, bb) = (a.bundle_get("f1").unwrap().unwrap(), b.bundle_get("f1").unwrap().unwrap());
    assert_eq!(ba.instructions, bb.instructions);
}

#[test]
fn an_entry_kept_on_this_machine_is_not_touched_by_a_write_or_a_removal() {
    let store = store();
    put(&store, &entry("f1", "mine", 1_000));
    store.bundle_scope_set("f1", SCOPE_MACHINE).unwrap();
    assert_eq!(put(&store, &entry("f1", "theirs", 9_000)), Outcome::Skipped(Skipped::MachineScope));
    let removal = FleetRemoval { id: "f1".into(), removed_at: 9_000 };
    assert_eq!(apply_remove(&store, &removal, REMOTE).unwrap(), Outcome::Skipped(Skipped::MachineScope));
    let b = store.bundle_get("f1").unwrap().unwrap();
    assert!(b.is_global);
    assert_eq!(b.instructions, "mine");
    assert_eq!(put(&store, &entry("never-seen", "x", 1)), Outcome::Applied(Applied::Created));
}

#[test]
fn entries_that_existed_before_the_fleet_stay_on_their_machine() {
    let store = Store::open_in_memory().unwrap();
    store.bundle_upsert(&bundle("hand-copied", "local wording", true)).unwrap();
    store.bundle_scope_ensure().unwrap();
    assert_eq!(put(&store, &entry("hand-copied", "pushed wording", 9_000)), Outcome::Skipped(Skipped::MachineScope));
    assert_eq!(store.bundle_get("hand-copied").unwrap().unwrap().instructions, "local wording");
}

#[test]
fn agentmux_own_entries_the_blank_bundle_and_agent_presets_are_never_written() {
    let store = store();
    let system = Bundle { is_system: true, ..bundle("sys-1", "agentmux rule", true) };
    store.bundle_upsert_system(&system).unwrap();
    store.bundle_upsert(&bundle("preset-1", "an agent's preset", false)).unwrap();

    assert_eq!(put(&store, &entry("sys-1", "hijack", 9_000)), Outcome::Skipped(Skipped::Protected));
    assert_eq!(put(&store, &entry("seed-rules", "hijack", 9_000)), Outcome::Skipped(Skipped::Protected));
    assert_eq!(put(&store, &entry("blank", "hijack", 9_000)), Outcome::Skipped(Skipped::Protected));
    assert_eq!(put(&store, &entry("preset-1", "hijack", 9_000)), Outcome::Skipped(Skipped::NotMemory));
    let removal = |id: &str| FleetRemoval { id: id.into(), removed_at: 9_000 };
    assert_eq!(apply_remove(&store, &removal("sys-1"), REMOTE).unwrap(), Outcome::Skipped(Skipped::Protected));
    assert_eq!(apply_remove(&store, &removal("preset-1"), REMOTE).unwrap(), Outcome::Skipped(Skipped::NotMemory));
    assert_eq!(store.bundle_get("sys-1").unwrap().unwrap().instructions, "agentmux rule");
    assert_eq!(store.bundle_get("preset-1").unwrap().unwrap().instructions, "an agent's preset");
}

#[test]
fn a_removal_is_a_write_that_a_newer_edit_beats_and_an_older_one_loses_to() {
    let store = store();
    put(&store, &entry("f1", "text", 1_000));

    assert_eq!(
        apply_remove(&store, &FleetRemoval { id: "f1".into(), removed_at: 500 }, REMOTE).unwrap(),
        Outcome::Skipped(Skipped::Older),
        "an edit made after the removal wins"
    );
    assert_eq!(
        apply_remove(&store, &FleetRemoval { id: "f1".into(), removed_at: 2_000 }, REMOTE).unwrap(),
        Outcome::Applied(Applied::Removed)
    );
    assert!(!store.bundle_get("f1").unwrap().unwrap().is_global);
    assert!(store.bundle_is_removed_global("f1").unwrap());
    assert_eq!(
        apply_remove(&store, &FleetRemoval { id: "f1".into(), removed_at: 3_000 }, REMOTE).unwrap(),
        Outcome::Skipped(Skipped::AlreadyRemoved)
    );
    assert_eq!(
        apply_remove(&store, &FleetRemoval { id: "nope".into(), removed_at: 3_000 }, REMOTE).unwrap(),
        Outcome::Skipped(Skipped::Unknown)
    );

    assert_eq!(put(&store, &entry("f1", "older edit", 1_500)), Outcome::Skipped(Skipped::Older), "the removal is newer");
    assert_eq!(put(&store, &entry("f1", "edited later", 4_000)), Outcome::Applied(Applied::Restored));
    let b = store.bundle_get("f1").unwrap().unwrap();
    assert!(b.is_global);
    assert_eq!(b.instructions, "edited later");
}

#[test]
fn a_removal_is_recorded_as_a_removal_by_the_remote_agent() {
    let store = store();
    put(&store, &entry("f1", "text", 1_000));
    apply_remove(&store, &FleetRemoval { id: "f1".into(), removed_at: 2_000 }, REMOTE).unwrap();
    let v = store.bundle_version_latest("f1").unwrap().unwrap();
    assert_eq!((v.source.as_str(), v.written_by.as_str()), (REMOVE_SOURCE, "loap@charlie"));
}

#[test]
fn a_malformed_write_is_refused_and_writes_nothing() {
    let store = store();
    let bad = |f: fn(&mut FleetEntry)| {
        let mut e = entry("f1", "text", 1_000);
        f(&mut e);
        matches!(apply_put(&store, &e, REMOTE), Err(FleetError::Invalid(_)))
    };
    assert!(bad(|e| e.id.clear()));
    assert!(bad(|e| e.name = "  ".into()));
    assert!(bad(|e| e.context_files = "not json".into()));
    assert!(bad(|e| e.instructions_by_provider = "{".into()));
    assert!(bad(|e| e.instructions = "x".repeat(MAX_INSTRUCTIONS + 1)));
    assert!(store.bundle_get("f1").unwrap().is_none());
}

#[test]
fn list_meta_has_the_fleet_entries_and_the_removals_and_nothing_else() {
    let store = Store::open_in_memory().unwrap();
    store.bundle_upsert(&bundle("hand-copied", "x", true)).unwrap();
    store.bundle_scope_ensure().unwrap();
    store.bundle_upsert(&bundle("seed-x", "s", true)).unwrap();
    put(&store, &entry("f1", "one", 1_000));
    put(&store, &entry("f2", "two", 2_000));
    apply_remove(&store, &FleetRemoval { id: "f2".into(), removed_at: 3_000 }, REMOTE).unwrap();

    let mut metas = list_meta(&store).unwrap();
    metas.sort_by(|a, b| a.id.cmp(&b.id));
    let rows: Vec<(&str, bool, i64)> = metas.iter().map(|m| (m.id.as_str(), m.removed, m.updated_at)).collect();
    assert_eq!(rows, [("f1", false, 1_000), ("f2", true, 3_000)]);
    assert_eq!(metas[0].hash, entry("f1", "one", 1_000).hash());
}

#[test]
fn entry_for_push_returns_a_fleet_entry_and_nothing_for_the_rest() {
    let store = store();
    put(&store, &entry("f1", "text", 1_000));
    store.bundle_upsert(&bundle("m1", "mine", true)).unwrap();
    store.bundle_scope_set("m1", SCOPE_MACHINE).unwrap();
    store.bundle_upsert(&bundle("p1", "preset", false)).unwrap();

    assert_eq!(entry_for_push(&store, "f1").unwrap(), Some(entry("f1", "text", 1_000)));
    assert_eq!(entry_for_push(&store, "m1").unwrap(), None);
    assert_eq!(entry_for_push(&store, "p1").unwrap(), None);
    assert_eq!(entry_for_push(&store, "seed-x").unwrap(), None);
    assert_eq!(entry_for_push(&store, "missing").unwrap(), None);
}

fn meta(id: &str, hash: &str, updated_at: i64, removed: bool) -> FleetMeta {
    FleetMeta { id: id.into(), updated_at, hash: hash.into(), removed }
}

#[test]
fn a_machine_that_was_offline_gets_what_it_lacks_or_holds_older_newest_first() {
    let mine = [
        meta("same", "h", 5_000, false),
        meta("newer-here", "h2", 9_000, false),
        meta("newer-there", "h3", 1_000, false),
        meta("only-here", "h4", 7_000, false),
        meta("removed-here", "gone", 8_000, true),
        meta("removed-and-unknown-there", "gone", 8_500, true),
    ];
    let theirs = [
        meta("same", "h", 5_000, false),
        meta("newer-here", "old", 2_000, false),
        meta("newer-there", "new", 4_000, false),
        meta("removed-here", "live", 3_000, false),
        meta("only-there", "x", 6_000, false),
    ];
    assert_eq!(
        to_push(&mine, &theirs),
        [
            Push::Put { id: "newer-here".into() },
            Push::Remove { id: "removed-here".into(), removed_at: 8_000 },
            Push::Put { id: "only-here".into() },
        ]
    );
}

#[test]
fn two_machines_pushing_to_each_other_end_with_the_same_entries() {
    let (a, b) = (store(), store());
    put(&a, &entry("shared", "a wrote this first", 1_000));
    put(&b, &entry("shared", "b wrote this later", 2_000));
    put(&a, &entry("only-a", "a only", 3_000));
    put(&b, &entry("gone", "b removes this", 1_000));
    put(&a, &entry("gone", "b removes this", 1_000));
    apply_remove(&b, &FleetRemoval { id: "gone".into(), removed_at: 4_000 }, REMOTE).unwrap();

    // Each side lists the other and sends what to_push says, as the catch-up will.
    let sync = |from: &Store, to: &Store| {
        for push in to_push(&list_meta(from).unwrap(), &list_meta(to).unwrap()) {
            match push {
                Push::Put { id } => {
                    let _ = apply_put(to, &entry_for_push(from, &id).unwrap().unwrap(), REMOTE).unwrap();
                }
                Push::Remove { id, removed_at } => {
                    let _ = apply_remove(to, &FleetRemoval { id, removed_at }, REMOTE).unwrap();
                }
            }
        }
    };
    sync(&a, &b);
    sync(&b, &a);

    let view = |s: &Store| {
        let mut m = list_meta(s).unwrap();
        m.sort_by(|x, y| x.id.cmp(&y.id));
        m
    };
    assert_eq!(view(&a), view(&b));
    let ids: Vec<(String, bool)> = view(&a).into_iter().map(|m| (m.id, m.removed)).collect();
    assert_eq!(ids, [("gone".to_string(), true), ("only-a".to_string(), false), ("shared".to_string(), false)]);
    assert_eq!(a.bundle_get("shared").unwrap().unwrap().instructions, "b wrote this later");
    assert!(to_push(&list_meta(&a).unwrap(), &list_meta(&b).unwrap()).is_empty(), "nothing left to send");
}
