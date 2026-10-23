// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A run: plan, preconditions, backup, then each agent's steps through the
//! journal and its verification, agent by agent (§5.2); and the two ways
//! back, journal mode and restore mode.

use std::path::{Path, PathBuf};

use super::backup::{self, BackupList};
use super::home::{fold, Home};
use super::journal::{journals, Journal, Record, Step, STEPS};
use super::preconditions;
use super::steps::{self, Agent, Ctx};
use super::{steps_files, verify};
use crate::backend::rpc_types::{ForkSlugCandidate, ForkSlugMigrated, ForkSlugReport};

/// Where a test stops a run, as a crash would.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Crash {
    /// After the step's change, before its result is recorded.
    BeforeDone(Step),
    /// After its result is recorded.
    AfterDone(Step),
}

#[derive(Default)]
pub(super) struct Options {
    #[cfg(test)]
    pub crash: Option<(String, Crash)>,
    /// Runs just before an agent's target is checked again, as another
    /// channel creating an agent during the backup would.
    #[cfg(test)]
    pub before_recheck: Option<Box<dyn Fn()>>,
}

pub(super) const CRASHED: &str = "crashed (test)";

fn agents(candidates: &[ForkSlugCandidate]) -> Vec<Agent> {
    candidates
        .iter()
        .filter(|c| c.refused.is_none())
        .map(|c| Agent { uid: c.uid.clone(), name: c.name.clone(), old: c.old_slug.clone(), new: c.new_slug.clone() })
        .collect()
}

/// The latest run that neither finished nor was rolled back: the one an
/// apply resumes.
fn unfinished(home: &Home) -> Result<Option<Journal>, String> {
    for path in journals(&home.backups()) {
        let j = Journal::open(&path)?;
        if j.finished().is_none() && !j.rolled_back() {
            return Ok(Some(j));
        }
    }
    Ok(None)
}

/// Every file a run of `agents` may change, for the backup. S8's moves
/// (`gh-<slug>`, the bashwrap cwd file) need none: they are moved back.
pub(super) fn backup_list(ctx: &Ctx, agents: &[Agent]) -> Result<BackupList, String> {
    let mut list = BackupList { databases: ctx.objects_dbs(), files: Vec::new() };
    list.databases.extend(ctx.wan_dbs());
    list.databases.extend(ctx.host_stores());
    for a in agents {
        list.files.push(steps_files::def_path(ctx.home, &a.uid));
        list.files.extend(steps_files::mcp_files(ctx, a)?);
        list.files.extend(steps_files::stale_registrations(ctx, a));
    }
    list.files.retain(|f| f.is_file());
    list.files.dedup();
    Ok(list)
}

fn new_run_dir(home: &Home) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    home.backups().join(format!("slug-migration-{stamp}"))
}

/// Migrate every candidate the plan doesn't refuse, or resume the run that
/// stopped. Refuses before any write when a precondition fails.
pub(super) fn apply(home: &Home, opts: &Options) -> Result<ForkSlugReport, String> {
    let _lock = preconditions::take_lock(home)?;
    let installs = home.installs()?;
    let mut journal = match unfinished(home)? {
        Some(j) => {
            let pending: Vec<ForkSlugCandidate> =
                j.header().1.candidates.iter().filter(|c| !j.verified(&c.uid)).cloned().collect();
            let mut refusals = Vec::new();
            for c in pending.iter().filter(|c| c.refused.is_none()) {
                let def = super::plan::load_defs(home)?.0.into_iter().find(|d| d.uid == c.uid);
                if let Some(def) = def {
                    for r in super::running::running(home, &installs, &def)? {
                        refusals.push(format!("{} is running ({}): stop it first", c.name, r.how));
                    }
                }
            }
            refusals.extend(preconditions::old_instances(home, &installs));
            if !refusals.is_empty() {
                return Err(format!("not resuming the run in {}: {}", j.dir().display(), refusals.join("; ")));
            }
            j
        }
        None => {
            let plan = super::plan::plan_with(home, false, false)?;
            if !plan.blockers.is_empty() {
                let why: Vec<&str> = plan.blockers.iter().map(|b| b.message.as_str()).collect();
                return Err(format!("not migrating: {}", why.join("; ")));
            }
            let ready = agents(&plan.candidates);
            if ready.is_empty() {
                return Ok(ForkSlugReport { notes: vec![plan.summary.clone()], ..Default::default() });
            }
            let dir = new_run_dir(home);
            let ctx = Ctx { home, installs: &installs, run_dir: dir.clone(), backups: &[], retired_at: 0 };
            let entries = backup::take(&dir, &backup_list(&ctx, &ready)?)?;
            let now = agentmux_common::time::now_ms();
            Journal::create(&dir, now, now, plan, entries)?
        }
    };
    let (retired_at, plan, backups) = journal.header();
    let (plan, backups) = (plan.clone(), backups.to_vec());
    let ctx = Ctx { home, installs: &installs, run_dir: journal.dir(), backups: &backups, retired_at };
    for a in agents(&plan.candidates) {
        if journal.verified(&a.uid) || journal.skipped(&a.uid).is_some() {
            continue;
        }
        if let Err(e) = run_agent_reserved(&ctx, &mut journal, &a, opts) {
            if e != CRASHED {
                journal.append(Record::Failed { uid: a.uid.clone(), error: e.clone() })?;
            }
            return Err(e);
        }
        if journal.skipped(&a.uid).is_some() {
            continue;
        }
        let problems = verify::verify(&ctx, &a)?;
        if !problems.is_empty() {
            let error = format!("{} didn't verify: {}", a.name, problems.join("; "));
            journal.append(Record::Failed { uid: a.uid.clone(), error: error.clone() })?;
            return Ok(report(&journal, &plan, Some(error)));
        }
        journal.append(Record::Verified { uid: a.uid.clone() })?;
    }
    // Only files: restore mode compares them, and takes databases back row
    // by row without a digest.
    let mut after = Vec::new();
    for b in backups.iter().filter(|b| !b.database) {
        after.push(backup::fingerprint(Path::new(&b.original), false)?);
    }
    journal.append(Record::Finished { at: agentmux_common::time::now_ms(), after })?;
    Ok(report(&journal, &plan, None))
}

/// One agent's steps. Before its first write, its new id is checked again
/// under the host-wide slug allocation lock, the one every channel creating
/// an agent holds while it picks and publishes a slug: the plan was made
/// before the backup, which can take a while, and another channel may have
/// handed the id out since. The lock is held through S1 and S2, which
/// publish the new id in the shared definition and every `objects.db`, and
/// released before the rest. A target taken in the meantime leaves the agent
/// out of the run with nothing written for it (`Record::Skipped`).
///
/// Only a finished S1 reserves the id, so a resume checks again until S1 is
/// done; a target taken meanwhile stops the run for a rollback (#5342).
fn run_agent_reserved(ctx: &Ctx, journal: &mut Journal, a: &Agent, opts: &Options) -> Result<(), String> {
    if journal.done(&a.uid, Step::S1) {
        return run_agent(ctx, journal, a, opts, &STEPS);
    }
    #[cfg(test)]
    if let Some(hook) = &opts.before_recheck {
        hook();
    }
    let store = crate::registry::DefinitionStore::open(ctx.home.definitions()).map_err(|e| e.to_string())?;
    let allocation = store.lock_slug_allocation().map_err(|e| e.to_string())?;
    let (defs, unreadable) = super::plan::load_defs(ctx.home)?;
    let holder = if unreadable.is_empty() {
        super::plan::holder_of(ctx.home, ctx.installs, &defs, &Default::default(), &a.uid, &a.new)?
    } else {
        Some(format!("the shared definitions can't all be read ({})", unreadable.join("; ")))
    };
    if let Some(holder) = holder {
        if journal.intent(&a.uid, Step::S1).is_some() {
            return Err(format!(
                "{}: {} was taken while the run was interrupted ({holder}); roll this run back from its journal",
                a.name, a.new
            ));
        }
        let reason = format!("{} wasn't updated: {} was taken after the plan ({holder})", a.name, a.new);
        tracing::warn!(uid = %a.uid, target = %a.new, %holder, "fork slug migration: target taken since the plan; agent left out");
        journal.append(Record::Skipped { uid: a.uid.clone(), reason })?;
        return Ok(());
    }
    run_agent(ctx, journal, a, opts, &STEPS[..2])?;
    drop(allocation);
    run_agent(ctx, journal, a, opts, &STEPS[2..])
}

/// `steps` of one agent, each recorded before and after it runs.
fn run_agent(ctx: &Ctx, journal: &mut Journal, a: &Agent, opts: &Options, steps: &[Step]) -> Result<(), String> {
    let _ = opts;
    for &step in steps {
        if journal.done(&a.uid, step) {
            continue;
        }
        let before = match journal.intent(&a.uid, step) {
            Some(b) => b.clone(),
            None => {
                let b = steps::before(ctx, a, step)?;
                journal.append(Record::Intent { uid: a.uid.clone(), step, before: b.clone() })?;
                b
            }
        };
        let changes = steps::apply(ctx, a, step, &before).map_err(|e| format!("{} {step:?}: {e}", a.name))?;
        #[cfg(test)]
        if opts.crash == Some((a.uid.clone(), Crash::BeforeDone(step))) {
            return Err(CRASHED.into());
        }
        journal.append(Record::Done { uid: a.uid.clone(), step, changes })?;
        #[cfg(test)]
        if opts.crash == Some((a.uid.clone(), Crash::AfterDone(step))) {
            return Err(CRASHED.into());
        }
    }
    Ok(())
}

fn report(journal: &Journal, plan: &crate::backend::rpc_types::ForkSlugPlan, failed: Option<String>) -> ForkSlugReport {
    let migrated: Vec<ForkSlugMigrated> = plan
        .candidates
        .iter()
        .filter(|c| journal.verified(&c.uid))
        .map(|c| ForkSlugMigrated {
            uid: c.uid.clone(),
            name: c.name.clone(),
            old_slug: c.old_slug.clone(),
            new_slug: c.new_slug.clone(),
            changes: journal.changes(&c.uid),
        })
        .collect();
    let pr_searches = migrated
        .iter()
        .map(|m| format!("gh-agent pr list --state open --search \"agentmux:agent_id={} in:body\"", fold(&m.old_slug)))
        .collect();
    let mut notes: Vec<String> = plan.candidates.iter().filter_map(|c| journal.skipped(&c.uid).map(str::to_string)).collect();
    if !migrated.is_empty() {
        notes.extend([
            "Relaunch each migrated agent: it takes its new id, fresh signing keys and a new WAN key publication at its next spawn.".to_string(),
            "Run each search in every repository the agent opens PRs in, and change agentmux:agent_id in each body with gh-agent pr edit; for 30 days the old id still receives review notices.".to_string(),
            "The old id's WAN key record stays in the account's key directory: nothing on this side can remove it, and no key here signs under that name any more.".to_string(),
            "LAN peers pin the new id's key on first use; other agents' notes and memory that mention the old id are theirs to update.".to_string(),
            format!(
                "The backup in {} holds copies of the databases, signing keys and credentials included: delete it once you won't roll back.",
                journal.dir().display()
            ),
        ]);
    }
    ForkSlugReport {
        journal: journal.path.display().to_string(),
        backup_dir: journal.dir().display().to_string(),
        migrated,
        failed: failed.or_else(|| journal.last_failure()),
        pr_searches,
        notes,
    }
}

/// The journal to roll back: `path`, or the latest run.
fn pick(home: &Home, path: Option<&str>) -> Result<Journal, String> {
    match path {
        Some(p) => Journal::open(Path::new(p)),
        None => {
            let latest = journals(&home.backups()).into_iter().next().ok_or("no fork slug migration has run here")?;
            Journal::open(&latest)
        }
    }
}

/// Refuse to undo `journal`'s run while any agent it touched runs anywhere
/// on this host, under its old id or its new one (a relaunch takes the new
/// one), or while an AgentMux too old for former slugs runs: either would
/// keep using, or write back, what the undo changes. The caller holds the
/// migration lock.
fn refuse_undo_while_running(home: &Home, installs: &[super::home::Install], journal: &Journal) -> Result<(), String> {
    let (defs, _) = super::plan::load_defs(home)?;
    let touched: std::collections::HashSet<String> = journal.intents().into_iter().map(|(uid, _, _)| uid).collect();
    let mut why = Vec::new();
    for c in journal.header().1.candidates.iter().filter(|c| touched.contains(&c.uid)) {
        let mut def = defs.iter().find(|d| d.uid == c.uid).cloned().unwrap_or_else(|| super::plan::Def {
            uid: c.uid.clone(),
            name: c.name.clone(),
            slug: c.new_slug.clone(),
            ..Default::default()
        });
        def.former.extend([c.old_slug.clone(), c.new_slug.clone()]);
        for r in super::running::running(home, installs, &def)? {
            let at = if r.channel.is_empty() { String::new() } else { format!(" in {}", r.channel) };
            why.push(format!("{} is running{at} ({}): stop it first", c.name, r.how));
        }
    }
    why.extend(preconditions::old_instances(home, installs));
    if why.is_empty() {
        Ok(())
    } else {
        Err(format!("not rolling back: {}", why.join("; ")))
    }
}

/// Journal mode: undo every step that has an intent, newest first, each
/// from its before-image. Resumable: a step already undone is skipped.
pub(super) fn rollback(home: &Home, path: Option<&str>) -> Result<ForkSlugReport, String> {
    let _lock = preconditions::take_lock(home)?;
    let mut journal = pick(home, path)?;
    if journal.rolled_back() {
        return Ok(ForkSlugReport {
            journal: journal.path.display().to_string(),
            backup_dir: journal.dir().display().to_string(),
            notes: vec!["this run was already rolled back".into()],
            ..Default::default()
        });
    }
    let installs = home.installs()?;
    refuse_undo_while_running(home, &installs, &journal)?;
    let (retired_at, plan, backups) = journal.header();
    let (plan, backups) = (plan.clone(), backups.to_vec());
    let ctx = Ctx { home, installs: &installs, run_dir: journal.dir(), backups: &backups, retired_at };
    let mut notes = Vec::new();
    for (uid, step, before) in journal.intents().into_iter().rev() {
        if journal.undone(&uid, step) {
            continue;
        }
        let Some(c) = plan.candidates.iter().find(|c| c.uid == uid) else { continue };
        let a = Agent { uid: uid.clone(), name: c.name.clone(), old: c.old_slug.clone(), new: c.new_slug.clone() };
        journal.append(Record::Undo { uid: uid.clone(), step })?;
        let changes = steps::undo(&ctx, &a, step, &before).map_err(|e| format!("{} {step:?} undo: {e}", a.name))?;
        notes.extend(changes.iter().map(|c| format!("{} {step:?}: {c}", a.name)));
        journal.append(Record::Undone { uid, step, changes })?;
    }
    journal.append(Record::RolledBack { at: agentmux_common::time::now_ms(), mode: "journal".into() })?;
    Ok(ForkSlugReport {
        journal: journal.path.display().to_string(),
        backup_dir: journal.dir().display().to_string(),
        notes,
        ..Default::default()
    })
}

/// Restore mode: put the backed-up files back as they were, only while every
/// one the run left is unchanged since it finished (its bytes and
/// modification time); the moved folders go back through the journal.
///
/// Databases go back row by row through journal mode's database undos, never
/// as a whole copy, which would drop other panes' writes since (#5342).
pub(super) fn restore(home: &Home, path: Option<&str>) -> Result<ForkSlugReport, String> {
    let _lock = preconditions::take_lock(home)?;
    let mut journal = pick(home, path)?;
    if journal.rolled_back() {
        return Err("this run was already rolled back".into());
    }
    let after = journal.finished().ok_or("this run didn't finish: roll it back from its journal instead")?.to_vec();
    refuse_undo_while_running(home, &home.installs()?, &journal)?;
    let mut changed = Vec::new();
    for f in after.iter().filter(|f| !is_database(&journal, &f.path)) {
        let now = backup::fingerprint(Path::new(&f.path), false)?;
        if now.digest != f.digest || now.mtime_ms != f.mtime_ms {
            changed.push(f.path.clone());
        }
    }
    if !changed.is_empty() {
        return Err(format!("written since the run, so the backup can't be put back: {}; roll back from the journal instead", changed.join(", ")));
    }
    let (retired_at, plan, backups) = journal.header();
    let (plan, backups) = (plan.clone(), backups.to_vec());
    let dir = journal.dir();
    let mut notes = Vec::new();
    for b in backups.iter().filter(|b| !b.database) {
        let copy = backup::copy_of(&dir, &backups, Path::new(&b.original))?;
        let bytes = std::fs::read(&copy).map_err(|e| e.to_string())?;
        super::home::write_file(Path::new(&b.original), &bytes)?;
        notes.push(format!("restored {}", b.original));
    }
    let installs = home.installs()?;
    let ctx = Ctx { home, installs: &installs, run_dir: dir.clone(), backups: &backups, retired_at };
    for (uid, step, before) in journal.intents().into_iter().rev() {
        let Some(c) = plan.candidates.iter().find(|c| c.uid == uid) else { continue };
        let a = Agent { uid: uid.clone(), name: c.name.clone(), old: c.old_slug.clone(), new: c.new_slug.clone() };
        let changes = match step {
            // S1's file came back with the others above; S5 changes nothing.
            Step::S1 | Step::S5 => continue,
            Step::S2 | Step::S3 | Step::S4 | Step::S6 | Step::S8 => steps::undo(&ctx, &a, step, &before)?,
            // Its `.mcp.json` files came back above; its rows go back here.
            Step::S7 => super::steps_db::s7_undo(&a, &before)?,
        };
        notes.extend(changes.iter().map(|c| format!("{} {step:?}: {c}", a.name)));
    }
    journal.append(Record::RolledBack { at: agentmux_common::time::now_ms(), mode: "restore".into() })?;
    Ok(ForkSlugReport { journal: journal.path.display().to_string(), backup_dir: dir.display().to_string(), notes, ..Default::default() })
}

fn is_database(journal: &Journal, path: &str) -> bool {
    journal.header().2.iter().any(|b| b.original == path && b.database)
}
