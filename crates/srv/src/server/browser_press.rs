// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Enter and Space on a granted site: as the click they are
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §5).
//!
//! Enter and Space skip the classification that keeps an Auto-approve grant's
//! promises, so `key` refuses them on a pane an agent drives. A grant that
//! carries the Enter and Space power lifts that refusal for the sites it names,
//! and then the key is never sent: the host says what it would do (a form's
//! default button, a button, a link, a checkbox) and makes that click through
//! `act`'s check. A payment, a form that sends to another site and a click the
//! page changed after approval ask or stop exactly as for `click`. A key whose
//! click the host can't name is refused.

use axum::response::Response;
use serde_json::{json, Value};

use super::browser_actions::Note;
use super::browser_bypass as bypass;
use super::browser_powers::Power;
use super::ui_handlers::{banner_for, proxy_value};
use super::AppState;

/// What became of a `key` call with Enter or Space.
pub(crate) enum Pressed {
    /// No grant gives this agent the power on this page: refuse as before.
    NotGranted,
    /// Answered: the click was made, or refused.
    Done(Response),
}

/// The grant that gives `agent` the power `power` on `pane`, which also covers the page.
/// The person's standing grant for the agent on the page's site makes one when
/// the pane has none.
pub(crate) fn grant(state: &AppState, pane: &str, agent: &str, page: &str, power: Power) -> Option<bypass::Grant> {
    let has = |g: &bypass::Grant| g.allows(power) && g.covers(page);
    bypass::active_for(pane, agent)
        .filter(has)
        .or_else(|| super::browser_standing::activate_at(state, pane, agent, page).filter(has))
}

/// `key` (`Enter`, `NumpadEnter` or `Space`) on `block_id`, if the agent holds the
/// power there. `selector` is focused first, as `key` does for any key.
pub(crate) async fn press(state: &AppState, block_id: &str, agent: &str, selector: Option<&str>, key: &str, log: &impl Fn() -> Note) -> Pressed {
    let pane = banner_for(block_id).0;
    let Ok(page) = super::browser_bypass_routes::page_url(state, block_id).await else { return Pressed::NotGranted };
    let Some(g) = grant(state, &pane, agent, &page, Power::EnterSpace) else { return Pressed::NotGranted };
    let host_key = if key == "Space" { "Space" } else { "Enter" };
    let timeout = std::time::Duration::from_secs(20);
    if let Err((code, e)) = super::browser_actions::not_paused(block_id) {
        return Pressed::Done(log().fail(state, code, e));
    }
    if let Some(selector) = selector {
        if let Err((code, e)) = proxy_value(state, "focus_element", json!({ "block_id": block_id, "selector": selector }), timeout).await {
            return Pressed::Done(log().fail(state, code, e));
        }
    }
    // `approved` carries what the person was shown: the host makes the click only if the page still shows it.
    let body = |approved: Option<&Value>| {
        json!({ "block_id": block_id, "key": host_key, "allowed_origins": g.origins, "approved": approved.is_some(), "approved_as": approved })
    };
    let first = match proxy_value(state, "press", body(None), timeout).await {
        Ok(d) => d,
        Err((code, e)) => return Pressed::Done(log().fail(state, code, e)),
    };
    let Some(ask) = first.pointer("/after/needs_approval").cloned() else {
        return Pressed::Done(log().outcome(state, Ok(first)));
    };
    if let Some(resp) = super::browser_bypass_routes::bypassed_click(state, block_id, agent, &ask, body(Some(&ask)), "press", log()).await {
        return Pressed::Done(resp);
    }
    Pressed::Done(super::browser_click_approval::ask_then_click(state, block_id, agent, "press", &ask, body(Some(&ask)), log).await)
}

/// `key` with Enter or Space (or `text` with a line break) on a pane an agent drives: taken as the
/// click it is when a grant gives the power, otherwise refused with the reason and the way
/// forward. `None`: carry on with the key (not a driven pane).
pub(crate) async fn press_or_refuse(
    state: &AppState,
    block_id: &str,
    req: &agentmux_common::api_types::UiBrowserDispatchKeyRequest,
    log: impl Fn() -> Note,
) -> Option<Response> {
    if let (None, Some(key)) = (req.text.as_deref(), req.key.as_deref()) {
        match press(state, block_id, &req.auth.agent_id, req.selector.as_deref(), key, &log).await {
            Pressed::Done(resp) => return Some(resp),
            Pressed::NotGranted => {}
        }
    }
    super::ui_handlers::refuse_on_driven_pane(
        state,
        log(),
        block_id,
        "Browser's key action with Enter or Space",
        "Browser's click action on the button (it asks the user when the click submits something), or ask for the enter_space power with request_auto_approve",
    )
    .err()
}
