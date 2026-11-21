// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each browser's page is doing, for `muxspect browsers`
//! (docs/specs/SPEC_RELEASE_BUILD_BROWSER_PAGE_DIAGNOSTIC_2026_11_13.md §3.2, §3.3).
//!
//! A release build has no DevTools port, and an OS pid does not say which page a renderer draws.
//! This route answers, for every browser the host holds: its role, whether Chromium thinks the
//! page is visible and focused, how many animations run, and, on request, the page's main-thread
//! time counters. It runs two fixed expressions chosen by what the browser is and takes nothing
//! from the caller: it is a diagnostic, not a second `eval`.
//!
//! - An app window or a pool window is AgentMux's own page: its title and the names of its running
//!   animations are part of the answer.
//! - A browser pane shows a person's page: only visibility, focus and the number of running
//!   animations leave the page. Its origin comes from the browser's address, never a path or query.
//!
//! Never wakes a page: a pane the browser discarded or the window froze is listed as such and not
//! asked (any CDP call would thaw a frozen page). Never hangs on one: each page has
//! [`PROBE_TIMEOUT`], all are asked at once, and `Performance.disable` is sent from the cleanup
//! path when the read did not finish. Writes nothing and logs nothing.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{call, fail, isolated_context, unauthorized};
use super::cdp::{is_frozen, CdpSession};
use super::types::ApiResponse;
use crate::browser_panes::discard::DiscardOp;
use crate::renderer_map::BrowserKind;
use crate::state::AppState;

/// How long one page may take to answer. A renderer wedged in script is the thing being
/// diagnosed, so a short wait is itself a finding; the CDP client's own 10 s is for actions.
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// AgentMux's own pages: what the page says about itself is part of the answer.
const APP_EXPRESSION: &str = r#"(() => {
  const running = document.getAnimations().filter((a) => a.playState === "running");
  const name = (a) => a.animationName ?? a.transitionProperty ?? "";
  return {
    visibility: document.visibilityState,
    hasFocus: document.hasFocus(),
    animations: running.length,
    kinds: [...new Set(running.map((a) => a.constructor.name + (name(a) ? ":" + name(a) : "")))].slice(0, 8),
    title: document.title.slice(0, 120),
  };
})()"#;

/// A person's page: three facts and nothing it shows.
const PANE_EXPRESSION: &str = r#"(() => ({
  visibility: document.visibilityState,
  hasFocus: document.hasFocus(),
  animations: document.getAnimations().filter((a) => a.playState === "running").length,
}))()"#;

/// The counters worth showing for a busy page (seconds of main-thread time since it started).
const METRICS: [&str; 5] = ["Timestamp", "TaskDuration", "ScriptDuration", "LayoutDuration", "RecalcStyleDuration"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    AppWindow,
    PoolWindow,
    BrowserPane,
}

/// How the page's row was filled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// The page answered.
    Answered,
    /// The browser discarded the page: it has no document and no renderer to ask.
    Asleep,
    /// The window froze the page; asking would thaw it.
    Frozen,
    /// No answer within [`PROBE_TIMEOUT`]: what a wedged renderer looks like.
    NoAnswer,
    /// The page answered with an error.
    Failed,
}

/// The page's own facts. A browser pane's carry no `kinds` and no `title`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Facts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_focus: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animations: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageProbe {
    pub label: String,
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    /// A pane's page origin (scheme, host, port); empty for AgentMux's own pages.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub origin: String,
    pub answer: Answer,
    #[serde(flatten)]
    pub facts: Facts,
    /// Main-thread counters, when asked for (`detail`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<BTreeMap<String, f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProbeReq {
    /// Also read the page's main-thread time counters (`Performance.getMetrics`).
    #[serde(default)]
    pub detail: bool,
}

/// What a browser is.
pub fn role_of(kind: &BrowserKind) -> Role {
    if kind.block_id.is_some() {
        Role::BrowserPane
    } else if kind.pool {
        Role::PoolWindow
    } else {
        Role::AppWindow
    }
}

fn expression_for(role: Role) -> &'static str {
    match role {
        Role::BrowserPane => PANE_EXPRESSION,
        Role::AppWindow | Role::PoolWindow => APP_EXPRESSION,
    }
}

/// The facts out of what the page returned. Only the fields this role may carry are read, so a page
/// that answers with more (or a pane that answers with a title) has the rest dropped here.
pub fn facts_from(role: Role, value: &Value) -> Facts {
    let visibility = value
        .get("visibility")
        .and_then(Value::as_str)
        .filter(|v| matches!(*v, "visible" | "hidden" | "prerender"))
        .map(str::to_string);
    let mut facts = Facts {
        visibility,
        has_focus: value.get("hasFocus").and_then(Value::as_bool),
        animations: value.get("animations").and_then(Value::as_u64).map(|n| n.min(9999)),
        ..Facts::default()
    };
    if role != Role::BrowserPane {
        facts.kinds = value
            .get("kinds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).take(8).map(|k| k.chars().take(80).collect()).collect())
            .unwrap_or_default();
        facts.title = value.get("title").and_then(Value::as_str).map(|t| t.chars().take(120).collect());
    }
    facts
}

/// Why a page is not asked, when it is not: a discarded page has nothing to ask, and any call on a
/// frozen one would thaw it.
pub fn skip(discarded: bool, frozen: bool) -> Option<Answer> {
    if discarded {
        Some(Answer::Asleep)
    } else if frozen {
        Some(Answer::Frozen)
    } else {
        None
    }
}

/// The counters in a `Performance.getMetrics` answer that the diagnostic shows.
pub fn metrics_from(reply: &Value) -> BTreeMap<String, f64> {
    reply
        .get("metrics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m.get("name")?.as_str()?, m.get("value")?.as_f64()?)))
        .filter(|(name, _)| METRICS.contains(name))
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

/// One browser's address as an origin: scheme, host and port, never a path or query. Empty for a
/// page that is not http(s) (AgentMux's own, `about:blank`).
fn origin_of_browser(state: &AppState, label: &str) -> String {
    use cef::*;
    let Some(browser) = state.get_browser(label) else { return String::new() };
    let Some(frame) = browser.main_frame() else { return String::new() };
    agentmux_common::popup_rules::origin_of(&CefString::from(&frame.url()).to_string())
}

/// The page's facts, and its counters when `detail`.
async fn read_page(state: &Arc<AppState>, label: &str, role: Role, detail: bool) -> Result<(Facts, Option<BTreeMap<String, f64>>), String> {
    let mut cdp = CdpSession::attach(state, label);
    let ctx = isolated_context(&mut cdp, None).await?;
    let reply = call(
        &mut cdp,
        "Runtime.evaluate",
        json!({ "expression": expression_for(role), "contextId": ctx, "returnByValue": true }),
    )
    .await?;
    if reply.get("exceptionDetails").is_some() {
        return Err("the probe threw in the page".to_string());
    }
    let facts = facts_from(role, reply.pointer("/result/value").unwrap_or(&Value::Null));
    let metrics = if detail { Some(read_metrics(&mut cdp).await?) } else { None };
    Ok((facts, metrics))
}

/// `Performance.enable`, one read, `Performance.disable` (also when the read failed).
async fn read_metrics(cdp: &mut CdpSession) -> Result<BTreeMap<String, f64>, String> {
    let got = match call(cdp, "Performance.enable", json!({})).await {
        Ok(_) => call(cdp, "Performance.getMetrics", json!({})).await,
        Err(e) => Err(e),
    };
    let _ = call(cdp, "Performance.disable", json!({})).await;
    got.map(|r| metrics_from(&r))
}

/// A read that did not finish may have left the page's Performance domain on: turn it off from
/// here without waiting (a wedged page would hold the call for the CDP timeout).
fn disable_performance_later(state: &Arc<AppState>, label: &str) {
    let (state, label) = (state.clone(), label.to_string());
    tokio::spawn(async move {
        let mut cdp = CdpSession::attach(&state, &label);
        let _ = call(&mut cdp, "Performance.disable", json!({})).await;
    });
}

async fn probe_one(state: Arc<AppState>, kind: BrowserKind, detail: bool) -> PageProbe {
    let role = role_of(&kind);
    let origin = if role == Role::BrowserPane { origin_of_browser(&state, &kind.label) } else { String::new() };
    let mut row = PageProbe {
        label: kind.label.clone(),
        role,
        block_id: kind.block_id.clone(),
        window_id: kind.window_id.clone(),
        origin,
        answer: Answer::Answered,
        facts: Facts::default(),
        metrics: None,
        error: None,
    };
    let discarded = match &kind.block_id {
        Some(block_id) => state.browser_panes.discard_op(&state, block_id, DiscardOp::IsDiscarded).await == Some(true),
        None => false,
    };
    if let Some(answer) = skip(discarded, is_frozen(kind.browser_id)) {
        row.answer = answer;
        return row;
    }
    match tokio::time::timeout(PROBE_TIMEOUT, read_page(&state, &kind.label, role, detail)).await {
        Ok(Ok((facts, metrics))) => {
            row.facts = facts;
            row.metrics = metrics;
        }
        Ok(Err(e)) => {
            row.answer = Answer::Failed;
            row.error = Some(e);
        }
        Err(_) => row.answer = Answer::NoAnswer,
    }
    if detail && row.answer != Answer::Answered {
        disable_performance_later(&state, &kind.label);
    }
    row
}

/// `POST /agentmux/browser/page_probe` — `{ browsers: [PageProbe] }`, one per browser the host holds.
pub async fn page_probe_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ProbeReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut set = tokio::task::JoinSet::new();
    for kind in crate::renderer_map::browser_kinds(&state) {
        set.spawn(probe_one(state.clone(), kind, req.detail));
    }
    let mut rows = Vec::new();
    while let Some(done) = set.join_next().await {
        match done {
            Ok(row) => rows.push(row),
            Err(e) => return fail(format!("the page probe failed: {e}")),
        }
    }
    rows.sort_by(|a, b| a.label.cmp(&b.label));
    (StatusCode::OK, Json(ApiResponse::ok(json!({ "browsers": rows }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(block_id: Option<&str>, pool: bool) -> BrowserKind {
        BrowserKind { label: "x".into(), browser_id: 1, block_id: block_id.map(Into::into), pool, window_id: None }
    }

    #[test]
    fn a_pane_is_a_pane_before_it_is_anything_else() {
        assert_eq!(role_of(&kind(Some("b1"), false)), Role::BrowserPane);
        assert_eq!(role_of(&kind(None, true)), Role::PoolWindow);
        assert_eq!(role_of(&kind(None, false)), Role::AppWindow);
    }

    #[test]
    fn a_pane_gets_the_expression_that_reads_no_text() {
        assert!(!expression_for(Role::BrowserPane).contains("title"));
        assert!(!expression_for(Role::BrowserPane).contains("animationName"));
        assert!(expression_for(Role::AppWindow).contains("document.title"));
        assert_eq!(expression_for(Role::PoolWindow), expression_for(Role::AppWindow));
    }

    #[test]
    fn a_pane_answer_keeps_three_facts_whatever_the_page_returned() {
        let page = json!({
            "visibility": "hidden", "hasFocus": false, "animations": 3,
            "title": "Bank statement", "kinds": ["CSSAnimation:spin"], "url": "https://x.test/secret",
        });
        let f = facts_from(Role::BrowserPane, &page);
        assert_eq!(f, Facts { visibility: Some("hidden".into()), has_focus: Some(false), animations: Some(3), kinds: vec![], title: None });
        let wire = serde_json::to_string(&f).unwrap();
        assert!(!wire.contains("Bank") && !wire.contains("spin") && !wire.contains("secret"), "{wire}");
    }

    #[test]
    fn an_app_window_answer_carries_its_title_and_animation_names() {
        let page = json!({ "visibility": "visible", "hasFocus": true, "animations": 2, "title": "AgentMux (pre-warmed window)", "kinds": ["CSSAnimation:startup-spin", 4] });
        let f = facts_from(Role::PoolWindow, &page);
        assert_eq!(f.title.as_deref(), Some("AgentMux (pre-warmed window)"));
        assert_eq!(f.kinds, vec!["CSSAnimation:startup-spin".to_string()]);
    }

    #[test]
    fn a_visibility_the_browser_does_not_have_is_not_passed_on() {
        assert_eq!(facts_from(Role::BrowserPane, &json!({ "visibility": "<script>" })).visibility, None);
        assert_eq!(facts_from(Role::BrowserPane, &Value::Null), Facts::default());
        assert_eq!(facts_from(Role::BrowserPane, &json!({ "animations": 1_000_000 })).animations, Some(9999));
    }

    #[test]
    fn a_discarded_or_frozen_page_is_not_asked() {
        assert_eq!(skip(true, false), Some(Answer::Asleep));
        assert_eq!(skip(false, true), Some(Answer::Frozen));
        assert_eq!(skip(true, true), Some(Answer::Asleep));
        assert_eq!(skip(false, false), None);
    }

    #[test]
    fn only_the_counters_the_diagnostic_shows_are_kept() {
        let reply = json!({ "metrics": [
            { "name": "Timestamp", "value": 100.5 },
            { "name": "TaskDuration", "value": 2.25 },
            { "name": "JSHeapUsedSize", "value": 1.0e8 },
            { "name": "LayoutDuration", "value": "n/a" },
        ] });
        let m = metrics_from(&reply);
        assert_eq!(m.len(), 2);
        assert_eq!(m["TaskDuration"], 2.25);
        assert!(metrics_from(&json!({})).is_empty());
    }

    #[test]
    fn a_row_serialises_flat_and_leaves_out_what_it_has_not() {
        let row = PageProbe {
            label: "browser-pane-a-1".into(),
            role: Role::BrowserPane,
            block_id: Some("b1".into()),
            window_id: None,
            origin: "https://dash.example.com".into(),
            answer: Answer::Asleep,
            facts: Facts::default(),
            metrics: None,
            error: None,
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["role"], "browser-pane");
        assert_eq!(v["answer"], "asleep");
        assert!(v.get("visibility").is_none() && v.get("metrics").is_none() && v.get("window_id").is_none());
    }
}
