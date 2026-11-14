// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A click in a Views pane's page on Windows, taken from the UI thread's message queue.
//!
//! A Views pane has no window of AgentMux's and no subclass (`browser_pane/hwnd.rs` is the child-window
//! kind's). Its page is a native `Chrome_RenderWidgetHostHWND` inside the app window, so a click goes
//! straight to the renderer and no code of ours sees it; the only thing that told the input owner was
//! CEF's `on_set_focus`, which Chromium sends only when the click changes which window has focus. A click
//! in a page the user is already reading did not, so the pane was not selected and the app kept the keys
//! (#47567, docs/reports/REPORT_BROWSER_PANE_TYPING_REGRESSION_RCA_2026_11_14.md).
//!
//! `watch`'s `WH_GETMESSAGE` hook sees every message the UI thread takes off its queue before it is
//! dispatched, so a button going down is looked at there: when it is inside a shown pane's page, outside
//! what is cut away of it for a menu, the pane is selected before the click lands, as the child-window
//! kind's subclass does (`views::pane_clicked`). The click is placed by the message's own coordinates,
//! not the sampled `MSG::pt`, so it is the click Chromium is about to handle.
//!
//! Mouse buttons (left, right, middle) and pointer messages (touch and pen) count. Keyboard-only
//! entry (Tab into a page) has no click: it still arrives by `on_set_focus`.

use std::sync::Arc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GA_ROOT, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_POINTERDOWN, WM_RBUTTONDOWN,
};

use crate::state::AppState;

/// `(x, y, width, height)` in the page's physical pixels, from the window's top-left corner.
type Rect = (i32, i32, i32, i32);

/// A pane of a window as far as a click is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneBox {
    pub block: String,
    pub rect: Rect,
    /// Drawn and hit-tested: not hidden whole for something over it.
    pub shown: bool,
}

/// Whether `message` is a button or a finger going down.
pub(crate) fn is_button_down(message: u32) -> bool {
    matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_POINTERDOWN)
}

fn inside(rect: Rect, x: i32, y: i32) -> bool {
    x >= rect.0 && x < rect.0 + rect.2 && y >= rect.1 && y < rect.1 + rect.3
}

/// The pane whose page is at (`x`, `y`): shown, and not under one of `holes` (the pieces of the app page
/// over the window, a menu or a tooltip, which the pane is cut around: the app page takes the click
/// there).
pub(crate) fn pane_hit<'a>(panes: &'a [PaneBox], holes: &[Rect], x: i32, y: i32) -> Option<&'a PaneBox> {
    if holes.iter().any(|h| h.2 > 0 && h.3 > 0 && inside(*h, x, y)) {
        return None;
    }
    panes.iter().find(|p| p.shown && p.rect.2 > 0 && p.rect.3 > 0 && inside(p.rect, x, y))
}

/// What a click is placed by: the window it is in, where its panes and the overlays over them are.
pub(crate) trait Place {
    /// The window the clicked `hwnd` is in, and where in that window's page (physical pixels) the click is.
    /// `lparam` is the message's: the point in `hwnd`'s client area for a mouse button, on the screen for a
    /// pointer message (`pointer`).
    fn page_point(&self, hwnd: isize, lparam: isize, pointer: bool) -> Option<(String, i32, i32)>;
    /// `window`'s Views panes.
    fn panes(&self, window: &str) -> Vec<PaneBox>;
    /// What of the app page is over `window`.
    fn holes(&self, window: &str) -> Vec<Rect>;
}

/// The pane of a window that a button going down at `lparam` of `hwnd` is in, with its window.
pub(crate) fn clicked_pane(place: &impl Place, message: u32, hwnd: isize, lparam: isize) -> Option<(String, String)> {
    if !is_button_down(message) {
        return None;
    }
    let (window, x, y) = place.page_point(hwnd, lparam, message == WM_POINTERDOWN)?;
    let panes = place.panes(&window);
    if panes.is_empty() {
        return None;
    }
    let holes = place.holes(&window);
    pane_hit(&panes, &holes, x, y).map(|p| (window, p.block.clone()))
}

/// A message the UI thread is about to dispatch. Called by `watch`'s hook; on the UI thread.
pub(crate) fn on_message(state: &Arc<AppState>, message: u32, hwnd: HWND, lparam: LPARAM) {
    if !is_button_down(message) {
        return;
    }
    if let Some((window, block)) = clicked_pane(&Real { state }, message, hwnd as isize, lparam) {
        tracing::info!("[input] native click in pane:{} of win={window}", &block[..block.len().min(8)]);
        super::views::pane_clicked(state, &block);
    }
}

/// The real window system and app state.
struct Real<'a> {
    state: &'a Arc<AppState>,
}

/// The signed low and high words of a message's `lParam`.
fn words(lparam: isize) -> (i32, i32) {
    ((lparam & 0xFFFF) as u16 as i16 as i32, ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32)
}

impl Place for Real<'_> {
    fn page_point(&self, hwnd: isize, lparam: isize, pointer: bool) -> Option<(String, i32, i32)> {
        let hwnd = hwnd as HWND;
        if hwnd.is_null() {
            return None;
        }
        let (x, y) = words(lparam);
        let mut pt = POINT { x, y };
        // SAFETY: plain window-system calls on a handle the message named; they fail on a stale one.
        let root = unsafe {
            if !pointer && ClientToScreen(hwnd, &mut pt) == 0 {
                return None;
            }
            GetAncestor(hwnd, GA_ROOT)
        };
        let window = self.state.label_for_hwnd(root)?;
        // SAFETY: as above; `root` is the window of `window`.
        if unsafe { ScreenToClient(root, &mut pt) } == 0 {
            return None;
        }
        Some((window, pt.x, pt.y))
    }

    fn panes(&self, window: &str) -> Vec<PaneBox> {
        use cef::ImplOverlayController;
        // One lock at a time: none is held across another.
        let overlays: Vec<(String, bool)> = self
            .state
            .browser_pane_overlays
            .lock()
            .iter()
            .filter(|(_, (w, _))| w == window)
            .map(|(label, (_, controller))| (label.clone(), controller.is_visible() != 0))
            .collect();
        let rects = self.state.browser_pane_physical_rects.lock();
        overlays
            .into_iter()
            .filter_map(|(label, shown)| {
                let rect = *rects.get(&label)?;
                let block = label.strip_prefix("browser-pane-")?.rsplit_once('-')?.0.to_string();
                Some(PaneBox { block, rect, shown })
            })
            .collect()
    }

    fn holes(&self, window: &str) -> Vec<Rect> {
        self.state.pane_overlay_rects.lock().get(window).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
#[path = "native_click_tests.rs"]
mod tests;
