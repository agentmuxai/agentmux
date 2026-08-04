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
//! logic: on `SRV_HANG_REQUIRED_MISSES` consecutive failed probes, the
//! caller kills srv's process directly (`Child::start_kill()`) and resets
//! this module's counters. The next loop iteration's existing
//! `srv_status = srv_child.wait()` arm sees the exit and runs the
//! already-shipped #2107 respawn/rebind/host-recycle path unmodified — this
//! module's only job is deciding "treat this as a crash", never how to
//! recover from one.
//!
//! Like `ui_liveness`, all logic lives on the struct (unit-testable without
//! real I/O or a mock clock — there's no elapsed-time gate, just a
//! consecutive-failure counter); the module-level functions delegate to one
//! process-global instance, mirroring `ui_liveness`'s reasoning for why each
//! test constructs its own instance instead of sharing the process-global
//! cell (parallel test execution would otherwise interleave).

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Consecutive missed health probes before srv counts as wedged. At the
/// `SRV_PROBE_INTERVAL` (10s) this bounds worst-case wedge→recycle latency
/// to roughly 3 probe intervals plus their timeouts (~30s) — the same
/// order of magnitude as the host teardown backstop's 30s grace.
pub const SRV_HANG_REQUIRED_MISSES: u32 = 3;

#[derive(Debug, Default)]
pub struct SrvLiveness {
    consecutive_misses: u32,
    last_alive: Option<Instant>,
}

impl SrvLiveness {
    /// Record a successful probe. Clears the miss streak — any answer
    /// proves srv's async runtime is pumping right now.
    pub fn record_success(&mut self, now: Instant) {
        self.consecutive_misses = 0;
        self.last_alive = Some(now);
    }

    /// Record a missed probe (timeout or connection failure). Returns the
    /// new consecutive-miss count.
    pub fn record_failure(&mut self) -> u32 {
        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
        self.consecutive_misses
    }

    /// Clear all state. Called after every srv (re)spawn — a freshly
    /// started srv must not inherit its predecessor's miss count.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The recycle decision: pure read, caller executes.
    pub fn should_recycle(&self, required_misses: u32) -> bool {
        self.consecutive_misses >= required_misses
    }

    #[allow(dead_code)] // telemetry surface, mirrors ui_liveness::last_alive
    pub fn last_alive(&self) -> Option<Instant> {
        self.last_alive
    }
}

fn cell() -> &'static Mutex<SrvLiveness> {
    static S: OnceLock<Mutex<SrvLiveness>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(SrvLiveness::default()))
}

/// See [`SrvLiveness::record_success`]. Process-global instance.
pub fn record_success() {
    cell().lock().unwrap().record_success(Instant::now());
}

/// See [`SrvLiveness::record_failure`]. Process-global instance.
pub fn record_failure() -> u32 {
    cell().lock().unwrap().record_failure()
}

/// See [`SrvLiveness::reset`]. Process-global instance.
pub fn reset() {
    cell().lock().unwrap().reset();
}

/// See [`SrvLiveness::should_recycle`]. Process-global instance, evaluated
/// against the spec constant.
pub fn should_recycle() -> bool {
    cell().lock().unwrap().should_recycle(SRV_HANG_REQUIRED_MISSES)
}

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
/// `srv_result.web_endpoint`, already in `http://127.0.0.1:{port}` form
/// (see `srv_spawner::SrvSpawnResult`).
pub async fn probe(web_endpoint: &str, timeout: Duration) -> bool {
    tokio::time::timeout(timeout, probe_inner(web_endpoint))
        .await
        .unwrap_or(false)
}

async fn probe_inner(web_endpoint: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let Some(host_port) = web_endpoint
        .strip_prefix("http://")
        .or_else(|| web_endpoint.strip_prefix("https://"))
    else {
        return false;
    };
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

#[cfg(test)]
mod tests {
    use super::SrvLiveness;
    use std::time::Instant;

    // Each test owns its OWN SrvLiveness instance — no process-global
    // state, no parallel-execution interleaving (same reasoning as
    // ui_liveness's tests).

    #[test]
    fn first_failure_does_not_recycle() {
        let mut l = SrvLiveness::default();
        l.record_failure();
        assert!(!l.should_recycle(3));
    }

    #[test]
    fn required_consecutive_failures_recycles() {
        let mut l = SrvLiveness::default();
        l.record_failure();
        l.record_failure();
        assert!(!l.should_recycle(3), "2 of 3 required misses must not recycle yet");
        l.record_failure();
        assert!(l.should_recycle(3));
    }

    #[test]
    fn success_resets_miss_count() {
        let mut l = SrvLiveness::default();
        l.record_failure();
        l.record_failure();
        l.record_success(Instant::now());
        assert!(l.last_alive().is_some());
        l.record_failure();
        l.record_failure();
        assert!(!l.should_recycle(3), "streak must have reset on success");
    }

    #[test]
    fn reset_clears_state() {
        let mut l = SrvLiveness::default();
        l.record_failure();
        l.record_failure();
        l.record_failure();
        assert!(l.should_recycle(3));
        l.reset();
        assert!(!l.should_recycle(3));
        assert!(l.last_alive().is_none());
    }

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
        let ok = super::probe(&format!("http://{}", addr), std::time::Duration::from_secs(2)).await;
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
        let ok = super::probe(&format!("http://{}", addr), std::time::Duration::from_millis(200)).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn probe_fails_against_nothing_listening() {
        // Bind-then-drop to get a port that's guaranteed refused.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let ok = super::probe(&format!("http://{}", addr), std::time::Duration::from_secs(1)).await;
        assert!(!ok);
    }
}
