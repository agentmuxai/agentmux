// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MspController and its protocol state (`msp_session.rs`). The host's lines
//! below are the shapes `muse serve` 1.4.7-R6101.1 wrote in the spike
//! (initialize, session/start, a signed-out turn, an echo turn); tool calls
//! and approvals follow its schema.

use serde_json::{json, Map, Value};

use super::super::msp_protocol::TURN_ENDED_BY_SRV;
use super::super::msp_session::{Effects, MspSession, PromptOutcome};
use super::*;

fn parse(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

fn methods(fx: &Effects) -> Vec<String> {
    fx.send.iter().map(|l| parse(l)["method"].as_str().unwrap_or("<reply>").to_string()).collect()
}

fn id_of(line: &str) -> u64 {
    parse(line)["id"].as_u64().unwrap()
}

fn agentmux_mcp() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("agentmux".into(), json!({"transport": "stdio", "command": "agentmux-mcp", "mode": "optional"}));
    m
}

fn init_result() -> Value {
    json!({"serverInfo": {"name": "muse", "version": "1.4.7"},
           "schema": {"version": 1, "fingerprint": "sha256:6de32db08bbbe0deb478d2bfcaba001e9fb490986bc80db3aceb4d996abf7df9"},
           "grantedCapabilities": ["sessionMcp"], "experimentalApi": false})
}

/// A session past the handshake: `(session, session/start id)`.
fn opened(resume: Option<&str>) -> MspSession {
    let mut s = MspSession::new();
    let init = s.begin("C:/agents/m".into(), agentmux_mcp(), resume.map(str::to_string));
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id_of(&init), "result": init_result()}));
    let start = id_of(&fx.send[1]);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": start, "result": {"session": {"sessionId": "s1"}, "viewCursor": ""}}));
    assert_eq!(fx.session.as_deref(), Some("s1"));
    s
}

fn send(s: &mut MspSession, text: &str) -> (u64, Value) {
    match s.prompt(text).unwrap() {
        PromptOutcome::Send { id, line } => (id, parse(&line)),
        other => panic!("expected a send, got {other:?}"),
    }
}

fn ack(s: &mut MspSession, id: u64, turn: &str, disposition: &str) -> Effects {
    s.on_message(&json!({"jsonrpc": "2.0", "id": id, "result": {"commandId": turn, "status": "accepted", "turnId": turn, "disposition": disposition}}))
}

fn completed(s: &mut MspSession, turn: &str) -> Effects {
    s.on_message(&json!({"jsonrpc": "2.0", "method": "turn/completed", "params": {"sessionId": "s1", "turnId": turn, "terminal": "completed"}}))
}

#[test]
fn the_handshake_initializes_then_starts_a_session_with_the_mcp_servers() {
    let mut s = MspSession::new();
    let init = parse(&s.begin("C:/agents/m".into(), agentmux_mcp(), None));
    assert_eq!(init["method"], "initialize");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": init["id"], "result": init_result()}));
    assert_eq!(methods(&fx), ["initialized", "session/start"]);
    let start = parse(&fx.send[1]);
    assert_eq!(start["params"]["workspaceRoot"], "C:/agents/m");
    assert_eq!(start["params"]["config"]["mcpServers"]["agentmux"]["command"], "agentmux-mcp");
    assert!(!fx.hide, "the handshake's answers stay in the transcript");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": start["id"], "result": {"session": {"sessionId": "s1"}}}));
    assert_eq!(fx.session.as_deref(), Some("s1"));
    assert_eq!(s.session_id(), Some("s1"));
}

#[test]
fn without_session_mcp_granted_the_session_starts_without_servers() {
    let mut s = MspSession::new();
    let init = s.begin("C:/a".into(), agentmux_mcp(), None);
    let mut result = init_result();
    result["grantedCapabilities"] = json!([]);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id_of(&init), "result": result}));
    assert!(parse(&fx.send[1])["params"].get("config").is_none());
}

#[test]
fn a_message_before_the_session_waits_and_starts_the_turn_when_it_opens() {
    let mut s = MspSession::new();
    let init = s.begin("C:/a".into(), Map::new(), None);
    assert_eq!(s.prompt("one").unwrap(), PromptOutcome::Queued { reopen: None });
    assert_eq!(s.prompt("two").unwrap(), PromptOutcome::Queued { reopen: None });
    assert!(s.turn_active(), "a waiting message is a turn in flight");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id_of(&init), "result": init_result()}));
    let start = id_of(&fx.send[1]);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": start, "result": {"session": {"sessionId": "s1"}}}));
    assert_eq!(methods(&fx), ["turn/start"]);
    let turn = parse(&fx.send[0]);
    assert_eq!(turn["params"]["input"][0]["text"], "one\n\ntwo");
    assert_eq!(turn["params"]["sessionId"], "s1");
    assert_eq!(fx.turn_active, Some(true));
}

#[test]
fn a_waiting_message_survives_a_restart() {
    let mut s = MspSession::new();
    s.begin("C:/a".into(), Map::new(), None);
    s.prompt("hello").unwrap();
    s.begin("C:/a".into(), Map::new(), None);
    assert!(s.turn_active());
}

#[test]
fn a_turn_runs_from_its_ack_to_turn_completed() {
    let mut s = opened(None);
    let (id, req) = send(&mut s, "say hello");
    assert_eq!(req["method"], "turn/start");
    assert_eq!(req["params"]["ifBusy"], "steer");
    let fx = ack(&mut s, id, "t1", "started");
    assert_eq!(fx.turn_active, None, "still running");
    assert!(!fx.hide);
    assert!(s.turn_active());
    let fx = completed(&mut s, "t1");
    assert_eq!(fx.turn_active, Some(false));
    assert!(fx.append.is_empty(), "the host's own turn/completed ends it in the pane");
    assert!(!s.turn_active());
}

#[test]
fn a_steering_message_joins_the_running_turn() {
    let mut s = opened(None);
    let (first, _) = send(&mut s, "a");
    ack(&mut s, first, "t1", "started");
    let (second, _) = send(&mut s, "b");
    assert_eq!(ack(&mut s, second, "t1", "steered").turn_active, None);
    assert_eq!(completed(&mut s, "t1").turn_active, Some(false));
}

#[test]
fn a_turn_that_completes_before_its_ack_is_not_running_again() {
    let mut s = opened(None);
    let (id, _) = send(&mut s, "a");
    s.on_message(&json!({"jsonrpc": "2.0", "method": "turn/started", "params": {"sessionId": "s1", "turnId": "t1"}}));
    assert_eq!(completed(&mut s, "t1").turn_active, None, "its turn/start is still unanswered");
    assert_eq!(ack(&mut s, id, "t1", "started").turn_active, Some(false));
    assert!(!s.turn_active());
}

#[test]
fn a_refused_turn_ends_in_the_pane() {
    let mut s = opened(None);
    let (id, _) = send(&mut s, "a");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32024, "message": "session not loaded", "data": {"kind": "sessionNotLoaded"}}}));
    assert!(!fx.hide, "the error is shown");
    assert_eq!(fx.append, [TURN_ENDED_BY_SRV]);
    assert_eq!(fx.turn_active, Some(false));
}

#[test]
fn a_signed_out_turn_ends_with_the_hosts_own_turn_completed() {
    // `muse serve` signed out: the turn is accepted, then fails `authRequired`.
    let mut s = opened(None);
    let (id, _) = send(&mut s, "say hello");
    ack(&mut s, id, "t1", "started");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "method": "turn/completed", "params": {
        "sessionId": "s1", "turnId": "t1", "terminal": "failed",
        "reason": "not logged in: run /login to add an API key",
        "error": {"kind": "authRequired", "message": "not logged in: run /login to add an API key", "retryable": false}}}));
    assert_eq!(fx.turn_active, Some(false));
    assert!(fx.append.is_empty() && !fx.hide);
}

#[test]
fn a_session_that_cant_be_resumed_is_replaced_by_a_new_one() {
    let mut s = MspSession::new();
    let init = s.begin("C:/a".into(), Map::new(), Some("old".into()));
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id_of(&init), "result": init_result()}));
    let resume = parse(&fx.send[1]);
    assert_eq!(resume["method"], "session/resume");
    assert_eq!(resume["params"]["sessionId"], "old");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": resume["id"], "error": {"code": -32020, "message": "session old not found", "data": {"kind": "sessionNotFound"}}}));
    assert!(fx.hide, "recovered: not shown");
    assert_eq!(methods(&fx), ["session/start"]);
}

#[test]
fn a_refused_session_fails_the_waiting_message_and_later_ones() {
    let mut s = MspSession::new();
    let init = s.begin("C:/a".into(), Map::new(), None);
    s.prompt("hi").unwrap();
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id_of(&init), "result": init_result()}));
    let start = id_of(&fx.send[1]);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": start, "error": {"code": -32030, "message": "refused", "data": {"kind": "commandRejected"}}}));
    assert!(!fx.hide);
    assert_eq!(fx.append, [TURN_ENDED_BY_SRV]);
    assert_eq!(fx.turn_active, Some(false));
    assert!(s.prompt("again").is_err());
}

#[test]
fn a_refused_initialize_fails_the_session() {
    let mut s = MspSession::new();
    let init = s.begin("C:/a".into(), Map::new(), None);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": id_of(&init), "error": {"code": -32600, "message": "bad"}}));
    assert!(fx.send.is_empty());
    assert!(s.prompt("hi").is_err());
}

#[test]
fn approvals_are_decided_once_per_stage_and_their_answers_hidden() {
    let mut s = opened(None);
    let request = json!({"jsonrpc": "2.0", "id": 900, "method": "approval/request", "params": {
        "approvalId": "a1", "sessionId": "s1", "currentRequirementId": {"stage": 1},
        "availableChoices": [{"choiceId": "c-once", "decision": "approved", "label": "Allow", "scope": "once"}],
        "toolName": "powershell", "turnId": "t1"}});
    let fx = s.on_message(&request);
    assert_eq!(parse(&fx.send[0]), json!({"jsonrpc": "2.0", "id": 900, "result": {}}));
    let decide = parse(&fx.send[1]);
    assert_eq!(decide["method"], "approval/decide");
    assert_eq!(decide["params"]["choiceId"], "c-once");
    assert_eq!(decide["params"]["requirementId"], json!({"stage": 1}));
    // Re-issued (a resubscribe): receipted again, not decided twice.
    assert_eq!(s.on_message(&request).send.len(), 1);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": decide["id"], "error": {"code": -32051, "message": "already resolved"}}));
    assert!(fx.hide);
}

#[test]
fn a_question_is_declined_and_other_requests_refused() {
    let mut s = opened(None);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": 5, "method": "userInput/request", "params": {"sessionId": "s1", "userInputId": "q1"}}));
    assert_eq!(methods(&fx), ["<reply>", "userInput/cancel"]);
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": 6, "method": "fs/read", "params": {}}));
    assert_eq!(parse(&fx.send[0])["error"]["code"], -32601);
}

#[test]
fn a_session_the_host_unloaded_is_resumed_for_the_next_message() {
    let mut s = opened(None);
    s.on_message(&json!({"jsonrpc": "2.0", "method": "session/closed", "params": {"sessionId": "s1", "reason": "idle", "viewCursor": null}}));
    assert_eq!(s.session_id(), None);
    let PromptOutcome::Queued { reopen: Some(line) } = s.prompt("hi").unwrap() else { panic!("expected a reopen") };
    let resume = parse(&line);
    assert_eq!(resume["method"], "session/resume");
    assert_eq!(resume["params"]["sessionId"], "s1");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": resume["id"], "result": {"session": {"sessionId": "s1"}}}));
    assert_eq!(methods(&fx), ["turn/start"]);
}

#[test]
fn another_sessions_close_is_ignored() {
    let mut s = opened(None);
    s.on_message(&json!({"jsonrpc": "2.0", "method": "session/closed", "params": {"sessionId": "other", "reason": "idle"}}));
    assert_eq!(s.session_id(), Some("s1"));
}

#[test]
fn interrupt_needs_a_running_turn() {
    let mut s = opened(None);
    assert!(s.interrupt().is_none());
    let (id, _) = send(&mut s, "a");
    ack(&mut s, id, "t1", "started");
    let line = parse(&s.interrupt().unwrap());
    assert_eq!(line["method"], "turn/interrupt");
    assert_eq!(line["params"]["sessionId"], "s1");
    let fx = s.on_message(&json!({"jsonrpc": "2.0", "id": line["id"], "result": {"commandId": "x", "status": "accepted"}}));
    assert!(fx.hide, "srv's own command");
}

#[test]
fn a_process_that_ends_mid_turn_reports_it_once() {
    let mut s = opened(None);
    assert!(!s.process_ended());
    let mut s = opened(None);
    send(&mut s, "a");
    assert!(s.process_ended());
    assert!(!s.turn_active());
    assert_eq!(s.session_id(), None);
}

#[test]
fn an_unsent_turn_is_forgotten() {
    let mut s = opened(None);
    let (id, _) = send(&mut s, "a");
    assert!(s.prompt_unsent(id));
    assert!(!s.turn_active());
}

fn controller() -> MspController {
    MspController::new("msp-test-block".to_string(), None, None, None, None)
}

#[test]
fn the_controller_is_an_msp_session_agent() {
    let c = controller();
    assert_eq!(c.controller_type(), BLOCK_CONTROLLER_MSP);
    assert!(super::super::session_agent::session_agent(&c).is_some());
    assert!(super::super::session_agent::is_session_agent_type("msp"));
    assert!(!super::super::session_agent::is_session_agent_type("persistent"));
}

#[test]
fn a_message_to_a_stopped_agent_waits_for_its_next_start() {
    let c = controller();
    let err = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None).unwrap_err();
    assert!(err.contains("queued"), "{err}");
    assert!(c.inner.lock().unwrap().session.turn_active());
}

#[test]
fn a_start_without_a_command_is_refused() {
    let c = controller();
    let err = Controller::start(&c, crate::backend::obj::MetaMapType::new(), None, false).unwrap_err();
    assert!(err.contains("no cmd"), "{err}");
}

/// Waits up to 30 s for the session to be open with no turn running.
async fn idle_after(c: &MspController) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let inner = c.inner.lock().unwrap();
        if inner.session.session_id().is_some() && !inner.session.turn_active() {
            return true;
        }
    }
    false
}

/// The controller against a real `muse serve`, on its built-in `echo`
/// provider (no sign-in, no network): the handshake, one turn start to end,
/// and a second turn on the same session. Runs only with `MUSE_BIN` set to the
/// native binary, e.g. `%LOCALAPPDATA%\Programs\muse\muse-bin-1.4.7-R6101.1.exe`.
#[tokio::test]
async fn a_real_muse_serve_runs_a_turn_on_its_echo_provider() {
    let Ok(bin) = std::env::var("MUSE_BIN") else { return };
    let dir = tempfile::tempdir().unwrap();
    let home = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("cmd".into(), json!(bin));
    meta.insert("cmd:args".into(), json!(["serve", "--provider", "echo", "--no-session-log"]));
    meta.insert("cmd:cwd".into(), json!(dir.path().to_string_lossy()));
    meta.insert(
        "cmd:env".into(),
        json!({"XDG_CONFIG_HOME": home("cfg"), "XDG_DATA_HOME": home("data"), "MUSE_NO_AUTO_UPDATE": "1", "TBH_DISABLE_TELEMETRY": "1"}),
    );
    let c = controller();
    Controller::start(&c, meta, None, false).unwrap();
    c.send_message("hello".into(), None).unwrap();
    assert!(c.health_monitor.is_active_turn(), "the message started a turn");
    assert!(idle_after(&c).await, "the turn ended");
    assert!(!c.health_monitor.is_active_turn());
    let sid = c.inner.lock().unwrap().session.session_id().map(str::to_string);
    c.send_message("again".into(), None).unwrap();
    assert!(idle_after(&c).await, "the second turn ended");
    assert_eq!(c.inner.lock().unwrap().session.session_id().map(str::to_string), sid, "same session");
    Controller::stop(&c, true, STATUS_DONE).unwrap();
}
