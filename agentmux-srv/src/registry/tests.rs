// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Module-level integration tests for the registry. Per-file unit
//! tests live alongside each submodule.

use super::*;

fn fresh() -> (tempfile::TempDir, Registry) {
    let tmp = tempfile::tempdir().unwrap();
    let reg = Registry::open(tmp.path().to_path_buf()).unwrap();
    (tmp, reg)
}

fn record(id: &str, name: &str, ts: i64) -> NamedAgentRecord {
    NamedAgentRecord {
        schema_version: 1,
        data: NamedAgentRecordV1 {
            instance_id: id.to_string(),
            instance_name: name.to_string(),
            definition_id: "claude-code".to_string(),
            identity_id: Some("agenta".to_string()),
            memory_id: Some("default".to_string()),
            session_id: None,
            working_dir: format!("{name}-0512a"),
            source_agents_base: None,
            created_at_ms: ts,
            last_launched_at_ms: ts,
            created_by_version: "0.33.822".to_string(),
            last_launched_by_version: "0.33.822".to_string(),
        },
    }
}

#[test]
fn upsert_create_then_list() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    let listed = reg.list_active().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].data.instance_name, "demo");
}

#[test]
fn upsert_update_replaces_known_fields() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    reg.upsert(&record("aaa", "demo", 200)).unwrap();
    let listed = reg.list_active().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].data.last_launched_at_ms, 200);
}

/// #3586: a launch mirror keeps the session id the record already holds,
/// fills one in where it holds none, and moves every other field.
#[test]
fn upsert_keeping_session_keeps_a_held_session_id() {
    let (_t, reg) = fresh();
    let mut newer = record("aaa", "demo", 100);
    newer.data.session_id = Some("s-new".to_string());
    reg.upsert(&newer).unwrap();

    let mut launch = record("aaa", "demo", 200);
    launch.data.session_id = Some("s-old".to_string());
    reg.upsert_keeping_session(&launch).unwrap();
    let got = reg.get("aaa").unwrap().unwrap();
    assert_eq!(got.data.session_id.as_deref(), Some("s-new"));
    assert_eq!(got.data.last_launched_at_ms, 200);

    let mut unset = record("bbb", "other", 100);
    reg.upsert(&unset).unwrap();
    unset.data.session_id = Some("s-first".to_string());
    reg.upsert_keeping_session(&unset).unwrap();
    assert_eq!(
        reg.get("bbb").unwrap().unwrap().data.session_id.as_deref(),
        Some("s-first")
    );

    // A plain upsert — a session capture — still replaces it.
    reg.upsert(&launch).unwrap();
    assert_eq!(
        reg.get("aaa").unwrap().unwrap().data.session_id.as_deref(),
        Some("s-old")
    );
}

#[test]
fn get_returns_none_for_missing_record() {
    let (_t, reg) = fresh();
    assert!(reg.get("nope").unwrap().is_none());
}

#[test]
fn get_returns_existing_active_record() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    let got = reg.get("aaa").unwrap().unwrap();
    assert_eq!(got.data.instance_name, "demo");
    assert_eq!(got.data.last_launched_at_ms, 100);
}

#[test]
fn get_does_not_see_retired_records() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    reg.retire("aaa").unwrap();
    assert!(reg.get("aaa").unwrap().is_none());
}

#[test]
fn retire_then_unretire_round_trips() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    reg.retire("aaa").unwrap();
    assert!(reg.list_active().unwrap().is_empty());
    assert!(reg.root().join("retired").join("aaa.json").exists());

    reg.unretire("aaa").unwrap();
    assert_eq!(reg.list_active().unwrap().len(), 1);
}

#[test]
fn retire_is_idempotent_when_absent() {
    let (_t, reg) = fresh();
    reg.retire("never-existed").unwrap();
}

#[test]
fn hard_delete_removes_both_paths() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    reg.retire("aaa").unwrap();
    reg.upsert(&record("bbb", "demo2", 100)).unwrap();

    reg.hard_delete("aaa").unwrap();
    reg.hard_delete("bbb").unwrap();
    assert!(reg.list_active().unwrap().is_empty());
    assert!(!reg.root().join("retired").join("aaa.json").exists());
}

/// A record keyed by its own definition id and a legacy record keyed by the
/// launch it came from, both for one agent. `hard_delete(def_id)` only ever
/// matched the first; the second stayed active with its definition gone,
/// and `listrecentsessions` kept rendering it as an iconless
/// "(missing definition)" row — a delete whose card never went away.
#[test]
fn hard_delete_for_agent_catches_a_legacy_launch_keyed_record() {
    let (_t, reg) = fresh();
    let mut current = record("agent-1", "Maks", 100);
    current.data.definition_id = "agent-1".to_string();
    let mut legacy = record("old-launch-id", "Maks", 90);
    legacy.data.definition_id = "agent-1".to_string();
    reg.upsert(&current).unwrap();
    reg.upsert(&legacy).unwrap();

    reg.hard_delete("agent-1").unwrap();
    assert_eq!(
        reg.list_active().unwrap().len(),
        1,
        "the instance-keyed delete leaves the legacy record behind — the bug"
    );

    assert_eq!(reg.hard_delete_for_agent("agent-1", RecordScope::Agent).unwrap(), 1);
    assert!(reg.list_active().unwrap().is_empty());
}

#[test]
fn hard_delete_for_agent_also_drops_retired_records() {
    let (_t, reg) = fresh();
    let mut rec = record("old-launch-id", "Maks", 100);
    rec.data.definition_id = "agent-1".to_string();
    reg.upsert(&rec).unwrap();
    reg.retire("old-launch-id").unwrap();

    assert_eq!(reg.hard_delete_for_agent("agent-1", RecordScope::Agent).unwrap(), 1);
    assert!(!reg.root().join("retired").join("old-launch-id.json").exists());
}

#[test]
fn hard_delete_for_agent_leaves_other_agents_alone() {
    let (_t, reg) = fresh();
    let mut mine = record("old-launch-id", "Maks", 100);
    mine.data.definition_id = "agent-1".to_string();
    let mut theirs = record("other-launch-id", "Korp", 100);
    theirs.data.definition_id = "agent-2".to_string();
    reg.upsert(&mine).unwrap();
    reg.upsert(&theirs).unwrap();

    assert_eq!(reg.hard_delete_for_agent("agent-1", RecordScope::Agent).unwrap(), 1);
    let left = reg.list_active().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].data.definition_id, "agent-2");
}

/// The other direction, and why matching `definition_id` alone isn't
/// enough: a pre-consolidation registry-only row is addressed by its LAUNCH
/// id, and its `definition_id` still names the template it came from. The
/// `instance_set_hidden`/`instance_delete` callers hold that launch id.
#[test]
fn for_agent_also_matches_a_records_own_file_key() {
    let (_t, reg) = fresh();
    // `record`'s definition_id is "claude-code" — a template, not this id.
    reg.upsert(&record("legacy-only-row", "crossver", 100)).unwrap();

    assert_eq!(reg.retire_for_agent("legacy-only-row", RecordScope::Agent).unwrap(), 1);
    assert!(reg.root().join("retired").join("legacy-only-row.json").exists());
    assert_eq!(reg.unretire_for_agent("legacy-only-row", RecordScope::Agent).unwrap(), 1);
    assert_eq!(reg.hard_delete_for_agent("legacy-only-row", RecordScope::Agent).unwrap(), 1);
    assert!(reg.list_active().unwrap().is_empty());
}

#[test]
fn hard_delete_for_agent_on_an_unknown_id_is_a_no_op() {
    let (_t, reg) = fresh();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    assert_eq!(reg.hard_delete_for_agent("never-existed", RecordScope::Agent).unwrap(), 0);
    assert_eq!(reg.list_active().unwrap().len(), 1);
}

/// "Forget agent" retiring by file key left the legacy record active, so the
/// forgotten agent came straight back — the failure
/// `m0026_registry_agent_id_rekey`'s own doc comment predicted.
#[test]
fn retire_for_agent_round_trips_every_record_for_one_agent() {
    let (_t, reg) = fresh();
    let mut current = record("agent-1", "Maks", 100);
    current.data.definition_id = "agent-1".to_string();
    let mut legacy = record("old-launch-id", "Maks", 90);
    legacy.data.definition_id = "agent-1".to_string();
    reg.upsert(&current).unwrap();
    reg.upsert(&legacy).unwrap();

    assert_eq!(reg.retire_for_agent("agent-1", RecordScope::Agent).unwrap(), 2);
    assert!(reg.list_active().unwrap().is_empty());

    assert_eq!(reg.unretire_for_agent("agent-1", RecordScope::Agent).unwrap(), 2);
    assert_eq!(reg.list_active().unwrap().len(), 2);
    assert!(
        reg.get("old-launch-id").unwrap().is_some(),
        "each record keeps its own file name across the round trip"
    );
}

/// `listrecentsessions` dedupes rows by `(definition_id, instance_name)`, so
/// renaming one of an agent's records and not the others splits it into two
/// picker rows — one under each name.
#[test]
fn set_instance_name_for_agent_renames_every_record_for_that_agent() {
    let (_t, reg) = fresh();
    let mut current = record("agent-1", "Maks", 100);
    current.data.definition_id = "agent-1".to_string();
    let mut legacy = record("old-launch-id", "Maks", 90);
    legacy.data.definition_id = "agent-1".to_string();
    let mut other = record("other-id", "Korp", 100);
    other.data.definition_id = "agent-2".to_string();
    reg.upsert(&current).unwrap();
    reg.upsert(&legacy).unwrap();
    reg.upsert(&other).unwrap();

    assert_eq!(reg.set_instance_name_for_agent("agent-1", RecordScope::Agent, "Renamed").unwrap(), 2);
    let names: std::collections::BTreeSet<String> = reg
        .list_active()
        .unwrap()
        .into_iter()
        .map(|r| r.data.instance_name)
        .collect();
    assert_eq!(
        names,
        ["Korp".to_string(), "Renamed".to_string()].into_iter().collect(),
        "both of agent-1's records move; agent-2's is untouched"
    );
    // Idempotent — a second pass has nothing left to change.
    assert_eq!(reg.set_instance_name_for_agent("agent-1", RecordScope::Agent, "Renamed").unwrap(), 0);
}

/// A record with no `instance_name` fails validation on READ, so it never
/// reaches the picker — but `upsert` doesn't validate, so one can sit on
/// disk. `set_instance_name_for_agent` scans raw JSON (it has to: matching
/// on `definition_id` is the whole point), so without its empty-name guard
/// a rename would "repair" such a record into a visible row for an agent
/// that never had one.
#[test]
fn set_instance_name_for_agent_leaves_an_unnamed_record_invisible() {
    let (_t, reg) = fresh();
    let mut unnamed = record("agent-1", "nameless", 100);
    unnamed.data.definition_id = "agent-1".to_string();
    unnamed.data.instance_name = String::new();
    reg.upsert(&unnamed).unwrap();
    assert!(
        reg.list_active().unwrap().is_empty(),
        "precondition: an unnamed record is already invisible"
    );

    assert_eq!(reg.set_instance_name_for_agent("agent-1", RecordScope::Agent, "Renamed").unwrap(), 0);
    assert!(
        reg.list_active().unwrap().is_empty(),
        "renaming must not promote an invalid record into a picker row"
    );
}

/// Renaming must not resurrect an agent the user deliberately forgot.
#[test]
fn set_instance_name_for_agent_leaves_retired_records_retired() {
    let (_t, reg) = fresh();
    let mut rec = record("agent-1", "Maks", 100);
    rec.data.definition_id = "agent-1".to_string();
    reg.upsert(&rec).unwrap();
    reg.retire("agent-1").unwrap();

    assert_eq!(reg.set_instance_name_for_agent("agent-1", RecordScope::Agent, "Renamed").unwrap(), 0);
    assert!(reg.list_active().unwrap().is_empty());
}

#[test]
fn unknown_envelope_schema_is_skipped() {
    let (_t, reg) = fresh();
    // Forge a future-schema file directly on disk.
    let path = reg.root().join("future.json");
    let raw = serde_json::json!({
        "schema_version": 999,
        "data": { "instance_id": "future", "anything": "goes" }
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    let listed = reg.list_active().unwrap();
    assert!(listed.is_empty(), "v999 row must be skipped");
    assert!(path.exists(), "skipped file stays on disk");
}

#[test]
fn unknown_fields_in_data_survive_round_trip() {
    let (_t, reg) = fresh();
    // Write a record with a future field directly.
    let path = reg.root().join("aaa.json");
    let raw = serde_json::json!({
        "schema_version": 1,
        "data": {
            "instance_id": "aaa",
            "instance_name": "demo",
            "definition_id": "claude-code",
            "identity_id": null,
            "memory_id": null,
            "working_dir": "demo-0512a",
            "created_at_ms": 100,
            "last_launched_at_ms": 100,
            "created_by_version": "0.33.999",
            "last_launched_by_version": "0.33.999",
            "tags": ["important", "long-running"]
        }
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

    // An older binary touches last_launched_at_ms.
    reg.upsert(&record("aaa", "demo", 200)).unwrap();

    // The unknown `tags` field must still be on disk.
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let tags = on_disk
        .pointer("/data/tags")
        .expect("tags field preserved across older-writer update");
    assert_eq!(tags, &serde_json::json!(["important", "long-running"]));
    // And the known field did get updated.
    assert_eq!(
        on_disk.pointer("/data/last_launched_at_ms"),
        Some(&serde_json::json!(200))
    );
}

#[test]
fn corrupt_file_is_preserved_not_overwritten() {
    // A corrupt-on-disk file might be a newer-schema file with a
    // syntax error this binary doesn't understand. Refuse to clobber
    // it; SQLite remains authoritative for the row.
    let (_t, reg) = fresh();
    let path = reg.root().join("aaa.json");
    std::fs::write(&path, b"{ not json").unwrap();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"{ not json",
        "corrupt on-disk file must not be overwritten"
    );
}

#[test]
fn upsert_refuses_to_downgrade_schema_version() {
    let (_t, reg) = fresh();
    let path = reg.root().join("aaa.json");
    // Future v999 envelope already on disk.
    let v999 = serde_json::json!({
        "schema_version": 999,
        "data": { "instance_id": "aaa", "future_only": "field" }
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&v999).unwrap()).unwrap();

    // v1 binary calls upsert — must not clobber.
    reg.upsert(&record("aaa", "demo", 200)).unwrap();

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        after.pointer("/schema_version"),
        Some(&serde_json::json!(999)),
        "schema_version must not be downgraded"
    );
    assert_eq!(
        after.pointer("/data/future_only"),
        Some(&serde_json::json!("field")),
        "future_only field must be preserved"
    );
}

#[test]
fn upsert_refuses_when_schema_version_overflows_u32() {
    // A future binary writing `schema_version: u32::MAX + 1` would
    // wrap to 1 under an unchecked `as u32` cast and bypass the
    // downgrade guard. With `u32::try_from`, the cast fails and we
    // treat it the same as a missing/unparseable version — refuse.
    let (_t, reg) = fresh();
    let path = reg.root().join("aaa.json");
    let oversized: u64 = u32::MAX as u64 + 1;
    let raw = serde_json::json!({
        "schema_version": oversized,
        "data": { "instance_id": "aaa", "future_field": "x" }
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

    reg.upsert(&record("aaa", "demo", 200)).unwrap();

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        after.pointer("/schema_version"),
        Some(&serde_json::json!(oversized)),
        "schema_version above u32::MAX must not be downgraded via wraparound"
    );
}

#[test]
fn upsert_refuses_when_schema_version_missing() {
    let (_t, reg) = fresh();
    let path = reg.root().join("aaa.json");
    // Envelope with no schema_version at all (some future format we
    // can't reason about).
    let raw = serde_json::json!({ "data": { "instance_id": "aaa", "foo": 1 } });
    std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

    reg.upsert(&record("aaa", "demo", 200)).unwrap();

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(after.get("schema_version").is_none());
    assert_eq!(after.pointer("/data/foo"), Some(&serde_json::json!(1)));
}

#[test]
fn filename_id_mismatch_is_skipped() {
    let (_t, reg) = fresh();
    let path = reg.root().join("wrongname.json");
    let mut rec = record("realid", "demo", 100);
    rec.data.instance_id = "realid".to_string();
    std::fs::write(&path, serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
    assert!(reg.list_active().unwrap().is_empty());
}

#[test]
fn list_ignores_non_json_files() {
    let (_t, reg) = fresh();
    std::fs::write(reg.root().join("readme.txt"), b"hi").unwrap();
    reg.upsert(&record("aaa", "demo", 100)).unwrap();
    assert_eq!(reg.list_active().unwrap().len(), 1);
}

#[test]
fn concurrent_upserts_same_id_dont_corrupt() {
    use std::sync::Arc;
    use std::thread;

    let (_t, reg) = fresh();
    let reg = Arc::new(reg);
    let mut handles = Vec::new();
    for i in 0..16 {
        let reg = reg.clone();
        handles.push(thread::spawn(move || {
            reg.upsert(&record("aaa", "demo", i)).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let listed = reg.list_active().unwrap();
    assert_eq!(listed.len(), 1);
    // ts is whichever thread wrote last — we just care it's a valid value.
    assert!((0..16).contains(&listed[0].data.last_launched_at_ms));
}
