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
        self.controller_in(boot, format!("block-{}", uuid::Uuid::new_v4()))
    }

    /// A controller for a block id the store can address (`ORef` wants a bare UUID).
    fn controller_in(&self, boot: &str, block_id: String) -> Arc<PersistentSubprocessController> {
        Arc::new(
            PersistentSubprocessController::new(
                "tab".to_string(),
                block_id,
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

/// The 2026-10-27 incident: the agent's live pane is in THIS process (same boot id), so the lease
/// store's "other instance" read does not see it. The early check names the pane instead, and does
/// so before the spawn env is built.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_early_admission_check_refuses_while_another_pane_of_this_process_holds_the_agent() {
    let f = Fixture::new();
    let live = f.controller("this-srv");
    let held = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("this-srv"), &live.block_id, None, |_| {},
    )
    .unwrap();
    live.inner.lock().unwrap().agent_lease = Some(Arc::new(held));
    crate::backend::blockcontroller::register_controller(&live.block_id, live.clone());

    let duplicate = f.controller("this-srv");
    let err = duplicate.check_admission(&f.uid, "Agent3").await.unwrap_err();
    let holder = crate::agents::failure::open_in_other_pane_holder(&err);
    // The holder pane itself, and an agent nobody holds, pass.
    let own = live.check_admission(&f.uid, "Agent3").await;
    let free = duplicate.check_admission("uid-nobody-holds", "Agent3").await;
    crate::backend::blockcontroller::delete_controller(&live.block_id);

    assert!(is_held_elsewhere_error(&err), "got: {err}");
    assert_eq!(holder.as_deref(), Some(live.block_id.as_str()), "names the live pane: {err}");
    assert!(own.is_ok(), "the holder is not refused by itself");
    assert!(free.is_ok());
}

/// Move here: the holder pane is told to stop (gracefully), the claimant waits until the agent is
/// free, and the holder pane is left with the same notice, pointing back at the claimant.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moving_the_agent_here_stops_the_holder_pane_and_waits_for_its_lease() {
    let f = Fixture::new();
    let state = crate::server::tests::test_state();
    let holder = f.controller_in("this-srv", uuid::Uuid::new_v4().to_string());
    let claimant = f.controller_in("this-srv", uuid::Uuid::new_v4().to_string());
    let held = crate::backend::agent_admission::acquire(
        &f.store(), &f.uid, "Agent3", &Arc::from("this-srv"), &holder.block_id, None, |_| {},
    )
    .unwrap();
    let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<KillRequest>();
    {
        let mut g = holder.inner.lock().unwrap();
        g.agent_lease = Some(Arc::new(held));
        g.kill_tx = Some(kill_tx);
    }
    crate::backend::blockcontroller::register_controller(&holder.block_id, holder.clone());
    for (block_id, uid) in [(&holder.block_id, &f.uid), (&claimant.block_id, &f.uid)] {
        let mut pane = crate::backend::obj::Block { oid: block_id.clone(), ..Default::default() };
        pane.meta.insert("agentId".to_string(), serde_json::json!(uid));
        state.mstore.insert(&mut pane).unwrap();
    }
    // The holder's process exits when told to stop; its lease drops with it.
    let exiting = Arc::clone(&holder);
    let stopped = tokio::spawn(async move {
        let request = kill_rx.await;
        exiting.inner.lock().unwrap().agent_lease = None;
        request
    });

    let moved = crate::server::agent_move_here::move_here(&state, &claimant.block_id).await;
    let request = stopped.await.unwrap();
    let notice = state.mstore.get::<crate::backend::obj::Block>(&holder.block_id).unwrap().unwrap();
    crate::backend::blockcontroller::delete_controller(&holder.block_id);

    assert_eq!(moved.unwrap().from_block.as_deref(), Some(holder.block_id.as_str()));
    assert!(matches!(request, Ok(KillRequest::Graceful(_))), "a graceful stop, not a kill");
    let failure = notice.meta.get("agent:last_failure").map(|v| v.to_string()).unwrap_or_else(|| format!("no failure in {:?}", notice.meta));
    assert!(failure.contains("open_in_other_pane") && failure.contains(&claimant.block_id), "got: {failure}");
}

/// Nobody else holds the agent: nothing to move, and the caller just retries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moving_an_agent_nobody_else_holds_is_a_no_op() {
    let f = Fixture::new();
    let state = crate::server::tests::test_state();
    let claimant = f.controller("this-srv");
    let mut pane = crate::backend::obj::Block { oid: claimant.block_id.clone(), ..Default::default() };
    pane.meta.insert("agentId".to_string(), serde_json::json!(f.uid));
    state.mstore.insert(&mut pane).unwrap();
    let moved = crate::server::agent_move_here::move_here(&state, &claimant.block_id).await.unwrap();
    assert_eq!(moved.from_block, None);
}
