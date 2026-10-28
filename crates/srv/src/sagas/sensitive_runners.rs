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
    // The person may have answered meanwhile, in Settings: approved it themselves (nothing left to
    // do) or turned it off (a later approval must not turn it back on).
    match service.list().into_iter().find(|p| p.id == target.id).map(|p| p.state) {
        Some(crate::backend::widget_packages::WidgetState::Disabled) => return Err("the person turned this widget off in the meantime; it stays off".to_string()),
        Some(crate::backend::widget_packages::WidgetState::Approved) => return Ok(()),
        _ => {}
    }
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

    use serde_json::json;

    /// A package copied in and listed, as `WidgetInstall` leaves it.
    async fn installed(state: &AppState, id: &str, kind: &str, permissions: serde_json::Value) -> crate::backend::widget_packages::WidgetPackageInfo {
        crate::backend::widget_packages::WidgetPackages::for_tests();
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join(id);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("widget.json"),
            json!({ "manifestVersion": 1, "id": id, "name": "Made by an agent", "version": "1.0.0", "kind": kind, "entry": "index.html",
                    "permissions": permissions, "contributes": { "panes": [{ "name": "main" }] } })
            .to_string(),
        )
        .unwrap();
        std::fs::write(src.join("index.html"), "<p>hi</p>").unwrap();
        let svc = crate::backend::widget_packages::service().unwrap();
        crate::backend::widget_packages::install(&svc.widgets_dir, &src, true).unwrap();
        crate::backend::widget_packages::refresh_off_thread(&state.config_watcher, &state.event_bus, &state.broker)
            .await
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    }

    fn filed(pkg: &crate::backend::widget_packages::WidgetPackageInfo) -> Request {
        let reg = crate::backend::sensitive_requests::registry();
        let mut facts = crate::server::widget_agent_handlers::install_facts(pkg);
        facts.insert("widget".into(), format!("{} {}", facts["widget"], uuid::Uuid::new_v4().simple()));
        let id = reg.request(&format!("u-{}", uuid::Uuid::new_v4().simple()), "asker", "widget_install", &facts, "", agentmux_common::time::now_ms()).unwrap().id;
        reg.get(&id).unwrap()
    }

    #[tokio::test]
    async fn the_countdown_ending_approves_exactly_the_package_that_was_asked_for() {
        let state = crate::server::tests::test_state();
        let pkg = installed(&state, "acme.runner-ok", "sandboxed", json!(["storage"])).await;
        assert_eq!(pkg.state, crate::backend::widget_packages::WidgetState::NeedsApproval);
        let request = filed(&pkg);
        crate::server::widget_agent_handlers::remember_target(
            &request.id,
            crate::server::widget_agent_handlers::InstallTarget { id: pkg.id.clone(), hash: pkg.hash.clone(), signer: String::new() },
        );
        widget_install(state.clone(), request.clone()).await.unwrap();
        let after = crate::backend::widget_packages::service().unwrap().list().into_iter().find(|p| p.id == pkg.id).unwrap();
        assert_eq!(after.state, crate::backend::widget_packages::WidgetState::Approved);
        // The target is used once: running it again finds nothing to approve.
        assert!(widget_install(state, request).await.unwrap_err().contains("no longer known"));
    }

    #[tokio::test]
    async fn a_request_that_ended_is_forgotten_and_a_second_target_never_replaces_the_first() {
        let state = crate::server::tests::test_state();
        let pkg = installed(&state, "acme.runner-forget", "sandboxed", json!([])).await;
        let open = filed(&pkg);
        let target = |hash: &str| crate::server::widget_agent_handlers::InstallTarget { id: pkg.id.clone(), hash: hash.into(), signer: String::new() };
        crate::server::widget_agent_handlers::remember_target(&open.id, target("first"));
        crate::server::widget_agent_handlers::remember_target(&open.id, target("second"));
        let ended = filed(&pkg);
        crate::server::widget_agent_handlers::remember_target(&ended.id, target("ended"));
        crate::backend::sensitive_requests::registry().deny(&ended.id, "boss", "no", agentmux_common::time::now_ms()).unwrap();
        crate::server::widget_agent_handlers::remember_target("some-later-request", target("later"));
        assert_eq!(crate::server::widget_agent_handlers::take_target(&ended.id), None, "denied: forgotten");
        assert_eq!(crate::server::widget_agent_handlers::take_target(&open.id).unwrap().hash, "first");
    }

    #[tokio::test]
    async fn a_widget_the_person_turned_off_or_approved_meanwhile_is_left_as_they_left_it() {
        let state = crate::server::tests::test_state();
        let off = installed(&state, "acme.runner-off", "sandboxed", json!([])).await;
        let service = crate::backend::widget_packages::service().unwrap();
        // The person approves it in Settings, then turns it off.
        service.approve(&off.id, &off.hash, "").unwrap();
        service.set_enabled(&off.id, false).unwrap();
        crate::backend::widget_packages::refresh_off_thread(&state.config_watcher, &state.event_bus, &state.broker).await;
        let request = filed(&off);
        crate::server::widget_agent_handlers::remember_target(
            &request.id,
            crate::server::widget_agent_handlers::InstallTarget { id: off.id.clone(), hash: off.hash.clone(), signer: String::new() },
        );
        let err = widget_install(state.clone(), request).await.unwrap_err();
        assert!(err.contains("turned this widget off"), "{err}");
        let after = service.list().into_iter().find(|p| p.id == off.id).unwrap();
        assert_eq!(after.state, crate::backend::widget_packages::WidgetState::Disabled, "still off");
        // One the person approved themselves needs nothing more.
        let done = installed(&state, "acme.runner-done", "sandboxed", json!([])).await;
        service.approve(&done.id, &done.hash, "").unwrap();
        crate::backend::widget_packages::refresh_off_thread(&state.config_watcher, &state.event_bus, &state.broker).await;
        let request = filed(&done);
        crate::server::widget_agent_handlers::remember_target(
            &request.id,
            crate::server::widget_agent_handlers::InstallTarget { id: done.id.clone(), hash: done.hash.clone(), signer: String::new() },
        );
        widget_install(state, request).await.unwrap();
    }

    #[tokio::test]
    async fn a_package_changed_after_the_request_is_not_approved() {
        let state = crate::server::tests::test_state();
        let pkg = installed(&state, "acme.runner-changed", "sandboxed", json!([])).await;
        let request = filed(&pkg);
        crate::server::widget_agent_handlers::remember_target(
            &request.id,
            crate::server::widget_agent_handlers::InstallTarget { id: pkg.id.clone(), hash: "not-the-hash".into(), signer: String::new() },
        );
        let err = widget_install(state, request).await.unwrap_err();
        assert!(err.contains("changed since"), "{err}");
        let after = crate::backend::widget_packages::service().unwrap().list().into_iter().find(|p| p.id == pkg.id).unwrap();
        assert_ne!(after.state, crate::backend::widget_packages::WidgetState::Approved);
    }

    #[tokio::test]
    async fn what_the_person_is_shown_names_every_permission_and_says_when_it_is_trusted() {
        let state = crate::server::tests::test_state();
        let plain = installed(&state, "acme.facts-plain", "sandboxed", json!(["storage", "net:https://a.example"])).await;
        let facts = crate::server::widget_agent_handlers::install_facts(&plain);
        assert_eq!(facts["permissions"], "storage, net:https://a.example");
        assert!(facts["widget"].contains("acme.facts-plain 1.0.0, content ") && facts["publisher"].contains("unsigned"), "{facts:?}");
        assert!(facts["widget"].contains(&plain.hash[..12]), "different files are a different request: {facts:?}");
        assert!(crate::backend::sensitive_ops::spec("widget_install").unwrap().check_facts(&facts).is_ok(), "the table takes them as they are");
        let none = installed(&state, "acme.facts-none", "sandboxed", json!([])).await;
        assert_eq!(crate::server::widget_agent_handlers::install_facts(&none)["permissions"], "none");
        let trusted = installed(&state, "acme.facts-trusted", "trusted", json!([])).await;
        assert_eq!(crate::server::widget_agent_handlers::install_facts(&trusted)["permissions"], "TRUSTED: full access to AgentMux");
    }

    #[test]
    fn upgrade_has_a_runner_and_a_refusal_says_why() {
        register_all();
        assert!(window::has_runner("upgrade"));
        assert_eq!(why_not(&Outcome::UpToDate { current: "1.2.3".into() }), "AgentMux 1.2.3 is already the latest release");
        assert_eq!(why_not(&Outcome::Unavailable { message: "a development build".into(), url: None }), "a development build");
    }
}
