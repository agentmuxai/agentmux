// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may call which command (`server::dispatch_guard`), through a real connection's handlers.

use std::sync::Arc;

use serde_json::{json, Value};

use super::{decide, dev_open_from, guard_with, Mode, Verdict};
use crate::backend::rpc::action_access::{OPEN, OWN_PANE};
use crate::backend::rpc::action_commands::{ALWAYS, LIVE_INPUT, READ_ONLY, UNLESS_OPERATOR};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;

const MARKER: &str = "isn't the AgentMux window";

fn desktop() -> Mode {
    Mode::default()
}

fn guarded(mode: Mode, ctx: RpcContext) -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) {
    let (engine, rx) = WshRpcEngine::new();
    register_handlers(&engine, test_state(), format!("guard-{}", uuid::Uuid::new_v4()));
    engine.set_dispatch_guard(guard_with(move || mode));
    engine.set_rpc_context(ctx);
    (engine, rx)
}

/// What `command` answers on `engine`: the error text, or empty for a success.
async fn answer(engine: &Arc<WshRpcEngine>, rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>, command: &str, data: Value) -> String {
    let reqid = format!("guard-{}", uuid::Uuid::new_v4());
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: reqid.clone(), data: Some(data), ..Default::default() });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let m = rx.recv().await.expect("the connection stays open");
            if m.resid == reqid {
                return m.error;
            }
        }
    })
    .await
    .expect("an answer within 10s")
}

fn agent(slug: &str) -> RpcContext {
    RpcContext { agent_id: slug.into(), ..Default::default() }
}

#[test]
fn the_window_calls_anything_and_an_unproven_connection_only_what_is_open() {
    let window = RpcContext::operator_for_tests();
    assert_eq!(decide("agent.kill-tree", &window, desktop()), Verdict::Allow);
    assert_eq!(decide("getmemory", &window, desktop()), Verdict::Allow);
    let unproven = RpcContext::default();
    for command in ["agent.kill-tree", "getmemory", "agent.output", "controllerinput", "blockinput", "setblocktermsize", "a-command-nobody-decided"] {
        let Verdict::Refuse(why) = decide(command, &unproven, desktop()) else { panic!("{command} was not refused") };
        assert!(why.starts_with("FORBIDDEN") && why.contains(MARKER), "{why}");
    }
    for command in OPEN {
        assert_eq!(decide(command, &unproven, desktop()), Verdict::Allow, "{command}");
    }
}

#[test]
fn a_registered_agent_reaches_the_open_commands_and_the_pane_checked_ones_only() {
    let a = agent("fenn");
    for command in OPEN.iter().chain(OWN_PANE) {
        assert_eq!(decide(command, &a, desktop()), Verdict::Allow, "{command}");
    }
    for command in ["agent.kill-tree", "getmemory", "controllerinput", "blockinput", "agent.output", "a-command-nobody-decided"] {
        let Verdict::Refuse(why) = decide(command, &a, desktop()) else { panic!("{command} was not refused") };
        assert!(why.starts_with("FORBIDDEN") && why.contains("MuxAPI tools"), "{why}");
    }
    // The window's proof does not survive registering as an agent, and a claim of it would not help.
    let claimed = RpcContext { agent_id: "fenn".into(), operator_proven: true, ..Default::default() };
    assert!(matches!(decide("agent.kill-tree", &claimed, desktop()), Verdict::Refuse(_)));
}

#[test]
fn a_lapsed_proof_is_refused_with_the_words_that_prove_the_connection_again() {
    let lapsed = RpcContext { operator_proven: true, operator_proof_until: Some(1), ..Default::default() };
    let Verdict::Refuse(why) = decide("agent.kill-tree", &lapsed, desktop()) else { panic!("not refused") };
    assert!(why.contains(MARKER), "{why}");
}

#[test]
fn a_headless_srv_admits_an_unproven_connection_only_with_no_host_and_no_sign_in() {
    let unproven = RpcContext::default();
    let bare = Mode { headless: true, ..Default::default() };
    assert_eq!(decide("agent.kill-tree", &unproven, bare), Verdict::AllowUnproven);
    // A registered external host attests the connections it proxies: a connection it did not is refused.
    assert!(matches!(decide("agent.kill-tree", &unproven, Mode { external_host: true, ..bare }), Verdict::Refuse(_)));
    // A browser signs in to this srv, so the person's page is vouched and anything else is not.
    assert!(matches!(decide("agent.kill-tree", &unproven, Mode { local_sign_in: true, ..bare }), Verdict::Refuse(_)));
    // A registered agent is refused in every mode.
    assert!(matches!(decide("agent.kill-tree", &agent("fenn"), bare), Verdict::Refuse(_)));
    // And a desktop srv, which has a CEF host, never takes the fallback.
    assert!(matches!(decide("agent.kill-tree", &unproven, desktop()), Verdict::Refuse(_)));
}

#[test]
fn the_dev_switch_counts_only_in_a_dev_build() {
    use agentmux_common::RuntimeMode;
    let dev = || Some(RuntimeMode::Dev { branch: "b".into(), clone_id: None });
    assert!(dev_open_from(dev(), Some("1")));
    assert!(!dev_open_from(dev(), None) && !dev_open_from(dev(), Some("0")), "a dev build runs the guard unless asked");
    for runtime in [Some(RuntimeMode::Installed), Some(RuntimeMode::Portable), None] {
        assert!(!dev_open_from(runtime, Some("1")), "ignored outside a dev build");
    }
    let open = Mode { dev_open: true, ..Default::default() };
    assert_eq!(decide("agent.kill-tree", &RpcContext::default(), open), Verdict::Allow);
}

#[tokio::test]
async fn an_unproven_connection_that_holds_the_key_is_refused_before_the_handler_runs() {
    let (engine, mut rx) = guarded(desktop(), RpcContext::default());
    for command in ["agent.kill-tree", "agent.output", "getmeta", "subprocessspawn", "controllerinput"] {
        let refused = answer(&engine, &mut rx, command, json!({})).await;
        assert!(refused.starts_with("FORBIDDEN") && refused.contains(MARKER), "{command}: {refused}");
    }
    assert!(engine.check_dispatch("blockinput").is_err() && engine.check_dispatch("setblocktermsize").is_err());
}

#[tokio::test]
async fn the_window_reaches_the_handler() {
    let (engine, mut rx) = guarded(desktop(), RpcContext::operator_for_tests());
    let answered = answer(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "no-such-pane"})).await;
    assert!(!answered.contains(MARKER), "{answered}");
    assert!(engine.check_dispatch("blockinput").is_ok());
}

#[tokio::test]
async fn what_the_launcher_sends_unproven_still_reaches_its_handlers() {
    let (engine, mut rx) = guarded(desktop(), RpcContext::default());
    // Subscriptions and the acknowledgement of a notice. (`setconfig` writes the settings file, so it is checked below
    // by the guard's decision and the handler's own rule, not by a call.)
    let calls = [
        ("eventsub", json!({"event": "notification", "allscopes": true})),
        ("eventsub", json!({"event": "notification:retract", "allscopes": true})),
        ("eventsub", json!({"event": "notification:state", "allscopes": true})),
        ("notify.ack", json!({"id": "no-such-notice", "clicked": false})),
    ];
    for (command, data) in calls {
        let answered = answer(&engine, &mut rx, command, data).await;
        assert!(!answered.contains(MARKER), "{command}: {answered}");
    }
    let unproven = RpcContext::default();
    assert_eq!(decide("setconfig", &unproven, desktop()), Verdict::Allow);
    // The handler's own check stays and the guard does not widen it: the launcher's two keys pass it, a setting only
    // the person changes does not.
    let keys = |v: Value| v.as_object().cloned().unwrap();
    for key in ["notify:pause:until", "app:startatlogin"] {
        assert_eq!(crate::backend::settings_consent::refuse_unless_person(&unproven, &keys(json!({ key: 1 }))), Ok(()), "{key}");
    }
    let refused = crate::backend::settings_consent::refuse_unless_person(&unproven, &keys(json!({"cmd:env": {}}))).unwrap_err();
    assert!(refused.contains(MARKER), "{refused}");
}

#[tokio::test]
async fn an_agent_reaches_its_own_pane_checks_and_nothing_else() {
    let (engine, mut rx) = guarded(desktop(), agent("fenn"));
    // The handler's own-pane rule answers (the pane does not exist), not the guard.
    let own = answer(&engine, &mut rx, "controllerresync", json!({"blockid": "pane-nobody-has"})).await;
    assert!(!own.contains("MuxAPI tools"), "{own}");
    let other = answer(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "pane-nobody-has"})).await;
    assert!(other.contains("MuxAPI tools"), "{other}");
}

#[tokio::test]
async fn every_listed_command_is_registered_and_each_registered_command_is_decided() {
    let (engine, _rx) = guarded(desktop(), RpcContext::default());
    let registered: std::collections::BTreeSet<String> = engine.command_names().into_iter().collect();
    for name in OPEN.iter().chain(OWN_PANE) {
        assert!(registered.contains(*name), "{name} is listed in action_access.rs but no connection registers it");
    }
    // Every registered command is in the recording table too, so the generated list of the window's commands
    // (scripts/gen-operator-commands.mjs, which reads both) has no gap.
    let decided: std::collections::BTreeSet<&str> = [ALWAYS, UNLESS_OPERATOR, LIVE_INPUT, READ_ONLY].iter().flat_map(|l| l.iter().copied()).collect();
    let missing: Vec<_> = registered.iter().filter(|r| !decided.contains(r.as_str())).collect();
    assert!(missing.is_empty(), "registered commands in no list of action_commands.rs: {missing:?}");
}
