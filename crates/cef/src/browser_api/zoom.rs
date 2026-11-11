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
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Is what was asked for in range? Checked before anything is changed, so a refused request leaves a zoom as it was.
pub fn check(ask: Ask) -> Result<(), String> {
    match ask {
        Ask::Factor(f) if !f.is_finite() || !(MIN_FACTOR..=MAX_FACTOR).contains(&f) => {
            Err(format!("zoom factor {f} is out of range: use 0.25 to 2.0 (25% to 200%)"))
        }
        Ask::Width(w) if !w.is_finite() || !(MIN_WIDTH..=MAX_WIDTH).contains(&w) => {
            Err(format!("zoom width {w} is out of range: use {MIN_WIDTH} to {MAX_WIDTH} CSS pixels"))
        }
        _ => Ok(()),
    }
}

/// The viewport for a pane of `pane_w` x `pane_h` CSS pixels. `content_w` is the page's width at the pane's
/// own size, which `Fit` needs. A factor of 1 is a plan of its own (no emulation to apply).
pub fn plan(pane_w: f64, pane_h: f64, ask: Ask, content_w: f64) -> Result<Plan, String> {
    if !(pane_w >= 1.0 && pane_h >= 1.0) {
        return Err("the pane has no size to fit (it may be hidden)".to_string());
    }
    check(ask)?;
    let factor = match ask {
        Ask::Factor(f) => f,
        Ask::Width(w) => clamp_factor(pane_w / w),
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
    /// From `next_generation`, so a resize that waited out `RESIZE_SETTLE` can tell it was overtaken.
    generation: u64,
}

fn next_generation() -> u64 {
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed) + 1
}

/// Zoomed panes by browser label (what a `CdpSession` addresses).
static ZOOMED: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// What the mouse events of a click on this browser are multiplied by: DevTools reads them in the scaled view.
pub fn input_scale(label: &str) -> f64 {
    ZOOMED.lock().get(label).map_or(1.0, |e| e.plan.scale)
}

/// A point in the page's CSS pixels as the mouse is sent it on this browser: DevTools reads mouse events in the
/// scaled view.
pub fn scale_point(label: &str, x: f64, y: f64) -> (f64, f64) {
    let scale = input_scale(label);
    (x * scale, y * scale)
}

// ── Applying it ─────────────────────────────────────────────────────────────

/// Measure the pane at its own size, then emulate `ask`. A factor of 1 leaves no emulation. Returns the plan,
/// and `None` when nothing is emulated.
async fn apply_once(cdp: &mut CdpSession, block_id: &str, ask: Ask) -> Result<Option<Plan>, String> {
    check(ask)?;
    let label = cdp.label().to_string();
    // What the browser showed under the zoom being replaced: until it shows something else, the old
    // emulation is still in force and its size is not the pane's.
    let prior = ZOOMED.lock().get(&label).map(|e| (e.plan.width, e.plan.height));
    cdp.call("Emulation.clearDeviceMetricsOverride", json!({})).await.map_err(|e| e.to_string())?;
    // The view is no longer scaled: a click now must not be (the wait below can take a while).
    ZOOMED.lock().remove(&label);
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
    ZOOMED.lock().insert(label, Entry { block_id: block_id.to_string(), ask, plan, generation: next_generation() });
    Ok(Some(plan))
}

/// A pane being fitted now.
#[derive(Debug, Clone, Copy)]
struct Applying {
    /// The pane was resized meanwhile, so the fit is done again.
    resized: bool,
    /// The zoom it had when the fit began (the entry is out of `ZOOMED` while the fit runs), for a status asked
    /// meanwhile.
    prior: Option<Plan>,
}

static APPLYING: LazyLock<Mutex<HashMap<String, Applying>>> = LazyLock::new(Default::default);
/// The last size (in DIP) each pane was given: a move that keeps it needs no new fit.
static LAST_SIZE: LazyLock<Mutex<HashMap<String, (i32, i32)>>> = LazyLock::new(Default::default);
/// Panes whose size is kept before the map starts over.
const SIZES_KEPT: usize = 512;
/// Passes at most over one pane when it is resized while being fitted.
const FIT_PASSES: u32 = 3;

/// Measure the pane and emulate `ask` on it; once more if the pane was resized meanwhile. A factor of 1
/// leaves no emulation. Returns the plan, and `None` when nothing is emulated.
async fn apply(cdp: &mut CdpSession, block_id: &str, ask: Ask) -> Result<Option<Plan>, String> {
    let label = cdp.label().to_string();
    let mut result = Ok(None);
    let prior = ZOOMED.lock().get(&label).map(|e| e.plan);
    for _ in 0..FIT_PASSES {
        APPLYING.lock().insert(label.clone(), Applying { resized: false, prior });
        result = apply_once(cdp, block_id, ask).await;
        if result.is_err() || !APPLYING.lock().get(&label).is_some_and(|a| a.resized) {
            break;
        }
    }
    APPLYING.lock().remove(&label);
    result
}

/// Is `size` a new size for `block_id`? Remembers it.
fn size_changed(sizes: &mut HashMap<String, (i32, i32)>, block_id: &str, size: (i32, i32)) -> bool {
    if sizes.len() >= SIZES_KEPT && !sizes.contains_key(block_id) {
        sizes.clear();
    }
    sizes.insert(block_id.to_string(), size) != Some(size)
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
    /// Only say how the pane is zoomed now (srv asks, to keep what it tells the agent true).
    #[serde(default)]
    pub status: bool,
}

/// What a zoom call does.
#[derive(Debug, PartialEq)]
enum Op {
    Set(Ask),
    Reset,
    Status,
}

impl ZoomReq {
    /// What was asked: exactly one of `factor`, `width`, `fit`, `reset` and `status`.
    fn op(&self) -> Result<Op, String> {
        let given = [self.factor.is_some(), self.width.is_some(), self.fit, self.reset, self.status];
        if given.iter().filter(|g| **g).count() != 1 {
            return Err("zoom takes exactly one of factor (0.25 to 2.0), width (CSS pixels), fit or reset".to_string());
        }
        Ok(match (self.factor, self.width) {
            (Some(f), _) => Op::Set(Ask::Factor(f)),
            (_, Some(w)) => Op::Set(Ask::Width(w)),
            _ if self.fit => Op::Set(Ask::Fit),
            _ if self.reset => Op::Reset,
            _ => Op::Status,
        })
    }
}

/// A zoom as the host tells srv.
fn zoomed_json(plan: &Plan) -> Value {
    json!({
        "zoomed": true,
        "factor": plan.scale,
        "viewport_width": plan.width,
        "viewport_height": plan.height,
        "line": describe(plan),
    })
}

/// How `block_id` is zoomed now, whatever the host did on its own since: a zoom that was dropped (a refused
/// request, a pane that could not be fitted) or lost with its browser (a pane that woke from hibernation is a
/// new browser) is not one. `live` is the label of the pane's browser now.
fn status(block_id: &str, live: Option<&str>) -> Value {
    // Being fitted right now (the zoom is out of the record until the fit is done): it is as it was before.
    let refitting = live.and_then(|l| APPLYING.lock().get(l).and_then(|a| a.prior));
    let mut zoomed = ZOOMED.lock();
    let held = zoomed.iter().find(|(_, e)| e.block_id == block_id).map(|(l, e)| (l.clone(), e.plan));
    if let (None, Some(plan)) = (&held, refitting) {
        return zoomed_json(&plan);
    }
    match held {
        Some((label, plan)) if live == Some(label.as_str()) => zoomed_json(&plan),
        stale => {
            if let Some((label, _)) = stale {
                zoomed.remove(&label);
            }
            json!({ "zoomed": false, "factor": 1.0, "line": "zoom is off (100%)" })
        }
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
    let op = match req.op() {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    if op == Op::Status {
        let live = state.live_browser_pane_label(&req.block_id);
        return (StatusCode::OK, Json(ApiResponse::ok(status(&req.block_id, live.as_deref()))));
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "zoom").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = match op {
        Op::Set(ask) => apply(&mut cdp, &req.block_id, ask).await,
        _ => {
            clear(&mut cdp).await;
            Ok(None)
        }
    };
    let _ = cdp.close().await;
    match result {
        Ok(Some(p)) => (StatusCode::OK, Json(ApiResponse::ok(zoomed_json(&p)))),
        Ok(None) => (
            StatusCode::OK,
            Json(ApiResponse::ok(json!({ "zoomed": false, "factor": 1.0, "line": "zoom is off (100%)" }))),
        ),
        Err(e) => fail(e),
    }
}

// ── When the pane changes under the zoom ────────────────────────────────────

/// The pane was given a rect of `size` DIP: if that is a new size, fit the zoom to it once it has held still.
pub fn on_pane_resized(state: &Arc<AppState>, block_id: &str, size: (i32, i32)) {
    // A move that keeps the size is not a resize, and a refit would reflow the page under the agent.
    if !size_changed(&mut LAST_SIZE.lock(), block_id, size) {
        return;
    }
    if ZOOMED.lock().is_empty() && APPLYING.lock().is_empty() {
        return;
    }
    let Some(label) = state.live_browser_pane_label(block_id) else { return };
    // Being fitted right now: that pass looks again when it is done.
    if let Some(applying) = APPLYING.lock().get_mut(&label) {
        applying.resized = true;
        return;
    }
    let generation = next_generation();
    let Some(()) = ZOOMED.lock().get_mut(&label).map(|e| e.generation = generation) else {
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
        let req = |factor, width, fit, reset, status| ZoomReq { block_id: "b".into(), factor, width, fit, reset, status };
        assert_eq!(req(Some(0.5), None, false, false, false).op(), Ok(Op::Set(Ask::Factor(0.5))));
        assert_eq!(req(None, Some(1280.0), false, false, false).op(), Ok(Op::Set(Ask::Width(1280.0))));
        assert_eq!(req(None, None, true, false, false).op(), Ok(Op::Set(Ask::Fit)));
        assert_eq!(req(None, None, false, true, false).op(), Ok(Op::Reset));
        assert_eq!(req(None, None, false, false, true).op(), Ok(Op::Status));
        assert!(req(None, None, false, false, false).op().is_err());
        assert!(req(Some(0.5), None, true, false, false).op().is_err());
    }

    #[test]
    fn a_request_out_of_range_is_refused_before_anything_changes() {
        assert!(check(Ask::Factor(3.0)).is_err());
        assert!(check(Ask::Width(100.0)).is_err());
        assert!(check(Ask::Factor(0.5)).is_ok());
        assert!(check(Ask::Fit).is_ok());
    }

    #[test]
    fn the_status_is_what_the_host_holds_for_a_pane_that_is_still_the_same_browser() {
        let plan = p(640.0, 400.0, Ask::Factor(0.5), 0.0);
        ZOOMED.lock().insert(
            "status-test-label".to_string(),
            Entry { block_id: "status-test-block".into(), ask: Ask::Factor(0.5), plan, generation: next_generation() },
        );
        let s = status("status-test-block", Some("status-test-label"));
        assert_eq!(s["zoomed"], true);
        assert_eq!(s["line"], "viewport 1280x800 CSS, shown at 50%");
        // The pane woke as a new browser: the zoom went with the old one.
        let s = status("status-test-block", Some("another-label"));
        assert_eq!(s["zoomed"], false);
        assert_eq!(input_scale("status-test-label"), 1.0, "and the stale entry is gone");
        // A pane that was never zoomed.
        assert_eq!(status("never-zoomed", Some("x"))["zoomed"], false);
    }

    #[test]
    fn a_point_is_moved_into_the_scaled_view_only_where_a_zoom_is_held() {
        let plan = p(640.0, 400.0, Ask::Factor(0.5), 0.0);
        ZOOMED.lock().insert(
            "pane-zoom-point-test".to_string(),
            Entry { block_id: "b".into(), ask: Ask::Factor(0.5), plan, generation: next_generation() },
        );
        assert_eq!(scale_point("pane-zoom-point-test", 200.0, 100.0), (100.0, 50.0));
        assert_eq!(scale_point("another-pane", 200.0, 100.0), (200.0, 100.0));
        ZOOMED.lock().remove("pane-zoom-point-test");
    }

    #[test]
    fn a_status_asked_while_the_pane_is_being_fitted_says_the_zoom_it_had() {
        let plan = p(640.0, 400.0, Ask::Factor(0.5), 0.0);
        APPLYING.lock().insert("refit-test-label".to_string(), Applying { resized: false, prior: Some(plan) });
        let s = status("refit-test-block", Some("refit-test-label"));
        assert_eq!(s["zoomed"], true);
        assert_eq!(s["line"], "viewport 1280x800 CSS, shown at 50%");
        // A first fit has no zoom to report yet.
        APPLYING.lock().insert("refit-test-label".to_string(), Applying { resized: false, prior: None });
        assert_eq!(status("refit-test-block", Some("refit-test-label"))["zoomed"], false);
        APPLYING.lock().remove("refit-test-label");
    }

    #[test]
    fn only_a_new_size_is_a_resize() {
        let mut sizes = HashMap::new();
        assert!(size_changed(&mut sizes, "b", (700, 500)), "the first size is new");
        assert!(!size_changed(&mut sizes, "b", (700, 500)), "a move that keeps it is not");
        assert!(size_changed(&mut sizes, "b", (700, 400)));
        assert!(size_changed(&mut sizes, "other", (700, 400)), "sizes are per pane");
    }

    #[test]
    fn the_sizes_kept_are_bounded() {
        let mut sizes = HashMap::new();
        for i in 0..(SIZES_KEPT + 10) {
            size_changed(&mut sizes, &format!("b{i}"), (1, 1));
        }
        assert!(sizes.len() <= SIZES_KEPT);
    }

    #[test]
    fn a_later_change_has_a_later_generation() {
        let (a, b) = (next_generation(), next_generation());
        assert!(b > a);
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
