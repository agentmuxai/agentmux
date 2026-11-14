// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/pr-flow`: the last reviews, check results and merges of agents' PRs this srv was told of,
//! oldest first, as ids, states and times only (`backend::pr_flow`), and when the newest notice came. In
//! memory, so empty after a restart. Read-only.

use axum::response::{IntoResponse, Json};

pub async fn handle_pr_flow() -> impl IntoResponse {
    Json(crate::backend::pr_flow::recent())
}
