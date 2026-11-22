// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The virtual X screen: its pixels read out, and pointer and key events sent to it at screen level (the XTEST extension: the same
//! events a person's mouse and keyboard make). Nothing here attaches to the browser; the browser is a program on a screen.

use x11rb::connection::Connection;
use x11rb::protocol::damage::{ConnectionExt as _, ReportLevel};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt as _, EventMask, ImageFormat, MapState, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::frames::Rect;

pub struct Screen {
    conn: RustConnection,
    root: u32,
    pub w: u16,
    pub h: u16,
}

impl Screen {
    /// Connect to display `display` (`:12`).
    pub fn connect(display: &str) -> Result<Self, String> {
        let (conn, n) = RustConnection::connect(Some(display)).map_err(|e| format!("cannot reach the virtual screen {display}: {e}"))?;
        let s = &conn.setup().roots[n];
        let (root, w, h) = (s.root, s.width_in_pixels, s.height_in_pixels);
        Ok(Self { conn, root, w, h })
    }

    /// The whole screen: four bytes a pixel (blue, green, red, one unused), row after row.
    pub fn grab(&self) -> Result<Vec<u8>, String> {
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, 0, 0, self.w, self.h, u32::MAX)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("the screen could not be read: {e}"))?;
        Ok(reply.data)
    }

    /// A part of the screen, as `grab` gives the whole of it.
    pub fn grab_rect(&self, at: Rect) -> Result<Vec<u8>, String> {
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, at.x as i16, at.y as i16, at.w, at.h, u32::MAX)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("the screen could not be read: {e}"))?;
        Ok(reply.data)
    }

    /// Ask the screen to say which part of it changed (the damage extension), so the screen is read when it changed and only where. The
    /// answer is the damage's id, for `clear_damage`.
    pub fn watch_damage(&self) -> Result<u32, String> {
        self.conn.damage_query_version(1, 1).map_err(|e| e.to_string())?.reply().map_err(|e| format!("the screen has no damage extension: {e}"))?;
        let id = self.conn.generate_id().map_err(|e| e.to_string())?;
        self.conn.damage_create(id, self.root, ReportLevel::RAW_RECTANGLES).map_err(|e| e.to_string())?.check().map_err(|e| format!("the screen's changes could not be followed: {e}"))?;
        Ok(id)
    }

    /// Wait until some part of the screen changes, and say which part (cut to the screen). An error when the screen has gone.
    pub fn wait_damage(&self) -> Result<Rect, String> {
        loop {
            let event = self.conn.wait_for_event().map_err(|e| format!("the screen has gone: {e}"))?;
            if let Some(at) = self.damaged(&event) {
                return Ok(at);
            }
        }
    }

    /// The parts that changed since the last look, as one rectangle that holds them all; none when nothing did.
    pub fn take_damage(&self) -> Result<Option<Rect>, String> {
        let mut all: Option<Rect> = None;
        while let Some(event) = self.conn.poll_for_event().map_err(|e| format!("the screen has gone: {e}"))? {
            if let Some(at) = self.damaged(&event) {
                all = Some(all.map_or(at, |a| a.union(at)));
            }
        }
        Ok(all)
    }

    /// Tell the screen the changes so far are taken in (it forgets the region it has kept for them).
    pub fn clear_damage(&self, id: u32) -> Result<(), String> {
        self.conn.damage_subtract(id, x11rb::NONE, x11rb::NONE).map_err(|e| e.to_string())?;
        self.conn.flush().map_err(|e| e.to_string())
    }

    fn damaged(&self, event: &Event) -> Option<Rect> {
        let Event::DamageNotify(d) = event else { return None };
        Rect::clipped(d.area.x, d.area.y, d.area.width, d.area.height, self.w, self.h)
    }

    /// How many top-level windows a person would see: shown, not a menu or tooltip (override-redirect), and bigger than 100 by 100.
    pub fn visible_windows(&self) -> Result<usize, String> {
        let tree = self.conn.query_tree(self.root).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let mut shown = 0;
        for window in tree.children {
            let Ok(attrs) = self.conn.get_window_attributes(window).map_err(|e| e.to_string())?.reply() else { continue };
            if attrs.map_state != MapState::VIEWABLE || attrs.override_redirect {
                continue;
            }
            if let Ok(geometry) = self.conn.get_geometry(window).map_err(|e| e.to_string())?.reply() {
                if geometry.width >= 100 && geometry.height >= 100 {
                    shown += 1;
                }
            }
        }
        Ok(shown)
    }

    /// Ask the screen to say when a top-level window is mapped, unmapped or destroyed (events on the root window), so a window opening is known
    /// at the moment it opens and not at the next look.
    pub fn follow_windows(&self) -> Result<(), String> {
        let attrs = ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY);
        self.conn.change_window_attributes(self.root, &attrs).map_err(|e| e.to_string())?.check().map_err(|e| format!("the screen's window events could not be followed: {e}"))
    }

    /// Wait for the next event about a top-level window (after `follow_windows`). An error when the screen has gone.
    pub fn next_window_event(&self) -> Result<(), String> {
        self.conn.wait_for_event().map(|_| ()).map_err(|e| format!("the screen has gone: {e}"))
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<(), String> {
        self.conn.xtest_fake_input(kind, detail, 0, self.root, x, y, 0).map_err(|e| e.to_string())?;
        self.conn.flush().map_err(|e| e.to_string())
    }

    pub fn move_to(&self, x: f64, y: f64) -> Result<(), String> {
        let (x, y) = (x.round().clamp(0.0, f64::from(self.w) - 1.0) as i16, y.round().clamp(0.0, f64::from(self.h) - 1.0) as i16);
        self.fake(MOTION_NOTIFY_EVENT, 0, x, y)
    }

    /// Button 1 left, 2 middle, 3 right, 4 and 5 wheel up and down, 6 and 7 wheel left and right.
    pub fn button(&self, button: u8, down: bool) -> Result<(), String> {
        self.fake(if down { BUTTON_PRESS_EVENT } else { BUTTON_RELEASE_EVENT }, button, 0, 0)
    }

    pub fn key(&self, keycode: u8, down: bool) -> Result<(), String> {
        self.fake(if down { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT }, keycode, 0, 0)
    }
}
