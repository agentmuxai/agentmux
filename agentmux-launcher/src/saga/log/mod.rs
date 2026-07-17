// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// In-memory launcher saga registry — Pillar 1 Step 6
// (docs/specs/SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16.md).
//
// This replaced the durable SQLite saga log (sagas.db / launcher-sagas.db,
// `SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`) wholesale. The durable
// layer's crash-time behavior was, by its own design, a no-op: the LSD-3
// recovery walker never replayed or compensated anything — it only marked
// interrupted rows `failed_compensation` for operator review via
// `--diag sagas` ("we DO NOT auto-replay or auto-compensate launcher
// sagas", the deleted recovery.rs:19). With srv authoritative over session
// state and crash-reproject rebuilding windows from it (Pillar 1 Steps
// 1–5), an interrupted launcher saga leaves nothing durable to review:
// both concrete sagas (window_cleanup_cascade, pool_respawn) narrate
// cleanup the host performs organically, and their srv-side effects are
// reconstructed by reproject regardless. The SQLite file bought WAL churn
// every session in exchange for a diagnostic tombstone.
//
// What stays: the LIVE coordinator semantics. The registry keeps the same
// method surface the coordinator drives (start/terminate saga,
// start/finish/fail step, snapshot for diagnostics) so `--diag`-style
// in-process introspection and the `[saga]` launcher-log narration are
// unchanged. What's gone: `open(path)` / `open_read_only` (no file),
// `vacuum_older_than` (replaced by a bounded in-memory retention cap),
// and the startup recovery walker (a fresh process has an empty registry
// by construction — there is nothing to walk).
//
// Concurrency: a single `Mutex<Inner>` — same serialization the SQLite
// `Mutex<Connection>` provided, without the I/O under the lock.

use std::collections::BTreeMap;
use std::sync::Mutex;

use agentmux_common::ipc::{Command, Event};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::PipeTarget;

#[cfg(test)]
mod tests;

/// Retention cap for TERMINAL sagas (completed / failed /
/// failed_compensation). When exceeded, the oldest terminal sagas are
/// evicted. In-flight sagas are never evicted — mirroring the old
/// vacuum's "never vacuum running/compensating" rule. 128 comfortably
/// exceeds anything a session produces (each window close = 1 saga,
/// each pool promote = 1 saga) while bounding memory at a few hundred KB
/// worst case.
const TERMINAL_RETENTION_CAP: usize = 128;

/// Errors from the saga registry. JSON serialization is the only
/// fallible operation left after the SQLite layer's removal; the
/// duplicate-id insert error is preserved from the durable log's
/// PRIMARY KEY semantics ("a duplicate saga_id is a bug worth
/// surfacing, not a silent overwrite").
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("duplicate saga_id {0}")]
    DuplicateSagaId(u64),
}

/// Outcome of a launcher saga, written by `terminate_saga`.
///
/// `FailedCompensation` survives the durability collapse: it is no
/// longer produced by a recovery walker (deleted), but the state
/// string remains part of the diagnostic vocabulary and the variant
/// is kept so `mark_failed_compensation` (retained for API parity and
/// tests) still has its terminal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SagaOutcome {
    /// Saga ran to completion successfully. `SagaAction::Done` path.
    Completed,
    /// Saga failed with no compensation having run.
    Failed { reason: String },
    /// Terminal "needs operator attention" state. Historically written
    /// by the startup recovery walker; retained in the vocabulary.
    #[allow(dead_code)]
    FailedCompensation { reason: String },
}

impl SagaOutcome {
    fn state_str(&self) -> &'static str {
        match self {
            SagaOutcome::Completed => "completed",
            SagaOutcome::Failed { .. } => "failed",
            SagaOutcome::FailedCompensation { .. } => "failed_compensation",
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            SagaOutcome::Completed => None,
            SagaOutcome::Failed { reason }
            | SagaOutcome::FailedCompensation { reason } => Some(reason.as_str()),
        }
    }
}

/// Serialize a `PipeTarget` for the step's `target` field. snake_case
/// strings so diagnostic output stays greppable.
fn pipe_target_str(t: PipeTarget) -> &'static str {
    match t {
        PipeTarget::LauncherSelf => "launcher_self",
        PipeTarget::Host => "host",
        PipeTarget::Srv => "srv",
    }
}

/// A saga in `running`, `compensating`, or `failed` state.
/// Kept (name included) for the in-process diagnostic surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedLauncherSaga {
    pub saga_id: u64,
    pub name: String,
    pub state: String,
    pub started_at: String,
    pub input_json: String,
    pub failure_reason: Option<String>,
    pub steps: Vec<UnresolvedLauncherStep>,
}

/// A step attached to a saga. Steps are returned in `step_index`
/// ascending order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedLauncherStep {
    pub step_index: u32,
    pub name: String,
    pub state: String,
    pub target: Option<String>,
    pub cmd_json: Option<String>,
    pub output_json: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub failure_reason: Option<String>,
}

/// Operator-facing snapshot of a recent saga. Sorted most-recent-first
/// by `ended_at` falling back to `started_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SagaSummary {
    pub saga_id: u64,
    pub name: String,
    pub state: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub failure_reason: Option<String>,
    /// Count of steps in `succeeded` or `compensated` state — i.e.
    /// progress through the saga.
    pub step_count: u32,
    /// JSON of saga input args, for operator triage.
    pub input_json: String,
}

#[derive(Debug, Clone)]
struct StepRecord {
    step_index: u32,
    name: String,
    state: String, // "pending" | "succeeded" | "failed"
    target: Option<String>,
    cmd_json: Option<String>,
    output_json: Option<String>,
    started_at: String,
    ended_at: Option<String>,
    failure_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct SagaRecord {
    name: String,
    state: String, // "running" | "completed" | "failed" | "failed_compensation"
    started_at: String,
    ended_at: Option<String>,
    failure_reason: Option<String>,
    input_json: String,
    steps: Vec<StepRecord>, // kept sorted by step_index (appended in order)
}

impl SagaRecord {
    fn is_terminal(&self) -> bool {
        matches!(
            self.state.as_str(),
            "completed" | "failed" | "failed_compensation"
        )
    }

    fn recency_key(&self) -> &str {
        self.ended_at.as_deref().unwrap_or(&self.started_at)
    }
}

/// In-memory launcher saga registry. Owned by `SagaCoordinator` as
/// `Arc<LauncherSagaLog>` (the name is kept from the durable era to
/// minimize churn at the ~5 coordinator call sites; it IS still a log —
/// an in-memory, bounded one).
pub struct LauncherSagaLog {
    inner: Mutex<BTreeMap<u64, SagaRecord>>,
}

impl Default for LauncherSagaLog {
    fn default() -> Self {
        Self::new()
    }
}

impl LauncherSagaLog {
    /// Create an empty registry. Infallible — there is no file to open
    /// and no schema to migrate.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    /// Compatibility constructor from the durable era (the registry is
    /// always in-memory now). Kept so the coordinator's tests — written
    /// against the old API — compile unchanged.
    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self, LogError> {
        Ok(Self::new())
    }

    /// Test hook: force a saga into an arbitrary state (e.g.
    /// `compensating`, which no production path writes today). The
    /// durable-era tests did this with raw SQL.
    #[cfg(test)]
    pub(crate) fn set_state_for_test(&self, saga_id: u64, state: &str) {
        if let Some(saga) = self.inner.lock().unwrap().get_mut(&saga_id) {
            saga.state = state.to_string();
        }
    }

    /// Highest existing `saga_id`, or 0 if empty. The coordinator seeds
    /// `next_saga_id` from this; for a fresh in-memory registry it is
    /// always 0, so saga ids are per-process correlation ids (which is
    /// all the host's idempotency LRU and the pipe's drop-failure
    /// paths ever needed them to be).
    pub fn max_saga_id(&self) -> Result<u64, LogError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.keys().next_back().copied().unwrap_or(0))
    }

    /// Insert a fresh saga in `running` state. A duplicate saga_id is
    /// a bug worth surfacing, not a silent overwrite.
    pub fn start_saga(
        &self,
        saga_id: u64,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<(), LogError> {
        let input_json = serde_json::to_string(input)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.contains_key(&saga_id) {
            return Err(LogError::DuplicateSagaId(saga_id));
        }
        inner.insert(
            saga_id,
            SagaRecord {
                name: name.to_string(),
                state: "running".to_string(),
                started_at: now_rfc3339(),
                ended_at: None,
                failure_reason: None,
                input_json,
                steps: Vec::new(),
            },
        );
        Self::enforce_retention(&mut inner);
        Ok(())
    }

    /// Write a saga's terminal state. No-op when the saga_id is
    /// unknown (mirrors the durable log's UPDATE-on-missing-row).
    pub fn terminate_saga(&self, saga_id: u64, outcome: SagaOutcome) -> Result<(), LogError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(saga) = inner.get_mut(&saga_id) {
            saga.state = outcome.state_str().to_string();
            saga.ended_at = Some(now_rfc3339());
            saga.failure_reason = outcome.reason().map(str::to_string);
        }
        Ok(())
    }

    /// Record a `pending` step before its command is dispatched.
    pub fn start_step(
        &self,
        saga_id: u64,
        step_index: u32,
        name: &str,
        target: PipeTarget,
        cmd: &Command,
    ) -> Result<(), LogError> {
        let cmd_json = serde_json::to_string(cmd)?;
        let mut inner = self.inner.lock().unwrap();
        if let Some(saga) = inner.get_mut(&saga_id) {
            saga.steps.push(StepRecord {
                step_index,
                name: name.to_string(),
                state: "pending".to_string(),
                target: Some(pipe_target_str(target).to_string()),
                cmd_json: Some(cmd_json),
                output_json: None,
                started_at: now_rfc3339(),
                ended_at: None,
                failure_reason: None,
            });
        }
        Ok(())
    }

    /// Mark a step `succeeded` and store the awaited event as JSON.
    pub fn finish_step(
        &self,
        saga_id: u64,
        step_index: u32,
        output: &Event,
    ) -> Result<(), LogError> {
        let output_json = serde_json::to_string(output)?;
        let mut inner = self.inner.lock().unwrap();
        if let Some(step) = Self::step_mut(&mut inner, saga_id, step_index) {
            step.state = "succeeded".to_string();
            step.output_json = Some(output_json);
            step.ended_at = Some(now_rfc3339());
        }
        Ok(())
    }

    /// Mark a step `failed` with a reason.
    pub fn fail_step(
        &self,
        saga_id: u64,
        step_index: u32,
        reason: &str,
    ) -> Result<(), LogError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(step) = Self::step_mut(&mut inner, saga_id, step_index) {
            step.state = "failed".to_string();
            step.failure_reason = Some(reason.to_string());
            step.ended_at = Some(now_rfc3339());
        }
        Ok(())
    }

    /// All sagas in `running`, `compensating`, or `failed` state, each
    /// with its full step list, ordered by saga_id ascending. With no
    /// recovery walker this is now purely a diagnostic read.
    pub fn unresolved_sagas(&self) -> Result<Vec<UnresolvedLauncherSaga>, LogError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .iter()
            .filter(|(_, s)| matches!(s.state.as_str(), "running" | "compensating" | "failed"))
            .map(|(id, s)| UnresolvedLauncherSaga {
                saga_id: *id,
                name: s.name.clone(),
                state: s.state.clone(),
                started_at: s.started_at.clone(),
                input_json: s.input_json.clone(),
                failure_reason: s.failure_reason.clone(),
                steps: s.steps.iter().map(step_out).collect(),
            })
            .collect())
    }

    /// Step rows for a single saga regardless of saga state, ordered by
    /// step_index ascending.
    pub fn get_saga_steps(&self, saga_id: u64) -> Result<Vec<UnresolvedLauncherStep>, LogError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .get(&saga_id)
            .map(|s| s.steps.iter().map(step_out).collect())
            .unwrap_or_default())
    }

    /// Mark a saga `failed_compensation`. Idempotent; preserves any
    /// existing failure_reason by APPENDING the new reason (post-mortem
    /// preservation, codex P2 PR #647 round 1). No-op on unknown id.
    /// Retained for API parity + diagnostics even though the recovery
    /// walker that was its sole production caller is gone.
    #[allow(dead_code)]
    pub fn mark_failed_compensation(
        &self,
        saga_id: u64,
        reason: &str,
    ) -> Result<(), LogError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(saga) = inner.get_mut(&saga_id) {
            saga.state = "failed_compensation".to_string();
            saga.ended_at = Some(now_rfc3339());
            saga.failure_reason = match saga.failure_reason.take() {
                Some(existing) if !existing.is_empty() => {
                    Some(format!("{} | recovered: {}", existing, reason))
                }
                _ => Some(reason.to_string()),
            };
        }
        Ok(())
    }

    /// Up to `limit` recent sagas, most-recent-first by
    /// `ended_at ?? started_at`, ties broken by saga_id descending.
    pub fn snapshot_recent(&self, limit: usize) -> Result<Vec<SagaSummary>, LogError> {
        let inner = self.inner.lock().unwrap();
        let mut rows: Vec<(&u64, &SagaRecord)> = inner.iter().collect();
        rows.sort_by(|(id_a, a), (id_b, b)| {
            b.recency_key()
                .cmp(a.recency_key())
                .then(id_b.cmp(id_a))
        });
        Ok(rows
            .into_iter()
            .take(limit)
            .map(|(id, s)| SagaSummary {
                saga_id: *id,
                name: s.name.clone(),
                state: s.state.clone(),
                started_at: s.started_at.clone(),
                ended_at: s.ended_at.clone(),
                failure_reason: s.failure_reason.clone(),
                step_count: s
                    .steps
                    .iter()
                    .filter(|st| matches!(st.state.as_str(), "succeeded" | "compensated"))
                    .count() as u32,
                input_json: s.input_json.clone(),
            })
            .collect())
    }

    /// Evict the oldest TERMINAL sagas beyond the retention cap.
    /// In-flight sagas are never evicted (they'd vanish from
    /// diagnostics mid-run and their terminal write would silently
    /// no-op). Replaces the durable log's `vacuum_older_than`.
    fn enforce_retention(inner: &mut BTreeMap<u64, SagaRecord>) {
        let terminal: Vec<u64> = inner
            .iter()
            .filter(|(_, s)| s.is_terminal())
            .map(|(id, _)| *id)
            .collect();
        if terminal.len() > TERMINAL_RETENTION_CAP {
            // BTreeMap iteration is saga_id ascending == oldest first.
            let excess = terminal.len() - TERMINAL_RETENTION_CAP;
            for id in terminal.into_iter().take(excess) {
                inner.remove(&id);
            }
        }
    }

    fn step_mut<'a>(
        inner: &'a mut BTreeMap<u64, SagaRecord>,
        saga_id: u64,
        step_index: u32,
    ) -> Option<&'a mut StepRecord> {
        inner
            .get_mut(&saga_id)?
            .steps
            .iter_mut()
            .find(|s| s.step_index == step_index)
    }
}

fn step_out(s: &StepRecord) -> UnresolvedLauncherStep {
    UnresolvedLauncherStep {
        step_index: s.step_index,
        name: s.name.clone(),
        state: s.state.clone(),
        target: s.target.clone(),
        cmd_json: s.cmd_json.clone(),
        output_json: s.output_json.clone(),
        started_at: s.started_at.clone(),
        ended_at: s.ended_at.clone(),
        failure_reason: s.failure_reason.clone(),
    }
}

/// RFC3339 timestamp for `started_at` / `ended_at` fields. Single
/// helper so test+production paths agree on format precisely.
fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}
