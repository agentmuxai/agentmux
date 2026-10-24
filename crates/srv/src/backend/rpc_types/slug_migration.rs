// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agents.migrateForkSlugs`: the fork slug migration's plan, its report, and
//! the request that asks for either (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md`
//! §4 Phase 2). The migration itself is `backend::slug_migration`.

use serde::{Deserialize, Serialize};

/// What `agents.migrateForkSlugs` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum ForkSlugMigrationMode {
    /// Work out what would change, and change nothing.
    Plan,
    /// Stop the panes the plan finds running, each with its user's 15 s
    /// override; nothing is migrated.
    Stop,
    /// Migrate every agent the plan can, or resume a run that stopped.
    Apply,
    /// Undo a run from its journal, step by step.
    Rollback,
    /// Put the backup back, only while nothing has changed since the run.
    Restore,
}

/// Request for `agents.migrateForkSlugs`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandMigrateForkSlugsData {
    pub mode: ForkSlugMigrationMode,
    /// The run's `journal.jsonl`, for `rollback` and `restore`; the latest
    /// finished run when absent.
    #[serde(default)]
    #[ts(optional)]
    pub journal: Option<String>,
    /// `plan` only: find the candidates and nothing else (no collision
    /// check, no locations, no preconditions), for the startup notice.
    #[serde(default)]
    #[ts(optional)]
    pub quick: Option<bool>,
}

/// One place an agent's old slug is stored, with what it holds there. Never
/// a key or a secret: a key row is reported by its presence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugLocation {
    /// `K1` to `K13`, as the spec's §3 table numbers them.
    pub key: String,
    /// The database, file or directory.
    pub place: String,
    pub value: String,
}

/// Where an affected agent is running now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugRunning {
    /// The pane, when known.
    pub block_id: String,
    pub channel: String,
    /// How it was found: a lease, a registration.
    pub how: String,
}

/// An agent whose slug the old fork rule chained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugCandidate {
    pub uid: String,
    pub name: String,
    pub old_slug: String,
    pub new_slug: String,
    /// Why this agent can't be migrated now (its new id is taken); the others
    /// still can.
    #[serde(default)]
    #[ts(optional)]
    pub refused: Option<String>,
    /// The next free id, when the new one is taken. Never used by itself.
    #[serde(default)]
    #[ts(optional)]
    pub suggestion: Option<String>,
    #[serde(default)]
    pub locations: Vec<ForkSlugLocation>,
    #[serde(default)]
    pub running: Vec<ForkSlugRunning>,
}

/// A fork left alone, and why (not a pre-fix chain, a missing parent, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugSkipped {
    pub uid: String,
    pub name: String,
    pub slug: String,
    pub reason: String,
}

/// What kind of thing blocks a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum ForkSlugBlockerKind {
    /// An affected agent runs: stopping its pane clears it.
    Running,
    /// An AgentMux too old to know former slugs runs on this host.
    OldInstance,
    /// Another migration holds the host-wide lock.
    Locked,
    /// A shared definition can't be read, so no new id can be checked.
    Unreadable,
}

/// One reason a run can't start now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugBlocker {
    pub kind: ForkSlugBlockerKind,
    pub message: String,
    /// Whether stopping the affected panes (`stop`) clears it. Only a
    /// running agent does; nothing else is worth stopping a pane for.
    pub clearable_by_stopping: bool,
}

impl ForkSlugBlocker {
    pub fn new(kind: ForkSlugBlockerKind, message: impl Into<String>) -> Self {
        ForkSlugBlocker { kind, message: message.into(), clearable_by_stopping: kind == ForkSlugBlockerKind::Running }
    }
}

/// What a migration would do. Computing it changes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugPlan {
    /// Parent first, so a chain is migrated from its root down.
    pub candidates: Vec<ForkSlugCandidate>,
    pub skipped: Vec<ForkSlugSkipped>,
    /// What would refuse an `apply` now: a running agent, an AgentMux too
    /// old to know former slugs, another migration under way.
    pub blockers: Vec<ForkSlugBlocker>,
    pub summary: String,
    /// The latest run, unless it was rolled back: what Settings resumes or
    /// undoes after the session that ran it is gone.
    #[serde(default)]
    #[ts(optional)]
    pub last_run: Option<ForkSlugLastRun>,
    /// Slugs two or more agents hold: reported, never changed by a run.
    #[serde(default)]
    pub duplicates: Vec<ForkSlugDuplicate>,
}

/// One slug held by more than one agent (two uids, one slug), as an older
/// build could leave it: agents made in two channels each picked it, unique
/// in their own `objects.db`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugDuplicate {
    pub slug: String,
    /// Oldest first: the first keeps the slug.
    pub holders: Vec<ForkSlugDuplicateHolder>,
}

/// An agent holding a duplicated slug.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugDuplicateHolder {
    pub uid: String,
    pub name: String,
    /// When it was made, in ms; 0 when unknown.
    #[ts(type = "number")]
    pub created_at: i64,
    /// Where it holds the slug: the shared definitions, or the `objects.db`
    /// of an install (an agent never published).
    pub place: String,
    /// For each holder after the first, the first free id a rename could
    /// give it. A proposal only: nothing renames it.
    #[serde(default)]
    #[ts(optional)]
    pub proposed: Option<String>,
}

/// A run that wasn't rolled back.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugLastRun {
    pub journal: String,
    /// False while an apply would resume it.
    pub finished: bool,
    /// Why it stopped, while nothing has moved on since.
    #[serde(default)]
    #[ts(optional)]
    pub failed: Option<String>,
}

/// One agent a run migrated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugMigrated {
    pub uid: String,
    pub name: String,
    pub old_slug: String,
    pub new_slug: String,
    /// What each step changed, in order.
    pub changes: Vec<String>,
}

/// What a run did (`apply`, `rollback`, `restore`), and what is left to the
/// agents: srv can't edit a PR body or another agent's notes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugReport {
    pub journal: String,
    pub backup_dir: String,
    pub migrated: Vec<ForkSlugMigrated>,
    /// Set when the run stopped: the agent, the step and why. The journal
    /// resumes or reverses it.
    #[serde(default)]
    #[ts(optional)]
    pub failed: Option<String>,
    /// Ready-to-run searches for open PRs whose body still names an old id.
    pub pr_searches: Vec<String>,
    pub notes: Vec<String>,
}

/// A pane `stop` asked to stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ForkSlugStopPending {
    pub block_id: String,
    pub request_id: String,
}

/// Result of `agents.migrateForkSlugs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MigrateForkSlugsResult {
    pub mode: ForkSlugMigrationMode,
    #[serde(default)]
    #[ts(optional)]
    pub plan: Option<ForkSlugPlan>,
    #[serde(default)]
    #[ts(optional)]
    pub report: Option<ForkSlugReport>,
    /// `stop`: the panes now counting down, and how long their override lasts.
    #[serde(default)]
    pub pending: Vec<ForkSlugStopPending>,
    #[serde(default)]
    #[ts(type = "number")]
    pub wait_ms: u64,
    pub summary: String,
}
