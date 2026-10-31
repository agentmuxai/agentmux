// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sign-in link as a notice on the machine the person is using (spec
//! `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` §4.4, D6). An agent that started a sign-in on
//! another machine (`identity.signin.start`) asks its own srv to put the link in front of the
//! person: here if a window of this AgentMux is focused, else on the LAN peer that reports one.
//! The notice is a persistent toast with an Open action; the credentials stay on the machine that
//! is signing in, and only the link travels.

use axum::extract::{Extension, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::*;

/// The `identity:signinlink` event: show this link as a notice.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SigninLinkEvent {
    pub provider: String,
    /// The machine that is signing in: "Sign narko in to Claude".
    pub machine: String,
    pub url: String,
    /// The one-time code to enter on that page, for a sign-in that uses one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
}

/// Where a provider's sign-in page may be: an Open action opens a link a remote machine named,
/// so only the provider's own https pages (and their subdomains) are accepted.
fn allowed_hosts(provider: &str) -> &'static [&'static str] {
    match provider {
        "claude" => &["claude.com", "claude.ai", "anthropic.com"],
        "codex" => &["openai.com", "chatgpt.com"],
        _ => &[],
    }
}

pub(crate) fn link_allowed(provider: &str, link: &str) -> bool {
    let Ok(url) = url::Url::parse(link) else { return false };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else { return false };
    allowed_hosts(provider).iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// A machine name fit for a toast: printable, short.
fn clean_machine(machine: &str) -> String {
    machine.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string()
}

/// A one-time code fit for a toast: letters, digits and dashes, short.
fn clean_code(code: Option<&str>) -> Option<String> {
    let code = code?.trim();
    (!code.is_empty() && code.len() <= 32 && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then(|| code.to_string())
}

/// Whether a window of this AgentMux is focused now.
pub(crate) fn window_focused(state: &AppState) -> bool {
    crate::backend::notify::router::get(&state.broker).is_some_and(|r| r.app_focused())
}

/// Show the link as a notice on this machine.
pub(crate) fn show_local(state: &AppState, provider: &str, machine: &str, link: &str, code: Option<&str>) -> Result<(), String> {
    if !link_allowed(provider, link) {
        return Err(format!("not a {provider} sign-in link"));
    }
    let event = SigninLinkEvent { provider: provider.to_string(), machine: clean_machine(machine), url: link.to_string(), code: clean_code(code) };
    state.broker.publish_event::<crate::backend::mps_events::SigninLink>(&event, vec![], 0);
    Ok(())
}

/// Put the link in front of the person: on this machine when a window here is focused, else on the
/// first LAN peer that has one focused and lets this agent ask (an approved install), else here.
/// Returns the machine that got it.
pub(crate) async fn route(state: &AppState, slug: &str, provider: &str, machine: &str, link: &str, code: Option<&str>) -> Result<String, String> {
    if !link_allowed(provider, link) {
        return Err(format!("not a {provider} sign-in link"));
    }
    if !window_focused(state) {
        let mut peers: Vec<String> = state.lan_discovery.get_instances().iter().map(|p| p.hostname.trim().to_string()).filter(|h| !h.is_empty()).collect();
        peers.sort();
        peers.dedup();
        for peer in peers {
            let asked = super::remote_ops::send::call(state, slug, &peer, "app.activity", json!({}), "where is the person").await;
            if asked.result["window_focused"].as_bool() != Some(true) {
                continue;
            }
            let params = json!({ "provider": provider, "machine": machine, "url": link, "code": code });
            let shown = super::remote_ops::send::call(state, slug, &peer, "identity.signin.notice", params, "show the sign-in link").await;
            if shown.result["shown"].as_bool() == Some(true) {
                return Ok(peer);
            }
        }
    }
    show_local(state, provider, machine, link, code)?;
    Ok(state.hostname.clone())
}

#[derive(Deserialize)]
pub(super) struct SigninNoticeRequest {
    agent_id: String,
    provider: String,
    /// The machine that is signing in.
    machine: String,
    url: String,
    #[serde(default)]
    code: Option<String>,
}

/// `POST /api/v1/agent/identity/signin/notice`: backs `Identity` `signin` with a `host`, once the
/// other machine has returned the link.
pub(super) async fn handle_agent_signin_notice(
    State(state): State<AppState>,
    who: Option<Extension<caller::Caller>>,
    Json(req): Json<SigninNoticeRequest>,
) -> impl IntoResponse {
    let (slug, _) = match actor::require_token_actor(&state, who.as_deref(), actor::ActorSite::IdentitySigninNotice, &req.agent_id).await {
        Ok(who) => who,
        Err(e) => return app_api_response::<Value>(Err(e)),
    };
    let shown_on = route(&state, &slug, &req.provider, &req.machine, &req.url, req.code.as_deref()).await;
    app_api_response(shown_on.map(|machine| json!({ "shown_on": machine })))
}

#[cfg(test)]
#[path = "signin_notice_tests.rs"]
mod tests;
