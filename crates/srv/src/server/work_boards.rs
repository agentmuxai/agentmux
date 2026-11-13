// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Several Kanban boards on the MuxAPI (docs/specs/SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md
//! §9.2, §9.3), beside the queue's own routes in `work_queue.rs`:
//!
//!   GET  /agentmux/work/boards   — the boards the caller sees, with how many tasks each holds
//!   POST /agentmux/work/boards   — { action: create | edit | archive | unarchive | delete |
//!                                    cancel_delete | default, board, name, slug, readable, agents }
//!
//! Moving a task to another board is `Tasks move` with a `board` (`/agentmux/work/act`).
//! The same moves exist for the operator in the Kanban; an agent makes every one of them too, and the
//! board's history carries the agent. A board whose agent list leaves the caller out is refused like a
//! board that doesn't exist.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use agentmux_common::time::now_ms;

use crate::backend::storage::work_boards::{Board, BoardEdit, Deletion, DELETE_COUNTDOWN_MS};

use super::AppState;

#[derive(Debug, Deserialize, Default)]
pub(super) struct BoardsQuery {
    /// Include the archived boards.
    #[serde(default)]
    pub archived: bool,
}

/// What a board action takes. `board` names the board it acts on (name, slug or id).
#[derive(Debug, Deserialize, Default)]
pub(crate) struct BoardsRequest {
    pub action: String,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub readable: Option<bool>,
    /// The agents that see the board (ids); an empty list means all agents.
    #[serde(default)]
    pub agents: Option<Vec<String>>,
}

pub(crate) fn board_json(b: &Board) -> Value {
    json!({
        "id": b.id, "slug": b.slug, "name": b.name, "tier": b.tier, "readable": b.readable,
        "agents": b.agents, "default": b.is_default, "archived": b.archived,
        "open": b.open, "total": b.total, "pending_delete_at": b.pending_delete_at,
    })
}

/// The boards `who` sees (none: all), default first.
pub(crate) fn boards_view(state: &AppState, who: Option<(&str, &str)>, include_archived: bool) -> Result<Value, String> {
    let boards = state.work_store.work_boards_list(include_archived, now_ms()).map_err(|e| e.to_string())?;
    let seen: Vec<Value> = boards.iter().filter(|b| who.map_or(true, |(a, u)| b.visible_to(a, u))).map(board_json).collect();
    let default = boards.iter().find(|b| b.is_default).map(|b| b.slug.clone());
    Ok(json!({ "boards": seen, "default": default }))
}

/// One board action by `actor`. `who` is the caller's id and UID when it is an agent (then it must see
/// the board it acts on), none for the operator.
pub(crate) fn board_act(state: &AppState, who: Option<(&str, &str)>, actor: &str, req: BoardsRequest) -> Result<Value, String> {
    let store = &state.work_store;
    let now = now_ms();
    let on = |key: &Option<String>| -> Result<Board, String> {
        let key = key.as_deref().map(str::trim).filter(|k| !k.is_empty()).ok_or("which board? name it with board")?;
        super::work_tasks::pick_board(state, Some(key), who, false)
    };
    let edit = BoardEdit {
        name: req.name.clone(),
        slug: req.slug.clone(),
        tier: req.tier.clone(),
        readable: req.readable,
        agents: req.agents.clone(),
    };
    let out = match req.action.as_str() {
        "create" => {
            let b = store.work_board_create(&edit, actor, now).map_err(|e| e.to_string())?;
            json!({ "ok": true, "board": board_json(&b) })
        }
        "edit" => {
            let b = on(&req.board)?;
            let b = store.work_board_edit(&b.id, &edit, now).map_err(|e| e.to_string())?;
            json!({ "ok": true, "board": board_json(&b) })
        }
        "archive" | "unarchive" => {
            let b = on(&req.board)?;
            let b = store.work_board_archive(&b.id, req.action == "archive", now).map_err(|e| e.to_string())?;
            json!({ "ok": true, "board": board_json(&b) })
        }
        "default" => {
            let b = on(&req.board)?;
            let b = store.work_board_set_default(&b.id, now).map_err(|e| e.to_string())?;
            json!({ "ok": true, "board": board_json(&b) })
        }
        "delete" => {
            let b = on(&req.board)?;
            match store.work_board_delete(&b.id, now).map_err(|e| e.to_string())? {
                Deletion::Deleted => json!({ "ok": true, "deleted": true }),
                Deletion::Pending(at) => json!({
                    "ok": true,
                    "deleted": false,
                    "pending_until": at,
                    "note": format!(
                        "board {} and its {} task(s) will be deleted in {} seconds; the Kanban shows a countdown with a veto. Cancel with board_cancel_delete",
                        b.slug, b.total, DELETE_COUNTDOWN_MS / 1000
                    ),
                }),
            }
        }
        "cancel_delete" => {
            let b = on(&req.board)?;
            let b = store.work_board_cancel_delete(&b.id, now).map_err(|e| e.to_string())?;
            json!({ "ok": true, "board": board_json(&b) })
        }
        other => return Err(format!("unknown board action {other:?}: create, edit, archive, unarchive, delete, cancel_delete or default")),
    };
    tracing::info!(target: "workqueue", actor, action = %req.action, board = ?req.board.as_deref().or(req.name.as_deref()), "board changed");
    super::work_queue::publish_changed(state);
    Ok(out)
}

pub(super) async fn handle_boards_list(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Query(q): Query<BoardsQuery>,
) -> (StatusCode, Json<Value>) {
    let who = super::work_tasks::viewer(&state, caller.as_deref());
    let who = who.as_ref().map(|(a, u)| (a.as_str(), u.as_str()));
    match boards_view(&state, who, q.archived) {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

pub(super) async fn handle_boards_act(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<BoardsRequest>,
) -> (StatusCode, Json<Value>) {
    let who = super::work_tasks::viewer(&state, caller.as_deref());
    let actor = who.as_ref().map(|(a, _)| a.clone()).unwrap_or_else(|| "agent".to_string());
    let who = who.as_ref().map(|(a, u)| (a.as_str(), u.as_str()));
    match board_act(&state, who, &actor, req) {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": e }))),
    }
}

#[cfg(test)]
#[path = "work_boards_tests.rs"]
mod tests;
