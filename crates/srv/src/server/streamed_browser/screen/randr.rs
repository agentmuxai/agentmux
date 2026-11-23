// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A person resizes the pane: the virtual screen follows (the RANDR extension), and the browser's maximised window with it. Xvfb can make
//! its screen smaller or bigger only up to the size it was started with, so a session starts the screen at `MAX_W` by `MAX_H` and sets it to
//! the pane's size at once, and every later resize is a new mode of that screen. Nothing here attaches to the browser.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::randr::{ConnectionExt as _, GetScreenResourcesCurrentReply, ModeFlag, ModeInfo, Rotation, SetConfig};
use x11rb::CURRENT_TIME;

use super::x::Screen;

/// The smallest side of a screen, and the largest screen a session can have (the size its Xvfb is started with).
pub const MIN_SIDE: u16 = 100;
pub const MAX_W: u16 = 4096;
pub const MAX_H: u16 = 2304;
/// The prefix of the names of the modes this makes, so the ones no longer used can be told from the screen's own.
const MODE_PREFIX: &str = "amx-";
/// How long the pane's size must stay the same before the screen is resized (a drag sends many sizes).
pub const SETTLE: Duration = Duration::from_millis(150);

/// A size asked for, cut to what a screen can be.
pub fn clamp(w: u32, h: u32) -> (u16, u16) {
    (w.clamp(u32::from(MIN_SIDE), u32::from(MAX_W)) as u16, h.clamp(u32::from(MIN_SIDE), u32::from(MAX_H)) as u16)
}

/// The size the screen is set to first, so that the screen's output (which is as big as the screen was) fits in it while it changes size:
/// big enough for both the old size and the new.
pub fn bridge(from: (u16, u16), to: (u16, u16)) -> (u16, u16) {
    (from.0.max(to.0), from.1.max(to.1))
}

/// The name of the mode for a size.
pub fn mode_name(w: u16, h: u16) -> String {
    format!("{MODE_PREFIX}{w}x{h}")
}

/// The millimetres a side of `pixels` is, at the 96 dots an inch the screen is made with.
pub fn millimetres(pixels: u16) -> u32 {
    u32::from(pixels) * 254 / 960
}

/// The last size the pane asked for, until it has stayed the same for `SETTLE`: then it is due.
#[derive(Default)]
pub struct Settle {
    wanted: Option<((u16, u16), Instant)>,
}

impl Settle {
    pub fn ask(&mut self, size: (u16, u16), now: Instant) {
        if !matches!(self.wanted, Some((s, _)) if s == size) {
            self.wanted = Some((size, now));
        }
    }

    /// When the wanted size is due, if there is one.
    pub fn due_at(&self) -> Option<Instant> {
        self.wanted.map(|(_, at)| at + SETTLE)
    }

    /// The wanted size once it is due; it is forgotten when taken.
    pub fn take(&mut self, now: Instant) -> Option<(u16, u16)> {
        match self.wanted {
            Some((size, at)) if now >= at + SETTLE => {
                self.wanted = None;
                Some(size)
            }
            _ => None,
        }
    }
}

impl Screen {
    /// Make the screen `w` by `h` (cut to what a screen can be). The windows on it follow if the window manager lets a maximised window follow.
    pub fn resize(&self, w: u16, h: u16) -> Result<(), String> {
        let (w, h) = clamp(u32::from(w), u32::from(h));
        let from = self.size();
        if from == (w, h) {
            return Ok(());
        }
        let x = |e: &dyn std::fmt::Display| format!("the screen could not be resized: {e}");
        self.conn.randr_query_version(1, 3).map_err(|e| x(&e))?.reply().map_err(|e| x(&e))?;
        let res = self.conn.randr_get_screen_resources_current(self.root).map_err(|e| x(&e))?.reply().map_err(|e| x(&e))?;
        let (Some(&crtc), Some(&output)) = (res.crtcs.first(), res.outputs.first()) else {
            return Err("the screen could not be resized: it has no output".to_string());
        };
        let mode = match found(&res, w, h) {
            Some(id) => id,
            None => self.make_mode(w, h)?,
        };
        self.conn.randr_add_output_mode(output, mode).map_err(|e| x(&e))?.check().map_err(|e| x(&e))?;
        let (bw, bh) = bridge(from, (w, h));
        if (bw, bh) != from {
            self.set_screen(bw, bh)?;
        }
        let set = self.conn.randr_set_crtc_config(crtc, CURRENT_TIME, res.config_timestamp, 0, 0, mode, Rotation::ROTATE0, &[output]).map_err(|e| x(&e))?.reply().map_err(|e| x(&e))?;
        if set.status != SetConfig::SUCCESS {
            return Err(format!("the screen could not be resized: the output refused the mode ({:?})", set.status));
        }
        self.set_screen(w, h)?;
        self.set_size(w, h);
        // The modes this made before and no longer shows are let go, so a long drag does not fill the screen's list.
        for old in ours(&res).filter(|&id| id != mode) {
            let _ = self.conn.randr_delete_output_mode(output, old);
            let _ = self.conn.randr_destroy_mode(old);
        }
        self.conn.flush().map_err(|e| x(&e))
    }

    fn set_screen(&self, w: u16, h: u16) -> Result<(), String> {
        self.conn
            .randr_set_screen_size(self.root, w, h, millimetres(w), millimetres(h))
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("the screen could not be resized: {e}"))
    }

    fn make_mode(&self, w: u16, h: u16) -> Result<u32, String> {
        let name = mode_name(w, h);
        // A mode with no timing of its own: a virtual screen has no monitor to drive.
        let info = ModeInfo { id: 0, width: w, height: h, dot_clock: 0, hsync_start: w, hsync_end: w, htotal: w, hskew: 0, vsync_start: h, vsync_end: h, vtotal: h, name_len: name.len() as u16, mode_flags: ModeFlag::from(0u32) };
        let made = self.conn.randr_create_mode(self.root, info, name.as_bytes()).map_err(|e| e.to_string())?.reply().map_err(|e| format!("the screen could not be resized: {e}"))?;
        Ok(made.mode)
    }

}

/// The id of the mode that is `w` by `h` and was made by this, if the screen has it.
fn found(res: &GetScreenResourcesCurrentReply, w: u16, h: u16) -> Option<u32> {
    let want = mode_name(w, h);
    named(res).find(|(_, name)| *name == want.as_bytes()).map(|(id, _)| id)
}

/// The modes this made.
fn ours(res: &GetScreenResourcesCurrentReply) -> impl Iterator<Item = u32> + '_ {
    named(res).filter(|(_, name)| name.starts_with(MODE_PREFIX.as_bytes())).map(|(id, _)| id)
}

/// Each mode's id and name (the names are one run of bytes, in the order of the modes).
fn named(res: &GetScreenResourcesCurrentReply) -> impl Iterator<Item = (u32, &[u8])> {
    let mut at = 0usize;
    res.modes.iter().map(move |m| {
        let end = (at + usize::from(m.name_len)).min(res.names.len());
        let name = &res.names[at.min(end)..end];
        at = end;
        (m.id, name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_cut_to_what_a_screen_can_be() {
        assert_eq!(clamp(10, 10), (MIN_SIDE, MIN_SIDE));
        assert_eq!(clamp(800, 600), (800, 600));
        assert_eq!(clamp(100_000, 100_000), (MAX_W, MAX_H));
    }

    #[test]
    fn the_screen_is_bridged_by_a_size_that_holds_both() {
        assert_eq!(bridge((800, 600), (1024, 400)), (1024, 600));
        assert_eq!(bridge((1280, 720), (640, 480)), (1280, 720));
    }

    #[test]
    fn a_mode_is_named_for_its_size_and_a_side_has_millimetres() {
        assert_eq!(mode_name(1280, 720), "amx-1280x720");
        assert_eq!(millimetres(960), 254);
    }

    #[test]
    fn a_size_is_due_when_it_has_stayed_the_same_for_the_settle_time() {
        let t0 = Instant::now();
        let mut s = Settle::default();
        assert!(s.take(t0).is_none() && s.due_at().is_none());
        s.ask((800, 600), t0);
        assert!(s.take(t0 + SETTLE / 2).is_none(), "not yet");
        // The same size asked again does not move the time; another size does.
        s.ask((800, 600), t0 + SETTLE / 2);
        assert_eq!(s.due_at(), Some(t0 + SETTLE));
        s.ask((900, 600), t0 + SETTLE / 2);
        assert!(s.take(t0 + SETTLE).is_none());
        assert_eq!(s.take(t0 + SETTLE / 2 + SETTLE), Some((900, 600)));
        assert!(s.take(t0 + SETTLE * 10).is_none(), "taken once");
    }
}
