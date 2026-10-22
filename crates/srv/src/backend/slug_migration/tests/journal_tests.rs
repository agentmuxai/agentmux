// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The journal survives a write cut short: a torn last record is cut off the
//! file on open, and a record never shares a line with the one before it.

use std::path::Path;

use super::super::journal::{Journal, Record, Step};
use super::super::run::{self, Crash, Options};
use super::fixture::*;

fn new_journal(dir: &Path) -> Journal {
    let mut j = Journal::create(dir, 1, 1, Default::default(), Vec::new()).unwrap();
    j.append(Record::Intent { uid: "u".into(), step: Step::S1, before: serde_json::json!({}) }).unwrap();
    j
}

fn append_raw(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(bytes).unwrap();
}

#[test]
fn a_torn_last_record_is_cut_off_and_the_next_appends_parse() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_journal(dir.path()).path;
    let whole = std::fs::read(&path).unwrap();
    append_raw(&path, br#"{"t":"done","uid":"u","st"#);

    let mut j = Journal::open(&path).unwrap();
    assert_eq!(j.records.len(), 2, "the torn record is discarded");
    assert_eq!(std::fs::read(&path).unwrap(), whole, "and cut off the file, back to its last complete line");
    j.append(Record::Done { uid: "u".into(), step: Step::S1, changes: vec![] }).unwrap();
    j.append(Record::Verified { uid: "u".into() }).unwrap();

    let again = Journal::open(&path).unwrap();
    assert_eq!(again.records.len(), 4);
    assert!(again.done("u", Step::S1) && again.verified("u"));
}

#[test]
fn an_append_onto_a_record_without_its_newline_starts_a_line_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_journal(dir.path()).path;
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'));
    std::fs::write(&path, &bytes).unwrap();

    let mut j = Journal::open(&path).unwrap();
    assert_eq!(j.records.len(), 2, "a complete record missing only its newline is kept");
    j.append(Record::Done { uid: "u".into(), step: Step::S1, changes: vec![] }).unwrap();
    let again = Journal::open(&path).unwrap();
    assert_eq!(again.records.len(), 3);
    assert!(again.done("u", Step::S1));
    assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));
}

/// A line that doesn't parse anywhere but last is a real corruption, not a
/// torn write: the journal refuses to open rather than guess.
#[test]
fn a_bad_line_before_the_last_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = new_journal(dir.path());
    append_raw(&j.path, b"not json\n");
    j.append(Record::Verified { uid: "u".into() }).unwrap();
    let Err(e) = Journal::open(&j.path) else { panic!("a corrupt journal opened") };
    assert!(e.contains("line 3"), "{e}");
}

/// A crash in the middle of writing a record, then a resume and a rollback,
/// as a power loss would leave them.
#[test]
fn a_run_resumes_and_rolls_back_after_a_torn_record() {
    let fx = Fx::standard();
    let original = fx.snapshot();
    let opts = Options { crash: Some((LOAP3.to_string(), Crash::BeforeDone(Step::S4))), ..Default::default() };
    assert!(run::apply(&fx.home, &opts).is_err());
    let path = super::super::journal::journals(&fx.home.backups()).remove(0);
    append_raw(&path, br#"{"t":"done","uid":"uid-loap-3","step":"S4","chan"#);

    let report = super::apply(&fx.home).unwrap();
    assert_eq!(report.failed, None);
    assert_eq!(report.migrated.len(), 3);
    let j = Journal::open(&path).unwrap();
    assert!(j.finished().is_some() && j.done(LOAP3, Step::S4));

    run::rollback(&fx.home, Some(&report.journal)).unwrap();
    let back = fx.snapshot();
    let diff: Vec<_> = original.keys().chain(back.keys()).filter(|k| original.get(*k) != back.get(*k)).collect();
    assert!(diff.is_empty(), "{diff:?}");
    assert!(Journal::open(&path).unwrap().rolled_back());
}
