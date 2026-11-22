// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Activity lines as sync records: their ids, what leaves a machine, and that they never conflict. Invented values.

use super::*;
use crate::backend::vault::sync_seal::{open_record, seal_record, Place, Version};
use crate::backend::vault::seal;

fn row() -> VaultActivityRow {
    VaultActivityRow {
        seq: 41,
        at: 5_000,
        entry_id: "entry-1".into(),
        entry_label: "Invoice provider key".into(),
        actor: "Agent-x".into(),
        action: "ran a command".into(),
        target: "deploy.sh".into(),
        outcome: "ok".into(),
        version: 3,
        actor_kind: "agent".into(),
        host: "machine-a".into(),
        reason: "deploying".into(),
        refs: r##"["#12"]"##.into(),
        prev_hash: "aa".into(),
        hash: "bb".into(),
    }
}

#[test]
fn a_line_leaves_without_its_place_in_this_machines_chain() {
    let body = ActivityBody::of_row(&row());
    let json = serde_json::to_string(&body).unwrap();
    assert!(!json.contains("prev_hash") && !json.contains("\"hash\"") && !json.contains("\"seq\""));
    let made = body.into_row(7);
    assert_eq!((made.seq, made.prev_hash.as_str(), made.hash.as_str()), (7, "", ""));
    assert_eq!((made.action.as_str(), made.host.as_str(), made.version), ("ran a command", "machine-a", 3));
    assert_eq!(ActivityBody::of_row(&made), body, "nothing else is lost on the way");
}

#[test]
fn an_id_names_the_machine_and_the_sequence_and_reads_back_one_way() {
    let id = record_id("machine-a", 41);
    assert_eq!(id, "a.machine-a.41");
    assert_eq!(parse_record_id(&id), Some(("machine-a", 41)));
    // A machine name with a dot still reads back, because the sequence is after the last one.
    assert_eq!(parse_record_id(&record_id("m.1", 5)), Some(("m.1", 5)));
    for bad in ["entry-1", "a.machine", "a..5", "a.machine.x", "a.machine.0", "a.machine.-3"] {
        assert_eq!(parse_record_id(bad), None, "{bad}");
    }
}

#[test]
fn lines_never_conflict_and_a_repeat_changes_nothing() {
    assert_eq!(merge(false), Decision::Applied);
    assert_eq!(merge(true), Decision::KeptLocal);
}

#[test]
fn a_line_goes_through_the_seal_and_shows_nothing_readable() {
    let (key, v) = (seal::test_key(), Version::new(9_000, 0, "machine-a"));
    let id = record_id("machine-a", 41);
    let place = Place { vault: "vault-1", record: &id, version: &v, base: None, epoch: 1 };
    let sealed = seal_record(&key, &place, &ActivityBody::of_row(&row())).unwrap();
    assert!(!sealed.windows(6).any(|w| w == b"deploy"), "the target is inside the seal");
    assert_eq!(open_record::<ActivityBody>(&key, &place, &sealed).unwrap(), ActivityBody::of_row(&row()));
}
