// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A per-key trailing debounce served by one long-lived worker thread.
//
// The srv write-throughs for window position (`position_persist.rs`) and
// opacity (`transparency.rs`) each used to spawn an OS thread per report,
// which slept out the debounce window and then checked a generation counter
// to see whether a newer report had superseded it. During a window drag the
// position reports arrive at ~20 Hz from the WinEvent hook on the CEF UI
// thread, so that was ~20 thread spawns a second on the thread that is also
// sizing the window (docs/analysis/ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md
// §3.4). Here each writer keeps the latest job per key on one worker and runs
// it once `delay` has passed with no newer job for that key: the same
// last-value-wins behaviour, one thread per writer, and jobs for a key can no
// longer race each other.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send + 'static>;

pub(crate) struct TrailingWriter {
    name: &'static str,
    delay: Duration,
    tx: OnceLock<Mutex<Sender<(String, Job)>>>,
}

impl TrailingWriter {
    pub(crate) const fn new(name: &'static str, delay: Duration) -> Self {
        Self { name, delay, tx: OnceLock::new() }
    }

    /// Replace any pending job for `key` with `job`, to run once `delay` has
    /// passed with no newer job for the same key.
    pub(crate) fn schedule(&self, key: impl Into<String>, job: impl FnOnce() + Send + 'static) {
        let tx = self.tx.get_or_init(|| {
            let (tx, rx) = channel();
            let delay = self.delay;
            if let Err(err) = std::thread::Builder::new()
                .name(format!("trailing-writer-{}", self.name))
                .spawn(move || run(rx, delay))
            {
                tracing::warn!(writer = self.name, %err, "[trailing-writer] could not start worker; writes are dropped");
            }
            Mutex::new(tx)
        });
        let sent = tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send((key.into(), Box::new(job)));
        if sent.is_err() {
            tracing::warn!(writer = self.name, "[trailing-writer] worker gone; write dropped");
        }
    }
}

/// The worker: keep the latest job per key, run each once its deadline passes.
/// Exits (after running whatever is still pending) when every sender is gone.
fn run(rx: Receiver<(String, Job)>, delay: Duration) {
    let mut pending: HashMap<String, (Instant, Job)> = HashMap::new();
    loop {
        let received = match pending.values().map(|(due, _)| *due).min() {
            Some(due) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok((key, job)) => {
                pending.insert(key, (Instant::now() + delay, job));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                for (_, (_, job)) in pending.drain() {
                    job();
                }
                return;
            }
        }
        let now = Instant::now();
        let due: Vec<String> = pending
            .iter()
            .filter(|(_, (deadline, _))| *deadline <= now)
            .map(|(key, _)| key.clone())
            .collect();
        for key in due {
            if let Some((_, job)) = pending.remove(&key) {
                job();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TrailingWriter;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn writer(delay_ms: u64) -> &'static TrailingWriter {
        Box::leak(Box::new(TrailingWriter::new("test", Duration::from_millis(delay_ms))))
    }

    /// A burst for one key (a drag, a slider) collapses to the last value.
    #[test]
    fn burst_runs_only_the_last_job() {
        let w = writer(40);
        let seen = Arc::new(Mutex::new(Vec::new()));
        for i in 0..5 {
            let seen = seen.clone();
            w.schedule("window-a", move || seen.lock().unwrap().push(i));
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(*seen.lock().unwrap(), vec![4]);
    }

    /// Keys are independent: a burst on one window doesn't drop another's write.
    #[test]
    fn keys_are_independent() {
        let w = writer(30);
        let seen = Arc::new(Mutex::new(Vec::new()));
        for key in ["window-a", "window-b"] {
            let seen = seen.clone();
            w.schedule(key, move || seen.lock().unwrap().push(key));
        }
        std::thread::sleep(Duration::from_millis(150));
        let mut got = seen.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, vec!["window-a", "window-b"]);
    }

    /// Nothing runs before the delay, and a single job still runs after it.
    #[test]
    fn waits_out_the_delay() {
        let w = writer(80);
        let seen = Arc::new(Mutex::new(0));
        let s = seen.clone();
        w.schedule("window-a", move || *s.lock().unwrap() += 1);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(*seen.lock().unwrap(), 0);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(*seen.lock().unwrap(), 1);
    }

    /// Spaced-out jobs (deliberate clicks, separate drags) each get written.
    #[test]
    fn spaced_jobs_each_run() {
        let w = writer(20);
        let seen = Arc::new(Mutex::new(Vec::new()));
        for i in 0..3 {
            let s = seen.clone();
            w.schedule("window-a", move || s.lock().unwrap().push(i));
            std::thread::sleep(Duration::from_millis(80));
        }
        assert_eq!(*seen.lock().unwrap(), vec![0, 1, 2]);
    }
}
