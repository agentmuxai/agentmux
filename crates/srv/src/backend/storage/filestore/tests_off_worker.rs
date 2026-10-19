// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A store call that waits for the connection does not pin a runtime worker
//! (off_worker.rs): while every caller is stuck behind a held connection, the
//! rest of the runtime keeps running.
//! docs/specs/SPEC_FILESTORE_STORE_CALLS_OFF_WORKERS_2026_10_19.md

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{FileMeta, FileOpts, FileStore};

const ZONE: &str = "block-off-worker";

/// Hold the connection for `hold` on a plain thread, and make `callers` async
/// tasks (more than there are workers) call `call` meanwhile. Returns the
/// longest gap between ticks of an unrelated task that only sleeps 10 ms at a
/// time.
async fn longest_tick_gap(callers: usize, hold: Duration, call: impl Fn(&FileStore) + Send + Sync + 'static) -> Duration {
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    fs.make_file(ZONE, "f", FileMeta::new(), FileOpts::default()).unwrap();
    let call = Arc::new(call);

    let holder = {
        let fs = fs.clone();
        std::thread::spawn(move || {
            let _conn = fs.lock_conn();
            std::thread::sleep(hold);
        })
    };
    // Let the holder take the connection before anyone asks for it.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let tasks: Vec<_> = (0..callers)
        .map(|_| {
            let (fs, call) = (fs.clone(), call.clone());
            tokio::spawn(async move { call(&fs) })
        })
        .collect();

    let started = Instant::now();
    let mut last = Instant::now();
    let mut longest = Duration::ZERO;
    while started.elapsed() < hold {
        tokio::time::sleep(Duration::from_millis(10)).await;
        longest = longest.max(last.elapsed());
        last = Instant::now();
    }
    for t in tasks {
        t.await.unwrap();
    }
    holder.join().unwrap();
    longest
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writes_waiting_for_the_connection_leave_the_workers_free() {
    let gap = longest_tick_gap(6, Duration::from_millis(600), |fs| {
        fs.append_data(ZONE, "f", b"line\n").unwrap();
    })
    .await;
    assert!(gap < Duration::from_millis(250), "the runtime stalled for {gap:?} while 6 writers waited on 2 workers");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_waiting_for_the_connection_leave_the_workers_free() {
    let gap = longest_tick_gap(6, Duration::from_millis(600), |fs| {
        // `stat` after a cache miss and `list_files` go to the database through the same connection.
        let _ = fs.list_files(ZONE).unwrap();
    })
    .await;
    assert!(gap < Duration::from_millis(250), "the runtime stalled for {gap:?} while 6 readers waited on 2 workers");
}

#[tokio::test]
async fn a_current_thread_runtime_just_calls_through() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, "f", FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, "f", b"x\n").unwrap();
    assert_eq!(fs.read_file(ZONE, "f").unwrap().unwrap(), b"x\n");
}

#[test]
fn without_a_runtime_it_just_calls_through() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, "f", FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, "f", b"x\n").unwrap();
    assert_eq!(fs.read_file(ZONE, "f").unwrap().unwrap(), b"x\n");
}

/// Every other way into a multi-thread runtime that is not one of its workers:
/// the root future of `block_on`, a blocking-pool thread, a plain thread that
/// entered the runtime's handle. `block_in_place` must not panic in any of them.
#[test]
fn the_other_ways_into_a_multi_thread_runtime_do_not_panic() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    fs.make_file(ZONE, "f", FileMeta::new(), FileOpts::default()).unwrap();

    // The root future of `block_on`.
    rt.block_on(async { fs.append_data(ZONE, "f", b"a\n").unwrap() });

    // A blocking-pool thread.
    let fs2 = fs.clone();
    rt.block_on(async move { tokio::task::spawn_blocking(move || fs2.append_data(ZONE, "f", b"b\n").unwrap()).await.unwrap() });

    // A plain thread that entered the handle.
    let handle = rt.handle().clone();
    let fs3 = fs.clone();
    std::thread::spawn(move || {
        let _enter = handle.enter();
        fs3.append_data(ZONE, "f", b"c\n").unwrap();
    })
    .join()
    .unwrap();

    // A worker, inside a block_in_place already (nested).
    let fs4 = fs.clone();
    rt.block_on(async move {
        tokio::spawn(async move { tokio::task::block_in_place(|| fs4.append_data(ZONE, "f", b"d\n").unwrap()) }).await.unwrap()
    });

    assert_eq!(fs.read_file(ZONE, "f").unwrap().unwrap(), b"a\nb\nc\nd\n");
}

/// Measurement, not an assertion (`--ignored --nocapture`): what an uncontended
/// store call costs when made from a runtime worker. The off-worker handoff
/// must not tax the common case (the connection free).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn uncontended_call_cost_on_a_worker() {
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    fs.make_file(ZONE, "f", FileMeta::new(), FileOpts::default()).unwrap();
    let fs2 = fs.clone();
    let (append_us, list_us) = tokio::spawn(async move {
        let data = vec![b'x'; 300];
        for _ in 0..200 {
            fs2.append_data(ZONE, "f", &data).unwrap();
        }
        let n = 5000;
        let t = Instant::now();
        for _ in 0..n {
            fs2.append_data(ZONE, "f", &data).unwrap();
        }
        let append_us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            let _ = fs2.list_files(ZONE).unwrap();
        }
        (append_us, t.elapsed().as_secs_f64() * 1e6 / n as f64)
    })
    .await
    .unwrap();
    println!("MEASURE uncontended on a worker: append {append_us:.1} us/call, list_files {list_us:.1} us/call");
}
