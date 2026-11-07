// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/browser/take_control` and `/release`: an agent drives a browser pane it did not open, and lets one go
//! (docs/specs/SPEC_BROWSER_RETAKE_AND_COMMANDEER_2026_11_04.md §2). The profile rule is the one that decides who may
//! open a profile (`browser_identity::check_agent_may_use`); on top of it the person is asked, in the pane, when their
//! own sign-ins, their hands or another agent's work are involved. Neither route goes through `target_block_id`: the
//! pane is not the caller's yet.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::server::browser_actions::{note, How, Kind};
use crate::server::browser_attention::{self, Answer};
use crate::server::browser_owner::{self, IDLE_BEFORE_RETAKE};
use crate::server::ui_handlers::{err_response, verified_block_id};
use crate::server::AppState;

/// How long the person has to answer "X wants to drive this pane".
const ASK_FOR: Duration = Duration::from_secs(120);

/// What the person is asked, and why; nothing when taking the pane needs no one's say.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Need {
    Nothing,
    Ask(String),
}

/// The facts that decide whether `agent` may take a pane (`what_it_takes`).
pub(crate) struct Facts<'a> {
    /// The pane shows the person's Personal sign-ins (no profile of its own).
    pub personal: bool,
    /// The agent runs in a container.
    pub in_container: bool,
    /// The agent that drives the pane now, if it is not the one asking.
    pub driven_by: Option<&'a str>,
    /// How long ago the host last saw the person use the pane's page (`Duration::MAX`: never), None if it can't say.
    pub since_input: Option<Duration>,
}

/// What taking a pane takes, once the profile rule has passed: a refusal, the person's say, or nothing. Personal
/// sign-ins are the person's own: a container agent never gets them without the person sharing a profile, and a
/// native agent asks.
pub(crate) fn what_it_takes(f: &Facts) -> Result<Need, String> {
    if f.personal && f.in_container {
        return Err(
            "this pane shows the user's own sign-ins (their Personal profile), which a container agent can't take. Ask the \
             user to open the site in a profile they let agents use (Settings → Browser → Agents may use it), or open your own pane"
                .to_string(),
        );
    }
    if f.personal {
        return Ok(Need::Ask("it shows your own sign-ins".to_string()));
    }
    if let Some(owner) = f.driven_by {
        return Ok(Need::Ask(format!("{owner} is driving it")));
    }
    match f.since_input {
        Some(d) if d >= IDLE_BEFORE_RETAKE => Ok(Need::Nothing),
        Some(d) => Ok(Need::Ask(format!("you used it {} s ago", d.as_secs()))),
        None => Ok(Need::Ask("AgentMux couldn't tell whether you are using it".to_string())),
    }
}

/// How long the person has left the pane alone, for [`Facts::since_input`]: the later of their last input in the page and
/// their Take over click, which the host does not see. A pane the person took over from an agent, whose takeover srv no
/// longer remembers (it restarted), is not known to be quiet.
fn quiet_for(since_input: Option<Duration>, since_takeover: Option<Duration>, person_took: bool) -> Option<Duration> {
    match (since_input, since_takeover) {
        (Some(i), Some(t)) => Some(i.min(t)),
        (_, None) if person_took => None,
        (i, _) => i,
    }
}

fn trimmed(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// The pane `pane` as stored, when it is a browser pane the caller may be refused or allowed on.
fn browser_block(state: &AppState, pane: &str) -> Result<crate::backend::obj::Block, axum::response::Response> {
    match state.mstore.get::<crate::backend::obj::Block>(pane) {
        Ok(Some(b)) if b.meta.get("view").and_then(|v| v.as_str()) == Some("browser") => Ok(b),
        Ok(Some(_)) => Err(err_response(StatusCode::BAD_REQUEST, format!("pane {pane:?} is not a browser pane"))),
        Ok(None) => Err(err_response(StatusCode::NOT_FOUND, format!("no pane {pane:?} (it may have been closed)"))),
        Err(e) => Err(err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("load pane {pane:?}: {e}"))),
    }
}

pub(crate) async fn handle_ui_browser_take_control(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserTakeControlRequest>,
) -> impl IntoResponse {
    let own = match verified_block_id(&state, caller.as_deref(), &req.auth) {
        Ok(b) => b,
        Err(e) => return err_response(StatusCode::UNAUTHORIZED, e),
    };
    let agent = req.auth.agent_id.as_str();
    let Some(pane) = trimmed(&req.pane) else {
        return err_response(StatusCode::BAD_REQUEST, "Browser take_control needs pane: the id of a browser pane (Pane layout lists them)".to_string());
    };
    if crate::server::browser_popup::is_window_id(pane) {
        return err_response(StatusCode::BAD_REQUEST, "take_control takes a browser pane, not a window a page opened".to_string());
    }
    let block = match browser_block(&state, pane) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let driven_by = browser_owner::owner_of(pane);
    if driven_by.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(agent)) {
        return (StatusCode::OK, Json(json!({ "ok": true, "data": { "pane": pane, "already": true } }))).into_response();
    }
    // The profile rule first: a pane in a profile the agent could not have opened is not asked about.
    let owner_id = crate::server::browser_identity::profile_owner(&state, agent);
    if let Err(e) = crate::server::browser_identity::check_agent_may_use(&block.meta, &owner_id) {
        return err_response(StatusCode::FORBIDDEN, format!("{e}: switch on Agents may use it for that profile in Settings → Browser, or ask the user to"));
    }
    let person_took = browser_owner::driver_of(&block.meta).as_deref() == Some("person");
    let since_input = crate::server::browser_retake::since_person_input(&state, pane).await;
    let facts = Facts {
        personal: crate::server::browser_identity::browsing_as(&block.meta) == "personal",
        in_container: crate::server::browser_identity::in_container(&state, &own),
        driven_by: driven_by.as_deref(),
        since_input: quiet_for(since_input, browser_owner::taken_over_ago(pane), person_took),
    };
    let need = match what_it_takes(&facts) {
        Ok(n) => n,
        Err(e) => return err_response(StatusCode::FORBIDDEN, e),
    };
    let current_url = crate::server::browser_bypass_routes::page_url(&state, pane).await;
    let limit = match site_limit(req.allowed_origins.as_deref(), &current_url) {
        Ok(list) => list,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let current_url = current_url.unwrap_or_default();
    let how = match need {
        Need::Nothing => How::Allowed,
        Need::Ask(why) => match ask_the_person(&state, pane, agent, &why).await {
            Ok(()) => How::Approved,
            Err(r) => {
                let how = if r.status() == StatusCode::REQUEST_TIMEOUT { How::TimedOut } else { How::Declined };
                note(pane, agent, Kind::TookControl).detail(&why).how(how).record(&state);
                return r;
            }
        },
    };
    // Whoever drove it stops: its bypass and pause end with the ownership, as when the pane closes.
    if driven_by.is_some() {
        browser_owner::forget(pane);
    }
    // The header reads these keys: written before the agent drives, so the person never sees it driving unlabelled.
    let mut meta = browser_owner::took_control_meta(agent);
    crate::server::browser_allowlist::mirror(&mut meta, Some(&limit));
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(&state, pane, &meta) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't show you as the pane's driver: {e}"));
    }
    crate::server::browser_allowlist::limit_new_pane(&state, pane, Some(limit.clone())).await;
    browser_owner::record(pane, agent);
    let browsing_as = crate::server::browser_identity::browsing_as(&block.meta);
    let last = facts.since_input.filter(|d| *d != Duration::MAX).map_or("never".to_string(), |d| format!("{} s ago", d.as_secs()));
    note(pane, agent, Kind::TookControl).detail(&format!("profile {browsing_as}, the person last acted {last}")).how(how).record(&state);
    tracing::info!(agent_id = %agent, pane = %pane, profile = %browsing_as, "[browser-owner] agent took control of the pane");
    let data = json!({ "pane": pane, "already": false, "browsing_as": browsing_as, "limited_to": limit, "url": current_url });
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// The sites a pane taken is limited to: those the agent passed, else the site the page is on. A pane that shows no web page
/// (blank, a browser page) or whose address can't be read has no site to limit it to, and is not taken without a list: a
/// taken pane is never left unlimited by accident.
fn site_limit(asked: Option<&[String]>, page: &Result<String, String>) -> Result<Vec<String>, String> {
    if let Some(entries) = asked {
        let list = agentmux_common::allowed_origins::parse_list(entries).map_err(|e| format!("allowed_origins: {e}"))?;
        if list.is_empty() {
            return Err("allowed_origins is empty: list the sites the pane may go to".to_string());
        }
        return Ok(list);
    }
    let url = page.as_deref().map_err(|e| format!("couldn't read the pane's address ({e}): pass allowed_origins, the sites it may go to"))?;
    agentmux_common::allowed_origins::target_origin(url)
        .map(|o| vec![o])
        .ok_or_else(|| "the pane isn't showing a web page to limit it to: pass allowed_origins, the sites it may go to".to_string())
}

/// Put "X wants to drive this pane" up in the pane and wait for the person: Ok when they allow it, else the response to
/// give the agent (they said Not now, or nobody answered).
async fn ask_the_person(state: &AppState, pane: &str, agent: &str, why: &str) -> Result<(), axum::response::Response> {
    if let Err(e) = crate::server::app_api::connections::host_to_ask_user(state).await {
        return Err(err_response(StatusCode::SERVICE_UNAVAILABLE, e));
    }
    let payload = json!({ "reason": why });
    match browser_attention::ask(state, pane, agent, browser_attention::Kind::Take, payload, ASK_FOR).await {
        Ok((Answer::Yes | Answer::YesAlways | Answer::YesAsking, _waiting)) => Ok(()),
        Ok((Answer::Cancelled, _)) => Err(err_response(StatusCode::FORBIDDEN, format!("the user said Not now: you can't drive pane {pane:?} ({why}). Don't ask again unless they say so"))),
        Ok((Answer::TimedOut, _)) => Err(err_response(StatusCode::REQUEST_TIMEOUT, format!("the user didn't answer in {} s: pane {pane:?} is still theirs", ASK_FOR.as_secs()))),
        Err(e) => Err(err_response(e.status(), e.to_string())),
    }
}

pub(crate) async fn handle_ui_browser_release(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserReleaseRequest>,
) -> impl IntoResponse {
    if let Err(e) = verified_block_id(&state, caller.as_deref(), &req.auth) {
        return err_response(StatusCode::UNAUTHORIZED, e);
    }
    let agent = req.auth.agent_id.as_str();
    let Some(pane) = trimmed(&req.pane) else {
        return err_response(StatusCode::BAD_REQUEST, "Browser release needs pane: the id of a pane you drive".to_string());
    };
    if let Err(r) = browser_block(&state, pane) {
        return r;
    }
    if !browser_owner::owner_of(pane).is_some_and(|o| o.eq_ignore_ascii_case(agent)) {
        return err_response(StatusCode::FORBIDDEN, format!("you don't drive pane {pane:?}, so there is nothing to release"));
    }
    // The meta first: the header stops saying the agent drives it, and Give back is not offered for a pane nobody took.
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(&state, pane, &browser_owner::released_meta()) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't release the pane: {e}"));
    }
    note(pane, agent, Kind::Released).how(How::Allowed).record(&state);
    browser_owner::forget(pane);
    crate::server::browser_allowlist::drop_pane(pane);
    tracing::info!(agent_id = %agent, pane = %pane, "[browser-owner] agent released the pane");
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "pane": pane } }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as D;

    fn facts<'a>(personal: bool, in_container: bool, driven_by: Option<&'a str>, since_input: Option<D>) -> Facts<'a> {
        Facts { personal, in_container, driven_by, since_input }
    }

    use crate::backend::obj::MetaMapType;
    use crate::server::tests::test_state;
    use agentmux_common::api_types::{UiAutomationAuth, UiBrowserReleaseRequest, UiBrowserTakeControlRequest};

    fn auth(state: &AppState, agent_id: &str) -> UiAutomationAuth {
        let key = state.mstore.agent_jekt_key_ensure(agent_id).unwrap();
        let ts_secs = agentmux_common::time::now_secs();
        let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", agent_id, "__srv__", ts_secs, "");
        UiAutomationAuth { agent_id: agent_id.to_string(), ts_secs, sig }
    }

    /// A host that answers `page_url` and `last_input` (null: the person never used the page), on a port of its own.
    async fn fake_host(state: &AppState, idle_secs: Option<u64>) {
        fake_host_on(state, idle_secs, "https://example.com/a").await;
    }

    async fn fake_host_on(state: &AppState, idle_secs: Option<u64>, url: &'static str) {
        use axum::{routing::post, Router};
        let app = Router::new()
            .route(
                "/agentmux/browser/page_url",
                post(move |Json(_b): Json<serde_json::Value>| async move { Json(json!({ "ok": true, "data": { "url": url } })) }),
            )
            .route(
                "/agentmux/browser/last_input",
                post(move |Json(_b): Json<serde_json::Value>| async move { Json(json!({ "ok": true, "data": { "idle_secs": idle_secs } })) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-ipc-token".into() });
    }

    /// An agent with a pane of its own, and a browser pane for it to take (in an Incognito jar of its own unless
    /// `personal`).
    fn agent_and_pane(state: &AppState, personal: bool) -> (String, String) {
        let (own, pane) = (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string());
        crate::server::browser_attention::tests::browser_block(state, &own);
        crate::server::browser_attention::tests::browser_block(state, &pane);
        if !personal {
            let mut meta = MetaMapType::new();
            meta.insert("browser:identity".to_string(), json!(format!("incognito:{}", uuid::Uuid::new_v4())));
            crate::server::http_shell::broadcast_meta_update(state, &pane, &meta).unwrap();
        }
        let agent = format!("take-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent, &own, None).unwrap();
        (agent, pane)
    }

    async fn take(state: &AppState, agent: &str, pane: &str) -> StatusCode {
        let req = UiBrowserTakeControlRequest { auth: auth(state, agent), pane: Some(pane.to_string()), allowed_origins: None };
        handle_ui_browser_take_control(State(state.clone()), None, Json(req)).await.into_response().status()
    }

    async fn release(state: &AppState, agent: &str, pane: &str) -> StatusCode {
        let req = UiBrowserReleaseRequest { auth: auth(state, agent), pane: Some(pane.to_string()) };
        handle_ui_browser_release(State(state.clone()), None, Json(req)).await.into_response().status()
    }

    /// The person's answer to the "wants to drive this pane" banner, once it is up.
    fn person_answers(state: &AppState, pane: &str, answer: Answer) -> tokio::task::JoinHandle<()> {
        let (state, pane) = (state.clone(), pane.to_string());
        tokio::spawn(async move {
            for _ in 0..200 {
                let banner = browser_attention::tests::banner(&state, &pane);
                if banner["kind"] == "take" {
                    assert!(banner["reason"].as_str().is_some_and(|r| !r.is_empty()), "the banner says why: {banner}");
                    browser_attention::resolve(banner["id"].as_str().unwrap(), &pane, answer).unwrap();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no banner came up");
        })
    }

    #[tokio::test]
    async fn a_pane_nobody_uses_in_a_jar_agents_may_use_is_taken_without_asking() {
        let state = test_state();
        fake_host(&state, Some(120)).await;
        let (agent, pane) = agent_and_pane(&state, false);
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::OK);
        assert_eq!(browser_owner::owner_of(&pane).as_deref(), Some(agent.as_str()));
        let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
        assert_eq!(block.meta.get(browser_owner::OWNER_META_KEY).and_then(|v| v.as_str()), Some(agent.as_str()));
        assert_eq!(block.meta.get(browser_owner::TOOK_CONTROL_META_KEY), Some(&json!(true)));
        assert_eq!(
            block.meta.get(crate::server::browser_allowlist::ALLOWED_META_KEY),
            Some(&json!(["https://example.com"])),
            "limited to the site it is on"
        );
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::OK, "already driving it");
        let log = crate::server::browser_actions::entries(&pane).1;
        assert!(log.iter().any(|e| e.kind == Kind::TookControl && e.detail.as_deref().is_some_and(|d| d.contains("incognito"))), "{log:?}");

        // Letting it go: nobody drives it, and Give back is not offered for a pane nobody took from them.
        assert_eq!(release(&state, &agent, &pane).await, StatusCode::OK);
        assert!(browser_owner::owner_of(&pane).is_none());
        let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
        assert_eq!(browser_owner::driver_of(&block.meta), None);
        assert_eq!(release(&state, &agent, &pane).await, StatusCode::FORBIDDEN, "nothing left to release");
        crate::server::browser_allowlist::drop_pane(&pane);
    }

    #[tokio::test]
    async fn a_pane_that_shows_no_web_page_is_not_taken_without_a_list_of_sites() {
        let state = test_state();
        fake_host_on(&state, Some(120), "about:blank").await;
        let (agent, pane) = agent_and_pane(&state, false);
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::BAD_REQUEST);
        assert!(browser_owner::owner_of(&pane).is_none());
        let asked = Some(vec!["example.com".to_string()]);
        let req = UiBrowserTakeControlRequest { auth: auth(&state, &agent), pane: Some(pane.clone()), allowed_origins: asked };
        let status = handle_ui_browser_take_control(State(state.clone()), None, Json(req)).await.into_response().status();
        assert_eq!(status, StatusCode::OK, "with the sites it may go to");
        browser_owner::forget(&pane);
        crate::server::browser_allowlist::drop_pane(&pane);
    }

    #[tokio::test]
    async fn a_pane_the_person_just_took_over_asks_even_if_they_only_read_it() {
        let state = test_state();
        fake_host(&state, None).await;
        let (previous, pane) = agent_and_pane(&state, false);
        let (agent, _) = agent_and_pane(&state, false);
        browser_owner::record(&pane, &previous);
        let mut clear = MetaMapType::new();
        clear.insert(browser_owner::OWNER_META_KEY.to_string(), serde_json::Value::Null);
        browser_owner::guard_client_meta_write(&format!("block:{pane}"), &clear).unwrap();
        let answered = person_answers(&state, &pane, Answer::Cancelled);
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::FORBIDDEN, "took it over a moment ago, never touched the page");
        answered.await.unwrap();
        browser_owner::age_lost(&pane, Duration::from_secs(61));
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::OK, "61 s since the takeover and no input");
        browser_owner::forget(&pane);
        crate::server::browser_allowlist::drop_pane(&pane);
    }

    #[tokio::test]
    async fn a_pane_with_the_persons_own_sign_ins_waits_for_their_yes() {
        let state = test_state();
        fake_host(&state, Some(120)).await;
        let (agent, pane) = agent_and_pane(&state, true);
        let answered = person_answers(&state, &pane, Answer::Cancelled);
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::FORBIDDEN, "Not now");
        answered.await.unwrap();
        assert!(browser_owner::owner_of(&pane).is_none());
        let answered = person_answers(&state, &pane, Answer::Yes);
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::OK, "Allow");
        answered.await.unwrap();
        assert_eq!(browser_owner::owner_of(&pane).as_deref(), Some(agent.as_str()));
        let log = crate::server::browser_actions::entries(&pane).1;
        assert!(log.iter().any(|e| e.kind == Kind::TookControl && e.how == "approved"), "{log:?}");
        browser_owner::forget(&pane);
        crate::server::browser_allowlist::drop_pane(&pane);
    }

    #[tokio::test]
    async fn a_pane_another_agent_drives_is_taken_after_a_yes_and_a_released_one_without() {
        let state = test_state();
        fake_host(&state, Some(120)).await;
        let (first, pane) = agent_and_pane(&state, false);
        browser_owner::record(&pane, &first);
        let (second, _) = agent_and_pane(&state, false);
        let answered = person_answers(&state, &pane, Answer::Yes);
        assert_eq!(take(&state, &second, &pane).await, StatusCode::OK);
        answered.await.unwrap();
        assert_eq!(browser_owner::owner_of(&pane).as_deref(), Some(second.as_str()), "the first agent no longer drives it");
        assert_eq!(release(&state, &first, &pane).await, StatusCode::FORBIDDEN, "not the first agent's to release any more");
        assert_eq!(release(&state, &second, &pane).await, StatusCode::OK);
        let (third, _) = agent_and_pane(&state, false);
        assert_eq!(take(&state, &third, &pane).await, StatusCode::OK, "released: no one to ask");
        browser_owner::forget(&pane);
        crate::server::browser_allowlist::drop_pane(&pane);
    }

    #[tokio::test]
    async fn a_pane_the_person_used_a_moment_ago_asks_and_a_missing_one_is_not_found() {
        let state = test_state();
        fake_host(&state, Some(5)).await;
        let (agent, pane) = agent_and_pane(&state, false);
        let answered = person_answers(&state, &pane, Answer::Cancelled);
        assert_eq!(take(&state, &agent, &pane).await, StatusCode::FORBIDDEN);
        answered.await.unwrap();
        assert_eq!(take(&state, &agent, "no-such-pane").await, StatusCode::NOT_FOUND);
    }


    #[test]
    fn the_persons_takeover_click_counts_as_a_touch_of_the_pane() {
        let (s, m) = (|n| Some(D::from_secs(n)), D::MAX);
        assert_eq!(quiet_for(Some(m), s(5), true), s(5), "took it over 5 s ago and only read the page");
        assert_eq!(quiet_for(s(300), s(900), true), s(300), "typed 5 min ago, took it over before that");
        assert_eq!(quiet_for(Some(m), None, true), None, "taken over, and srv forgot when: not known to be quiet");
        assert_eq!(quiet_for(Some(m), None, false), Some(m), "never taken over from an agent");
        assert_eq!(quiet_for(None, s(900), true), None, "the host could not say");
    }

    #[test]
    fn a_taken_pane_is_limited_to_the_sites_asked_for_or_the_one_it_is_on() {
        let page = |u: &str| Ok::<String, String>(u.to_string());
        let asked = vec!["github.com".to_string(), "*.example.org".to_string()];
        let list = site_limit(Some(&asked), &page("https://elsewhere.test/x")).unwrap();
        assert_eq!(list.len(), 2, "the list the agent passed, wherever the page is now");
        assert_eq!(site_limit(None, &page("https://example.com/a?b=1")).unwrap(), vec!["https://example.com".to_string()]);
        assert!(site_limit(None, &page("about:blank")).unwrap_err().contains("pass allowed_origins"), "no site to limit it to");
        assert!(site_limit(None, &Err("timed out".to_string())).unwrap_err().contains("couldn't read"), "address unreadable");
        assert!(site_limit(Some(&[]), &page("https://example.com")).unwrap_err().contains("empty"));
        assert!(site_limit(Some(&["not a site".to_string()]), &page("https://example.com")).is_err());
        assert!(site_limit(Some(&asked), &Err("x".to_string())).is_ok(), "a list needs no address");
    }

    /// The table of §2.2 and §2.3: who is refused, who is asked, who walks in.
    #[test]
    fn what_taking_a_pane_takes() {
        let idle = Some(D::from_secs(120));
        assert_eq!(what_it_takes(&facts(false, false, None, idle)), Ok(Need::Nothing), "a profile agents may use, nobody in it");
        assert_eq!(what_it_takes(&facts(false, true, None, Some(D::MAX))), Ok(Need::Nothing), "a container agent, a shared profile, never touched");
        assert!(matches!(what_it_takes(&facts(true, false, None, idle)), Ok(Need::Ask(w)) if w.contains("sign-ins")), "Personal asks a native agent");
        assert!(what_it_takes(&facts(true, true, None, idle)).unwrap_err().contains("container agent"), "Personal is a container agent's no");
        assert!(matches!(what_it_takes(&facts(false, false, Some("korp"), idle)), Ok(Need::Ask(w)) if w.contains("korp")), "another agent drives it");
        assert!(matches!(what_it_takes(&facts(false, false, None, Some(D::from_secs(12)))), Ok(Need::Ask(w)) if w.contains("12 s ago")), "used 12 s ago");
        assert_eq!(what_it_takes(&facts(false, false, None, Some(D::from_secs(60)))), Ok(Need::Nothing), "60 s is a minute");
        assert!(matches!(what_it_takes(&facts(false, false, None, None)), Ok(Need::Ask(_))), "the host cannot say: ask");
    }
}
