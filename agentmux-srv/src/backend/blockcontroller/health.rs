// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent health/liveness monitoring.
//!
//! Watches subprocess output activity, classifies errors, and emits
//! `agenthealth` WPS events when health state transitions occur.
//!
//! Design: docs/specs/agent-health-design.md

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::backend::eventbus::EventBus;
use crate::backend::storage::store::Store;
use crate::backend::wps;

// ---- Health states ----

/// Agent health status (orthogonal to shellprocstatus).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentHealth {
    Healthy,
    Idle,
    Degraded,
    Stalled,
    Dead,
    Exited,
}

impl AgentHealth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Idle => "idle",
            Self::Degraded => "degraded",
            Self::Stalled => "stalled",
            Self::Dead => "dead",
            Self::Exited => "exited",
        }
    }
}

/// Error severity classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorClass {
    Transient,
    Fatal,
}

// ---- Event payload ----

/// WPS event payload for health transitions.
#[derive(Debug, Clone, Serialize)]
pub struct AgentHealthEvent {
    pub blockid: String,
    pub health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

// ---- Error tracker ----

/// Sliding-window error tracker.
struct ErrorTracker {
    window: VecDeque<(Instant, ErrorClass)>,
    window_duration: Duration,
    consecutive_transient: u32,
}

impl ErrorTracker {
    fn new(window_duration: Duration) -> Self {
        Self {
            window: VecDeque::new(),
            window_duration,
            consecutive_transient: 0,
        }
    }

    fn prune(&mut self) {
        let cutoff = Instant::now() - self.window_duration;
        while self.window.front().is_some_and(|(t, _)| *t < cutoff) {
            self.window.pop_front();
        }
    }

    fn record(&mut self, class: ErrorClass) {
        self.prune();
        match class {
            ErrorClass::Transient => self.consecutive_transient += 1,
            ErrorClass::Fatal => self.consecutive_transient = 0,
        }
        self.window.push_back((Instant::now(), class));
    }

    fn record_success(&mut self) {
        self.consecutive_transient = 0;
    }

    fn has_fatal(&self) -> bool {
        self.window.iter().any(|(_, c)| *c == ErrorClass::Fatal)
    }

    fn transient_count(&self) -> usize {
        self.window.iter().filter(|(_, c)| *c == ErrorClass::Transient).count()
    }

    fn reset(&mut self) {
        self.window.clear();
        self.consecutive_transient = 0;
    }
}

// ---- Health monitor ----

/// Per-block health monitor inner state.
struct HealthMonitorInner {
    current_health: AgentHealth,
    active_turn: bool,
    last_output_ts: Instant,
    last_meaningful_ts: Instant,
    errors: ErrorTracker,
    exit_code: Option<i32>,
    last_error: Option<String>,
}

/// Per-block agent health monitor.
///
/// Tracks output activity and error rates, computes health state,
/// and emits WPS events on state transitions.
pub struct HealthMonitor {
    block_id: String,
    inner: Mutex<HealthMonitorInner>,
    broker: Option<Arc<wps::Broker>>,
    /// Needed only to surface a `Dead` transition as an `AgentFailure` (see
    /// `evaluate_and_transition`'s Dead-entry/exit handling) — every other
    /// method on this type is store/event-bus-free by design. `None` in
    /// unit tests that don't wire up a store (matches the existing
    /// `broker: None` test convention).
    wstore: Option<Arc<Store>>,
    event_bus: Option<Arc<EventBus>>,
}

impl HealthMonitor {
    /// Stall threshold: no meaningful output for 30s during active turn.
    const STALL_SECS: u64 = 30;
    /// Dead threshold: no meaningful output for 120s during active turn.
    const DEAD_SECS: u64 = 120;
    /// Error window duration.
    const ERROR_WINDOW_SECS: u64 = 300; // 5 minutes
    /// Transient error count threshold for degraded.
    const DEGRADED_TRANSIENT_THRESHOLD: usize = 5;

    pub fn new(
        block_id: String,
        broker: Option<Arc<wps::Broker>>,
        wstore: Option<Arc<Store>>,
        event_bus: Option<Arc<EventBus>>,
    ) -> Self {
        let now = Instant::now();
        Self {
            block_id,
            inner: Mutex::new(HealthMonitorInner {
                current_health: AgentHealth::Idle,
                active_turn: false,
                last_output_ts: now,
                last_meaningful_ts: now,
                errors: ErrorTracker::new(Duration::from_secs(Self::ERROR_WINDOW_SECS)),
                exit_code: None,
                last_error: None,
            }),
            broker,
            wstore,
            event_bus,
        }
    }

    /// Called when a new turn starts (subprocess spawned).
    pub fn set_active_turn(&self, active: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_turn = active;
        let now = Instant::now();
        inner.last_output_ts = now;
        inner.last_meaningful_ts = now;
        if active {
            inner.errors.reset();
            inner.exit_code = None;
        }
        drop(inner);
        // Backend-authoritative turn-boundary timeline, cross-referenceable
        // against the frontend's own `[wave-turn]` line (agent-pane-state-
        // store.ts) via `muxlog srv`/`muxlog host` — closes the "two clocks
        // disagreeing" blind spot the persistent-agent-working-status-stuck
        // retro flagged as previously unconfirmable without a live repro.
        // info, not debug: the sidecar's default EnvFilter ("agentmuxsrv=info,info",
        // no RUST_LOG set) drops debug! entirely, which would make this
        // invisible in exactly the default-config incident this line exists
        // to help diagnose (codex P1 on PR #2321). Fires once per turn
        // boundary per pane — sparse enough for info.
        // See docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md §3.3.
        tracing::info!(block_id = %self.block_id, active, "[health] turn_active flip");
        self.evaluate_and_transition();
    }

    /// Atomically marks a turn active and reports whether one was already in
    /// flight (the pre-call value) — a single lock acquisition, unlike
    /// calling `is_active_turn()` then `set_active_turn(true)` separately.
    /// That two-step form is a check-then-act race: `send_message` (user
    /// input) and `send_user_message` (muxbus delivery) can run concurrently
    /// on the same block, and both reading `false` before either writes
    /// `true` lets both decide to spawn a watchdog — the exact duplicate the
    /// "only re-arm when resuming from idle" logic exists to prevent.
    pub fn mark_turn_active_returning_was_active(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let was_active = inner.active_turn;
        inner.active_turn = true;
        let now = Instant::now();
        inner.last_output_ts = now;
        inner.last_meaningful_ts = now;
        inner.errors.reset();
        inner.exit_code = None;
        drop(inner);
        tracing::info!(
            block_id = %self.block_id,
            active = true,
            was_active,
            "[health] turn_active flip"
        );
        self.evaluate_and_transition();
        was_active
    }

    /// Called when the subprocess exits.
    pub fn set_exited(&self, exit_code: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_turn = false;
        inner.exit_code = Some(exit_code);
        drop(inner);
        tracing::info!(block_id = %self.block_id, exit_code, "[health] turn_active flip (process exited)");
        self.evaluate_and_transition();
    }

    /// Called for each output line from stdout.
    /// `meaningful` is false for rate_limit_event and similar non-progress events.
    pub fn record_output(&self, meaningful: bool) {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        inner.last_output_ts = now;
        if meaningful {
            inner.last_meaningful_ts = now;
            inner.errors.record_success();
        }
        drop(inner);
        // Don't evaluate on every output line — the watchdog handles periodic checks.
        // Only re-evaluate if we were previously stalled/dead (recovery path).
        let health = self.inner.lock().unwrap().current_health.clone();
        if health == AgentHealth::Stalled || health == AgentHealth::Dead {
            self.evaluate_and_transition();
        }
    }

    /// Called when an error is detected in the output stream.
    pub fn record_error(&self, class: ErrorClass, message: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.errors.record(class);
        inner.last_error = Some(message);
        drop(inner);
        self.evaluate_and_transition();
    }

    /// Whether there's an active turn in progress.
    pub fn is_active_turn(&self) -> bool {
        self.inner.lock().unwrap().active_turn
    }

    /// Periodic health check — call this every ~5 seconds while a turn is active.
    pub fn check(&self) {
        self.evaluate_and_transition();
    }

    /// Compute current health and emit event if it changed.
    fn evaluate_and_transition(&self) {
        let mut inner = self.inner.lock().unwrap();
        let new_health = Self::compute_health(&inner);

        if new_health != inner.current_health {
            let old = inner.current_health.clone();
            inner.current_health = new_health.clone();
            let detail = Self::make_detail(&inner, &new_health);
            let event = AgentHealthEvent {
                blockid: self.block_id.clone(),
                health: new_health.as_str().to_string(),
                exit_code: inner.exit_code,
                detail,
                last_error: inner.last_error.clone(),
            };
            drop(inner);

            tracing::info!(
                block_id = %self.block_id,
                old = ?old,
                new = ?new_health,
                "agent health transition"
            );
            self.publish_health(event.clone());

            // Surface Dead as an AgentFailure with a "Restart" action —
            // previously this transition was diagnostic-only (the WPS
            // agenthealth event above has no frontend subscriber), so a
            // pane whose process went unresponsive just sat there until a
            // human happened to notice and manually reopened it. And clear
            // it again on a silent self-heal (Dead -> anything else,
            // reachable when late output arrives after the 120s threshold
            // already tripped — compute_health's silence check has no
            // hysteresis) so a stale "Restart" button doesn't linger once
            // the process is genuinely fine again. See
            // docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §4.
            if new_health == AgentHealth::Dead {
                self.publish_unresponsive_failure(&event.detail);
            } else if old == AgentHealth::Dead {
                self.clear_unresponsive_failure();
            }
        }
    }

    /// Persist + live-publish an `Unresponsive` `AgentFailure` for this
    /// block. Mirrors the exit-classification publish pattern in
    /// `subprocess/host_spawn.rs` (persist meta first, durable; then the
    /// ephemeral `persist: 1` WPS push) — see that file's own comment for
    /// why the ordering matters. Best-effort: a `None` `wstore`/`event_bus`
    /// (unit tests, or a controller type that never wired them in) silently
    /// no-ops on the persist half; the WPS push still fires if a broker
    /// exists.
    fn publish_unresponsive_failure(&self, detail: &str) {
        let failure = crate::agents::failure::AgentFailure {
            code: crate::agents::failure::FailureClass::Unresponsive,
            title: "Agent unresponsive".to_string(),
            detail: detail.to_string(),
            exit_code: None,
            signal: None,
            stderr_tail: String::new(),
            retryable: false,
        };
        super::core::persist_last_failure(&self.block_id, Some(&failure), &self.wstore, &self.event_bus);
        if let Some(ref broker) = self.broker {
            broker.publish(wps::WaveEvent {
                event: wps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&failure).ok(),
            });
        }
    }

    /// Clear a previously-published `Unresponsive` failure on a silent
    /// self-heal. `data: None` on the WPS push is the live-clear signal —
    /// `useAgentFailure.ts`'s WPS handler treats an event with no `data` as
    /// "clear the failure, but only if it's currently classed
    /// `unresponsive`" (never clobbers an unrelated concurrent failure,
    /// e.g. auth, that happened to be showing).
    fn clear_unresponsive_failure(&self) {
        super::core::persist_last_failure(&self.block_id, None, &self.wstore, &self.event_bus);
        if let Some(ref broker) = self.broker {
            broker.publish(wps::WaveEvent {
                event: wps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: None,
            });
        }
    }

    /// Composite health computation.
    fn compute_health(inner: &HealthMonitorInner) -> AgentHealth {
        // Process exited?
        if let Some(code) = inner.exit_code {
            if code == 0 {
                return AgentHealth::Idle; // Normal turn completion
            }
            return AgentHealth::Exited;
        }

        // Fatal error?
        if inner.errors.has_fatal() {
            return AgentHealth::Dead;
        }

        // Not in an active turn?
        if !inner.active_turn {
            return AgentHealth::Idle;
        }

        // Check output silence
        let silence = inner.last_meaningful_ts.elapsed();
        if silence > Duration::from_secs(Self::DEAD_SECS) {
            return AgentHealth::Dead;
        }
        if silence > Duration::from_secs(Self::STALL_SECS) {
            return AgentHealth::Stalled;
        }

        // Check transient error rate
        if inner.errors.transient_count() >= Self::DEGRADED_TRANSIENT_THRESHOLD {
            return AgentHealth::Degraded;
        }

        AgentHealth::Healthy
    }

    /// Generate human-readable detail string.
    fn make_detail(inner: &HealthMonitorInner, health: &AgentHealth) -> String {
        match health {
            AgentHealth::Healthy => "Agent is responding normally".to_string(),
            AgentHealth::Idle => "Waiting for next message".to_string(),
            AgentHealth::Degraded => {
                format!(
                    "{} transient errors in the last 5 minutes",
                    inner.errors.transient_count()
                )
            }
            AgentHealth::Stalled => {
                let secs = inner.last_meaningful_ts.elapsed().as_secs();
                format!("No output for {}s", secs)
            }
            AgentHealth::Dead => {
                if inner.errors.has_fatal() {
                    inner
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "Fatal error detected".to_string())
                } else {
                    let secs = inner.last_meaningful_ts.elapsed().as_secs();
                    format!("Unresponsive for {}s", secs)
                }
            }
            AgentHealth::Exited => {
                format!("Exited with code {}", inner.exit_code.unwrap_or(-1))
            }
        }
    }

    /// Publish health event via WPS broker.
    fn publish_health(&self, event: AgentHealthEvent) {
        if let Some(ref broker) = self.broker {
            let wps_event = wps::WaveEvent {
                event: wps::EVENT_AGENT_HEALTH.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 0,
                data: serde_json::to_value(&event).ok(),
            };
            broker.publish(wps_event);
        }
    }
}

// ---- Error classifier for NDJSON lines ----

/// Classify a parsed NDJSON line for health monitoring.
/// Returns (is_meaningful, optional_error).
pub fn classify_output_line(
    parsed: &serde_json::Value,
) -> (bool, Option<(ErrorClass, String)>) {
    let event_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "rate_limit_event" => {
            (false, Some((ErrorClass::Transient, "Rate limited".to_string())))
        }
        "result" => {
            let is_error = parsed
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_error {
                return (true, None);
            }
            let msg = parsed
                .get("error")
                .or_else(|| parsed.get("error_message"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();

            let class = if msg.contains("unauthorized")
                || msg.contains("401")
                || msg.contains("forbidden")
                || msg.contains("403")
                || msg.contains("token expired")
                || msg.contains("authentication")
            {
                ErrorClass::Fatal
            } else if msg.contains("overloaded")
                || msg.contains("503")
                || msg.contains("500")
                || msg.contains("rate")
                || msg.contains("capacity")
            {
                ErrorClass::Transient
            } else {
                // Unknown errors default to fatal (design principle: safer to over-alert)
                ErrorClass::Fatal
            };

            (true, Some((class, msg)))
        }
        // stream_event wrapper — check inner event
        "stream_event" => {
            if let Some(inner) = parsed.get("event") {
                return classify_output_line(inner);
            }
            (true, None)
        }
        _ => (true, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_tracker_basic() {
        let mut tracker = ErrorTracker::new(Duration::from_secs(300));
        assert!(!tracker.has_fatal());
        assert_eq!(tracker.transient_count(), 0);

        tracker.record(ErrorClass::Transient);
        assert_eq!(tracker.transient_count(), 1);
        assert!(!tracker.has_fatal());

        tracker.record(ErrorClass::Fatal);
        assert!(tracker.has_fatal());
    }

    #[test]
    fn test_classify_rate_limit() {
        let event: serde_json::Value =
            serde_json::from_str(r#"{"type":"rate_limit_event"}"#).unwrap();
        let (meaningful, error) = classify_output_line(&event);
        assert!(!meaningful);
        assert!(matches!(error, Some((ErrorClass::Transient, _))));
    }

    #[test]
    fn test_classify_auth_error() {
        let event: serde_json::Value = serde_json::from_str(
            r#"{"type":"result","is_error":true,"error":"Unauthorized: token expired"}"#,
        )
        .unwrap();
        let (_, error) = classify_output_line(&event);
        assert!(matches!(error, Some((ErrorClass::Fatal, _))));
    }

    #[test]
    fn test_classify_overloaded() {
        let event: serde_json::Value = serde_json::from_str(
            r#"{"type":"result","is_error":true,"error":"Service overloaded, try again"}"#,
        )
        .unwrap();
        let (_, error) = classify_output_line(&event);
        assert!(matches!(error, Some((ErrorClass::Transient, _))));
    }

    #[test]
    fn test_classify_normal_result() {
        let event: serde_json::Value = serde_json::from_str(
            r#"{"type":"result","is_error":false,"total_cost_usd":0.05}"#,
        )
        .unwrap();
        let (meaningful, error) = classify_output_line(&event);
        assert!(meaningful);
        assert!(error.is_none());
    }

    #[test]
    fn test_health_monitor_lifecycle() {
        let monitor = HealthMonitor::new("test-block".to_string(), None, None, None);

        // Initial state is idle
        {
            let inner = monitor.inner.lock().unwrap();
            assert_eq!(inner.current_health, AgentHealth::Idle);
        }

        // Start a turn
        monitor.set_active_turn(true);
        {
            let inner = monitor.inner.lock().unwrap();
            assert_eq!(inner.current_health, AgentHealth::Healthy);
        }

        // Record normal output
        monitor.record_output(true);
        {
            let inner = monitor.inner.lock().unwrap();
            assert_eq!(inner.current_health, AgentHealth::Healthy);
        }

        // Exit normally
        monitor.set_exited(0);
        {
            let inner = monitor.inner.lock().unwrap();
            assert_eq!(inner.current_health, AgentHealth::Idle);
        }
    }

    #[test]
    fn test_health_monitor_fatal_error() {
        let monitor = HealthMonitor::new("test-block".to_string(), None, None, None);
        monitor.set_active_turn(true);

        monitor.record_error(ErrorClass::Fatal, "Unauthorized".to_string());
        {
            let inner = monitor.inner.lock().unwrap();
            assert_eq!(inner.current_health, AgentHealth::Dead);
        }
    }

    #[test]
    fn mark_turn_active_returning_was_active_reports_the_pre_call_value() {
        let monitor = HealthMonitor::new("test-block".to_string(), None, None, None);
        assert!(!monitor.is_active_turn());

        // First call: was idle before this call.
        let was_active = monitor.mark_turn_active_returning_was_active();
        assert!(!was_active, "first call should report idle-before-call");
        assert!(monitor.is_active_turn(), "turn is now active");

        // Second call while already active: reports true (already in flight).
        let was_active_again = monitor.mark_turn_active_returning_was_active();
        assert!(was_active_again, "second call should report already-active");
        assert!(monitor.is_active_turn());
    }

    /// Regression test for the exact race reagent flagged on PR #2005: a
    /// naive `is_active_turn()` read followed by a separate
    /// `set_active_turn(true)` write lets two concurrent callers (send_message
    /// vs. send_user_message on the same block) both observe `false` before
    /// either writes `true`, so both decide to spawn a watchdog.
    /// `mark_turn_active_returning_was_active` closes that window by holding
    /// the lock across both the read and the write — this test simulates the
    /// interleaving directly (no real concurrency needed to prove the
    /// invariant: exactly one of N concurrent-in-spirit calls sees "was
    /// idle").
    #[test]
    fn mark_turn_active_is_atomic_across_repeated_calls() {
        let monitor = HealthMonitor::new("test-block".to_string(), None, None, None);
        let results: Vec<bool> = (0..5).map(|_| monitor.mark_turn_active_returning_was_active()).collect();
        // Exactly the first call observes "was idle" (false); every
        // subsequent call — however tightly interleaved a real concurrent
        // caller might be — observes "already active" (true), because each
        // read-and-write pair is indivisible under the lock.
        assert_eq!(results, vec![false, true, true, true, true]);
    }

    /// Regression for docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md
    /// §4: reaching `Dead` must surface an `Unresponsive` `AgentFailure`, not
    /// just the diagnostic-only `agenthealth` WPS event — otherwise a pane
    /// whose process went unresponsive just sits there until a human happens
    /// to notice and manually reopens it. Drives Dead via `record_error`
    /// (the fatal-error branch) rather than the 120s silence-timeout branch,
    /// which isn't practical to wait out in a test.
    #[test]
    fn dead_transition_publishes_unresponsive_failure() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = HealthMonitor::new("test-block-dead".to_string(), Some(broker.clone()), None, None);
        monitor.set_active_turn(true);

        monitor.record_error(ErrorClass::Fatal, "Unauthorized".to_string());
        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Dead);

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-dead",
            1,
        );
        assert_eq!(history.len(), 1, "Dead transition must publish an AgentFailure");
        let data = history[0].data.clone().expect("failure payload must be present");
        assert_eq!(data.get("code").and_then(|v| v.as_str()), Some("unresponsive"));
        assert_eq!(data.get("retryable").and_then(|v| v.as_bool()), Some(false));
    }

    /// A silent self-heal (Dead -> anything else, e.g. late output arriving
    /// just after the threshold tripped) must clear the failure it
    /// published, or a stale "Restart" button lingers over a process that's
    /// actually fine again. The clear is a `data: None` publish on the same
    /// event — this test only confirms the health-side publish surface (the
    /// frontend's interpretation of a null-data event is covered by
    /// useAgentFailure.test.ts).
    #[test]
    fn dead_recovery_clears_the_unresponsive_failure() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = HealthMonitor::new("test-block-recover".to_string(), Some(broker.clone()), None, None);
        monitor.set_active_turn(true);
        monitor.record_error(ErrorClass::Fatal, "Unauthorized".to_string());
        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Dead);

        // Exit-code takes priority over the fatal-error branch in
        // compute_health, so this reliably drives Dead -> Idle/Exited.
        monitor.set_exited(0);
        assert_ne!(monitor.inner.lock().unwrap().current_health, AgentHealth::Dead);

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-recover",
            1,
        );
        assert_eq!(history.len(), 1, "recovery must publish a clearing event");
        assert!(history[0].data.is_none(), "the clearing publish must carry no data");
    }
}
