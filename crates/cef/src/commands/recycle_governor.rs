// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The recycle governor: reloads an app window's page by itself when the window
//! has been idle for a while and its renderer is over budget
//! (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §4).
//!
//! Every minute it measures each app window's renderer and picks at most one
//! window that is over budget (§4.2) and idle (§4.3). Before asking, it forces
//! a garbage collection in that window and measures again: closing many panes
//! leaves garbage a collection frees, and a reload would be wasted on it. Then
//! it sends the window `recycle:request`; the page applies the checks only it
//! can make (input in the window, a waiting agent, a modal, a drag, and every
//! recycle blocker) and either recycles itself or declines with
//! `recycle_declined`, which holds the window back for the idle time.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::memory_pressure::PressureLevel;
use crate::state::AppState;

/// How often the governor looks.
const TICK: Duration = Duration::from_secs(60);

/// The thresholds of spec §4.2–§4.5.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Never recycle a renderer smaller than this.
    pub min_mb: u64,
    /// ... nor one less than this many times its baseline.
    pub growth: f64,
    /// How long a window must have been out of use.
    pub idle: Duration,
    /// The idle time under `Warn` commit pressure.
    pub idle_under_pressure: Duration,
    /// At most one recycle per window in this long.
    pub per_window: Duration,
    /// Never two windows within this long.
    pub between: Duration,
    /// A window's baseline is its size this long after its last load.
    pub baseline_after: Duration,
}

impl Limits {
    pub const SPEC: Limits = Limits {
        min_mb: 600,
        growth: 2.0,
        idle: Duration::from_secs(10 * 60),
        idle_under_pressure: Duration::from_secs(2 * 60),
        per_window: Duration::from_secs(60 * 60),
        between: Duration::from_secs(60),
        baseline_after: Duration::from_secs(60),
    };

    /// The spec's limits, with test-only overrides for the memory lab
    /// (`AGENTMUX_RECYCLE_MIN_MB`, `AGENTMUX_RECYCLE_IDLE_SECS`); nothing sets
    /// them in a normal launch.
    fn from_env() -> Limits {
        let var = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
        let mut limits = Limits::SPEC;
        if let Some(mb) = var("AGENTMUX_RECYCLE_MIN_MB") {
            limits.min_mb = mb;
            limits.growth = 1.0;
        }
        if let Some(secs) = var("AGENTMUX_RECYCLE_IDLE_SECS") {
            limits.idle = Duration::from_secs(secs);
            limits.idle_under_pressure = Duration::from_secs(secs);
            limits.per_window = Duration::from_secs(secs.max(60));
        }
        limits
    }
}

/// One app window, as measured this tick.
#[derive(Debug, Clone)]
pub(crate) struct Sample {
    pub label: String,
    /// Its renderer's private memory; `None` when it can't be measured on its
    /// own (a renderer shared with another window, or Linux).
    pub mb: Option<u64>,
    pub minimized: bool,
}

/// What the governor remembers about a window between ticks.
#[derive(Debug, Clone)]
pub(crate) struct Track {
    pub loaded_at: Instant,
    pub baseline_mb: Option<u64>,
    /// The last tick the window was the active one (or when it was first seen).
    pub last_active: Instant,
    pub last_request: Option<Instant>,
    pub declined_until: Option<Instant>,
}

impl Track {
    fn new(now: Instant) -> Track {
        Track { loaded_at: now, baseline_mb: None, last_active: now, last_request: None, declined_until: None }
    }
}

/// How long a window must have been out of use (§4.3, §4.4).
fn idle_time(limits: &Limits, pressure: PressureLevel) -> Duration {
    if pressure == PressureLevel::Normal { limits.idle } else { limits.idle_under_pressure }
}

/// Whether `sample` is over budget (§4.2, §4.4).
fn over_budget(sample: &Sample, track: &Track, limits: &Limits, pressure: PressureLevel) -> bool {
    let Some(mb) = sample.mb else { return false };
    if mb < limits.min_mb {
        return false;
    }
    if pressure == PressureLevel::Critical {
        return true;
    }
    track.baseline_mb.is_some_and(|base| mb as f64 >= base as f64 * limits.growth)
}

/// The window to recycle now, if any: over budget, out of use for the idle
/// time, past its own and the global rate limits, not holding back after a
/// decline. The largest one when several qualify. Pure, for the tests.
pub(crate) fn decide(
    now: Instant,
    samples: &[Sample],
    tracks: &HashMap<String, Track>,
    limits: &Limits,
    pressure: PressureLevel,
    last_any_request: Option<Instant>,
) -> Option<String> {
    if last_any_request.is_some_and(|at| now.duration_since(at) < limits.between) {
        return None;
    }
    let idle_for = idle_time(limits, pressure);
    samples
        .iter()
        .filter(|s| {
            let Some(t) = tracks.get(&s.label) else { return false };
            over_budget(s, t, limits, pressure)
                && (s.minimized || now.duration_since(t.last_active) >= idle_for)
                && t.last_request.is_none_or(|at| now.duration_since(at) >= limits.per_window)
                && t.declined_until.is_none_or(|until| now >= until)
        })
        .max_by_key(|s| s.mb)
        .map(|s| s.label.clone())
}

/// Bring the tracks up to this tick: new windows start now, gone ones are
/// forgotten, the active window is marked, and a baseline is taken once a
/// window has been loaded for `baseline_after`.
pub(crate) fn update_tracks(
    now: Instant,
    samples: &[Sample],
    active: &dyn Fn(&str) -> bool,
    tracks: &mut HashMap<String, Track>,
    limits: &Limits,
) {
    tracks.retain(|label, _| samples.iter().any(|s| &s.label == label));
    for s in samples {
        let t = tracks.entry(s.label.clone()).or_insert_with(|| Track::new(now));
        if active(&s.label) {
            t.last_active = now;
        }
        if t.baseline_mb.is_none() && now.duration_since(t.loaded_at) >= limits.baseline_after {
            t.baseline_mb = s.mb;
        }
    }
}

struct Governor {
    tracks: HashMap<String, Track>,
    last_any_request: Option<Instant>,
}

static GOVERNOR: LazyLock<Mutex<Governor>> =
    LazyLock::new(|| Mutex::new(Governor { tracks: HashMap::new(), last_any_request: None }));

/// The window's page (re)loaded: its old baseline no longer applies.
pub(crate) fn note_window_loaded(label: &str) {
    let mut g = GOVERNOR.lock().unwrap();
    let now = Instant::now();
    let t = g.tracks.entry(label.to_string()).or_insert_with(|| Track::new(now));
    t.loaded_at = now;
    t.baseline_mb = None;
}

/// `recycle_declined { label, reason }`: the page wouldn't recycle (input, a
/// waiting agent, a blocker). Leave the window alone for the idle time (the
/// same one the request used, shorter under memory pressure).
pub fn recycle_declined(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).ok_or("recycle_declined: label is required")?;
    let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let limits = Limits::from_env();
    if let Some(t) = GOVERNOR.lock().unwrap().tracks.get_mut(label) {
        t.declined_until = Some(Instant::now() + idle_time(&limits, crate::memory_pressure::current_level()));
    }
    tracing::info!(target: "recycle", label, reason, "[recycle] {label} declined: {reason}");
    Ok(serde_json::Value::Null)
}

/// The app windows the governor looks after: user windows, not pool windows,
/// floating panes, browser panes or approval windows (§4).
fn app_windows(state: &AppState) -> Vec<String> {
    state
        .user_window_labels()
        .into_iter()
        .filter(|l| state.is_live_top_level_browser(l) && !state.is_approval_window(l))
        .collect()
}

#[cfg(target_os = "windows")]
fn minimized(state: &AppState, label: &str) -> bool {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::IsIconic;
    // SAFETY: IsIconic tolerates a stale or invalid handle (returns FALSE).
    state.window_hwnd(label).is_some_and(|h| unsafe { IsIconic(h as HWND) } != 0)
}

#[cfg(not(target_os = "windows"))]
fn minimized(_state: &AppState, _label: &str) -> bool {
    false
}

/// Force a major garbage collection in `label`'s page and wait for it.
fn collect_garbage(state: &Arc<AppState>, label: &str) {
    let Some(rt) = crate::browser_pane::auth::runtime() else { return };
    let state = Arc::clone(state);
    let label = label.to_string();
    rt.block_on(async move {
        let mut cdp = crate::browser_api::cdp::CdpSession::attach(&state, &label);
        if let Err(e) = cdp.call("HeapProfiler.collectGarbage", serde_json::json!({})).await {
            tracing::warn!(target: "recycle", label, error = %e, "[recycle] couldn't collect garbage first");
        }
    });
}

fn tick(state: &Arc<AppState>, limits: &Limits) {
    let now = Instant::now();
    let samples: Vec<Sample> = app_windows(state)
        .into_iter()
        .map(|label| Sample { mb: super::recycle::renderer_mb(state, &label), minimized: minimized(state, &label), label })
        .collect();
    let pressure = crate::memory_pressure::current_level();
    let pick = {
        let mut g = GOVERNOR.lock().unwrap();
        update_tracks(now, &samples, &crate::input_owner::registry::is_active, &mut g.tracks, limits);
        decide(now, &samples, &g.tracks, limits, pressure, g.last_any_request)
    };
    let Some(label) = pick else { return };
    if crate::floating_pane::host_dialog_open() {
        return;
    }
    // Read live, so turning it off takes effect at the next tick.
    if !crate::app::window_settings::read_window_bool_setting("window:autorecycle", true) {
        return;
    }
    let before = samples.iter().find(|s| s.label == label).and_then(|s| s.mb);
    collect_garbage(state, &label);
    std::thread::sleep(Duration::from_secs(2));
    // The window may have come into use meanwhile (activated from the taskbar
    // makes no input in the page), or a native dialog opened: look again.
    if crate::input_owner::registry::is_active(&label) || crate::floating_pane::host_dialog_open() {
        if let Some(t) = GOVERNOR.lock().unwrap().tracks.get_mut(&label) {
            t.last_active = Instant::now();
        }
        return;
    }
    let after_gc = super::recycle::renderer_mb(state, &label);
    let still_over = {
        let g = GOVERNOR.lock().unwrap();
        let sample = Sample { label: label.clone(), mb: after_gc, minimized: false };
        g.tracks.get(&label).is_some_and(|t| over_budget(&sample, t, limits, pressure))
    };
    let mut g = GOVERNOR.lock().unwrap();
    if !still_over {
        // Nothing was reloaded, so no limit is used up; only this window rests
        // for the idle time, so it isn't collected every minute.
        if let Some(t) = g.tracks.get_mut(&label) {
            t.declined_until = Some(now + idle_time(limits, pressure));
        }
        drop(g);
        tracing::info!(target: "recycle", label, before_mb = before, after_gc_mb = after_gc, "[recycle] {label} back under budget after a garbage collection");
        return;
    }
    g.last_any_request = Some(now);
    if let Some(t) = g.tracks.get_mut(&label) {
        t.last_request = Some(now);
    }
    drop(g);
    tracing::info!(target: "recycle", label, before_mb = before, after_gc_mb = after_gc, "[recycle] {label} idle and over budget: asking it to recycle");
    // The page checks its own input against the same idle time (pressure and
    // the lab's override shorten it).
    let idle_ms = idle_time(limits, pressure).as_millis() as u64;
    crate::events::emit_event_to_window(state, &label, "recycle:request", &serde_json::json!({ "reason": "idle", "idle_ms": idle_ms }));
}

/// Start the governor's thread. Runs for the life of the process.
pub fn start(state: Arc<AppState>) {
    let limits = Limits::from_env();
    let spawned = std::thread::Builder::new().name("recycle-governor".into()).spawn(move || loop {
        std::thread::sleep(TICK);
        tick(&state, &limits);
    });
    if let Err(e) = spawned {
        tracing::warn!(target: "recycle", error = %e, "[recycle] the governor didn't start");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn sample(label: &str, mb: u64) -> Sample {
        Sample { label: label.into(), mb: Some(mb), minimized: false }
    }

    /// A window loaded long ago with a 300 MB baseline, inactive for `idle`.
    fn track(now: Instant, idle: Duration) -> Track {
        Track {
            loaded_at: now - 3 * 60 * MIN,
            baseline_mb: Some(300),
            last_active: now - idle,
            last_request: None,
            declined_until: None,
        }
    }

    fn one(label: &str, t: Track) -> HashMap<String, Track> {
        HashMap::from([(label.to_string(), t)])
    }

    fn now() -> Instant {
        Instant::now() + 24 * 60 * MIN
    }

    #[test]
    fn an_idle_window_over_budget_is_recycled() {
        let n = now();
        let pick = decide(n, &[sample("main", 900)], &one("main", track(n, 11 * MIN)), &Limits::SPEC, PressureLevel::Normal, None);
        assert_eq!(pick.as_deref(), Some("main"));
    }

    #[test]
    fn under_600_mb_or_under_twice_the_baseline_is_not() {
        let n = now();
        let tracks = one("main", track(n, 11 * MIN));
        assert_eq!(decide(n, &[sample("main", 590)], &tracks, &Limits::SPEC, PressureLevel::Normal, None), None, "under 600");
        let small_base = one("main", Track { baseline_mb: Some(400), ..track(n, 11 * MIN) });
        assert_eq!(decide(n, &[sample("main", 700)], &small_base, &Limits::SPEC, PressureLevel::Normal, None), None, "under 2x");
        let no_base = one("main", Track { baseline_mb: None, ..track(n, 11 * MIN) });
        assert_eq!(decide(n, &[sample("main", 900)], &no_base, &Limits::SPEC, PressureLevel::Normal, None), None, "no baseline yet");
    }

    #[test]
    fn a_window_in_use_in_the_last_10_minutes_is_not_unless_minimized() {
        let n = now();
        let tracks = one("main", track(n, 9 * MIN));
        assert_eq!(decide(n, &[sample("main", 900)], &tracks, &Limits::SPEC, PressureLevel::Normal, None), None);
        let min = Sample { minimized: true, ..sample("main", 900) };
        assert_eq!(decide(n, &[min], &tracks, &Limits::SPEC, PressureLevel::Normal, None).as_deref(), Some("main"));
    }

    #[test]
    fn rate_limits_and_a_decline_hold_it_back() {
        let n = now();
        let recent = one("main", Track { last_request: Some(n - 30 * MIN), ..track(n, 11 * MIN) });
        assert_eq!(decide(n, &[sample("main", 900)], &recent, &Limits::SPEC, PressureLevel::Normal, None), None, "once an hour");
        let tracks = one("main", track(n, 11 * MIN));
        assert_eq!(decide(n, &[sample("main", 900)], &tracks, &Limits::SPEC, PressureLevel::Normal, Some(n - Duration::from_secs(30))), None, "another window 30 s ago");
        let declined = one("main", Track { declined_until: Some(n + MIN), ..track(n, 11 * MIN) });
        assert_eq!(decide(n, &[sample("main", 900)], &declined, &Limits::SPEC, PressureLevel::Normal, None), None, "declined");
    }

    #[test]
    fn an_unmeasured_renderer_is_never_picked() {
        let n = now();
        let s = Sample { mb: None, ..sample("main", 0) };
        assert_eq!(decide(n, &[s], &one("main", track(n, 11 * MIN)), &Limits::SPEC, PressureLevel::Normal, None), None);
    }

    #[test]
    fn pressure_shortens_the_idle_time_and_critical_drops_the_growth_rule() {
        let n = now();
        let tracks = one("main", track(n, 3 * MIN));
        assert_eq!(decide(n, &[sample("main", 900)], &tracks, &Limits::SPEC, PressureLevel::Normal, None), None);
        assert_eq!(decide(n, &[sample("main", 900)], &tracks, &Limits::SPEC, PressureLevel::Warn, None).as_deref(), Some("main"));
        let big_base = one("main", Track { baseline_mb: Some(500), ..track(n, 3 * MIN) });
        assert_eq!(decide(n, &[sample("main", 700)], &big_base, &Limits::SPEC, PressureLevel::Warn, None), None, "Warn keeps 2x");
        assert_eq!(decide(n, &[sample("main", 700)], &big_base, &Limits::SPEC, PressureLevel::Critical, None).as_deref(), Some("main"), "Critical drops it");
    }

    #[test]
    fn of_several_the_largest_goes_first() {
        let n = now();
        let tracks = HashMap::from([("main".to_string(), track(n, 11 * MIN)), ("window-2".to_string(), track(n, 11 * MIN))]);
        let pick = decide(n, &[sample("main", 700), sample("window-2", 1200)], &tracks, &Limits::SPEC, PressureLevel::Normal, None);
        assert_eq!(pick.as_deref(), Some("window-2"));
    }

    #[test]
    fn tracks_follow_the_windows_and_take_a_baseline_after_a_minute() {
        let start = Instant::now();
        let mut tracks = HashMap::new();
        let inactive = |_: &str| false;
        update_tracks(start, &[sample("main", 250)], &inactive, &mut tracks, &Limits::SPEC);
        assert_eq!(tracks["main"].baseline_mb, None, "not before a minute");
        update_tracks(start + MIN, &[sample("main", 280)], &inactive, &mut tracks, &Limits::SPEC);
        assert_eq!(tracks["main"].baseline_mb, Some(280));
        update_tracks(start + 2 * MIN, &[sample("main", 900)], &inactive, &mut tracks, &Limits::SPEC);
        assert_eq!(tracks["main"].baseline_mb, Some(280), "kept until the next load");
        let active = |l: &str| l == "main";
        update_tracks(start + 3 * MIN, &[sample("main", 900)], &active, &mut tracks, &Limits::SPEC);
        assert_eq!(tracks["main"].last_active, start + 3 * MIN);
        update_tracks(start + 4 * MIN, &[], &inactive, &mut tracks, &Limits::SPEC);
        assert!(tracks.is_empty(), "a closed window is forgotten");
    }
}
