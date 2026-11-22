// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The private and secret rectangles of each AgentMux window, read at the instant of a `UI` window capture
//! (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §7, G5).
//!
//! The capturing agent's MCP asks twice through `POST /api/v1/ui/capture_mask`: `begin` just before it takes the
//! frame, `end` just after. Each ask publishes `capture:mask` to every window, and each window answers
//! `capture.mask.report` with its rectangles, its size and scale, and (for `end`) whether anything changed since
//! `begin`. The answer is accepted only from the AgentMux window (`operator_only`), so an agent can't report a mask.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::mps_events::CaptureMaskEvent;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::AppState;
use agentmux_common::time::now_ms;

pub const COMMAND_CAPTURE_MASK_REPORT: &str = "capture.mask.report";
/// How long an ask waits for the windows' answers.
pub const WAIT_FOR: Duration = Duration::from_millis(1500);
/// The most rectangles one window may report.
const MAX_RECTS: usize = 64;

/// A rectangle in the window's viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CaptureRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One window's answer.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WindowCaptureMask {
    pub window_id: String,
    /// The window's title, as the operating system shows it.
    pub title: String,
    /// The viewport in CSS pixels, and the device pixels per CSS pixel.
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    /// Each pane or panel showing private or secret content.
    pub rects: Vec<CaptureRect>,
    /// The layout is moving (a resize, a drag, a pane animating, a tab switching).
    pub moving: bool,
    /// `end` only: the layout changed at some frame since `begin`.
    pub changed: bool,
    /// Set when the window can't give its rectangles: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub refuse: Option<String>,
}

impl WindowCaptureMask {
    fn well_formed(&self) -> bool {
        let finite = |v: f64| v.is_finite() && (0.0..=100_000.0).contains(&v);
        self.window_id.len() <= 128
            && self.title.len() <= 1024
            && finite(self.width)
            && finite(self.height)
            && self.scale.is_finite()
            && self.scale > 0.0
            && self.scale <= 16.0
            && self.rects.len() <= MAX_RECTS
            && self.rects.iter().all(|r| r.x.is_finite() && r.y.is_finite() && finite(r.w) && finite(r.h))
            && self.refuse.as_ref().is_none_or(|t| t.len() <= 500)
    }
}

/// Data for `capture.mask.report`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandCaptureMaskReportData {
    pub capture_id: String,
    #[ts(type = r#""begin" | "end""#)]
    pub phase: String,
    pub mask: WindowCaptureMask,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CaptureMaskReportResult {
    #[ts(type = r#""recorded" | "too_late""#)]
    pub status: String,
}

struct Entry {
    masks: Vec<WindowCaptureMask>,
    expected: usize,
    wake: Arc<tokio::sync::Notify>,
    expires_at_ms: i64,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn entries() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

fn key(capture_id: &str, phase: &str) -> String {
    format!("{phase}:{capture_id}")
}

/// How many windows srv knows, so an ask stops waiting once each has answered.
fn window_count(state: &AppState) -> usize {
    state.mstore.get_all::<crate::backend::obj::Window>().map(|w| w.len()).unwrap_or(0)
}

/// Ask every window for its mask and gather the answers that come within `wait`.
pub async fn ask(state: &AppState, capture_id: &str, phase: &str, wait: Duration) -> Vec<WindowCaptureMask> {
    gather(capture_id, phase, window_count(state), wait, || {
        let event = CaptureMaskEvent { capture_id: capture_id.to_string(), phase: phase.to_string(), expires_at_ms: now_ms() + wait.as_millis() as i64 };
        state.broker.publish_event::<crate::backend::mps_events::CaptureMask>(&event, vec![], 0);
    })
    .await
}

async fn gather(capture_id: &str, phase: &str, expected: usize, wait: Duration, publish: impl FnOnce()) -> Vec<WindowCaptureMask> {
    let k = key(capture_id, phase);
    let wake = Arc::new(tokio::sync::Notify::new());
    {
        let mut map = entries();
        let now = now_ms();
        map.retain(|_, e| e.expires_at_ms > now);
        map.insert(k.clone(), Entry { masks: Vec::new(), expected: expected.max(1), wake: wake.clone(), expires_at_ms: now + wait.as_millis() as i64 });
    }
    publish();
    let _ = tokio::time::timeout(wait, async {
        loop {
            let notified = wake.notified();
            if entries().get(&k).is_none_or(|e| e.masks.len() >= e.expected) {
                return;
            }
            notified.await;
        }
    })
    .await;
    entries().remove(&k).map(|e| e.masks).unwrap_or_default()
}

/// A window's answer to the ask `capture_id`, `phase`.
pub fn report(capture_id: &str, phase: &str, mask: WindowCaptureMask) -> &'static str {
    if !mask.well_formed() {
        return "too_late";
    }
    let mut map = entries();
    let Some(entry) = map.get_mut(&key(capture_id, phase)).filter(|e| e.expires_at_ms > now_ms()) else {
        return "too_late";
    };
    if entry.masks.iter().any(|m| m.window_id == mask.window_id) {
        return "too_late";
    }
    entry.masks.push(mask);
    entry.wake.notify_one();
    "recorded"
}

/// `capture.mask.report`, accepted only from the proven AgentMux window (`operator_only`), never from an agent.
pub fn register(engine: &Arc<WshRpcEngine>) {
    engine.register_typed(COMMAND_CAPTURE_MASK_REPORT, move |cmd: CommandCaptureMaskReportData, ctx| async move {
        ctx.operator_only("only the AgentMux window reports what it shows")?;
        Ok(CaptureMaskReportResult { status: report(&cmd.capture_id, &cmd.phase, cmd.mask).to_string() })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(window: &str) -> WindowCaptureMask {
        WindowCaptureMask {
            window_id: window.into(),
            title: "AgentMux".into(),
            width: 800.0,
            height: 600.0,
            scale: 1.5,
            rects: vec![CaptureRect { x: 10.0, y: 20.0, w: 300.0, h: 200.0 }],
            ..Default::default()
        }
    }

    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[tokio::test]
    async fn an_ask_gathers_each_windows_answer_and_stops_once_all_have_answered() {
        let cid = id();
        let started = std::time::Instant::now();
        let task = {
            let cid = cid.clone();
            tokio::spawn(async move { gather(&cid, "begin", 2, Duration::from_secs(5), || {}).await })
        };
        for w in ["w1", "w2"] {
            for _ in 0..200 {
                if report(&cid, "begin", mask(w)) == "recorded" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        let got = task.await.unwrap();
        assert_eq!(got.len(), 2);
        assert!(started.elapsed() < Duration::from_secs(4), "it did not wait out the whole time");
        assert_eq!(report(&cid, "begin", mask("w3")), "too_late", "an ask that ended takes no answer");
    }

    #[tokio::test]
    async fn a_window_answers_once_and_the_two_phases_are_kept_apart() {
        let cid = id();
        let task = {
            let cid = cid.clone();
            tokio::spawn(async move { gather(&cid, "end", 3, Duration::from_millis(300), || {}).await })
        };
        for _ in 0..200 {
            if report(&cid, "end", mask("w1")) == "recorded" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(report(&cid, "end", mask("w1")), "too_late");
        assert_eq!(report(&cid, "begin", mask("w2")), "too_late", "no begin ask is open");
        assert_eq!(task.await.unwrap(), vec![mask("w1")]);
    }

    #[test]
    fn a_malformed_mask_is_not_taken() {
        let mut bad = mask("w1");
        bad.scale = 0.0;
        assert!(!bad.well_formed());
        let mut many = mask("w1");
        many.rects = vec![CaptureRect::default(); MAX_RECTS + 1];
        assert!(!many.well_formed());
        let mut nan = mask("w1");
        nan.rects[0].x = f64::NAN;
        assert!(!nan.well_formed());
        assert_eq!(report("nothing-asked", "begin", mask("w1")), "too_late");
    }
}
