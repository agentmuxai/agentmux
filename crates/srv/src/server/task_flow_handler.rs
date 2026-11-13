// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/task-flow`: the last task filings, moves, starts and hand-backs this srv saw, oldest first, as names,
//! numbers and times only (`backend::task_flow`). A caller that is an agent gets the boards `Tasks list` shows it and no
//! others; a caller with no token (the app's own windows) gets every board. In memory, so empty after a restart.

use std::collections::HashSet;

use axum::extract::State;
use axum::response::{IntoResponse, Json};
use serde_json::json;

use super::AppState;

pub async fn handle_task_flow(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>) -> impl IntoResponse {
    let who = super::work_tasks::viewer(&state, caller.as_deref());
    let boards = state.work_store.work_boards_list(true, agentmux_common::time::now_ms()).unwrap_or_default();
    let visible: HashSet<String> = boards.into_iter().filter(|b| who.as_ref().map_or(true, |(agent, uid)| b.visible_to(agent, uid))).map(|b| b.id).collect();
    Json(json!({ "events": crate::backend::task_flow::recent_where(|board_id| who.is_none() || visible.contains(board_id)) }))
}
