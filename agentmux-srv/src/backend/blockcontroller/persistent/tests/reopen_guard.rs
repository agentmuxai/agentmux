// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One session, one process: a spawn that would `--resume` a session another
//! live process is still on is refused (pane-close spec §4.7).

use super::super::*;
use crate::backend::obj::Block;

fn resume_config(sid: &str) -> PersistentSpawnConfig {
    PersistentSpawnConfig {
        // Never actually spawned when the guard refuses; "git" (a real
        // executable on every CI platform) keeps the allowed case honest.
        cli_command: "git".to_string(),
        cli_args: vec!["--version".to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: sid.to_string(),
        message_id: None,
    }
}

fn controller(block_id: &str, mstore: Option<Arc<Store>>) -> Arc<PersistentSubprocessController> {
    Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        None,
        None,
        mstore,
        None,
    ))
}

const MSG: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;

#[tokio::test]
async fn a_session_live_in_another_pane_is_not_resumed_twice() {
    let sid = format!("sid-{}", uuid::Uuid::new_v4());
    let live_block = format!("live-{}", uuid::Uuid::new_v4());
    let live = controller(&live_block, None);
    {
        let mut g = live.inner.lock().unwrap();
        g.session_id = Some(sid.clone());
        g.current_pid = Some(4242);
    }
    crate::backend::blockcontroller::register_controller(&live_block, live.clone());

    let reopen = controller(&format!("reopen-{}", uuid::Uuid::new_v4()), None);
    let err = reopen.send_message(MSG.to_string(), resume_config(&sid)).unwrap_err();
    crate::backend::blockcontroller::delete_controller(&live_block);

    assert!(err.contains("already open in another pane"), "got: {err}");
    assert!(reopen.inner.lock().unwrap().current_pid.is_none(), "no second process");
}

#[tokio::test]
async fn a_session_whose_process_is_still_closing_is_not_resumed_yet() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let sid = format!("sid-{}", uuid::Uuid::new_v4());
    let closing_block = format!("closing-{}", uuid::Uuid::new_v4());
    let mut block = Block {
        oid: closing_block.clone(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert(core::META_SESSION_ID.to_string(), serde_json::json!(sid));
            m
        },
        subblockids: None,
    };
    store.insert(&mut block).unwrap();
    crate::backend::blockcontroller::mark_closing(&closing_block);

    let reopen = controller(&format!("reopen-{}", uuid::Uuid::new_v4()), Some(store.clone()));
    let err = reopen.send_message(MSG.to_string(), resume_config(&sid)).unwrap_err();
    assert!(err.contains("still shutting down"), "got: {err}");

    // Once that process has exited, the same reopen is allowed.
    crate::backend::blockcontroller::mark_closing_stopped(&closing_block);
    let allowed = controller(&format!("reopen-{}", uuid::Uuid::new_v4()), Some(store));
    let result = allowed.send_message(MSG.to_string(), resume_config(&sid));
    crate::backend::blockcontroller::unmark_closing(&closing_block);
    assert!(result.is_ok(), "got: {result:?}");
}

/// reagent P1 on #3421: check-then-spawn must be atomic across
/// controllers. Several reopens of one session at once — exactly one may
/// win. Uses a node process that stays alive (EOF-only exit), so the
/// winner's `current_pid` stays set while the others check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reopens_of_one_session_start_exactly_one_process() {
    let has_node = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    });
    if !has_node {
        eprintln!("reopen_guard_tests: `node` not on PATH — skipping");
        return;
    }
    let stub = std::env::temp_dir().join(format!("agentmux-reopen-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&stub, "process.stdin.on('data', () => {}); process.stdin.on('end', () => process.exit(0));").unwrap();

    let sid = format!("sid-{}", uuid::Uuid::new_v4());
    let controllers: Vec<Arc<PersistentSubprocessController>> =
        (0..4).map(|i| controller(&format!("race-{i}-{}", uuid::Uuid::new_v4()), None)).collect();
    for c in &controllers {
        crate::backend::blockcontroller::register_controller(&c.block_id, c.clone());
    }
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![stub.to_string_lossy().to_string()],
        ..resume_config(&sid)
    };

    let barrier = Arc::new(std::sync::Barrier::new(controllers.len()));
    let handles: Vec<_> = controllers
        .iter()
        .map(|c| {
            let (c, config, barrier) = (c.clone(), config.clone(), barrier.clone());
            tokio::task::spawn_blocking(move || {
                barrier.wait();
                c.send_message(MSG.to_string(), config)
            })
        })
        .collect();
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.unwrap());
    }

    for c in &controllers {
        let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(3)).await;
        crate::backend::blockcontroller::delete_controller(&c.block_id);
    }
    let _ = std::fs::remove_file(stub);

    let ok = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(ok, 1, "exactly one reopen may win; got {results:?}");
    assert!(
        results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.contains("already open in another pane")),
        "the others are refused by the guard: {results:?}"
    );
}
