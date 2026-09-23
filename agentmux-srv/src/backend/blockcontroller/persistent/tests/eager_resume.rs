// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `start()`'s eager-resume path
//! (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`,
//! issue #3463). Covers the security property that made this a bigger change
//! than the original spec scoped: eager resume must go through the SAME
//! Layer 3 identity/credential spawn gate a live message send does
//! (`SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`), not spawn
//! unconditionally the moment a session id is found.

use super::super::*;
use crate::backend::obj::MetaMapType;
use crate::backend::storage::store::{
    AgentDefinition, AgentInstance, IdentityAccount, InstanceStatus, SecretRef,
};

fn make_store() -> Arc<Store> {
    Arc::new(Store::open_in_memory().unwrap())
}

/// A block with NO agent-instance row. `inject_identity_env`'s own Step 1
/// treats this as "outside the managed-credentials contract" and returns
/// `Ok(())` unconditionally — the simplest fixture for "the gate passes",
/// requiring no agent/account/binding setup at all.
fn meta_with_session(session_id: &str, cli_args: &[&str]) -> MetaMapType {
    let mut m = MetaMapType::new();
    m.insert(core::META_SESSION_ID.to_string(), serde_json::json!(session_id));
    m.insert("cmd".to_string(), serde_json::json!("node"));
    m.insert(
        "cmd:args".to_string(),
        serde_json::json!(cli_args.iter().collect::<Vec<_>>()),
    );
    m.insert("cmd:cwd".to_string(), serde_json::json!(""));
    m.insert("cmd:env".to_string(), serde_json::json!({}));
    m
}

/// A minimal `Block` + `AgentDefinition` + `IdentityAccount` +
/// `AgentInstance` wiring an oauth-class provider ("claude") bound to an
/// account whose `SecretRef` can never resolve — the same shape
/// `identity::resolver::inject::tests` uses to prove the gate blocks a
/// spawn (`MissingCredentials`). Duplicated here (not imported) because
/// that module's fixture helpers are private to its own `#[cfg(test)]`.
fn wire_ungated_agent(store: &Store, block_id: &str) {
    let mut block = crate::backend::obj::Block {
        oid: block_id.to_string(),
        parentoref: String::new(),
        version: 0,
        runtimeopts: None,
        stickers: None,
        meta: {
            let mut m = MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("agent"));
            m.insert("agentId".to_string(), serde_json::json!("def-1"));
            m
        },
        subblockids: None,
    };
    store.insert(&mut block).unwrap();

    let mut def = AgentDefinition {
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id: "def-1".to_string(),
        slug: String::new(),
        name: "T".to_string(),
        icon: "\u{2726}".to_string(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: String::new(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: String::new(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: 0,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: "[]".to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        auto_continue_enabled: 0,
        model_vendor_base_url: String::new(),
        memory_id: String::new(),
    };
    store.agent_def_insert(&mut def).unwrap();

    let bad = IdentityAccount {
        id: "acct-bad".to_string(),
        name: "claude-acct-bad".to_string(),
        provider: "claude".to_string(),
        kind: "pat".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::Env {
            env_var: "AGENTMUX_TEST_TOKEN_DOES_NOT_EXIST".to_string(),
        },
        context: serde_json::json!({}),
        status: "unknown".to_string(),
        created_at: 0,
        updated_at: 0,
    };
    store.identity_upsert(&bad).unwrap();
    store.agent_identity_link("def-1", "acct-bad", "claude").unwrap();

    let inst = AgentInstance {
        id: format!("inst-{block_id}"),
        definition_id: "def-1".to_string(),
        parent_instance_id: String::new(),
        block_id: block_id.to_string(),
        session_id: String::new(),
        status: InstanceStatus::Running.as_str().to_string(),
        github_context: String::new(),
        started_at: 0,
        ended_at: 0,
        created_at: 0,
        identity_id: "id-1".to_string(),
        memory_id: String::new(),
        instance_name: String::new(),
        working_directory: String::new(),
        display_hidden: false,
    };
    store.instance_create(&inst).unwrap();
}

fn controller(block_id: &str) -> PersistentSubprocessController {
    PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        None,
        None,
        None,
        None,
    )
}

async fn wait_for_spawn(c: &PersistentSubprocessController) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if c.inner.lock().unwrap().current_pid.is_some() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// `node` on PATH is how every other real-process test in this file
/// gates itself (see `shutdown_tests::stub_path`) — matched here for
/// consistency, not reinvented.
fn has_node() -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    })
}

const IDLE_STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
out({ type: "system", subtype: "init", session_id: "resumed-session" });
setInterval(() => {}, 1000);
"#;

fn write_stub() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("agentmux-eager-resume-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&path, IDLE_STUB).unwrap();
    path
}

/// Force-kills the wrapped controller's spawned process on drop —
/// including on a PANICKING unwind (a failed `assert!`), not just the
/// success path. Without this, an assertion added after a spawn (like
/// the `turn_active` one below) leaks the stub's `setInterval`-forever
/// node process on failure: on Windows that process holds the test
/// binary's inherited stdio pipes open, hanging the ENTIRE test
/// binary's shutdown rather than just failing this one test — observed
/// directly while mutation-testing that assertion.
struct KillOnDrop<'a>(&'a PersistentSubprocessController);
impl Drop for KillOnDrop<'_> {
    fn drop(&mut self) {
        // NOT `self.0.stop_process(true)`. That only SENDS a kill
        // request over a channel to an async task that does the actual
        // `child.kill()` — `Drop::drop` is sync and can't wait for that
        // task to run it, and if the test's own `#[tokio::test]`
        // runtime is tearing down at the same moment (exactly when a
        // test function is returning), that task can be dropped before
        // it ever processes the message, leaving the child alive
        // despite this guard. Confirmed live: with the async form, this
        // test module hung the whole test BINARY's shutdown on two of
        // three consecutive runs, always immediately after the first
        // test that reaches this guard via the success path. A direct,
        // synchronous OS-level kill by pid has no such gap.
        // `.unwrap_or_else(PoisonError::into_inner)`, not `.unwrap()`:
        // this Drop impl runs during a panicking unwind whenever a test
        // assertion fails while it holds `inner`'s lock (every
        // assertion in this module that pattern-matches `inner.resume`
        // does, since the match arms borrow from the guard). A plain
        // `.unwrap()` here would panic a SECOND time on the resulting
        // `PoisonError` — a panic during a panic's unwind, which Rust
        // escalates straight to `abort()`. Confirmed live: an
        // intentionally-failing assertion in this exact spot surfaced
        // as `STATUS_STACK_BUFFER_OVERRUN` with no test-failure message
        // printed at all, not a normal, readable assertion failure —
        // exactly this double-panic. The underlying data is still
        // valid after a poison (the panic happened elsewhere, this
        // struct's own fields are untouched); recovering it here is
        // safe and is what actually lets a future real assertion
        // failure in this module report itself normally.
        if let Some(pid) = self.0.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).current_pid {
            #[cfg(windows)]
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/PID", &pid.to_string()])
                .output();
            #[cfg(not(windows))]
            let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output();
        }
    }
}

#[tokio::test]
async fn does_not_spawn_when_no_session_id() {
    // Regression test: the lazy path (no agent:sessionid) must be
    // byte-for-byte unchanged, including when a controller HAS identity
    // stores configured — the branch decision is on the session id,
    // not on whether eager resume is theoretically possible.
    let store = make_store();
    let c = controller("blk-no-sid").with_identity_stores(Some(store.clone()), Some(store), "key".to_string());
    let meta = MetaMapType::new(); // no agent:sessionid
    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok());
    assert!(c.inner.lock().unwrap().current_pid.is_none());
}

#[tokio::test]
async fn does_not_spawn_when_identity_stores_not_configured() {
    // A controller constructed via plain `new()` (no
    // `with_identity_stores()`) must decline to eager-resume rather than
    // spawn WITHOUT the gate — the safe-by-default case.
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let stub = write_stub();
    let c = controller("blk-no-stores");
    let meta = meta_with_session("sid-123", &[stub.to_string_lossy().as_ref()]);
    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        c.inner.lock().unwrap().current_pid.is_none(),
        "must not spawn without identity stores configured, even though node was available"
    );
}

// `flavor = "multi_thread"`: this test's spawn attempt reaches
// `try_eager_resume`'s `tokio::task::block_in_place` call — which
// panics outright on the default current-thread test runtime (fewer
// than one worker thread to hand off to). Production always runs
// multi-threaded (`#[tokio::main]`, no flavor override), so this is
// what actually reproduces the real call context, not a workaround.
#[tokio::test(flavor = "multi_thread")]
async fn declines_when_gate_denies() {
    // The security property this whole change exists for: an agent
    // whose bound account can never resolve (the same shape a deleted/
    // revoked account produces) must NOT be eagerly spawned, even though
    // node is on PATH and the session id is present.
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store();
    wire_ungated_agent(&store, "blk-ungated");
    let stub = write_stub();
    let c = controller("blk-ungated").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    // mstore is also required — set directly since `new()` takes it as a
    // constructor arg, not the builder.
    let c = PersistentSubprocessController {
        mstore: Some(store),
        ..c
    };
    let meta = meta_with_session("sid-ungated", &[stub.to_string_lossy().as_ref()]);
    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok(), "declining eager-resume must not fail the resync");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        c.inner.lock().unwrap().current_pid.is_none(),
        "the identity gate must have blocked the spawn"
    );
    // codex P1 on PR #3513 (the spawn-claim race): the exclusive claim
    // taken before the gate runs MUST be released on a decline, not
    // just on success. A leaked claim here would be worse than the
    // original bug — instead of a pane that revives on the next
    // message, it would refuse EVERY future message forever
    // (`decide_send_action` treats `spawning_in_progress: true` as
    // "something is already spawning" indefinitely).
    assert!(
        !c.inner.lock().unwrap().spawning_in_progress,
        "the spawn claim must be released when the gate declines, or this pane can never send again"
    );
}

// See `declines_when_gate_denies`'s comment on `flavor = "multi_thread"`.
#[tokio::test(flavor = "multi_thread")]
async fn spawns_with_resume_flag_when_gate_passes() {
    // The happy path, end to end: a real process spawn, through the
    // SAME meta-reading code (`cmd`/`cmd:args`/`cmd:cwd`/`cmd:env`) a
    // live message send uses, gated by a real (trivially-passing)
    // identity check — not a shortcut around either.
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store(); // no instance row for this block => gate Step 1 passes trivially
    let stub = write_stub();
    let c = controller("blk-eager").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController {
        mstore: Some(store),
        ..c
    };
    let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
    let _kill_on_drop = KillOnDrop(&c);

    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok(), "{result:?}");
    assert!(wait_for_spawn(&c).await, "expected a real process to spawn");
    assert_eq!(c.inner.lock().unwrap().session_id.as_deref(), Some("resumed-session"));

    // codex P1 on PR #3513: no message was ever sent, so this pane must
    // read as idle, not perpetually WORKING. `spawn_process` used to
    // mark a turn active unconditionally on every spawn — correct for
    // every OTHER caller (a message is always about to flow through
    // one way or another) but wrong for eager resume, which revives a
    // session so it's ready for the next message, not mid-turn.
    assert!(
        !c.health_monitor.is_active_turn(),
        "an eager resume with nothing queued must not report an active turn"
    );
    // codex P1 on PR #3513 (the spawn-claim race): the SUCCESS path
    // must also release the claim, via `drain_queue_after_successful_
    // spawn`. A leaked claim here would refuse every future message to
    // this pane forever. That release happens inside a `tokio::spawn`ed
    // task, not synchronously before `start()` returns — `current_pid`
    // (what `wait_for_spawn` polls) is set synchronously inside
    // `spawn_process` itself, well before that task is even scheduled,
    // so this needs its own short wait rather than piggybacking on
    // `wait_for_spawn`'s already-satisfied condition.
    let claim_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if !c.inner.lock().unwrap().spawning_in_progress {
            break;
        }
        assert!(std::time::Instant::now() < claim_deadline, "spawn claim was never released");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // `_kill_on_drop` force-kills the stub on scope exit, success or
    // panic — see `KillOnDrop`'s own doc comment.
}

// See `declines_when_gate_denies`'s comment on `flavor = "multi_thread"`.
#[tokio::test(flavor = "multi_thread")]
async fn eager_resume_config_carries_the_resume_flag_and_session_id() {
    // Direct proof the built PersistentSpawnConfig is correct: the stub
    // writes its OWN received argv to a file before printing anything,
    // so this reads back exactly what the process was launched with —
    // no dependency on stdout-parsing timing (unlike asserting on
    // `session_id`, which is only captured after the init line is read
    // and races the pid becoming visible).
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let argv_out = std::env::temp_dir().join(format!("agentmux-eager-resume-argv-{}.json", uuid::Uuid::new_v4()));
    let echo_argv_stub = std::env::temp_dir().join(format!("agentmux-eager-resume-echo-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(
        &echo_argv_stub,
        format!(
            r#"
require("fs").writeFileSync({argv_out:?}, JSON.stringify(process.argv.slice(2)));
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
out({{ type: "system", subtype: "init", session_id: "echoed-session" }});
setInterval(() => {{}}, 1000);
"#,
            argv_out = argv_out.to_string_lossy(),
        ),
    )
    .unwrap();

    let store = make_store();
    let c = controller("blk-echo").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController {
        mstore: Some(store),
        ..c
    };
    let meta = meta_with_session("sid-to-resume", &[echo_argv_stub.to_string_lossy().as_ref()]);
    let _kill_on_drop = KillOnDrop(&c);

    Controller::start(&c, meta, None, false).unwrap();
    assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let argv: Vec<String> = loop {
        if let Ok(raw) = std::fs::read_to_string(&argv_out) {
            if let Ok(parsed) = serde_json::from_str(&raw) {
                break parsed;
            }
        }
        if std::time::Instant::now() >= deadline {
            panic!("stub never wrote its argv file");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(
        argv.windows(2).any(|w| w[0] == "--resume" && w[1] == "sid-to-resume"),
        "expected --resume sid-to-resume in argv, got {argv:?}"
    );

    // `_kill_on_drop` force-kills the stub on scope exit, success or
    // panic — see `KillOnDrop`'s own doc comment.
}

// codex P1 on PR #3513: the controller is placed in the global registry
// before `start()` runs, so a concurrent message can reach
// `send_message` while `try_eager_resume` is still resolving the
// identity gate. Without a claim held for that whole window, this
// concurrent message would see `stdin_tx: None` and
// `spawning_in_progress: false` and become its own spawner — a SECOND
// process launched against the same `--resume <sid>`, silently
// overwriting the first one's `current_pid`/`stdin_tx`/`kill_tx` while
// it stays alive on the same conversation.
//
// Doesn't need real timing/concurrency to prove: `spawning_in_progress:
// true` IS the exact state `try_eager_resume` holds for that whole
// window (see the claim block at the top of that method). Setting it
// directly and calling `send_message` exercises the same
// `decide_send_action` branch a genuinely concurrent message would hit.
#[test]
fn a_message_arriving_while_spawning_in_progress_queues_instead_of_double_spawning() {
    let c = controller("blk-concurrent");
    c.inner.lock().unwrap().spawning_in_progress = true;

    let msg = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let result = c.send_message(msg.to_string(), config);

    assert!(result.is_ok(), "a queued message is accepted, not an error: {result:?}");
    assert!(
        c.inner.lock().unwrap().current_pid.is_none(),
        "must NOT spawn a second process while one is already claimed as spawning"
    );
    assert_eq!(
        c.inner.lock().unwrap().pending_send_messages.len(),
        1,
        "the message must be queued for the in-flight spawn to drain, not dropped"
    );
}

// Direct coverage for `try_claim_eager_resume_spawn` itself — see its
// own doc comment for why this exists as a separate test rather than
// trusting the end-to-end tests above to exercise it: mutation-tested,
// deleting `try_eager_resume`'s call to this function (or the function's
// own body) is exactly what those tests failed to catch.
#[test]
fn claim_succeeds_when_nothing_else_is_running_or_spawning() {
    let c = controller("blk-claim-free");
    assert!(c.try_claim_eager_resume_spawn());
    assert!(c.inner.lock().unwrap().spawning_in_progress);
}

#[test]
fn claim_fails_when_spawning_is_already_in_progress() {
    let c = controller("blk-claim-spawning");
    c.inner.lock().unwrap().spawning_in_progress = true;
    assert!(!c.try_claim_eager_resume_spawn());
}

#[test]
fn claim_fails_when_a_stdin_channel_is_already_live() {
    // A live `stdin_tx` means the process is already running — eager
    // resume must not attempt a second spawn just because
    // `spawning_in_progress` happens to be false at this instant (e.g.
    // between an earlier spawn completing and the caller having set
    // anything else).
    let (tx, _rx) = mpsc::channel::<String>(1);
    let c = controller("blk-claim-live");
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    assert!(!c.try_claim_eager_resume_spawn());
}

#[test]
fn claim_fails_during_a_drain() {
    let c = controller("blk-claim-draining");
    c.inner.lock().unwrap().drain_claim = true;
    assert!(!c.try_claim_eager_resume_spawn());
}

// End-to-end wiring check, deterministic rather than timing-dependent:
// the four tests above prove `try_claim_eager_resume_spawn` itself is
// correct in isolation, but none of the OTHER eager-resume tests would
// have caught `try_eager_resume` simply never calling it — confirmed by
// mutation, removing that one call left every other test in this module
// still passing. This drives the claim through the REAL `start()` entry
// point instead of calling the claim function directly, so a future
// regression that skips the call (not just breaks the function) fails
// here.
//
// Pre-claiming `spawning_in_progress` before `start()` runs stands in
// for "another spawn raced in first" without needing to actually win a
// timing race: same effect the eager-resume path itself achieves by
// claiming BEFORE its identity gate work, on a real concurrent message.
// The identity gate and stub here are otherwise IDENTICAL to
// `spawns_with_resume_flag_when_gate_passes`, which proves this exact
// setup DOES spawn when unclaimed — so `current_pid` staying `None`
// here is real evidence of the early decline, not an artifact of a
// broken fixture.
#[tokio::test(flavor = "multi_thread")]
async fn start_declines_the_whole_attempt_when_the_claim_is_already_held() {
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store();
    let stub = write_stub();
    let c = controller("blk-preclaimed").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController {
        mstore: Some(store),
        ..c
    };
    c.inner.lock().unwrap().spawning_in_progress = true;

    let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
    // Correct behavior never spawns anything here, so there's normally
    // nothing for this to clean up — it's for a FUTURE regression that
    // reintroduces the bug this test catches: without it, that failure
    // mode is a leaked real process hanging the whole test binary's
    // shutdown (see `KillOnDrop`'s own doc comment), not a clean
    // assertion failure.
    let _kill_on_drop = KillOnDrop(&c);
    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok(), "declining must not fail the resync: {result:?}");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        c.inner.lock().unwrap().current_pid.is_none(),
        "must not spawn at all while another spawn's claim is already held"
    );
}

// codex P1 + reagent P1 on PR #3538, found independently by both. A
// message arriving while the identity gate runs is routed to `Queued`
// (the claim is held) and its caller told "accepted". If the gate then
// DECLINES, the prompt is not about to run — and this used to be
// completely silent.
//
// Both reviewers proposed handing off to
// `respawn_once_for_leftover_queue`. That is deliberately not done: the
// gate declining means the credentials were refused, so respawning would
// launch the process the gate just denied. The queue is therefore left in
// place (a later send re-runs the gate and drains the whole backlog once
// credentials are fixed) and the operator is told why nothing is running.
#[tokio::test(flavor = "multi_thread")]
async fn a_gate_decline_with_queued_work_surfaces_a_failure_and_keeps_the_queue() {
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store();
    wire_ungated_agent(&store, "blk-gate-strand"); // bindings that can never resolve
    let stub = write_stub();
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let c = controller("blk-gate-strand").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController {
        mstore: Some(store),
        broker: Some(broker.clone()),
        ..c
    };
    let _kill_on_drop = KillOnDrop(&c);

    // A prompt accepted while the claim was held, exactly as
    // `decide_send_action`'s `Queued` branch would leave it.
    {
        let mut inner = c.inner.lock().unwrap();
        let seq = inner.take_next_message_seq();
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(seq, "{\"accepted\":\"prompt\"}".to_string()));
    }

    let meta = meta_with_session("sid-gate-strand", &[stub.to_string_lossy().as_ref()]);
    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok(), "declining must not fail the resync: {result:?}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // The security property first: the gate refused, so nothing spawned.
    assert!(
        c.inner.lock().unwrap().current_pid.is_none(),
        "a refused credential gate must never spawn, queued work or not"
    );
    assert!(
        !c.inner.lock().unwrap().spawning_in_progress,
        "the spawn claim must still be released"
    );

    // The queue survives — not dropped, and not delivered by bypassing
    // the gate. A later send re-runs the gate and drains it.
    assert_eq!(
        c.inner.lock().unwrap().pending_send_messages.len(),
        1,
        "the accepted prompt must stay queued for a properly-gated later spawn"
    );

    // And it is no longer silent.
    let failures = broker.read_event_history(
        crate::backend::mps::EVENT_AGENT_FAILURE,
        &format!("block:{}", "blk-gate-strand"),
        10,
    );
    assert!(
        !failures.is_empty(),
        "the operator must be told why an accepted prompt is not running"
    );
}

// reagent P1 on PR #3523. The success path used to check "is anything
// queued?" and kick off the drain as two SEPARATE lock acquisitions,
// holding the spawn claim across the gap. A `send_message` landing in
// that gap is routed to `Queued` — which deliberately does NOT publish
// turn-active, because the spawn-claim holder is supposed to — and the
// drain loop does not publish it either. So the message was delivered
// while the pane, Swarm view and subagent watcher all read idle.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_queue_eager_resume_releases_its_claim_before_returning() {
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store();
    let stub = write_stub();
    let c = controller("blk-claim-sync").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController { mstore: Some(store), ..c };
    let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
    let _kill_on_drop = KillOnDrop(&c);

    let result = Controller::start(&c, meta, None, false);
    assert!(result.is_ok(), "{result:?}");
    assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

    // Synchronously — deliberately NOT a polling loop. "Eventually
    // released, one tokio task hop later" is precisely the window the
    // bug lived in, so a test that waits for it would still pass
    // against the broken version.
    assert!(
        !c.inner.lock().unwrap().spawning_in_progress,
        "an empty-queue eager resume must release its spawn claim before start() returns"
    );

    // What that release buys: the next message is delivered by its own
    // caller — and `DeliverDirect` publishes turn-active itself —
    // instead of being queued behind a claim whose holder has already
    // decided not to publish for it.
    assert!(
        matches!(
            c.decide_send_action("{\"probe\":true}", None),
            SendAction::DeliverDirect
        ),
        "with the claim released and stdin live, the next send must go direct"
    );

    assert!(
        !c.health_monitor.is_active_turn(),
        "an eager resume with nothing queued must still not report an active turn"
    );
}

// codex P1 + reagent P1 on PR #3523, found independently by both: the
// SIBLING of the test below. That one covers `send_message`'s
// `DeliverDirect`, the path a human typing in the UI takes. This covers
// `send_user_message` — the MuxBus/reactive injection path
// (`deliver_agent_message`), which writes straight to stdin and was
// never appended to the retry batch at all.
//
// Same consequence, worse optics: the caller was told delivery
// succeeded and the transcript already rendered the message to the
// operator, so a stale resume silently loses a prompt everyone has been
// told landed.
#[tokio::test(flavor = "multi_thread")]
async fn an_injected_message_after_eager_resume_is_tracked_for_stale_resume_retry() {
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store();
    let stub = write_stub();
    let c = controller("blk-track-injected").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController { mstore: Some(store), ..c };
    let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
    let _kill_on_drop = KillOnDrop(&c);

    Controller::start(&c, meta, None, false).unwrap();
    assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

    // Precondition: an unconfirmed `--resume` with an empty batch — the
    // state in which losing an injected message is possible at all.
    {
        let inner = c.inner.lock().unwrap();
        match &inner.resume {
            persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
                assert!(retry.messages.is_empty(), "nothing delivered yet");
            }
            other => panic!("expected AwaitingOutcome, got {other:?}"),
        }
    }

    // Raw text, not a stream-json envelope — `send_user_message` does
    // the wrapping itself, same as `send_message`.
    let msg = "injected from muxbus";
    c.send_user_message(msg.to_string()).unwrap();

    let inner = c.inner.lock().unwrap();
    match &inner.resume {
        persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
            assert_eq!(
                retry.messages.len(),
                1,
                "the injected message must be tracked into the retry batch, not silently dropped"
            );
            let tracked: serde_json::Value = serde_json::from_str(&retry.messages[0].json)
                .expect("tracked entry must be valid JSON");
            assert_eq!(tracked["message"]["content"], msg);
        }
        other => panic!("expected AwaitingOutcome with the tracked message, got {other:?}"),
    }
}

// codex P1 on PR #3513 (re-review): eager resume attempts `--resume
// <sid>` but starts with no message of its own to seed the retry batch
// with (unlike every other resume-respawn, which always has one). If
// that resume turns out to be stale and a direct message arrives before
// the failure is detected, the message must still be tracked so a
// confirmed stale-resume retry can redeliver it — otherwise it is
// silently lost when the doomed process exits. Proven at the STATE
// level rather than by actually killing the process and racing the
// stderr reader: this is really two composed fixes (spawn_process's
// event-routing, and DeliverDirect's own tracking call), and asserting
// on `ResumeState` directly pins each independently of the other's
// timing.
#[tokio::test(flavor = "multi_thread")]
async fn a_direct_message_after_eager_resume_is_tracked_for_stale_resume_retry() {
    if !has_node() {
        eprintln!("eager_resume_tests: `node` not on PATH — skipping");
        return;
    }
    let store = make_store();
    let stub = write_stub();
    let c = controller("blk-track-direct").with_identity_stores(
        Some(store.clone()),
        Some(store.clone()),
        "key".to_string(),
    );
    let c = PersistentSubprocessController {
        mstore: Some(store),
        ..c
    };
    let meta = meta_with_session("resumed-session", &[stub.to_string_lossy().as_ref()]);
    let _kill_on_drop = KillOnDrop(&c);

    Controller::start(&c, meta, None, false).unwrap();
    assert!(wait_for_spawn(&c).await, "expected a real process to spawn");

    // The resume attempt must be tracked as AwaitingOutcome (not
    // NotTracking/SpawnedFresh) even with nothing delivered yet — this
    // is the `spawn_process` routing half of the fix.
    {
        let inner = c.inner.lock().unwrap();
        match &inner.resume {
            persistent_resume::ResumeState::AwaitingOutcome { attempted_sid, retry, .. } => {
                assert_eq!(attempted_sid, "resumed-session");
                assert!(retry.messages.is_empty(), "no seed message — nothing delivered yet");
            }
            other => panic!("expected AwaitingOutcome with no messages yet, got {other:?}"),
        }
    }

    // A direct message now must land in that SAME tracking state — the
    // DeliverDirect half of the fix.
    //
    // Plain raw text, NOT a pre-formatted stream-json envelope —
    // `send_message` does that wrapping itself (`content: message`).
    // Every OTHER test in this file passing a full envelope as `message`
    // (e.g. `shutdown_tests::start()`) gets away with it because those
    // stubs only ever check the outer `m.type`, never `m.message.
    // content` — so the resulting double-wrap is invisible to them.
    // THIS test asserts on the tracked entry's actual content, so it
    // needs the API used as a real caller (a human typing "hi") would.
    let msg = "hi";
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.send_message(msg.to_string(), config).unwrap();

    let inner = c.inner.lock().unwrap();
    match &inner.resume {
        persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
            assert_eq!(
                retry.messages.len(),
                1,
                "the direct message must be tracked into the retry batch, not silently dropped"
            );
            // Parse and check the semantic content rather than hardcode
            // `send_message`'s exact wrapped-string format — this proves
            // the RIGHT message was tracked without this test being
            // fragile to that internal format ever changing.
            let tracked: serde_json::Value = serde_json::from_str(&retry.messages[0].json)
                .expect("tracked entry must be valid JSON");
            assert_eq!(tracked["message"]["content"], msg);
        }
        other => panic!("expected AwaitingOutcome with the tracked message, got {other:?}"),
    }
}

fn bogus_config(session_id: &str) -> PersistentSpawnConfig {
    PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: session_id.to_string(),
        message_id: None,
    }
}

/// The predicate and the producer live together in `mod.rs`; this pins
/// that they agree on both variants and reject an unrelated failure.
#[test]
fn is_held_elsewhere_error_recognises_both_refusals_and_nothing_else() {
    assert!(is_held_elsewhere_error(&held_elsewhere_error("blk-other", false)));
    assert!(is_held_elsewhere_error(&held_elsewhere_error("blk-other", true)));
    assert!(!is_held_elsewhere_error("stdin send failed: channel closed"));
    assert!(!is_held_elsewhere_error("failed to spawn: No such file or directory"));
}

/// Codex P1 on PR #3551 (rounds one and three): an eager resume refused by
/// the duplicate-session guard must NOT fall back to a fresh spawn for the
/// prompt that queued during the attempt — that hands it to a blank
/// conversation while the real one is open next door — and must not
/// retain it either, since the next send's generic failed-spawn fallback
/// would do the same thing later. Release the claim, discard the queue,
/// keep the session id, report what was discarded and why.
#[tokio::test(flavor = "multi_thread")]
async fn an_ownership_refusal_with_queued_work_reports_and_discards_the_queue_instead_of_spawning_fresh() {
    let store = make_store();
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let c = PersistentSubprocessController {
        mstore: Some(store),
        broker: Some(broker.clone()),
        filestore: Some(filestore.clone()),
        ..controller("blk-owned-elsewhere")
    };
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true; // the eager attempt's claim
        inner.session_id = Some("sid-owned".to_string());
        let seq = inner.take_next_message_seq();
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(seq, "{\"accepted\":\"prompt\"}".to_string()));
    }

    c.settle_eager_spawn_failure(&held_elsewhere_error("blk-other", false), bogus_config("sid-owned"));

    let inner = c.inner.lock().unwrap();
    assert!(inner.current_pid.is_none(), "must not spawn anything");
    assert!(!inner.spawning_in_progress, "the claim must be released");
    assert!(
        inner.pending_send_messages.is_empty(),
        "the prompt is discarded, not retained: a retained leftover is exactly what the next \
         send's generic failed-spawn fallback would hand to a fresh conversation"
    );
    assert_eq!(
        inner.session_id.as_deref(),
        Some("sid-owned"),
        "the session id must survive — clearing it is exactly the bypass"
    );
    drop(inner);
    // An ownership refusal is not a failure CLASS (`agents::failure` keys on
    // credential/config phrases, and this is neither), so the report is the
    // error-result frame appended to the pane's output — the same channel
    // the CLI's own errors arrive on.
    let output = filestore
        .read_file("blk-owned-elsewhere", PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .unwrap_or_default();
    assert!(
        output.contains("1 queued prompt(s) were not delivered and have been discarded")
            && output.contains("already open in another pane"),
        "the operator must be told what was discarded and why; got: {output}"
    );
}

/// The other failure class keeps its fallback: a generic spawn failure with
/// queued work still hands off to `respawn_once_for_leftover_queue`, whose
/// first act is clearing the session id for a fresh start.
#[tokio::test(flavor = "multi_thread")]
async fn a_generic_spawn_failure_with_queued_work_still_falls_back_to_a_fresh_respawn() {
    let c = controller("blk-generic-failure");
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        inner.session_id = Some("sid-any".to_string());
        let seq = inner.take_next_message_seq();
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(seq, "{\"accepted\":\"prompt\"}".to_string()));
    }

    c.settle_eager_spawn_failure("failed to spawn: No such file or directory", bogus_config("sid-any"));

    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.session_id, None, "the fallback respawn clears the session id for a fresh start");
    assert!(!inner.spawning_in_progress, "a failed fallback releases the claim too");
    assert_eq!(inner.pending_send_messages.len(), 1, "an undeliverable prompt is still not dropped");
}
