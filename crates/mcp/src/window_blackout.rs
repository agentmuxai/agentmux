// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The blackout of a `UI` window capture of an AgentMux window
//! (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §7, G5).
//!
//! Each pane or panel showing private or secret content (the Vault, the Audit, Memory, a browser pane in the
//! person's own profile, and the panels listed in the app's `capture-mask.ts`) is blacked out by its rectangle, and
//! the rest of the window is captured. The rectangles and the frame come from the same instant: srv asks every window
//! for its rectangles just before the frame (`begin`) and just after (`end`); the capture is refused if the window
//! did not answer, if its layout moved or changed in between, if it says it can't give them, or if the frame's size
//! or scale differs from the one they were measured for. Nothing is ever blacked out from rectangles of another
//! moment.

use anyhow::Result;
use serde::Deserialize;

/// A rectangle in the window's viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq)]
pub(crate) struct MaskRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) w: f64,
    pub(crate) h: f64,
}

/// One window's answer, as srv's `POST /api/v1/ui/capture_mask` gives it.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub(crate) struct WindowMask {
    pub(crate) window_id: String,
    pub(crate) title: String,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) scale: f64,
    #[serde(default)]
    pub(crate) rects: Vec<MaskRect>,
    #[serde(default)]
    pub(crate) moving: bool,
    #[serde(default)]
    pub(crate) changed: bool,
    #[serde(default)]
    pub(crate) refuse: Option<String>,
}

/// What to do instead, said with every refusal.
const INSTEAD: &str = "try again in a moment, or use UI screenshot of your own pane";

/// What the host adds to a window's title on Linux when it runs without the sandbox
/// (`crates/cef/src/client/display.rs`); the page's own title, which the window reports, does not have it.
pub(crate) const UNSANDBOXED_SUFFIX: &str = " — Sandbox Disabled";

/// Whether the operating system's `title` for a window is the page title `page` the window reported.
fn same_title(title: &str, page: &str) -> bool {
    title == page || title.strip_suffix(UNSANDBOXED_SUFFIX) == Some(page)
}

/// Refuses when more than one of this AgentMux's own windows has the target's title (`same_title` counts the
/// windows on screen, the target included), whichever of them answered: an answer is found by its title.
pub(crate) fn unique_title(same_title: usize, title: &str) -> Result<()> {
    if same_title > 1 {
        anyhow::bail!(
            "{same_title} AgentMux windows have the title {title:?}, so their private panes can't be told apart and neither is captured: rename one, or use UI screenshot of your own pane"
        );
    }
    Ok(())
}

/// The answer for the window titled `title`, out of every window's answer to `begin`, if it may be captured now.
pub(crate) fn before<'a>(masks: &'a [WindowMask], title: &str) -> Result<&'a WindowMask> {
    let mine: Vec<&WindowMask> = masks.iter().filter(|m| same_title(title, &m.title)).collect();
    let mask = match mine.as_slice() {
        [] => anyhow::bail!("this AgentMux window did not give the rectangles of its private panes, so it is not captured: {INSTEAD}"),
        [one] => *one,
        _ => anyhow::bail!(
            "{} AgentMux windows have the title {title:?}, so their private panes can't be told apart and neither is captured: rename one, or use UI screenshot of your own pane",
            mine.len()
        ),
    };
    if let Some(why) = &mask.refuse {
        anyhow::bail!("this AgentMux window can't be captured now: {why}; {INSTEAD}");
    }
    if mask.moving {
        anyhow::bail!("this AgentMux window's layout is moving, so it is not captured: {INSTEAD}");
    }
    Ok(mask)
}

/// Whether the frame (`width` × `height` device pixels) may be blacked out by `begin`'s rectangles, given every
/// window's answer to `end`.
pub(crate) fn after(begin: &WindowMask, ends: &[WindowMask], width: u32, height: u32) -> Result<()> {
    let Some(end) = ends.iter().find(|m| m.window_id == begin.window_id) else {
        anyhow::bail!("this AgentMux window did not answer after the frame was taken, so it is not captured: {INSTEAD}");
    };
    if end.changed || end.moving || end.refuse.is_some() || end.rects != begin.rects {
        anyhow::bail!("this AgentMux window's layout changed while the frame was taken, so it is not captured: {INSTEAD}");
    }
    let same_geometry = end.width == begin.width && end.height == begin.height && end.scale == begin.scale;
    let near = |frame: u32, css: f64| (frame as f64 - css * begin.scale).abs() <= 2.0;
    if !same_geometry || !near(width, begin.width) || !near(height, begin.height) {
        anyhow::bail!(
            "the frame ({width}×{height}) is not the size or scale the rectangles were measured for ({:.0}×{:.0} at {}), so it is not captured: {INSTEAD}",
            begin.width * begin.scale,
            begin.height * begin.scale,
            begin.scale
        );
    }
    Ok(())
}

/// Black out each of `mask`'s rectangles in `image`, a pixel wider on every side so no edge of the content shows.
pub(crate) fn black_out(image: &mut image::RgbaImage, mask: &WindowMask) {
    let (w, h) = (image.width() as i64, image.height() as i64);
    for r in &mask.rects {
        let x0 = ((r.x * mask.scale).floor() as i64 - 1).clamp(0, w);
        let y0 = ((r.y * mask.scale).floor() as i64 - 1).clamp(0, h);
        let x1 = (((r.x + r.w) * mask.scale).ceil() as i64 + 1).clamp(0, w);
        let y1 = (((r.y + r.h) * mask.scale).ceil() as i64 + 1).clamp(0, h);
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x as u32, y as u32, image::Rgba([0, 0, 0, 255]));
            }
        }
    }
}

/// The refusal for an AgentMux window of another AgentMux, which this one can't ask for its rectangles.
pub(crate) const OTHER_INSTANCE: &str = "that window belongs to another AgentMux, which this one can't ask for the rectangles of its private panes, so it is not captured: capture it from an agent of that AgentMux, or use UI screenshot of your own pane";

/// The refusal for a container agent, which has no access to the host's screen.
pub(crate) const IN_CONTAINER: &str = "a container agent can't capture the host's windows: use UI screenshot of your own pane, or of a browser pane you opened";

#[cfg(test)]
#[path = "tests/window_blackout_tests.rs"]
mod tests;
