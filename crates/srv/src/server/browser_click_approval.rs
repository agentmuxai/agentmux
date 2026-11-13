// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The person's approval of a committing click (browser spec §5.4): a click that
//! submits a form, or a button named like Send, Pay or Delete, waits for them in
//! its pane's banner. `act` clicks and `Browser` `capture` of a download use it.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use super::browser_actions::How;
use super::ui_handlers::banner_for;
use super::AppState;

/// Why a committing click wasn't approved: the banner couldn't be raised, or the
/// person answered (how, when the Log says so) no, too late, or took the pane over.
pub(crate) enum ClickRefused {
    NotAsked(StatusCode, String),
    Asked(Option<How>, StatusCode, String),
}

/// A committing click's request, put up and not yet answered: what settling the answer needs.
pub(crate) struct Asked {
    banner_at: String,
    owner_before: Option<String>,
    asked_at: std::time::Instant,
}

/// How long the person has to answer an approval.
const WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Put the approval of the committing click `ask` describes up in its pane's banner (spec §5.4) without waiting
/// for the answer. The summary is shown, not the fingerprint of everything the click sends (hidden controls
/// included): that stays in `ask`, for the host's check before the approved click.
pub(crate) fn raise_approval(
    state: &AppState,
    block_id: &str,
    agent: &str,
    ask: &serde_json::Value,
) -> Result<(super::browser_attention::Raised, Asked), ClickRefused> {
    let (banner_at, in_window) = banner_for(block_id);
    let mut banner = ask.clone();
    if let Some(b) = banner.as_object_mut() {
        b.remove("fingerprint");
        b.insert("window".to_string(), json!(in_window));
    }
    let (owner_before, asked_at) = (super::browser_owner::owner_of(&banner_at), std::time::Instant::now());
    let raised = super::browser_attention::raise(state, &banner_at, agent, super::browser_attention::Kind::Approval, banner)
        .map_err(|e| ClickRefused::NotAsked(e.status(), e.to_string()))?;
    Ok((raised, Asked { banner_at, owner_before, asked_at }))
}

/// What the person's answer means for the click: `Ok` holds the pane locked (the returned [`Waiting`]) for the
/// click to be made; `Err` says why there is none.
pub(crate) fn settle_approval(
    asked: &Asked,
    block_id: &str,
    ask: &serde_json::Value,
    answer: super::browser_attention::Answer,
    waiting: super::browser_attention::Waiting,
) -> Result<(super::browser_attention::Waiting, std::time::Instant), ClickRefused> {
    use super::browser_attention::Answer;
    let took_over = super::browser_owner::owner_of(&asked.banner_at) != asked.owner_before;
    match answer {
        // The user may have taken the pane over while the banner was up:
        // their Take over wins over an Approve that comes after it.
        Answer::Yes | Answer::YesAlways | Answer::YesAsking if took_over => Err(ClickRefused::Asked(
            None,
            StatusCode::FORBIDDEN,
            "the user took over this pane, so the approved click wasn't made".to_string(),
        )),
        Answer::Yes | Answer::YesAlways | Answer::YesAsking => {
            super::browser_dialog::note_approved_click(block_id, ask);
            Ok((waiting, asked.asked_at))
        }
        // Taking the pane over answers its requests as Cancelled: that is not the person's "no".
        Answer::Cancelled if took_over => Err(ClickRefused::Asked(
            None,
            StatusCode::FORBIDDEN,
            "the user took over this pane, so the click wasn't made".to_string(),
        )),
        Answer::Cancelled => Err(ClickRefused::Asked(
            Some(How::Declined),
            StatusCode::FORBIDDEN,
            "the user didn't approve this action: don't retry it; ask them what to do".to_string(),
        )),
        Answer::TimedOut => Err(ClickRefused::Asked(
            Some(How::TimedOut),
            StatusCode::REQUEST_TIMEOUT,
            "the user didn't answer the approval within 10 minutes; ask them in chat before trying again".to_string(),
        )),
    }
}

/// Asks the person to approve the committing click `ask` describes, in its pane's
/// banner (spec §5.4). `Ok` holds the pane locked until it is dropped, so the form
/// can't change between what they approved and what is sent; make the approved
/// click before dropping it.
pub(crate) async fn approve_click(
    state: &AppState,
    block_id: &str,
    agent: &str,
    ask: &serde_json::Value,
) -> Result<(super::browser_attention::Waiting, std::time::Instant), ClickRefused> {
    let (raised, asked) = raise_approval(state, block_id, agent, ask)?;
    let (answer, waiting) = raised.answer(Some(WAIT)).await;
    settle_approval(&asked, block_id, ask, answer, waiting)
}

/// The question the agent is told it waits on: the click's own words and its page.
fn question_of(ask: &serde_json::Value) -> String {
    let s = |k: &str| ask.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match (s("what"), s("page")) {
        ("", "") => "a committing click".to_string(),
        (what, "") => what.to_string(),
        (what, page) => format!("{what} on {page}"),
    }
}

/// Where the approved click left the page, for the agent's note.
fn where_text(clicked: &serde_json::Value) -> String {
    match clicked.pointer("/page/url").and_then(|v| v.as_str()).filter(|u| !u.is_empty()) {
        Some(url) => format!("The page is now {url}."),
        None => "Take a new snapshot to see the page.".to_string(),
    }
}

/// The state a click that wasn't made leaves its request in.
fn refused_state(how: Option<&How>, why: &str) -> super::browser_waiting::State {
    use super::browser_waiting::State;
    match how {
        Some(How::Declined) => State::Declined,
        Some(How::TimedOut) => State::TimedOut,
        _ => State::Void(why.to_string()),
    }
}

/// The tail of a committing click that wasn't let through: ask the person about
/// `ask`, then make the click the same way once they approve (`approved_body`,
/// sent to the host's `route`, carries what they were shown). Used by `act` and
/// by Enter and Space taken as a click.
///
/// If the person hasn't answered within the grace, the call returns "waiting" with the request's id and
/// the agent ends its turn ([`super::browser_waiting`]); a task of srv's makes the click on their approval
/// and tells the agent how it went.
pub(crate) async fn ask_then_click(
    state: &AppState,
    block_id: &str,
    agent: &str,
    route: &str,
    ask: &serde_json::Value,
    approved_body: serde_json::Value,
    log: impl Fn() -> super::browser_actions::Note,
) -> axum::response::Response {
    ask_then_click_within(super::browser_waiting::grace(), state, block_id, agent, route, ask, approved_body, log).await
}

/// [`ask_then_click`] with the grace given (the tests', which don't wait 8 s).
#[allow(clippy::too_many_arguments)]
async fn ask_then_click_within(
    grace: std::time::Duration,
    state: &AppState,
    block_id: &str,
    agent: &str,
    route: &str,
    ask: &serde_json::Value,
    approved_body: serde_json::Value,
    log: impl Fn() -> super::browser_actions::Note,
) -> axum::response::Response {
    use super::ui_handlers::{err_response, proxy_value};
    let refuse = |e: ClickRefused| match e {
        ClickRefused::NotAsked(code, e) => err_response(code, e),
        ClickRefused::Asked(how, code, e) => match how {
            Some(how) => log().asked(ask).how(how).fail(state, code, e),
            None => log().asked(ask).fail(state, code, e),
        },
    };
    let (raised, asked) = match raise_approval(state, block_id, agent, ask) {
        Ok(r) => r,
        Err(e) => return refuse(e),
    };
    let id = raised.id().to_string();
    let settled = match raised.answer_within(grace).await {
        Ok((answer, waiting)) => settle_approval(&asked, block_id, ask, answer, waiting),
        Err(raised) => {
            let question = question_of(ask);
            super::browser_waiting::open(&id, agent, &asked.banner_at, "approval", &question, WAIT.saturating_sub(asked.asked_at.elapsed()));
            let (state, block_id, agent, route, ask, note) =
                (state.clone(), block_id.to_string(), agent.to_string(), route.to_string(), ask.clone(), log());
            let task_id = id.clone();
            tokio::spawn(async move {
                let (super::browser_attention::Settled { answer, by_agent }, waiting) =
                    raised.settled(Some(WAIT.saturating_sub(asked.asked_at.elapsed()))).await;
                // The agent took the question back itself: nothing is clicked and nobody is woken.
                if by_agent {
                    drop(waiting);
                    note.asked(&ask).how(How::Refused("taken back by the agent".to_string())).fail(&state, StatusCode::CONFLICT, String::new());
                    super::browser_waiting::finish(&task_id, super::browser_waiting::State::Cancelled);
                    return;
                }
                let settled = match settle_approval(&asked, &block_id, &ask, answer, waiting) {
                    Ok((_waiting, asked_at)) => {
                        let clicked = proxy_value(&state, &route, approved_body, std::time::Duration::from_secs(20)).await;
                        let made = super::browser_actions::approved_by_user(clicked, asked_at.elapsed());
                        let result = match &made {
                            Ok(data) => super::browser_waiting::State::Done(where_text(data)),
                            Err((_, e)) => super::browser_waiting::State::Failed(e.clone()),
                        };
                        note.asked(&ask).how(How::Approved).outcome(&state, made);
                        result
                    }
                    Err(ClickRefused::Asked(how, code, e)) => {
                        let result = refused_state(how.as_ref(), &e);
                        match how {
                            Some(how) => note.asked(&ask).how(how).fail(&state, code, e),
                            None => note.asked(&ask).fail(&state, code, e),
                        };
                        result
                    }
                    Err(ClickRefused::NotAsked(_, e)) => super::browser_waiting::State::Void(e),
                };
                // An agent that took the question down itself settled it first, and knows.
                if super::browser_waiting::finish(&task_id, settled.clone()) {
                    let text = super::browser_waiting::outcome_text(&task_id, "approval", &question_of(&ask), &settled);
                    super::browser_waiting::wake(&agent, text).await;
                }
            });
            return (StatusCode::OK, axum::Json(json!({ "ok": true, "data": super::browser_waiting::waiting_json(&id) }))).into_response();
        }
    };
    match settled {
        Ok((_waiting, asked_at)) => {
            let clicked = proxy_value(state, route, approved_body, std::time::Duration::from_secs(20)).await;
            log().asked(ask).how(How::Approved).outcome(state, super::browser_actions::approved_by_user(clicked, asked_at.elapsed()))
        }
        Err(e) => refuse(e),
    }
}

#[cfg(test)]
mod tests {
    use super::super::browser_actions::{note, Kind};
    use super::super::browser_attention::{self, Answer};
    use super::super::browser_waiting as waiting;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const AGENT: &str = "agent-click";

    /// A host whose `act` answers like an approved click and counts how often it is asked.
    async fn fake_host(state: &AppState) -> Arc<AtomicUsize> {
        use axum::{routing::post, Json, Router};
        let clicks = Arc::new(AtomicUsize::new(0));
        let seen = clicks.clone();
        let app = Router::new().route(
            "/agentmux/browser/act",
            post(move |Json(_b): Json<serde_json::Value>| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Json(json!({ "ok": true, "data": { "page": { "url": "https://a.test/sent", "title": "", "changed": true } } }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-ipc-token".into() });
        clicks
    }

    fn ask() -> serde_json::Value {
        json!({ "what": "Click \"Send\"", "page": "https://a.test/form", "fingerprint": "fp" })
    }

    async fn body_of(r: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let code = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (code, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    async fn settled(id: &str) -> waiting::Status {
        for _ in 0..150 {
            let s = waiting::status(AGENT, Some(id), None).unwrap();
            if s.state != "open" {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("request {id} never settled");
    }

    /// The call, with `grace`, on `pane`; whatever it answered.
    async fn call(state: &AppState, pane: &str, grace: Duration) -> (StatusCode, serde_json::Value) {
        let log = || note(pane, AGENT, Kind::Click);
        body_of(ask_then_click_within(grace, state, pane, AGENT, "act", &ask(), json!({ "block_id": pane, "approved": true }), log).await).await
    }

    fn pane(state: &AppState) -> String {
        let pane = uuid::Uuid::new_v4().to_string();
        browser_attention::tests::browser_block(state, &pane);
        pane
    }

    #[tokio::test]
    async fn a_quick_allow_still_returns_the_finished_click() {
        let state = crate::server::tests::test_state();
        let clicks = fake_host(&state).await;
        let pane = pane(&state);
        let (answered_state, answered_pane) = (state.clone(), pane.clone());
        tokio::spawn(async move {
            for _ in 0..200 {
                if let Some(id) = browser_attention::tests::banner(&answered_state, &answered_pane)["id"].as_str() {
                    let _ = browser_attention::resolve(id, &answered_pane, Answer::Yes);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let (code, body) = call(&state, &pane, Duration::from_secs(10)).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert!(body["data"].get("waiting").is_none(), "it answered with the finished result: {body}");
        assert_eq!(body["data"]["approved_by_user"], true);
        assert_eq!(clicks.load(Ordering::SeqCst), 1);
        assert_eq!(waiting::open_on(&pane), None, "nothing was left waiting");
    }

    #[tokio::test]
    async fn a_slow_allow_returns_waiting_then_clicks_once_and_settles_done() {
        let state = crate::server::tests::test_state();
        let clicks = fake_host(&state).await;
        let pane = pane(&state);
        let (code, body) = call(&state, &pane, Duration::from_millis(30)).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let id = body["data"]["request"].as_str().expect("waiting names the request").to_string();
        assert_eq!((body["data"]["waiting"].as_bool(), body["data"]["kind"].as_str()), (Some(true), Some("approval")));
        assert!(body["data"]["question"].as_str().unwrap().contains("Send"));
        assert_eq!(clicks.load(Ordering::SeqCst), 0, "nothing is clicked before the person answers");
        // The banner stays up, and the agent's other calls on the pane are refused with the request's id.
        assert_eq!(browser_attention::tests::banner(&state, &pane)["kind"], "approval");
        let refused = crate::server::ui_handlers::not_waiting_on_user(&pane).unwrap_err().1;
        assert!(refused.contains(&id) && refused.contains("end your turn"), "{refused}");
        browser_attention::resolve(&id, &pane, Answer::Yes).unwrap();
        let s = settled(&id).await;
        assert_eq!((s.state, s.detail.as_deref()), ("done", Some("The page is now https://a.test/sent.")));
        assert_eq!(clicks.load(Ordering::SeqCst), 1, "the approved click is made once");
        assert!(browser_attention::tests::banner(&state, &pane).is_null(), "the banner comes down");
        assert!(crate::server::ui_handlers::not_waiting_on_user(&pane).is_ok());
    }

    #[tokio::test]
    async fn a_declined_request_clicks_nothing_and_says_declined() {
        let state = crate::server::tests::test_state();
        let clicks = fake_host(&state).await;
        let pane = pane(&state);
        let (_, body) = call(&state, &pane, Duration::from_millis(30)).await;
        let id = body["data"]["request"].as_str().unwrap().to_string();
        browser_attention::resolve(&id, &pane, Answer::Cancelled).unwrap();
        assert_eq!(settled(&id).await.state, "declined");
        assert_eq!(clicks.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_agent_that_takes_its_request_back_ends_it_without_a_click() {
        let state = crate::server::tests::test_state();
        let clicks = fake_host(&state).await;
        let pane = pane(&state);
        let (_, body) = call(&state, &pane, Duration::from_millis(30)).await;
        let id = body["data"]["request"].as_str().unwrap().to_string();
        let at = waiting::open_pane_of(AGENT, &id).unwrap();
        browser_attention::withdraw(&id, &at).unwrap();
        assert_eq!(settled(&id).await.state, "cancelled");
        assert_eq!(clicks.load(Ordering::SeqCst), 0);
        assert!(browser_attention::tests::banner(&state, &pane).is_null());
    }

    #[tokio::test]
    async fn an_answer_that_came_first_stands_against_a_late_cancel() {
        let state = crate::server::tests::test_state();
        let clicks = fake_host(&state).await;
        let pane = pane(&state);
        let (_, body) = call(&state, &pane, Duration::from_millis(30)).await;
        let id = body["data"]["request"].as_str().unwrap().to_string();
        // The person's Allow is in; the agent's cancel starts after it.
        browser_attention::resolve(&id, &pane, Answer::Yes).unwrap();
        let at = waiting::open_pane_of(AGENT, &id).unwrap();
        assert!(browser_attention::withdraw(&id, &at).is_err(), "the request was already answered");
        let s = settled(&id).await;
        assert_eq!(s.state, "done", "the approved click is made and the agent is told");
        assert_eq!(clicks.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_person_who_cancels_while_the_agent_cancels_is_read_as_one_or_the_other_never_both() {
        let state = crate::server::tests::test_state();
        let clicks = fake_host(&state).await;
        let pane = pane(&state);
        let (_, body) = call(&state, &pane, Duration::from_millis(30)).await;
        let id = body["data"]["request"].as_str().unwrap().to_string();
        // The person's Cancel is in; the agent's cancel starts after it and finds the request answered.
        browser_attention::resolve(&id, &pane, Answer::Cancelled).unwrap();
        let at = waiting::open_pane_of(AGENT, &id).unwrap();
        assert!(browser_attention::withdraw(&id, &at).is_err(), "the person's answer settled it");
        assert_eq!(settled(&id).await.state, "declined", "the agent is told it was declined, not that it cancelled");
        assert_eq!(clicks.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_click_that_was_not_made_settles_by_why() {
        use super::super::browser_waiting::State;
        assert_eq!(refused_state(Some(&How::Declined), "x"), State::Declined);
        assert_eq!(refused_state(Some(&How::TimedOut), "x"), State::TimedOut);
        assert_eq!(refused_state(None, "the user took over this pane"), State::Void("the user took over this pane".to_string()));
        assert_eq!(question_of(&ask()), "Click \"Send\" on https://a.test/form");
        assert_eq!(where_text(&json!({ "page": { "url": "https://a.test/x" } })), "The page is now https://a.test/x.");
        assert!(where_text(&json!({})).contains("snapshot"));
    }
}
