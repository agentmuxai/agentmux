// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A probe for a stalled UI thread (task #47826, docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §4).
//
// On a Linux machine with a full swap file and disk, the host's UI thread and the page's main thread
// stopped together for 6 to 55 seconds, and the log could not say what they waited on. This puts one
// no-op task on the UI thread every second and notes when it runs late. When it has not run for
// `STALL_AFTER`, the probe writes one `[ui-stall]` line with what the machine looked like then: this
// process's size, free RAM and swap, free disk, the pressure the kernel reports and, on Linux, the UI
// thread's state and the kernel function it sleeps in. It writes again every `REPEAT_EVERY` while the
// stall lasts, and once more when the thread runs again, with how long it was away.
//
// Cost: one empty task posted and one atomic read per second; the context is read only during a stall.
// Logging only: nothing here acts on a stall.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cef::{rc::Rc, ImplTask, Task, WrapTask};

/// How often a ping is posted, once the last one has run.
const PING_EVERY: Duration = Duration::from_secs(1);
/// A ping that has not run for this long is a stall.
pub(super) const STALL_AFTER: Duration = Duration::from_secs(3);
/// While a stall lasts, say so this often.
const REPEAT_EVERY: Duration = Duration::from_secs(15);

/// What the tracker says to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Event {
    /// The UI thread has not answered for `waited`. `repeat` is false for the first line of a stall.
    Stalled { waited: Duration, repeat: bool },
    /// It answered; the last ping took `latency`.
    Recovered { latency: Duration },
}

/// Turns the probe's readings into lines: one when a stall starts, one every `REPEAT_EVERY`, one at its end.
#[derive(Debug, Default)]
pub(super) struct StallTracker {
    /// When the stall was first written, and when it was last written.
    stalled: Option<(Instant, Instant)>,
}

impl StallTracker {
    /// `outstanding` is when the oldest unanswered ping was posted; `latency` is how long the last answered
    /// ping took (only read when the stall just ended).
    pub(super) fn observe(&mut self, now: Instant, outstanding: Option<Instant>, latency: Duration) -> Option<Event> {
        match (outstanding, self.stalled) {
            (Some(sent), None) => {
                let waited = now.saturating_duration_since(sent);
                if waited < STALL_AFTER {
                    return None;
                }
                self.stalled = Some((now, now));
                Some(Event::Stalled { waited, repeat: false })
            }
            (Some(sent), Some((first, last))) => {
                if now.saturating_duration_since(last) < REPEAT_EVERY {
                    return None;
                }
                self.stalled = Some((first, now));
                Some(Event::Stalled { waited: now.saturating_duration_since(sent), repeat: true })
            }
            (None, Some(_)) => {
                self.stalled = None;
                Some(Event::Recovered { latency })
            }
            (None, None) => None,
        }
    }
}

/// What the UI thread and the probe share.
struct Beat {
    epoch: Instant,
    /// Pings posted, and pings the UI thread has run.
    sent: AtomicU64,
    acked: AtomicU64,
    /// When the latest ping was posted, in ms since `epoch`.
    sent_at_ms: AtomicU64,
    /// How long the latest answered ping took.
    latency_ms: AtomicU64,
    /// The UI thread's OS thread id, once it has run a ping (Linux).
    ui_tid: AtomicU64,
}

impl Beat {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

cef::wrap_task! {
    struct PingTask {
        beat: Arc<Beat>,
        seq: u64,
    }

    impl Task {
        fn execute(&self) {
            let latency = self.beat.now_ms().saturating_sub(self.beat.sent_at_ms.load(Ordering::Relaxed));
            self.beat.latency_ms.store(latency, Ordering::Relaxed);
            if let Some(tid) = current_tid() {
                self.beat.ui_tid.store(tid, Ordering::Relaxed);
            }
            self.beat.acked.store(self.seq, Ordering::Release);
        }
    }
}

/// This thread's OS id: `/proc/thread-self` links to `<pid>/task/<tid>`. Written so every platform compiles it (the
/// CI legs that build this crate are not Linux ones); elsewhere the link is not there and the answer is none.
fn current_tid() -> Option<u64> {
    let link = std::fs::read_link("/proc/thread-self").ok()?;
    link.file_name()?.to_str()?.parse().ok()
}

/// Start the probe on its own thread, for the life of the process.
pub(super) fn start(data_root: Option<std::path::PathBuf>) {
    let beat = Arc::new(Beat {
        epoch: Instant::now(),
        sent: AtomicU64::new(0),
        acked: AtomicU64::new(0),
        sent_at_ms: AtomicU64::new(0),
        latency_ms: AtomicU64::new(0),
        ui_tid: AtomicU64::new(0),
    });
    let spawned = std::thread::Builder::new().name("ui-stall-probe".into()).spawn(move || run(beat, data_root));
    if let Err(e) = spawned {
        tracing::warn!(target: "ui_stall", error = %e, "[ui-stall] the probe thread did not start");
    }
}

fn run(beat: Arc<Beat>, data_root: Option<std::path::PathBuf>) {
    let mut tracker = StallTracker::default();
    loop {
        std::thread::sleep(PING_EVERY);
        let sent = beat.sent.load(Ordering::Relaxed);
        let outstanding = (beat.acked.load(Ordering::Acquire) < sent).then(|| {
            let ago = Duration::from_millis(beat.now_ms().saturating_sub(beat.sent_at_ms.load(Ordering::Relaxed)));
            Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
        });
        let latency = Duration::from_millis(beat.latency_ms.load(Ordering::Relaxed));
        match tracker.observe(Instant::now(), outstanding, latency) {
            Some(Event::Stalled { waited, repeat }) => {
                let tid = beat.ui_tid.load(Ordering::Relaxed);
                let context = context(if tid == 0 { None } else { Some(tid) }, data_root.as_deref());
                tracing::warn!(
                    target: "ui_stall",
                    waited_ms = waited.as_millis() as u64,
                    repeat,
                    "[ui-stall] the UI thread has not run a task for {} s: {context}",
                    waited.as_secs()
                );
            }
            Some(Event::Recovered { latency }) => {
                tracing::warn!(
                    target: "ui_stall",
                    latency_ms = latency.as_millis() as u64,
                    "[ui-stall] the UI thread ran again after {} s",
                    latency.as_secs()
                );
            }
            None => {}
        }
        if outstanding.is_none() {
            let seq = sent + 1;
            beat.sent_at_ms.store(beat.now_ms(), Ordering::Relaxed);
            beat.sent.store(seq, Ordering::Relaxed);
            let mut task = PingTask::new(Arc::clone(&beat), seq);
            if cef::post_task(cef::ThreadId::UI, Some(&mut task)) == 0 {
                // Not posted (shutting down): nothing will answer it, so don't call that a stall.
                beat.acked.store(seq, Ordering::Release);
            }
        }
    }
}

/// What the machine looked like during a stall, in one line.
fn context(ui_tid: Option<u64>, data_root: Option<&std::path::Path>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(mb) = own_private_mb() {
        parts.push(format!("host {mb} MB private"));
    }
    parts.push(format!("free RAM {} MB", mb_or_unknown(super::phys_free_mb())));
    #[cfg(target_os = "windows")]
    parts.push(format!("free commit {} MB", mb_or_unknown(super::commit_free_mb())));
    if let Some(root) = data_root {
        if let Some((free, total)) = agentmux_common::disk_space::volume_free_total_bytes(root) {
            parts.push(format!("free disk {} GB of {} GB", free / (1 << 30), total / (1 << 30)));
        }
    }
    #[cfg(target_os = "linux")]
    parts.extend(linux_context(ui_tid));
    #[cfg(not(target_os = "linux"))]
    let _ = ui_tid;
    parts.join(", ")
}

/// `u64::MAX` is the heartbeat's "not sampled yet".
fn mb_or_unknown(mb: u64) -> String {
    if mb == u64::MAX {
        "unknown".to_string()
    } else {
        mb.to_string()
    }
}

/// This process's private memory, in MB.
fn own_private_mb() -> Option<u64> {
    let pid = std::process::id();
    let procs = agentmux_procstats::snapshot().ok()?;
    let bytes = procs.iter().find(|p| p.pid == pid)?.mem_private?;
    Some(bytes / (1024 * 1024))
}

/// Written so every platform compiles it; only a Linux build calls it.
fn linux_context(ui_tid: Option<u64>) -> Vec<String> {
    let read = |path: &str| std::fs::read_to_string(path).ok();
    let mut parts = Vec::new();
    if let Some(status) = read("/proc/self/status") {
        if let Some(kb) = kb_of(&status, "VmRSS") {
            parts.push(format!("host RSS {} MB", kb / 1024));
        }
        if let Some(kb) = kb_of(&status, "VmSwap") {
            parts.push(format!("host in swap {} MB", kb / 1024));
        }
    }
    if let Some(meminfo) = read("/proc/meminfo") {
        let swap_total = kb_of(&meminfo, "SwapTotal").unwrap_or(0);
        let swap_free = kb_of(&meminfo, "SwapFree").unwrap_or(0);
        if swap_total > 0 {
            parts.push(format!("swap used {} of {} MB", swap_total.saturating_sub(swap_free) / 1024, swap_total / 1024));
        }
        if let Some(kb) = kb_of(&meminfo, "MemAvailable") {
            parts.push(format!("MemAvailable {} MB", kb / 1024));
        }
    }
    for kind in ["memory", "io", "cpu"] {
        if let Some(avg) = read(&format!("/proc/pressure/{kind}")).and_then(|t| psi_some_avg10(&t)) {
            parts.push(format!("{kind} pressure (10 s) {avg}%"));
        }
    }
    if let Some(tid) = ui_tid {
        let state = read(&format!("/proc/self/task/{tid}/stat")).and_then(|t| thread_state(&t));
        let wchan = read(&format!("/proc/self/task/{tid}/wchan")).map(|w| w.trim().to_string());
        match (state, wchan) {
            (Some(s), Some(w)) if !w.is_empty() && w != "0" => parts.push(format!("UI thread {} in {w}", state_name(s))),
            (Some(s), _) => parts.push(format!("UI thread {}", state_name(s))),
            _ => {}
        }
    }
    parts
}

/// The value of `key:   1234 kB` in `/proc/self/status` or `/proc/meminfo`, in kB.
fn kb_of(text: &str, key: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.strip_prefix(key).is_some_and(|rest| rest.starts_with(':')))?;
    line.split_once(':')?.1.split_whitespace().next()?.parse().ok()
}

/// The `some avg10=` figure of a `/proc/pressure/*` file: the share of the last 10 s some task waited.
fn psi_some_avg10(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.starts_with("some"))?;
    let field = line.split_whitespace().find_map(|f| f.strip_prefix("avg10="))?;
    Some(field.to_string())
}

/// The state letter of `/proc/<pid>/task/<tid>/stat`: the field after the command's closing parenthesis, which
/// can itself hold spaces and parentheses.
fn thread_state(stat: &str) -> Option<char> {
    stat.rsplit_once(')')?.1.trim_start().chars().next()
}

fn state_name(state: char) -> &'static str {
    match state {
        'R' => "running",
        'S' => "sleeping",
        'D' => "in uninterruptible sleep (waiting on disk or memory)",
        'T' | 't' => "stopped",
        'Z' => "a zombie",
        _ => "in an unknown state",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    #[test]
    fn a_ping_that_runs_in_time_says_nothing() {
        let t0 = Instant::now();
        let mut t = StallTracker::default();
        assert_eq!(t.observe(at(t0, 1), Some(at(t0, 0)), Duration::ZERO), None);
        assert_eq!(t.observe(at(t0, 2), None, Duration::from_millis(5)), None);
        // Late, but not by enough to be a stall.
        assert_eq!(t.observe(at(t0, 5), Some(at(t0, 3)), Duration::ZERO), None);
    }

    #[test]
    fn a_stall_is_written_once_when_it_starts_then_every_fifteen_seconds_then_when_it_ends() {
        let t0 = Instant::now();
        let mut t = StallTracker::default();
        let sent = at(t0, 0);
        assert_eq!(t.observe(at(t0, 2), Some(sent), Duration::ZERO), None);
        assert_eq!(
            t.observe(at(t0, 3), Some(sent), Duration::ZERO),
            Some(Event::Stalled { waited: Duration::from_secs(3), repeat: false })
        );
        // The next seconds of the same stall say nothing.
        assert_eq!(t.observe(at(t0, 4), Some(sent), Duration::ZERO), None);
        assert_eq!(t.observe(at(t0, 17), Some(sent), Duration::ZERO), None);
        assert_eq!(
            t.observe(at(t0, 18), Some(sent), Duration::ZERO),
            Some(Event::Stalled { waited: Duration::from_secs(18), repeat: true })
        );
        assert_eq!(t.observe(at(t0, 40), Some(sent), Duration::ZERO), Some(Event::Stalled { waited: Duration::from_secs(40), repeat: true }));
        assert_eq!(
            t.observe(at(t0, 56), None, Duration::from_secs(55)),
            Some(Event::Recovered { latency: Duration::from_secs(55) })
        );
        // And it is over: nothing more, and a later stall starts a new run of lines.
        assert_eq!(t.observe(at(t0, 57), None, Duration::from_secs(55)), None);
        assert_eq!(
            t.observe(at(t0, 70), Some(at(t0, 66)), Duration::ZERO),
            Some(Event::Stalled { waited: Duration::from_secs(4), repeat: false })
        );
    }

    #[test]
    fn a_recovery_is_only_reported_for_a_stall_that_was() {
        let t0 = Instant::now();
        let mut t = StallTracker::default();
        assert_eq!(t.observe(at(t0, 2), Some(at(t0, 0)), Duration::ZERO), None);
        assert_eq!(t.observe(at(t0, 3), None, Duration::from_secs(2)), None);
    }

    #[test]
    fn the_kernel_files_parse() {
        let status = "Name:\tagentmux\nVmRSS:\t  123456 kB\nVmSwap:\t    2048 kB\nVmSwapper: 9 kB\n";
        assert_eq!(kb_of(status, "VmRSS"), Some(123456));
        assert_eq!(kb_of(status, "VmSwap"), Some(2048));
        assert_eq!(kb_of(status, "VmData"), None);
        let psi = "some avg10=12.34 avg60=5.00 avg300=1.00 total=123\nfull avg10=1.00 avg60=0.00 avg300=0.00 total=5\n";
        assert_eq!(psi_some_avg10(psi).as_deref(), Some("12.34"));
        assert_eq!(psi_some_avg10(""), None);
    }

    #[test]
    fn a_thread_state_survives_a_command_name_with_spaces_and_parentheses() {
        assert_eq!(thread_state("4321 (CrBrowserMain) D 1 4321 4321 0 -1 4194560"), Some('D'));
        assert_eq!(thread_state("4321 (a (b) c) S 1 2 3"), Some('S'));
        assert_eq!(thread_state("garbage"), None);
        assert!(state_name('D').contains("uninterruptible"));
    }

    #[test]
    fn a_reading_not_taken_yet_is_not_a_number() {
        assert_eq!(mb_or_unknown(u64::MAX), "unknown");
        assert_eq!(mb_or_unknown(512), "512");
    }
}
