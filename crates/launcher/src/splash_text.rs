// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Software text blitter for the splash footer — shared by the Linux software
//! buffers (`splash_linux`) and the Win32 layered-window DIB (`splash.rs`).
//! macOS uses native `NSTextField` instead, so it does not use this module.
//!
//! Composites grayscale glyph coverage (`splash_font`) into a **pre-multiplied**
//! 4-bytes-per-pixel buffer (the same format `render_frame` / the DIB produce):
//! `dst = dst*(1-a) + color*a`, with `a = coverage * window_alpha`, so the footer
//! fades in/out with the rest of the card.

use crate::splash_font::{COVERAGE, FIRST, GLYPH_H, GLYPH_W, LAST};

/// Pixel width of `s` rendered in the monospace glyph cell.
pub fn text_width(s: &str) -> i32 {
    s.chars().count() as i32 * GLYPH_W as i32
}

#[inline]
fn glyph_index(ch: char) -> usize {
    let code = ch as u32;
    if code >= FIRST as u32 && code <= LAST as u32 {
        (code - FIRST as u32) as usize
    } else {
        ('?' as u32 - FIRST as u32) as usize // unknown glyph → '?'
    }
}

/// Blit `text` into `buf` (pre-multiplied, `buf_w`×`buf_h`, 4 bpp) with top-left
/// at (`x0`, `y0`). `color` is straight RGB; `window_alpha` scales the whole draw
/// (fade); `bgr` selects byte order (`true` → B,G,R,A as Wayland ARGB8888-on-LE /
/// X11 ARGB / Win32 BGRA expect, `false` → R,G,B,A).
pub fn draw_text(
    buf: &mut [u8],
    buf_w: i32,
    buf_h: i32,
    x0: i32,
    y0: i32,
    text: &str,
    color: [u8; 3],
    window_alpha: f32,
    bgr: bool,
) {
    let (cr, cg, cb) = (color[0] as f32, color[1] as f32, color[2] as f32);
    let (o0, o2) = if bgr { (cb, cr) } else { (cr, cb) };
    let mut pen_x = x0;
    for ch in text.chars() {
        let base = glyph_index(ch) * GLYPH_H * GLYPH_W;
        for gy in 0..GLYPH_H as i32 {
            let py = y0 + gy;
            if py < 0 || py >= buf_h {
                continue;
            }
            for gx in 0..GLYPH_W as i32 {
                let px = pen_x + gx;
                if px < 0 || px >= buf_w {
                    continue;
                }
                let cov = COVERAGE[base + gy as usize * GLYPH_W + gx as usize] as f32 / 255.0;
                if cov <= 0.0 {
                    continue;
                }
                let a = cov * window_alpha;
                let di = ((py * buf_w + px) * 4) as usize;
                // Pre-multiplied over-composite.
                buf[di] = (buf[di] as f32 * (1.0 - a) + o0 * a) as u8;
                buf[di + 1] = (buf[di + 1] as f32 * (1.0 - a) + cg * a) as u8;
                buf[di + 2] = (buf[di + 2] as f32 * (1.0 - a) + o2 * a) as u8;
                buf[di + 3] = (buf[di + 3] as f32 * (1.0 - a) + 255.0 * a) as u8;
            }
        }
        pen_x += GLYPH_W as i32;
    }
}

/// Horizontally-centered [`draw_text`] at row `y0`.
pub fn draw_text_centered(
    buf: &mut [u8],
    buf_w: i32,
    buf_h: i32,
    y0: i32,
    text: &str,
    color: [u8; 3],
    window_alpha: f32,
    bgr: bool,
) {
    let x0 = (buf_w - text_width(text)) / 2;
    draw_text(buf, buf_w, buf_h, x0, y0, text, color, window_alpha, bgr);
}
