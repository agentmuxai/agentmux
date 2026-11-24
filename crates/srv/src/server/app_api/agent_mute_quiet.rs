// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents mute_quiet` (`POST /api/v1/agent/mute-quiet`): a leader tells the quiet watch that one of its agents will
//! be quiet for a stated time ("building, back in 40 minutes"), so the watch neither nudges it nor tells anyone until
//! that time is over (docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §5). Only the agent's lead
//! may (the lead of its seat, which for a spawned worker is the agent that spawned it), never the agent itself (its
//! own way is `Presence status` with state `waiting` and `until_minutes`), and never without an agent token: the mute
//! is recorded as the agent that asked, in the audit log, never as the operator. Zero minutes lifts a mute.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::agent_lifecycle::{actor_of, audit, refuse, target_of, Refusal};
use crate::backend::quiet_watch;
use crate::server::caller::Caller;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct MuteRequest {
    /// The agent's id, slug or name.
    pub(super) agent: String,
    /// How long, 1 to 240; 0 lifts the mute.
    pub(super) minutes: u64,
    /// Why, in a few words; shown with the mute.
    #[serde(default)]
    pub(super) reason: String,
}

async fn run(state: &AppState, caller: Option<&Caller>, req: MuteRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller, "mute_quiet")?;
    let blocking = state.clone();
    let named = req.agent.clone();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents mute_quiet: {e}")))??;
    let block_id = target.block_id.clone().unwrap_or_default();
    if target.agent_id == actor.uid() {
        return Err(refuse(StatusCode::CONFLICT, "SELF_MUTE: that is you; Presence status with state waiting and until_minutes says you will be quiet"));
    }
    if !quiet_watch::leads(&state.mstore, actor.uid(), &target.agent_id) {
        let why = format!("NOT_ITS_LEAD: only {}'s lead (its seat's lead, or the agent that spawned it) can mute it", target.name);
        audit(state, &actor, "mute_quiet", &target.name, &block_id, false, Some(&why), "refused");
        return Err(refuse(StatusCode::FORBIDDEN, why));
    }
    let now = agentmux_common::time::now_ms_u64();
    if req.minutes == 0 {
        let lifted = quiet_watch::unmute(&target.agent_id, now);
        audit(state, &actor, "mute_quiet", &target.name, &block_id, true, None, "lifted");
        return Ok(json!({ "agent": target.name, "agent_id": target.agent_id, "muted": false, "lifted": lifted }));
    }
    let muted = quiet_watch::mute(&target.agent_id, req.minutes, &req.reason, actor.slug(), now).map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    audit(state, &actor, "mute_quiet", &target.name, &block_id, true, None, &format!("{} minutes: {}", req.minutes, muted.reason));
    Ok(json!({
        "agent": target.name, "agent_id": target.agent_id, "muted": true, "until_ms": muted.until_ms, "reason": muted.reason,
        "note": "the quiet watch neither nudges it nor tells anyone until then; it starts again from the end of the mute. minutes: 0 lifts it",
    }))
}

pub(crate) async fn handle_agent_mute_quiet(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<MuteRequest>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

#[cfg(test)]
#[path = "tests/agent_mute_quiet_tests.rs"]
mod tests;
