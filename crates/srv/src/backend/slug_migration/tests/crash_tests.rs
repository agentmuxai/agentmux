// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Crash and resume, as a table over every step, before and after its
//! "done" record: the next apply resumes and verifies, and a rollback from
//! the crashed journal gives back the original state.

use super::super::journal::{Journal, Record, Step, STEPS};
use super::super::run::{self, Crash, Options, CRASHED};
use super::fixture::*;

fn crash_at(fx: &Fx, uid: &str, crash: Crash) {
    let opts = Options { crash: Some((uid.to_string(), crash)), ..Default::default() };
    assert_eq!(run::apply(&fx.home, &opts).unwrap_err(), CRASHED);
}

/// A crash at `crash`, then the next apply resumes and verifies.
fn a_run_resumes_after(crash: Crash, step: Step) {
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

/// A crash at `crash`, then a rollback gives back the original state.
fn a_crashed_run_rolls_back_from(crash: Crash) {
    let fx = Fx::standard();
    let original = fx.snapshot();
    crash_at(&fx, LOAP3, crash);
    run::rollback(&fx.home, None).unwrap_or_else(|e| panic!("{crash:?}: rollback failed: {e}"));
    let back = fx.snapshot();
    let diff: Vec<_> = original.keys().chain(back.keys()).filter(|k| original.get(*k) != back.get(*k)).collect();
    assert!(diff.is_empty(), "{crash:?}: {diff:?}");
    assert!(super::super::plan(&fx.home, false).unwrap().candidates.iter().all(|c| c.refused.is_none()));
}

/// One test per step and crash point, not one loop over all sixteen: each case builds a fixture of several
/// databases and runs a whole apply, which takes seconds on a Windows runner's disk, so the loop took
/// minutes and hit the test runner's kill time. As separate tests the cases run in parallel, each in its
/// own process, and a failure names the case.
macro_rules! crash_cases {
    ($($module:ident: $step:ident),* $(,)?) => {
        mod resumes {
            use super::*;
            $(
                mod $module {
                    use super::*;
                    #[test]
                    fn before_done() {
                        a_run_resumes_after(Crash::BeforeDone(Step::$step), Step::$step);
                    }
                    #[test]
                    fn after_done() {
                        a_run_resumes_after(Crash::AfterDone(Step::$step), Step::$step);
                    }
                }
            )*
        }
        mod rolls_back {
            use super::*;
            $(
                mod $module {
                    use super::*;
                    #[test]
                    fn before_done() {
                        a_crashed_run_rolls_back_from(Crash::BeforeDone(Step::$step));
                    }
                    #[test]
                    fn after_done() {
                        a_crashed_run_rolls_back_from(Crash::AfterDone(Step::$step));
                    }
                }
            )*
        }
        /// The cases above are listed by hand: a new step needs its own.
        const CASES: usize = [$(stringify!($step)),*].len();
    };
}

crash_cases!(s1: S1, s2: S2, s3: S3, s4: S4, s5: S5, s6: S6, s7: S7, s8: S8);

#[test]
fn every_step_has_its_crash_cases() {
    assert_eq!(STEPS.len(), CASES, "add the new step to crash_cases!");
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
