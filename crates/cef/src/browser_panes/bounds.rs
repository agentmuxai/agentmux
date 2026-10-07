// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browser_panes_set_rects`: move a window's panes in one step.
//!
//! The frontend used to send one `browser_pane_resize` per pane per frame, and
//! each was applied on its own by a cross-thread `SetWindowPos` from the IPC
//! thread that waited for the UI thread. A window drag over nine panes queued
//! up to 111 requests, so panes trailed the page by about half a second
//! (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md). Now the frontend
//! sends one batch per window with at most one in flight
//! (`frontend/app/view/browser/pane-rect-batcher.ts`), and here:
//!
//! - A batch merges into the window's pending rects (a pane's newer rect
//!   replaces its older one) and one UI task is scheduled for the window if
//!   none is waiting. Batches can't pile up on the UI thread.
//! - That task takes everything pending and applies it: on Windows every
//!   pane's wrapper moves in one `BeginDeferWindowPos`/`EndDeferWindowPos`
//!   per parent window, so they move together; elsewhere each pane goes
//!   through its usual Views resize, all in the one task.
//! - The IPC call returns once its rects have been applied, or after
//!   `APPLY_WAIT` if the UI thread is busy, so the page's next batch follows
//!   the host's pace without ever waiting on a stuck UI thread.
//! - A pane is moved only while it is live and still in the window that sent
//!   the batch: a rect from a window a pane was torn off from is dropped.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use cef::*;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::state::AppState;

use super::BrowserPaneManager;

/// How long `set_rects` waits for its rects to be applied before answering anyway.
const APPLY_WAIT: Duration = Duration::from_millis(250);

/// One pane's new rect, in the window's physical client pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneBounds {
    pub block_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PaneBounds {
    fn rect(&self) -> Rect {
        Rect { x: self.x, y: self.y, width: self.width, height: self.height }
    }
}

/// Parse `{ window_label, rects: [{ block_id, x, y, width, height }] }`.
pub(crate) fn parse_set_rects_args(args: &serde_json::Value) -> Result<(String, Vec<PaneBounds>), String> {
    let window_label = args
        .get("window_label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("browser_panes_set_rects: missing window_label")?
        .to_string();
    let rects = args
        .get("rects")
        .and_then(|v| v.as_array())
        .ok_or("browser_panes_set_rects: missing rects")?;
    let int = |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let mut out = Vec::with_capacity(rects.len());
    for r in rects {
        let Some(block_id) = r.get("block_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(PaneBounds {
            block_id: block_id.to_string(),
            x: int(r, "x"),
            y: int(r, "y"),
            width: int(r, "width"),
            height: int(r, "height"),
        });
    }
    Ok((window_label, out))
}

/// A window's rects waiting for the UI thread, and who is waiting on them.
#[derive(Default)]
struct Pending {
    /// In first-arrival order; a newer rect for a pane replaces its entry in place.
    rects: Vec<PaneBounds>,
    waiters: Vec<oneshot::Sender<()>>,
    /// A UI task for this window is posted and hasn't run yet.
    scheduled: bool,
}

#[derive(Default)]
struct PendingByWindow(HashMap<String, Pending>);

impl PendingByWindow {
    /// Merge `rects` into `window`'s pending set. Returns the receiver that
    /// fires once they are applied, and whether the caller must post a task.
    fn enqueue(&mut self, window: &str, rects: Vec<PaneBounds>) -> (oneshot::Receiver<()>, bool) {
        let p = self.0.entry(window.to_string()).or_default();
        for r in rects {
            match p.rects.iter_mut().find(|e| e.block_id == r.block_id) {
                Some(e) => *e = r,
                None => p.rects.push(r),
            }
        }
        let (tx, rx) = oneshot::channel();
        p.waiters.push(tx);
        let post = !p.scheduled;
        p.scheduled = true;
        (rx, post)
    }

    /// Everything pending for `window`; a later batch schedules a new task.
    fn take(&mut self, window: &str) -> Pending {
        self.0.remove(window).unwrap_or_default()
    }
}

static PENDING: LazyLock<Mutex<PendingByWindow>> = LazyLock::new(Default::default);

impl BrowserPaneManager {
    /// Move `window_label`'s panes to `rects` in one UI-thread step. Resolves
    /// when they have moved, or after `APPLY_WAIT`.
    pub async fn set_rects(&self, state: &Arc<AppState>, window_label: &str, rects: Vec<PaneBounds>) {
        if rects.is_empty() {
            return;
        }
        let (applied, post) = PENDING.lock().enqueue(window_label, rects);
        if post {
            let mut task = ApplyPaneBoundsTask::new(state.clone(), window_label.to_string());
            if post_task(ThreadId::UI, Some(&mut task)) == 0 {
                // No UI thread to run it (shutting down): drop what's pending
                // so the window isn't left marked scheduled with no task, and
                // answer every waiter now.
                tracing::warn!(window = window_label, "[pane-bounds] post_task failed; dropping pending rects");
                for w in PENDING.lock().take(window_label).waiters {
                    let _ = w.send(());
                }
            }
        }
        let _ = tokio::time::timeout(APPLY_WAIT, applied).await;
    }
}

wrap_task! {
    pub struct ApplyPaneBoundsTask {
        state: Arc<AppState>,
        window_label: String,
    }

    impl Task {
        fn execute(&self) {
            let pending = PENDING.lock().take(&self.window_label);
            apply(&self.state, &self.window_label, &pending.rects);
            for w in pending.waiters {
                let _ = w.send(());
            }
        }
    }
}

/// Apply on the UI thread: only live panes that still belong to `window_label`.
fn apply(state: &Arc<AppState>, window_label: &str, rects: &[PaneBounds]) {
    let live: Vec<(&PaneBounds, String)> = rects
        .iter()
        .filter(|r| state.browser_pane_window_label(&r.block_id).as_deref() == Some(window_label))
        .filter_map(|r| state.live_browser_pane_label(&r.block_id).map(|label| (r, label)))
        .collect();
    if live.is_empty() {
        return;
    }
    #[cfg(target_os = "windows")]
    apply_windows(state, &live);
    #[cfg(not(target_os = "windows"))]
    for (r, label) in &live {
        crate::browser_pane::creation_views::resize_browser_pane_view(state, label, r.rect());
    }
}

/// Every pane's wrapper HWND in one deferred move per parent, with the same
/// flags `wrapper::resize_wrapper` uses (`SWP_NOACTIVATE`, insert-after null =
/// top, so a moved pane is raised as before). Each wrapper's own `WM_SIZE`
/// then sizes CEF's HWND inside it.
#[cfg(target_os = "windows")]
fn apply_windows(state: &Arc<AppState>, live: &[(&PaneBounds, String)]) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GetParent, SWP_NOACTIVATE,
    };

    let started = std::time::Instant::now();
    let mut by_parent: Vec<(isize, Vec<(isize, Rect)>)> = Vec::new();
    let mut moved: Vec<&str> = Vec::new();
    for (r, label) in live {
        let Some(wrapper) = crate::browser_pane::wrapper::peek_wrapper_hwnd(label) else { continue };
        let parent = unsafe { GetParent(wrapper as *mut std::ffi::c_void) } as isize;
        match by_parent.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, v)) => v.push((wrapper, r.rect())),
            None => by_parent.push((parent, vec![(wrapper, r.rect())])),
        }
        moved.push(label);
    }

    for (_, wrappers) in &by_parent {
        let mut hdwp = unsafe { BeginDeferWindowPos(wrappers.len() as i32) };
        for (hwnd, rect) in wrappers {
            if hdwp.is_null() {
                break;
            }
            hdwp = unsafe {
                DeferWindowPos(
                    hdwp,
                    *hwnd as *mut std::ffi::c_void,
                    std::ptr::null_mut(),
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    SWP_NOACTIVATE,
                )
            };
        }
        // A failed DeferWindowPos frees the batch and returns null: move each
        // wrapper on its own instead, which is what this replaced.
        if hdwp.is_null() || unsafe { EndDeferWindowPos(hdwp) } == 0 {
            tracing::warn!(panes = wrappers.len(), "[pane-bounds] deferred move failed; moving one by one");
            for (hwnd, rect) in wrappers {
                crate::browser_pane::wrapper::resize_wrapper(*hwnd as *mut std::ffi::c_void, rect);
            }
        }
    }

    tracing::debug!(panes = moved.len(), ms = started.elapsed().as_secs_f64() * 1000.0, "[pane-bounds] moved");
    for label in moved {
        if let Some(host) = state.get_browser(label).and_then(|b| b.host()) {
            host.notify_move_or_resize_started();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(id: &str, x: i32) -> PaneBounds {
        PaneBounds { block_id: id.to_string(), x, y: 0, width: 100, height: 50 }
    }

    #[test]
    fn parses_a_batch_and_skips_entries_without_a_block_id() {
        let args = serde_json::json!({
            "window_label": "main",
            "rects": [
                { "block_id": "a", "x": 1, "y": 2, "width": 3, "height": 4 },
                { "x": 9 },
                { "block_id": "", "x": 9 },
            ],
        });
        let (window, rects) = parse_set_rects_args(&args).unwrap();
        assert_eq!(window, "main");
        assert_eq!(rects, vec![PaneBounds { block_id: "a".into(), x: 1, y: 2, width: 3, height: 4 }]);
    }

    #[test]
    fn rejects_a_batch_without_a_window_or_rects() {
        assert!(parse_set_rects_args(&serde_json::json!({ "rects": [] })).is_err());
        assert!(parse_set_rects_args(&serde_json::json!({ "window_label": "main" })).is_err());
    }

    #[test]
    fn posts_one_task_per_window_until_it_runs() {
        let mut p = PendingByWindow::default();
        let (_a, post_a) = p.enqueue("main", vec![b("x", 1)]);
        let (_b, post_b) = p.enqueue("main", vec![b("y", 2)]);
        let (_c, post_c) = p.enqueue("floating-1", vec![b("z", 3)]);
        assert!(post_a);
        assert!(!post_b);
        assert!(post_c);
        p.take("main");
        let (_d, post_d) = p.enqueue("main", vec![b("x", 4)]);
        assert!(post_d);
    }

    #[test]
    fn a_newer_rect_replaces_a_panes_pending_one_in_place() {
        let mut p = PendingByWindow::default();
        let _ = p.enqueue("main", vec![b("x", 1), b("y", 2)]);
        let _ = p.enqueue("main", vec![b("x", 9)]);
        let taken = p.take("main");
        assert_eq!(taken.rects, vec![b("x", 9), b("y", 2)]);
        assert_eq!(taken.waiters.len(), 2);
    }

    #[test]
    fn taking_wakes_every_waiter() {
        let mut p = PendingByWindow::default();
        let (mut rx1, _) = p.enqueue("main", vec![b("x", 1)]);
        let (mut rx2, _) = p.enqueue("main", vec![b("y", 2)]);
        for w in p.take("main").waiters {
            w.send(()).unwrap();
        }
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }
}
