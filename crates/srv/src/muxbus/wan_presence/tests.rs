// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record, the HTTP side, and the driver against a fake relay. The
//! state machine's own rules are tested in `machine.rs`.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, AtomicUsize, Ordering};

use super::machine::{Config, MALFORMED_RECORD};
use super::*;
use crate::backend::agent_state::{AgentState, AgentStatus};
use crate::backend::fleet_feed::FleetObservation;
use crate::backend::rpc_types::PresenceState;

/// (instance id from the path, Authorization header, body) per PUT.
type Seen = Vec<(String, Option<String>, serde_json::Value)>;

/// A relay with a presence route and a health check, whose answers a test
/// changes as it goes.
#[derive(Default)]
struct FakeRelay {
    /// Answer every PUT with this status; 0 to judge the record.
    put_status: AtomicU16,
    version: Mutex<String>,
    /// The relay's clock minus this computer's.
    skew_ms: AtomicI64,
    /// Refuse anything but v1 as malformed, like a relay that predates v2.
    v1_only: AtomicBool,
    /// Refuse a record further than this from the relay's clock; 0: any.
    clock_window_ms: AtomicU64,
    seen: Mutex<Seen>,
    health_checks: AtomicUsize,
}

impl FakeRelay {
    fn clock_ms(&self) -> u64 {
        (agentmux_common::time::now_ms() + self.skew_ms.load(Ordering::SeqCst)) as u64
    }

    fn date(&self) -> String {
        let at = chrono::DateTime::from_timestamp_millis(self.clock_ms() as i64).unwrap();
        at.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
    }

    fn puts(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn put(&self, body: &serde_json::Value) -> (axum::http::StatusCode, serde_json::Value) {
        use axum::http::StatusCode;
        let forced = self.put_status.load(Ordering::SeqCst);
        if forced != 0 {
            let status = StatusCode::from_u16(forced).unwrap();
            return (status, serde_json::json!({ "error": status.canonical_reason() }));
        }
        if self.v1_only.load(Ordering::SeqCst) && body["v"] != 1 {
            return (StatusCode::BAD_REQUEST, serde_json::json!({ "error": MALFORMED_RECORD }));
        }
        let window = self.clock_window_ms.load(Ordering::SeqCst);
        let published = body["published_at_ms"].as_u64().unwrap_or(0);
        if window != 0 && published.abs_diff(self.clock_ms()) > window {
            let error = "published_at_ms is too far from the relay's clock";
            return (StatusCode::BAD_REQUEST, serde_json::json!({ "error": error }));
        }
        (StatusCode::OK, serde_json::json!({ "stored": true }))
    }
}

struct Stub {
    relay: Arc<FakeRelay>,
    _guard: tokio_util::sync::DropGuard,
    url: String,
}

async fn fake_relay(relay: FakeRelay) -> Stub {
    let relay = Arc::new(relay);
    let (put_side, health_side) = (relay.clone(), relay.clone());
    let app = axum::Router::new()
        .route(
            "/wan-instances/:instance_id/presence",
            axum::routing::put(
                move |axum::extract::Path(instance_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::Json<serde_json::Value>| {
                    let relay = put_side.clone();
                    async move {
                        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string);
                        let (status, answer) = relay.put(&body.0);
                        relay.seen.lock().unwrap().push((instance_id, auth, body.0));
                        (status, [("date", relay.date())], axum::Json(answer))
                    }
                },
            ),
        )
        .route(
            "/api/health",
            axum::routing::get(move || {
                let relay = health_side.clone();
                async move {
                    relay.health_checks.fetch_add(1, Ordering::SeqCst);
                    let version = relay.version.lock().unwrap().clone();
                    ([("date", relay.date())], axum::Json(serde_json::json!({ "status": "ok", "version": version })))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = CancellationToken::new();
    let child = token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).with_graceful_shutdown(async move { child.cancelled().await }).await;
    });
    Stub { relay, _guard: token.drop_guard(), url: format!("http://{addr}") }
}

fn relay_answering(status: u16) -> FakeRelay {
    FakeRelay { put_status: AtomicU16::new(status), ..Default::default() }
}

fn instance() -> (tempfile::TempDir, WanInstance) {
    let dir = tempfile::tempdir().unwrap();
    let wan = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
    let instance = wan.instance_ensure("narko").unwrap();
    (dir, instance)
}

fn feed() -> FleetFeed {
    let f = FleetFeed::new("narko".into(), "stable".into(), "0.59.11".into());
    f.observe_fleet(FleetObservation {
        agents: vec![
            ("Camper".into(), Some("host")),
            ("AgentX".into(), Some("container")),
            ("Ghost".into(), None),
        ],
        channels_running: 3,
        agent_status: [
            ("AgentX".to_string(), AgentStatus { state: AgentState::Working, since_ms: 5 }),
            ("Ghost".to_string(), AgentStatus { state: AgentState::Idle, since_ms: 5 }),
        ]
        .into(),
    });
    f
}

struct TestSession {
    token: Option<String>,
    instance: WanInstance,
    feed: Arc<FleetFeed>,
    _dir: tempfile::TempDir,
}

impl TestSession {
    fn new(token: Option<&str>) -> Self {
        let (dir, instance) = instance();
        Self { token: token.map(str::to_string), instance, feed: Arc::new(feed()), _dir: dir }
    }
}

impl Session for TestSession {
    async fn token(&self) -> Option<String> {
        self.token.clone()
    }

    fn record(&self, snapshot: &FleetSnapshot, v: u32, published_at_ms: u64) -> Result<InstallPresence, String> {
        build_record(&self.instance, &self.feed, snapshot, v, published_at_ms).ok_or_else(|| OUT_OF_BOUNDS.to_string())
    }
}

#[test]
fn the_record_carries_the_snapshot_and_verifies() {
    let (_dir, instance) = instance();
    let f = feed();
    let record = build_record(&instance, &f, &f.snapshot(), 2, 1_791_352_493_388).unwrap();
    assert!(record.verify());
    assert_eq!(record.instance_id, instance.instance_id);
    assert_eq!(record.hostname, "narko");
    assert_eq!(record.channel, "stable");
    assert_eq!(record.version, "0.59.11");
    assert_eq!(record.os, crate::backend::host_os::local_os());
    assert_eq!(record.channels_running, 3);
    assert_eq!(record.v, 2);
    assert_eq!(
        record.agents,
        vec![
            PresenceAgent { name: "AgentX".into(), kind: "container".into(), state: Some("working".into()) },
            PresenceAgent { name: "Camper".into(), kind: "host".into(), state: None },
        ],
        "sorted, the agent of unknown kind left out, no state where none is known"
    );
}

#[test]
fn a_v1_record_of_the_same_snapshot_drops_the_states_and_verifies() {
    let (_dir, instance) = instance();
    let f = feed();
    let record = build_record(&instance, &f, &f.snapshot(), 1, 42).unwrap();
    assert_eq!(record.v, 1);
    assert!(record.agents.iter().all(|a| a.state.is_none()));
    assert!(record.verify());
}

fn observation(agents: &[&str], channels_running: u32, working: &[&str]) -> FleetObservation {
    FleetObservation {
        agents: agents.iter().map(|n| (n.to_string(), Some("host"))).collect(),
        channels_running,
        agent_status: working
            .iter()
            .map(|n| (n.to_string(), AgentStatus { state: AgentState::Working, since_ms: 1 }))
            .collect(),
    }
}

fn snapshot_with(agents: &[&str], channels_running: u32, working: &[&str]) -> FleetSnapshot {
    let f = FleetFeed::new("narko".into(), "stable".into(), "1".into());
    f.observe_fleet(observation(agents, channels_running, working));
    f.snapshot()
}

#[test]
fn only_agents_kinds_and_channel_count_are_structure() {
    let base = snapshot_with(&["a", "b"], 1, &[]);
    assert!(!structure_changed(&base, &snapshot_with(&["a", "b"], 1, &["a"])), "states alone");
    assert!(structure_changed(&base, &snapshot_with(&["a"], 1, &[])), "an agent left");
    assert!(structure_changed(&base, &snapshot_with(&["a", "b"], 2, &[])), "a channel started");
    let mut container = base.clone();
    container.agent_kinds = Arc::new([("a".to_string(), "container")].into());
    assert!(structure_changed(&base, &container), "a kind changed");
}

const TEST_TIMING: Timing = Timing {
    debounce: Duration::from_millis(100),
    state_min_interval: Duration::from_millis(600),
    max_settle: Duration::from_secs(5),
};

fn observe(f: &FleetFeed, agents: &[&str], working: &[&str]) {
    f.observe_fleet(observation(agents, 1, working));
}

#[tokio::test]
async fn a_state_change_alone_waits_out_the_minimum_interval() {
    let f = FleetFeed::new("narko".into(), "stable".into(), "1".into());
    observe(&f, &["a"], &[]);
    let mut changes = f.subscribe();
    let sent = changes.borrow_and_update().clone();
    let sent_at = tokio::time::Instant::now();
    observe(&f, &["a"], &["a"]);
    let token = CancellationToken::new();
    assert!(wait_to_publish(&mut changes, &sent, sent_at, &token, &TEST_TIMING).await);
    assert!(sent_at.elapsed() >= TEST_TIMING.state_min_interval, "{:?}", sent_at.elapsed());
}

#[tokio::test]
async fn a_structural_change_is_published_after_the_debounce_even_while_states_flip() {
    let f = Arc::new(FleetFeed::new("narko".into(), "stable".into(), "1".into()));
    observe(&f, &["a"], &[]);
    let mut changes = f.subscribe();
    let sent = changes.borrow_and_update().clone();
    let sent_at = tokio::time::Instant::now();
    observe(&f, &["a", "b"], &[]);
    let flipping = f.clone();
    let flipper = tokio::spawn(async move {
        for i in 0..40 {
            let working: &[&str] = if i % 2 == 0 { &["a"] } else { &[] };
            observe(&flipping, &["a", "b"], working);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let token = CancellationToken::new();
    assert!(wait_to_publish(&mut changes, &sent, sent_at, &token, &TEST_TIMING).await);
    let waited = sent_at.elapsed();
    flipper.abort();
    assert!(waited >= TEST_TIMING.debounce, "{waited:?}");
    assert!(waited < TEST_TIMING.state_min_interval, "state flips did not hold it back: {waited:?}");
}

#[tokio::test]
async fn shutdown_ends_the_wait() {
    let f = FleetFeed::new("narko".into(), "stable".into(), "1".into());
    observe(&f, &["a"], &[]);
    let mut changes = f.subscribe();
    let sent = changes.borrow_and_update().clone();
    observe(&f, &["a"], &["a"]);
    let token = CancellationToken::new();
    token.cancel();
    assert!(!wait_to_publish(&mut changes, &sent, tokio::time::Instant::now(), &token, &TEST_TIMING).await);
}

#[tokio::test]
async fn a_publish_puts_a_verifiable_record_under_its_instance_id() {
    let stub = fake_relay(FakeRelay::default()).await;
    let session = TestSession::new(Some("tok"));
    let snapshot = session.feed.snapshot();
    let event = attempt(&session, &reqwest::Client::new(), &stub.url, &snapshot, 2, 42).await;
    assert!(
        matches!(event, Event::Answer { sent_v: 2, answer: Answer::Stored, relay_date_ms: Some(_) }),
        "{event:?}"
    );
    let seen = stub.relay.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let (path_id, auth, body) = &seen[0];
    assert_eq!(auth.as_deref(), Some("Bearer tok"));
    let record: InstallPresence = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(&record.instance_id, path_id, "the path names the record's instance");
    assert_eq!(record.published_at_ms, 42);
    assert!(record.verify(), "what goes on the wire passes the relay's check");
}

#[tokio::test]
async fn signed_out_sends_nothing() {
    let stub = fake_relay(FakeRelay::default()).await;
    let session = TestSession::new(None);
    let snapshot = session.feed.snapshot();
    let event = attempt(&session, &reqwest::Client::new(), &stub.url, &snapshot, 2, 42).await;
    assert_eq!(event, Event::NoSignIn);
    assert_eq!(stub.relay.puts(), 0);
}

#[tokio::test]
async fn the_relays_answers_are_read_with_its_clock() {
    let stub = fake_relay(relay_answering(503)).await;
    let session = TestSession::new(Some("tok"));
    let record = session.record(&session.feed.snapshot(), 2, 1).unwrap();
    let reply = put_presence(&stub.url, &reqwest::Client::new(), "tok", &record).await;
    assert_eq!(
        reply.answer,
        Answer::Unavailable { reason: "the cloud answered 503".into(), detail: "Service Unavailable".into() }
    );
    let date = reply.date_ms.expect("the Date header is read");
    assert!(date.abs_diff(agentmux_common::time::now_ms_u64()) < 5_000);

    *stub.relay.version.lock().unwrap() = "1.1.0".into();
    let (version, date) = relay_health(&stub.url, &reqwest::Client::new()).await;
    assert_eq!(version.as_deref(), Some("1.1.0"));
    assert!(date.is_some());
}

#[tokio::test]
async fn an_unreachable_relay_is_a_failure_not_a_hang() {
    let url = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let session = TestSession::new(Some("tok"));
    let record = session.record(&session.feed.snapshot(), 2, 1).unwrap();
    let reply = put_presence(&url, &reqwest::Client::new(), "tok", &record).await;
    assert!(
        matches!(reply.answer, Answer::Unavailable { ref reason, .. } if reason == "cloud unreachable"),
        "{reply:?}"
    );
    assert_eq!(relay_health(&url, &reqwest::Client::new()).await, (None, None));
}

#[test]
fn http_dates_and_retry_after_are_read() {
    assert_eq!(parse_http_date("Wed, 07 Oct 2026 19:44:00 GMT"), Some(1_791_402_240_000));
    assert_eq!(parse_http_date("yesterday"), None, "unreadable: no offset from it");
    assert_eq!(parse_http_date(""), None);
    assert_eq!(parse_retry_after("120", None, 0), Some(Duration::from_secs(120)));
    assert_eq!(parse_retry_after(" 5 ", None, 0), Some(Duration::from_secs(5)));
    // A date is read against the relay's clock, not this computer's.
    let relay_now = parse_http_date("Wed, 07 Oct 2026 19:43:00 GMT");
    assert_eq!(
        parse_retry_after("Wed, 07 Oct 2026 19:44:00 GMT", relay_now, 0),
        Some(Duration::from_secs(60))
    );
    assert_eq!(parse_retry_after("Wed, 07 Oct 2026 19:44:00 GMT", None, u64::MAX), Some(Duration::ZERO));
    assert_eq!(parse_retry_after("soon", None, 0), None);
}

#[test]
fn a_tick_far_later_on_the_wall_clock_is_a_wake() {
    let tick = Duration::from_secs(15);
    assert!(!woke(tick, tick, 15_000), "on time");
    assert!(!woke(tick, tick, 40_000), "a little late");
    assert!(woke(tick, tick, 3_600_000), "the monotonic clock stopped while asleep");
    assert!(woke(tick, Duration::from_secs(3600), 3_600_000), "the monotonic clock ran on while asleep");
    assert!(!woke(tick, tick, -600_000), "the wall clock set back is no wake");
}

#[test]
fn a_network_change_is_a_different_address_list() {
    let a = Some(vec!["198.51.100.5".parse().unwrap()]);
    let b = Some(vec!["203.0.113.7".parse().unwrap()]);
    assert!(network_changed(&a, &b));
    assert!(!network_changed(&a, &a.clone()));
    assert!(!network_changed(&a, &None), "unreadable: no change");
    assert!(!network_changed(&None, &b));
}

#[test]
fn random_units_are_in_range() {
    for _ in 0..1000 {
        let r = random_unit();
        assert!((0.0..1.0).contains(&r), "{r}");
    }
}

/// Short waits, so a test sees the transitions it causes. Only a trigger
/// (a signal, a new relay version) can end a 404 or refusal wait here.
fn quick() -> Config {
    Config {
        interval: Duration::from_secs(60),
        backoff_base: Duration::from_millis(20),
        backoff_cap: Duration::from_millis(100),
        backoff_floor: Duration::from_millis(5),
        retry_after_cap: Duration::from_secs(1),
        unsupported_retry: Duration::from_secs(60),
        health_every: Duration::from_millis(30),
        rejected_retry: Duration::from_secs(60),
        signed_out_recheck: Duration::from_secs(60),
        v1_memory: Duration::from_secs(60),
        outage_warn_after: Duration::from_secs(60),
    }
}

struct Running {
    status: Arc<Mutex<Option<PresenceStatusResult>>>,
    signals: mpsc::UnboundedSender<Event>,
    feed: Arc<FleetFeed>,
    _stop: tokio_util::sync::DropGuard,
}

impl Running {
    fn status(&self) -> Option<PresenceStatusResult> {
        self.status.lock().unwrap().clone()
    }

    /// Wait (up to 5 s) for the status to satisfy `ok`.
    async fn until(&self, what: &str, ok: impl Fn(&PresenceStatusResult) -> bool) -> PresenceStatusResult {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.status() {
                if ok(&status) {
                    return status;
                }
            }
            assert!(tokio::time::Instant::now() < deadline, "never {what}: {:?}", self.status());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn until_state(&self, state: PresenceState) -> PresenceStatusResult {
        self.until(&format!("{state:?}"), |s| s.state == state).await
    }
}

fn drive(url: &str, token: Option<&str>) -> Running {
    let session = TestSession::new(token);
    let feed = session.feed.clone();
    let status = Arc::new(Mutex::new(None));
    let (signals, rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let (url, slot, stop) = (url.to_string(), status.clone(), cancel.clone());
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        let config = quick();
        let driver = Driver {
            session: &session,
            http: &http,
            base_url: &url,
            config: &config,
            timing: &TEST_TIMING,
            status: &slot,
        };
        driver.run(session.feed.subscribe(), rx, &stop).await;
    });
    Running { status, signals, feed, _stop: cancel.drop_guard() }
}

#[tokio::test]
async fn a_404_then_a_relay_deploy_publishes_without_a_restart() {
    let stub = fake_relay(FakeRelay { version: Mutex::new("1.0.0".into()), ..relay_answering(404) }).await;
    let run = drive(&stub.url, Some("tok"));
    let status = run.until("checked the relay's version", |s| {
        s.state == PresenceState::Unsupported && s.relay_version.is_some()
    })
    .await;
    assert_eq!(status.relay_version.as_deref(), Some("1.0.0"));
    // Changes while unsupported send nothing.
    for i in 0..5 {
        observe(&run.feed, &["a", &format!("b{i}")], &[]);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(stub.relay.puts(), 1, "a change doesn't hammer a relay without the route");
    let checks = stub.relay.health_checks.load(Ordering::SeqCst);
    assert!(checks >= 2, "the version is checked again: {checks}");

    // The relay is deployed with the route.
    stub.relay.put_status.store(0, Ordering::SeqCst);
    *stub.relay.version.lock().unwrap() = "1.1.0".into();
    let status = run.until_state(PresenceState::Publishing).await;
    assert_eq!(stub.relay.puts(), 2, "one try on the new version");
    assert_eq!(status.relay_version.as_deref(), Some("1.1.0"));
    assert!(status.last_ok_ms.is_some());
    assert_eq!(status.last_error, None);
    let (_, _, body) = stub.relay.seen.lock().unwrap().last().unwrap().clone();
    let record: InstallPresence = serde_json::from_value(body).unwrap();
    assert!(record.verify());
    assert!(record.agents.iter().any(|a| a.name == "b4"), "the latest snapshot is the one sent");
}

#[tokio::test]
async fn a_sign_in_ends_a_404_wait() {
    let stub = fake_relay(FakeRelay { version: Mutex::new("1.0.0".into()), ..relay_answering(404) }).await;
    let run = drive(&stub.url, Some("tok"));
    run.until_state(PresenceState::Unsupported).await;
    stub.relay.put_status.store(0, Ordering::SeqCst);
    run.signals.send(Event::SignIn).unwrap();
    run.until_state(PresenceState::Publishing).await;
    assert_eq!(stub.relay.puts(), 2);
}

#[tokio::test]
async fn publish_now_ends_a_refusal_wait() {
    let stub = fake_relay(relay_answering(410)).await;
    let run = drive(&stub.url, Some("tok"));
    let status = run.until_state(PresenceState::Rejected).await;
    assert_eq!(status.last_error.as_deref(), Some("Gone"));
    assert!(status.next_try_ms.unwrap() >= status.since_ms + 60_000, "{status:?}");
    stub.relay.put_status.store(0, Ordering::SeqCst);
    run.signals.send(Event::PublishNow).unwrap();
    run.until_state(PresenceState::Publishing).await;
}

#[tokio::test]
async fn an_outage_backs_off_and_recovers() {
    let stub = fake_relay(relay_answering(503)).await;
    let run = drive(&stub.url, Some("tok"));
    run.until("retried a few times", |_| stub.relay.puts() >= 4).await;
    assert_eq!(run.status().unwrap().state, PresenceState::Retrying);
    assert_eq!(run.status().unwrap().last_error.as_deref(), Some("the cloud answered 503"));
    stub.relay.put_status.store(0, Ordering::SeqCst);
    run.until_state(PresenceState::Publishing).await;
}

#[tokio::test]
async fn a_relay_that_only_knows_v1_gets_v1() {
    let stub = fake_relay(FakeRelay { v1_only: AtomicBool::new(true), ..Default::default() }).await;
    let run = drive(&stub.url, Some("tok"));
    let status = run.until_state(PresenceState::Publishing).await;
    assert_eq!(status.record_version, 1);
    assert!(status.note.as_deref().unwrap_or("").contains("older version"), "{status:?}");
    let versions: Vec<_> = stub.relay.seen.lock().unwrap().iter().map(|(_, _, b)| b["v"].clone()).collect();
    assert_eq!(versions, vec![serde_json::json!(2), serde_json::json!(1)]);
}

#[tokio::test]
async fn a_clock_fifteen_minutes_off_still_publishes() {
    for skew_ms in [15 * 60_000i64, -15 * 60_000] {
        let stub = fake_relay(FakeRelay {
            skew_ms: AtomicI64::new(skew_ms),
            clock_window_ms: AtomicU64::new(5 * 60_000),
            ..Default::default()
        })
        .await;
        let run = drive(&stub.url, Some("tok"));
        let status = run.until_state(PresenceState::Publishing).await;
        assert_eq!(stub.relay.puts(), 2, "refused on this computer's time, stored on the relay's");
        let offset = status.offset_ms.unwrap();
        assert!((offset - skew_ms).abs() < 2_000, "{offset} vs {skew_ms}");
        assert_eq!(status.note.as_deref(), Some("Your clock differs from the cloud's by 15 min"));
    }
}

#[tokio::test]
async fn signed_out_the_driver_sends_nothing() {
    let stub = fake_relay(FakeRelay::default()).await;
    let run = drive(&stub.url, None);
    let status = run.until_state(PresenceState::SignedOut).await;
    assert_eq!(status.next_try_ms, None);
    observe(&run.feed, &["a"], &[]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stub.relay.puts(), 0);
    assert_eq!(stub.relay.health_checks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn without_a_publisher_publish_now_says_so() {
    // No `spawn` in tests: nothing listens.
    assert!(!publish_now());
    sign_in_changed();
}
