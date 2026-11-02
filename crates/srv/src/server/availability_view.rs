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
use crate::backend::agent_status::{decide, AgentStatus, Decision, Intent, Pane, Reach, Wanted};
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

/// What can be said about the agent on `block_id` now, or about a defined agent with no live pane (`None`).
pub(super) fn availability_of_block(mstore: &Store, block_id: Option<&str>, now_ms: u64) -> Availability {
    match block_id {
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
    }
}

/// The model API of a block's agent as a pane reads it: tracked and answering, offline, not checked yet, or not tracked.
fn reach_of(meta: &std::collections::HashMap<String, Value>, net: &crate::connectivity::ConnectivityStatus, now_ms: u64) -> Reach {
    use crate::connectivity::Reachability;
    let Some(id) = super::held_input::service_of(meta) else { return Reach::NotTracked };
    let Some(service) = net.services.iter().find(|s| s.service == id) else { return Reach::NotTracked };
    match service.state {
        Reachability::Online => Reach::Online,
        Reachability::Degraded => Reach::Degraded,
        Reachability::Offline => Reach::Offline,
        Reachability::Unknown => Reach::Unchecked { for_ms: now_ms.saturating_sub(u64::try_from(service.since_ms).unwrap_or(0)) },
    }
}

/// Everything a path needs to know about an agent: its availability (`live`, the block it is registered
/// in, if any) and, from `block` (its live block, or the one that holds its desired state), whether it
/// has a pane, whether it is wanted running, and whether its model API answers (`net`: one snapshot of
/// the connectivity status, so a list of agents takes it once). Writes nothing.
pub(super) fn status_of(
    mstore: &Store,
    live: Option<&str>,
    block: Option<&crate::backend::obj::Block>,
    net: &crate::connectivity::ConnectivityStatus,
    now_ms: u64,
) -> AgentStatus {
    let (pane, wanted, api, controller) = match block {
        None => (Pane::None, Wanted::Unset, Reach::NotTracked, false),
        Some(block) => {
            let parked = crate::backend::obj::meta_get_bool(&block.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false);
            let wanted = match super::app_api::agent_attach::desired_of(block).as_deref() {
                Some(super::app_api::agent_attach::DESIRED_RUNNING) => Wanted::Running,
                Some(super::app_api::agent_attach::DESIRED_STOPPED) => Wanted::Stopped,
                _ => Wanted::Unset,
            };
            let controller = crate::backend::blockcontroller::get_controller(&block.oid).is_some();
            (if parked { Pane::Parked } else { Pane::Shown }, wanted, reach_of(&block.meta, net, now_ms), controller)
        }
    };
    AgentStatus { availability: availability_of_block(mstore, live, now_ms), pane, wanted, api, controller }
}

/// [`describe`] for a status: what a routine message does now is `decide`'s answer (so an offline model
/// API reads as a hold), and the status's `pane`, `wanted` and `api` are named, so a sender sees "parked,
/// running" or "shut down" before it sends.
pub(super) fn describe_status(status: &AgentStatus, cold_after_ms: Option<u64>) -> Value {
    let message = Intent::Message { urgent: false, routine: true, cold_after_ms };
    let verdict = match decide(status, message, || false) {
        Decision::Wait { reason, until_ms } => Verdict::Wait { reason, until_ms },
        _ => Verdict::Deliver,
    };
    let mut out = describe(&status.availability, &verdict);
    out["pane"] = json!(status.pane.as_str());
    out["wanted"] = json!(status.wanted.as_str());
    out["api"] = json!(status.api.as_str());
    out
}

/// [`describe_status`] for the agent registered in `live` (`None` for a defined agent with no live pane).
/// `state_block` is the block that holds its pane and desired state when that is not `live`: the parked
/// block of an agent with no process.
pub(super) fn describe_block(
    mstore: &Store,
    live: Option<&str>,
    state_block: Option<&str>,
    net: &crate::connectivity::ConnectivityStatus,
    cold_after_ms: Option<u64>,
    now_ms: u64,
) -> Value {
    let block = live.or(state_block).and_then(|id| mstore.get::<crate::backend::obj::Block>(id).ok().flatten());
    describe_status(&status_of(mstore, live, block.as_ref(), net, now_ms), cold_after_ms)
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
    match view_of_agent(&state, agent).await {
        Known::Live(view) => Json(view).into_response(),
        Known::Ambiguous => (StatusCode::CONFLICT, Json(json!({ "error": format!("{agent} names more than one agent here") }))).into_response(),
        Known::Unknown => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no agent named {agent} here") }))).into_response(),
    }
}

/// [`describe_block`] for the agent `agent` (a name or a UID) names on this instance.
pub(super) async fn view_of_agent(state: &AppState, agent: &str) -> Known {
    let route = crate::backend::reactive::sanitize::route_name(agent).unwrap_or_else(|| agent.to_string());
    let block_id = match state.reactive_handler.lookup_delivery_target(&route) {
        LookupOutcome::One(reg) => Some(reg.block_id),
        LookupOutcome::Ambiguous(_) => return Known::Ambiguous,
        LookupOutcome::NotFound => None,
    };
    let (mstore, agent_owned) = (state.mstore.clone(), agent.to_string());
    let after_ms = crate::backend::cold_hold::cold_after_ms(&state.config_watcher.get_full_config().settings.extra)
        .and_then(|ms| u64::try_from(ms).ok());
    let now_ms = agentmux_common::time::now_ms_u64();
    let net = crate::connectivity::status();
    tokio::task::spawn_blocking(move || {
        if block_id.is_some() {
            return Known::Live(describe_block(&mstore, block_id.as_deref(), None, &net, after_ms, now_ms));
        }
        let defined = super::reactive::defined_here(&mstore, &agent_owned).is_some();
        let matches = mstore.agents_matching_name(&agent_owned).map_or(0, |rows| rows.len());
        match classify_stopped(defined, matches) {
            Stopped::Defined => Known::Live(describe_block(&mstore, None, None, &net, after_ms, now_ms)),
            Stopped::Ambiguous => Known::Ambiguous,
            Stopped::Unknown => Known::Unknown,
        }
    })
    .await
    .unwrap_or(Known::Unknown)
}

/// What to tell a sender who filed a task for the agent `lookup` (a name or a UID) that the agent can't
/// take up now: its availability (`target`) and a `note` saying the task waits, naming the agent `name`
/// as the sender typed it (`lookup` when that is empty). `None` for an agent that can work, a group or
/// unaddressed task, or a name no agent here has, so the answer stays as it was.
pub(super) async fn enqueue_note(state: &AppState, lookup: &str, name: &str) -> Option<Value> {
    let lookup = lookup.trim();
    if lookup.is_empty() {
        return None;
    }
    let Known::Live(view) = view_of_agent(state, lookup).await else {
        return None;
    };
    let name = Some(name.trim()).filter(|n| !n.is_empty()).unwrap_or(lookup);
    let note = wait_note(&view, name)?;
    Some(json!({ "target": view, "note": note }))
}

/// `body` with the fields of [`enqueue_note`], if there is one.
pub(super) fn with_note(mut body: Value, note: Option<Value>) -> Value {
    if let (Some(fields), Some(map)) = (note, body.as_object_mut()) {
        if let Some(extra) = fields.as_object() {
            map.extend(extra.clone());
        }
    }
    body
}

/// The sentence for an agent in `view` that can't work: `limited` or `signed_out`. It says the task is
/// filed and waits for the agent, and when that is expected.
fn wait_note(view: &Value, agent: &str) -> Option<String> {
    let when = match view["state"].as_str()? {
        "limited" => match view["resets_at_ms"].as_u64() {
            Some(at) => format!("at the limit, which lifts at {}", super::jekt_hold::local_time_label(at).unwrap_or_default()),
            None => "at its usage limit (no reset time known)".to_string(),
        },
        "signed_out" => "signed out".to_string(),
        _ => return None,
    };
    Some(format!(
        "{agent} is {when}. The task is filed and stays in its queue; {agent} can take it only once it can work again, \
         so do not expect it sooner."
    ))
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

pub(super) enum Known {
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
    fn a_task_for_an_agent_that_cannot_work_says_it_waits_and_for_one_that_can_says_nothing() {
        let limited = view(Availability::Limited { resets_at_ms: Some(9_000), since_ms: 7 });
        let note = wait_note(&limited, "ann").unwrap();
        assert!(note.starts_with("ann is at the limit, which lifts at "), "{note}");
        assert!(note.contains("stays in its queue"), "{note}");
        let no_reset = view(Availability::Limited { resets_at_ms: None, since_ms: 7 });
        assert!(wait_note(&no_reset, "ann").unwrap().contains("no reset time known"));
        let out = view(Availability::SignedOut { since_ms: 7 });
        assert!(wait_note(&out, "ann").unwrap().starts_with("ann is signed out."));
        for av in [Availability::Idle { quiet_for_ms: None }, Availability::Running { waiting_on_user: false }, Availability::NotRunning { exited_with_error: false }] {
            assert_eq!(wait_note(&view(av), "ann"), None);
        }
    }

    #[test]
    fn the_note_joins_the_enqueue_answer_only_when_there_is_one() {
        let body = json!({ "id": "w1", "num": 4 });
        assert_eq!(with_note(body.clone(), None), body);
        let joined = with_note(body, Some(json!({ "target": { "state": "limited" }, "note": "n" })));
        assert_eq!((joined["num"].clone(), joined["note"].clone(), joined["target"]["state"].clone()), (json!(4), json!("n"), json!("limited")));
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
        let v = describe_block(&store, None, None, &crate::connectivity::ConnectivityStatus { revision: 0, services: vec![] }, Some(AFTER), 1_000);
        assert_eq!((v["state"].as_str(), v["would"].as_str(), v["reason"].as_str()), (Some("stopped"), Some("hold"), Some("not_running")));
        assert_eq!((v["pane"].as_str(), v["wanted"].as_str(), v["api"].as_str()), (Some("none"), Some("unset"), Some("not_tracked")));
    }

    fn status(availability: Availability, pane: Pane, wanted: Wanted, api: Reach) -> AgentStatus {
        AgentStatus { availability, pane, wanted, api, controller: true }
    }

    #[test]
    fn the_view_names_the_pane_the_wanted_state_and_the_api_and_holds_for_an_offline_api() {
        let idle = Availability::Idle { quiet_for_ms: Some(5) };
        let v = describe_status(&status(idle, Pane::Parked, Wanted::Running, Reach::Online), Some(AFTER));
        assert_eq!((v["pane"].as_str(), v["wanted"].as_str(), v["api"].as_str(), v["would"].as_str()), (Some("parked"), Some("running"), Some("online"), Some("deliver")));
        let v = describe_status(&status(idle, Pane::Shown, Wanted::Running, Reach::Offline), Some(AFTER));
        assert_eq!((v["api"].as_str(), v["would"].as_str(), v["reason"].as_str(), v["retry_when"].as_str()), (Some("offline"), Some("hold"), Some("api_offline"), Some("api_online")));
        let working = Availability::Running { waiting_on_user: false };
        let v = describe_status(&status(working, Pane::Shown, Wanted::Running, Reach::Offline), Some(AFTER));
        assert_eq!(v["would"].as_str(), Some("deliver"), "a message that joins a turn goes");
    }
}
