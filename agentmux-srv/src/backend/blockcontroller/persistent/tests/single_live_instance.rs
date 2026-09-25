// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One live instance per agent, host tier
//! (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md Phase 1): a persistent
//! pane holds its agent's lease for its CLI process's lifetime, is refused
//! while another instance holds it, and stops driving once it loses it.

use super::super::*;

const MSG: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;

fn config(uid: Option<&str>) -> PersistentSpawnConfig {
    // No `AGENTMUX_AGENT_ID`: with one, a real spawn auto-registers that name
    // in the HOST-GLOBAL shared reactive registry (`~/.agentmux/shared`),
    // outside any temp dir — an earlier revision of this test left a stray
    // `agent3/stable.json` there. The refusal text then says "This agent".
    let mut env_vars = HashMap::new();
    if let Some(uid) = uid {
        env_vars.insert("AGENTMUX_AGENT_UID".to_string(), uid.to_string());
    }
    PersistentSpawnConfig {
        // A real executable on every CI platform that exits at once.
        cli_command: "git".to_string(),
        cli_args: vec!["--version".to_string()],
        working_dir: String::new(),
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: String::new(),
        session_id: String::new(),
        message_id: None,
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    registry: Arc<crate::registry::Registry>,
    uid: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::Registry::open(tmp.path().to_path_buf()).unwrap());
        Self { _tmp: tmp, registry, uid: format!("uid-{}", uuid::Uuid::new_v4()) }
    }

    fn store(&self) -> Arc<crate::registry::LeaseStore> {
        crate::backend::agent_admission::lease_store_for(Some(Arc::clone(&self.registry))).unwrap()
    }

    fn lease_file(&self) -> std::path::PathBuf {
        self.registry.root().join("leases").join(format!("{}.lease.json", self.uid))
    }

    fn controller(&self, boot: &str) -> Arc<PersistentSubprocessController> {
        Arc::new(
            PersistentSubprocessController::new(
                "tab".to_string(),
                format!("block-{}", uuid::Uuid::new_v4()),
                None,
                None,
                None,
                None,
            )
            .with_agent_lease_store(Some(Arc::clone(&self.registry)), Arc::from(boot)),
        )
    }
}

/// The 2026-09-25 incident at the spawn choke point: another instance is
/// live on this agent, so this pane starts no process at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_instance_of_the_agent_is_refused_before_spawning() {
    let f = Fixture::new();
    let other = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("other-srv"), "other-block", None, |_| {},
    )
    .unwrap();

    let ctrl = f.controller("this-srv");
    let err = ctrl.send_message(MSG.to_string(), config(Some(&f.uid))).unwrap_err();

    assert!(err.contains("This agent is already running in another AgentMux instance"), "got: {err}");
    let g = ctrl.inner.lock().unwrap();
    assert!(g.current_pid.is_none(), "no process may be spawned");
    assert!(g.agent_lease.is_none());
    drop(g);
    drop(other);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_spawn_holds_the_lease_until_its_process_exits() {
    let f = Fixture::new();
    let ctrl = f.controller("this-srv");
    ctrl.send_message(MSG.to_string(), config(Some(&f.uid))).unwrap();

    // `git --version` exits almost at once; the exit arm releases the lease.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while f.lease_file().exists() || ctrl.inner.lock().unwrap().agent_lease.is_some() {
        assert!(std::time::Instant::now() < deadline, "lease not released after the process exited");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // And the agent is free for another instance again.
    assert!(crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("other-srv"), "other-block", None, |_| {},
    )
    .is_ok());
}

/// Fencing (Codex P1 on #3730): a holder that lost the agent — suspended
/// past the TTL and taken over — starts no turn and its process is stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pane_that_lost_its_lease_starts_no_turn_and_is_stopped() {
    let f = Fixture::new();
    let ctrl = f.controller("this-srv");
    let held = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("this-srv"), "block", None, |_| {},
    )
    .unwrap();
    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<KillRequest>();
    {
        let mut g = ctrl.inner.lock().unwrap();
        g.agent_lease = Some(Arc::new(held));
        g.kill_tx = Some(kill_tx);
        g.current_pid = Some(4242);
    }

    // Another instance takes the agent over (as after a TTL reclaim).
    std::fs::remove_file(f.lease_file()).unwrap();
    let _new = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("other-srv"), "other-block", None, |_| {},
    )
    .unwrap();

    let err = ctrl.send_user_message("next prompt".to_string()).unwrap_err();
    assert!(err.contains("taken over"), "got: {err}");
    assert!(matches!(kill_rx.try_recv(), Ok(KillRequest::Force)), "the process must be stopped");
    // The spawn path refuses too, rather than reusing the lost lease.
    assert!(ctrl.send_message(MSG.to_string(), config(Some(&f.uid))).is_err());
}

/// A spawn env with no agent UID (no `db_agents` row yet) runs as before
/// Phase 1 — unguarded, logged — rather than being refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spawn_without_an_agent_uid_takes_no_lease() {
    let f = Fixture::new();
    let ctrl = f.controller("this-srv");
    ctrl.send_message(MSG.to_string(), config(None)).unwrap();
    assert!(ctrl.inner.lock().unwrap().agent_lease.is_none());
    assert!(!f.lease_file().exists());
}

/// The early check `run_agent_turn` / eager resume run before any shared
/// write (spec I9) refuses while another instance holds the agent, and
/// passes for the holder itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_early_admission_check_refuses_while_the_agent_is_live_elsewhere() {
    let f = Fixture::new();
    let _other = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("other-srv"), "other-block", None, |_| {},
    )
    .unwrap();
    let err = f.controller("this-srv").check_admission(&f.uid, "Agent3").await.unwrap_err();
    assert!(err.contains("already running"), "got: {err}");
    assert!(f.controller("other-srv").check_admission(&f.uid, "Agent3").await.is_ok());
}
