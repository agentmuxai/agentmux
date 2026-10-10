// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which renderer process serves which window or pane, for srv's Tower pane
//! (docs/reports/REPORT_PROCESS_ICONS_NAMES_AND_LABELS_2026_10_10.md §7).
//!
//! Chromium tells the browser process nothing about its renderers' PIDs, so
//! each renderer reports its own. [`SubprocessApp`] is the `CefApp` passed to
//! `execute_process` in subprocesses; its render-process handler sends
//! [`PID_MESSAGE`] with the renderer's PID when a browser is created in it.
//! The client records browser → PID ([`record`]) and drops it when the browser
//! closes ([`forget`]). srv asks for the map while a Tower pane is sampling
//! (`POST /agentmux/browser/renderer_map`), and [`snapshot`] answers from live
//! state, so nothing goes stale as windows are registered, pool windows are
//! promoted or panes come and go.
//!
//! The subprocess app implements nothing else, on purpose: the browser
//! process's `AgentMuxApp` rewrites command lines, which must not happen in
//! every subprocess. The browser process passes no app to `execute_process`,
//! as before.

use std::collections::HashMap;
use std::sync::OnceLock;

use cef::*;
use parking_lot::Mutex;
use serde::Serialize;

use crate::state::{AppState, BrowserPaneLifecycle};

/// Browser id → the PID of the renderer serving it. Process-wide: the host
/// has one `AppState`, and this is only ever read for srv's Tower pane.
fn pids() -> &'static Mutex<HashMap<i32, u32>> {
    static PIDS: OnceLock<Mutex<HashMap<i32, u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The process message a renderer sends with its PID.
pub const PID_MESSAGE: &str = "agentmux.renderer_pid";

wrap_app! {
    pub struct SubprocessApp;

    impl App {
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(PidReporter::new())
        }
    }
}

wrap_render_process_handler! {
    pub struct PidReporter;

    impl RenderProcessHandler {
        // Runs in the renderer, once for each browser created in it. On Linux
        // the sandbox's PID namespace makes this a namespace-local PID; srv
        // translates (tower_agentmux::own_view_pid).
        fn on_browser_created(&self, browser: Option<&mut Browser>, _extra_info: Option<&mut DictionaryValue>) {
            let Some(frame) = browser.and_then(|b| b.main_frame()) else { return };
            let Some(mut message) = process_message_create(Some(&CefString::from(PID_MESSAGE))) else { return };
            let Some(args) = message.argument_list() else { return };
            args.set_int(0, std::process::id() as i32);
            frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
        }
    }
}

/// The browser process's half: take a renderer's report. Returns whether
/// `message` was ours (handled, whatever its content).
pub fn record(
    browser: Option<&mut Browser>,
    source: ProcessId,
    message: Option<&mut ProcessMessage>,
) -> bool {
    let Some(message) = message else { return false };
    if CefString::from(&message.name()).to_string() != PID_MESSAGE {
        return false;
    }
    if source != ProcessId::RENDERER {
        return true;
    }
    let (Some(browser), Some(args)) = (browser, message.argument_list()) else { return true };
    if let Some(pid) = u32::try_from(args.int(0)).ok().filter(|&p| p > 0) {
        pids().lock().insert(browser.identifier(), pid);
    }
    true
}

/// A browser closed: its renderer no longer serves it.
pub fn forget(browser_id: i32) {
    pids().lock().remove(&browser_id);
}

/// One browser and the renderer serving it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RendererEntry {
    pub pid: u32,
    /// The browser's label (`main`, `window-pool-3`, a pane's own label, …).
    pub label: String,
    /// A browser pane's block id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    /// An unpromoted pool window: kept ready, not shown yet.
    pub pool: bool,
    /// An app window's srv window id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RendererMapData {
    pub renderers: Vec<RendererEntry>,
}

/// Every browser whose renderer has reported, and what that browser is.
pub fn snapshot(state: &AppState) -> Vec<RendererEntry> {
    let pids: HashMap<i32, u32> = pids().lock().clone();
    if pids.is_empty() {
        return Vec::new();
    }
    // Pool-side browsers: the unpromoted sets, the pool flag set at creation,
    // and the pane pool's label prefix (the same signals orphan_reconcile.rs
    // combines). One still counts as idle only while it has no srv window:
    // a promoted pool window is registered as a window.
    let mut pool = state.unpromoted_pool_labels_snapshot();
    pool.extend(state.pool_side_top_level_labels());
    // Each live browser pane's block id, by its browser's label.
    let panes: HashMap<String, String> = state
        .host_state
        .lock()
        .browser_panes
        .iter()
        .filter(|(_, e)| e.lifecycle == BrowserPaneLifecycle::Live)
        .map(|(block_id, e)| (e.label.clone(), block_id.clone()))
        .collect();
    let mut out: Vec<RendererEntry> = state
        .list_browsers()
        .into_iter()
        .filter_map(|(label, browser)| {
            let pid = *pids.get(&browser.identifier())?;
            let window_id = state.backend_window_id(&label);
            let pool_side = pool.contains(&label) || label.starts_with("floating-pool-");
            Some(RendererEntry {
                pid,
                block_id: panes.get(&label).cloned(),
                pool: pool_side && window_id.is_none(),
                window_id,
                label,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.pid, &a.label).cmp(&(b.pid, &b.label)));
    out
}
