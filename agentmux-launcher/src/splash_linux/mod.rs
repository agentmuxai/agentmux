// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session-aware Linux startup splash.
//!
//! The launcher is up in milliseconds; `agentmux-cef` *is* the multi-second cold
//! start (CEF init + GPU channel + page load). This paints a pulsing brain logo
//! over a dark backdrop in that gap — the Linux analogue of `splash.rs` (Win32)
//! and `splash_mac.rs` (Cocoa).
//!
//! The backend is chosen for **centering fidelity**, not to mirror the host's
//! ozone choice: the X11 backend draws an `override_redirect` window it positions
//! itself (centered on the primary monitor) and XWayland honors that, so it
//! centers under both X11 and native-Wayland sessions. A native-Wayland
//! `xdg_toplevel`, by contrast, cannot self-position — Mutter drops small
//! toplevels at the top-left, not centered. XWayland is present in nearly every
//! Wayland session, so we prefer the X11 backend whenever an X server is reachable
//! and fall back to the Wayland backend only when there is no X display at all.
//!
//! Dismiss protocol is the macOS one: `spawn()` sets `AGENTMUX_SPLASH_READY_FILE`
//! (the host inherits it and writes the file on first paint); the splash thread
//! polls for that file and tears down, with a safety timeout for a host that
//! crashes before painting.
//!
//! Spec: docs/specs/SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md

mod wayland;
mod x11;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::startup_events::{StartupEvent, StartupStatus};

// ── Shared look (matches splash_mac.rs / splash.rs) ─────────────────────────
/// Opaque dark backdrop RGB(26, 26, 31) = #1A1A1F.
pub(crate) const BG_R: u8 = 0x1A;
pub(crate) const BG_G: u8 = 0x1A;
pub(crate) const BG_B: u8 = 0x1F;
/// Backdrop padding around the brain, in px.
pub(crate) const PADDING: i32 = 28;
/// ~60 fps animation tick.
pub(crate) const FRAME_MS: u64 = 16;
const FADE_IN_S: f32 = 0.2;
const PULSE_HZ: f32 = 1.1;
const PULSE_MIN: f32 = 0.73;
const PULSE_MAX: f32 = 1.0;
/// Self-dismiss if the host never signals first paint (crash before frame).
pub(crate) const DISMISS_TIMEOUT: Duration = Duration::from_secs(10);
/// Rounded-corner radius for the backdrop (px). Matches splash_mac.rs.
pub(crate) const CORNER_RADIUS_PX: f32 = 16.0;
/// Opacity fade-out duration on dismiss. Matches splash_mac.rs.
pub(crate) const FADE_OUT: Duration = Duration::from_millis(160);

// Darkened window-edge border — SPEC_SPLASH_SCREEN_BORDER_2026_08_25.md.
// Roughly half BG's RGB values, a straightforward "darker than the
// backdrop" reading; not yet confirmed against a real display. Matches
// splash.rs / splash_mac.rs.
pub(crate) const BORDER_R: u8 = 0x0D;
pub(crate) const BORDER_G: u8 = 0x0D;
pub(crate) const BORDER_B: u8 = 0x10;
pub(crate) const BORDER_WIDTH_PX: i32 = 2;

/// Brain logo as straight (non-pre-multiplied) RGBA8, emitted by build.rs.
pub(crate) static BRAIN_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/brain_rgba.bin"));
include!(concat!(env!("OUT_DIR"), "/brain_dims.rs")); // pub const BRAIN_W / BRAIN_H (i32)

// ── Stage list band (startup telemetry between brain and footer) ─────────────
/// Muted stage text color — slightly brighter than the footer for readability.
pub(crate) const STAGE_COLOR: [u8; 3] = [0xA0, 0xA0, 0xAA];
/// Vertical padding above the first stage line.
pub(crate) const STAGE_PAD_TOP: i32 = 10;
/// Vertical gap between stage lines.
pub(crate) const STAGE_LINE_GAP: i32 = 4;
/// Maximum number of stage lines shown simultaneously.
pub(crate) const STAGE_MAX_LINES: usize = 4;
/// Height of the stage band.
pub(crate) const STAGE_H: i32 = STAGE_PAD_TOP
    + STAGE_MAX_LINES as i32 * crate::splash_font::GLYPH_H as i32
    + (STAGE_MAX_LINES as i32 - 1) * STAGE_LINE_GAP;
/// Left margin for stage lines (px from card edge).
pub(crate) const STAGE_X: i32 = 20;

// ── Footer (identity strip near the bottom) ─────────────────────────────────
/// Muted footer text color (#8A8A93) on the #1A1A1F backdrop.
pub(crate) const FOOTER_COLOR: [u8; 3] = [0x8A, 0x8A, 0x93];
pub(crate) const FOOTER_PAD_TOP: i32 = 12;
pub(crate) const FOOTER_LINE_GAP: i32 = 3;
pub(crate) const FOOTER_PAD_BOTTOM: i32 = 12;
/// Bottom band height: top pad + 2 glyph rows + inter-line gap + bottom pad.
pub(crate) const FOOTER_H: i32 =
    FOOTER_PAD_TOP + 2 * crate::splash_font::GLYPH_H as i32 + FOOTER_LINE_GAP + FOOTER_PAD_BOTTOM;
/// Full card dimensions: brain + padding, stage band, footer band.
pub(crate) const CARD_W: i32 = BRAIN_W + PADDING * 2;
pub(crate) const CARD_H: i32 = BRAIN_H + PADDING * 2 + STAGE_H + FOOTER_H;

// ── Startup telemetry stage list ─────────────────────────────────────────────

struct StageEntry {
    stage: &'static str,
    label: &'static str,
    started_at: Instant,
    duration_ms: Option<u64>,
    status: StartupStatus,
    subs: Vec<SubEntry>,
}

struct SubEntry {
    id: String,
    label: String,
    started_at: Instant,
    duration_ms: Option<u64>,
}

/// Accumulates startup events and emits rendered text lines for the stage band.
pub(super) struct StageList {
    stages: Vec<StageEntry>,
}

impl StageList {
    pub(super) fn new() -> Self {
        Self { stages: vec![] }
    }

    pub(super) fn apply(&mut self, event: StartupEvent) {
        match event {
            StartupEvent::StageBegin { stage, label } => {
                self.stages.push(StageEntry {
                    stage,
                    label,
                    started_at: Instant::now(),
                    duration_ms: None,
                    status: StartupStatus::Ok,
                    subs: vec![],
                });
            }
            StartupEvent::StageEnd { stage, duration_ms, status, .. } => {
                if let Some(e) = self.stages.iter_mut().rev().find(|e| e.stage == stage) {
                    e.duration_ms = Some(duration_ms);
                    e.status = status;
                }
            }
            StartupEvent::SubBegin { stage, id, label } => {
                if let Some(e) = self.stages.iter_mut().rev().find(|e| e.stage == stage) {
                    e.subs.push(SubEntry {
                        id,
                        label,
                        started_at: Instant::now(),
                        duration_ms: None,
                    });
                }
            }
            StartupEvent::SubEnd { stage, id, duration_ms, .. } => {
                if let Some(e) = self.stages.iter_mut().rev().find(|e| e.stage == stage) {
                    if let Some(s) = e.subs.iter_mut().find(|s| s.id == id) {
                        s.duration_ms = Some(duration_ms);
                    }
                }
            }
        }
    }

    /// Up to `STAGE_MAX_LINES` formatted lines for the stage band.
    pub(super) fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.stages {
            let pfx = match s.duration_ms {
                None => ">> ",
                Some(_) => match s.status {
                    StartupStatus::Ok => "ok ",
                    _ => "!! ",
                },
            };
            let ms = match s.duration_ms {
                Some(ms) => ms,
                None => s.started_at.elapsed().as_millis() as u64,
            };
            // "ok  Saga recovery       42ms"
            out.push(format!("{pfx}{:<18}{:>5}ms", s.label, ms));
            if out.len() >= STAGE_MAX_LINES {
                break;
            }
            // Show the last sub-item (e.g. most recent migration).
            if let Some(sub) = s.subs.last() {
                let sub_ms = match sub.duration_ms {
                    Some(ms) => ms,
                    None => sub.started_at.elapsed().as_millis() as u64,
                };
                out.push(format!("   >{:<16}{:>5}ms", sub.label, sub_ms));
                if out.len() >= STAGE_MAX_LINES {
                    break;
                }
            }
        }
        out
    }
}

enum Session {
    Wayland,
    X11,
}

/// Pick the splash backend by **centering ability**. The X11 backend self-centers
/// (override-redirect, positioned on the RANDR primary) and XWayland honors that,
/// so prefer it whenever an X server is actually reachable — including under a
/// native-Wayland session, where the Wayland backend can't self-center and Mutter
/// drops the splash top-left. Only with no usable X display do we fall back to the
/// best-effort Wayland backend.
///
/// An explicit `AGENTMUX_OZONE_PLATFORM=x11|wayland` still pins a specific backend
/// (escape hatch / for exercising the Wayland path itself); note that pinning
/// `wayland` re-introduces the top-left placement Mutter gives uncentered toplevels.
fn detect() -> Session {
    match std::env::var("AGENTMUX_OZONE_PLATFORM").ok().as_deref() {
        Some("x11") => return Session::X11,
        Some("wayland") => return Session::Wayland,
        _ => {}
    }
    // `DISPLAY` being set doesn't prove a server is live (stale env / no
    // XWayland), so probe a real connection — a failed handshake is fast.
    if x11::server_reachable() {
        Session::X11
    } else {
        Session::Wayland
    }
}

/// Minimum on-screen time before the splash is allowed to dismiss, so a fast
/// cold start can't produce a sub-perceptible flash (worse than no splash).
/// Overridable via `AGENTMUX_SPLASH_HOLD_MS` — also handy for eyeballing the
/// splash during testing (e.g. `AGENTMUX_SPLASH_HOLD_MS=3000`).
pub(crate) fn min_hold() -> Duration {
    std::env::var("AGENTMUX_SPLASH_HOLD_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(450))
}

/// Pulse alpha in 0..=1 for elapsed seconds since the splash appeared: a linear
/// ramp 0→1 over `FADE_IN_S`, then a `PULSE_HZ` sine between `PULSE_MIN..PULSE_MAX`
/// (the backdrop stays opaque; only the brain's alpha breathes). Shared by both
/// backends.
pub(crate) fn pulse_alpha(t: f32) -> f32 {
    if t < FADE_IN_S {
        return t / FADE_IN_S;
    }
    let s = (((t - FADE_IN_S) * std::f32::consts::TAU * PULSE_HZ).sin() + 1.0) * 0.5;
    PULSE_MIN + s * (PULSE_MAX - PULSE_MIN)
}

/// Global fade-out opacity in 0..=1. Returns 1.0 until `fade_start` is set, then
/// ramps linearly to 0.0 over `FADE_OUT`. Shared by both backends.
pub(crate) fn fade_alpha(fade_start: Option<Instant>, now: Instant) -> f32 {
    match fade_start {
        None => 1.0,
        Some(s) => (1.0 - now.duration_since(s).as_secs_f32() / FADE_OUT.as_secs_f32())
            .clamp(0.0, 1.0),
    }
}

/// Rounded-rect coverage (0..=1) at pixel (x, y) for a `w`×`h` rect with corner
/// `radius` (0 = square), anti-aliased over a 1-px band on the corner arcs.
fn corner_coverage(x: i32, y: i32, w: i32, h: i32, radius: f32) -> f32 {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    if radius <= 0.0 {
        // No rounding to compute -- but still bounds-check (x, y) against
        // the w×h rect rather than unconditionally returning 1.0. The old
        // unconditional shortcut was only ever safe because render_frame's
        // one call site always passed in-bounds coordinates by
        // construction (x in 0..w, y in 0..h); the border-band check below
        // calls this a second time with coordinates shifted relative to a
        // SHRUNK inner rect, which are legitimately out-of-bounds for
        // border-band pixels. Under the square X11-fallback path
        // (radius=0, no compositor), inner_radius also clamps to 0
        // (BORDER_WIDTH_PX - BORDER_WIDTH_PX), so without this bounds
        // check interior_t was always 1.0 and the border never rendered
        // at all in that fallback (Codex P2, PR #2804 review).
        return if fx >= 0.0 && fx <= w as f32 && fy >= 0.0 && fy <= h as f32 {
            1.0
        } else {
            0.0
        };
    }
    // Clamp to the corner-arc center; in the straight edges/interior this yields
    // dist 0 → full coverage. Only the four corner boxes produce a nonzero dist.
    let cx = fx.clamp(radius, w as f32 - radius);
    let cy = fy.clamp(radius, h as f32 - radius);
    let dist = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
    if dist <= radius - 0.5 {
        1.0
    } else if dist >= radius + 0.5 {
        0.0
    } else {
        radius + 0.5 - dist
    }
}

/// Composite one frame, **pre-multiplied**, into a 4-byte-per-pixel buffer: the
/// dark backdrop with the centered brain blended at `brain_alpha`, the stage list
/// band, and the footer identity strip — all masked to a `radius`-rounded rect
/// and scaled by the global `window_alpha` (fade-out). Pre-multiplied output is
/// what both X Render compositors and `wl_shm` ARGB8888 expect. `bgr` selects
/// byte order: `true` → B,G,R,A (X11 ARGB / Wayland ARGB8888 on LE), `false` →
/// R,G,B,A. Shared by both backends.
///
/// `radius=0` + `window_alpha=1` yields a fully-opaque square (the X11
/// no-compositor fallback path). `stages` may be empty (no events received).
pub(crate) fn render_frame(
    buf: &mut [u8],
    w: i32,
    h: i32,
    brain_alpha: f32,
    window_alpha: f32,
    radius: f32,
    bgr: bool,
    footer: &[String],
    stages: &[String],
) {
    let ox = (w - BRAIN_W) / 2;
    // Center the brain in the brain region (above stage band and footer), not the
    // whole card — otherwise the stage band + footer would push it off-center.
    let oy = (h - FOOTER_H - STAGE_H - BRAIN_H) / 2;
    for y in 0..h {
        for x in 0..w {
            // Backdrop, in RGB.
            let mut rr = BG_R as f32;
            let mut gg = BG_G as f32;
            let mut bb = BG_B as f32;
            // Brain over backdrop (straight alpha).
            let (bx, by) = (x - ox, y - oy);
            if bx >= 0 && bx < BRAIN_W && by >= 0 && by < BRAIN_H {
                let si = ((by * BRAIN_W + bx) * 4) as usize;
                let ba = (BRAIN_RGBA[si + 3] as f32 / 255.0) * brain_alpha;
                rr = rr * (1.0 - ba) + BRAIN_RGBA[si] as f32 * ba;
                gg = gg * (1.0 - ba) + BRAIN_RGBA[si + 1] as f32 * ba;
                bb = bb * (1.0 - ba) + BRAIN_RGBA[si + 2] as f32 * ba;
            }
            // Darkened border band: blend toward BORDER_* the further a pixel
            // falls outside the shrunk "interior" rect (inset by
            // BORDER_WIDTH_PX on all sides, radius reduced to match) versus
            // the outer rect — same anti-aliased coverage math as the corner
            // rounding itself, just against two nested rects instead of one.
            // The brain only ever occupies well-interior pixels (PADDING=28
            // >> BORDER_WIDTH_PX), so blending the already-brain-composited
            // color here is safe. SPEC_SPLASH_SCREEN_BORDER_2026_08_25.md.
            let outer_cov = corner_coverage(x, y, w, h, radius);
            let inner_radius = (radius - BORDER_WIDTH_PX as f32).max(0.0);
            let inner_cov = corner_coverage(
                x - BORDER_WIDTH_PX,
                y - BORDER_WIDTH_PX,
                w - 2 * BORDER_WIDTH_PX,
                h - 2 * BORDER_WIDTH_PX,
                inner_radius,
            );
            let interior_t = if outer_cov > 0.0 { (inner_cov / outer_cov).clamp(0.0, 1.0) } else { 0.0 };
            rr = BORDER_R as f32 * (1.0 - interior_t) + rr * interior_t;
            gg = BORDER_G as f32 * (1.0 - interior_t) + gg * interior_t;
            bb = BORDER_B as f32 * (1.0 - interior_t) + bb * interior_t;

            // Coverage (rounded corners) × global fade → pre-multiplied alpha.
            let a = outer_cov * window_alpha;
            let di = ((y * w + x) * 4) as usize;
            let (o0, o2) = if bgr { (bb, rr) } else { (rr, bb) };
            buf[di] = (o0 * a) as u8;
            buf[di + 1] = (gg * a) as u8;
            buf[di + 2] = (o2 * a) as u8;
            buf[di + 3] = (a * 255.0) as u8;
        }
    }

    // Stage list: startup telemetry lines above the footer.
    let stage_top = h - FOOTER_H - STAGE_H + STAGE_PAD_TOP;
    for (i, line) in stages.iter().enumerate() {
        let y = stage_top + i as i32 * (crate::splash_font::GLYPH_H as i32 + STAGE_LINE_GAP);
        crate::splash_text::draw_text(buf, w, h, STAGE_X, y, line, STAGE_COLOR, window_alpha, bgr);
    }

    // Footer: muted identity lines in the bottom band (static; fades with the card).
    let footer_top = h - FOOTER_H + FOOTER_PAD_TOP;
    for (i, line) in footer.iter().enumerate() {
        let y = footer_top + i as i32 * (crate::splash_font::GLYPH_H as i32 + FOOTER_LINE_GAP);
        crate::splash_text::draw_text_centered(buf, w, h, y, line, FOOTER_COLOR, window_alpha, bgr);
    }
}

/// Spawn the startup splash. Sets `AGENTMUX_SPLASH_READY_FILE` (inherited by the
/// host, which writes it on first paint) and runs the matching backend on a
/// dedicated thread, so the launcher's main thread continues to start srv + host.
/// The thread self-terminates on first-paint or the safety timeout. Best-effort:
/// any failure (no display, protocol error) just means no splash — the launcher
/// is unaffected.
///
/// `startup_rx` is the receiver end of the startup event bus. The splash thread
/// drains it every animation frame to update the stage list. Pass the receiver
/// from `startup_events::StartupEventSink::new()` created in `main()` before
/// the Tokio runtime is started.
pub fn spawn(startup_rx: Receiver<StartupEvent>) {
    let ready_file =
        std::env::temp_dir().join(format!("agentmux-splash-ready-{}", std::process::id()));
    let _ = std::fs::remove_file(&ready_file);
    // Set before any host is spawned so the host inherits it (same trick as
    // splash_mac.rs). Safe: the launcher is single-threaded at this point.
    std::env::set_var("AGENTMUX_SPLASH_READY_FILE", &ready_file);

    // Footer identity, gathered once and clamped to the card width.
    let info = crate::splash_info::SplashInfo::gather();
    let max_chars = ((CARD_W - 24) / crate::splash_font::GLYPH_W as i32).max(8) as usize;
    let footer = info.footer_lines(max_chars);

    match detect() {
        Session::X11 => {
            std::thread::Builder::new()
                .name("agentmux-splash".into())
                .spawn(move || {
                    if let Err(e) = x11::run(&ready_file, footer, startup_rx) {
                        eprintln!("[splash] x11 backend disabled: {e}");
                    }
                })
                .ok();
        }
        Session::Wayland => {
            std::thread::Builder::new()
                .name("agentmux-splash".into())
                .spawn(move || {
                    if let Err(e) = wayland::run(&ready_file, footer, startup_rx) {
                        eprintln!("[splash] wayland backend disabled: {e}");
                    }
                })
                .ok();
        }
    }
}
