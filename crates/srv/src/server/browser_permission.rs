// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page's camera, microphone, location or notification request in a browser pane an agent drives
//! (docs/reports/REPORT_AGENTS_AND_OS_PROMPTS_2026_11_07.md §3 tier 1, §4). The host holds the request
//! and asks here (`/api/v1/host/browser_permission`). Nothing is allowed on the agent's word: the
//! first ask files an `os_prompt_answer` request like any sensitive operation (the super agent
//! reviews it, the person can veto its countdown), and answers `pending` at once, so the page's
//! request is denied now and the agent is told. When the countdown ends unvetoed the site holds a
//! grant for a while, and the page's next request is allowed without asking again.
//!
//! Screen and system-audio capture are not offered: the agent would be sharing the person's screen.

use std::collections::BTreeMap;
use std::sync::Mutex;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::server::browser_dialog::record;
use crate::server::ui_handlers::{banner_for, err_response, from_host};
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// CEF's `cef_media_access_permission_types_t` bits.
const DEVICE_AUDIO_CAPTURE: u32 = 1;
const DEVICE_VIDEO_CAPTURE: u32 = 2;
/// CEF's `cef_permission_request_types_t` bits for a permission prompt (a different list from the media bits).
const PROMPT_GEOLOCATION: u32 = 256;
const PROMPT_NOTIFICATIONS: u32 = 32768;
/// How long an approved site stays allowed.
const GRANT_MS: i64 = 30 * 60 * 1000;
/// Asks and grants kept: the oldest go first.
const MAX_KEPT: usize = 64;

/// The request an agent's pane is waiting on: who asked, for which site and what.
struct Ask {
    request_id: String,
    uid: String,
    agent: String,
    target: String,
    site: String,
    kinds: Vec<&'static str>,
}

struct Grant {
    agent: String,
    target: String,
    site: String,
    kinds: Vec<&'static str>,
    until_ms: i64,
}

static ASKS: Mutex<Vec<Ask>> = Mutex::new(Vec::new());
static GRANTS: Mutex<Vec<Grant>> = Mutex::new(Vec::new());

fn asks() -> std::sync::MutexGuard<'static, Vec<Ask>> {
    ASKS.lock().unwrap_or_else(|e| e.into_inner())
}

fn grants() -> std::sync::MutexGuard<'static, Vec<Grant>> {
    GRANTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the page asked for, in words. `prompt` says which list `requested` is from: a permission prompt
/// (location, notifications) or a media request (camera, microphone). Anything else is refused with why.
fn kinds_of(requested: u32, prompt: bool) -> Result<Vec<&'static str>, &'static str> {
    let (table, refusal): (&[(u32, &'static str)], &str) = if prompt {
        (
            &[(PROMPT_GEOLOCATION, "location"), (PROMPT_NOTIFICATIONS, "notifications")],
            "the page asked for a permission an agent's pane is never given",
        )
    } else {
        (
            &[(DEVICE_VIDEO_CAPTURE, "camera"), (DEVICE_AUDIO_CAPTURE, "microphone")],
            "the page asked to capture the screen or the computer's sound, which an agent's pane is never allowed",
        )
    };
    if requested & !table.iter().fold(0, |all, (bit, _)| all | bit) != 0 {
        return Err(refusal);
    }
    let kinds: Vec<&'static str> = table.iter().filter(|(bit, _)| requested & bit != 0).map(|(_, name)| *name).collect();
    if kinds.is_empty() {
        return Err("the page asked for nothing AgentMux can name");
    }
    Ok(kinds)
}

/// The kinds as the thing used, for a sentence: "the camera and the microphone", "notifications".
fn what_of(kinds: &[&str]) -> String {
    let named: Vec<&str> = kinds
        .iter()
        .map(|k| match *k {
            "camera" => "the camera",
            "microphone" => "the microphone",
            "location" => "the computer's location",
            _ => "notifications",
        })
        .collect();
    named.join(" and ")
}

fn covers(g: &Grant, agent: &str, target: &str, site: &str, kinds: &[&str], now: i64) -> bool {
    g.until_ms > now && g.agent == agent && g.target == target && g.site == site && kinds.iter().all(|k| g.kinds.contains(k))
}

fn is_granted(agent: &str, target: &str, site: &str, kinds: &[&str], now: i64) -> bool {
    grants().iter().any(|g| covers(g, agent, target, site, kinds, now))
}

/// A remembered grant covered this ask, with no fresh countdown: one Audit row for it, flagged
/// `standing` (§4.6) so it reads the same as a fresh countdown's row rather than vanishing into
/// the per-pane notice log alone — a site allowed many times unattended should still show up.
fn audit_standing_allow(state: &AppState, agent: &str, site: &str, kinds: &[&str], message: &str) {
    let facts = facts_for(agent, site, kinds);
    state.reactive_handler.log_sensitive_op_audit(
        Some(agent),
        agent,
        "os_prompt_answer.standing",
        true,
        None,
        &uuid::Uuid::new_v4().to_string(),
        Some(message),
        facts.get("prompt").map(String::as_str),
        facts.get("answer").map(String::as_str),
        facts.get("level").map(String::as_str),
        true,
    );
}

/// The open request this agent already filed for this site and these kinds, if it still waits.
fn open_ask(agent: &str, target: &str, site: &str, kinds: &[&str]) -> Option<String> {
    asks()
        .iter()
        .filter(|a| a.agent == agent && a.target == target && a.site == site && a.kinds == kinds)
        .map(|a| a.request_id.clone())
        .find(|id| crate::backend::sensitive_requests::registry().get(id).is_some_and(|r| r.status.is_open()))
}

fn remember(ask: Ask) {
    let mut a = asks();
    // A request that ended without running (denied, vetoed, withdrawn) leaves no use for its entry.
    a.retain(|x| crate::backend::sensitive_requests::registry().get(&x.request_id).is_some_and(|r| r.status.is_open()));
    if a.len() >= MAX_KEPT {
        a.remove(0);
    }
    a.push(ask);
}

/// The facts of the request, from what AgentMux saw: only the site is the page's, cleaned as every fact is.
fn facts_for(agent: &str, site: &str, kinds: &[&str]) -> BTreeMap<String, String> {
    let what = what_of(kinds);
    BTreeMap::from([
        ("agent".to_string(), agent.to_string()),
        ("app".to_string(), "Browser".to_string()),
        ("window".to_string(), site.to_string()),
        ("prompt".to_string(), format!("{site} wants to use {what}")),
        ("answer".to_string(), format!("allow {what} for {site} for 30 minutes")),
        ("level".to_string(), "medium".to_string()),
        ("reason".to_string(), "the page asked while the agent was driving this pane".to_string()),
    ])
}

/// What the agent is told when its pane's page asked and the person is being asked.
fn asked_reason(site: &str, what: &str) -> String {
    format!(
        "{site} asked to use {what}. AgentMux denied it for now and filed it as a sensitive operation: the super agent \
         reviews it and the person can veto its countdown. An [AgentMux] note brings the answer; then ask the page again"
    )
}

/// `POST /api/v1/host/browser_permission` — the host holds a page's camera, microphone, location or
/// notification request in a pane an agent drives and asks what to do. Body `{target, origin, requested,
/// prompt}` (`prompt`: the bits are a permission prompt's, not a media request's); answers `allow`, or
/// `deny` (with the agent told why in its next `Browser` call). Host only.
pub(crate) async fn handle_host_browser_permission(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can report a page's permission request".to_string());
    }
    let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let target = s("target");
    let requested = body.get("requested").and_then(Value::as_u64).unwrap_or(0) as u32;
    let (banner_at, _window) = banner_for(&target);
    let agent = crate::server::browser_owner::owner_of(&banner_at).unwrap_or_default();
    let answered = |decision: &str| (StatusCode::OK, Json(json!({ "ok": true, "data": { "decision": decision } }))).into_response();
    let origin = s("origin");
    let site = agentmux_common::allowed_origins::target_origin(&origin).unwrap_or(origin);
    let kinds = match kinds_of(requested, body.get("prompt").and_then(Value::as_bool).unwrap_or(false)) {
        Ok(k) => k,
        Err(why) => {
            record(&agent, &target, "permission", &format!("{site} asked for a permission"), "denied", why);
            return answered("deny");
        }
    };
    let what = what_of(&kinds);
    let message = format!("{site} asked to use {what}");
    if agent.is_empty() || site.is_empty() {
        return answered("deny");
    }
    if is_granted(&agent, &target, &site, &kinds, now_ms()) {
        record(&agent, &target, "permission", &message, "allowed", "the person's countdown for this site ended without a veto");
        audit_standing_allow(&state, &agent, &site, &kinds, &message);
        return answered("allow");
    }
    if open_ask(&agent, &target, &site, &kinds).is_some() {
        record(&agent, &target, "permission", &message, "denied", "it is already asked for; the answer comes as an [AgentMux] note");
        return answered("deny");
    }
    let uid = crate::backend::reactive::get_global_handler().get_agent(&agent).and_then(|r| r.uid).unwrap_or_default();
    if uid.is_empty() {
        record(&agent, &target, "permission", &message, "denied", "this agent has no token to ask with");
        return answered("deny");
    }
    let facts = facts_for(&agent, &site, &kinds);
    let reason = format!("let {site} use {what} in the pane {agent} is driving");
    match crate::server::sensitive_handlers::file_request(&state, &uid, &agent, "os_prompt_answer", &facts, &reason).await {
        Ok(filed) => {
            let request_id = filed["id"].as_str().unwrap_or_default().to_string();
            remember(Ask { request_id, uid, agent: agent.clone(), target: target.clone(), site: site.clone(), kinds });
            record(&agent, &target, "permission", &message, "denied for now", &asked_reason(&site, &what));
        }
        Err(e) => record(&agent, &target, "permission", &message, "denied", &format!("couldn't ask: {e}")),
    }
    answered("deny")
}

/// What the countdown ending unvetoed does: the site may use what it asked for for a while,
/// for each agent that still asks.
pub(crate) fn grant_after_countdown(_state: &AppState, request_id: &str) -> Result<(), String> {
    // One lock for the take and the store-back, so an ask filed meanwhile is not overwritten.
    let mine: Vec<Ask> = {
        let mut a = asks();
        let (mine, rest): (Vec<Ask>, Vec<Ask>) = std::mem::take(&mut *a).into_iter().partition(|x| x.request_id == request_id);
        *a = rest;
        mine
    };
    if mine.is_empty() {
        return Err("the permission this request was for is no longer known; ask the page again".to_string());
    }
    let request = crate::backend::sensitive_requests::registry().get(request_id);
    let askers = request.as_ref().map(|r| r.askers()).unwrap_or_default();
    let until_ms = now_ms() + GRANT_MS;
    let mut any = false;
    for a in mine.into_iter().filter(|a| askers.contains(&a.uid.as_str())) {
        let mut g = grants();
        g.retain(|x| x.until_ms > now_ms());
        if g.len() >= MAX_KEPT {
            g.remove(0);
        }
        g.push(Grant { agent: a.agent, target: a.target, site: a.site, kinds: a.kinds, until_ms });
        any = true;
    }
    if any {
        Ok(())
    } else {
        Err("everyone who asked has withdrawn".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_camera_or_microphone_is_named_and_anything_else_is_refused() {
        assert_eq!(kinds_of(DEVICE_VIDEO_CAPTURE, false), Ok(vec!["camera"]));
        assert_eq!(kinds_of(DEVICE_AUDIO_CAPTURE | DEVICE_VIDEO_CAPTURE, false), Ok(vec!["camera", "microphone"]));
        assert!(kinds_of(4, false).unwrap_err().contains("screen"));
        assert!(kinds_of(DEVICE_VIDEO_CAPTURE | 8, false).is_err(), "a screen share with the camera is still a screen share");
        assert!(kinds_of(0, false).is_err());
    }

    #[test]
    fn a_location_or_notification_prompt_is_named_and_the_media_bits_are_not_its_bits() {
        assert_eq!(kinds_of(PROMPT_GEOLOCATION, true), Ok(vec!["location"]));
        assert_eq!(kinds_of(PROMPT_GEOLOCATION | PROMPT_NOTIFICATIONS, true), Ok(vec!["location", "notifications"]));
        assert!(kinds_of(1, true).is_err(), "bit 1 is a camera in a media request, nothing in a prompt");
        assert!(kinds_of(PROMPT_NOTIFICATIONS | (1 << 14), true).is_err(), "something else with it is refused");
        assert!(kinds_of(PROMPT_GEOLOCATION, false).is_err(), "a prompt's bits are not a media request's");
        assert_eq!(what_of(&["camera", "microphone"]), "the camera and the microphone");
        assert_eq!(what_of(&["location"]), "the computer's location");
        assert_eq!(what_of(&["notifications"]), "notifications");
    }

    #[test]
    fn the_facts_fit_the_operation_and_say_who_wrote_what() {
        let spec = crate::backend::sensitive_ops::spec("os_prompt_answer").unwrap();
        let facts = facts_for("posa", "https://meet.example", &["camera", "microphone"]);
        let checked = spec.check_facts(&facts).unwrap();
        assert_eq!(checked["app"], "Browser");
        assert_eq!(checked["level"], "medium");
        assert!(checked["prompt"].contains("https://meet.example") && checked["answer"].contains("the camera and the microphone"));
        let summary = spec.summary(&checked);
        assert!(summary.starts_with("answer a prompt for an agent (") && summary.contains("meet.example"), "{summary}");
    }

    #[test]
    fn a_grant_covers_its_agent_pane_site_and_kinds_until_it_runs_out() {
        let g = Grant { agent: "a".into(), target: "p".into(), site: "https://s".into(), kinds: vec!["camera", "microphone"], until_ms: 1000 };
        assert!(covers(&g, "a", "p", "https://s", &["camera"], 999));
        assert!(covers(&g, "a", "p", "https://s", &["camera", "microphone"], 999));
        assert!(!covers(&g, "a", "p", "https://s", &["camera"], 1000), "expired");
        assert!(!covers(&g, "b", "p", "https://s", &["camera"], 10), "another agent");
        assert!(!covers(&g, "a", "q", "https://s", &["camera"], 10), "another pane");
        assert!(!covers(&g, "a", "p", "https://other", &["camera"], 10), "another site");
        let cam = Grant { kinds: vec!["camera"], ..g };
        assert!(!covers(&cam, "a", "p", "https://s", &["camera", "microphone"], 10), "a camera grant is not a microphone grant");
    }

    #[tokio::test]
    async fn the_countdown_ending_grants_only_what_was_asked_and_only_once() {
        let state = crate::server::tests::test_state();
        let uid = format!("u-bp-{}", uuid::Uuid::new_v4().simple());
        let site = format!("https://{}.example", uuid::Uuid::new_v4().simple());
        let facts = facts_for("bp-agent", &site, &["camera"]);
        let id = crate::backend::sensitive_requests::registry().request(&uid, "bp-agent", "os_prompt_answer", &facts, "", now_ms()).unwrap().id;
        remember(Ask { request_id: id.clone(), uid: uid.clone(), agent: "bp-agent".into(), target: "pane-1".into(), site: site.clone(), kinds: vec!["camera"] });
        assert!(open_ask("bp-agent", "pane-1", &site, &["camera"]).is_some());
        assert!(!is_granted("bp-agent", "pane-1", &site, &["camera"], now_ms()), "nothing is allowed before the countdown ends");
        grant_after_countdown(&state, &id).unwrap();
        assert!(is_granted("bp-agent", "pane-1", &site, &["camera"], now_ms()));
        assert!(!is_granted("bp-agent", "pane-1", &site, &["microphone"], now_ms()), "the microphone was not asked for");
        assert!(grant_after_countdown(&state, &id).is_err(), "a second run finds nothing to grant");
    }

    #[tokio::test]
    async fn a_standing_grant_writes_one_audit_row_flagged_standing() {
        let state = crate::server::tests::test_state();
        let before = state.reactive_handler.get_audit_log(200).len();
        audit_standing_allow(&state, "bp-agent-2", "https://standing.example", &["camera", "microphone"], "https://standing.example asked to use the camera and the microphone");
        let log = state.reactive_handler.get_audit_log(200);
        assert_eq!(log.len(), before + 1, "exactly one row");
        let row = &log[0];
        assert_eq!(row.standing, Some(true));
        assert!(row.prompt.as_deref().unwrap().contains("standing.example"));
        assert!(row.answer.as_deref().unwrap().contains("camera") && row.answer.as_deref().unwrap().contains("microphone"));
        assert_eq!(row.level.as_deref(), Some("medium"));
    }
}
