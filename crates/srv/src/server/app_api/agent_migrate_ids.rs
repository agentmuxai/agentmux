// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents migrate_ids` (`POST /api/v1/agent/migrate-ids`): an agent runs the agent id update for
//! the person, the same steps Settings › Agents › Outdated agent ids runs
//! (`agents.migrateForkSlugs`, `agent_handlers/fork_slugs.rs`): `plan` reads and changes nothing,
//! `stop` asks every pane running an affected agent to stop (each with its user's 30 s override,
//! and the answer says how long to wait), `apply` migrates, `rollback` undoes a run from its
//! journal (always the latest finished run: an agent names no journal file). The caller is the agent its token names; a request without one is refused, so a change
//! is never attributed to the operator. Every step but `plan` is recorded in the audit log under
//! the calling agent's slug.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::backend::rpc_types::{CommandMigrateForkSlugsData, ForkSlugChoice, ForkSlugMigrationMode};
use crate::server::agent_handlers::fork_slugs;
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The audit entry's action for a step.
pub(crate) const AUDIT_ACTION: &str = "agent.ids.update";

#[derive(Debug, Deserialize)]
pub(crate) struct MigrateIdsRequest {
    /// `plan`, `stop`, `apply` or `rollback`.
    step: String,
    /// Not accepted: opening a journal repairs it in place and its undo steps rename and restore the paths it
    /// names, so a path from an agent would reach any file. A rollback undoes the latest finished run.
    #[serde(default)]
    journal: Option<String>,
    /// The id to give an agent whose computed id is taken: `[{uid, new_slug}]`.
    #[serde(default)]
    choices: Option<Vec<ForkSlugChoice>>,
}

/// What the migration does for `step`, if it is one an agent may ask for.
pub(crate) fn mode_of(step: &str) -> Option<ForkSlugMigrationMode> {
    match step {
        "plan" => Some(ForkSlugMigrationMode::Plan),
        "stop" => Some(ForkSlugMigrationMode::Stop),
        "apply" => Some(ForkSlugMigrationMode::Apply),
        "rollback" => Some(ForkSlugMigrationMode::Rollback),
        _ => None,
    }
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn audit(state: &AppState, uid: &str, slug: &str, step: &str, error: Option<&str>) {
    let block_id = state.reactive_handler.block_for_uid(uid).unwrap_or_default();
    let reason = format!("{slug} (uid {uid}): agent id update, step {step}");
    state.reactive_handler.log_fleet_action_audit(
        Some(slug),
        "all agents",
        &block_id,
        AUDIT_ACTION,
        error.is_none(),
        error,
        &uuid::Uuid::new_v4().to_string(),
        Some(&reason),
    );
}

pub(crate) async fn handle_agent_migrate_ids(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<MigrateIdsRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    let Some(uid) = caller.as_ref().and_then(Caller::uid) else {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "no agent token: Agents migrate_ids acts as the calling agent; the operator runs the update in Settings › Agents",
        );
    };
    if req.journal.is_some() {
        return refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: journal is not accepted from an agent: rollback undoes the latest finished run");
    }
    let step = req.step.trim().to_string();
    let Some(mode) = mode_of(&step) else {
        return refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: step is plan, stop, apply or rollback");
    };
    let def = match state.mstore.agent_def_get(uid) {
        Ok(Some(def)) => def,
        Ok(None) => return refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"),
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent id update: {e}")),
    };
    let cmd = CommandMigrateForkSlugsData { mode, journal: None, quick: None, choices: req.choices };
    let stopped_by = format!("{} (Agents migrate_ids)", def.slug);
    let result = fork_slugs::run(&state, cmd, &stopped_by).await;
    if mode != ForkSlugMigrationMode::Plan {
        audit(&state, uid, &def.slug, &step, result.as_ref().err().map(String::as_str));
    }
    match result {
        Ok(r) => Json(r).into_response(),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[cfg(test)]
#[path = "tests/agent_migrate_ids_tests.rs"]
mod tests;
