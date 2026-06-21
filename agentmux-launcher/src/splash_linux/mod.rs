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

use std::time::{Duration, Instant};

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

/// Brain logo as straight (non-pre-multiplied) RGBA8, emitted by build.rs.
pub(crate) static BRAIN_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/brain_rgba.bin"));
include!(concat!(env!("OUT_DIR"), "/brain_dims.rs")); // pub const BRAIN_W / BRAIN_H (i32)

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
    if radius <= 0.0 {
        return 1.0;
    }
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
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
/// dark backdrop with the centered brain blended at `brain_alpha`, masked to a
/// `radius`-rounded rect, all scaled by the global `window_alpha` (fade-out).
/// Pre-multiplied output is what both X Render compositors and `wl_shm`
/// ARGB8888 expect. `bgr` selects byte order: `true` → B,G,R,A (X11 ARGB /
/// Wayland ARGB8888 on LE), `false` → R,G,B,A. Shared by both backends.
///
/// `radius=0` + `window_alpha=1` yields a fully-opaque square (the X11
/// no-compositor fallback path).
pub(crate) fn render_frame(
    buf: &mut [u8],
    w: i32,
    h: i32,
    brain_alpha: f32,
    window_alpha: f32,
    radius: f32,
    bgr: bool,
) {
    let ox = (w - BRAIN_W) / 2;
    let oy = (h - BRAIN_H) / 2;
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
            // Coverage (rounded corners) × global fade → pre-multiplied alpha.
            let a = corner_coverage(x, y, w, h, radius) * window_alpha;
            let di = ((y * w + x) * 4) as usize;
            let (o0, o2) = if bgr { (bb, rr) } else { (rr, bb) };
            buf[di] = (o0 * a) as u8;
            buf[di + 1] = (gg * a) as u8;
            buf[di + 2] = (o2 * a) as u8;
            buf[di + 3] = (a * 255.0) as u8;
        }
    }
}

/// Spawn the startup splash. Sets `AGENTMUX_SPLASH_READY_FILE` (inherited by the
/// host, which writes it on first paint) and runs the matching backend on a
/// dedicated thread, so the launcher's main thread continues to start srv + host.
/// The thread self-terminates on first-paint or the safety timeout. Best-effort:
/// any failure (no display, protocol error) just means no splash — the launcher
/// is unaffected.
pub fn spawn() {
    let ready_file =
        std::env::temp_dir().join(format!("agentmux-splash-ready-{}", std::process::id()));
    let _ = std::fs::remove_file(&ready_file);
    // Set before any host is spawned so the host inherits it (same trick as
    // splash_mac.rs). Safe: the launcher is single-threaded at this point.
    std::env::set_var("AGENTMUX_SPLASH_READY_FILE", &ready_file);

    match detect() {
        Session::X11 => {
            std::thread::Builder::new()
                .name("agentmux-splash".into())
                .spawn(move || {
                    if let Err(e) = x11::run(&ready_file) {
                        eprintln!("[splash] x11 backend disabled: {e}");
                    }
                })
                .ok();
        }
        Session::Wayland => {
            std::thread::Builder::new()
                .name("agentmux-splash".into())
                .spawn(move || {
                    if let Err(e) = wayland::run(&ready_file) {
                        eprintln!("[splash] wayland backend disabled: {e}");
                    }
                })
                .ok();
        }
    }
}
