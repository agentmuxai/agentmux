// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF UI thread task dispatch.
//
// All CEF Views operations (Window::close, minimize, maximize, etc.) must run
// on the CEF UI thread. IPC commands arrive on tokio threads. This module
// provides tasks that can be posted to the UI thread via post_task().
//
// Key insight: don't pass Browser/Window handles across threads. Instead,
// pass Arc<AppState> and look up the browser on the UI thread.
//
// Used on Linux (and macOS). On Windows, Win32 APIs are used directly since
// they are safe to call from any thread.
//
// This module was split from a single `ui_tasks.rs` into category submodules
// (pure reorganization — zero logic / public-API changes). Every task type and
// `post_*` / `get_*` function is re-exported below so external call sites keep
// using `crate::ui_tasks::…` unchanged.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;

mod window;
mod drag;
mod pool;
mod pane_geometry;
#[cfg(target_os = "windows")]
mod snap_preview;
#[cfg(target_os = "macos")]
pub mod pane_hole_mask;
mod platform_macos;

pub use window::*;
pub use drag::*;
pub use pool::*;
#[cfg(not(target_os = "windows"))]
pub use pane_geometry::*;
// `clear_pane_swizzle_statics` is always present (real impl on macOS, no-op
// elsewhere) and is the only cross-platform item in `platform_macos`. The
// remaining swizzle statics/fns are macOS-only, so glob-importing the module
// only makes sense there.
// `clear_pane_swizzle_statics` (real impl on macOS, no-op on Linux) is only
// called from the non-Windows browser-pane detach path, so its re-export is
// gated to match. The remaining swizzle statics/fns are macOS-only.
#[cfg(not(target_os = "windows"))]
pub(crate) use platform_macos::clear_pane_swizzle_statics;
#[cfg(target_os = "macos")]
pub use platform_macos::*;

/// Get the CEF Views Window for a browser label on the UI thread.
pub(crate) fn get_window_on_ui(state: &Arc<AppState>, label: &str) -> Option<Window> {
    // Phase H.2.b — reducer-aware lookup with fallback.
    let mut browser = state.get_browser(label)?;
    let browser_view = browser_view_get_for_browser(Some(&mut browser))?;
    browser_view.window()
}
