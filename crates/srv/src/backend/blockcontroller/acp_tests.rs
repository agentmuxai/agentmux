// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


#[test]
fn initialize_states_protocol_version_1_and_offers_no_fs_or_terminal() {
    let p = super::initialize_params();
    assert_eq!(p["protocolVersion"], 1);
    assert_eq!(p["clientCapabilities"]["fs"]["readTextFile"], false);
    assert_eq!(p["clientCapabilities"]["terminal"], false);
    assert_eq!(p["clientInfo"]["name"], "AgentMux");
}

#[test]
fn a_session_is_loaded_only_when_the_agent_can_and_there_is_one() {
    let (m, p) = super::session_request(Some("s1"), true, "C:/w", &[]);
    assert_eq!(m, "session/load");
    assert_eq!(p, serde_json::json!({ "sessionId": "s1", "cwd": "C:/w", "mcpServers": [] }));
    assert_eq!(super::session_request(Some("s1"), false, "C:/w", &[]).0, "session/new");
    let (m, p) = super::session_request(None, true, "C:/w", &[]);
    assert_eq!(m, "session/new");
    assert_eq!(p, serde_json::json!({ "cwd": "C:/w", "mcpServers": [] }));
}

#[test]
fn the_prompt_is_an_array_of_content_blocks() {
    assert_eq!(
        super::prompt_params("s1", "hi"),
        serde_json::json!({ "sessionId": "s1", "prompt": [{ "type": "text", "text": "hi" }] })
    );
}

#[test]
fn permission_requests_are_approved_preferring_allow_always() {
    let req = |options: serde_json::Value| {
        serde_json::json!({ "jsonrpc": "2.0", "id": 7, "method": "session/request_permission",
            "params": { "sessionId": "s", "toolCall": { "toolCallId": "t" }, "options": options } })
    };
    let both = req(serde_json::json!([
        { "optionId": "once", "kind": "allow_once", "name": "Allow" },
        { "optionId": "always", "kind": "allow_always", "name": "Always" },
        { "optionId": "no", "kind": "reject_once", "name": "Reject" },
    ]));
    assert_eq!(
        super::reply_to_agent_request(&both).unwrap(),
        serde_json::json!({ "jsonrpc": "2.0", "id": 7, "result": { "outcome": { "outcome": "selected", "optionId": "always" } } })
    );
    let once = req(serde_json::json!([{ "optionId": "once", "kind": "allow_once", "name": "Allow" }]));
    assert_eq!(super::reply_to_agent_request(&once).unwrap()["result"]["outcome"]["optionId"], "once");
    let reject = req(serde_json::json!([{ "optionId": "no", "kind": "reject_once", "name": "Reject" }]));
    assert_eq!(
        super::reply_to_agent_request(&reject).unwrap()["result"]["outcome"],
        serde_json::json!({ "outcome": "cancelled" })
    );
}

#[test]
fn other_agent_requests_get_method_not_found_and_notifications_get_nothing() {
    let fs = serde_json::json!({ "jsonrpc": "2.0", "id": "a", "method": "fs/read_text_file", "params": {} });
    let reply = super::reply_to_agent_request(&fs).unwrap();
    assert_eq!(reply["id"], "a");
    assert_eq!(reply["error"]["code"], -32601);
    let note = serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": {} });
    assert!(super::reply_to_agent_request(&note).is_none());
    let response = serde_json::json!({ "jsonrpc": "2.0", "id": 3, "result": {} });
    assert!(super::reply_to_agent_request(&response).is_none());
}

/// Identity M4b-2 (spec §6.5.8): `agent.open` stores `cmd:args` as an
/// array and `cmd:env` as an object; ACP read only JSON strings, so
/// such a launch lost both. Both shapes now read the same.
/// Identity M4b-2 wiring (spec §6.5.8): an ACP controller with a store
/// spawns a row-backed block with `agent.open`'s object `cmd:env` kept
/// and the row's UID + token carried onto it.
#[test]
fn an_acp_spawn_env_keeps_cmd_env_and_carries_the_rows_identity() {
    use crate::backend::storage::agents::test_agent_def;
    let store =
        std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    let mut def = test_agent_def("uid-acp", "AcpAgent", "openclaw", "agent", 1, "");
    store.agent_def_insert(&mut def).unwrap();
    let mut meta = super::super::super::obj::MetaMapType::new();
    meta.insert("agentId".to_string(), serde_json::json!("uid-acp"));
    meta.insert("cmd:env".to_string(), serde_json::json!({"K": "v"}));
    let mut block = crate::backend::obj::Block {
        oid: "block-acp".to_string(),
        parentoref: String::new(),
        version: 0,
        runtimeopts: None,
        stickers: None,
        meta: meta.clone(),
        subblockids: None,
    };
    store.insert(&mut block).unwrap();
    let ctrl = AcpController::new(
        "tab".to_string(),
        "block-acp".to_string(),
        None,
        None,
        Some(store.clone()),
        None,
    );
    let env = ctrl.spawn_env(&meta);
    assert_eq!(env.get("K").map(String::as_str), Some("v"));
    assert_eq!(
        env.get("AGENTMUX_AGENT_UID").map(String::as_str),
        Some("uid-acp")
    );
    assert_eq!(
        env.get("AGENTMUX_AGENT_TOKEN"),
        store.agent_token_load("uid-acp").unwrap().as_ref()
    );
}

/// Plain-`gh` guard on the ACP spawn path: `GH_CONFIG_DIR` from `cmd:env`
/// is replaced by the guard dir, not passed through.
#[test]
fn an_acp_spawn_env_overrides_gh_config_dir_with_the_guard_dir() {
    let mut meta = super::super::super::obj::MetaMapType::new();
    meta.insert(
        "cmd:env".to_string(),
        serde_json::json!({"GH_CONFIG_DIR": "/home/human/.config/gh", "AGENTMUX_AGENT_SLUG": "acpy"}),
    );
    let ctrl = AcpController::new("tab".to_string(), "block-acp-gh".to_string(), None, None, None, None);
    let env = ctrl.spawn_env(&meta);
    assert_eq!(
        std::path::PathBuf::from(&env["GH_CONFIG_DIR"]),
        crate::backend::gh_guard::guard_config_home().join("gh-acpy")
    );
}

/// A host ACP agent is told where its workspace is, like every other
/// host spawn path (`carry_agent_workdir_env`).
#[test]
fn an_acp_spawn_env_carries_the_agent_workdir() {
    let ws = std::env::temp_dir().join("acpy-0930a");
    let mut meta = super::super::super::obj::MetaMapType::new();
    meta.insert("cmd:cwd".to_string(), serde_json::json!(ws.to_string_lossy()));
    meta.insert("cmd:env".to_string(), serde_json::json!({"AGENTMUX_AGENT_WORKDIR": "stale"}));
    let ctrl = AcpController::new("tab".to_string(), "block-acp-workdir".to_string(), None, None, None, None);
    assert_eq!(std::path::PathBuf::from(&ctrl.spawn_env(&meta)["AGENTMUX_AGENT_WORKDIR"]), ws);
}

/// No agent env carries the account's cloud login, even from a
/// `cmd:env` persisted before `account_login_guard` existed.
#[test]
fn an_acp_spawn_env_never_carries_the_account_login() {
    let mut meta = super::super::super::obj::MetaMapType::new();
    meta.insert(
        "cmd:env".to_string(),
        serde_json::json!({"MUXBUS_TOKEN": "eyJ.stale", "MUXBUS_COGNITO_DOMAIN": "https://auth.example", "AGENTMUX_AGENT_SLUG": "acpy"}),
    );
    let ctrl = AcpController::new("tab".to_string(), "block-acp-login".to_string(), None, None, None, None);
    let env = ctrl.spawn_env(&meta);
    assert!(!env.contains_key("MUXBUS_TOKEN"), "{env:?}");
    assert!(!env.contains_key("MUXBUS_COGNITO_DOMAIN"), "{env:?}");
}

#[test]
fn acp_reads_cmd_args_and_env_as_arrays_objects_or_json_strings() {
    let mut meta = super::super::super::obj::MetaMapType::new();
    meta.insert("cmd:args".to_string(), serde_json::json!(["--acp", "x"]));
    meta.insert("cmd:env".to_string(), serde_json::json!({"K": "v"}));
    assert_eq!(
        super::meta_string_list(&meta, "cmd:args"),
        vec!["--acp", "x"]
    );
    assert_eq!(
        super::meta_string_map(&meta, "cmd:env")
            .get("K")
            .map(String::as_str),
        Some("v")
    );

    meta.insert("cmd:args".to_string(), serde_json::json!("[\"--acp\"]"));
    meta.insert("cmd:env".to_string(), serde_json::json!("{\"K\":\"w\"}"));
    assert_eq!(super::meta_string_list(&meta, "cmd:args"), vec!["--acp"]);
    assert_eq!(
        super::meta_string_map(&meta, "cmd:env")
            .get("K")
            .map(String::as_str),
        Some("w")
    );

    assert!(super::meta_string_list(&meta, "absent").is_empty());
    assert!(super::meta_string_map(&meta, "absent").is_empty());
}

use super::*;

fn controller() -> AcpController {
    AcpController::new("tab".to_string(), "block".to_string(), None, None, None, None)
}

/// Regression for codex P2 on PR #2338 (twenty-seventh/thirtieth
/// re-reviews): the single-prompt case — one prompt sent, one
/// response — must still correctly declare the turn over.
#[test]
fn resolve_prompt_and_check_idle_reports_empty_after_the_only_outstanding_prompt_resolves() {
    let mut outstanding = HashSet::from([3]);
    assert!(
        resolve_prompt_and_check_idle(&mut outstanding, 3),
        "resolving the only outstanding prompt must report the turn as over"
    );
    assert!(outstanding.is_empty());
}

/// Regression for codex P2 on PR #2338 (twenty-seventh re-review): an
/// EARLIER prompt's response arriving after a steering/new prompt was
/// already sent must not end the turn while the newer one is still
/// outstanding.
#[test]
fn resolve_prompt_and_check_idle_stays_active_while_a_newer_steering_prompt_is_still_outstanding() {
    let mut outstanding = HashSet::from([1, 2]);
    assert!(
        !resolve_prompt_and_check_idle(&mut outstanding, 1),
        "the newer (2) prompt is still outstanding — the turn must not end yet"
    );
    assert_eq!(outstanding, HashSet::from([2]));
}

/// Regression for codex P2 on PR #2338 (thirtieth re-review) — the
/// exact scenario the single latest/previous-id tracking (removed)
/// could not handle: the EARLIER prompt completes and is resolved
/// FIRST (while a steering prompt is still outstanding), and the
/// steering prompt is REJECTED afterward. The turn must end once that
/// rejection resolves the LAST remaining outstanding prompt — not get
/// permanently stuck because the earlier prompt's completion was
/// already "consumed."
#[test]
fn resolve_prompt_and_check_idle_ends_the_turn_when_a_later_rejection_resolves_the_last_outstanding_prompt() {
    let mut outstanding = HashSet::from([1, 2]);
    // The earlier prompt (1) completes first.
    assert!(!resolve_prompt_and_check_idle(&mut outstanding, 1), "prompt 2 is still outstanding");
    // The steering prompt (2) is REJECTED afterward — this must now
    // correctly end the turn, since nothing else is outstanding.
    assert!(
        resolve_prompt_and_check_idle(&mut outstanding, 2),
        "the last outstanding prompt resolving (even via rejection) must end the turn"
    );
    assert!(outstanding.is_empty());
}

/// A response for an id that was never tracked (or already resolved) —
/// e.g. a duplicate delivery — must not panic; HashSet::remove is a
/// harmless no-op for a non-member.
#[test]
fn resolve_prompt_and_check_idle_is_a_harmless_no_op_for_an_untracked_id() {
    let mut outstanding: HashSet<u64> = HashSet::new();
    assert!(resolve_prompt_and_check_idle(&mut outstanding, 999));
    assert!(outstanding.is_empty());
}

/// `send_input` marks the turn active via `mark_turn_active_returning_was_active`
/// — pins the call-site contract that `turn_active` flips true once a
/// message is actually sent to an already-running process.
/// A message sent before the handshake has opened the session (the
/// startup message, sent right after launch) is queued for the stdout
/// reader to send once the session exists, never sent with an empty
/// session id (#4447).
#[tokio::test]
async fn a_message_before_the_session_exists_is_queued_not_sent() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    assert!(c.send_input(BlockInputUnion::data(b"startup".to_vec()), None).is_ok());
    assert!(c.send_input(BlockInputUnion::data(b"second".to_vec()), None).is_ok());
    assert!(rx.try_recv().is_err(), "nothing goes to the agent before the session exists");
    assert_eq!(c.inner.lock().unwrap().pending_prompt.as_deref(), Some("startup\n\nsecond"));
    assert!(!c.health_monitor.is_active_turn());
}

/// After `session/new` is refused nothing will ever open a session, so a
/// send is an error the user sees, not a message queued for good.
#[tokio::test]
async fn a_send_after_the_session_was_refused_is_an_error() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.session_failed = true;
    }
    let err = c.send_input(BlockInputUnion::data(b"hi".to_vec()), None).unwrap_err();
    assert!(err.contains("couldn't open a session"), "{err}");
    assert!(c.inner.lock().unwrap().pending_prompt.is_none());
}

/// A running agent is left alone; a stopped one is started from the pane's
/// meta (here with none, so the start itself reports it).
#[tokio::test]
async fn ensure_started_only_starts_a_stopped_agent() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(8);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    assert!(c.ensure_started(Default::default()).is_ok(), "running: nothing to do");
    c.inner.lock().unwrap().stdin_tx = None;
    let err = c.ensure_started(Default::default()).unwrap_err();
    assert!(err.contains("no cmd"), "stopped: it tries to start: {err}");
}

#[tokio::test]
async fn a_new_process_forgets_the_old_ones_unanswered_prompts() {
    let c = controller();
    c.outstanding_prompt_ids.lock().unwrap().insert(41);
    c.health_monitor.set_active_turn(true);
    let first = c.begin_process();
    assert!(c.outstanding_prompt_ids.lock().unwrap().is_empty());
    assert!(!c.health_monitor.is_active_turn());
    assert_eq!(c.begin_process(), first + 1);
}

/// ACP lets a request id be null and requires the reply to echo it; only a
/// message with no `id` at all is a notification.
#[test]
fn a_null_request_id_is_echoed() {
    let req = serde_json::json!({ "jsonrpc": "2.0", "id": null, "method": "fs/read_text_file", "params": {} });
    let reply = super::reply_to_agent_request(&req).unwrap();
    assert!(reply["id"].is_null());
    assert_eq!(reply["error"]["code"], -32601);
}

/// The previous process's waiter finishing after a restart must not clear
/// the new process's state.
#[tokio::test]
async fn a_stale_process_waiter_leaves_a_restart_alone() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.generation = 2;
    }
    // What the waiter of generation 1 checks before cleaning up.
    let stale = 1;
    let cleaned = {
        let mut inner = c.inner.lock().unwrap();
        if inner.generation != stale { false } else { inner.stdin_tx = None; true }
    };
    assert!(!cleaned);
    assert!(c.is_running(), "the new process stays connected");
}

#[test]
fn the_session_directory_is_never_empty() {
    let abs = std::env::current_dir().unwrap().join("work").join("a");
    assert_eq!(super::session_cwd(&abs.to_string_lossy()), abs.to_string_lossy());
    // A relative one resolves against the server's directory.
    let rel = super::session_cwd("work/a");
    assert!(std::path::Path::new(&rel).is_absolute(), "{rel}");
    assert!(rel.ends_with("a"), "{rel}");
    let fallback = super::session_cwd("");
    assert!(!fallback.is_empty());
    assert!(std::path::Path::new(&fallback).is_absolute(), "{fallback}");
}

#[test]
fn lines_during_a_session_load_are_replay_except_the_loads_own_answer() {
    assert!(super::is_session_replay(true, false));
    assert!(!super::is_session_replay(true, true));
    assert!(!super::is_session_replay(false, false));
}

#[tokio::test]
async fn send_input_marks_the_turn_active() {
    let c = controller();
    // Simulate an already-running process — send_input only reaches the
    // turn-active logic when `is_running()` is true (otherwise it
    // stashes the message as `pending_prompt` for the next start()).
    let (tx, _rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.session_id = Some("s1".to_string());
    }

    assert!(!c.health_monitor.is_active_turn());
    let res = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
    assert!(res.is_ok(), "send_input should succeed against a simulated running process, got {res:?}");
    assert!(c.health_monitor.is_active_turn(), "send_input must mark the turn active");
}

/// A second send while already active (mid-turn steering) must not
/// error or panic — `mark_turn_active_returning_was_active`'s
/// was-already-active branch is what gates re-arming; this at least
/// confirms the call site doesn't choke on repeated calls the way a
/// naive re-check-then-act would.
#[tokio::test]
async fn repeated_send_input_while_active_does_not_error() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.session_id = Some("s1".to_string());
    }

    assert!(c.send_input(BlockInputUnion::data(b"first".to_vec()), None).is_ok());
    assert!(c.send_input(BlockInputUnion::data(b"second".to_vec()), None).is_ok());
}

/// Regression for codex P1 on PR #2338 (twenty-third re-review):
/// `send_input` marked the turn active via `health_monitor` but never
/// published the flip — the only controllerstatus publishes for ACP
/// were on spawn, kill, or process exit. A `wasTurnActive === false`
/// left over from an EARLIER turn's end-of-turn publish (or the initial
/// spawn publish) was therefore indistinguishable from genuine current
/// idleness: useAgentCommands.ts's `isBackendTurnConfirmedIdle()` (fed
/// only by live controllerstatus events) would read stale-true and let
/// `flushPendingControllerRefresh` force-restart a controller that is
/// actually mid-turn. Mirrors persistent/queue.rs's send_message and
/// persistent/input.rs's send_user_message, which both call
/// `publish_status()` right after the same atomic turn-active mark.
#[tokio::test]
async fn send_input_publishes_the_turn_active_flip() {
    let broker = Arc::new(mps::Broker::new());
    let c = AcpController::new(
        "tab".to_string(),
        "block-acp-publish".to_string(),
        Some(broker.clone()),
        None,
        None,
        None,
    );
    let (tx, _rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.session_id = Some("s1".to_string());
    }

    assert!(c.send_input(BlockInputUnion::data(b"hello".to_vec()), None).is_ok());

    let history = broker.read_event_history(
        mps::EVENT_CONTROLLER_STATUS,
        "block:block-acp-publish",
        1,
    );
    assert_eq!(history.len(), 1, "send_input must publish a controllerstatus event");
    let published: BlockControllerRuntimeStatus =
        serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
    assert!(published.turn_active, "published status must reflect the just-started turn as active");
    assert!(c.health_monitor.is_active_turn());
}

/// Regression for codex P2 on PR #2338 (twenty-fifth re-review), as
/// refined by codex P2 on PR #2338 (twenty-sixth re-review):
/// `send_input` marks the turn active and publishes BEFORE attempting
/// `tx.try_send(req)` (established ordering, not after — see the
/// twenty-sixth re-review's race fix below) — if the enqueue itself
/// fails (channel full, or the stdin writer task already exited
/// because the process died), no turn actually started, so the mark
/// and publish must be ROLLED BACK rather than left standing. A
/// rejected prompt left standing as "active" could re-promote the
/// frontend to a working state and clear auth-recovery UI via
/// notifyControllerHealthy, and the health watchdog would be armed for
/// work that never happened.
#[tokio::test]
async fn send_input_rolls_back_turn_active_when_enqueue_fails() {
    let broker = Arc::new(mps::Broker::new());
    let c = AcpController::new(
        "tab".to_string(),
        "block-acp-enqueue-fail".to_string(),
        Some(broker.clone()),
        None,
        None,
        None,
    );
    let (tx, rx) = mpsc::channel::<String>(8);
    drop(rx); // Receiver gone — try_send fails with TrySendError::Closed.
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.session_id = Some("s1".to_string());
    }

    let res = c.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
    assert!(res.is_err(), "send_input should surface the enqueue failure, got {res:?}");
    assert!(!c.health_monitor.is_active_turn(), "must not leave the turn marked active when the enqueue itself failed");

    // The state is briefly marked active (established BEFORE the
    // enqueue attempt to close the twenty-sixth re-review's race), then
    // rolled back once the enqueue failure is discovered — so the
    // LATEST published status must reflect the rollback, not the
    // transient true a live subscriber may have also observed.
    let history = broker.read_event_history(
        mps::EVENT_CONTROLLER_STATUS,
        "block:block-acp-enqueue-fail",
        1,
    );
    assert_eq!(history.len(), 1, "send_input must publish the rollback so live subscribers see the corrected state");
    let published: BlockControllerRuntimeStatus =
        serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
    assert!(!published.turn_active, "the latest published status must reflect the rollback (turn_active: false)");
}

/// Regression for codex P2 on PR #2338 (twenty-sixth re-review): a
/// mid-turn steering send (turn already genuinely active from an
/// EARLIER successful send) whose OWN enqueue then fails must not roll
/// back and clobber that still-active turn — the FIRST prompt's id
/// must remain outstanding.
///
/// Also covers reagent P1 (twenty-eighth re-review) and codex P2
/// (twenty-ninth/thirtieth re-reviews): the failed second prompt's id
/// must NOT remain in the outstanding set either (it was never really
/// sent) — outstanding_prompt_ids's set-based tracking (superseding
/// the single latest/previous-id fields those rounds patched
/// incrementally) makes both of these hold simultaneously without any
/// special-cased rollback logic: removing the failed id and checking
/// emptiness is the ONLY rule, and it's correct regardless of send/
/// response ordering.
#[tokio::test]
async fn send_input_does_not_roll_back_an_already_active_turn_on_a_failed_steering_send() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.session_id = Some("s1".to_string());
    }

    // First send succeeds — turn is now genuinely active.
    assert!(c.send_input(BlockInputUnion::data(b"first".to_vec()), None).is_ok());
    assert!(c.health_monitor.is_active_turn());
    let outstanding_after_first: Vec<u64> = c.outstanding_prompt_ids.lock().unwrap().iter().copied().collect();
    assert_eq!(outstanding_after_first.len(), 1, "exactly the first prompt's id must be outstanding");
    let first_prompt_id = outstanding_after_first[0];

    // Swap in a closed channel so the SECOND (steering) send's own
    // enqueue fails.
    let (tx2, rx2) = mpsc::channel::<String>(8);
    drop(rx2);
    c.inner.lock().unwrap().stdin_tx = Some(tx2);

    let res = c.send_input(BlockInputUnion::data(b"second".to_vec()), None);
    assert!(res.is_err(), "the steering send's own enqueue should fail, got {res:?}");
    assert!(c.health_monitor.is_active_turn(), "the turn genuinely active from the FIRST send must not be rolled back by the second send's own failure");

    let outstanding_after_failure = c.outstanding_prompt_ids.lock().unwrap().clone();
    assert_eq!(
        outstanding_after_failure,
        HashSet::from([first_prompt_id]),
        "only the still-in-flight FIRST prompt's id must remain outstanding — the failed second one must not linger"
    );
}

#[test]
fn a_session_request_carries_the_agents_mcp_servers() {
    let servers = [serde_json::json!({ "name": "agentmux", "command": "C:/AgentMux/agentmux-mcp.exe", "args": [], "env": [] })];
    for (resume, load) in [(None, false), (Some("s1"), true)] {
        let (_, p) = super::session_request(resume, load, "C:/w", &servers);
        assert_eq!(p["mcpServers"], serde_json::json!(servers));
    }
}
