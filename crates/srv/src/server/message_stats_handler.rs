// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/message-stats?days=N`: what messages cost per agent per day (`Agents message_stats`), counted
//! from the jekt audit file. Read-only.

use axum::extract::Query;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;

use crate::backend::audit_query::message_stats;

#[derive(Debug, Default, Deserialize)]
pub struct StatsParams {
    pub days: Option<u64>,
}

pub async fn handle_message_stats(Query(p): Query<StatsParams>) -> impl IntoResponse {
    let days = p.days.unwrap_or(7);
    let stats = tokio::task::spawn_blocking(move || {
        let paths: Vec<_> = [
            crate::backend::reactive::audit_store::shared_path_for_reading(),
            crate::backend::reactive::audit_store::own_path(),
        ]
        .into_iter()
        .flatten()
        .collect();
        message_stats::from_files(&paths, agentmux_common::time::now_ms() as u64, days)
    })
    .await
    .unwrap_or_default();
    Json(json!({ "days": days.clamp(1, message_stats::MAX_DAYS), "stats": stats }))
}
