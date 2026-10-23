// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Crash and resume, as a table over every step, before and after its
//! "done" record: the next apply resumes and verifies, and a rollback from
//! the crashed journal gives back the original state.

use super::super::journal::{Journal, Record, STEPS};
use super::super::run::{self, Crash, Options, CRASHED};
use super::fixture::*;

fn crash_at(fx: &Fx, uid: &str, crash: Crash) {
    let opts = Options { crash: Some((uid.to_string(), crash)), ..Default::default() };
    assert_eq!(run::apply(&fx.home, &opts).unwrap_err(), CRASHED);
}

#[test]
fn a_run_resumes_after_a_crash_at_any_step() {
    for step in STEPS {
        for crash in [Crash::BeforeDone(step), Crash::AfterDone(step)] {
            let fx = Fx::standard();
            crash_at(&fx, LOAP3, crash);
            let report = super::apply(&fx.home).unwrap_or_else(|e| panic!("{crash:?}: resume failed: {e}"));
            assert_eq!(report.failed, None, "{crash:?}");
            let mut migrated: Vec<&str> = report.migrated.iter().map(|m| m.new_slug.as_str()).collect();
            migrated.sort();
            assert_eq!(migrated, vec!["loap-2", "loap-3", "naki-2"], "{crash:?}");
            let journal = Journal::open(std::path::Path::new(&report.journal)).unwrap();
            let done = journal.records.iter().filter(|r| matches!(r, Record::Done { uid, step: s, .. } if uid == LOAP3 && *s == step)).count();
            assert_eq!(done, 1, "{crash:?}: the step is recorded done once");
            assert!(journal.finished().is_some());
        }
    }
}

#[test]
fn a_crashed_run_rolls_back_to_the_original() {
    for step in STEPS {
        for crash in [Crash::BeforeDone(step), Crash::AfterDone(step)] {
            let fx = Fx::standard();
            let original = fx.snapshot();
            crash_at(&fx, LOAP3, crash);
            run::rollback(&fx.home, None).unwrap_or_else(|e| panic!("{crash:?}: rollback failed: {e}"));
            let back = fx.snapshot();
            let diff: Vec<_> = original.keys().chain(back.keys()).filter(|k| original.get(*k) != back.get(*k)).collect();
            assert!(diff.is_empty(), "{crash:?}: {diff:?}");
            assert!(super::super::plan(&fx.home, false).unwrap().candidates.iter().all(|c| c.refused.is_none()));
        }
    }
}

/// A rollback interrupted halfway resumes too: it skips the steps already
/// undone.
#[test]
fn a_rollback_is_resumable() {
    let fx = Fx::standard();
    let original = fx.snapshot();
    let report = super::apply(&fx.home).unwrap();
    let mut journal = Journal::open(std::path::Path::new(&report.journal)).unwrap();
    // As if the rollback stopped right after undoing its first step.
    let (uid, step, _) = journal.intents().into_iter().last().unwrap();
    journal.append(Record::Undo { uid: uid.clone(), step }).unwrap();
    run::rollback(&fx.home, Some(&report.journal)).unwrap();
    assert_eq!(fx.snapshot(), original);
    let again = run::rollback(&fx.home, Some(&report.journal)).unwrap();
    assert!(again.notes.iter().any(|n| n.contains("already rolled back")));
}
