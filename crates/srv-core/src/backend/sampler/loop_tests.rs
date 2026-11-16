// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sysinfo loop against a real clock and the real broker: the machine's
//! counters are published all the time, and a pane's process stats are measured
//! only while something is subscribed to them.
//!
//! What is counted is every publish, through the broker's observer, which sees
//! an event whether or not any route subscribes to it. Counting what reaches a
//! subscribed route would pass whether or not the panes were scanned.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::backend::blockcontroller::pidregistry;
use crate::backend::mps::{Broker, SubscriptionRequest, EVENT_BLOCK_STATS, EVENT_SYS_INFO};
use crate::backend::wconfig::{ConfigState, SettingsType};

#[derive(Default)]
struct Published {
    sysinfo: AtomicUsize,
    blockstats: AtomicUsize,
}

impl Published {
    fn sysinfo(&self) -> usize {
        self.sysinfo.load(Ordering::SeqCst)
    }
    fn blockstats(&self) -> usize {
        self.blockstats.load(Ordering::SeqCst)
    }
}

fn watch_publishes(broker: &Broker) -> Arc<Published> {
    let seen = Arc::new(Published::default());
    let counted = Arc::clone(&seen);
    broker.add_observer(Arc::new(move |event| match event.event.as_str() {
        EVENT_SYS_INFO => {
            counted.sysinfo.fetch_add(1, Ordering::SeqCst);
        }
        EVENT_BLOCK_STATS => {
            counted.blockstats.fetch_add(1, Ordering::SeqCst);
        }
        _ => {}
    }));
    seen
}

fn badge(block: &str) -> SubscriptionRequest {
    SubscriptionRequest {
        event: EVENT_BLOCK_STATS.to_string(),
        scopes: vec![format!("block:{block}")],
        allscopes: false,
    }
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pane_stats_are_measured_only_while_something_subscribes() {
    let broker = Arc::new(Broker::new());
    let seen = watch_publishes(&broker);

    let config = Arc::new(ConfigState::new());
    config.update_settings(
        serde_json::from_value::<SettingsType>(serde_json::json!({ "telemetry:interval": 0.25 })).unwrap(),
    );

    // A pane whose shell is this test process, so the scan finds it alive.
    let block = "sampler-loop-smoke-block";
    pidregistry::register(block, std::process::id());
    // Every pane is named, as the store would name a real one.
    let panes = crate::backend::sampler::panes::tower_sampler(|_| {
        Some(crate::backend::tower::sampler::BlockLabel { label: "Terminal".into(), agent: false, provider: None })
    });
    let stop = Arc::new(AtomicBool::new(false));
    let sampler =
        crate::backend::sampler::thread::spawn(Arc::clone(&broker), config, "local".into(), panes, Arc::clone(&stop)).unwrap();

    // The machine's counters flow at the telemetry rate; nobody watches the pane,
    // so it isn't scanned (the janitor's first scan is 30 s away).
    // At 250 ms about eight would come in 2 s; a loaded test machine still gives a few.
    pause(2000).await;
    assert!(seen.sysinfo() >= 3, "sysinfo samples: {}", seen.sysinfo());
    assert_eq!(seen.blockstats(), 0, "no viewer, so no pane scan");

    // A badge appears: the pane is scanned at the telemetry rate.
    broker.subscribe("viewer-badge", badge(block));
    pause(1200).await;
    assert!(seen.blockstats() >= 2, "pane scans while watched: {}", seen.blockstats());

    // And it goes away: the scan slows back to the janitor's rate.
    broker.unsubscribe_all("viewer-badge");
    pause(400).await;
    let settled = seen.blockstats();
    pause(1300).await;
    assert_eq!(seen.blockstats(), settled, "no more pane scans once nobody watches");

    stop.store(true, Ordering::SeqCst);
    sampler.join().unwrap();
    pidregistry::unregister(block);
}

/// The sampler is not a task on the runtime it measures: with every worker of that runtime held, as
/// a stall holds them, the samples keep coming at the telemetry rate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn samples_keep_coming_while_the_runtime_is_stalled() {
    let broker = Arc::new(Broker::new());
    let seen = watch_publishes(&broker);
    let config = Arc::new(ConfigState::new());
    config.update_settings(
        serde_json::from_value::<SettingsType>(serde_json::json!({ "telemetry:interval": 0.25 })).unwrap(),
    );
    let panes = crate::backend::sampler::panes::tower_sampler(|_| None);
    let stop = Arc::new(AtomicBool::new(false));
    let sampler =
        crate::backend::sampler::thread::spawn(Arc::clone(&broker), config, "local".into(), panes, Arc::clone(&stop)).unwrap();
    pause(600).await;

    // Hold both workers for 2 s, the way a runtime stall does.
    let before = seen.sysinfo();
    let holds: Vec<_> = (0..2).map(|_| tokio::spawn(async { std::thread::sleep(Duration::from_millis(2000)) })).collect();
    std::thread::sleep(Duration::from_millis(2100));
    let during = seen.sysinfo() - before;
    for h in holds {
        h.await.unwrap();
    }

    stop.store(true, Ordering::SeqCst);
    sampler.join().unwrap();
    // 8 are due at 250 ms; a loaded test machine still gives several, a sampler on the stalled runtime gives none.
    assert!(during >= 4, "samples while the runtime was held: {during}");
}

#[test]
fn a_late_tick_reports_the_average_over_the_whole_gap() {
    use crate::backend::sysinfo::NetState;
    use std::collections::HashMap;
    use std::time::Instant;
    let (mut net, t0) = (NetState::new(), Instant::now());
    let mut v = HashMap::new();
    net.get_net_data_at(t0, (0, 0), &mut v);
    assert!(v.is_empty(), "the first read has nothing to compare with");
    // 4 MB in 1 s, then 4 MB over the next 4 s (a late tick): 4 MB/s, then 1 MB/s, not a spike or a hole.
    net.get_net_data_at(t0 + Duration::from_secs(1), (0, 4 << 20), &mut v);
    assert!((v["net:bytesrecv"] - 4.0).abs() < 1e-9);
    net.get_net_data_at(t0 + Duration::from_secs(5), (0, 8 << 20), &mut v);
    assert!((v["net:bytesrecv"] - 1.0).abs() < 1e-9, "{}", v["net:bytesrecv"]);
}
