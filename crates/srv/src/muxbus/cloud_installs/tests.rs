// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::muxbus::install_presence::{PresenceAgent, PRESENCE_VERSION_GOODBYE, PRESENCE_VERSION_LINES, PRESENCE_VERSION_V1};

fn agent(name: &str, kind: &str, state: Option<&str>, line: Option<&str>) -> PresenceAgent {
    PresenceAgent { name: name.into(), kind: kind.into(), state: state.map(Into::into), line: line.map(Into::into) }
}

fn record(v: u32, seed: u8, host: &str, channel: &str) -> InstallPresence {
    let agents = [agent("Opaz", "host", Some("working"), Some("Fixing the race")), agent("Lark", "container", Some("idle"), None)];
    InstallPresence::sign_version(v, &[seed; 32], host, channel, "linux", "0.60.1", 1, agents, 1).unwrap()
}

fn wire(record: &InstallPresence, received_at_ms: u64) -> serde_json::Value {
    let mut value = serde_json::to_value(record).unwrap();
    value["received_at_ms"] = received_at_ms.into();
    value
}

#[test]
fn a_valid_record_is_kept_and_aged_on_this_machines_clock() {
    let raw = vec![wire(&record(PRESENCE_VERSION_LINES, 7, "charlie", "stable"), 940_000)];
    // The relay's clock says 60 s passed since it heard the install; this machine's says it is 5_000_000.
    let got = verified(raw, Some(1_000_000), 5_000_000);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].record.hostname.as_str(), got[0].seen_at_ms), ("charlie", 4_940_000));
    assert_eq!(got[0].record.agents.len(), 2);
}

#[test]
fn without_the_relays_date_the_records_own_time_stands() {
    let got = verified(vec![wire(&record(PRESENCE_VERSION_LINES, 7, "charlie", "stable"), 940_000)], None, 5_000_000);
    assert_eq!(got[0].seen_at_ms, 940_000);
}

#[test]
fn a_record_that_does_not_verify_is_left_out() {
    let good = record(PRESENCE_VERSION_LINES, 7, "charlie", "stable");
    let mut forged = wire(&record(PRESENCE_VERSION_LINES, 8, "starpower", "stable"), 1);
    forged["agents"][0]["line"] = "a line the install never signed".into();
    let mut wrong_id = wire(&record(PRESENCE_VERSION_LINES, 9, "narko", "stable"), 1);
    wrong_id["instance_id"] = good.instance_id.clone().into();
    let mut too_long = wire(&record(PRESENCE_VERSION_LINES, 10, "area54", "stable"), 1);
    too_long["hostname"] = "x".repeat(5_000).into();
    let mut no_time = wire(&record(PRESENCE_VERSION_LINES, 11, "laptop", "stable"), 1);
    no_time.as_object_mut().unwrap().remove("received_at_ms");
    let junk = serde_json::json!({ "hello": 1 });
    let got = verified(vec![wire(&good, 1), forged, wrong_id, too_long, no_time, junk], Some(1), 1);
    assert_eq!(got.iter().map(|c| c.record.hostname.as_str()).collect::<Vec<_>>(), ["charlie"]);
}

#[test]
fn a_goodbye_is_dropped_at_once_and_an_old_record_has_no_state() {
    let goodbye = record(PRESENCE_VERSION_GOODBYE, 7, "charlie", "stable");
    assert!(goodbye.gone);
    let old = record(PRESENCE_VERSION_V1, 8, "starpower", "stable");
    let got = verified(vec![wire(&goodbye, 1), wire(&old, 1)], Some(1), 1);
    assert_eq!(got.len(), 1);
    assert!(got[0].record.agents.iter().all(|a| a.state.is_none()));
}

#[test]
fn an_install_listed_twice_is_kept_once_with_the_newer_time() {
    let r = record(PRESENCE_VERSION_LINES, 7, "charlie", "stable");
    let got = verified(vec![wire(&r, 100), wire(&r, 900)], Some(1_000), 1_000);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].seen_at_ms, 900);
}

/// Answers from a script, one per read, and counts them.
struct Fake {
    script: Mutex<VecDeque<Fetched>>,
    reads: AtomicUsize,
    takes: Duration,
}

impl Fake {
    fn new(script: Vec<Fetched>) -> Arc<Self> {
        Arc::new(Self { script: Mutex::new(script.into()), reads: AtomicUsize::new(0), takes: Duration::ZERO })
    }
}

impl Source for Arc<Fake> {
    async fn fetch(&self) -> Fetched {
        self.reads.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.takes).await;
        self.script.lock().unwrap().pop_front().unwrap_or(Fetched::Failed)
    }
}

fn records(host: &str) -> Fetched {
    Fetched::Records { raw: vec![wire(&record(PRESENCE_VERSION_LINES, 7, host, "stable"), 1_000)], relay_now_ms: Some(1_000) }
}

#[tokio::test(start_paused = true)]
async fn a_read_is_kept_for_thirty_seconds() {
    let fake = Fake::new(vec![records("charlie"), records("starpower")]);
    let reader = Reader::new(Arc::clone(&fake));
    let first = reader.view(1_000).await;
    assert_eq!((first.state, first.installs.len()), (CloudState::Ok, 1));
    reader.view(20_000).await;
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1, "the second ask is answered from the cache");
    let later = reader.view(31_001).await;
    assert_eq!(fake.reads.load(Ordering::SeqCst), 2);
    assert_eq!(later.installs[0].record.hostname, "starpower");
    reader.invalidate();
    reader.view(31_002).await;
    assert_eq!(fake.reads.load(Ordering::SeqCst), 3, "a sign-in changed: read again");
}

#[tokio::test(start_paused = true)]
async fn signed_out_or_no_route_is_no_cloud_and_no_error() {
    let fake = Fake::new(vec![Fetched::SignedOut, Fetched::NoRoute]);
    let reader = Reader::new(Arc::clone(&fake));
    for now in [1_000, 40_000] {
        let view = reader.view(now).await;
        assert_eq!((view.state, view.installs.len()), (CloudState::Off, 0));
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_read_keeps_the_last_list_for_ten_minutes_and_then_drops_it() {
    let fake = Fake::new(vec![records("charlie"), Fetched::Failed, Fetched::Failed]);
    let reader = Reader::new(Arc::clone(&fake));
    reader.view(1_000).await;
    let kept = reader.view(100_000).await;
    assert_eq!((kept.state, kept.installs.len()), (CloudState::Unavailable, 1), "the list is kept, and said to be old");
    let dropped = reader.view(1_000 + KEEP_LAST_GOOD_MS + 1).await;
    assert_eq!((dropped.state, dropped.installs.len()), (CloudState::Unavailable, 0));
}

#[tokio::test(start_paused = true)]
async fn two_asks_at_once_make_one_read() {
    let fake = Arc::new(Fake {
        script: Mutex::new(vec![records("charlie")].into()),
        reads: AtomicUsize::new(0),
        takes: Duration::from_millis(500),
    });
    let reader = Reader::new(Arc::clone(&fake));
    let (a, b) = tokio::join!(reader.view(1_000), reader.view(1_000));
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1);
    // The one that read has the list; the one that did not answered from the cache, which had none yet.
    assert_eq!((a.installs.len(), b.installs.len()), (1, 0));
}

#[tokio::test(start_paused = true)]
async fn an_ask_does_not_wait_for_a_slow_read() {
    let fake = Arc::new(Fake { script: Mutex::new(vec![records("charlie")].into()), reads: AtomicUsize::new(0), takes: Duration::from_secs(30) });
    let reader = Reader::new(Arc::clone(&fake));
    let before = tokio::time::Instant::now();
    let view = reader.view(1_000).await;
    assert!(before.elapsed() <= READ_BUDGET + Duration::from_millis(10), "{:?}", before.elapsed());
    assert_eq!((view.state, view.installs.len()), (CloudState::Off, 0), "the old cache answers");
}

#[test]
fn an_agent_in_a_record_has_its_state_by_name() {
    let r = |seed, v| wire(&record(v, seed, "charlie", "stable"), 1);
    let installs = verified(vec![r(7, PRESENCE_VERSION_LINES)], Some(1), 1);
    let view = CloudView { installs, state: CloudState::Ok };
    assert_eq!(view.agent_state("opaz"), Some("working"), "by name, whatever the case");
    assert_eq!(view.agent_state("LARK"), Some("idle"));
    assert_eq!(view.agent_state("nobody"), None);
    let old = CloudView { installs: verified(vec![r(8, PRESENCE_VERSION_V1)], Some(1), 1), state: CloudState::Ok };
    assert_eq!(old.agent_state("Opaz"), None, "a record from before states has none");
}
