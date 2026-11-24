// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Stop, a replacement or a pane close that lands while an eager resume is between its spawn claim and
//! its child (the identity and credential gate in the middle can be slow) cancels that spawn: the child
//! would belong to a controller nothing can reach any more, and race the replacement on the same
//! `--resume` session. Spawns that are not eager resumes are never judged by it.

use super::super::eager_resume::STOPPED_WHILE_STARTING;
use super::super::*;
use crate::backend::obj::MetaMapType;

fn controller(block_id: &str) -> PersistentSubprocessController {
    PersistentSubprocessController::new("tab".to_string(), block_id.to_string(), None, None, None, None)
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

/// Both checkpoints a spawn passes: just before the child starts, and in the acquisition that installs it.
fn checkpoints(c: &PersistentSubprocessController) -> (Result<(), String>, Result<(), String>) {
    let mut inner = c.inner.lock().unwrap();
    let before = PersistentSubprocessController::restart_spawn_still_permitted_locked(&mut inner);
    let install = PersistentSubprocessController::commit_restart_spawn_locked(&mut inner);
    (before, install)
}

#[test]
fn a_stop_after_the_claim_cancels_the_spawn_at_both_checkpoints() {
    let c = controller("blk-stop-mid-gate");
    assert!(c.try_claim_eager_resume_spawn());
    assert_eq!(checkpoints(&c), (Ok(()), Ok(())), "nothing has happened since the claim");

    Controller::stop(&c, true, STATUS_DONE).unwrap();

    let (before, install) = checkpoints(&c);
    assert_eq!(before, Err(STOPPED_WHILE_STARTING.to_string()));
    assert_eq!(install, Err(STOPPED_WHILE_STARTING.to_string()));
}

#[test]
fn a_replacement_stopping_the_old_controller_cancels_its_spawn() {
    let c = controller("blk-replaced-mid-gate");
    assert!(c.try_claim_eager_resume_spawn());

    Controller::stop_for_replace(&c, STATUS_DONE).unwrap();

    assert_eq!(checkpoints(&c).0, Err(STOPPED_WHILE_STARTING.to_string()));
}

#[tokio::test]
async fn closing_the_pane_after_the_claim_cancels_the_spawn() {
    let c = controller("blk-closed-mid-gate");
    assert!(c.try_claim_eager_resume_spawn());

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(3)).await;

    assert_eq!(checkpoints(&c).1, Err(STOPPED_WHILE_STARTING.to_string()));
}

#[test]
fn a_spawn_that_is_not_an_eager_resume_is_never_cancelled_by_a_stop() {
    let c = controller("blk-plain-spawn");
    Controller::stop(&c, true, STATUS_DONE).unwrap();
    assert_eq!(checkpoints(&c), (Ok(()), Ok(())), "a message's own spawn answers to its own rules");
}

#[test]
fn a_stop_before_the_claim_does_not_cancel_a_later_resume() {
    let c = controller("blk-stop-before-claim");
    Controller::stop(&c, true, STATUS_DONE).unwrap();

    assert!(c.try_claim_eager_resume_spawn());

    assert_eq!(checkpoints(&c), (Ok(()), Ok(())), "only a stop that lands after the claim counts");
}

#[test]
fn a_config_restarts_own_stop_does_not_cancel_its_replacement() {
    let c = controller("blk-restart-own-stop");
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawn_generation = 3;
        inner.config_restart_generation = Some(3);
    }
    assert!(c.try_claim_eager_resume_spawn_for(Some(3)));

    // The restart's own stop of the old process, as `stop_for_config_restart` sends it.
    let _ = PersistentSubprocessController::request_stop_inner(&c.inner, KillRequest::Force, false, true);

    assert_eq!(checkpoints(&c).0, Ok(()));
}

/// The whole of a cancelled spawn: no child, the claim released, and the prompts queued behind it are
/// discarded with a reason, not handed to a fallback respawn that would start the stopped agent.
#[tokio::test]
async fn a_cancelled_spawn_starts_no_process_and_discards_what_was_queued() {
    let c = controller("blk-cancelled-spawn");
    assert!(c.try_claim_eager_resume_spawn());
    {
        let mut inner = c.inner.lock().unwrap();
        inner.session_id = Some("sid-kept".to_string());
        let seq = inner.take_next_message_seq();
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(seq, "{\"queued\":\"prompt\"}".to_string(), RecordOrigin::new(MessageOrigin::user())));
    }
    Controller::stop(&c, true, STATUS_DONE).unwrap();

    let err = c.spawn_process(bogus_config("sid-kept"), None).expect_err("the stop wins");
    assert_eq!(err, STOPPED_WHILE_STARTING);
    c.settle_eager_spawn_failure(&err, bogus_config("sid-kept"));

    let inner = c.inner.lock().unwrap();
    assert!(inner.current_pid.is_none(), "no child was started");
    assert!(!inner.spawning_in_progress, "the claim is released");
    assert!(!inner.stop_watch.claimed(), "and the stop it saw goes with it, in the same step: the next spawn is a message's own");
    assert!(inner.pending_send_messages.is_empty(), "no fallback respawn is left to start the agent");
    assert_eq!(inner.session_id.as_deref(), Some("sid-kept"));
}

/// The claim's epoch goes when the attempt ends, so a message's own spawn afterwards is not judged by it.
#[tokio::test(flavor = "multi_thread")]
async fn the_claim_epoch_does_not_outlive_the_attempt() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let c = controller("blk-epoch-cleared").with_identity_stores(Some(store.clone()), Some(store.clone()), "key".to_string());
    let c = PersistentSubprocessController { mstore: Some(store), ..c };
    let mut meta = MetaMapType::new();
    meta.insert(core::META_SESSION_ID.to_string(), serde_json::json!("sid-1"));
    meta.insert("cmd".to_string(), serde_json::json!("definitely-not-a-real-binary-xyz"));
    meta.insert("cmd:args".to_string(), serde_json::json!(["x"]));
    meta.insert("cmd:cwd".to_string(), serde_json::json!(""));
    meta.insert("cmd:env".to_string(), serde_json::json!({}));

    // A stop that came before the attempt, and a spawn that fails on its own account (no such program).
    Controller::stop(&c, true, STATUS_DONE).unwrap();
    let outcome = c.try_eager_resume(&meta, "sid-1");

    assert!(matches!(outcome, EagerResumeOutcome::DeclinedTo("spawn failed")), "failed for its own reason, not as cancelled");
    let inner = c.inner.lock().unwrap();
    assert!(!inner.stop_watch.claimed());
    assert!(!inner.spawning_in_progress);
}
