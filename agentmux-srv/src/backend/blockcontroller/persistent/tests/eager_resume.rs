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
        let _ = self.0.stop_process(true);
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
