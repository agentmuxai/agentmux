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

/// The renderers serving one browser: the one its browser object was created
/// in (its main frame's), and each frame's, which site isolation can put in
/// other processes (a cross-site iframe: ads, embeds, sign-in widgets).
/// SPEC_TOWER_PANES_VIEW_2026_10_21.md §6.1.
#[derive(Debug, Default, Clone, PartialEq)]
struct BrowserPids {
    main: Option<u32>,
    /// Frame identifier → the PID of the process its document runs in.
    frames: HashMap<String, u32>,
}

impl BrowserPids {
    /// Every PID serving the browser, the main one first, each once.
    fn all(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.main.into_iter().collect();
        let mut rest: Vec<u32> = self.frames.values().copied().filter(|p| Some(*p) != self.main).collect();
        rest.sort_unstable();
        rest.dedup();
        out.extend(rest);
        out
    }
}

/// What a renderer's message reports (`PID_MESSAGE`'s second argument).
const REPORT_BROWSER: i32 = 0;
const REPORT_FRAME: i32 = 1;
const REPORT_FRAME_GONE: i32 = 2;

/// Browser id → the renderers serving it. Process-wide: the host has one
/// `AppState`, and this is read for srv's Tower pane and the recycle governor.
fn pids() -> &'static Mutex<HashMap<i32, BrowserPids>> {
    static PIDS: OnceLock<Mutex<HashMap<i32, BrowserPids>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Apply one report to a browser's renderers.
fn apply_report(entry: &mut BrowserPids, kind: i32, pid: u32, frame: &str) {
    match kind {
        REPORT_FRAME if !frame.is_empty() => {
            entry.frames.insert(frame.to_string(), pid);
        }
        REPORT_FRAME_GONE => {
            entry.frames.remove(frame);
        }
        _ => entry.main = Some(pid),
    }
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
            let Some(mut frame) = browser.and_then(|b| b.main_frame()) else { return };
            send_report(&mut frame, REPORT_BROWSER);
        }

        // Runs for every frame's document, in whichever process hosts it: a
        // cross-site iframe's is another renderer, which Tower counts as the
        // page's too (SPEC_TOWER_PANES_VIEW_2026_10_21.md §6.1).
        fn on_context_created(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _context: Option<&mut V8Context>) {
            if let Some(frame) = frame {
                send_report(frame, REPORT_FRAME);
            }
        }

        fn on_context_released(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _context: Option<&mut V8Context>) {
            if let Some(frame) = frame {
                send_report(frame, REPORT_FRAME_GONE);
            }
        }
    }
}

/// A renderer's report, sent from `frame` to the browser process: this
/// process's PID, what it reports, and the frame's identifier.
fn send_report(frame: &mut Frame, kind: i32) {
    let Some(mut message) = process_message_create(Some(&CefString::from(PID_MESSAGE))) else { return };
    let Some(args) = message.argument_list() else { return };
    args.set_int(0, std::process::id() as i32);
    args.set_int(1, kind);
    args.set_string(2, Some(&CefString::from(&frame.identifier())));
    frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
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
        // An older renderer sends only the PID: its browser's main renderer.
        let kind = if args.size() > 1 { args.int(1) } else { REPORT_BROWSER };
        let frame = if args.size() > 2 { CefString::from(&args.string(2)).to_string() } else { String::new() };
        apply_report(pids().lock().entry(browser.identifier()).or_default(), kind, pid, &frame);
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
    /// The browser's own renderer (its main frame's), not one of its frames'.
    pub main: bool,
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
    let pids: HashMap<i32, BrowserPids> = pids().lock().clone();
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
        .flat_map(|(label, browser)| {
            let served = pids.get(&browser.identifier()).cloned().unwrap_or_default();
            let window_id = state.backend_window_id(&label);
            let pool_side = pool.contains(&label) || label.starts_with("floating-pool-");
            let block_id = panes.get(&label).cloned();
            // One entry per renderer serving the browser: its own, then each
            // of its frames' (SPEC_TOWER_PANES_VIEW_2026_10_21.md §6.1).
            served
                .all()
                .into_iter()
                .map(|pid| RendererEntry {
                    pid,
                    main: served.main.map_or(true, |m| m == pid),
                    block_id: block_id.clone(),
                    pool: pool_side && window_id.is_none(),
                    window_id: window_id.clone(),
                    label: label.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort_by(|a, b| (a.pid, &a.label).cmp(&(b.pid, &b.label)));
    out
}

/// Each browser pane's own memory in bytes, by block id: the private memory of the
/// renderers that serve that pane and no other browser, so it is what the pane frees
/// when its page goes (a renderer a pane shares with a window or another pane would
/// free nothing, and says nothing about the pane). Panes with no such renderer, or
/// whose renderers could not be measured, are left out. `only`, when not empty,
/// limits the answer to those block ids.
pub fn pane_memory(
    entries: &[RendererEntry],
    procs: &[agentmux_procstats::ProcInfo],
    only: &[String],
) -> HashMap<String, u64> {
    let mut labels_of: HashMap<u32, std::collections::HashSet<&str>> = HashMap::new();
    for e in entries {
        labels_of.entry(e.pid).or_default().insert(e.label.as_str());
    }
    let private: HashMap<u32, u64> = procs.iter().filter_map(|p| Some((p.pid, p.mem_private?))).collect();
    let mut counted: HashMap<&str, std::collections::HashSet<u32>> = HashMap::new();
    let mut out: HashMap<String, u64> = HashMap::new();
    for e in entries {
        let Some(block_id) = e.block_id.as_deref() else { continue };
        if !only.is_empty() && !only.iter().any(|b| b == block_id) {
            continue;
        }
        if labels_of.get(&e.pid).is_some_and(|labels| labels.len() > 1) {
            continue;
        }
        let Some(bytes) = private.get(&e.pid) else { continue };
        if counted.entry(block_id).or_default().insert(e.pid) {
            *out.entry(block_id.to_string()).or_default() += bytes;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, label: &str, block_id: Option<&str>) -> RendererEntry {
        RendererEntry {
            pid,
            label: label.into(),
            main: true,
            block_id: block_id.map(Into::into),
            pool: false,
            window_id: None,
        }
    }

    fn proc(pid: u32, private: Option<u64>) -> agentmux_procstats::ProcInfo {
        agentmux_procstats::ProcInfo {
            pid,
            ppid: None,
            start_key: 0,
            started_at_ms: None,
            name: "agentmux-cef".into(),
            cpu_ns: None,
            mem_private: private,
            mem_resident: None,
            mem_commit: None,
        }
    }

    #[test]
    fn a_pane_has_the_memory_of_its_own_renderers() {
        // Pane a: its page and a cross-site iframe in two processes. Pane b: one.
        let entries = [
            entry(10, "browser-pane-a-1", Some("a")),
            entry(11, "browser-pane-a-1", Some("a")),
            entry(20, "browser-pane-b-1", Some("b")),
        ];
        let procs = [proc(10, Some(100)), proc(11, Some(40)), proc(20, Some(70)), proc(99, Some(5))];
        let m = pane_memory(&entries, &procs, &[]);
        assert_eq!(m.get("a"), Some(&140));
        assert_eq!(m.get("b"), Some(&70));
    }

    #[test]
    fn a_renderer_shared_with_another_browser_is_nobodys() {
        // Pane a shares renderer 10 with the main window: closing a frees none of it.
        let entries = [
            entry(10, "browser-pane-a-1", Some("a")),
            entry(10, "main", None),
            entry(11, "browser-pane-a-1", Some("a")),
        ];
        let procs = [proc(10, Some(900)), proc(11, Some(40))];
        assert_eq!(pane_memory(&entries, &procs, &[]).get("a"), Some(&40));
        // Nothing of its own left: no figure, rather than a made-up one.
        assert!(pane_memory(&entries[..2], &procs, &[]).is_empty());
    }

    #[test]
    fn a_process_is_counted_once_and_an_unmeasured_one_not_at_all() {
        let entries = [
            entry(10, "browser-pane-a-1", Some("a")),
            entry(10, "browser-pane-a-1", Some("a")),
            entry(12, "browser-pane-c-1", Some("c")),
        ];
        let procs = [proc(10, Some(100)), proc(12, None)];
        let m = pane_memory(&entries, &procs, &[]);
        assert_eq!(m.get("a"), Some(&100));
        assert!(!m.contains_key("c"));
    }

    #[test]
    fn the_answer_can_be_limited_to_some_panes() {
        let entries = [entry(10, "browser-pane-a-1", Some("a")), entry(20, "browser-pane-b-1", Some("b"))];
        let procs = [proc(10, Some(100)), proc(20, Some(70))];
        let m = pane_memory(&entries, &procs, &["b".to_string()]);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("b"), Some(&70));
    }

    #[test]
    fn a_browser_keeps_its_own_renderer_and_each_frames() {
        let mut b = BrowserPids::default();
        apply_report(&mut b, REPORT_BROWSER, 10, "");
        apply_report(&mut b, REPORT_FRAME, 10, "main-frame");
        apply_report(&mut b, REPORT_FRAME, 20, "ad-frame");
        apply_report(&mut b, REPORT_FRAME, 30, "embed-frame");
        assert_eq!(b.all(), vec![10, 20, 30]);
        apply_report(&mut b, REPORT_FRAME_GONE, 20, "ad-frame");
        assert_eq!(b.all(), vec![10, 30]);
    }

    #[test]
    fn a_frame_moving_to_another_process_is_counted_once() {
        let mut b = BrowserPids::default();
        apply_report(&mut b, REPORT_BROWSER, 10, "");
        apply_report(&mut b, REPORT_FRAME, 20, "f");
        apply_report(&mut b, REPORT_FRAME, 30, "f");
        apply_report(&mut b, REPORT_FRAME, 30, "g");
        assert_eq!(b.all(), vec![10, 30]);
    }

    #[test]
    fn an_older_renderers_report_is_its_main_renderer() {
        let mut b = BrowserPids::default();
        apply_report(&mut b, REPORT_BROWSER, 7, "");
        assert_eq!(b.main, Some(7));
        assert_eq!(b.all(), vec![7]);
    }
}
