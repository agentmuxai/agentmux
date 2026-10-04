// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32-native unowned popup creator for floating panes. Phase 1 of
//! issue #810 (floating-pane tear-off).
//!
//! This module is intentionally Windows-only and intentionally
//! separate from `crate::ui_tasks` (which posts the standard CEF Views
//! top-level windows). A floating pane is a *raw* `WS_POPUP` HWND with
//! `WS_EX_TOOLWINDOW` and **no Win32 owner** (null parent). CEF Views
//! does not expose tool-window semantics, so we drop down to
//! `CreateWindowExW` and embed a CEF browser inside the resulting HWND
//! via `WindowInfo::set_as_child` — the same mechanism the browser-pane
//! creation path uses (`browser_pane/creation.rs`).
//!
//! ## Why no owner (issue #1560)
//!
//! The original design used the main window as the Win32 owner, which
//! gave minimize/restore/destroy cascade for free. However, Win32's
//! owned-window invariant — owned windows are ALWAYS z-above their owner
//! and cannot be pushed below with `SetWindowPos` — meant the floater
//! was permanently stuck above the main window even after the user
//! activated the main window. The fix is to remove the owner and handle
//! the cascade explicitly:
//!
//! - **Taskbar/Alt+Tab hiding**: `WS_EX_TOOLWINDOW` (already set, not
//!   ownership-derived).
//! - **Minimize/restore cascade**: handled by
//!   `install_main_window_floater_cascade_hook` in `client/wndproc.rs`,
//!   which intercepts `WM_SIZE` on the main window and
//!   shows/hides all registered floaters.
//! - **Destroy cascade**: same hook intercepts `WM_DESTROY`.
//! - **Z-order on activation**: the hook intercepts `WM_ACTIVATE` on the
//!   main window and calls `SetWindowPos(floater, main_hwnd, ...)` to
//!   place each floater below main, so clicking main brings it to front.
//!
//! See issue #810 / `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`
//! for the full design and phase plan.
//!
//! ## Scope of Phase 1
//!
//! - Register the IPC command (`open_floating_pane_window`).
//! - Allocate a stable window label.
//! - Create the owned `WS_POPUP | WS_EX_TOOLWINDOW` HWND.
//! - Embed a CEF browser inside it via `WindowInfo::set_as_child`.
//! - Browser loads `<frontend>?floatingPaneId=<id>&windowLabel=<lbl>`.
//!
//! ## Out of scope for Phase 1 (per spec §9)
//!
//! - Drag-to-tear-off wiring (Phase 3).
//! - Floating-pane frontend shell that renders the full `<Block>`
//!   (Phase 2). Phase 1's stub shell renders only a placeholder so the
//!   primitive can be validated end-to-end.
//! - Re-dock (Phase 4).
//! - Persistence (Phase 5).
//! - macOS / Linux ports (deferred).

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use cef::*;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Global floater HWND registry — used by the main-window cascade hook to
// minimize/restore/destroy/z-order floaters without the owned-window
// invariant. Keyed by window label ("floating-<uuid>"); value is
// (floater_hwnd, parent_main_hwnd), both as isize (Send-safe).
// The parent_main_hwnd binding lets the cascade hook in wndproc.rs filter
// to ONLY the floaters that belong to the window sending the message —
// prevents closing a secondary full window from destroying floaters that
// belong to a different window. Populated at floater creation, removed on
// WM_DESTROY in floating_pane_wndproc.
// ---------------------------------------------------------------------------
static ACTIVE_FLOATER_HWNDS: LazyLock<Mutex<HashMap<String, (isize, isize)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register a floater. `parent_hwnd` is the HWND of the FullInstance main
/// window that spawned this floater — used by the cascade hook to restrict
/// min/restore/destroy operations to only that window's floaters.
pub(crate) fn register_floater_hwnd(label: String, floater_hwnd: isize, parent_hwnd: isize) {
    if let Ok(mut m) = ACTIVE_FLOATER_HWNDS.lock() {
        m.insert(label, (floater_hwnd, parent_hwnd));
    }
}

/// Called from `floating_pane_wndproc` on WM_DESTROY; reverse-scans by
/// floater HWND value since the wndproc doesn't have the label in scope.
pub(crate) fn unregister_floater_by_hwnd(hwnd: isize) {
    if let Ok(mut m) = ACTIVE_FLOATER_HWNDS.lock() {
        m.retain(|_, (fh, _)| *fh != hwnd);
    }
}

// `floater_hwnds_for_parent` removed: the window→floater cascade it fed was
// deleted in favour of full floater independence (see the FLOATER INDEPENDENCE
// note in `client/wndproc.rs::floater_cascade_wndproc`). The registry below is
// retained for the diagnostic snapshot only.

/// Snapshot for the `get_pane_debug_state` diagnostic command.
pub(crate) fn floater_debug_snapshot() -> Vec<(String, isize)> {
    ACTIVE_FLOATER_HWNDS
        .lock()
        .map(|m| m.iter().map(|(l, (fh, _))| (l.clone(), *fh)).collect())
        .unwrap_or_default()
}

/// Resolve a floater's HWND by its window label ("floating-<uuid>").
///
/// Floaters are raw `WS_POPUP` HWNDs with the CEF browser embedded as a
/// child — they have no CEF Views `Window`, so any label-addressed window
/// operation (focus, opacity) must branch to Win32 via this lookup instead
/// of `get_window_on_ui` (whose `browser_view.window()` returns None for
/// floaters and silently no-ops). Spec:
/// docs/specs/instance-panel-floating-panes.md §2.
pub(crate) fn floater_hwnd_for_label(label: &str) -> Option<isize> {
    ACTIVE_FLOATER_HWNDS
        .lock()
        .ok()
        .and_then(|m| m.get(label).map(|(fh, _)| *fh))
}

// ---------------------------------------------------------------------------
// "Always on top" (the header tack) — SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27.
//
// A tacked floater is HWND_TOPMOST only while this process is the active app:
// that keeps it above every window of this instance without floating over
// other applications (or another AgentMux instance). `floating_pane_wndproc`
// re-stacks on WM_ACTIVATEAPP. Tacked floaters are ordinary peers of each
// other inside the topmost band, and every re-stack preserves their existing
// relative order (Codex P2 on #3970).
// ---------------------------------------------------------------------------

/// Outer HWNDs of tacked floaters. Read on the UI thread in the wndproc, so
/// keyed by HWND (the wndproc has no label in scope).
static TACKED_FLOATERS: LazyLock<Mutex<std::collections::HashSet<isize>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// Windows that must stay ABOVE tacked floaters — the approval pages
/// (credential / memory-adoption), label → top-level HWND. They are made
/// topmost only while tacked floaters are (`keep_above_should_be_topmost`),
/// and re-raised over them on every re-stack: topmost windows stack by the
/// most recent assertion, so without that an app switch back would put a
/// floater over a security prompt (ReAgent P1 on #3970). Outside that window
/// they are ordinary windows — never desktop-topmost (Codex P2 on #3970).
static KEEP_ABOVE_FLOATERS: LazyLock<Mutex<HashMap<String, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Open host file dialogs (nesting count). While > 0, tacked floaters step
/// down to the normal band so a picker can't end up behind one (spec §3.1).
static HOST_DIALOGS_OPEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What changed, for `z_action`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZEvent {
    /// The user toggled the tack (or the frontend re-applied a stored value).
    TackToggled,
    /// This process became the active app (WM_ACTIVATEAPP, wParam TRUE).
    AppActivated,
    /// Another process became the active app (WM_ACTIVATEAPP, wParam FALSE).
    AppDeactivated,
    /// A host file dialog opened / the last one closed.
    DialogOpened,
    DialogClosed,
}

/// The z-order change to apply to one floater.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZAction {
    None,
    /// `HWND_TOPMOST`.
    MakeTopmost,
    /// Leave the topmost band, under the newly active app's window if we can
    /// identify it (see `apply_z_action`).
    DropBelowForeground,
    /// `HWND_NOTOPMOST`: leave the topmost band, keep current position.
    DropToNormal,
}

/// Pure decision (spec §3): given the event and the floater's situation, what
/// should happen to its z-order.
pub(crate) fn z_action(event: ZEvent, tacked: bool, app_active: bool, dialog_open: bool) -> ZAction {
    match event {
        ZEvent::TackToggled if !tacked => ZAction::DropToNormal,
        ZEvent::TackToggled | ZEvent::AppActivated | ZEvent::DialogClosed => {
            if tacked && app_active && !dialog_open {
                ZAction::MakeTopmost
            } else {
                ZAction::None
            }
        }
        ZEvent::AppDeactivated if tacked => ZAction::DropBelowForeground,
        ZEvent::DialogOpened if tacked => ZAction::DropToNormal,
        ZEvent::AppDeactivated | ZEvent::DialogOpened => ZAction::None,
    }
}

/// Pure: should the keep-above windows (approval pages) be topmost right now?
/// Exactly while tacked floaters are topmost — otherwise they'd float over
/// other applications for no reason.
pub(crate) fn keep_above_should_be_topmost(any_tacked: bool, app_active: bool, dialog_open: bool) -> bool {
    any_tacked && app_active && !dialog_open
}

/// Pure: the order to re-stack tacked floaters in so their relative z-order
/// survives. `z_top_to_bottom` is the current window z-order (topmost first);
/// the result is the tacked ones, BOTTOM first. Every insert — `HWND_TOPMOST`,
/// `HWND_NOTOPMOST`, or "directly under window X" — lands the window above the
/// previous one from the same pass, so bottom-first reproduces the order.
pub(crate) fn restack_order(
    z_top_to_bottom: &[isize],
    tacked: &std::collections::HashSet<isize>,
) -> Vec<isize> {
    z_top_to_bottom.iter().rev().copied().filter(|h| tacked.contains(h)).collect()
}

/// Record a floater's tack state (by outer HWND).
pub(crate) fn set_floater_tacked(hwnd: isize, on: bool) {
    if let Ok(mut s) = TACKED_FLOATERS.lock() {
        if on {
            s.insert(hwnd);
        } else {
            s.remove(&hwnd);
        }
    }
}

fn is_floater_tacked(hwnd: isize) -> bool {
    TACKED_FLOATERS.lock().map(|s| s.contains(&hwnd)).unwrap_or(false)
}

fn tacked_snapshot() -> std::collections::HashSet<isize> {
    TACKED_FLOATERS.lock().map(|s| s.clone()).unwrap_or_default()
}

fn host_dialog_open() -> bool {
    HOST_DIALOGS_OPEN.load(std::sync::atomic::Ordering::SeqCst) > 0
}

/// Register a window that must never be covered by a tacked floater (an
/// approval page), and bring it above them now if they are on top.
#[cfg(target_os = "windows")]
pub(crate) fn register_keep_above_floaters(label: String, hwnd: isize) {
    if let Ok(mut m) = KEEP_ABOVE_FLOATERS.lock() {
        m.insert(label, hwnd);
    }
    sync_keep_above(this_app_is_active(), false);
}

/// Forget it (window closing). No-op if absent.
pub(crate) fn unregister_keep_above_floaters(label: &str) {
    if let Ok(mut m) = KEEP_ABOVE_FLOATERS.lock() {
        m.remove(label);
    }
}

fn keep_above_hwnds() -> Vec<isize> {
    KEEP_ABOVE_FLOATERS.lock().map(|m| m.values().copied().collect()).unwrap_or_default()
}

/// Is the foreground window one of this process's windows?
#[cfg(target_os = "windows")]
pub(crate) fn this_app_is_active() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // SAFETY: plain Win32 queries with no preconditions; `pid` is a valid out-param.
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, &mut pid);
        pid == GetCurrentProcessId()
    }
}

#[cfg(target_os = "windows")]
fn swp_flags(asynchronous: bool) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let mut flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    if asynchronous {
        flags |= SWP_ASYNCWINDOWPOS;
    }
    flags
}

/// Apply `action` to `hwnd`. `asynchronous` (SWP_ASYNCWINDOWPOS) for callers
/// off the UI thread, so they never block on it.
#[cfg(target_os = "windows")]
fn apply_z_action(hwnd: isize, action: ZAction, asynchronous: bool) {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowLongW, GetWindowThreadProcessId, SetWindowPos, GWL_EXSTYLE,
        HWND_NOTOPMOST, HWND_TOPMOST, WS_EX_TOPMOST,
    };
    let h = hwnd as windows_sys::Win32::Foundation::HWND;
    // SAFETY: SetWindowPos on a stale HWND fails harmlessly; the other calls
    // are plain queries.
    unsafe {
        let insert_after = match action {
            ZAction::None => return,
            ZAction::MakeTopmost => HWND_TOPMOST,
            ZAction::DropToNormal => HWND_NOTOPMOST,
            ZAction::DropBelowForeground => {
                // Placing a topmost window after a non-topmost one takes it
                // out of the topmost band, directly under that window. Only
                // when the foreground window is another process's NORMAL
                // window: WM_ACTIVATEAPP can arrive before the switch (the
                // foreground is still ours), and sliding under another app's
                // topmost window would keep us topmost desktop-wide.
                // Otherwise HWND_NOTOPMOST; the window being activated is
                // raised over us right after anyway.
                let fg = GetForegroundWindow();
                let mut pid = 0u32;
                if !fg.is_null() {
                    GetWindowThreadProcessId(fg, &mut pid);
                }
                let foreign_normal = !fg.is_null()
                    && fg != h
                    && pid != GetCurrentProcessId()
                    && (GetWindowLongW(fg, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) == 0;
                if foreign_normal {
                    fg
                } else {
                    HWND_NOTOPMOST
                }
            }
        };
        SetWindowPos(h, insert_after, 0, 0, 0, 0, swp_flags(asynchronous));
    }
}

/// Bring the keep-above windows (approval pages) into line with the floaters:
/// topmost and raised over them while tacked floaters are on top, ordinary
/// windows otherwise. Call after any change to the floaters' band.
#[cfg(target_os = "windows")]
fn sync_keep_above(app_active: bool, asynchronous: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST};
    let on_top = keep_above_should_be_topmost(!tacked_snapshot().is_empty(), app_active, host_dialog_open());
    let insert_after = if on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
    for above in keep_above_hwnds() {
        // SAFETY: stale HWNDs fail harmlessly.
        unsafe {
            SetWindowPos(
                above as windows_sys::Win32::Foundation::HWND,
                insert_after,
                0,
                0,
                0,
                0,
                swp_flags(asynchronous),
            );
        }
    }
}

/// Current z-order of this process's top-level windows, topmost first.
#[cfg(target_os = "windows")]
fn our_top_levels_top_to_bottom() -> Vec<isize> {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetTopWindow, GetWindow, GetWindowThreadProcessId, GW_HWNDNEXT,
    };
    let mut out = Vec::new();
    // SAFETY: z-order walk with plain queries; bounded to guard against a
    // pathological list mutating under us.
    unsafe {
        let me = GetCurrentProcessId();
        let mut h = GetTopWindow(std::ptr::null_mut());
        let mut guard = 0;
        while !h.is_null() && guard < 10_000 {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, &mut pid);
            if pid == me {
                out.push(h as isize);
            }
            h = GetWindow(h, GW_HWNDNEXT);
            guard += 1;
        }
    }
    out
}

/// Re-stack every tacked floater for `event`, preserving their relative order,
/// then bring the approval pages into line. Idempotent: WM_ACTIVATEAPP reaches
/// every tacked floater, and each one's handler producing the same order is
/// what keeps the pass from scrambling it (Codex P2 on #3970).
#[cfg(target_os = "windows")]
pub(crate) fn restack_tacked(event: ZEvent, app_active: bool, asynchronous: bool) {
    let tacked = tacked_snapshot();
    if !tacked.is_empty() {
        let action = z_action(event, true, app_active, host_dialog_open());
        for hwnd in restack_order(&our_top_levels_top_to_bottom(), &tacked) {
            apply_z_action(hwnd, action, asynchronous);
        }
    }
    sync_keep_above(app_active, asynchronous);
}

/// Tack or untack floater `label` now (spec §5). Returns false if the floater
/// isn't known (closed, or not a Windows floater).
#[cfg(target_os = "windows")]
pub(crate) fn set_floater_always_on_top(label: &str, on: bool) -> bool {
    let Some(hwnd) = floater_hwnd_for_label(label) else {
        return false;
    };
    set_floater_tacked(hwnd, on);
    let active = this_app_is_active();
    apply_z_action(hwnd, z_action(ZEvent::TackToggled, on, active, host_dialog_open()), false);
    sync_keep_above(active, false);
    true
}

/// A floater is being destroyed: drop its tack, and if it was the last
/// tacked one, let the approval pages go back to being ordinary windows.
#[cfg(target_os = "windows")]
fn forget_destroyed_floater(hwnd: isize) {
    if is_floater_tacked(hwnd) {
        set_floater_tacked(hwnd, false);
        sync_keep_above(this_app_is_active(), false);
    }
}

/// Held while a host file dialog is open: tacked floaters step down to the
/// normal band, and go back on top when the last dialog closes (spec §3.1).
/// Safe to create on any thread (the z-order calls are asynchronous).
pub(crate) struct HostDialogGuard;

impl HostDialogGuard {
    pub(crate) fn new() -> Self {
        HOST_DIALOGS_OPEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        #[cfg(target_os = "windows")]
        restack_tacked(ZEvent::DialogOpened, this_app_is_active(), true);
        HostDialogGuard
    }
}

impl Drop for HostDialogGuard {
    fn drop(&mut self) {
        let left = HOST_DIALOGS_OPEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1;
        #[cfg(target_os = "windows")]
        if left == 0 {
            restack_tacked(ZEvent::DialogClosed, this_app_is_active(), true);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = left;
    }
}

#[cfg(test)]
mod always_on_top_tests {
    use super::{
        keep_above_hwnds, keep_above_should_be_topmost, restack_order, unregister_keep_above_floaters,
        z_action, ZAction, ZEvent, KEEP_ABOVE_FLOATERS,
    };
    use std::collections::HashSet;

    #[test]
    fn tacking_while_active_goes_topmost() {
        assert_eq!(z_action(ZEvent::TackToggled, true, true, false), ZAction::MakeTopmost);
    }

    #[test]
    fn untacking_always_leaves_the_topmost_band() {
        for active in [true, false] {
            for dialog in [true, false] {
                assert_eq!(z_action(ZEvent::TackToggled, false, active, dialog), ZAction::DropToNormal);
            }
        }
    }

    #[test]
    fn tacking_while_inactive_or_under_a_dialog_waits() {
        assert_eq!(z_action(ZEvent::TackToggled, true, false, false), ZAction::None);
        assert_eq!(z_action(ZEvent::TackToggled, true, true, true), ZAction::None);
    }

    #[test]
    fn app_switches_move_only_tacked_floaters() {
        assert_eq!(z_action(ZEvent::AppActivated, true, true, false), ZAction::MakeTopmost);
        assert_eq!(z_action(ZEvent::AppDeactivated, true, false, false), ZAction::DropBelowForeground);
        assert_eq!(z_action(ZEvent::AppActivated, false, true, false), ZAction::None);
        assert_eq!(z_action(ZEvent::AppDeactivated, false, false, false), ZAction::None);
    }

    #[test]
    fn reactivating_during_a_dialog_stays_down() {
        assert_eq!(z_action(ZEvent::AppActivated, true, true, true), ZAction::None);
    }

    #[test]
    fn dialogs_step_tacked_floaters_down_and_back() {
        assert_eq!(z_action(ZEvent::DialogOpened, true, true, true), ZAction::DropToNormal);
        assert_eq!(z_action(ZEvent::DialogClosed, true, true, false), ZAction::MakeTopmost);
        // Dialog closed while another app is active: stay down until we're active.
        assert_eq!(z_action(ZEvent::DialogClosed, true, false, false), ZAction::None);
        assert_eq!(z_action(ZEvent::DialogOpened, false, true, true), ZAction::None);
    }

    #[test]
    fn restack_goes_bottom_first_so_the_order_survives() {
        // z-order, topmost first: A(tacked) main B(tacked) other C(tacked)
        let (a, main, b, other, c) = (1, 2, 3, 4, 5);
        let tacked: HashSet<isize> = [a, b, c].into_iter().collect();
        // Bottom-first: C, then B, then A — each insert lands above the last,
        // so A ends up on top again, then B, then C.
        assert_eq!(restack_order(&[a, main, b, other, c], &tacked), vec![c, b, a]);
    }

    #[test]
    fn restack_is_the_same_whichever_floater_runs_it() {
        // Every tacked floater's WM_ACTIVATEAPP runs the pass; it must depend
        // only on the z-order, not on who runs it. After one pass the order
        // is unchanged, so a second pass computes the same sequence.
        let tacked: HashSet<isize> = [10, 20].into_iter().collect();
        let first = restack_order(&[20, 7, 10], &tacked);
        let second = restack_order(&[20, 7, 10], &tacked);
        assert_eq!(first, vec![10, 20]);
        assert_eq!(first, second);
    }

    #[test]
    fn approval_pages_are_topmost_only_while_tacked_floaters_are() {
        assert!(keep_above_should_be_topmost(true, true, false));
        assert!(!keep_above_should_be_topmost(false, true, false), "no tacked floaters → ordinary window");
        assert!(!keep_above_should_be_topmost(true, false, false), "another app active → not over it");
        assert!(!keep_above_should_be_topmost(true, true, true), "floaters stepped down for a dialog");
    }

    #[test]
    fn keep_above_registry_tracks_open_approval_windows() {
        // Process-global registry shared with parallel tests: unique labels,
        // containment checks. Insert directly (registration's Win32 sync is
        // not what's under test).
        KEEP_ABOVE_FLOATERS.lock().unwrap().insert("approval-test-a".into(), 0x7a01);
        KEEP_ABOVE_FLOATERS.lock().unwrap().insert("approval-test-b".into(), 0x7a02);
        let hs = keep_above_hwnds();
        assert!(hs.contains(&0x7a01) && hs.contains(&0x7a02));

        unregister_keep_above_floaters("approval-test-a");
        let hs = keep_above_hwnds();
        assert!(!hs.contains(&0x7a01), "a closed approval window is no longer re-raised");
        assert!(hs.contains(&0x7a02));
        unregister_keep_above_floaters("approval-test-b");
        unregister_keep_above_floaters("approval-test-b"); // idempotent
    }
}

/// Return the Win32 window-class name for floating panes, suffixed with
/// the launcher-supplied `AGENTMUX_IPC_HASH` (= `hash(data_dir, version)`)
/// so that two parallel AgentMux instances register distinct class atoms
/// and their `wndproc`/`hInstance` pointers never collide (I5 invariant).
/// Falls back to the bare name in dev/test builds where the launcher may
/// not have set the env var.
pub(crate) fn floater_class_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        match std::env::var("AGENTMUX_IPC_HASH") {
            Ok(h) if !h.is_empty() => format!("AgentMuxFloatingPane-{}", h),
            _ => "AgentMuxFloatingPane".to_string(),
        }
    })
}

wrap_task! {
    pub struct CreateFloatingWindowTask {
        state: Arc<AppState>,
        pane_id: String,
        window_label: String,
        url: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        // HWND of the FullInstance main window that initiated the tear-off.
        // 0 if the caller didn't provide a source_window_label (back-compat).
        parent_main_hwnd: isize,
    }

    impl Task {
        fn execute(&self) {
            // Runs on the CEF UI thread.
            //
            //   1. CreateWindowExW with WS_EX_TOOLWINDOW + WS_POPUP (no owner
            //      — see module doc §"Why no owner (issue #1560)").
            //   2. Register in ACTIVE_FLOATER_HWNDS keyed by parent_main_hwnd
            //      so the cascade hook affects only this window's floaters.
            //   3. Embed a CEF browser inside via `set_as_child` —
            //      same pattern as `CreateBrowserPaneTask`'s Windows path in
            //      `browser_pane/creation.rs`.

            // Every early-return from execute() AFTER the host's
            // `post_create_floating_window` enqueued a
            // `PendingWindowCreation` must dispatch
            // `DequeuePendingWindowCreation` — `on_after_created`
            // only fires on success. The `floating-` exclusion in
            // `orphan_reconcile.rs` is belt-and-suspenders; this is
            // the actual cleanup. Codex/reagent P1 round 2 on #811.
            let dequeue = || {
                self.state.host_dispatch(
                    crate::reducer::HostCommand::DequeuePendingWindowCreation,
                );
            };

            // No Win32 owner — see module doc §"Why no owner (issue #1560)".
            // Minimize/restore/destroy cascade is handled by
            // install_main_window_floater_cascade_hook (client/wndproc.rs).
            let outer_hwnd = match create_popup(
                &self.window_label,
                self.x,
                self.y,
                self.width,
                self.height,
            ) {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!(
                        pane_id = %self.pane_id,
                        label = %self.window_label,
                        error = %e,
                        "[floating-pane] CreateWindowExW failed",
                    );
                    dequeue();
                    return;
                }
            };

            tracing::info!(
                pane_id = %self.pane_id,
                label = %self.window_label,
                hwnd = ?outer_hwnd,
                "[floating-pane] outer HWND created",
            );

            // Register the outer HWND in `state.window_hwnds` under the
            // floater's label. Without this, `resolve_window_hwnd(label)`
            // falls through to the reducer-registry path which goes
            // host.window_handle() (returns the CEF inner WS_CHILD) →
            // GetAncestor(GA_ROOT), and in our setup GA_ROOT lands on
            // MAIN's HWND (not the outer floater), so any
            // close_window_by_label("floating-…") would actually post
            // WM_CLOSE to MAIN and cascade-destroy every owned floater.
            //
            // Capturing the known-good outer HWND here ensures the
            // cache lookup (added to resolve_window_hwnd) returns the
            // right window for floater-targeted IPCs.
            self.state
                .window_hwnds
                .lock()
                .insert(self.window_label.clone(), outer_hwnd as isize);
            // Register in the global cascade registry so the main-window
            // WM_ACTIVATE/WM_SIZE/WM_DESTROY hook can reach this floater.
            // parent_main_hwnd binds this floater to its source window so
            // the hook only cascades its own floaters (not those of other
            // full-instance windows in the same process).
            crate::floating_pane::register_floater_hwnd(
                self.window_label.clone(),
                outer_hwnd as isize,
                self.parent_main_hwnd,
            );

            // CEF embed — the browser is a WS_CHILD of the outer HWND.
            let rect = Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            };

            let handler = crate::client::AgentMuxHandler::new_with_browser_pane(
                self.state.clone(),
                true,
            );
            let mut client = Some(crate::client::AgentMuxClient::new(handler, true, /* drag_capture: a floater renders panes that take file drops */ true));

            let url_cef = CefString::from(self.url.as_str());
            let mut settings = BrowserSettings::default();
            // Opaque dark base (theme #1e1e2e) so the browser surface paints
            // dark — not white — between window-show and first content paint.
            // A 0-alpha would fall back to the transparent CefSettings default
            // and flash white over the desktop on this unowned popup. (#1662 polish)
            settings.background_color = 0xFF1E1E2E;

            let parent_hwnd = sys::HWND(outer_hwnd as *mut _);
            let mut window_info = WindowInfo::default().set_as_child(parent_hwnd, &rect);
            window_info.runtime_style = RuntimeStyle::ALLOY;

            let result = browser_host_create_browser(
                Some(&window_info),
                client.as_mut(),
                Some(&url_cef),
                Some(&settings),
                None, // extra_info
                None, // request_context
            );

            if result == 0 {
                tracing::error!(
                    pane_id = %self.pane_id,
                    label = %self.window_label,
                    "[floating-pane] browser_host_create_browser returned 0",
                );
                // Cleanup-on-failure (codex P1 on #811). The outer
                // HWND was already created + shown via
                // `SW_SHOWNOACTIVATE` inside `create_popup`; if
                // we return here without `DestroyWindow` it sits on
                // screen as a phantom empty tool window. Also dequeue
                // the pending-creation entry that
                // `post_create_floating_window` enqueued — without
                // this, `on_after_created` (which fires only on
                // success) never dequeues, and the leaked entry
                // permanently blocks orphan reconciliation despite
                // the `floating-` exclusion in orphan_reconcile.rs
                // (belt-and-suspenders).
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
                    DestroyWindow(outer_hwnd as *mut std::ffi::c_void);
                }
                dequeue();
                return;
            }

            tracing::info!(
                pane_id = %self.pane_id,
                label = %self.window_label,
                "[floating-pane] CEF browser embedded in floating HWND",
            );
        }
    }
}

/// Posts the create-floating-window task to the CEF UI thread. Returns
/// immediately. Mirrors the shape of `ui_tasks::post_create_window` but
/// goes through this module so the path is grep-able.
pub fn post_create_floating_window(
    state: &Arc<AppState>,
    args: &crate::commands::floating_pane::OpenFloatingPaneArgs,
    window_label: &str,
    parent_main_hwnd: isize,
) {
    // Compose the URL the floating window's CEF browser will load. The
    // frontend's cef-init detects `floatingPaneId` and routes to the
    // floating-pane shell instead of the main workspace.
    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    // pane_id is a UUID-ish identifier in current callers — no
    // percent-encoding needed today. Use a minimal escape that handles
    // a few special chars in case future callers pass arbitrary names.
    // workspaceId threads through to the floater's `initApp` →
    // `initHostNewWindow` path which already understands a `?workspaceId=`
    // URL param (the existing tab tear-off uses the same handoff —
    // see initHostNewWindow in frontend/app-init.ts). Phase 1 floaters
    // didn't pass it and rendered the placeholder shell; Phase 2 callers
    // (#1077) pass the newly-created workspace id so the floater renders
    // the actual `<Block>` via the standard new-window init.
    let url = match crate::commands::window::resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let mut u = format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}&floatingPaneId={}",
                base_url,
                separator,
                ipc_port,
                ipc_token,
                window_label,
                escape_query_value(&args.pane_id),
            );
            if let Some(ws_id) = args.workspace_id.as_deref().filter(|s| !s.is_empty()) {
                u.push_str("&workspaceId=");
                u.push_str(&escape_query_value(ws_id));
            }
            u
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                window_label = %window_label,
                "[floating-pane] frontend assets unavailable — opening static error page",
            );
            crate::commands::window::assets_missing_data_url(&e)
        }
    };

    // Phase B.5 pre-create handoff — same shape as the main
    // open-window path so the existing window_meta plumbing (label →
    // kind → parent) sees the floater as a recognized creation.
    // Phase 6 will introduce a dedicated `WindowKind::FloatingPane`
    // to skip the taskbar / report-open logic in `on_after_created`;
    // Phase 1 reuses `Subwindow` (also hidden from taskbar today) so
    // the existing handler path holds.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: window_label.to_string(),
                kind: crate::state::WindowKind::Subwindow,
                parent_instance_id: None,
            },
        },
    );

    let mut task = CreateFloatingWindowTask::new(
        state.clone(),
        args.pane_id.clone(),
        window_label.to_string(),
        url,
        args.x,
        args.y,
        args.width,
        args.height,
        parent_main_hwnd,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

/// Pre-warmed pane pool window task (Windows). Creates a WS_POPUP +
/// WS_EX_TOOLWINDOW window at the off-screen pool position and embeds a
/// CEF child browser. The outer HWND is cached in `PANE_POOL_HWND_CACHE`
/// before `browser_host_create_browser` fires so `promote_pane_pool_window`
/// has a reliable handle even after CEF nulls `window_handle()` post-load.
///
/// Differs from `CreateFloatingWindowTask`:
///   1. No `register_floater_hwnd` — parent HWND unknown at spawn; deferred to promote.
///   2. Offscreen position + pane pool dimensions; window is hidden after create_popup.
///   3. URL carries `?pane-pool=1`; frontend defers init until `pool:pane-promote`.
wrap_task! {
    pub(crate) struct CreatePanePoolWindowWin32Task {
        state: Arc<AppState>,
        label: String,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            let dequeue = || {
                self.state.host_dispatch(
                    crate::reducer::HostCommand::DequeuePendingWindowCreation,
                );
            };

            let outer_hwnd = match create_popup(
                &self.label,
                crate::commands::window_pool::POOL_OFFSCREEN_X,
                crate::commands::window_pool::POOL_OFFSCREEN_Y,
                crate::commands::window_pool::PANE_POOL_WIDTH,
                crate::commands::window_pool::PANE_POOL_HEIGHT,
            ) {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!(
                        label = %self.label,
                        error = %e,
                        "[pane-pool] create_popup failed (Windows)"
                    );
                    // Clean up reducer state without triggering a refill spawn.
                    // HWND was not cached yet (create_popup failed before that step).
                    // on_pane_pool_window_destroyed must NOT be used here — it triggers
                    // spawn_pane_pool_window which would re-enter this failure → loop.
                    crate::commands::window_pool::cleanup_failed_pane_pool_creation(
                        &self.state, &self.label,
                    );
                    dequeue();
                    return;
                }
            };

            // create_popup ends with SW_SHOWNOACTIVATE — hide immediately so the
            // offscreen WS_POPUP doesn't appear in Alt+Tab while pre-warming.
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                let _ = ShowWindow(outer_hwnd as *mut std::ffi::c_void, SW_HIDE);
            }

            // Cache the outer HWND before browser creation. CEF's window_handle()
            // returns null after page load (same issue as tab pool, diagnosed
            // 2026-05-06); the cache is the only reliable source at promote time.
            crate::commands::window_pool::cache_pane_pool_hwnd(&self.label, outer_hwnd as usize);

            // The outer WS_POPUP is the top-level window that is activated
            // (during browser creation, and while hidden during a promote), so
            // it needs the WM_ACTIVATE observer that hands activation back from
            // an invisible window. `on_after_created` hooks the browser's own
            // HWND, which for a `set_as_child` browser may be the child.
            // Installing is idempotent, so a window hooked both ways is fine.
            unsafe {
                crate::client::wndproc::install_top_level_focus_restore_hook(
                    outer_hwnd as *mut std::ffi::c_void,
                );
            }

            // Register in window_hwnds so promote_pane_pool_window can reach
            // the outer HWND via state.window_hwnds if needed.
            self.state
                .window_hwnds
                .lock()
                .insert(self.label.clone(), outer_hwnd as isize);

            // Embed CEF browser as WS_CHILD of the outer WS_POPUP.
            let rect = Rect {
                x: 0,
                y: 0,
                width: crate::commands::window_pool::PANE_POOL_WIDTH,
                height: crate::commands::window_pool::PANE_POOL_HEIGHT,
            };
            let handler = crate::client::AgentMuxHandler::new_with_browser_pane(
                self.state.clone(),
                true,
            );
            let mut client = Some(crate::client::AgentMuxClient::new(handler, true, /* drag_capture: a floater renders panes that take file drops */ true));
            let url_cef = CefString::from(self.url.as_str());
            let mut settings = BrowserSettings::default();
            // Opaque dark base (theme #1e1e2e) so the browser surface paints
            // dark — not white — between window-show and first content paint.
            // A 0-alpha would fall back to the transparent CefSettings default
            // and flash white over the desktop on this unowned popup. (#1662 polish)
            settings.background_color = 0xFF1E1E2E;
            let parent_hwnd = sys::HWND(outer_hwnd as *mut _);
            let mut window_info = WindowInfo::default().set_as_child(parent_hwnd, &rect);
            window_info.runtime_style = RuntimeStyle::ALLOY;

            let result = browser_host_create_browser(
                Some(&window_info),
                client.as_mut(),
                Some(&url_cef),
                Some(&settings),
                None,
                None,
            );

            if result == 0 {
                tracing::error!(
                    label = %self.label,
                    "[pane-pool] browser_host_create_browser returned 0 (Windows)"
                );
                // Destroy the outer HWND (no browser to close it for us).
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
                    DestroyWindow(outer_hwnd as *mut std::ffi::c_void);
                }
                // Remove from window_hwnds (inserted above before browser creation).
                self.state.window_hwnds.lock().remove(&self.label);
                // Clean up reducer + HWND cache without triggering a refill spawn.
                // on_pane_pool_window_destroyed must NOT be used here — it triggers
                // spawn_pane_pool_window which would re-enter this failure → loop.
                crate::commands::window_pool::cleanup_failed_pane_pool_creation(
                    &self.state, &self.label,
                );
                dequeue();
            }
        }
    }
}

/// Post `CreatePanePoolWindowWin32Task` to the CEF UI thread.
///
/// Called by `spawn_pane_pool_window` on Windows instead of
/// `ui_tasks::post_create_window` so the pool window is a WS_POPUP +
/// WS_EX_TOOLWINDOW — the same window type as `post_create_floating_window`.
/// This ensures `promote_pane_pool_window` can reuse the same HWND without
/// re-creating the window at tear-off time.
pub(crate) fn post_create_pane_pool_window_win32(
    state: &Arc<AppState>,
    label: &str,
    url: &str,
) {
    let mut task = CreatePanePoolWindowWin32Task::new(
        state.clone(),
        label.to_string(),
        url.to_string(),
    );
    post_task(ThreadId::UI, Some(&mut task));
}

/// Minimal query-string escaping for the pane id. Encodes the small
/// set of characters that would break query-string parsing. Avoids
/// pulling in a `url`/`urlencoding` dependency for a single-call site.
fn escape_query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                // Encode as %XX for each UTF-8 byte.
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

/// WndProc for the floating-pane class. Removes the system non-client
/// area (no native title bar / borders drawn) and maps the 6 CSS-px
/// edge bands to `HT{LEFT,RIGHT,...}` so the window remains resizable.
///
/// Window-drag is intentionally NOT handled here. We tried HTCAPTION
/// over the pane header but two problems made it unworkable:
///
///   1. The pane header sits below tile-layout padding (some y-offset
///      from the window top), so any hard-coded Y range is fragile.
///   2. Mouse events in an HTCAPTION zone never reach the CEF child
///      HWND — buttons inside the header (close / magnify / mic /
///      view-specific endIconButtons) stop responding.
///
/// Window drag is instead JS-driven in
/// `frontend/app/workspace/floating-pane-workspace.tsx`: a targeted
/// document-level mousedown listener scoped to `[data-role="block-header"]`,
/// `preventDefault`-ing to suppress the HTML5 dragstart that
/// pragmatic-dnd would have used (which otherwise tore the pane off
/// into a second floating window — the "double tear-off" bug). Same
/// pattern as the main window's `useWindowDrag` hook.
///
/// See `docs/analysis/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`.
#[cfg(target_os = "windows")]
unsafe extern "system" fn floating_pane_wndproc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // Edge resize zone in CSS / DIP pixels, scaled to physical pixels
    // per the HWND's DPI inside the WM_NCHITTEST branch. A hard-coded
    // physical-pixel constant would shrink the hit zone on HiDPI —
    // a 6-physical-px resize border is 3 CSS px at 200% DPI and
    // effectively unreachable.
    const RESIZE_BORDER_CSS: i32 = 6;

    match msg {
        // Claim the entire window rect as client area — no system title
        // bar / borders drawn. WS_THICKFRAME (via WS_OVERLAPPEDWINDOW)
        // still gives us the resize border for `WM_NCHITTEST` to map.
        WM_NCCALCSIZE if wparam == 1 => return 0,
        // Suppress the DWM activation border repaint.
        WM_NCACTIVATE => return 1,
        WM_NCHITTEST => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            let mut rect = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
            GetWindowRect(hwnd, &mut rect);

            // Scale the resize-border CSS px to physical px against
            // THIS HWND's current monitor — handles mid-life monitor
            // changes (the window can move between monitors at
            // different DPIs).
            let dpi = GetDpiForWindow(hwnd) as i32;
            let dpi = if dpi > 0 { dpi } else { 96 };
            let resize_border_px = (RESIZE_BORDER_CSS * dpi / 96).max(1);

            let left = x - rect.left < resize_border_px;
            let right = rect.right - x < resize_border_px;
            let top = y - rect.top < resize_border_px;
            let bottom = rect.bottom - y < resize_border_px;
            if top && left {
                return HTTOPLEFT as isize;
            }
            if top && right {
                return HTTOPRIGHT as isize;
            }
            if bottom && left {
                return HTBOTTOMLEFT as isize;
            }
            if bottom && right {
                return HTBOTTOMRIGHT as isize;
            }
            if left {
                return HTLEFT as isize;
            }
            if right {
                return HTRIGHT as isize;
            }
            if top {
                return HTTOP as isize;
            }
            if bottom {
                return HTBOTTOM as isize;
            }

            // Pane-header drag is JS-driven (see floating-pane-workspace.tsx);
            // we don't map any zone to HTCAPTION here. Fall through to
            // HTCLIENT so clicks reach CEF and the JS handler.
        }
        // Unregister from the cascade registry so the main-window hook no
        // longer tries to show/hide/z-order a destroyed HWND.
        WM_DESTROY => {
            crate::floating_pane::unregister_floater_by_hwnd(hwnd as isize);
            forget_destroyed_floater(hwnd as isize);
        }
        // "Always on top": sent to every top-level window of this thread when
        // activation moves to or from another process (another app, or another
        // AgentMux instance). A tacked floater is topmost only while we are the
        // active app. SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27 §3.
        WM_ACTIVATEAPP => {
            // Every tacked floater gets this; each runs the same
            // order-preserving pass over all of them (idempotent).
            if is_floater_tacked(hwnd as isize) {
                let (event, active) = if wparam != 0 {
                    (ZEvent::AppActivated, true)
                } else {
                    (ZEvent::AppDeactivated, false)
                };
                restack_tacked(event, active, false);
            }
        }
        // Resize the floater's FRONTEND browser (header + layout) to fill the
        // client area on every outer-window resize (maximize / restore, and
        // future edge-resize). CEF `set_as_child` browsers don't self-resize,
        // and our custom wndproc replaced CEF's default proc, so without this
        // the browser stays at its creation size while the outer grows.
        //
        // CRITICAL: resize the BOTTOM-most direct child, NOT `GW_CHILD` (the
        // topmost). The frontend browser is embedded at floater creation, so
        // it's the oldest child and sits at the BOTTOM of the Z-order. A
        // BROWSER pane adds a SECOND direct child later — the native
        // web-content window from `browser_pane_create` — which lands ABOVE
        // the frontend. Resizing the topmost child therefore stretched the
        // web-content window over the whole floater, hiding the header and
        // swallowing all input (the "maximized browser pane is fullscreen,
        // can't click anything" trap). We only size the frontend browser; once
        // it has the new viewport it reflows and repositions its own
        // web-content child below the header via `browser_pane_resize`. CEF
        // cascades the resize to the frontend's inner render-widget children.
        // (Terminal/agent floaters have a single child, so bottom-most == that
        // child — unchanged.) Falls through to DefWindowProcW afterwards.
        WM_SIZE => {
            // Walk the sibling Z-order from topmost (GW_CHILD) to the
            // bottom-most direct child = the frontend browser.
            let mut frontend = GetWindow(hwnd, GW_CHILD);
            let mut next = frontend;
            while !next.is_null() {
                frontend = next;
                next = GetWindow(next, GW_HWNDNEXT);
            }
            if !frontend.is_null() {
                let mut rc = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                if GetClientRect(hwnd, &mut rc) != 0 {
                    SetWindowPos(
                        frontend,
                        std::ptr::null_mut(),
                        0,
                        0,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// CreateWindowExW wrapper that produces an unowned `WS_POPUP +
/// WS_EX_TOOLWINDOW` HWND used as the floating-pane outer shell.
/// No Win32 owner — cascade behavior is implemented in the main-window
/// WndProc hook (`install_main_window_floater_cascade_hook`).
///
/// The class is registered once per process; subsequent calls reuse
/// the registered atom.
fn create_popup(
    window_label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<*mut std::ffi::c_void, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, RegisterClassExW, ShowWindow, CS_HREDRAW, CS_VREDRAW, SW_SHOWNOACTIVATE,
        WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP, WS_THICKFRAME,
    };

    // ---- Register the class once per process ----
    static CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

    // I5 compliance: embed AGENTMUX_IPC_HASH (= hash(data_dir, version),
    // set by the launcher) so two parallel instances register distinct
    // class atoms and their wndproc/hInstance pointers don't collide.
    // Falls back to the bare name in dev/test builds without the launcher.
    let class_name = crate::floating_pane::floater_class_name();
    let mut class_name_utf16: Vec<u16> = OsStr::new(class_name).encode_wide().collect();
    class_name_utf16.push(0);

    // TODO(phase-6, codex P1 on #811 — explicitly deferred): The
    // documented CEF embedding pattern is for the host's wndproc to
    // intercept WM_CLOSE and route through `CloseBrowser(false)` so
    // DoClose fires before destroy. Today the OS X-button cascade
    // still works end-to-end via `floating_pane_wndproc`'s fallthrough
    // to `DefWindowProcW(WM_CLOSE)`:
    //
    //   1. User clicks X → DefWindowProcW(WM_CLOSE) → DestroyWindow.
    //   2. Outer HWND's WM_DESTROY cascades into the CEF child HWND
    //      (WS_CHILD of outer).
    //   3. CEF's wndproc on the child runs its destroy handler →
    //      OnBeforeClose fires on AgentMuxHandler → reducer
    //      UnregisterBrowser cleans `state.browsers` + `window_meta`.
    //
    // What's *skipped*: the DoClose hook's chance to cancel close
    // (e.g. for a "Are you sure?" prompt). Floating panes have no
    // such prompt, so this is harmless. The full WM_CLOSE → CloseBrowser
    // routing is still future work.
    CLASS_REGISTERED.call_once(|| unsafe {
        let h_instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        // Dark class background brush (theme #1e1e2e). With a null brush the
        // popup's initial WM_ERASEBKGND paints white, flashing on tear-off
        // before the embedded CEF child covers the client area. COLORREF is
        // 0x00BBGGRR, so #1e1e2e -> 0x002E1E1E. Process-lifetime leak is fine
        // (one brush for the one class). (#1662 polish)
        let dark_brush =
            windows_sys::Win32::Graphics::Gdi::CreateSolidBrush(0x002E1E1E);
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(floating_pane_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: dark_brush,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name_utf16.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        let atom = RegisterClassExW(&wnd_class);
        if atom == 0 {
            tracing::error!(
                "[floating-pane] RegisterClassExW failed for '{}'; CreateWindowExW will fail",
                class_name,
            );
        }
    });

    let mut title_utf16: Vec<u16> = OsStr::new(&format!("AgentMux — {window_label}"))
        .encode_wide()
        .collect();
    title_utf16.push(0);

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name_utf16.as_ptr(),
            title_utf16.as_ptr(),
            // WS_POPUP for free positioning (NOT WS_CHILD — children
            // are clipped to parent's client area). WS_THICKFRAME for
            // the resize border. NO `WS_CAPTION` — Win32 still reserves
            // title-bar space for WS_CAPTION windows even when
            // WM_NCCALCSIZE returns 0, which leaves a system title bar
            // drawn on top of the client area AND truncates the
            // effective client size (CEF embedded at (0,0,W,H) overruns
            // the visible client → content cut off bottom+right). The
            // frontend's `BlockFrame_Header` is the only chrome.
            WS_POPUP | WS_THICKFRAME,
            x,
            y,
            width,
            height,
            std::ptr::null_mut(), // no owner — see module doc §"Why no owner"
            std::ptr::null_mut(),
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        return Err(format!("CreateWindowExW returned NULL (GetLastError={err})"));
    }

    // Tell DWM to extend the entire frame into the client area. Without
    // this, DWM keeps drawing the standard Win32 title bar (and the
    // minimize/maximize/close caption buttons that come with it) on top
    // of our client area, even though our `WM_NCCALCSIZE → 0` says the
    // client area fills the window. Combined with our WndProc's
    // `WM_NCCALCSIZE`/`WM_NCACTIVATE`/`WM_NCHITTEST`, this gives a truly
    // chrome-free outer HWND. The docked-pane's standard
    // `BlockFrame_Header` (33 CSS px, `--header-height` in theme.scss's
    // `:root`) is the sole chrome — drag is started from the `onMount` in
    // `frontend/app/workspace/floating-pane-workspace.tsx`, and on Windows
    // the move loop itself runs host-side in `Win32BeginMoveTask`.
    unsafe {
        use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
        use windows_sys::Win32::UI::Controls::MARGINS;
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        let hr = DwmExtendFrameIntoClientArea(hwnd, &margins);
        if hr != 0 {
            tracing::warn!(
                "[floating-pane] DwmExtendFrameIntoClientArea failed hr=0x{hr:08x} — system title bar may still be drawn",
            );
        }
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    Ok(hwnd as *mut std::ffi::c_void)
}

#[cfg(test)]
mod tests {
    use super::escape_query_value;

    #[test]
    fn escape_passes_through_safe_chars() {
        assert_eq!(escape_query_value("abc-XYZ_123.~"), "abc-XYZ_123.~");
    }

    #[test]
    fn escape_encodes_special_chars() {
        assert_eq!(escape_query_value("a b"), "a%20b");
        assert_eq!(escape_query_value("a&b=c"), "a%26b%3Dc");
        assert_eq!(escape_query_value("a/b"), "a%2Fb");
    }

    #[test]
    fn escape_encodes_multibyte_utf8() {
        // U+00E9 'é' is 0xC3 0xA9 in UTF-8.
        assert_eq!(escape_query_value("é"), "%C3%A9");
    }
}
