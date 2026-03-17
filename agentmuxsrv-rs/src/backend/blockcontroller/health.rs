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

    pub fn new(block_id: String, broker: Option<Arc<wps::Broker>>) -> Self {
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
        self.evaluate_and_transition();
    }

    /// Called when the subprocess exits.
    pub fn set_exited(&self, exit_code: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_turn = false;
        inner.exit_code = Some(exit_code);
        drop(inner);
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
            self.publish_health(event);
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
        let monitor = HealthMonitor::new("test-block".to_string(), None);

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
        let monitor = HealthMonitor::new("test-block".to_string(), None);
        monitor.set_active_turn(true);

        monitor.record_error(ErrorClass::Fatal, "Unauthorized".to_string());
        {
            let inner = monitor.inner.lock().unwrap();
            assert_eq!(inner.current_health, AgentHealth::Dead);
        }
    }
}
