// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which of srv's threads are the async runtime's workers.
//!
//! A blocking call is a different problem on a worker (the rest of srv's tasks wait for it) than on a thread of the
//! blocking pool or a thread srv started itself (only that call waits). Tokio gives no way to ask which one a thread
//! is, but it launches the workers while the runtime is built, before any blocking task can exist: the threads that
//! start before [`settle`] is called are the workers. `main` hands [`thread_started`] to the runtime's
//! `on_thread_start` and calls [`settle`] once the runtime is built; anything that never does (a test, a helper
//! binary) sees no worker.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static STARTED: AtomicUsize = AtomicUsize::new(0);
static SETTLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static WORKER: Cell<bool> = const { Cell::new(false) };
}

/// Called by each thread the runtime starts, on that thread.
pub fn thread_started() {
    if !SETTLED.load(Ordering::Acquire) {
        WORKER.with(|w| w.set(true));
    }
    STARTED.fetch_add(1, Ordering::AcqRel);
}

/// Called once the runtime is built, with its worker count: waits (a short while at most) for that many threads to
/// have started, then marks every later thread as not a worker.
pub fn settle(workers: usize) {
    let began = Instant::now();
    while STARTED.load(Ordering::Acquire) < workers && began.elapsed() < Duration::from_secs(2) {
        std::thread::yield_now();
    }
    SETTLED.store(true, Ordering::Release);
}

/// Whether the calling thread is one of the runtime's workers.
pub fn on_worker() -> bool {
    WORKER.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_threads_started_before_the_settle_are_workers() {
        assert!(!on_worker(), "a plain test thread is no worker");
        let worker = std::thread::spawn(|| {
            thread_started();
            on_worker()
        })
        .join()
        .unwrap();
        assert!(worker, "a thread the runtime started while it was built is a worker");
        settle(1);
        let later = std::thread::spawn(|| {
            thread_started();
            on_worker()
        })
        .join()
        .unwrap();
        assert!(!later, "a thread started after the runtime was built is not");
    }
}
