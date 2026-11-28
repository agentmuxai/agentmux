// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may call which command (`server::dispatch_guard`), through a real connection's handlers.

use std::sync::Arc;

use serde_json::{json, Value};

use super::{decide, dev_open_from, guard_with, Mode, Verdict};
use crate::backend::rpc::action_access::EVENT_COMMANDS;
use crate::backend::rpc::action_access::{OPEN, OWN_PANE};
use crate::backend::rpc::action_commands::{ALWAYS, LIVE_INPUT, READ_ONLY, UNLESS_OPERATOR};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;

const MARKER: &str = "isn't the AgentMux window";

/// A request for `command` that is as open as the command can be: an event command names an open event.
fn request_for(command: &str) -> Value {
    if EVENT_COMMANDS.contains(&command) {
        json!({"event": "notification", "allscopes": true})
    } else {
        Value::Null
    }
}

fn decide_for(command: &str, ctx: &RpcContext, mode: Mode) -> Verdict {
    decide(command, &request_for(command), ctx, mode)
}

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
    assert_eq!(decide_for("agent.kill-tree", &window, desktop()), Verdict::Allow);
    assert_eq!(decide_for("getmemory", &window, desktop()), Verdict::Allow);
    let unproven = RpcContext::default();
    for command in ["agent.kill-tree", "getmemory", "agent.output", "controllerinput", "blockinput", "setblocktermsize", "a-command-nobody-decided"] {
        let Verdict::Refuse(why) = decide_for(command, &unproven, desktop()) else { panic!("{command} was not refused") };
        assert!(why.starts_with("FORBIDDEN") && why.contains(MARKER), "{why}");
    }
    for command in OPEN {
        assert_eq!(decide_for(command, &unproven, desktop()), Verdict::Allow, "{command}");
    }
}

#[test]
fn a_registered_agent_reaches_the_open_commands_and_the_pane_checked_ones_only() {
    let a = agent("fenn");
    for command in OPEN.iter().chain(OWN_PANE) {
        assert_eq!(decide_for(command, &a, desktop()), Verdict::Allow, "{command}");
    }
    for command in ["agent.kill-tree", "getmemory", "controllerinput", "blockinput", "agent.output", "a-command-nobody-decided"] {
        let Verdict::Refuse(why) = decide_for(command, &a, desktop()) else { panic!("{command} was not refused") };
        assert!(why.starts_with("FORBIDDEN") && why.contains("MuxAPI tools"), "{why}");
    }
    // The window's proof does not survive registering as an agent, and a claim of it would not help.
    let claimed = RpcContext { agent_id: "fenn".into(), operator_proven: true, ..Default::default() };
    assert!(matches!(decide_for("agent.kill-tree", &claimed, desktop()), Verdict::Refuse(_)));
}

/// What writes into a pane, or into an agent's composer, over a connection. Each is the person's hands: typed into a pane it reads as
/// the person's own words.
const WINDOW_INPUT: [&str; 5] = ["blockinput", "setblocktermsize", "controllerinput", "agent.send", "agentinput"];

#[test]
fn a_connection_holding_only_the_key_cannot_type_into_a_pane() {
    // The instance key opens `/ws`; a native agent has it in its environment. Holding it, with no agent id and no proof from the host, is not proof.
    let key_holder = RpcContext::default();
    for command in WINDOW_INPUT {
        let Verdict::Refuse(why) = decide_for(command, &key_holder, desktop()) else { panic!("{command} was not refused") };
        assert!(why.starts_with("FORBIDDEN") && why.contains(MARKER), "{command}: {why}");
    }
    // A proof that has lapsed is no proof either, and a claim of one does not count once the connection is an agent's.
    let lapsed = RpcContext { operator_proven: true, operator_proof_until: Some(1), ..Default::default() };
    let claimed = RpcContext { agent_id: "fenn".into(), operator_proven: true, ..Default::default() };
    for command in WINDOW_INPUT {
        assert!(matches!(decide_for(command, &lapsed, desktop()), Verdict::Refuse(_)), "{command}, lapsed");
    }
    for command in &WINDOW_INPUT[..4] {
        assert!(matches!(decide_for(command, &claimed, desktop()), Verdict::Refuse(_)), "{command}, claimed");
    }
    // The window itself still types.
    for command in WINDOW_INPUT {
        assert_eq!(decide_for(command, &RpcContext::operator_for_tests(), desktop()), Verdict::Allow, "{command}");
    }
}

#[test]
fn an_agent_connection_writes_into_a_pane_only_through_agentinput_on_its_own_pane() {
    // `agentinput` is the one input command an agent's connection reaches; its handler answers for the pane
    // (`pane_caller_tests`: another agent's pane is refused, a typed message is the window's alone). The rest are refused.
    let a = agent("fenn");
    assert_eq!(decide_for("agentinput", &a, desktop()), Verdict::Allow);
    for command in &WINDOW_INPUT[..4] {
        let Verdict::Refuse(why) = decide_for(command, &a, desktop()) else { panic!("{command} was not refused") };
        assert!(why.starts_with("FORBIDDEN") && why.contains("MuxAPI tools"), "{command}: {why}");
    }
}

#[test]
fn a_lapsed_proof_is_refused_with_the_words_that_prove_the_connection_again() {
    let lapsed = RpcContext { operator_proven: true, operator_proof_until: Some(1), ..Default::default() };
    let Verdict::Refuse(why) = decide_for("agent.kill-tree", &lapsed, desktop()) else { panic!("not refused") };
    assert!(why.contains(MARKER), "{why}");
}

#[test]
fn a_headless_srv_admits_an_unproven_connection_only_with_no_host_and_no_sign_in() {
    let unproven = RpcContext::default();
    let bare = Mode { headless: true, ..Default::default() };
    assert_eq!(decide_for("agent.kill-tree", &unproven, bare), Verdict::AllowUnproven);
    // A registered external host attests the connections it proxies: a connection it did not is refused.
    assert!(matches!(decide_for("agent.kill-tree", &unproven, Mode { external_host: true, ..bare }), Verdict::Refuse(_)));
    // A browser signs in to this srv, so the person's page is vouched and anything else is not.
    assert!(matches!(decide_for("agent.kill-tree", &unproven, Mode { local_sign_in: true, ..bare }), Verdict::Refuse(_)));
    // A registered agent is refused in every mode.
    assert!(matches!(decide_for("agent.kill-tree", &agent("fenn"), bare), Verdict::Refuse(_)));
    // And a desktop srv, which has a CEF host, never takes the fallback.
    assert!(matches!(decide_for("agent.kill-tree", &unproven, desktop()), Verdict::Refuse(_)));
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
    assert_eq!(decide_for("agent.kill-tree", &RpcContext::default(), open), Verdict::Allow);
}

#[tokio::test]
async fn an_unproven_connection_that_holds_the_key_is_refused_before_the_handler_runs() {
    let (engine, mut rx) = guarded(desktop(), RpcContext::default());
    for command in ["agent.kill-tree", "agent.output", "getmeta", "subprocessspawn", "controllerinput"] {
        let refused = answer(&engine, &mut rx, command, json!({})).await;
        assert!(refused.starts_with("FORBIDDEN") && refused.contains(MARKER), "{command}: {refused}");
    }
    assert!(engine.check_dispatch("blockinput", &Value::Null).is_err() && engine.check_dispatch("setblocktermsize", &Value::Null).is_err());
}

#[tokio::test]
async fn the_window_reaches_the_handler() {
    let (engine, mut rx) = guarded(desktop(), RpcContext::operator_for_tests());
    let answered = answer(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "no-such-pane"})).await;
    assert!(!answered.contains(MARKER), "{answered}");
    assert!(engine.check_dispatch("blockinput", &Value::Null).is_ok());
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
    assert_eq!(decide_for("setconfig", &unproven, desktop()), Verdict::Allow);
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

#[test]
fn an_event_is_named_only_when_it_is_the_launchers_for_anyone_but_the_window() {
    let blockfile = json!({"event": "blockfile", "allscopes": true});
    let notice = json!({"event": "notification:state", "allscopes": true});
    let unproven = RpcContext::default();
    for command in ["eventsub", "eventreadhistory"] {
        let Verdict::Refuse(why) = decide(command, &blockfile, &unproven, desktop()) else { panic!("{command} blockfile was not refused") };
        assert!(why.contains(MARKER), "{why}");
        assert!(matches!(decide(command, &blockfile, &agent("fenn"), desktop()), Verdict::Refuse(_)), "an agent's connection too");
        assert_eq!(decide(command, &notice, &unproven, desktop()), Verdict::Allow);
        assert_eq!(decide(command, &notice, &agent("fenn"), desktop()), Verdict::Allow);
        assert_eq!(decide(command, &blockfile, &RpcContext::operator_for_tests(), desktop()), Verdict::Allow, "the window names any event");
    }
}

#[tokio::test]
async fn an_unproven_connection_cannot_subscribe_to_another_panes_output() {
    let (engine, mut rx) = guarded(desktop(), RpcContext::default());
    let refused = answer(&engine, &mut rx, "eventsub", json!({"event": "blockfile", "scopes": [], "allscopes": true})).await;
    assert!(refused.starts_with("FORBIDDEN") && refused.contains(MARKER), "{refused}");
    let refused = answer(&engine, &mut rx, "eventreadhistory", json!({"event": "blockfile", "scope": "", "maxitems": 10})).await;
    assert!(refused.contains(MARKER), "{refused}");
    let accepted = answer(&engine, &mut rx, "eventsub", json!({"event": "notification", "scopes": [], "allscopes": true})).await;
    assert_eq!(accepted, "");
}
