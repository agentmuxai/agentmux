// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The browser's window fills the virtual screen, and keeps filling it when the screen changes size. Chrome's own `--start-maximized` does not
//! make the window manager's window maximised here, so a window that is not maximised stays the size it was; the maximise is the window
//! manager's own request (EWMH `_NET_WM_STATE`), the same one a person's maximise button makes. Nothing here reaches into the browser.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask};

use super::x::Screen;

/// `_NET_WM_STATE` actions.
const REMOVE: u32 = 0;
const ADD: u32 = 1;
/// The request comes from an ordinary application (a pager would say 2).
const FROM_APPLICATION: u32 = 1;
/// Windows smaller than this (a tooltip's frame, a hidden helper) are not the browser's window.
const MIN_WINDOW: u16 = 100;

impl Screen {
    fn atom(&self, name: &str) -> Result<Atom, String> {
        Ok(self.conn.intern_atom(false, name.as_bytes()).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom)
    }

    /// The windows the window manager manages (its `_NET_CLIENT_LIST`); none when there is no window manager or it has none yet.
    fn clients(&self) -> Result<Vec<u32>, String> {
        let list = self.atom("_NET_CLIENT_LIST")?;
        let reply = self.conn.get_property(false, self.root, list, AtomEnum::WINDOW, 0, u32::MAX).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        Ok(reply.value32().map(|v| v.collect()).unwrap_or_default())
    }

    fn is_maximized(&self, window: u32) -> Result<bool, String> {
        let state = self.atom("_NET_WM_STATE")?;
        let (vert, horz) = (self.atom("_NET_WM_STATE_MAXIMIZED_VERT")?, self.atom("_NET_WM_STATE_MAXIMIZED_HORZ")?);
        let reply = self.conn.get_property(false, window, state, AtomEnum::ATOM, 0, 64).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let set: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();
        Ok(set.contains(&vert) && set.contains(&horz))
    }

    /// Ask the window manager to make `window` maximised (or not): the message a pager or a maximise button sends to the root window.
    fn maximize(&self, window: u32, on: bool) -> Result<(), String> {
        let state = self.atom("_NET_WM_STATE")?;
        let (vert, horz) = (self.atom("_NET_WM_STATE_MAXIMIZED_VERT")?, self.atom("_NET_WM_STATE_MAXIMIZED_HORZ")?);
        let event = ClientMessageEvent::new(32, window, state, [if on { ADD } else { REMOVE }, vert, horz, FROM_APPLICATION, 0]);
        self.conn
            .send_event(false, self.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("the window manager could not be asked to maximise the window: {e}"))
    }

    /// Make the browser's window fill the screen: the biggest window the window manager manages, once there is one (waits up to `wait`). It
    /// stays maximised, and follows the screen's size (`refit`).
    pub fn maximize_first_within(&self, wait: Duration) -> Result<(), String> {
        let until = Instant::now() + wait;
        loop {
            let mut biggest: Option<(u32, u32)> = None;
            for window in self.clients()? {
                let Ok(g) = self.conn.get_geometry(window).map_err(|e| e.to_string()).and_then(|c| c.reply().map_err(|e| e.to_string())) else { continue };
                let area = u32::from(g.width) * u32::from(g.height);
                if g.width >= MIN_WINDOW && g.height >= MIN_WINDOW && biggest.map_or(true, |(_, a)| area > a) {
                    biggest = Some((window, area));
                }
            }
            if let Some((window, _)) = biggest {
                return self.maximize(window, true);
            }
            if Instant::now() >= until {
                return Err("the browser's window did not appear".to_string());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// After the screen changed size: every window that is maximised is made so again, so the window manager fits it to the new screen. A window
    /// that is not maximised (a pop-up) keeps its size.
    pub fn refit(&self) -> Result<(), String> {
        for window in self.clients()? {
            if self.is_maximized(window)? {
                self.maximize(window, false)?;
                self.maximize(window, true)?;
            }
        }
        Ok(())
    }
}
