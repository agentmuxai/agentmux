// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// Runs `settle` with a counter of the changes it reports.
fn watch(signal: &Arc<Notify>) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let task = tokio::spawn(settle(signal.clone(), move || {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    }));
    (count, task)
}

async fn wait(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_notifications_is_one_change_after_it_settles() {
    let signal = Arc::new(Notify::new());
    let (count, task) = watch(&signal);
    wait(10).await;
    signal.notify_one();
    wait(1_000).await;
    signal.notify_one();
    wait(1_500).await;
    signal.notify_one();
    wait(1_500).await;
    assert_eq!(count.load(Ordering::SeqCst), 0, "still inside the settle time of the last one");
    wait(1_000).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    wait(60_000).await;
    assert_eq!(count.load(Ordering::SeqCst), 1, "a quiet network reports nothing");
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_later_change_is_reported_again() {
    let signal = Arc::new(Notify::new());
    let (count, task) = watch(&signal);
    wait(10).await;
    signal.notify_one();
    wait(2_500).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    signal.notify_one();
    wait(2_500).await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_notification_before_the_watcher_waits_is_not_lost() {
    let signal = Arc::new(Notify::new());
    signal.notify_one();
    let (count, task) = watch(&signal);
    wait(2_500).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    task.abort();
}

/// The OS accepts the registration, the callback's context is the signal, and ending it is clean.
#[cfg(windows)]
#[test]
fn the_os_registration_starts_and_ends_cleanly() {
    let signal = Arc::new(Notify::new());
    let watch = Watch::start(signal.clone()).expect("NotifyIpInterfaceChange registers");
    assert_eq!(Arc::strong_count(&signal), 2, "the registration holds the signal");
    drop(watch);
    assert_eq!(Arc::strong_count(&signal), 1, "and gives it back");
}

#[cfg(not(windows))]
#[test]
fn a_platform_without_notifications_starts_no_watch() {
    assert!(Watch::start(Arc::new(Notify::new())).is_none());
}
