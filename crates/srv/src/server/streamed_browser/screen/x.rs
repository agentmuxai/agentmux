// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The virtual X screen: its pixels read out, and pointer and key events sent to it at screen level (the XTEST extension: the same
//! events a person's mouse and keyboard make). Nothing here attaches to the browser; the browser is a program on a screen.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

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
