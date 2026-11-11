// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's zoom of a browser pane it drives (docs/specs/SPEC_BROWSER_PANE_AGENT_ZOOM_2026_11_10.md).
//!
//! Done with DevTools device-metrics emulation on the pane's own browser, not Chromium's page zoom: that lives
//! in the profile's `HostZoomMap`, so an agent's zoom would change the person's zoom for the site, be shared by
//! the agent's other panes on it and outlive the pane. The emulated viewport is `pane / factor` CSS pixels,
//! drawn at `factor`, so the page lays out as a window that size would. Refs and `getBoundingClientRect` stay
//! in the page's CSS pixels; only the mouse events DevTools is sent are in the scaled view, so `click_at`
//! multiplies by `input_scale`.
//!
//! The emulation outlives a call (`CdpSession::close` is a no-op: the DevTools channel stays), so the factor is
//! kept here per pane: for the mouse, for the note after every answer, for the resize hook and so it can be
//! cleared when the pane is no longer the agent's.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};

use super::act::{fail, unauthorized};
use super::cdp::CdpSession;
use super::types::ApiResponse;
use crate::state::AppState;

/// The smallest and largest factor (25% to 200%).
pub const MIN_FACTOR: f64 = 0.25;
pub const MAX_FACTOR: f64 = 2.0;
const MIN_WIDTH: f64 = 320.0;
const MAX_WIDTH: f64 = 3840.0;
/// How long a pane's size has to hold still before its zoom follows it.
const RESIZE_SETTLE: Duration = Duration::from_millis(200);
/// The page takes a moment to show its own size again after the emulation is cleared: how often to look, and
/// how many times at most.
const CLEAR_SETTLE_POLL: Duration = Duration::from_millis(50);
const CLEAR_SETTLE_POLLS: u32 = 10;

/// What the agent asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ask {
    /// This factor.
    Factor(f64),
    /// A viewport this many CSS pixels wide, scaled to fit the pane.
    Width(f64),
    /// Zoom out until the page's whole width fits the pane.
    Fit,
}

/// The emulation to apply: a `width` x `height` CSS-pixel viewport drawn at `scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

fn clamp_factor(f: f64) -> f64 {
    f.clamp(MIN_FACTOR, MAX_FACTOR)
}

/// The viewport for a pane of `pane_w` x `pane_h` CSS pixels. `content_w` is the page's width at the pane's
/// own size, which `Fit` needs. A factor of 1 is a plan of its own (no emulation to apply).
pub fn plan(pane_w: f64, pane_h: f64, ask: Ask, content_w: f64) -> Result<Plan, String> {
    if !(pane_w >= 1.0 && pane_h >= 1.0) {
        return Err("the pane has no size to fit (it may be hidden)".to_string());
    }
    let factor = match ask {
        Ask::Factor(f) => {
            if !f.is_finite() || !(MIN_FACTOR..=MAX_FACTOR).contains(&f) {
                return Err(format!(
                    "zoom factor {f} is out of range: use 0.25 to 2.0 (25% to 200%)"
                ));
            }
            f
        }
        Ask::Width(w) => {
            if !w.is_finite() || !(MIN_WIDTH..=MAX_WIDTH).contains(&w) {
                return Err(format!("zoom width {w} is out of range: use {MIN_WIDTH} to {MAX_WIDTH} CSS pixels"));
            }
            clamp_factor(pane_w / w)
        }
        Ask::Fit => {
            let wanted = if content_w.is_finite() { content_w.max(pane_w) } else { pane_w };
            clamp_factor(pane_w / wanted).min(1.0)
        }
    };
    // Four decimals: enough for a pixel at 4K, and a factor that prints as typed.
    let scale = (factor * 10_000.0).round() / 10_000.0;
    Ok(Plan {
        width: ((pane_w / scale).round() as u32).max(1),
        height: ((pane_h / scale).round() as u32).max(1),
        scale,
    })
}

/// Does a page of `now` CSS pixels still show the viewport `old` of the zoom just cleared?
fn still_the_old_view(old: (u32, u32), now: (f64, f64)) -> bool {
    now.0 == f64::from(old.0) && now.1 == f64::from(old.1)
}

/// "viewport 1280x720 CSS, shown at 47%".
pub fn describe(p: &Plan) -> String {
    format!("viewport {}x{} CSS, shown at {}%", p.width, p.height, percent(p.scale))
}

fn percent(scale: f64) -> String {
    let p = scale * 100.0;
    if (p - p.round()).abs() < 0.05 {
        format!("{}", p.round() as i64)
    } else {
        format!("{p:.1}")
    }
}

/// The pane's zoom as the host holds it.
#[derive(Debug, Clone)]
struct Entry {
    block_id: String,
    ask: Ask,
    plan: Plan,
    /// Bumped by every change, so a resize that waited out `RESIZE_SETTLE` can tell it was overtaken.
    generation: u64,
}

/// Zoomed panes by browser label (what a `CdpSession` addresses).
static ZOOMED: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// What the mouse events of a click on this browser are multiplied by: DevTools reads them in the scaled view.
pub fn input_scale(label: &str) -> f64 {
    ZOOMED.lock().get(label).map_or(1.0, |e| e.plan.scale)
}

// ── Applying it ─────────────────────────────────────────────────────────────

/// Measure the pane at its own size, then emulate `ask`. A factor of 1 leaves no emulation. Returns the plan,
/// and `None` when nothing is emulated.
async fn apply(cdp: &mut CdpSession, block_id: &str, ask: Ask) -> Result<Option<Plan>, String> {
    let label = cdp.label().to_string();
    // What the browser showed under the zoom being replaced: until it shows something else, the old
    // emulation is still in force and its size is not the pane's.
    let prior = ZOOMED.lock().get(&label).map(|e| (e.plan.width, e.plan.height));
    cdp.call("Emulation.clearDeviceMetricsOverride", json!({})).await.map_err(|e| e.to_string())?;
    let mut waited = 0;
    let size = loop {
        let size = cdp
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": "[innerWidth, innerHeight, document.documentElement ? document.documentElement.scrollWidth : innerWidth]",
                    "returnByValue": true,
                }),
            )
            .await
            .map_err(|e| e.to_string())?;
        let dims = |v: &Value| (0..2).map(|i| v.pointer("/result/value").and_then(|a| a.get(i)).and_then(|n| n.as_f64())).collect::<Option<Vec<_>>>();
        let still_zoomed = prior.is_some_and(|p| dims(&size).is_some_and(|d| still_the_old_view(p, (d[0], d[1]))));
        if !still_zoomed || waited >= CLEAR_SETTLE_POLLS {
            break size;
        }
        waited += 1;
        tokio::time::sleep(CLEAR_SETTLE_POLL).await;
    };
    let read = |i: usize| size.pointer("/result/value").and_then(|v| v.get(i)).and_then(|n| n.as_f64());
    let (pane_w, pane_h) = (read(0).ok_or("couldn't read the pane's size")?, read(1).ok_or("couldn't read the pane's size")?);
    let plan = plan(pane_w, pane_h, ask, read(2).unwrap_or(pane_w))?;
    if (plan.scale - 1.0).abs() < 1e-9 {
        ZOOMED.lock().remove(&label);
        return Ok(None);
    }
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": plan.width,
            "height": plan.height,
            // 0: the screen's own device scale, so devicePixelRatio is untouched.
            "deviceScaleFactor": 0,
            "mobile": false,
            "scale": plan.scale,
        }),
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut zoomed = ZOOMED.lock();
    let generation = zoomed.get(&label).map_or(0, |e| e.generation) + 1;
    zoomed.insert(label, Entry { block_id: block_id.to_string(), ask, plan, generation });
    Ok(Some(plan))
}

/// Take the emulation off a browser.
async fn clear(cdp: &mut CdpSession) {
    ZOOMED.lock().remove(cdp.label());
    let _ = cdp.call("Emulation.clearDeviceMetricsOverride", json!({})).await;
}

// ── Route ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ZoomReq {
    pub block_id: String,
    /// 0.25 to 2.0.
    #[serde(default)]
    pub factor: Option<f64>,
    /// A desktop viewport this wide, in CSS pixels.
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub fit: bool,
    #[serde(default)]
    pub reset: bool,
}

impl ZoomReq {
    /// What was asked: one of `factor`, `width`, `fit`; `None` for `reset`.
    fn ask(&self) -> Result<Option<Ask>, String> {
        let given = [self.factor.is_some(), self.width.is_some(), self.fit, self.reset];
        if given.iter().filter(|g| **g).count() != 1 {
            return Err("zoom takes exactly one of factor (0.25 to 2.0), width (CSS pixels), fit or reset".to_string());
        }
        Ok(match (self.factor, self.width) {
            (Some(f), _) => Some(Ask::Factor(f)),
            (_, Some(w)) => Some(Ask::Width(w)),
            _ if self.fit => Some(Ask::Fit),
            _ => None,
        })
    }
}

/// `POST /agentmux/browser/zoom` — zoom the pane's viewport, or put it back. Answers
/// `{ zoomed, factor, viewport_width, viewport_height, line }`.
pub async fn zoom_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ZoomReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let ask = match req.ask() {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "zoom").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = match ask {
        Some(ask) => apply(&mut cdp, &req.block_id, ask).await,
        None => {
            clear(&mut cdp).await;
            Ok(None)
        }
    };
    let _ = cdp.close().await;
    match result {
        Ok(Some(p)) => (
            StatusCode::OK,
            Json(ApiResponse::ok(json!({
                "zoomed": true,
                "factor": p.scale,
                "viewport_width": p.width,
                "viewport_height": p.height,
                "line": describe(&p),
            }))),
        ),
        Ok(None) => (
            StatusCode::OK,
            Json(ApiResponse::ok(json!({ "zoomed": false, "factor": 1.0, "line": "zoom is off (100%)" }))),
        ),
        Err(e) => fail(e),
    }
}

// ── When the pane changes under the zoom ────────────────────────────────────

/// The pane was resized: fit the zoom to its new size once it has held still.
pub fn on_pane_resized(state: &Arc<AppState>, block_id: &str) {
    if ZOOMED.lock().is_empty() {
        return;
    }
    let Some(label) = state.live_browser_pane_label(block_id) else { return };
    let Some(generation) = ZOOMED.lock().get_mut(&label).map(|e| {
        e.generation += 1;
        e.generation
    }) else {
        return;
    };
    let Some(rt) = crate::browser_pane::auth::runtime() else { return };
    let (state, block_id) = (state.clone(), block_id.to_string());
    rt.spawn(async move {
        tokio::time::sleep(RESIZE_SETTLE).await;
        let ask = match ZOOMED.lock().get(&label) {
            Some(e) if e.generation == generation => e.ask,
            _ => return,
        };
        let mut cdp = CdpSession::attach(&state, &label);
        if let Err(e) = apply(&mut cdp, &block_id, ask).await {
            tracing::warn!(%block_id, "[zoom] couldn't follow the pane's size: {e}");
        }
    });
}

/// srv's whole set of agent-owned panes arrived: a pane that is no longer the agent's, or was handed to the
/// person, gets its real size back.
pub fn clear_unowned(state: &Arc<AppState>, owned: &HashSet<String>, handoff: &[String]) {
    let gone: Vec<(String, String)> = ZOOMED
        .lock()
        .iter()
        .filter(|(_, e)| !owned.contains(&e.block_id) || handoff.contains(&e.block_id))
        .map(|(label, e)| (label.clone(), e.block_id.clone()))
        .collect();
    if gone.is_empty() {
        return;
    }
    let Some(rt) = crate::browser_pane::auth::runtime() else { return };
    for (label, block_id) in gone {
        let state = state.clone();
        rt.spawn(async move {
            tracing::info!(%block_id, "[zoom] the pane is no longer the agent's: zoom off");
            clear(&mut CdpSession::attach(&state, &label)).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pane_w: f64, pane_h: f64, ask: Ask, content_w: f64) -> Plan {
        plan(pane_w, pane_h, ask, content_w).expect("a plan")
    }

    #[test]
    fn a_factor_is_a_viewport_of_the_pane_over_the_factor() {
        let plan = p(640.0, 400.0, Ask::Factor(0.5), 0.0);
        assert_eq!((plan.width, plan.height, plan.scale), (1280, 800, 0.5));
        let plan = p(640.0, 400.0, Ask::Factor(2.0), 0.0);
        assert_eq!((plan.width, plan.height, plan.scale), (320, 200, 2.0));
    }

    #[test]
    fn a_factor_outside_the_range_is_refused_not_clamped() {
        for f in [0.1, 0.249, 2.01, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(plan(640.0, 400.0, Ask::Factor(f), 0.0).is_err(), "{f}");
        }
        assert!(plan(640.0, 400.0, Ask::Factor(0.25), 0.0).is_ok());
        assert!(plan(640.0, 400.0, Ask::Factor(2.0), 0.0).is_ok());
    }

    #[test]
    fn a_width_is_scaled_to_fit_the_pane() {
        let plan = p(640.0, 400.0, Ask::Width(1280.0), 0.0);
        assert_eq!((plan.width, plan.height, plan.scale), (1280, 800, 0.5));
        // A width the pane already exceeds zooms in, to the cap.
        let plan = p(1600.0, 900.0, Ask::Width(800.0), 0.0);
        assert_eq!(plan.scale, 2.0);
        // A width so large the factor would pass the floor stops at the floor.
        let plan = p(400.0, 300.0, Ask::Width(3840.0), 0.0);
        assert_eq!(plan.scale, MIN_FACTOR);
        assert_eq!(plan.width, 1600);
    }

    #[test]
    fn a_width_outside_the_range_is_refused() {
        assert!(plan(640.0, 400.0, Ask::Width(100.0), 0.0).is_err());
        assert!(plan(640.0, 400.0, Ask::Width(5000.0), 0.0).is_err());
        assert!(plan(640.0, 400.0, Ask::Width(f64::NAN), 0.0).is_err());
    }

    #[test]
    fn fit_zooms_out_to_the_pages_width_and_never_in() {
        let plan = p(600.0, 400.0, Ask::Fit, 1200.0);
        assert_eq!(plan.scale, 0.5);
        assert_eq!(plan.width, 1200);
        // A page that already fits stays at 100%.
        assert_eq!(p(600.0, 400.0, Ask::Fit, 500.0).scale, 1.0);
        // Never below the floor.
        assert_eq!(p(600.0, 400.0, Ask::Fit, 9000.0).scale, MIN_FACTOR);
        // A width that could not be read is the pane's own.
        assert_eq!(p(600.0, 400.0, Ask::Fit, f64::NAN).scale, 1.0);
    }

    #[test]
    fn a_page_still_at_the_cleared_zoom_size_has_not_shown_the_pane_yet() {
        assert!(still_the_old_view((280, 200), (280.0, 200.0)));
        assert!(!still_the_old_view((280, 200), (560.0, 400.0)));
        // A pane that changed in one direction only is the pane's own size.
        assert!(!still_the_old_view((280, 200), (280.0, 400.0)));
    }

    #[test]
    fn a_pane_with_no_size_has_no_plan() {
        assert!(plan(0.0, 400.0, Ask::Factor(0.5), 0.0).is_err());
        assert!(plan(640.0, f64::NAN, Ask::Factor(0.5), 0.0).is_err());
    }

    #[test]
    fn the_answer_reads_as_typed() {
        assert_eq!(describe(&p(640.0, 360.0, Ask::Factor(0.5), 0.0)), "viewport 1280x720 CSS, shown at 50%");
        assert_eq!(percent(0.4701), "47");
        assert_eq!(percent(0.4712), "47.1");
        assert_eq!(percent(2.0), "200");
    }

    #[test]
    fn exactly_one_thing_is_asked() {
        let req = |factor, width, fit, reset| ZoomReq { block_id: "b".into(), factor, width, fit, reset };
        assert_eq!(req(Some(0.5), None, false, false).ask(), Ok(Some(Ask::Factor(0.5))));
        assert_eq!(req(None, Some(1280.0), false, false).ask(), Ok(Some(Ask::Width(1280.0))));
        assert_eq!(req(None, None, true, false).ask(), Ok(Some(Ask::Fit)));
        assert_eq!(req(None, None, false, true).ask(), Ok(None));
        assert!(req(None, None, false, false).ask().is_err());
        assert!(req(Some(0.5), None, true, false).ask().is_err());
    }

    #[test]
    fn the_mouse_is_scaled_only_where_a_zoom_is_held() {
        let plan = p(640.0, 400.0, Ask::Factor(0.5), 0.0);
        ZOOMED.lock().insert(
            "pane-zoom-test".to_string(),
            Entry { block_id: "b".into(), ask: Ask::Factor(0.5), plan, generation: 1 },
        );
        assert_eq!(input_scale("pane-zoom-test"), 0.5);
        assert_eq!(input_scale("another-pane"), 1.0);
        ZOOMED.lock().remove("pane-zoom-test");
        assert_eq!(input_scale("pane-zoom-test"), 1.0);
    }
}
