// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a sender may know before sending: the target's availability and what a routine message
//! from a host-tier sender would do now (`docs/specs/SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md`
//! §3.3). It reads the same fact and asks the same `verdict` the jekt path applies, and delivers,
//! reserves and logs nothing, so it is safe to call for every agent in a list.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::backend::agent_availability::{availability, of_unavailable, verdict, Availability, Verdict, WaitReason};
use crate::backend::agent_state::{current_state_at, unavailable_of_block, AgentState};
use crate::backend::reactive::handler::LookupOutcome;
use crate::backend::storage::store::Store;

use super::AppState;

/// The wire name of a state: `AgentState`'s, with a defined agent that has no process as `stopped`.
fn state_name(a: &Availability) -> &'static str {
    match a {
        Availability::NotRunning { exited_with_error: true } => AgentState::Error.as_str(),
        Availability::NotRunning { exited_with_error: false } => AgentState::Stopped.as_str(),
        Availability::Idle { .. } => AgentState::Idle.as_str(),
        Availability::Running { waiting_on_user: true } => AgentState::Waiting.as_str(),
        Availability::Running { waiting_on_user: false } => AgentState::Working.as_str(),
        Availability::Limited { .. } => AgentState::Limited.as_str(),
        Availability::SignedOut { .. } => AgentState::SignedOut.as_str(),
        Availability::Unknown => "unknown",
    }
}

/// The answer for one agent: its `state`, when it began (`since_ms`) and, for a limit, when it lifts
/// (`resets_at_ms`); `would`, what a routine message does now (`deliver` or `hold`); and, when the agent
/// can't work, `reason` and `retry_when`. A limited agent with no known reset still gets `would: deliver`
/// (a turn is how it finds out), with `reason: usage_limit` so the sender knows it may fail.
pub(super) fn describe(av: &Availability, v: &Verdict) -> Value {
    let mut out = json!({ "state": state_name(av), "would": if v.waits() { "hold" } else { "deliver" } });
    match *av {
        Availability::Limited { resets_at_ms, since_ms } => {
            out["since_ms"] = json!(since_ms);
            out["resets_at_ms"] = json!(resets_at_ms);
        }
        Availability::SignedOut { since_ms } => out["since_ms"] = json!(since_ms),
        Availability::Idle { quiet_for_ms: Some(quiet) } => out["quiet_for_ms"] = json!(quiet),
        _ => {}
    }
    let reason = match (*v, *av) {
        (Verdict::Wait { reason, .. }, _) => Some(reason),
        (_, Availability::Limited { .. }) => Some(WaitReason::UsageLimit),
        _ => None,
    };
    if let Some(reason) = reason {
        out["reason"] = json!(reason.as_str());
        out["retry_when"] = json!(reason.retry_when());
    }
    if let Verdict::Wait { until_ms: Some(until), .. } = *v {
        out["until_ms"] = json!(until);
    }
    out
}

/// The block the agent with this UID runs in, when it is registered here under that UID.
pub(super) fn block_of_uid(handler: &crate::backend::reactive::handler::ReactiveHandler, uid: &str) -> Option<String> {
    match handler.lookup_delivery_target(uid) {
        LookupOutcome::One(reg) => Some(reg.block_id),
        LookupOutcome::Ambiguous(_) | LookupOutcome::NotFound => None,
    }
}

/// [`describe`] for the agent on `block_id`, or for a defined agent with no live pane (`None`).
pub(super) fn describe_block(mstore: &Store, block_id: Option<&str>, cold_after_ms: Option<u64>, now_ms: u64) -> Value {
    let av = match block_id {
        None => Availability::NotRunning { exited_with_error: false },
        Some(block_id) => match unavailable_of_block(mstore, None, block_id, now_ms)
            .or_else(|| super::limit_probe::unavailable_while_probing(mstore, block_id, now_ms))
        {
            Some(unavailable) => of_unavailable(&unavailable),
            None => {
                let state = current_state_at(mstore, block_id, now_ms);
                let quiet = (state == Some(AgentState::Idle))
                    .then(|| crate::backend::cold_hold::session_last_output(mstore, block_id))
                    .flatten()
                    .map(|at| now_ms.saturating_sub(u64::try_from(at).unwrap_or(0)));
                availability(state, None, quiet)
            }
        },
    };
    describe(&av, &verdict(&av, false, true, cold_after_ms, || false))
}

#[derive(serde::Deserialize)]
pub(super) struct AvailabilityQuery {
    agent: Option<String>,
}

/// `GET /agentmux/reactive/availability?agent=<name|uid>`: [`describe_block`] for one agent of this
/// instance. 400 without `agent`, 404 for a name no agent here has, 409 for one that names several.
pub(super) async fn handle_availability(State(state): State<AppState>, Query(q): Query<AvailabilityQuery>) -> Response {
    let Some(agent) = q.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing agent param" }))).into_response();
    };
    let route = crate::backend::reactive::sanitize::route_name(agent).unwrap_or_else(|| agent.to_string());
    let block_id = match state.reactive_handler.lookup_delivery_target(&route) {
        LookupOutcome::One(reg) => Some(reg.block_id),
        LookupOutcome::Ambiguous(_) => {
            return (StatusCode::CONFLICT, Json(json!({ "error": format!("{agent} names more than one agent here") }))).into_response();
        }
        LookupOutcome::NotFound => None,
    };
    let (mstore, agent_owned) = (state.mstore.clone(), agent.to_string());
    let after_ms = crate::backend::cold_hold::cold_after_ms(&state.config_watcher.get_full_config().settings.extra)
        .and_then(|ms| u64::try_from(ms).ok());
    let now_ms = agentmux_common::time::now_ms_u64();
    let known = tokio::task::spawn_blocking(move || {
        if block_id.is_some() {
            return Known::Live(describe_block(&mstore, block_id.as_deref(), after_ms, now_ms));
        }
        let defined = super::reactive::defined_here(&mstore, &agent_owned).is_some();
        let matches = mstore.agents_matching_name(&agent_owned).map_or(0, |rows| rows.len());
        match classify_stopped(defined, matches) {
            Stopped::Defined => Known::Live(describe_block(&mstore, None, after_ms, now_ms)),
            Stopped::Ambiguous => Known::Ambiguous,
            Stopped::Unknown => Known::Unknown,
        }
    })
    .await
    .unwrap_or(Known::Unknown);
    match known {
        Known::Live(view) => Json(view).into_response(),
        Known::Ambiguous => (StatusCode::CONFLICT, Json(json!({ "error": format!("{agent} names more than one agent here") }))).into_response(),
        Known::Unknown => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no agent named {agent} here") }))).into_response(),
    }
}

/// How a name that no live pane answers to stands in the directory.
#[derive(Debug, PartialEq, Eq)]
enum Stopped {
    /// Exactly one defined agent: it has no process.
    Defined,
    /// Several defined agents answer to it.
    Ambiguous,
    Unknown,
}

fn classify_stopped(defined_here: bool, matching: usize) -> Stopped {
    match (defined_here, matching) {
        (true, _) => Stopped::Defined,
        (false, 0) => Stopped::Unknown,
        (false, _) => Stopped::Ambiguous,
    }
}

enum Known {
    Live(Value),
    Ambiguous,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER: u64 = 55 * 60_000;

    fn view(av: Availability) -> Value {
        describe(&av, &verdict(&av, false, true, Some(AFTER), || false))
    }

    #[test]
    fn an_agent_that_can_work_is_delivered_to_with_no_reason() {
        let v = view(Availability::Running { waiting_on_user: false });
        assert_eq!(v, json!({ "state": "working", "would": "deliver" }));
        let v = view(Availability::Idle { quiet_for_ms: Some(60_000) });
        assert_eq!(v, json!({ "state": "idle", "would": "deliver", "quiet_for_ms": 60_000 }));
    }

    #[test]
    fn a_limit_with_a_reset_holds_and_says_until_when() {
        let v = view(Availability::Limited { resets_at_ms: Some(9_000), since_ms: 7 });
        assert_eq!(v["state"], "limited");
        assert_eq!(v["would"], "hold");
        assert_eq!(v["reason"], "usage_limit");
        assert_eq!(v["retry_when"], "reset_or_account_move");
        assert_eq!((v["until_ms"].as_u64(), v["resets_at_ms"].as_u64(), v["since_ms"].as_u64()), (Some(9_000), Some(9_000), Some(7)));
    }

    #[test]
    fn a_limit_with_no_known_reset_delivers_but_says_it_may_fail() {
        let v = view(Availability::Limited { resets_at_ms: None, since_ms: 7 });
        assert_eq!(v["would"], "deliver");
        assert_eq!(v["reason"], "usage_limit");
        assert!(v["resets_at_ms"].is_null() && v.get("until_ms").is_none());
    }

    #[test]
    fn signed_out_and_not_running_hold_with_their_own_advice() {
        let v = view(Availability::SignedOut { since_ms: 3 });
        assert_eq!((v["would"].as_str(), v["reason"].as_str(), v["retry_when"].as_str()), (Some("hold"), Some("needs_login"), Some("sign_in")));
        let v = view(Availability::NotRunning { exited_with_error: true });
        assert_eq!((v["state"].as_str(), v["reason"].as_str(), v["retry_when"].as_str()), (Some("error"), Some("not_running"), Some("start")));
    }

    #[test]
    fn a_cold_session_holds_a_routine_message_and_is_still_idle_on_the_wire() {
        let v = view(Availability::Idle { quiet_for_ms: Some(AFTER + 1) });
        assert_eq!((v["state"].as_str(), v["would"].as_str(), v["reason"].as_str()), (Some("idle"), Some("hold"), Some("idle")));
        assert_eq!(v["retry_when"], "session_active");
    }

    /// Two agents with one display name are told apart by UID, which is what discovery asks.
    #[test]
    fn agents_sharing_a_display_name_resolve_to_their_own_blocks_by_uid() {
        use crate::backend::reactive::handler::ReactiveHandler;
        let h = ReactiveHandler::new();
        h.register_agent_full("Twin", "block-a", None, 0, None, Some("uid-a"), "test").unwrap();
        h.register_agent_full("Twin", "block-b", None, 0, None, Some("uid-b"), "test").unwrap();
        assert_eq!((block_of_uid(&h, "uid-a").as_deref(), block_of_uid(&h, "uid-b").as_deref()), (Some("block-a"), Some("block-b")));
        assert_eq!(block_of_uid(&h, "uid-none"), None);
    }

    #[test]
    fn a_name_no_pane_answers_to_is_defined_ambiguous_or_unknown() {
        assert_eq!(classify_stopped(true, 1), Stopped::Defined);
        assert_eq!(classify_stopped(false, 2), Stopped::Ambiguous);
        assert_eq!(classify_stopped(false, 0), Stopped::Unknown);
    }

    #[test]
    fn a_defined_agent_with_no_pane_is_stopped_and_holds() {
        let store = Store::open_in_memory().unwrap();
        let v = describe_block(&store, None, Some(AFTER), 1_000);
        assert_eq!((v["state"].as_str(), v["would"].as_str(), v["reason"].as_str()), (Some("stopped"), Some("hold"), Some("not_running")));
    }
}
