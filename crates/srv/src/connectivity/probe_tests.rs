// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The probe driver on tokio's paused clock, with a fake prober: no network.

use std::collections::VecDeque;
use std::sync::Mutex;

use super::*;
use crate::connectivity::{Reachability, ANTHROPIC};

const T0: i64 = 1_800_000_000_000;

/// Answers from a script (then `Answered`), or never, and records when it was asked.
struct FakeProber {
    clock: Clock,
    script: Mutex<VecDeque<Outcome>>,
    hang: bool,
    calls: Arc<Mutex<Vec<(Service, i64)>>>,
}

impl Prober for FakeProber {
    fn probe(&self, service: Service) -> Pin<Box<dyn Future<Output = Outcome> + Send>> {
        self.calls.lock().unwrap().push((service, (self.clock)() - T0));
        if self.hang {
            return Box::pin(std::future::pending());
        }
        let outcome = self.script.lock().unwrap().pop_front().unwrap_or(Outcome::Answered);
        Box::pin(async move { outcome })
    }
}

fn paused_clock() -> Clock {
    let start = tokio::time::Instant::now();
    Arc::new(move || T0 + i64::try_from(start.elapsed().as_millis()).unwrap())
}

struct Harness {
    shared: Arc<Shared>,
    clock: Clock,
    calls: Arc<Mutex<Vec<(Service, i64)>>>,
    driver: tokio::task::JoinHandle<()>,
}

impl Harness {
    fn start(script: Vec<Outcome>, hang: bool) -> Self {
        let clock = paused_clock();
        let shared = Arc::new(Shared::new(T0));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let prober = FakeProber { clock: clock.clone(), script: Mutex::new(script.into()), hang, calls: calls.clone() };
        let driver = tokio::spawn(drive(shared.clone(), Arc::new(prober), clock.clone()));
        Self { shared, clock, calls, driver }
    }

    fn fail(&self, service: Service) {
        self.shared.report_at(service, Outcome::Unreachable("refused".into()), (self.clock)());
    }

    fn state(&self, id: &str) -> Reachability {
        self.shared.tracker().status().services.into_iter().find(|s| s.service == id).map(|s| s.state).unwrap()
    }

    fn call_times(&self) -> Vec<i64> {
        self.calls.lock().unwrap().iter().map(|(_, at)| *at).collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

async fn wait(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(start_paused = true)]
async fn probes_back_off_until_the_service_answers_then_stop() {
    let refused = || Outcome::Unreachable("refused".into());
    let h = Harness::start(vec![refused(), refused(), refused()], false);
    h.fail(Service::ModelApi(&ANTHROPIC));
    wait(4_000).await;
    assert!(h.call_times().is_empty(), "the first probe waits five seconds");
    wait(60_000).await;
    assert_eq!(h.state("model_api:anthropic"), Reachability::Offline, "a failed probe confirms it");
    wait(600_000).await;
    assert_eq!(h.call_times(), [5_000, 15_000, 35_000, 75_000]);
    assert_eq!(h.state("model_api:anthropic"), Reachability::Online);
    assert_eq!(h.calls.lock().unwrap()[0].0, Service::ModelApi(&ANTHROPIC));
}

#[tokio::test(start_paused = true)]
async fn nothing_is_probed_while_everything_is_online_and_a_failure_wakes_the_driver() {
    let h = Harness::start(Vec::new(), false);
    h.shared.report_at(Service::Muxbus, Outcome::Answered, (h.clock)());
    wait(3_600_000).await;
    assert!(h.call_times().is_empty());
    h.fail(Service::Muxbus);
    wait(5_001).await;
    assert_eq!(h.call_times(), [3_605_000]);
    assert_eq!(h.state("muxbus"), Reachability::Online);
}

#[tokio::test(start_paused = true)]
async fn a_probe_that_never_returns_times_out_as_a_failure() {
    let h = Harness::start(Vec::new(), true);
    h.fail(Service::Muxbus);
    wait(5_000 + 14_000).await;
    assert_eq!(h.state("muxbus"), Reachability::Degraded, "still waiting on the probe");
    wait(2_000).await;
    assert_eq!(h.state("muxbus"), Reachability::Offline);
    // Its successor is scheduled from when it gave up: 20 s, then 10 s later.
    wait(10_000).await;
    assert_eq!(h.call_times(), [5_000, 30_000]);
}

#[tokio::test(start_paused = true)]
async fn each_service_is_probed_on_its_own_schedule() {
    let h = Harness::start(vec![Outcome::Unreachable("refused".into())], false);
    h.fail(Service::ModelApi(&ANTHROPIC));
    wait(2_000).await;
    h.fail(Service::Muxbus);
    wait(30_000).await;
    let calls: Vec<(String, i64)> = h.calls.lock().unwrap().iter().map(|(s, at)| (s.id(), *at)).collect();
    // The API's first probe fails and its next is 10 s later; MuxBus answers its only one.
    assert_eq!(calls, [("model_api:anthropic".to_string(), 5_000), ("muxbus".to_string(), 7_000), ("model_api:anthropic".to_string(), 15_000)]);
}

#[tokio::test(start_paused = true)]
async fn a_tracked_api_is_probed_at_startup() {
    let h = Harness::start(Vec::new(), false);
    h.shared.seed_at((h.clock)());
    assert_eq!(h.state("model_api:anthropic"), Reachability::Unknown);
    h.shared.wake.notify_one();
    wait(10).await;
    assert_eq!(h.call_times(), [0]);
    assert_eq!(h.state("model_api:anthropic"), Reachability::Online);
}
