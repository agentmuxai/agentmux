// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/message-flow`: the last messages this srv handled between agents, oldest first, as ids and times
//! only (`backend::message_flow`). In memory, so empty after a restart. Read-only.

use axum::response::{IntoResponse, Json};
use serde_json::json;

pub async fn handle_message_flow() -> impl IntoResponse {
    Json(json!({ "messages": crate::backend::message_flow::recent() }))
}
