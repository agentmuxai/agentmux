// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;

const UID: &str = "93d0dfdd-0000-4000-8000-00000000abcd";

/// A channel's history store on disk, with rows for `UID`.
fn channel(dir: &std::path::Path, name: &str, rows: &[(&str, &str, &str)]) -> PathBuf {
    let path = dir.join(format!("{name}.db"));
    let store = Store::open(&path).unwrap();
    for (file, body, source) in rows {
        store.agent_native_memory_version_insert(UID, file, body, source, "", "").unwrap();
    }
    path
}

fn later() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn bodies(fs: &FileStore, file: &str) -> Vec<(String, String)> {
    record::history(fs, UID, file)
        .unwrap()
        .into_iter()
        .map(|v| (String::from_utf8(record::body(fs, UID, &v.sha256.unwrap()).unwrap().unwrap()).unwrap(), v.source))
        .collect()
}

#[test]
fn proven_history_from_every_channel_is_imported_once_oldest_first_and_deduplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let a = channel(tmp.path(), "a", &[("notes.md", "one", "agent"), ("notes.md", "two", "human")]);
    // The same content in a second channel counts once.
    let b = channel(tmp.path(), "b", &[("notes.md", "two", "agent"), ("notes.md", "three", "revert")]);
    let fs = FileStore::open_in_memory().unwrap();
    let got = import_once(&fs, UID, Some(&[a.clone(), b.clone()]), |_| None, later()).unwrap();
    assert_eq!(got, Some(3));
    assert_eq!(
        bodies(&fs, "notes.md"),
        vec![("one".into(), "agent".into()), ("two".into(), "human".into()), ("three".into(), "revert".into())]
    );
    let chain = record::history(&fs, UID, "notes.md").unwrap();
    assert_eq!(chain[1].parent.as_deref(), Some(chain[0].version.as_str()), "chained per file");
    assert!(chain[0].source_detail.starts_with("imported "));

    // History only: no head, so nothing is written to a folder.
    assert!(record::heads(&fs, UID).unwrap().files.is_empty());
    // Once: a later row is not picked up by a second call.
    let c = channel(tmp.path(), "c", &[("notes.md", "four", "agent")]);
    assert_eq!(import_once(&fs, UID, Some(&[a, b, c]), |_| None, later()).unwrap(), None);
    assert_eq!(bodies(&fs, "notes.md").len(), 3);
}

/// Unproven rows (m0024's `agent_inferred`, drift's `external_fs_write`)
/// may belong to another agent: imported only when the agent's own folder
/// holds exactly that content under that name.
#[test]
fn unproven_rows_are_imported_only_when_the_agents_own_folder_holds_them() {
    let tmp = tempfile::tempdir().unwrap();
    let store = channel(
        tmp.path(),
        "a",
        &[
            ("mine.md", "what my folder holds", "agent_inferred"),
            ("theirs.md", "someone else's", "external_fs_write"),
            ("mine.md", "an older guess", "external_fs_write"),
        ],
    );
    let fs = FileStore::open_in_memory().unwrap();
    let on_disk = record::sha256_hex(b"what my folder holds");
    let disk_sha = |name: &str| (name == "mine.md").then(|| on_disk.clone());
    assert_eq!(import_once(&fs, UID, Some(&[store]), disk_sha, later()).unwrap(), Some(1));
    assert_eq!(bodies(&fs, "mine.md"), vec![("what my folder holds".into(), "agent_inferred".into())]);
    assert!(bodies(&fs, "theirs.md").is_empty());
}

/// Content the record already has for a file isn't imported again; the rest
/// of that file's history still is.
#[test]
fn content_already_in_the_record_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let store = channel(tmp.path(), "a", &[("notes.md", "old", "agent"), ("notes.md", "current", "agent")]);
    let fs = FileStore::open_in_memory().unwrap();
    record::append_version(
        &fs,
        UID,
        record::NewVersion {
            file: "notes.md",
            body: Some(b"current"),
            expected_parent: None,
            merged_parent: None,
            conflicts_with: None,
            source: "adopted",
            source_detail: "",
            project_to: None,
        },
    )
    .unwrap();
    let head_before = record::heads(&fs, UID).unwrap().files["notes.md"].clone();
    assert_eq!(import_once(&fs, UID, Some(&[store]), |_| None, later()).unwrap(), Some(1));
    let history = bodies(&fs, "notes.md");
    assert_eq!(history.iter().map(|(b, _)| b.as_str()).collect::<Vec<_>>(), vec!["old", "current"], "oldest first");
    assert_eq!(record::heads(&fs, UID).unwrap().files["notes.md"], head_before, "the head is untouched");
}

#[test]
fn out_of_time_imports_nothing_and_tries_again_later() {
    let tmp = tempfile::tempdir().unwrap();
    let store = channel(tmp.path(), "a", &[("notes.md", "one", "agent")]);
    let fs = FileStore::open_in_memory().unwrap();
    let past = Instant::now() - Duration::from_secs(1);
    assert_eq!(import_once(&fs, UID, Some(&[store.clone()]), |_| None, past).unwrap(), None);
    assert_eq!(record::heads(&fs, UID).unwrap().imported_at_ms, None);
    assert_eq!(import_once(&fs, UID, Some(&[store]), |_| None, later()).unwrap(), Some(1));
}

/// A store that can't be read doesn't stop the others.
#[test]
fn an_unreadable_store_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("bad.db");
    std::fs::write(&bad, b"not a database").unwrap();
    let good = channel(tmp.path(), "good", &[("notes.md", "one", "agent")]);
    let fs = FileStore::open_in_memory().unwrap();
    assert_eq!(import_once(&fs, UID, Some(&[bad, good]), |_| None, later()).unwrap(), Some(1));
}

/// The record itself refuses a second import, inside its transaction, so
/// two passes racing past `import_once`'s own check import once.
#[test]
fn the_record_refuses_a_second_import() {
    let fs = FileStore::open_in_memory().unwrap();
    let rows = || {
        vec![ImportedVersion {
            file: "notes.md".into(),
            body: b"one".to_vec(),
            source: "agent".into(),
            source_detail: String::new(),
            created_at_ms: 1,
        }]
    };
    assert_eq!(record::import_history(&fs, UID, &rows()).unwrap(), Some(1));
    assert_eq!(record::import_history(&fs, UID, &rows()).unwrap(), None);
    assert_eq!(record::history(&fs, UID, "notes.md").unwrap().len(), 1);
}
