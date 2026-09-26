// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03 (#942 family) — srv
//! liveness probing. Closes the named non-goal in
//! `SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md`: that spec covers a
//! crashed (exited) srv; this covers srv staying alive but wedged
//! (deadlock, exhausted blocking pool, stuck await) so its process never
//! exits and the existing crash-recycle machinery never triggers.
//!
//! Simpler than the host's `ui_liveness`/`teardown_backstop` pair, and
//! deliberately so: the host's probe is fire-and-forget over a pipe (the
//! reply arrives asynchronously from a posted CEF UI-thread task, which is
//! why that side needs nonce-matching across ticks), and `teardown_backstop`
//! needs an armed state machine because it must not fire during legitimate
//! zero-window states. Neither applies here — srv already exposes a
//! synchronous, unauthenticated HTTP health endpoint
//! (`agentmux-srv/src/server/mod.rs::health_handler`, mounted outside
//! `auth_middleware`), so a single bounded round-trip per tick gives a
//! pass/fail answer within that same tick. No cross-tick reply matching, no
//! "is zero absence legitimate" guard — srv is expected to answer whenever
//! it is running.
//!
//! Recovery deliberately does NOT duplicate the existing crash-recycle
//! logic: when [`RecyclePolicy`] decides srv is stuck (no answer for a minute
//! AND no CPU progress, or three minutes regardless), the caller kills srv's
//! process directly (`Child::start_kill()`). The next loop iteration's
//! `srv_status = srv_child.wait()` arm sees the exit and runs the
//! already-shipped #2107 respawn/rebind/host-recycle path unmodified — this
//! module's only job is deciding "treat this as a crash", never how to
//! recover from one. [`LatencyTracker`] turns the same probes into a health
//! signal the host shows as a banner.
//!
//! All decisions are pure types owned by the supervisor loop (no process-
//! global state), so each test builds its own instances with injected times.

use std::time::{Duration, Instant};


/// Hand-rolled HTTP `GET /` against srv's web endpoint, bounded end-to-end
/// by `timeout`. Deliberately not reqwest — same reasoning as
/// `second_instance::forward_open_new_window`: the launcher binary stays
/// tiny and the request is fixed, so a hand-rolled request is the right
/// tool. Unlike that function (a one-shot call before the supervisor loop
/// starts), this runs every tick INSIDE the loop, so it uses `tokio::net`
/// + `tokio::time::timeout` rather than blocking `std::net` calls — a
/// blocking probe would stall the whole supervisor loop (host-exit
/// detection, the teardown backstop, everything else in the same
/// `select!`) for up to `timeout` on every hiccup.
///
/// Success = a response starting with `HTTP/1.1 200`. `web_endpoint` is
/// `srv_result.web_endpoint` — a bare `host:port` (e.g. `127.0.0.1:54321`),
/// NOT a URL: `emit_estart` (`agentmux-srv/src/bootstrap.rs`) writes
/// `web:127.0.0.1:{port}` with no scheme, and `parse_estart`
/// (`srv_spawner.rs`) carries that through verbatim — the same convention
/// `host_spawn.rs` relies on when it passes this value straight through as
/// `AGENTMUX_BACKEND_WEB`. (codex P1, PR #2411: an earlier version of this
/// function stripped a `http://` prefix that never exists, so every probe
/// silently failed and every healthy srv got recycled to death.)
pub async fn probe(web_endpoint: &str, timeout: Duration) -> bool {
    probe_rtt(web_endpoint, timeout).await.is_some()
}

/// [`probe`], returning how long the successful round-trip took. `None` is a
/// miss (timeout, refused, or a non-200 answer).
pub async fn probe_rtt(web_endpoint: &str, timeout: Duration) -> Option<Duration> {
    let started = Instant::now();
    match tokio::time::timeout(timeout, probe_inner(web_endpoint)).await {
        Ok(true) => Some(started.elapsed()),
        _ => None,
    }
}

async fn probe_inner(host_port: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let Ok(mut stream) = tokio::net::TcpStream::connect(host_port).await else {
        return false;
    };
    let req = format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", host_port);
    if stream.write_all(req.as_bytes()).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 32];
    match stream.read(&mut buf).await {
        Ok(n) if n > 0 => buf[..n].starts_with(b"HTTP/1.1 200"),
        _ => false,
    }
}

// ── Slow vs. dead, and a latency signal ─────────────────────────────────────
//
// docs/analysis/ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26.md §8.1/§8.2.
// Three consecutive 3s misses used to recycle srv — ~30s of slowness killed
// every agent session, including when srv was only slow under host load and
// recovering on its own. The probe's round-trip time is also a health signal
// the user never saw. Both are decided here, as pure types, so the supervisor
// only feeds samples in and acts on the answer.

/// Consecutive misses before a recycle is even considered (60s at the 10s
/// probe interval), and the minimum span they must cover.
pub const RECYCLE_MIN_MISSES: u32 = 6;
pub const RECYCLE_MIN_SPAN: Duration = Duration::from_secs(55);
/// CPU time srv must have used across the miss window to count as "still
/// working". A deadlocked or parked srv uses almost none; one grinding
/// through load uses far more than this.
pub const PROGRESS_CPU: Duration = Duration::from_millis(250);
/// Recycle regardless of progress after this many consecutive misses (~3 min):
/// a srv spinning without ever answering is not "slow", it is stuck too.
pub const RECYCLE_HARD_CAP_MISSES: u32 = 18;

/// Why a recycle was decided, for the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecycleReason {
    /// Missed for ≥ `RECYCLE_MIN_SPAN` and used under `PROGRESS_CPU` of CPU.
    NoProgress,
    /// `RECYCLE_HARD_CAP_MISSES` consecutive misses, progress or not.
    HardCap,
}

/// Recycle decision over a run of consecutive misses. Pure: the caller passes
/// the time and srv's cumulative CPU time (`None` when unreadable, which counts
/// as no evidence of progress).
#[derive(Debug, Default)]
pub struct RecyclePolicy {
    /// `(time, srv cpu)` of each miss in the current run, oldest first.
    run: std::collections::VecDeque<(Instant, Option<Duration>)>,
    misses: u32,
}

impl RecyclePolicy {
    pub fn record_success(&mut self) {
        *self = Self::default();
    }

    /// Record a miss; returns `Some(reason)` when srv should be recycled.
    ///
    /// Progress is measured over the last `RECYCLE_MIN_SPAN` only, not since
    /// the run began — a srv that worked for a minute and then deadlocked must
    /// not be kept alive by the CPU it used before it stuck.
    pub fn record_miss(&mut self, now: Instant, srv_cpu: Option<Duration>) -> Option<RecycleReason> {
        self.misses = self.misses.saturating_add(1);
        self.run.push_back((now, srv_cpu));
        if self.misses >= RECYCLE_HARD_CAP_MISSES {
            return Some(RecycleReason::HardCap);
        }
        let first = self.run.front().map(|(t, _)| *t).unwrap_or(now);
        if self.misses < RECYCLE_MIN_MISSES || now.duration_since(first) < RECYCLE_MIN_SPAN {
            return None;
        }
        // Window start: the newest miss at least RECYCLE_MIN_SPAN old.
        let window_start = self
            .run
            .iter()
            .rev()
            .find(|(t, _)| now.duration_since(*t) >= RECYCLE_MIN_SPAN)
            .map(|(_, cpu)| *cpu)
            .unwrap_or(None);
        // Keep the run bounded: nothing older than the window start matters.
        while self.run.len() > 1 && now.duration_since(self.run[1].0) >= RECYCLE_MIN_SPAN {
            self.run.pop_front();
        }
        let progressed = match (window_start, srv_cpu) {
            (Some(a), Some(b)) => b.saturating_sub(a) >= PROGRESS_CPU,
            _ => false,
        };
        if progressed { None } else { Some(RecycleReason::NoProgress) }
    }

    pub fn misses(&self) -> u32 {
        self.misses
    }
}

/// Health level of the probe's rolling average round-trip time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyLevel {
    Normal,
    Warn,
    Critical,
}

impl LatencyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LatencyLevel::Normal => "normal",
            LatencyLevel::Warn => "warn",
            LatencyLevel::Critical => "critical",
        }
    }
}

/// Probes averaged (1 minute at the 10s interval).
pub const LATENCY_WINDOW: usize = 6;
/// Enter/exit thresholds on the rolling average, in ms. Exit below entry so a
/// value hovering at a threshold doesn't flap the banner.
pub const LATENCY_WARN_ENTER_MS: u64 = 1_000;
pub const LATENCY_WARN_EXIT_MS: u64 = 600;
pub const LATENCY_CRITICAL_ENTER_MS: u64 = 2_500;
pub const LATENCY_CRITICAL_EXIT_MS: u64 = 1_800;

/// Rolling average of probe RTTs with a debounced level, the same shape as the
/// host's memory `PressureTracker`. A miss is fed in as the probe timeout.
#[derive(Debug)]
pub struct LatencyTracker {
    samples: std::collections::VecDeque<u64>,
    level: LatencyLevel,
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self { samples: std::collections::VecDeque::with_capacity(LATENCY_WINDOW), level: LatencyLevel::Normal }
    }
}

impl LatencyTracker {
    pub fn average_ms(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples.iter().sum::<u64>() / self.samples.len() as u64
    }

    pub fn level(&self) -> LatencyLevel {
        self.level
    }

    /// Add a sample; returns the new level when it changed.
    pub fn observe(&mut self, rtt_ms: u64) -> Option<LatencyLevel> {
        if self.samples.len() == LATENCY_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(rtt_ms);
        let avg = self.average_ms();
        let next = match self.level {
            LatencyLevel::Normal if avg >= LATENCY_CRITICAL_ENTER_MS => LatencyLevel::Critical,
            LatencyLevel::Normal if avg >= LATENCY_WARN_ENTER_MS => LatencyLevel::Warn,
            LatencyLevel::Warn if avg >= LATENCY_CRITICAL_ENTER_MS => LatencyLevel::Critical,
            LatencyLevel::Warn if avg < LATENCY_WARN_EXIT_MS => LatencyLevel::Normal,
            LatencyLevel::Critical if avg < LATENCY_WARN_EXIT_MS => LatencyLevel::Normal,
            LatencyLevel::Critical if avg < LATENCY_CRITICAL_EXIT_MS => LatencyLevel::Warn,
            same => same,
        };
        if next != self.level {
            self.level = next;
            Some(next)
        } else {
            None
        }
    }
}

/// Total CPU time (kernel + user) a process has used — the progress evidence
/// for [`RecyclePolicy`]. `None` if the process can't be opened.
#[cfg(windows)]
pub fn process_cpu_time(pid: u32) -> Option<Duration> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        // FILETIME ticks are 100ns.
        Some(Duration::from_nanos((ft(k) + ft(u)) * 100))
    }
}

#[cfg(not(windows))]
pub fn process_cpu_time(_pid: u32) -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use std::time::Instant;


    #[tokio::test]
    async fn probe_succeeds_against_a_real_200_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 512];
            let _ = sock.read(&mut buf).await;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
        });
        // `addr` here matches the real shape of `srv_result.web_endpoint`
        // (bare host:port, no scheme) — see the codex P1 fix this test
        // guards against.
        let ok = super::probe(&addr.to_string(), std::time::Duration::from_secs(2)).await;
        assert!(ok);
    }

    #[tokio::test]
    async fn probe_times_out_against_a_stalled_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            // Accept and never respond — simulates a wedged srv whose
            // listener still accepts connections but whose async runtime
            // never gets around to running the handler.
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });
        let ok = super::probe(&addr.to_string(), std::time::Duration::from_millis(200)).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn probe_fails_against_nothing_listening() {
        // Bind-then-drop to get a port that's guaranteed refused.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let ok = super::probe(&addr.to_string(), std::time::Duration::from_secs(1)).await;
        assert!(!ok);
    }

    mod slow_vs_dead {
        use super::super::*;
        use std::time::{Duration, Instant};

        fn feed(p: &mut RecyclePolicy, t0: Instant, n: u32, cpu_step_ms: u64) -> Option<RecycleReason> {
            let mut last = None;
            for i in 0..n {
                let now = t0 + Duration::from_secs(10 * i as u64);
                last = p.record_miss(now, Some(Duration::from_millis(1_000 + cpu_step_ms * i as u64)));
            }
            last
        }

        #[test]
        fn three_misses_no_longer_recycle() {
            let mut p = RecyclePolicy::default();
            assert_eq!(feed(&mut p, Instant::now(), 3, 0), None);
        }

        #[test]
        fn a_minute_of_misses_with_no_cpu_progress_recycles() {
            let mut p = RecyclePolicy::default();
            // 7 misses 10s apart = 60s span; CPU flat.
            assert_eq!(feed(&mut p, Instant::now(), 7, 0), Some(RecycleReason::NoProgress));
        }

        #[test]
        fn a_slow_but_working_srv_is_left_alone_until_the_hard_cap() {
            let mut p = RecyclePolicy::default();
            // CPU advances 100ms per 10s: working, just not answering in time.
            assert_eq!(feed(&mut p, Instant::now(), 17, 100), None);
            // …but three minutes without one answer is stuck regardless.
            let t = Instant::now() + Duration::from_secs(170);
            assert_eq!(p.record_miss(t, Some(Duration::from_secs(9))), Some(RecycleReason::HardCap));
        }

        #[test]
        fn a_srv_that_worked_then_deadlocked_is_recycled_within_a_minute_of_sticking() {
            let mut p = RecyclePolicy::default();
            let t0 = Instant::now();
            let mut cpu = 1_000u64;
            // 8 misses while working (CPU +100ms per probe)…
            for i in 0..8u64 {
                cpu += 100;
                assert_eq!(p.record_miss(t0 + Duration::from_secs(10 * i), Some(Duration::from_millis(cpu))), None);
            }
            // …then CPU goes flat. Within ~6 more probes the window holds no progress.
            let mut got = None;
            let mut probes = 0;
            for i in 8..18u64 {
                probes += 1;
                got = p.record_miss(t0 + Duration::from_secs(10 * i), Some(Duration::from_millis(cpu)));
                if got.is_some() { break; }
            }
            assert_eq!(got, Some(RecycleReason::NoProgress));
            assert!(probes <= 7, "took {probes} probes after sticking");
        }

        #[test]
        fn unreadable_cpu_counts_as_no_progress() {
            let mut p = RecyclePolicy::default();
            let t0 = Instant::now();
            let mut last = None;
            for i in 0..7u64 {
                last = p.record_miss(t0 + Duration::from_secs(10 * i), None);
            }
            assert_eq!(last, Some(RecycleReason::NoProgress));
        }

        #[test]
        fn one_answer_resets_the_run() {
            let mut p = RecyclePolicy::default();
            feed(&mut p, Instant::now(), 5, 0);
            p.record_success();
            assert_eq!(p.misses(), 0);
            assert_eq!(feed(&mut p, Instant::now(), 3, 0), None);
        }

        #[test]
        fn latency_levels_enter_and_exit_with_hysteresis() {
            let mut t = LatencyTracker::default();
            for _ in 0..6 {
                assert_eq!(t.observe(5), None);
            }
            assert_eq!(t.level(), LatencyLevel::Normal);
            // Six 1.2s samples → avg crosses 1s → Warn (once).
            let mut changes = Vec::new();
            for _ in 0..6 {
                if let Some(l) = t.observe(1_200) { changes.push(l); }
            }
            assert_eq!(changes, vec![LatencyLevel::Warn]);
            // Timeouts (3000) push the average past 2.5s → Critical.
            let mut changes = Vec::new();
            for _ in 0..6 {
                if let Some(l) = t.observe(3_000) { changes.push(l); }
            }
            assert_eq!(changes.last(), Some(&LatencyLevel::Critical));
            // Hovering at 2s (between critical exit 1.8s and entry 2.5s) stays Critical…
            for _ in 0..6 { t.observe(2_000); }
            assert_eq!(t.level(), LatencyLevel::Critical);
            // …fast answers bring it back to Normal.
            for _ in 0..6 { t.observe(5); }
            assert_eq!(t.level(), LatencyLevel::Normal);
            assert!(t.average_ms() < LATENCY_WARN_EXIT_MS);
        }

        #[tokio::test]
        async fn probe_rtt_measures_a_real_round_trip() {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 256];
                let _ = sock.read(&mut buf).await;
                tokio::time::sleep(Duration::from_millis(120)).await;
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await;
            });
            let rtt = probe_rtt(&addr.to_string(), Duration::from_secs(2)).await.expect("answered");
            assert!(rtt >= Duration::from_millis(100), "{rtt:?}");
        }

        #[cfg(windows)]
        #[test]
        fn own_process_cpu_time_is_readable() {
            assert!(process_cpu_time(std::process::id()).is_some());
        }
    }
}
