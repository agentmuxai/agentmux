// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Two channels' ledgers sharing one identity store (#1988): the sender fires
// to an agent in the other channel, the receiver follows the turn.

use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::backend::cron::{test_job, CronScheduler, FireOutcome, Slot};
use crate::backend::jobs::{Begin, BY_SCHEDULER, TRIGGER_SCHEDULE};
use crate::backend::mps::{Broker, MuxEvent, EVENT_AGENT_TURN};
use crate::backend::storage::job_runs::RunCost;
use crate::backend::storage::store::Store;

const PANE: &str = "b-remote";

struct Pair {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    sender: Arc<RunLedger>,
    receiver: Arc<RunLedger>,
    receiver_mstore: Arc<Store>,
    receiver_broker: Arc<Broker>,
}

/// The sender (`chan-a`) knows no pane `b-remote`; the receiver (`chan-b`)
/// holds it, on account `acct-b`.
fn pair() -> Pair {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap());
    let sender = RunLedger::new_in(store.clone(), Some(Arc::new(Store::open_in_memory().unwrap())), Arc::new(Broker::new()), "chan-a");
    let receiver_mstore = Arc::new(Store::open_in_memory().unwrap());
    let meta: crate::backend::obj::MetaMapType = [
        ("agentId".to_string(), json!("uid-y")),
        (crate::server::app_api::agent_spawn::RUN_ACCOUNT_META_KEY.to_string(), json!("acct-b")),
    ]
    .into_iter()
    .collect();
    let mut pane = crate::backend::obj::Block { oid: PANE.to_string(), meta, ..Default::default() };
    receiver_mstore.insert(&mut pane).unwrap();
    let receiver_broker = Arc::new(Broker::new());
    let receiver = RunLedger::new_in(store.clone(), Some(receiver_mstore.clone()), receiver_broker.clone(), "chan-b");
    Pair { _dir: dir, store, sender, receiver, receiver_mstore, receiver_broker }
}

impl Pair {
    fn begin(&self, slot: i64) -> (String, crate::backend::storage::cron::CronJob) {
        let job = test_job("j-x", "check the build", "AgentY", "uid-y", "uid-creator");
        let Begin::Run(id) = self.sender.begin_cron(&job, TRIGGER_SCHEDULE, Some(slot), BY_SCHEDULER) else { panic!() };
        (id, job)
    }

    fn deliver(&self, run_id: &str, job: &crate::backend::storage::cron::CronJob, block_id: &str) {
        let delivered = FireOutcome::Delivered { block_id: Some(block_id.to_string()) };
        self.sender.record_delivery(run_id, job, &delivered, true, now_ms());
    }

    fn turn(&self, data: serde_json::Value) {
        self.receiver_broker.publish(MuxEvent {
            event: EVENT_AGENT_TURN.to_string(),
            scopes: vec![format!("block:{PANE}")],
            sender: String::new(),
            persist: 1,
            data: Some(data),
        });
    }

    fn get(&self, id: &str) -> JobRun {
        self.store.job_run_get(id).unwrap().unwrap()
    }

    async fn wait_for(&self, id: &str, pred: impl Fn(&JobRun) -> bool) -> JobRun {
        for _ in 0..100 {
            let r = self.get(id);
            if pred(&r) {
                return r;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("run {id} never got there: {:?}", self.get(id));
    }
}

/// End to end: the sender's scheduler fires; the receiving instance's inject
/// handler takes the run before answering; the sender leaves it running;
/// the receiver's follower closes it with the turn's cost, tokens and steps.
#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_follows_a_run_another_channel_fired_and_closes_it_with_its_cost() {
    let p = pair();
    let sched = CronScheduler::new(Some(p.store.clone()), reqwest::Client::new(), String::new(), String::new(), Arc::new(Broker::new()));
    sched.install_runs(p.sender.clone());
    let receiver = p.receiver.clone();
    sched.install_delivery(Arc::new(move |req, _uid| {
        // The receiving instance's `inject_here_following_runs`.
        assert!(receiver.follow_forwarded(req.request_id.as_deref().unwrap_or(""), PANE));
        Box::pin(async move { json!({"success": true, "block_id": PANE}) })
    }));
    let job = test_job("j-x", "check the build", "AgentY", "uid-y", "uid-creator");
    p.store.cron_create(&job).unwrap();
    let run_id = sched.fire(&job, Slot::Scheduled(1_800_000_000_000)).await.run_id.unwrap();

    let r = p.get(&run_id);
    assert_eq!((r.state.as_str(), r.ended_at_ms, r.block_id.as_str()), (state::RUNNING, None, PANE), "left running");
    assert_eq!((r.account_id.as_str(), r.target_agent_uid.as_str()), ("acct-b", "uid-y"), "the receiver's pane, not overwritten");
    assert_eq!(r.links[FOLLOWED_BY_CHANNEL], "chan-b");
    assert_eq!(r.links["transcript"], json!({"block_id": PANE, "channel": "chan-b"}));
    assert_eq!(r.recorded_by.split('@').next(), Some("chan-a"));

    let t = now_ms() + 1;
    p.turn(json!({"block_id": PANE, "turn_id": t, "started_at_ms": t, "active": true}));
    p.wait_for(&run_id, |r| r.turn_id == Some(t)).await;
    p.turn(json!({
        "block_id": PANE, "turn_id": t, "started_at_ms": t, "active": false, "ended_at_ms": t + 4_000,
        "end": "completed", "cost_usd": 0.42, "output_tokens": 640, "steps": 7,
    }));
    let done = p.wait_for(&run_id, |r| r.state == state::FINISHED).await;
    assert_eq!((done.infra.as_str(), done.cost_usd, done.cost_known, done.cost_partial), (infra::OK, 0.42, true, false));
    assert_eq!((done.output_tokens, done.steps), (640, 7));

    // The sender's watch, past the timeout, finds it closed and writes nothing.
    assert!(p.sender.check_remote(&run_id, 0, now_ms() + NO_TURN_AFTER_MS + 1));
    assert_eq!(p.get(&run_id), done);
}

/// The receiver takes only another channel's open cron run, once.
#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_takes_only_another_channels_open_run_once() {
    let p = pair();
    let (id, _) = p.begin(1);
    assert!(!p.receiver.follow_forwarded("no-such-run", PANE));
    assert!(!p.sender.follow_forwarded(&id, PANE), "its own channel's run is the sender's to follow");
    assert!(p.receiver.follow_forwarded(&id, PANE));
    assert!(!p.receiver.follow_forwarded(&id, PANE), "once");
    let (closed, _) = p.begin(2);
    p.store.job_run_finish(&closed, infra::OK, None, "", 5).unwrap();
    assert!(!p.receiver.follow_forwarded(&closed, PANE), "a closed run stays closed");
}

/// The receiving agent's process exits during the turn: interrupted, with
/// what the turn reported. The receiving AgentMux stopping is interrupted
/// too: by the sender's watch when its process is gone, or by its own
/// restart; the sender's restart leaves a run the receiver follows alone.
#[tokio::test(flavor = "multi_thread")]
async fn the_receivers_process_exiting_interrupts_the_run() {
    let p = pair();
    let (exits, job) = p.begin(1);
    assert!(p.receiver.follow_forwarded(&exits, PANE));
    p.deliver(&exits, &job, PANE);
    let t = now_ms() + 1;
    p.turn(json!({"block_id": PANE, "turn_id": t, "started_at_ms": t, "active": true, "cost_usd": 0.1}));
    p.turn(json!({
        "block_id": PANE, "turn_id": t, "started_at_ms": t, "active": false, "ended_at_ms": t + 10,
        "end": "exited", "cost_usd": 0.1, "output_tokens": 5, "steps": 1,
    }));
    let r = p.wait_for(&exits, |r| r.state == state::FINISHED).await;
    assert_eq!((r.infra.as_str(), r.cost_usd), (infra::INTERRUPTED, 0.1));

    // The receiving AgentMux is gone (no process with its pid). Another
    // pane, so the turn above isn't this run's.
    let (gone, job) = p.begin(2);
    assert!(p.receiver.follow_forwarded(&gone, "b-other"));
    p.deliver(&gone, &job, "b-other");
    assert!(!p.sender.check_remote(&gone, now_ms(), now_ms()), "its follower is alive: the receiver's");
    p.store.job_run_add_links(&gone, &json!({ FOLLOWED_BY_PID: u32::MAX })).unwrap();
    assert!(p.sender.check_remote(&gone, now_ms(), now_ms()));
    let r = p.get(&gone);
    assert_eq!((r.state.as_str(), r.infra.as_str(), r.summary.as_str()), (state::FINISHED, infra::INTERRUPTED, FOLLOWER_GONE));

    // Restarts: the sender's leaves it, the receiver's closes it.
    let (restart, job) = p.begin(3);
    assert!(p.receiver.follow_forwarded(&restart, "b-other"));
    p.deliver(&restart, &job, "b-other");
    let sender_again = RunLedger::new_in(p.store.clone(), None, Arc::new(Broker::new()), "chan-a");
    sender_again.recover_after_restart();
    assert_eq!(p.get(&restart).state, state::RUNNING, "the receiver still follows it");
    let receiver_again = RunLedger::new_in(p.store.clone(), Some(p.receiver_mstore.clone()), Arc::new(Broker::new()), "chan-b");
    assert_eq!(receiver_again.recover_after_restart(), 1);
    assert_eq!(p.get(&restart).infra, infra::INTERRUPTED);
}

/// A receiver on an older build doesn't follow: the sender leaves the run
/// running through the no-turn timeout, then closes it as before, cost
/// unknown.
#[tokio::test(flavor = "multi_thread")]
async fn an_older_receiver_that_doesnt_follow_is_closed_by_the_senders_fallback() {
    let p = pair();
    let (id, job) = p.begin(1);
    let armed = now_ms();
    p.deliver(&id, &job, PANE);
    assert_eq!(p.get(&id).state, state::RUNNING);
    assert!(!p.sender.check_remote(&id, armed, armed + NO_TURN_AFTER_MS), "inside the timeout");
    assert_eq!(p.get(&id).state, state::RUNNING);
    assert!(p.sender.check_remote(&id, armed, armed + NO_TURN_AFTER_MS + 1));
    let r = p.get(&id);
    assert_eq!((r.state.as_str(), r.infra.as_str(), r.cost_known), (state::FINISHED, infra::OK, false));
    assert_eq!(r.summary, NOT_FOLLOWED);
}

/// Both sides closing at once: one write wins, the other changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn both_sides_closing_write_the_end_once() {
    let p = pair();
    for slot in 1..=20 {
        let (id, job) = p.begin(slot);
        assert!(p.receiver.follow_forwarded(&id, PANE));
        p.deliver(&id, &job, PANE);
        let cost = RunCost { cost_usd: 0.3, cost_known: true, output_tokens: 9, steps: 2 };
        let (s1, s2, a, b) = (p.store.clone(), p.store.clone(), id.clone(), id.clone());
        let receiver = std::thread::spawn(move || s1.job_run_finish(&a, infra::OK, Some(cost), "", now_ms()).unwrap());
        let sender = std::thread::spawn(move || s2.job_run_finish(&b, infra::INTERRUPTED, None, FOLLOWER_GONE, now_ms()).unwrap());
        let (by_receiver, by_sender) = (receiver.join().unwrap(), sender.join().unwrap());
        assert!(by_receiver ^ by_sender, "exactly one close for slot {slot}");
        let r = p.get(&id);
        if by_receiver {
            assert_eq!((r.infra.as_str(), r.cost_usd, r.summary.as_str()), (infra::OK, 0.3, ""));
        } else {
            assert_eq!((r.infra.as_str(), r.cost_known, r.summary.as_str()), (infra::INTERRUPTED, false, FOLLOWER_GONE));
        }
        let ended = r.ended_at_ms;
        assert!(p.sender.check_remote(&id, 0, i64::MAX / 2), "closed: nothing to watch");
        assert_eq!(p.get(&id).ended_at_ms, ended, "no second write");
    }
}

#[test]
fn the_senders_verdict() {
    let mut run = JobRun::new("r", "j", SOURCE_CRON, "n", TRIGGER_SCHEDULE, state::RUNNING, 0);
    let alive = |_: u32| true;
    assert_eq!(verdict(None, 0, 0, alive), Verdict::Done);
    assert_eq!(verdict(Some(&run), 0, NO_TURN_AFTER_MS, alive), Verdict::Wait);
    assert_eq!(verdict(Some(&run), 0, NO_TURN_AFTER_MS + 1, alive), Verdict::Close { infra: infra::OK, summary: NOT_FOLLOWED });
    run.links = json!({ FOLLOWED_BY_CHANNEL: "chan-b", FOLLOWED_BY_PID: 42 });
    assert_eq!(verdict(Some(&run), 0, i64::MAX / 2, alive), Verdict::Wait, "the receiver's, however long the turn");
    assert_eq!(verdict(Some(&run), 0, 0, |_| false), Verdict::Close { infra: infra::INTERRUPTED, summary: FOLLOWER_GONE });
    run.links = json!({ FOLLOWED_BY_CHANNEL: "chan-b" });
    assert_eq!(verdict(Some(&run), 0, 0, |_| false), Verdict::Wait, "no pid written: nothing to check");
    run.ended_at_ms = Some(1);
    assert_eq!(verdict(Some(&run), 0, 0, alive), Verdict::Done);
    assert!(recorded_in("chan-a@0.59.41", "chan-a") && recorded_in("chan-a", "chan-a"));
    assert!(!recorded_in("chan-ab@0.59.41", "chan-a"));
}
