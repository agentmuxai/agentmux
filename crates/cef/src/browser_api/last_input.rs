// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! When the person last used a browser pane's page with their own hands, for srv's retake rule: an agent that lost a
//! pane to the person's takeover may drive it again once the person has been idle in it for 60 seconds
//! (docs/specs/SPEC_BROWSER_RETAKE_AND_COMMANDEER_2026_11_04.md §1.2). Only input that reaches the page through the
//! window system counts (a click, the wheel on Windows, a key); the agent's own input goes in over CDP and a click
//! on the pane's chrome (address bar, buttons) is the app's, so neither is the person using the page.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;

use super::act::unauthorized;
use super::types::ApiResponse;
use crate::state::AppState;

/// Panes kept before the oldest entries are dropped: an entry only matters for a minute.
const KEPT: usize = 256;
const STALE_AFTER: Duration = Duration::from_secs(3600);

static LAST_INPUT: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// The person just used pane `block_id`'s page.
pub fn note(block_id: &str) {
    note_at(&mut LAST_INPUT.lock(), block_id, Instant::now());
}

fn note_at(map: &mut HashMap<String, Instant>, block_id: &str, now: Instant) {
    if map.len() >= KEPT && !map.contains_key(block_id) {
        map.retain(|_, at| now.saturating_duration_since(*at) < STALE_AFTER);
    }
    map.insert(block_id.to_string(), now);
}

/// Seconds since the person last used the pane's page; `None` when they have not since the host started.
pub fn idle_secs(block_id: &str) -> Option<u64> {
    LAST_INPUT.lock().get(block_id).map(|at| at.elapsed().as_secs())
}

#[derive(Debug, Deserialize)]
pub struct LastInputReq {
    pub block_id: String,
}

/// `POST /agentmux/browser/last_input` — `{ idle_secs }` for a pane: seconds since the person last used its page, or
/// null when they have not.
pub async fn last_input_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<LastInputReq>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    (StatusCode::OK, Json(ApiResponse::ok(json!({ "idle_secs": idle_secs(&req.block_id) }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_never_used_has_no_idle_time_and_a_used_one_has() {
        let pane = uuid::Uuid::new_v4().to_string();
        assert_eq!(idle_secs(&pane), None);
        note(&pane);
        assert_eq!(idle_secs(&pane), Some(0));
    }

    #[test]
    fn old_entries_go_when_the_map_is_full_and_the_new_pane_stays() {
        let now = Instant::now();
        let Some(old) = now.checked_sub(STALE_AFTER + Duration::from_secs(1)) else { return };
        let mut map: HashMap<String, Instant> = (0..KEPT).map(|i| (format!("old-{i}"), old)).collect();
        note_at(&mut map, "fresh", now);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("fresh"));
    }
}
