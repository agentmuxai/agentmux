// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! X11 splash backend — a software-drawn, override-redirect window showing the
//! pulsing brain over a dark backdrop, dismissed (with a fade-out) when the host
//! writes the ready-file or after the safety timeout. For X11 / XWayland sessions.
//!
//! When a compositor is present we use a 32-bit **ARGB** visual so the backdrop
//! can have rounded corners and the whole splash can fade out (per-pixel alpha,
//! pre-multiplied). With no compositor there's no per-pixel alpha, so we fall
//! back to an opaque depth-24 square that dismisses abruptly. Centering uses the
//! RANDR primary monitor. No GPU — we composite each frame into an off-screen
//! pixmap and `CopyArea` it to the window at ~60 fps.
//!
//! Spec: docs/specs/SPEC_LINUX_SPLASH_POLISH_2026_06_20.md

use std::error::Error;
use std::path::Path;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ColormapAlloc, CreateGCAux, CreateWindowAux, EventMask, ImageFormat, PropMode, Screen,
    VisualClass, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::NONE;

use super::{
    fade_alpha, min_hold, pulse_alpha, render_frame, CARD_H, CARD_W, CORNER_RADIUS_PX,
    DISMISS_TIMEOUT, FRAME_MS,
};

/// Cheap reachability probe for the backend selector (`super::detect`): can we
/// actually open an X11 connection? `DISPLAY` being set doesn't guarantee a live
/// server, so we test the real handshake (which fails fast when there's none).
/// Only when this succeeds do we prefer the self-centering X11/XWayland splash
/// over the native-Wayland fallback.
pub(super) fn server_reachable() -> bool {
    x11rb::connect(None).is_ok()
}

pub(super) fn run(ready_file: &Path, footer: Vec<String>) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn.setup().roots[screen_num].clone();
    let root = screen.root;

    let w = CARD_W as u16;
    let h = CARD_H as u16;

    // Per-pixel alpha (→ fade + rounded corners) needs a compositor + an ARGB
    // visual. Otherwise fall back to an opaque depth-24 square.
    let argb_visual = if has_compositor(&conn, screen_num).unwrap_or(false) {
        find_argb_visual(&screen)
    } else {
        None
    };

    let (depth, visual, colormap) = match argb_visual {
        Some(vid) => {
            let cmap = conn.generate_id()?;
            conn.create_colormap(ColormapAlloc::NONE, cmap, root, vid)?;
            (32u8, vid, Some(cmap))
        }
        None => (screen.root_depth, screen.root_visual, None),
    };
    let radius = if colormap.is_some() { CORNER_RADIUS_PX } else { 0.0 };

    // Center on the primary monitor (RANDR), falling back to the whole screen.
    let (mx, my, mw, mh) = primary_monitor(&conn, root, &screen);
    let x = (mx + (mw - w as i32) / 2).max(0) as i16;
    let y = (my + (mh - h as i32) / 2).max(0) as i16;

    let win = conn.generate_id()?;
    let mut win_aux = CreateWindowAux::new()
        .override_redirect(1)
        .event_mask(EventMask::EXPOSURE);
    win_aux = if let Some(cmap) = colormap {
        // A window whose depth differs from its parent needs its own colormap
        // and an explicit border_pixel (else BadMatch). 0 = transparent backdrop.
        win_aux.colormap(cmap).border_pixel(0).background_pixel(0)
    } else {
        win_aux.background_pixel(screen.black_pixel)
    };
    conn.create_window(
        depth,
        win,
        root,
        x,
        y,
        w,
        h,
        0,
        WindowClass::INPUT_OUTPUT,
        visual,
        &win_aux,
    )?;

    set_ewmh_hints(&conn, win)?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, win, &CreateGCAux::new())?;
    let pixmap = conn.generate_id()?;
    conn.create_pixmap(depth, pixmap, win, w, h)?;

    conn.map_window(win)?;
    conn.flush()?;

    let start = Instant::now();
    let hold = min_hold();
    let mut fade_start: Option<Instant> = None;
    // 4 bytes/pixel (depth-24 and depth-32 are both 32 bpp on modern servers).
    let mut buf = vec![0u8; w as usize * h as usize * 4];

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(start);
        let dismiss =
            (ready_file.exists() && elapsed >= hold) || elapsed >= DISMISS_TIMEOUT;
        if dismiss {
            if colormap.is_none() {
                break; // opaque depth-24: can't alpha-fade, dismiss now
            }
            if fade_start.is_none() {
                fade_start = Some(now);
            }
        }
        let window_alpha = fade_alpha(fade_start, now);
        if fade_start.is_some() && window_alpha <= 0.0 {
            break;
        }

        let brain_alpha = pulse_alpha(elapsed.as_secs_f32());
        render_frame(
            &mut buf,
            w as i32,
            h as i32,
            brain_alpha,
            window_alpha,
            radius,
            true,
            &footer,
        );

        // PutImage in horizontal strips so no single request exceeds the server's
        // max request size (~380 KB frame > the 256 KB non-BIG-REQUESTS limit).
        let row_bytes = w as usize * 4;
        let strip_rows = (200_000 / row_bytes).max(1) as u16;
        let mut row = 0u16;
        while row < h {
            let rows = strip_rows.min(h - row);
            let off = row as usize * row_bytes;
            let len = rows as usize * row_bytes;
            conn.put_image(
                ImageFormat::Z_PIXMAP,
                pixmap,
                gc,
                w,
                rows,
                0,
                row as i16,
                0,
                depth,
                &buf[off..off + len],
            )?;
            row += rows;
        }
        conn.copy_area(pixmap, win, gc, 0, 0, 0, 0, w, h)?;
        conn.flush()?;

        while let Ok(Some(_)) = conn.poll_for_event() {}
        std::thread::sleep(Duration::from_millis(FRAME_MS));
    }

    let _ = conn.destroy_window(win);
    let _ = conn.free_pixmap(pixmap);
    let _ = conn.free_gc(gc);
    if let Some(cmap) = colormap {
        let _ = conn.free_colormap(cmap);
    }
    let _ = conn.flush();
    Ok(())
}

/// True if a compositing manager owns the `_NET_WM_CM_S<screen>` selection
/// (GNOME/Mutter and XWayland under it always do) — i.e. per-pixel alpha works.
fn has_compositor<C: Connection>(conn: &C, screen_num: usize) -> Result<bool, Box<dyn Error>> {
    let name = format!("_NET_WM_CM_S{screen_num}");
    let atom = conn.intern_atom(false, name.as_bytes())?.reply()?.atom;
    let owner = conn.get_selection_owner(atom)?.reply()?.owner;
    Ok(owner != NONE)
}

/// First 32-bit TrueColor visual on the screen, if any.
fn find_argb_visual(screen: &Screen) -> Option<u32> {
    screen
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR)
        .map(|v| v.visual_id)
}

/// RANDR primary-monitor geometry `(x, y, w, h)`, falling back to the whole X
/// screen if RANDR/primary is unavailable.
fn primary_monitor<C: Connection>(conn: &C, root: Window, screen: &Screen) -> (i32, i32, i32, i32) {
    let fallback = (
        0,
        0,
        screen.width_in_pixels as i32,
        screen.height_in_pixels as i32,
    );
    let prim = match conn.randr_get_output_primary(root).ok().and_then(|c| c.reply().ok()) {
        Some(p) if p.output != NONE => p.output,
        _ => return fallback,
    };
    let info = match conn.randr_get_output_info(prim, 0).ok().and_then(|c| c.reply().ok()) {
        Some(i) if i.crtc != NONE => i,
        _ => return fallback,
    };
    match conn.randr_get_crtc_info(info.crtc, 0).ok().and_then(|c| c.reply().ok()) {
        Some(c) => (c.x as i32, c.y as i32, c.width as i32, c.height as i32),
        None => fallback,
    }
}

/// Best-effort EWMH hints (harmless under `override_redirect`): mark as a splash,
/// ask for above + no taskbar/pager entry.
fn set_ewmh_hints<C: Connection>(conn: &C, win: Window) -> Result<(), Box<dyn Error>> {
    let atom = |name: &[u8]| -> Result<u32, Box<dyn Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };

    let wm_type = atom(b"_NET_WM_WINDOW_TYPE")?;
    let wm_type_splash = atom(b"_NET_WM_WINDOW_TYPE_SPLASH")?;
    conn.change_property32(PropMode::REPLACE, win, wm_type, AtomEnum::ATOM, &[wm_type_splash])?;

    let wm_state = atom(b"_NET_WM_STATE")?;
    let above = atom(b"_NET_WM_STATE_ABOVE")?;
    let skip_taskbar = atom(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pager = atom(b"_NET_WM_STATE_SKIP_PAGER")?;
    conn.change_property32(
        PropMode::REPLACE,
        win,
        wm_state,
        AtomEnum::ATOM,
        &[above, skip_taskbar, skip_pager],
    )?;

    Ok(())
}
