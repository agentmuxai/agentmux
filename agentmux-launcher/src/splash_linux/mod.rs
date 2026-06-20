// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session-aware Linux startup splash.
//!
//! The launcher is up in milliseconds; `agentmux-cef` *is* the multi-second cold
//! start (CEF init + GPU channel + page load). This paints a pulsing brain logo
//! over a dark backdrop in that gap — the Linux analogue of `splash.rs` (Win32)
//! and `splash_mac.rs` (Cocoa).
//!
//! The backend mirrors the host's ozone choice (`agentmux-cef/src/app.rs`): an
//! X11 splash for X11 / XWayland sessions, a Wayland splash for native-Wayland
//! sessions — so the splash always speaks the same display protocol as the host.
//!
//! Dismiss protocol is the macOS one: `spawn()` sets `AGENTMUX_SPLASH_READY_FILE`
//! (the host inherits it and writes the file on first paint); the splash thread
//! polls for that file and tears down, with a safety timeout for a host that
//! crashes before painting.
//!
//! Spec: docs/specs/SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md

mod wayland;
mod x11;

use std::time::Duration;

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

/// Brain logo as straight (non-pre-multiplied) RGBA8, emitted by build.rs.
pub(crate) static BRAIN_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/brain_rgba.bin"));
include!(concat!(env!("OUT_DIR"), "/brain_dims.rs")); // pub const BRAIN_W / BRAIN_H (i32)

enum Session {
    Wayland,
    X11,
}

/// Mirror the host's ozone selection (`agentmux-cef/src/app.rs`): an explicit
/// `AGENTMUX_OZONE_PLATFORM` wins; otherwise native Wayland when `WAYLAND_DISPLAY`
/// is set; otherwise X11. Keeping this in lockstep guarantees splash and host
/// never end up on different display protocols.
fn detect() -> Session {
    if let Ok(forced) = std::env::var("AGENTMUX_OZONE_PLATFORM") {
        match forced.as_str() {
            "wayland" => return Session::Wayland,
            "x11" => return Session::X11,
            _ => {}
        }
    }
    let on_wayland = std::env::var("WAYLAND_DISPLAY")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if on_wayland {
        Session::Wayland
    } else {
        Session::X11
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

/// Composite one frame into a 4-byte-per-pixel buffer: fill the dark backdrop,
/// then alpha-blend the brain (centered) at `alpha` strength. Pixel byte order
/// is given by `bgr`: `true` writes B,G,R,X (X11 depth-24 / Wayland ARGB on LE),
/// `false` writes R,G,B,X. Shared by both backends.
pub(crate) fn render_frame(buf: &mut [u8], w: i32, h: i32, alpha: f32, bgr: bool) {
    let (c0, c2) = if bgr { (BG_B, BG_R) } else { (BG_R, BG_B) };
    for px in buf.chunks_exact_mut(4) {
        px[0] = c0;
        px[1] = BG_G;
        px[2] = c2;
        px[3] = 0xFF;
    }
    let ox = (w - BRAIN_W) / 2;
    let oy = (h - BRAIN_H) / 2;
    for by in 0..BRAIN_H {
        let dy = oy + by;
        if dy < 0 || dy >= h {
            continue;
        }
        for bx in 0..BRAIN_W {
            let dx = ox + bx;
            if dx < 0 || dx >= w {
                continue;
            }
            let si = ((by * BRAIN_W + bx) * 4) as usize;
            let sr = BRAIN_RGBA[si] as f32;
            let sg = BRAIN_RGBA[si + 1] as f32;
            let sb = BRAIN_RGBA[si + 2] as f32;
            let a = (BRAIN_RGBA[si + 3] as f32 / 255.0) * alpha;
            let di = ((dy * w + dx) * 4) as usize;
            let (b0, b2) = if bgr { (sb, sr) } else { (sr, sb) };
            buf[di] = (buf[di] as f32 * (1.0 - a) + b0 * a) as u8;
            buf[di + 1] = (buf[di + 1] as f32 * (1.0 - a) + sg * a) as u8;
            buf[di + 2] = (buf[di + 2] as f32 * (1.0 - a) + b2 * a) as u8;
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
