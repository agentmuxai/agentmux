// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tear-off Phase 4 — WH_MOUSE_LL low-level mouse hook for cross-window
// merge detection during the SC_MOVE modal move-loop.
//
// Background: while Windows runs the modal SC_MOVE loop (entered via
// `commands/drag.rs::tear_off_sc_move_handshake`), AgentMux's normal
// renderer message handlers DON'T fire — Windows owns the cursor
// until mouseup. To detect "is the cursor over another AgentMux
// window's tab strip?" we install a global low-level mouse hook on a
// dedicated thread with its own GetMessage loop.
//
// The hook callback runs on the install thread (not arbitrary
// threads). It uses thread-local storage to access the hook context
// (Arc<AppState>, source/dest labels, tab id, etc.) without risking
// re-entrant locking issues that a global Mutex would have.
//
// Architecture:
//   * `start_tear_off_tracking()` is called from the IPC handler
//     BEFORE the SC_MOVE post. Spawns a thread, installs the hook and
//     returns once it is installed; the thread runs until the user
//     releases the mouse (the thread's GetMessage loop sees
//     WM_LBUTTONUP and calls PostQuitMessage) or
//     `stop_active_hook_session` posts it WM_QUIT.
//   * On every WM_MOUSEMOVE, the callback does WindowFromPoint →
//     GetAncestor(GA_ROOT) and looks the HWND up in `state.browsers`
//     (skipping the dragged window itself). If the candidate target
//     changed, emits `tearoff:hover-changed` IPC events to the new
//     and old candidate's renderers.
//   * On WM_LBUTTONUP, emits a finalisation event: `tearoff:merge` to
//     the candidate window (which pulls the tab in and closes the
//     dragged window), `tearoff:cancel-back` to the source (drop on
//     source window / ESC), or `tearoff:standalone` to the source
//     (released over nothing).
//
// Spec: docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.3-§4.4
//
// Cross-window tab remount (docs/specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11):
// the same hook now also runs in `HookMode::TabDrag` for EVERY tab drag
// (installed at drag start via `start_tab_drag_tracking`, before any
// tear-off). In that mode the hover events are identical, but button-up
// over another window emits `tabdrag:merge-direct` (the tab still lives
// in its original multi-tab workspace — no temporary tear-off ws exists),
// and every other outcome emits nothing: the source window's normal
// in-window reorder / tear-off / HTML5 cross-drag paths own those. If a
// tear-off fires mid-drag, `start_tear_off_tracking` takes over the
// session (the previous hook thread is stopped via WM_QUIT).

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
pub use windows::{start_tab_drag_tracking, start_tear_off_tracking, stop_active_hook_session};

#[cfg(target_os = "macos")]
pub use macos::stop_active_hook_session;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn stop_active_hook_session() {}

#[cfg(target_os = "macos")]
pub use macos::start_tab_drag_tracking;

/// windowNumber → label registration, called from `app/mod.rs`'s
/// `on_window_created`/`on_window_destroyed` on the CEF UI thread. See
/// the doc comment at the top of `macos.rs` for why this cache exists.
#[cfg(target_os = "macos")]
pub(crate) use macos::register_window_number as macos_register_window_number;
#[cfg(target_os = "macos")]
pub(crate) use macos::unregister_window_label as macos_unregister_window_label;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn start_tab_drag_tracking(
    _state: std::sync::Arc<crate::state::AppState>,
    _source_label: String,
    _tab_id: String,
    _source_ws_id: String,
    _is_last_tab: bool,
) -> Result<(), String> {
    Ok(())
}
