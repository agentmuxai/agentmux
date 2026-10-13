// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sysinfo loop against a real clock and the real broker: the machine's
//! counters are published all the time, and a pane's process stats are measured
//! only while something is subscribed to them.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::backend::blockcontroller::pidregistry;
use crate::backend::mps::{Broker, MuxEvent, SubscriptionRequest, WpsClient, EVENT_BLOCK_STATS, EVENT_SYS_INFO};
use crate::backend::wconfig::{ConfigState, SettingsType};

#[derive(Default)]
struct Counts {
    sysinfo: AtomicUsize,
    blockstats: AtomicUsize,
}

impl WpsClient for Arc<Counts> {
    fn send_event(&self, _route_id: &str, event: MuxEvent) {
        match event.event.as_str() {
            EVENT_SYS_INFO => self.sysinfo.fetch_add(1, Ordering::SeqCst),
            EVENT_BLOCK_STATS => self.blockstats.fetch_add(1, Ordering::SeqCst),
            _ => 0,
        };
    }
}

fn sub(event: &str, scope: Option<&str>) -> SubscriptionRequest {
    SubscriptionRequest {
        event: event.to_string(),
        scopes: scope.map(|s| vec![s.to_string()]).unwrap_or_default(),
        allscopes: scope.is_none(),
    }
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pane_stats_are_measured_only_while_something_subscribes() {
    let counts = Arc::new(Counts::default());
    let broker = Arc::new(Broker::new());
    broker.set_client(Box::new(Arc::clone(&counts)));
    broker.subscribe("viewer-machine", sub(EVENT_SYS_INFO, None));

    let config = Arc::new(ConfigState::new());
    config.update_settings(serde_json::from_value::<SettingsType>(serde_json::json!({ "telemetry:interval": 0.25 })).unwrap());

    // A pane whose shell is this test process, so the scan finds it alive.
    let block = "sampler-loop-smoke-block";
    pidregistry::register(block, std::process::id());
    let task = tokio::spawn(crate::backend::sysinfo::run_sysinfo_loop(Arc::clone(&broker), config, "local".into()));

    // The machine's counters flow at the telemetry rate; nobody watches the pane.
    pause(1500).await;
    assert!(counts.sysinfo.load(Ordering::SeqCst) >= 3, "sysinfo samples: {}", counts.sysinfo.load(Ordering::SeqCst));
    assert_eq!(counts.blockstats.load(Ordering::SeqCst), 0, "no viewer, so no pane scan");

    // A badge appears: stats start within an interval or two.
    broker.subscribe("viewer-badge", sub(EVENT_BLOCK_STATS, Some(&format!("block:{block}"))));
    pause(1200).await;
    let watched = counts.blockstats.load(Ordering::SeqCst);
    assert!(watched >= 2, "pane stats while watched: {watched}");

    // And it goes away: the scan slows back to the janitor's rate.
    broker.unsubscribe_all("viewer-badge");
    pause(300).await;
    let settled = counts.blockstats.load(Ordering::SeqCst);
    pause(1300).await;
    assert_eq!(counts.blockstats.load(Ordering::SeqCst), settled, "no more pane stats once nobody watches");

    task.abort();
    pidregistry::unregister(block);
}
