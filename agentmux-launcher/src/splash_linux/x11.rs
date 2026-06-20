// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! X11 splash backend — a software-drawn, override-redirect window showing the
//! pulsing brain over a dark backdrop, dismissed when the host writes the
//! ready-file (or after the safety timeout). Used for X11 and XWayland sessions.
//!
//! No GPU: we composite each frame into an off-screen pixmap (depth-24 BGRX) and
//! `CopyArea` it to the window at ~60 fps. Window placement, `override_redirect`,
//! and EWMH splash/above/skip-taskbar hints give a proper splash under any X11
//! window manager and under XWayland.

use std::error::Error;
use std::path::Path;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, CreateGCAux, CreateWindowAux, EventMask, ImageFormat, PropMode, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use super::{
    pulse_alpha, render_frame, BRAIN_H, BRAIN_W, DISMISS_TIMEOUT, FRAME_MS, PADDING,
};

pub(super) fn run(ready_file: &Path) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let depth = screen.root_depth; // typically 24
    let visual = screen.root_visual;

    let w = (BRAIN_W + PADDING * 2) as u16;
    let h = (BRAIN_H + PADDING * 2) as u16;
    // Center on the X screen. (RANDR primary-monitor centering is P3 polish.)
    let x = (((screen.width_in_pixels as i32) - w as i32) / 2).max(0) as i16;
    let y = (((screen.height_in_pixels as i32) - h as i32) / 2).max(0) as i16;

    let win = conn.generate_id()?;
    let win_aux = CreateWindowAux::new()
        .override_redirect(1)
        .background_pixel(screen.black_pixel)
        .event_mask(EventMask::EXPOSURE);
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
    let min_hold = super::min_hold();
    // BGRX, 4 bytes/pixel (depth-24 pixmaps are 32 bpp on every modern server).
    let mut buf = vec![0u8; w as usize * h as usize * 4];

    loop {
        let elapsed = start.elapsed();
        // Dismiss once the host has painted AND we've shown for the minimum hold,
        // or unconditionally at the safety timeout (host crashed before paint).
        if (ready_file.exists() && elapsed >= min_hold) || elapsed >= DISMISS_TIMEOUT {
            break;
        }

        let alpha = pulse_alpha(start.elapsed().as_secs_f32());
        render_frame(&mut buf, w as i32, h as i32, alpha, /* bgr = */ true);

        // PutImage in horizontal strips so no single request exceeds the server's
        // max request size (a full ~312×312 BGRX frame is ~380 KB, over the
        // 256 KB non-BIG-REQUESTS limit — strips keep us portable either way).
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

        // Drain and ignore events so the connection's read buffer can't stall.
        while let Ok(Some(_)) = conn.poll_for_event() {}

        std::thread::sleep(Duration::from_millis(FRAME_MS));
    }

    let _ = conn.destroy_window(win);
    let _ = conn.free_pixmap(pixmap);
    let _ = conn.free_gc(gc);
    let _ = conn.flush();
    Ok(())
}

/// Best-effort EWMH hints (harmless under `override_redirect`, helpful if a WM
/// still considers them): mark the window as a splash, ask for above + no
/// taskbar/pager entry.
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
