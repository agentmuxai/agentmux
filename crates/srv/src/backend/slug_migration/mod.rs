// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The fork slug migration (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md`
//! §4 Phase 2): an operation the operator starts, never a boot step, that
//! gives each pre-fix fork the id `fork_slug` gives (`loap-loap-2` →
//! `loap-2`) and keeps the old one as a former slug, which Phase 1 already
//! routes and reserves.
//!
//! - **Plan** (`plan.rs`, the default): candidates by §5.1, a collision check
//!   across every definition, every `objects.db` on disk and every live
//!   registration, and every place the old slug is stored. Changes nothing.
//!   It also reports slugs two agents hold (`duplicates.rs`), with a proposed
//!   id for the later one, and renames neither.
//! - **Holders** (`holders.rs`): the same stores read in one pass, which a new
//!   agent's slug is picked against too (`Store::agent_def_insert`).
//! - **Preconditions** (`preconditions.rs`): the agents run nowhere on this
//!   host, every running AgentMux understands former slugs (`stamp.rs`), and
//!   the host-wide lock `shared/locks/slug-migration.lock` is taken.
//! - **Backup** (`backup.rs`) into `shared/backups/slug-migration-<ts>/`.
//! - **Apply** (`run.rs`, `steps*.rs`): S1–S8 per agent, parent first, each
//!   recorded in the write-ahead journal (`journal.rs`) before and after it
//!   runs; then **verify** (`verify.rs`) before the next agent.
//! - **Roll back**: from the journal, step by step, or by restoring the
//!   backup while nothing has changed since.
//!
//! It works on the disk, not on this instance's stores, so it covers
//! every channel and version and runs the same from the CLI
//! (`agentmux-srv migrate-fork-slugs`) and from `agents.migrateForkSlugs`.

mod backup;
mod duplicates;
mod holders;
mod home;
mod journal;
mod locations;
mod plan;
mod preconditions;
mod run;
mod running;
pub mod stamp;
mod steps;
mod steps_db;
mod steps_files;
mod verify;

#[cfg(test)]
mod tests;

pub use holders::HostSlugs;
pub use home::Home;

use crate::backend::rpc_types::{ForkSlugChoice, ForkSlugMigrationMode, ForkSlugPlan, ForkSlugReport, MigrateForkSlugsResult};

/// The plan for `home`; `quick` finds the candidates only.
pub fn plan(home: &Home, quick: bool) -> Result<ForkSlugPlan, String> {
    plan::plan(home, quick)
}

/// `plan`, `apply`, `rollback` or `restore` on `home`. `stop` needs the
/// running instance and is the RPC's.
pub fn run(home: &Home, mode: ForkSlugMigrationMode, journal: Option<&str>, quick: bool) -> Result<MigrateForkSlugsResult, String> {
    run_choosing(home, mode, journal, quick, &[])
}

/// The plan for `home` with ids the user picked for some agents (`choices`).
pub fn plan_choosing(home: &Home, quick: bool, choices: &[ForkSlugChoice]) -> Result<ForkSlugPlan, String> {
    plan::plan_choosing(home, quick, choices)
}

/// [`run`] with ids the user picked for some agents; `plan` and `apply` take them, the
/// other modes ignore them.
pub fn run_choosing(
    home: &Home,
    mode: ForkSlugMigrationMode,
    journal: Option<&str>,
    quick: bool,
    choices: &[ForkSlugChoice],
) -> Result<MigrateForkSlugsResult, String> {
    let result = |plan: Option<ForkSlugPlan>, report: Option<ForkSlugReport>| {
        let summary = match (&plan, &report) {
            (Some(p), _) => p.summary.clone(),
            (_, Some(r)) => report_summary(r),
            _ => String::new(),
        };
        MigrateForkSlugsResult { mode, plan, report, pending: Vec::new(), wait_ms: 0, summary }
    };
    match mode {
        ForkSlugMigrationMode::Plan => Ok(result(Some(plan::plan_choosing(home, quick, choices)?), None)),
        ForkSlugMigrationMode::Apply => Ok(result(None, Some(run::apply_choosing(home, &run::Options::default(), choices)?))),
        ForkSlugMigrationMode::Rollback => Ok(result(None, Some(run::rollback(home, journal)?))),
        ForkSlugMigrationMode::Restore => Ok(result(None, Some(run::restore(home, journal)?))),
        ForkSlugMigrationMode::Stop => Err("stop runs in the app, through agents.migrateForkSlugs".into()),
    }
}

/// A report as text: what changed, what is left, and the searches to run.
pub fn report_summary(r: &ForkSlugReport) -> String {
    let mut lines = Vec::new();
    for m in &r.migrated {
        lines.push(format!("{}: {} -> {}", m.name, m.old_slug, m.new_slug));
        lines.extend(m.changes.iter().map(|c| format!("  {c}")));
    }
    if let Some(f) = &r.failed {
        lines.push(format!("Stopped: {f}"));
    }
    if !r.pr_searches.is_empty() {
        lines.push("Open PRs that may still carry an old id:".into());
        lines.extend(r.pr_searches.iter().map(|s| format!("  {s}")));
    }
    lines.extend(r.notes.iter().cloned());
    if !r.journal.is_empty() {
        lines.push(format!("Journal: {}", r.journal));
    }
    lines.join("\n")
}

/// `agentmux-srv migrate-fork-slugs`: print the result's summary and its
/// JSON; exit 0 on success, 1 on a refusal or an error.
pub fn run_cli(mode: ForkSlugMigrationMode, journal: Option<&str>) -> i32 {
    let outcome = Home::from_env().and_then(|home| run(&home, mode, journal, false));
    match outcome {
        Ok(result) => {
            eprintln!("{}", result.summary);
            match serde_json::to_string_pretty(&result) {
                Ok(json) => println!("{json}"),
                Err(e) => eprintln!("migrate-fork-slugs: {e}"),
            }
            i32::from(result.report.as_ref().is_some_and(|r| r.failed.is_some()))
        }
        Err(e) => {
            eprintln!("migrate-fork-slugs: {e}");
            1
        }
    }
}
