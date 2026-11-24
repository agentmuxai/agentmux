// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A watch on srv's async runtime: it says in the log when the runtime stops
//! running tasks, and for how long.
//!
//! A task on the runtime beats every [`Config::beat_every`]; an OS thread
//! outside the runtime looks at the beat every [`Config::poll_every`]. A beat
//! that is [`Config::stall_after`] or more behind, or a look that itself comes
//! that much late (the process was paused and the beat ran again before the
//! look did), starts a stall, which ends
//! when beats are on time again and stay so for [`Config::merge_gap`] (a stall
//! that returns within that gap is the same stall). One line is written when it
//! starts and one when it ends, whatever its length, each beginning with
//! [`PREFIX`]:
//!
//! ```text
//! runtime-stall: start late_ms=1830 cause=runtime workers=8 alive_tasks=412 queue=17 load1=3.2
//! runtime-stall: start late_ms=1830 cause=runtime workers=32 alive_tasks=412 queue=0 over_ms=2000 cpu_machine=94 cpu_srv=2 io_read_mb=0 io_write_mb=3 mem_load=58
//! runtime-stall: end lasted_ms=5300 worst_late_ms=5200 stalls=2
//! ```
//!
//! `cause=machine` says the watching thread was itself late (the machine was
//! suspended or had no CPU for srv); `cause=runtime` says only the runtime was,
//! which is a blocked worker or a task that did not yield, or a machine that did not schedule srv's workers while it
//! kept the watching thread's sleep honest. The `cpu_` and `io_` figures (Windows, `stall_load`) say which: the share of
//! all cores the machine used and srv used over the last two seconds, srv's reads and writes in MB, and how full memory
//! was. A machine at 95% and srv at 2% is a machine with no CPU for srv; both low is a blocked worker.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backend::stall_load;

/// Every line this watch writes starts with it, so a log search finds them all.
pub const PREFIX: &str = "runtime-stall:";

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// How often the beat task runs.
    pub beat_every: Duration,
    /// How often the watching thread looks.
    pub poll_every: Duration,
    /// How far behind a beat has to be for a stall.
    pub stall_after: Duration,
    /// How long beats stay on time before the stall is over.
    pub merge_gap: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            beat_every: Duration::from_millis(100),
            poll_every: Duration::from_millis(250),
            stall_after: Duration::from_secs(1),
            merge_gap: Duration::from_secs(5),
        }
    }
}

/// What the runtime looked like when a stall was noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub workers: usize,
    pub alive_tasks: usize,
    pub queue: usize,
    /// One-minute load average, in hundredths, where the OS gives one.
    pub load1_centis: Option<u32>,
    /// What the machine and srv were doing over the last seconds, where the OS gives it (Windows).
    pub load: Option<crate::backend::stall_load::Load>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Start { late_ms: u64, machine: bool, snapshot: Snapshot },
    End { lasted_ms: u64, worst_late_ms: u64, stalls: u32 },
}

impl Event {
    /// The log line, beginning with [`PREFIX`].
    pub fn line(&self) -> String {
        match self {
            Event::Start { late_ms, machine, snapshot: s } => {
                let load = s.load1_centis.map(|c| format!(" load1={}.{:02}", c / 100, c % 100)).unwrap_or_default();
                let load = format!("{load}{}", s.load.map(load_fields).unwrap_or_default());
                let cause = if *machine { "machine" } else { "runtime" };
                format!(
                    "{PREFIX} start late_ms={late_ms} cause={cause} workers={} alive_tasks={} queue={}{load}",
                    s.workers, s.alive_tasks, s.queue
                )
            }
            Event::End { lasted_ms, worst_late_ms, stalls } => {
                format!("{PREFIX} end lasted_ms={lasted_ms} worst_late_ms={worst_late_ms} stalls={stalls}")
            }
        }
    }
}

/// The fields a line carries for a reading of the machine, with a leading space. Numbers only.
fn load_fields(l: stall_load::Load) -> String {
    let mem = l.mem_load_pct.map(|p| format!(" mem_load={p}")).unwrap_or_default();
    format!(
        " over_ms={} cpu_machine={} cpu_srv={} io_read_mb={} io_write_mb={}{mem}",
        l.over_ms, l.machine_cpu_pct, l.srv_cpu_pct, l.io_read_mb, l.io_write_mb
    )
}

/// Where a stall is, one look at a time. Pure over the times it is given.
#[derive(Debug)]
struct Tracker {
    cfg: Config,
    stall: Option<Stall>,
}

#[derive(Debug)]
struct Stall {
    /// When the first beat of it was due.
    began_ms: u64,
    worst_late_ms: u64,
    stalls: u32,
    /// The last look that found it late.
    last_late_look_ms: u64,
    /// When beats came back, and the beat that showed it: set while they stay
    /// on time, cleared by a new stall inside the merge gap.
    recovered: Option<(u64, u64)>,
}

impl Tracker {
    fn new(cfg: Config) -> Self {
        Self { cfg, stall: None }
    }

    /// One look at `now_ms`: the last beat was at `last_beat_ms`, and the
    /// watching thread's own sleep ran `watcher_late_ms` over. `snapshot` is read
    /// only when a stall starts.
    fn look(&mut self, now_ms: u64, last_beat_ms: u64, watcher_late_ms: u64, snapshot: impl FnOnce() -> Snapshot) -> Option<Event> {
        let beat_ms = self.cfg.beat_every.as_millis() as u64;
        let stall_ms = self.cfg.stall_after.as_millis() as u64;
        let gap_ms = self.cfg.merge_gap.as_millis() as u64;
        // The beat's lateness, or the look's own if that is more: after a pause the
        // beat may already have run again, and the look's lateness is what is left.
        let late_ms = now_ms.saturating_sub(last_beat_ms).saturating_sub(beat_ms).max(watcher_late_ms);
        let stalled_now = late_ms >= stall_ms;
        let on_time = late_ms < stall_ms / 2;
        match &mut self.stall {
            None if stalled_now => {
                self.stall = Some(Stall { began_ms: now_ms.saturating_sub(late_ms), worst_late_ms: late_ms, stalls: 1, last_late_look_ms: now_ms, recovered: None });
                Some(Event::Start { late_ms, machine: watcher_late_ms >= stall_ms / 2, snapshot: snapshot() })
            }
            None => None,
            Some(s) => {
                s.worst_late_ms = s.worst_late_ms.max(late_ms);
                if stalled_now {
                    s.last_late_look_ms = now_ms;
                    if s.recovered.take().is_some() {
                        s.stalls += 1;
                    }
                    return None;
                }
                if on_time && s.recovered.is_none() {
                    s.recovered = Some((now_ms, last_beat_ms));
                }
                match s.recovered {
                    Some((at, beat)) if now_ms.saturating_sub(at) >= gap_ms => {
                        // It ended at the beat that showed it, or at the last late look.
                        let ended = s.last_late_look_ms.max(if beat >= s.began_ms { beat } else { 0 });
                        let ev = Event::End { lasted_ms: ended.saturating_sub(s.began_ms), worst_late_ms: s.worst_late_ms, stalls: s.stalls };
                        self.stall = None;
                        Some(ev)
                    }
                    _ => None,
                }
            }
        }
    }
}

/// Stops the watching thread when the beat task is dropped with its runtime.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Watch `runtime` with `cfg`, calling `on_event` from the watching thread.
pub fn watch(runtime: tokio::runtime::Handle, cfg: Config, on_event: impl Fn(Event) + Send + 'static) {
    let epoch = Instant::now();
    let beat = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (beat, guard) = (beat.clone(), StopOnDrop(stop.clone()));
        runtime.spawn(async move {
            let _guard = guard;
            loop {
                tokio::time::sleep(cfg.beat_every).await;
                beat.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
            }
        });
    }
    let metrics = runtime.clone();
    let spawned = std::thread::Builder::new().name("runtime-stall-watch".into()).spawn(move || {
        let mut tracker = Tracker::new(cfg);
        let mut history = stall_load::History::default();
        loop {
            let slept = Instant::now();
            std::thread::sleep(cfg.poll_every);
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let watcher_late_ms = slept.elapsed().saturating_sub(cfg.poll_every).as_millis() as u64;
            let now_ms = epoch.elapsed().as_millis() as u64;
            if let Some(reading) = stall_load::read(now_ms) {
                history.push(reading);
            }
            let snapshot = || {
                let m = metrics.metrics();
                Snapshot {
                    workers: m.num_workers(),
                    alive_tasks: m.num_alive_tasks(),
                    queue: m.global_queue_depth(),
                    load1_centis: load1_centis(),
                    load: history.load_over(stall_load::REPORT_OVER_MS, stall_load::mem_load_pct()),
                }
            };
            if let Some(ev) = tracker.look(now_ms, beat.load(Ordering::Relaxed), watcher_late_ms, snapshot) {
                on_event(ev);
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("{PREFIX} the watch could not start: {e}");
    }
}

/// The record's line for a stall that ended: how long it lasted, its worst lateness, how many stalls were merged
/// into it, and whether the machine (not the runtime) was the cause. Numbers and a name only.
pub fn record_fields(lasted_ms: u64, worst_late_ms: u64, stalls: u32, machine: bool) -> serde_json::Value {
    serde_json::json!({ "ms": lasted_ms, "worst_ms": worst_late_ms, "n": stalls, "cause": if machine { "machine" } else { "runtime" } })
}

/// Start the watch on the current runtime, writing its lines to the log and each stall that ended to the record
/// (`agentmux_common::record_events`: a stall of a month ago is as findable as one of a minute ago).
pub fn start() {
    let machine = AtomicBool::new(false);
    watch(tokio::runtime::Handle::current(), Config::default(), move |ev| {
        tracing::warn!("{}", ev.line());
        match ev {
            Event::Start { machine: m, .. } => machine.store(m, Ordering::Relaxed),
            Event::End { lasted_ms, worst_late_ms, stalls } => {
                let Some(dir) = agentmux_common::record_events::record_dir() else { return };
                let fields = record_fields(lasted_ms, worst_late_ms, stalls, machine.load(Ordering::Relaxed));
                if let Err(e) = agentmux_common::record_events::append_event(&dir, agentmux_common::time::now_ms_u64(), "srv_stall", fields) {
                    tracing::debug!("{PREFIX} the record could not take the event: {e}");
                }
            }
        }
    });
}

#[cfg(unix)]
fn load1_centis() -> Option<u32> {
    let mut avg = [0f64; 1];
    // SAFETY: `avg` holds the one value asked for.
    let got = unsafe { libc::getloadavg(avg.as_mut_ptr(), 1) };
    (got == 1).then(|| (avg[0] * 100.0).round() as u32)
}

#[cfg(not(unix))]
fn load1_centis() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> Tracker {
        Tracker::new(Config::default())
    }

    /// Looks every 250 ms from `from` to `to` (ms), the last beat at `beat` until
    /// `back_at`, then on time; returns each look's event.
    fn run(t: &mut Tracker, from: u64, to: u64, beat: u64, back_at: u64) -> Vec<(u64, Event)> {
        let mut out = Vec::new();
        let mut now = from;
        while now <= to {
            let last = if now >= back_at { now - 50 } else { beat };
            if let Some(ev) = t.look(now, last, 0, Snapshot::default) {
                out.push((now, ev));
            }
            now += 250;
        }
        out
    }

    #[test]
    fn the_record_keeps_numbers_and_the_cause_of_a_stall() {
        assert_eq!(record_fields(5_300, 5_200, 2, false), serde_json::json!({ "ms": 5_300, "worst_ms": 5_200, "n": 2, "cause": "runtime" }));
        assert_eq!(record_fields(60_000, 59_000, 1, true)["cause"], "machine");
    }

    #[test]
    fn beats_on_time_are_not_a_stall() {
        let mut t = tracker();
        assert!(run(&mut t, 1_000, 60_000, 0, 0).is_empty());
        // 900 ms behind the due time is still under the 1 s line.
        assert_eq!(t.look(2_000, 1_000, 0, Snapshot::default), None);
    }

    #[test]
    fn a_long_stall_writes_a_start_and_an_end_not_a_line_a_second() {
        let mut t = tracker();
        // Last beat at 1 000; nothing until 61 000; then on time for longer than the gap.
        let events = run(&mut t, 1_250, 80_000, 1_000, 61_000);
        let kinds: Vec<_> = events.iter().map(|(_, e)| matches!(e, Event::Start { .. })).collect();
        assert_eq!(kinds, vec![true, false], "{events:?}");
        let (start_at, start) = &events[0];
        // Noticed once it was a second late (the beat was due at 1 100).
        assert_eq!(*start_at, 2_250);
        assert!(matches!(start, Event::Start { late_ms: 1_150, machine: false, .. }), "{start:?}");
        let (end_at, end) = &events[1];
        assert!(*end_at >= 61_000 + 5_000, "ended only after beats stayed on time for the gap: {end_at}");
        assert_eq!(*end, Event::End { lasted_ms: 61_000 - 50 - 1_100, worst_late_ms: 60_750 - 1_000 - 100, stalls: 1 });
    }

    #[test]
    fn a_stall_that_returns_inside_the_gap_is_the_same_stall() {
        let mut t = tracker();
        let mut events = run(&mut t, 1_250, 7_500, 1_000, 5_000);
        // Beats were back from 5 000, then stop again after the beat at 7 700.
        events.extend(run(&mut t, 7_750, 40_000, 7_700, 30_000));
        let starts = events.iter().filter(|(_, e)| matches!(e, Event::Start { .. })).count();
        let ends: Vec<_> = events.iter().filter_map(|(_, e)| if let Event::End { stalls, .. } = e { Some(*stalls) } else { None }).collect();
        assert_eq!((starts, ends), (1, vec![2]), "{events:?}");
    }

    #[test]
    fn stalls_further_apart_than_the_gap_are_two_stalls() {
        let mut t = tracker();
        let mut events = run(&mut t, 1_250, 20_000, 1_000, 4_000);
        events.extend(run(&mut t, 20_250, 60_000, 20_000, 30_000));
        let shape: Vec<_> = events.iter().map(|(_, e)| matches!(e, Event::Start { .. })).collect();
        assert_eq!(shape, vec![true, false, true, false], "{events:?}");
    }

    #[test]
    fn a_late_watching_thread_says_the_machine_was_the_cause() {
        let mut t = tracker();
        let ev = t.look(5_000, 1_000, 3_000, Snapshot::default);
        assert!(matches!(ev, Some(Event::Start { machine: true, .. })), "{ev:?}");
        let mut t = tracker();
        let ev = t.look(5_000, 1_000, 2, Snapshot::default);
        assert!(matches!(ev, Some(Event::Start { machine: false, .. })), "{ev:?}");
    }

    /// A paused process whose beat ran again before the look did: the look's own
    /// lateness is what shows it.
    #[test]
    fn a_pause_is_seen_even_when_the_beat_ran_again_first() {
        let mut t = tracker();
        let ev = t.look(5_000, 4_999, 3_750, Snapshot::default);
        assert!(matches!(ev, Some(Event::Start { late_ms: 3_750, machine: true, .. })), "{ev:?}");
        // Looks and beats on time after it: one end line once the gap has passed, and
        // no second start.
        let mut seen = Vec::new();
        for now in (5_250..=12_000).step_by(250) {
            seen.extend(t.look(now, now - 20, 1, Snapshot::default));
        }
        // It ended at the first beat seen on time, a look after the pause.
        assert_eq!(seen, vec![Event::End { lasted_ms: 3_980, worst_late_ms: 3_750, stalls: 1 }]);
    }

    #[test]
    fn every_line_starts_with_the_prefix() {
        let snapshot = Snapshot { workers: 8, alive_tasks: 412, queue: 17, load1_centis: Some(320), load: None };
        let start = Event::Start { late_ms: 1830, machine: false, snapshot }.line();
        let end = Event::End { lasted_ms: 5300, worst_late_ms: 5200, stalls: 2 }.line();
        assert_eq!(start, "runtime-stall: start late_ms=1830 cause=runtime workers=8 alive_tasks=412 queue=17 load1=3.20");
        assert_eq!(end, "runtime-stall: end lasted_ms=5300 worst_late_ms=5200 stalls=2");
        assert!(start.starts_with(PREFIX) && end.starts_with(PREFIX));
    }

    #[test]
    fn a_start_line_carries_what_the_machine_and_srv_were_doing() {
        let load = stall_load::Load { over_ms: 2_000, machine_cpu_pct: 94, srv_cpu_pct: 2, io_read_mb: 0, io_write_mb: 3, mem_load_pct: Some(58) };
        let snapshot = Snapshot { workers: 32, alive_tasks: 412, queue: 0, load1_centis: None, load: Some(load) };
        let start = Event::Start { late_ms: 1830, machine: false, snapshot }.line();
        assert_eq!(
            start,
            "runtime-stall: start late_ms=1830 cause=runtime workers=32 alive_tasks=412 queue=0 over_ms=2000 cpu_machine=94 cpu_srv=2 io_read_mb=0 io_write_mb=3 mem_load=58"
        );
        let no_memory = Snapshot { load: Some(stall_load::Load { mem_load_pct: None, ..load }), ..snapshot };
        assert!(!Event::Start { late_ms: 1830, machine: true, snapshot: no_memory }.line().contains("mem_load"));
    }

    fn fast() -> Config {
        Config {
            beat_every: Duration::from_millis(5),
            poll_every: Duration::from_millis(10),
            stall_after: Duration::from_millis(150),
            merge_gap: Duration::from_millis(100),
        }
    }

    /// A task that blocks the runtime's only worker is seen, and its end too.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_blocked_worker_is_reported_and_then_recovers() {
        let (tx, rx) = std::sync::mpsc::channel();
        watch(tokio::runtime::Handle::current(), fast(), move |ev| {
            let _ = tx.send(ev);
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err(), "an idle runtime is not a stall");
        tokio::spawn(async { std::thread::sleep(Duration::from_millis(400)) });
        let wait = Duration::from_secs(10);
        let start = rx.recv_timeout(wait).expect("start");
        // Not `machine: false`: on a loaded runner the watching thread can itself be late, which is rightly said to be the
        // machine (the classification has its own tests above).
        assert!(matches!(start, Event::Start { snapshot: Snapshot { workers: 1, .. }, .. }), "{start:?}");
        let end = rx.recv_timeout(wait).expect("end");
        match end {
            Event::End { lasted_ms, stalls, .. } => assert!((200..3_000).contains(&lasted_ms) && stalls == 1, "{end:?}"),
            other => panic!("{other:?}"),
        }
    }
}
