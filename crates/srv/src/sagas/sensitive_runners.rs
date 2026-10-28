// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What runs each sensitive operation when its countdown ends unvetoed
//! (`SPEC_SUPER_AGENT_2026_10_25.md` §4, §5). Each runner calls the operation's own code path
//! unchanged: no power is added, only who may start it.

use std::sync::Arc;

use crate::backend::sensitive_requests::Request;
use crate::backend::upgrade_request::{self, Outcome};
use crate::sagas::sensitive_window as window;
use crate::server::AppState;

/// Say what runs each operation that has a runner. Idempotent: every connection calls it.
pub fn register_all() {
    window::register_default_runner("upgrade", Arc::new(|state, request| Box::pin(upgrade(state, request))));
    window::register_default_runner("widget_install", Arc::new(|state, request| Box::pin(widget_install(state, request))));
}

/// Why a release can't be installed, as the requester is told.
fn why_not(outcome: &Outcome) -> String {
    match outcome {
        Outcome::UpToDate { current } => format!("AgentMux {current} is already the latest release"),
        Outcome::Unavailable { message, .. } => message.clone(),
        _ => "that release can't be installed now".to_string(),
    }
}

/// Start the upgrade the request named, if it is still the release to install. The agents that
/// asked hear if it stops short, as they do when the person's own turn starts one.
async fn upgrade(state: AppState, request: Request) -> Result<(), String> {
    let wanted = request.facts.get("version").cloned().unwrap_or_default();
    let status = crate::backend::update_check::check_now().await.unwrap_or_else(|_| crate::backend::update_check::current_status());
    let version = upgrade_request::installable(&status, Some(&wanted)).map_err(|o| why_not(&o))?;
    let handler = crate::backend::reactive::get_global_handler();
    let mut askers = upgrade_request::resolve_started(&state, &version);
    askers.extend(request.askers().into_iter().filter_map(|uid| handler.block_for_uid(uid)));
    tracing::info!(version = %version, requester = %request.requester_slug, "upgrade: approved and not vetoed; starting");
    upgrade_request::start_upgrade(version, askers);
    Ok(())
}

/// Approve the package the request was filed for, as the person's approval of it would, if it is
/// still exactly that version and signer. The person's countdown was the check.
async fn widget_install(state: AppState, request: Request) -> Result<(), String> {
    let target = crate::server::widget_agent_handlers::take_target(&request.id).ok_or("the package this request was for is no longer known; ask again")?;
    let service = crate::backend::widget_packages::service().ok_or("widget packages aren't available on this AgentMux")?;
    let svc = service.clone();
    let t = target.clone();
    tokio::task::spawn_blocking(move || svc.approve(&t.id, &t.hash, &t.signer)).await.map_err(|e| e.to_string())??;
    tracing::info!(id = %target.id, requester = %request.requester_slug, approver = ?request.decided_by, "widget package approved through a sensitive request");
    crate::backend::widget_packages::refresh_off_thread(&state.config_watcher, &state.event_bus, &state.broker).await;
    if crate::backend::widget_requests::requests().answer(&target.id, &target.hash, crate::backend::widget_requests::Answer::Approved) {
        crate::server::widget_agent_handlers::publish_requests(&state);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_has_a_runner_and_a_refusal_says_why() {
        register_all();
        assert!(window::has_runner("upgrade"));
        assert_eq!(why_not(&Outcome::UpToDate { current: "1.2.3".into() }), "AgentMux 1.2.3 is already the latest release");
        assert_eq!(why_not(&Outcome::Unavailable { message: "a development build".into(), url: None }), "a development build");
    }
}
