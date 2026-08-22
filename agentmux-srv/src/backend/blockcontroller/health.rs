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
    /// True while a known-legitimate long-silence operation (currently:
    /// Claude Code context compaction) is confirmed in progress — see
    /// `set_compacting`'s own doc comment.
    compacting: bool,
    /// When `compacting` last flipped to `true`. `None` when not compacting.
    /// Drives `COMPACTING_DEAD_SECS`, a separate and much longer ceiling than
    /// the normal `STALL_SECS`/`DEAD_SECS` silence thresholds — so a
    /// compaction that itself hangs is still eventually caught, instead of
    /// `compacting=true` suppressing the detector forever.
    compacting_started_ts: Option<Instant>,
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
    /// Dead threshold while `compacting` — a real captured Claude Code
    /// compaction took 231.6s (`docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
    /// §2), nearly double `DEAD_SECS`; Claude Code emits zero intermediate
    /// output for the whole duration (same spec's §7 Tier 4), so `DEAD_SECS`
    /// alone reliably false-positives on a legitimate compaction. 600s is
    /// ~2.6x the one real observed duration — generous enough to cover a
    /// much larger context's compaction without disabling the detector
    /// outright. See docs/specs/SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22.md.
    const COMPACTING_DEAD_SECS: u64 = 600;
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
                compacting: false,
                compacting_started_ts: None,
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

    /// Suspend (`true`) or resume (`false`) the normal `STALL_SECS`/
    /// `DEAD_SECS` silence thresholds for a known-legitimate long-silence
    /// operation — currently only Claude Code context compaction, which
    /// produces zero intermediate output for well over `DEAD_SECS` on a
    /// large context (see `COMPACTING_DEAD_SECS`'s own doc comment).
    /// `COMPACTING_DEAD_SECS` still applies while `compacting`, so an
    /// operation that hangs mid-compaction is eventually still caught
    /// rather than suppressing the detector forever.
    ///
    /// Deliberately does NOT touch `last_meaningful_ts` — resuming
    /// (`false`) re-arms the normal thresholds against whatever silence has
    /// already elapsed, not a fresh grace period. A compaction that ends
    /// into an already-stale stream (e.g. the process died independently
    /// during compaction) should be judged on its own merits, not handed
    /// unearned extra time.
    ///
    /// See docs/specs/SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22.md.
    pub fn set_compacting(&self, compacting: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.compacting = compacting;
        inner.compacting_started_ts = if compacting { Some(Instant::now()) } else { None };
        drop(inner);
        tracing::info!(block_id = %self.block_id, compacting, "[health] compacting flip");
        self.evaluate_and_transition();
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

    /// Whether a confirmed-legitimate long-silence operation (compaction) is
    /// currently suppressing the normal Stalled/Dead thresholds — see
    /// `set_compacting`'s own doc comment.
    pub fn is_compacting(&self) -> bool {
        self.inner.lock().unwrap().compacting
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

            // reagent P1: publishing/persisting AFTER dropping the lock let
            // two concurrent invocations of this function (the 5s watchdog
            // tick vs. record_output/record_error on the stdout-reader task
            // — both call this) race their side effects out of order — e.g.
            // a watchdog-observed Dead->failure publish landing on the wire
            // AFTER a stdout-reader-observed Dead->healthy clear, leaving a
            // stale "Restart" banner over a process that already recovered.
            // Keeping `inner` held across every side effect below makes the
            // whole transition (state mutation + every publish it causes)
            // one atomic critical section, so concurrent invocations'
            // publishes are strictly ordered the same way their state
            // mutations are — whichever invocation's compute_health() call
            // wins the lock race second sees (and publishes) whatever the
            // first one actually left behind, never a stale earlier state
            // published after a fresher one. Safe to hold across these
            // calls: none of them are async (no await inside this fn at
            // all) and none re-lock `self.inner`.
            tracing::info!(
                block_id = %self.block_id,
                old = ?old,
                new = ?new_health,
                "agent health transition"
            );
            self.publish_health(event.clone());

            // Surface Dead as an AgentFailure with a recovery action —
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
                // reagent P1: Dead has two distinct root causes (see
                // compute_health) — a genuinely silent process (the
                // `Unresponsive`/"Restart" case this feature was built for)
                // and a fatal IN-BAND error that was recognized but didn't
                // make the process exit (e.g. an auth failure printed to
                // stderr, process hangs afterward instead of exiting
                // cleanly). Blindly labeling the second case "Unresponsive"
                // would show "Restart" instead of "Login Again" — actively
                // wrong (a restart wouldn't fix an auth problem, and for
                // auth specifically no restart is even needed; the running
                // process re-reads its credential per request). Classify
                // the recorded error text through the same `classify()`
                // used for exit-time classification so the right recovery
                // action shows up instead.
                if inner.errors.has_fatal() {
                    if let Some(ref err_text) = inner.last_error {
                        let classified = crate::agents::failure::classify(None, None, err_text, None);
                        self.publish_failure(&classified);
                    } else {
                        self.publish_unresponsive_failure(&event.detail);
                    }
                } else {
                    self.publish_unresponsive_failure(&event.detail);
                }
            } else if old == AgentHealth::Dead {
                self.clear_unresponsive_failure();
            }
        }
    }

    /// Persist + live-publish an `AgentFailure` for this block. Mirrors the
    /// exit-classification publish pattern in `subprocess/host_spawn.rs`
    /// (persist meta first, durable; then the ephemeral `persist: 1` WPS
    /// push) — see that file's own comment for why the ordering matters.
    /// Best-effort: a `None` `wstore`/`event_bus` (unit tests, or a
    /// controller type that never wired them in) silently no-ops on the
    /// persist half; the WPS push still fires if a broker exists.
    fn publish_failure(&self, failure: &crate::agents::failure::AgentFailure) {
        super::core::persist_last_failure(&self.block_id, Some(failure), &self.wstore, &self.event_bus);
        if let Some(ref broker) = self.broker {
            broker.publish(wps::WaveEvent {
                event: wps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(failure).ok(),
            });
        }
    }

    /// The generic `Unresponsive` case — a genuinely silent process (no
    /// recognized fatal error to classify more specifically). See the
    /// Dead-entry handling in `evaluate_and_transition` for the other case.
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
        self.publish_failure(&failure);
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

        // Confirmed-legitimate long silence (context compaction) — skip the
        // normal Stalled/Dead thresholds, but still bounded by a much more
        // generous ceiling in case the compaction itself hangs. See
        // `set_compacting`'s own doc comment.
        if inner.compacting {
            let compacting_silence = inner
                .compacting_started_ts
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if compacting_silence > Duration::from_secs(Self::COMPACTING_DEAD_SECS) {
                return AgentHealth::Dead;
            }
            return AgentHealth::Healthy;
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
                } else if inner.compacting {
                    // Reached Dead via COMPACTING_DEAD_SECS, not the normal
                    // silence check — report elapsed time since compaction
                    // started, not since the last real output line (which
                    // could be arbitrarily older and would misleadingly
                    // understate/overstate how long the compaction itself
                    // has been running).
                    let secs = inner
                        .compacting_started_ts
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0);
                    format!("Compaction unresponsive for {}s", secs)
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

/// Whether a parsed NDJSON line is Claude Code's `compact_boundary` system
/// frame — the signal that a context compaction has just finished. Callers
/// that classify raw output lines for `record_output` should also check
/// this and call `HealthMonitor::set_compacting(false)` when it's true, so
/// the silence-suppression armed by the earlier `PreCompact` hook signal
/// (`compaction_started`) is torn down the moment compaction genuinely
/// ends — not left dangling until the next unrelated health check. See
/// docs/specs/SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22.md.
///
/// Deliberately permissive about the frame's OTHER fields (unlike the
/// translator's own `compact_boundary` handling in `claude.rs`, which
/// requires a well-formed `compactMetadata` to emit a real event) — this
/// check only needs "compaction is over," not the metadata, so a
/// malformed/partial frame still correctly clears `compacting`.
pub fn is_compact_boundary_frame(parsed: &serde_json::Value) -> bool {
    parsed.get("type").and_then(|v| v.as_str()) == Some("system")
        && parsed.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary")
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
        // publish_unresponsive_failure's own construction, exercised
        // directly — the ONLY path evaluate_and_transition actually takes
        // to this specific class is the plain-silence branch (`!has_fatal()`),
        // which isn't practical to wait out (120s) in a unit test. The
        // has_fatal() branch is covered separately below — it classifies
        // through `agents::failure::classify()` instead, per reagent P1 on
        // PR #2336 (a fatal in-band error like an auth failure must not be
        // mislabeled "Unresponsive"/"Restart").
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = HealthMonitor::new("test-block-dead".to_string(), Some(broker.clone()), None, None);

        monitor.publish_unresponsive_failure("Unresponsive for 120s");

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-dead",
            1,
        );
        assert_eq!(history.len(), 1, "must publish an AgentFailure");
        let data = history[0].data.clone().expect("failure payload must be present");
        assert_eq!(data.get("code").and_then(|v| v.as_str()), Some("unresponsive"));
        assert_eq!(data.get("retryable").and_then(|v| v.as_bool()), Some(false));
    }

    /// reagent P1 on PR #2336: Dead reached via a RECOGNIZED fatal in-band
    /// error (e.g. an auth failure that got printed to stderr but didn't
    /// make the process exit) must publish the correctly-classified
    /// failure, not a blanket "Unresponsive" — showing "Restart" instead of
    /// "Login Again" would actively mislead the user (a restart doesn't fix
    /// an auth problem, and none is even needed — the running process
    /// re-reads its credential per request).
    #[test]
    fn dead_via_recognized_fatal_error_publishes_the_correct_class_not_unresponsive() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = HealthMonitor::new("test-block-dead-auth".to_string(), Some(broker.clone()), None, None);
        monitor.set_active_turn(true);

        monitor.record_error(ErrorClass::Fatal, "Unauthorized: token expired".to_string());
        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Dead);

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-dead-auth",
            1,
        );
        assert_eq!(history.len(), 1, "Dead transition must publish an AgentFailure");
        let data = history[0].data.clone().expect("failure payload must be present");
        assert_eq!(
            data.get("code").and_then(|v| v.as_str()),
            Some("auth"),
            "a recognized auth error reaching Dead must classify as auth, not unresponsive"
        );
    }

    /// An UNRECOGNIZED fatal error still must not be mislabeled
    /// "Unresponsive" (that class is reserved for the plain-silence case) —
    /// `classify()`'s own generic fallback (`unknown_non_zero`) is the
    /// honest answer when nothing matches.
    #[test]
    fn dead_via_unrecognized_fatal_error_falls_back_to_classifys_own_default() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = HealthMonitor::new("test-block-dead-unknown".to_string(), Some(broker.clone()), None, None);
        monitor.set_active_turn(true);

        monitor.record_error(ErrorClass::Fatal, "zzz_totally_unrecognized_internal_error_zzz".to_string());
        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Dead);

        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-dead-unknown",
            1,
        );
        let data = history[0].data.clone().expect("failure payload must be present");
        assert_eq!(data.get("code").and_then(|v| v.as_str()), Some("unknown_non_zero"));
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

    /// Regression for reagent P1: `evaluate_and_transition` used to drop
    /// `inner`'s lock BEFORE publishing/persisting, so two concurrent
    /// invocations (the 5s watchdog vs. record_output/record_error on the
    /// stdout-reader task — the exact shape this method is actually called
    /// from in production) could race their side effects out of order,
    /// leaving a stale failure/clear published after a fresher one. With
    /// the fix (the whole transition, including every publish it causes, is
    /// one atomic critical section under the same lock), the LAST published
    /// `agentfailure` event must always agree with the FINAL `current_health`
    /// — deterministically, regardless of scheduling, not just probably.
    /// Hammers both directions from real OS threads to exercise the actual
    /// race window.
    #[test]
    fn concurrent_transitions_never_leave_a_stale_publish() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = Arc::new(HealthMonitor::new(
            "test-block-race".to_string(),
            Some(broker.clone()),
            None,
            None,
        ));
        monitor.set_active_turn(true);

        let mut handles = Vec::new();
        for i in 0..8 {
            let m = Arc::clone(&monitor);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    if i % 2 == 0 {
                        m.record_error(ErrorClass::Fatal, "boom".to_string());
                    } else {
                        m.set_exited(0);
                        m.set_active_turn(true); // re-arm so the next Fatal can still reach Dead
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let final_health = monitor.inner.lock().unwrap().current_health.clone();
        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-race",
            1,
        );
        match final_health {
            AgentHealth::Dead => {
                assert_eq!(history.len(), 1, "final state is Dead — the last publish must be the failure, not a stale clear");
                assert!(
                    history[0].data.is_some(),
                    "final state is Dead but the last published event was a clear — stale publish ordering regression"
                );
            }
            _ => {
                if let Some(last) = history.last() {
                    assert!(
                        last.data.is_none(),
                        "final state is not Dead but the last published event still carries a failure — stale publish ordering regression"
                    );
                }
            }
        }
    }

    // ---- Compaction false-positive fix (SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22) ----

    #[test]
    fn is_compact_boundary_frame_matches_the_real_frame_shape() {
        // Real shape captured in SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §2 —
        // deliberately includes fields this check doesn't care about (compactMetadata),
        // matching this fn's own "permissive about other fields" doc comment.
        let frame = serde_json::json!({
            "type": "system",
            "subtype": "compact_boundary",
            "content": "Conversation compacted",
            "level": "info",
            "compactMetadata": { "trigger": "manual", "durationMs": 231_606 },
        });
        assert!(is_compact_boundary_frame(&frame));
    }

    #[test]
    fn is_compact_boundary_frame_rejects_other_system_subtypes_and_other_frame_types() {
        assert!(!is_compact_boundary_frame(&serde_json::json!({"type": "system", "subtype": "other_thing"})));
        assert!(!is_compact_boundary_frame(&serde_json::json!({"type": "system"})));
        assert!(!is_compact_boundary_frame(&serde_json::json!({"type": "result", "subtype": "compact_boundary"})));
        assert!(!is_compact_boundary_frame(&serde_json::json!({})));
    }

    fn inner_with(
        active_turn: bool,
        last_meaningful_secs_ago: u64,
        compacting: bool,
        compacting_started_secs_ago: Option<u64>,
    ) -> HealthMonitorInner {
        HealthMonitorInner {
            current_health: AgentHealth::Healthy,
            active_turn,
            last_output_ts: Instant::now(),
            last_meaningful_ts: Instant::now() - Duration::from_secs(last_meaningful_secs_ago),
            errors: ErrorTracker::new(Duration::from_secs(HealthMonitor::ERROR_WINDOW_SECS)),
            exit_code: None,
            last_error: None,
            compacting,
            compacting_started_ts: compacting_started_secs_ago.map(|s| Instant::now() - Duration::from_secs(s)),
        }
    }

    /// The core fix: a turn silent for well over `DEAD_SECS` must NOT be
    /// classified `Dead` while `compacting` is true — this is exactly the
    /// shape a real Claude Code auto-compaction produces (zero output for
    /// well over 120s, per the spec's own captured 231.6s example).
    #[test]
    fn compacting_suppresses_dead_despite_a_last_meaningful_ts_far_past_dead_secs() {
        let inner = inner_with(true, HealthMonitor::DEAD_SECS + 100, true, Some(60));
        assert_eq!(HealthMonitor::compute_health(&inner), AgentHealth::Healthy);
    }

    /// Also suppresses the intermediate `Stalled` classification, not just `Dead`.
    #[test]
    fn compacting_suppresses_stalled_too() {
        let inner = inner_with(true, HealthMonitor::STALL_SECS + 5, true, Some(5));
        assert_eq!(HealthMonitor::compute_health(&inner), AgentHealth::Healthy);
    }

    /// The safety ceiling: compaction itself hanging past `COMPACTING_DEAD_SECS`
    /// must still eventually reach `Dead` — `compacting=true` is a suppression
    /// of the NORMAL thresholds, not a permanent exemption from ever being
    /// flagged unresponsive.
    #[test]
    fn compacting_still_reaches_dead_once_the_compacting_ceiling_is_exceeded() {
        let inner = inner_with(true, 1, true, Some(HealthMonitor::COMPACTING_DEAD_SECS + 1));
        assert_eq!(HealthMonitor::compute_health(&inner), AgentHealth::Dead);
    }

    /// Comfortably under the ceiling must not be Dead — deliberately not an
    /// exact-boundary check (`compacting_started_ts.elapsed()` always ticks
    /// forward slightly between test setup and the `compute_health` call
    /// below, so asserting the exact instant of the threshold against a
    /// real monotonic clock is inherently flaky; `- 1` leaves comfortable
    /// margin while still proving the check is `>`, not `>=`).
    #[test]
    fn compacting_just_under_the_ceiling_is_not_yet_dead() {
        let inner = inner_with(true, 1, true, Some(HealthMonitor::COMPACTING_DEAD_SECS - 1));
        assert_eq!(HealthMonitor::compute_health(&inner), AgentHealth::Healthy);
    }

    /// Not compacting: behavior is completely unchanged from before this fix —
    /// the normal DEAD_SECS threshold still applies exactly as it always did.
    #[test]
    fn non_compacting_turn_is_unaffected_by_the_compacting_fields_existing() {
        let inner = inner_with(true, HealthMonitor::DEAD_SECS + 1, false, None);
        assert_eq!(HealthMonitor::compute_health(&inner), AgentHealth::Dead);
    }

    /// `set_compacting(true)` immediately suppresses Dead even when
    /// `last_meaningful_ts` is already stale past `DEAD_SECS` — the live,
    /// through-the-public-API path (not just the pure `compute_health` unit
    /// tests above).
    #[test]
    fn set_compacting_true_immediately_suppresses_an_already_stale_turn() {
        let monitor = HealthMonitor::new("test-block-compacting-1".to_string(), None, None, None);
        monitor.set_active_turn(true);
        {
            let mut inner = monitor.inner.lock().unwrap();
            inner.last_meaningful_ts = Instant::now() - Duration::from_secs(HealthMonitor::DEAD_SECS + 1);
        }
        monitor.set_compacting(true);
        assert!(monitor.is_compacting());
        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Healthy);
    }

    /// `set_compacting(false)` must NOT reset `last_meaningful_ts` — resuming
    /// re-arms the normal thresholds against whatever silence has ALREADY
    /// elapsed, not a fresh grace period. A compaction that "ends" (or whose
    /// signal is cleared for any reason) into an already-dead-silent stream
    /// must still be judged Dead immediately, not given another free 120s.
    #[test]
    fn set_compacting_false_grants_no_fresh_grace_period() {
        let monitor = HealthMonitor::new("test-block-compacting-2".to_string(), None, None, None);
        monitor.set_active_turn(true);
        {
            let mut inner = monitor.inner.lock().unwrap();
            inner.last_meaningful_ts = Instant::now() - Duration::from_secs(HealthMonitor::DEAD_SECS + 1);
        }
        monitor.set_compacting(true);
        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Healthy);

        monitor.set_compacting(false);
        assert!(!monitor.is_compacting());
        assert_eq!(
            monitor.inner.lock().unwrap().current_health,
            AgentHealth::Dead,
            "resuming must re-evaluate against the already-stale last_meaningful_ts immediately"
        );
    }

    /// A real `compact_boundary` frame arriving while `compacting` is true
    /// must clear it — the wiring each controller's stdout-reader loop uses
    /// (`is_compact_boundary_frame` + `set_compacting(false)`), exercised
    /// here at the `HealthMonitor` level (per-controller wiring itself is
    /// mechanical and not independently tested).
    #[test]
    fn compact_boundary_frame_detection_pairs_with_set_compacting_false() {
        let monitor = HealthMonitor::new("test-block-compacting-3".to_string(), None, None, None);
        monitor.set_active_turn(true);
        monitor.set_compacting(true);
        assert!(monitor.is_compacting());

        let frame = serde_json::json!({
            "type": "system",
            "subtype": "compact_boundary",
            "compactMetadata": { "trigger": "auto" },
        });
        assert!(is_compact_boundary_frame(&frame));
        monitor.set_compacting(false);
        assert!(!monitor.is_compacting());
    }

    /// Reaching Dead via the compacting ceiling (not the normal silence path)
    /// must still publish a real `AgentFailure` (code `unresponsive`) — the
    /// escape hatch is scoped to false positives, not to hiding a genuine
    /// hang that happens to occur mid-compaction.
    #[test]
    fn compacting_ceiling_dead_still_publishes_an_unresponsive_failure() {
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let monitor = HealthMonitor::new("test-block-compacting-dead".to_string(), Some(broker.clone()), None, None);
        monitor.set_active_turn(true);
        monitor.set_compacting(true);
        {
            let mut inner = monitor.inner.lock().unwrap();
            inner.compacting_started_ts = Some(Instant::now() - Duration::from_secs(HealthMonitor::COMPACTING_DEAD_SECS + 1));
        }
        monitor.check();

        assert_eq!(monitor.inner.lock().unwrap().current_health, AgentHealth::Dead);
        let history = broker.read_event_history(
            crate::backend::wps::EVENT_AGENT_FAILURE,
            "block:test-block-compacting-dead",
            1,
        );
        assert_eq!(history.len(), 1, "must publish an AgentFailure even when Dead was reached via the compacting ceiling");
        let data = history[0].data.clone().expect("failure payload must be present");
        assert_eq!(data.get("code").and_then(|v| v.as_str()), Some("unresponsive"));
    }
}
