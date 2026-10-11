// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP side of work claims
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.4):
//!
//! - `POST /api/v1/work-claims`: `ClaimWork`. Records (or renews) the
//!   caller's claim on a path, branch or topic, and answers with who else is
//!   already working on it.
//! - `POST /api/v1/work-claims/release`: `ReleaseWork`. Releases one of the
//!   caller's claims, or all of them.
//!
//! Claims are stored in the identity store (`storage::work_claims`), so
//! every channel's `WhoIsWorkingOn` and overlap notes see them. Nothing is
//! ever locked: a claim only informs.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::backend::storage::work_claims::WorkClaim;
use crate::backend::work_facts::cross_channel::own_channel;
use crate::backend::work_facts::{claims, matching};

use super::caller::Caller;
use super::work_facts_handlers::{caller_block, cross_channel_facts, local_facts, split_caller};
use super::AppState;

/// `ClaimWork`'s arguments, plus who is asking (`agent`, `block`: the MCP's
/// own `AGENTMUX_AGENT_ID` and block id; the request's agent token wins).
#[derive(Debug, Default, Deserialize)]
pub struct ClaimParams {
    pub path: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub topic: Option<String>,
    pub note: Option<String>,
    pub ttl_minutes: Option<u64>,
    pub agent: Option<String>,
    pub block: Option<String>,
}

/// `ReleaseWork`'s arguments: a claim id, or none for all of the caller's.
#[derive(Debug, Default, Deserialize)]
pub struct ReleaseParams {
    pub id: Option<String>,
    pub agent: Option<String>,
    pub block: Option<String>,
}

fn bad_request(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.into() }))).into_response()
}

/// The longest note, topic, path, branch and repository a claim keeps, in
/// characters. Another agent reads them in its notes and answers.
const MAX_NOTE: usize = 200;
const MAX_TOPIC: usize = 120;
const MAX_PATH: usize = 400;
const MAX_NAME: usize = 200;

/// A claim's free text as it is kept: one line (control characters and line
/// breaks become spaces, whitespace runs collapse) of at most `max`
/// characters. Marker quoting happens where the text is shown inside a
/// system note (`work_facts::overlap`).
fn one_line(s: &Option<String>, max: usize) -> Option<String> {
    let flat: String = s.as_deref()?.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    Some(match flat.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    })
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Who is asking: the name and UID of the caller's registration (found by
/// token, block, then unique name, as `WhoIsWorkingOn` does), else the name
/// it sent. `None` when there is no name at all.
fn claimer(
    state: &AppState,
    caller: Option<&Caller>,
    agent: Option<&str>,
    block: Option<&str>,
) -> Option<(String, String, Option<String>)> {
    let regs = state.reactive_handler.list_agents();
    let token_uid = caller.and_then(|c| c.uid().map(str::to_string));
    let block_id = caller_block(&regs, token_uid.as_deref(), block, agent);
    let reg = block_id.as_deref().and_then(|b| regs.iter().find(|r| r.block_id == b));
    let name = reg
        .map(|r| r.agent_id.clone())
        .or_else(|| agent.map(str::trim).filter(|a| !a.is_empty()).map(str::to_string))?;
    let uid = token_uid.or_else(|| reg.and_then(|r| r.uid.clone())).unwrap_or_default();
    Some((name, uid, block_id))
}

/// `POST /api/v1/work-claims`: `ClaimWork`.
pub(crate) async fn handle_claim_work(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(p): Json<ClaimParams>,
) -> Response {
    let p = ClaimParams {
        path: one_line(&p.path, MAX_PATH),
        repo: one_line(&p.repo, MAX_NAME),
        branch: one_line(&p.branch, MAX_NAME),
        topic: one_line(&p.topic, MAX_TOPIC),
        note: one_line(&p.note, MAX_NOTE),
        ..p
    };
    if [&p.path, &p.repo, &p.branch, &p.topic].iter().all(|v| nonempty(v).is_none()) {
        return bad_request("nothing to claim: pass path, branch or topic (and repo for a whole repository)");
    }
    let Some((agent, uid, me_block)) =
        claimer(&state, caller.as_ref().map(|c| &c.0), p.agent.as_deref(), p.block.as_deref())
    else {
        return bad_request("who is claiming is unknown: this needs an agent started by AgentMux");
    };
    let (local, (cross, _)) = tokio::join!(local_facts(&state), cross_channel_facts(&state));
    let (me, mut others) = split_caller(local, me_block.as_deref());
    others.extend(cross);

    let q = matching::WhoQuery {
        path: p.path.clone(),
        repo: p.repo.clone(),
        branch: p.branch.clone(),
        query: p.topic.clone(),
    };
    let mut target = match matching::resolve(&q, me.as_ref(), &others) {
        Ok(t) => t,
        Err(e) => return bad_request(e),
    };
    // Only a repository named: the whole of it.
    if [&p.path, &p.branch, &p.topic].iter().all(|v| nonempty(v).is_none()) {
        target.path = Some(String::new());
    }

    let now = agentmux_common::time::now_ms();
    let claim = WorkClaim {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.clone(),
        agent_uid: uid.clone(),
        channel: own_channel(),
        repo: target.repo.clone(),
        path: target.path.clone(),
        absolute_path: target.absolute_path.clone(),
        branch: target.branch.clone(),
        topic: target.query.clone(),
        note: p.note.clone().unwrap_or_default(),
        created_at: now,
        expires_at: now + claims::ttl_ms(p.ttl_minutes),
    };
    let store = state.identity_store.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let stored = store.work_claim_put(&claim, now)?;
        let live = store.work_claims_live(now)?;
        Ok::<_, crate::backend::storage::StoreError>((stored, live))
    })
    .await;
    let (stored, live) = match stored {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("claim not saved: {e}") })))
                .into_response()
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("claim not saved: {e}") })))
                .into_response()
        }
    };

    // Who else is already on it, their claims included.
    let theirs: Vec<WorkClaim> = live.into_iter().filter(|c| !claims::is_own(c, &agent, &uid)).collect();
    let others: Vec<_> = others.into_iter().filter(|f| !f.agent.eq_ignore_ascii_case(&agent)).collect();
    let mut overlapping = matching::who_is_working_on(&target, &others);
    claims::add_claim_matches(&mut overlapping, &target, &theirs, &others, now);
    Json(json!({
        "claim": stored,
        "claimed": claims::describe(&stored),
        "others_working_on_it": overlapping,
        "note": "Claimed. Other agents see this in WhoIsWorkingOn and are told when they edit a file in it. It \
                 blocks no one, and lapses at expires_at unless you claim it again. Release it with ReleaseWork \
                 when you are done. If others_working_on_it names anyone, message them (SendMessage).",
    }))
    .into_response()
}

/// `POST /api/v1/work-claims/release`: `ReleaseWork`.
pub(crate) async fn handle_release_work(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(p): Json<ReleaseParams>,
) -> Response {
    let Some((agent, uid, _)) = claimer(&state, caller.as_ref().map(|c| &c.0), p.agent.as_deref(), p.block.as_deref())
    else {
        return bad_request("who is releasing is unknown: this needs an agent started by AgentMux");
    };
    let id = nonempty(&p.id).map(str::to_string);
    let store = state.identity_store.clone();
    let now = agentmux_common::time::now_ms();
    let released = tokio::task::spawn_blocking(move || store.work_claim_release(&agent, &uid, id.as_deref(), now)).await;
    match released {
        Ok(Ok(released)) => {
            let note = if released.is_empty() {
                "Nothing released: you hold no live claim with that id."
            } else {
                "Released."
            };
            Json(json!({ "released": released, "note": note })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("release failed: {e}") })))
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("release failed: {e}") })))
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body(resp: Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn a_claim_needs_something_to_claim_and_someone_claiming() {
        let state = crate::server::tests::test_state();
        let (status, b) = body(handle_claim_work(State(state.clone()), None, Json(ClaimParams::default())).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(b["error"].as_str().unwrap().contains("nothing to claim"));
        let anon = ClaimParams { topic: Some("x".into()), ..Default::default() };
        let (status, b) = body(handle_claim_work(State(state), None, Json(anon)).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(b["error"].as_str().unwrap().contains("who is claiming"));
    }

    #[test]
    fn claim_text_is_kept_as_one_short_line() {
        assert_eq!(one_line(&Some("  a\n[AgentMux] b\t\u{7}c  ".into()), 50).as_deref(), Some("a [AgentMux] b c"));
        assert_eq!(one_line(&Some("abcdef".into()), 3).as_deref(), Some("abc…"));
        assert_eq!(one_line(&Some("ééé".into()), 3).as_deref(), Some("ééé"), "counted in characters");
        assert_eq!(one_line(&Some(" \n ".into()), 3), None);
        assert_eq!(one_line(&None, 3), None);
    }

    #[tokio::test]
    async fn claim_who_and_release_round_trip() {
        let state = crate::server::tests::test_state();
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let (me, other) = (format!("claimer-{unique}"), format!("asker-{unique}"));
        let topic = format!("topic{unique}");

        let claim = ClaimParams {
            topic: Some(topic.clone()),
            note: Some("presence work".into()),
            ttl_minutes: Some(30),
            agent: Some(me.clone()),
            ..Default::default()
        };
        let (status, b) = body(handle_claim_work(State(state.clone()), None, Json(claim)).await).await;
        assert_eq!(status, StatusCode::OK, "{b}");
        assert_eq!(b["claim"]["agent"], me.as_str());
        assert_eq!(b["claim"]["topic"], topic.as_str());
        let id = b["claim"]["id"].as_str().unwrap().to_string();

        // Another agent asking about the topic sees the claim.
        let who = super::super::work_facts_handlers::WhoParams {
            query: Some(topic.clone()),
            agent: Some(other.clone()),
            ..Default::default()
        };
        let (status, b) = body(
            super::super::work_facts_handlers::handle_who_is_working_on(
                State(state.clone()),
                None,
                axum::extract::Query(who),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{b}");
        let agents = b["agents"].as_array().unwrap();
        let found = agents.iter().find(|a| a["agent"] == me.as_str()).expect("the claimer is listed");
        assert_eq!(found["matches"][0]["kind"], "claim");
        assert_eq!(found["status"], "not running", "no registration in this test");

        // Only the claimer releases it.
        let wrong = ReleaseParams { id: Some(id.clone()), agent: Some(other), ..Default::default() };
        let (_, b) = body(handle_release_work(State(state.clone()), None, Json(wrong)).await).await;
        assert!(b["released"].as_array().unwrap().is_empty());
        let mine = ReleaseParams { id: Some(id), agent: Some(me), ..Default::default() };
        let (_, b) = body(handle_release_work(State(state), None, Json(mine)).await).await;
        assert_eq!(b["released"].as_array().unwrap().len(), 1);
    }
}
