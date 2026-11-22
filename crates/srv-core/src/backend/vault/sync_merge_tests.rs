// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The merge of a Vault record that arrives from another machine: follow-on, repeat, partition, clock skew, a rotation
//! in progress, deletes and the tombstone horizon. Only versions and tags are used; every id is invented.

use super::*;

fn v(wall: i64, counter: u32, machine: &str) -> Version {
    Version::new(wall, counter, machine)
}

fn tags(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

fn held(versions: &[Version], t: &[&str]) -> Held {
    Held { versions: versions.to_vec(), deleted: false, pending_rotation: false, tags: tags(t) }
}

fn change(version: Version, base: Option<Version>, t: Option<&[&str]>) -> Change {
    Change { version, base, tags: t.map(tags) }
}

#[test]
fn a_record_the_machine_never_had_is_applied_and_a_repeat_changes_nothing() {
    let c = change(v(100, 0, "a"), None, Some(&["prod"]));
    assert_eq!(merge(None, &c), Decision::Applied);
    assert_eq!(merge(Some(&Held::default()), &c), Decision::Applied);
    assert_eq!(merge(Some(&held(&[v(100, 0, "a")], &["prod"])), &c), Decision::KeptLocal, "the same version again");
    // An old version of a record that has moved on is not put back.
    let moved = held(&[v(300, 0, "b"), v(200, 0, "a"), v(100, 0, "a")], &["prod"]);
    assert_eq!(merge(Some(&moved), &c), Decision::KeptLocal);
}

#[test]
fn a_change_made_on_top_of_what_this_machine_holds_follows_on() {
    let h = held(&[v(100, 0, "a")], &["prod"]);
    let c = change(v(200, 1, "b"), Some(v(100, 0, "a")), Some(&["prod", "billing"]));
    assert_eq!(merge(Some(&h), &c), Decision::Applied);
}

#[test]
fn two_machines_that_changed_it_apart_keep_both_and_the_later_is_current() {
    // Partition: both started from version 100 and neither saw the other.
    let mine = held(&[v(200, 0, "a"), v(100, 0, "a")], &["prod", "mine"]);
    let theirs = change(v(250, 0, "b"), Some(v(100, 0, "a")), Some(&["prod", "theirs"]));
    let Decision::Conflict { current, kept, tags } = merge(Some(&mine), &theirs) else { panic!("expected a conflict") };
    assert_eq!(current, Side::Incoming, "the later version is current");
    assert_eq!(kept, Kept::SameTime { machine: "a".into() }, "the one that lost is kept, with its machine");
    assert_eq!(tags, ["prod", "theirs", "mine"].map(String::from), "tags are a union");
    assert_eq!(kept.label(), "changed on a at the same time");

    // The other machine, hearing ours, reaches the same current version: the answers agree.
    let there = held(&[v(250, 0, "b"), v(100, 0, "a")], &["prod", "theirs"]);
    let ours = change(v(200, 0, "a"), Some(v(100, 0, "a")), Some(&["prod", "mine"]));
    let Decision::Conflict { current, kept, .. } = merge(Some(&there), &ours) else { panic!("expected a conflict") };
    assert_eq!((current, kept), (Side::Local, Kept::SameTime { machine: "a".into() }));
}

#[test]
fn a_fast_clock_wins_the_choice_but_never_loses_the_other_change() {
    // Machine b's clock is a day ahead, so its version is "later" though it was made first.
    let day = 24 * 60 * 60 * 1000;
    let mine = held(&[v(1_000, 0, "a"), v(10, 0, "a")], &[]);
    let skewed = change(v(1_000 + day, 0, "b"), Some(v(10, 0, "a")), Some(&[]));
    let Decision::Conflict { current, kept, .. } = merge(Some(&mine), &skewed) else { panic!("expected a conflict") };
    assert_eq!(current, Side::Incoming);
    assert_eq!(kept, Kept::SameTime { machine: "a".into() }, "our later-in-real-time change is kept as a version");
}

#[test]
fn the_same_time_the_machine_name_settles_it_so_both_sides_agree() {
    let mine = held(&[v(500, 3, "a"), v(100, 0, "a")], &[]);
    let theirs = change(v(500, 3, "b"), Some(v(100, 0, "a")), Some(&[]));
    assert!(matches!(merge(Some(&mine), &theirs), Decision::Conflict { current: Side::Incoming, .. }));
    let there = held(&[v(500, 3, "b"), v(100, 0, "a")], &[]);
    let ours = change(v(500, 3, "a"), Some(v(100, 0, "a")), Some(&[]));
    assert!(matches!(merge(Some(&there), &ours), Decision::Conflict { current: Side::Local, .. }));
}

#[test]
fn an_old_record_with_a_false_base_cannot_take_the_place_of_a_new_one() {
    // A relay replays version 150, which this machine never saw, claiming it follows on from the current 300.
    let h = held(&[v(300, 0, "a"), v(100, 0, "a")], &["prod"]);
    let replay = change(v(150, 0, "b"), Some(v(300, 0, "a")), Some(&["old"]));
    let Decision::Conflict { current, kept, .. } = merge(Some(&h), &replay) else { panic!("expected a conflict") };
    assert_eq!(current, Side::Local, "the current value stays");
    assert_eq!(kept, Kept::SameTime { machine: "b".into() }, "and the replayed one is only kept as a version");
}

#[test]
fn a_rotation_in_progress_is_never_overwritten() {
    let mut h = held(&[v(100, 0, "a")], &["prod"]);
    h.pending_rotation = true;
    // On top of the current version: waits, instead of replacing.
    let on_top = change(v(200, 0, "b"), Some(v(100, 0, "a")), Some(&["prod"]));
    let Decision::Conflict { current, kept, .. } = merge(Some(&h), &on_top) else { panic!("expected a wait") };
    assert_eq!((current, kept.clone()), (Side::Local, Kept::WaitsForRotation { machine: "b".into() }));
    assert!(kept.label().contains("being rotated"));
    // Made apart from it and later: still waits.
    let apart = change(v(900, 0, "c"), Some(v(50, 0, "a")), Some(&["prod"]));
    assert!(matches!(merge(Some(&h), &apart), Decision::Conflict { current: Side::Local, kept: Kept::WaitsForRotation { .. }, .. }));
}

#[test]
fn a_delete_waits_while_the_entry_is_being_rotated() {
    // The rotation may already have changed the credential at its source: deleting the entry would lose the new value.
    let mut h = held(&[v(100, 0, "a")], &["prod"]);
    h.pending_rotation = true;
    let delete = change(v(200, 0, "b"), Some(v(100, 0, "a")), None);
    let Decision::Conflict { current, kept, tags } = merge(Some(&h), &delete) else { panic!("expected the delete to wait") };
    assert_eq!((current, tags), (Side::Local, self::tags(&["prod"])));
    assert_eq!(kept, Kept::DeleteWaitsForRotation { machine: "b".into() });
    assert_eq!(kept.label(), "deleted on b while this was being rotated");
    // Once the rotation has ended the same delete applies.
    h.pending_rotation = false;
    assert_eq!(merge(Some(&h), &delete), Decision::Applied);
}

#[test]
fn a_delete_follows_on_but_loses_to_an_edit_it_did_not_see() {
    let h = held(&[v(100, 0, "a")], &["prod"]);
    assert_eq!(merge(Some(&h), &change(v(200, 0, "b"), Some(v(100, 0, "a")), None)), Decision::Applied, "a delete of what we hold");
    // A delete made from version 50, when we have already edited it to 100: the edit stays.
    let edited = held(&[v(100, 0, "a"), v(50, 0, "a")], &["prod"]);
    assert_eq!(merge(Some(&edited), &change(v(200, 0, "b"), Some(v(50, 0, "a")), None)), Decision::KeptLocal);

    // The other way round: we deleted, and an edit made without seeing the delete arrives.
    let gone = Held { versions: vec![v(200, 0, "a"), v(50, 0, "a")], deleted: true, pending_rotation: false, tags: vec![] };
    assert_eq!(merge(Some(&gone), &change(v(100, 0, "b"), Some(v(50, 0, "a")), Some(&["prod"]))), Decision::Applied, "the edit wins");
    // The deleted version itself, and a second delete, change nothing.
    assert_eq!(merge(Some(&gone), &change(v(50, 0, "a"), None, Some(&["prod"]))), Decision::KeptLocal);
    assert_eq!(merge(Some(&gone), &change(v(210, 0, "c"), Some(v(50, 0, "a")), None)), Decision::KeptLocal);
}

#[test]
fn what_only_this_machine_has_after_a_resync_is_listed_not_decided() {
    let mine = ["e1", "e2", "e3"].map(String::from);
    let theirs = ["e2"].map(String::from);
    assert_eq!(only_here(&mine, &theirs), ["e1", "e3"].map(String::from));
    assert!(only_here(&mine, &mine).is_empty());
}

#[test]
fn tags_are_a_union_that_keeps_the_first_side_first() {
    assert_eq!(union_tags(&tags(&["a", "b"]), &tags(&["b", "c"])), tags(&["a", "b", "c"]));
    assert_eq!(union_tags(&[], &tags(&["x"])), tags(&["x"]));
}
