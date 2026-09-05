// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window management commands for the CEF host.
// Ported from src-tauri/src/commands/window.rs.
//
// Phase 2: Single-window only. Multi-window commands are stubbed.
//
// Modularization complete (docs/analysis/ANALYSIS_LARGE_FILE_MODULARIZATION_CANDIDATES_2026_05_28.md,
// Plan 1): this file is now a pure re-export shim. Each handler family
// lives in its own sibling module — lifecycle / motion / chrome /
// transparency / meta / creation — and is re-exported here so call sites
// keep resolving `commands::window::<name>` unchanged.

mod lifecycle;
mod panel;
// Cross-platform command handlers dispatched by ipc.rs.
pub use lifecycle::{close_window, close_window_by_label, quit_app};
pub use panel::open_panel;
#[cfg(target_os = "windows")]
pub use lifecycle::find_main_window;
// Windows-only helpers other modules resolve as `commands::window::<name>`
// (browser_pane / client / backend call sites are all `#[cfg(windows)]`).
#[cfg(target_os = "windows")]
pub(crate) use lifecycle::{
    capture_hwnd_for_label, find_own_top_level_window, resolve_window_hwnd,
    resolve_window_hwnd_strict,
};
// Shared close-routing predicate — also consumed by the OS-close
// (WM_CLOSE) routing subclass in `client/wndproc.rs` (task #30).
#[cfg(target_os = "windows")]
pub(crate) use lifecycle::should_route_close_through_task;

mod motion;
// Position / drag / redock-hover command handlers, all dispatched by ipc.rs.
pub use motion::*;

mod chrome;
// Minimize / maximize command handlers, dispatched by ipc.rs.
pub use chrome::*;

mod transparency;
// Transparency + per-window opacity command handlers, dispatched by ipc.rs.
pub use transparency::*;

mod meta;
// Zoom / label / instance-listing / focus / devtools command handlers.
pub use meta::*;

mod gpu_trace;
// Dev-only memory-infra GPU tracing (#2218 diagnostics) — begin_gpu_trace/end_gpu_trace.
pub use gpu_trace::*;

mod creation;
// open_new_window / open_subwindow + frontend-URL resolution. The setters
// are `pub` (ipc.rs); `resolve_frontend_base_url` / `assets_missing_data_url`
// are `pub(crate)` and called by client / drag / window_pool / floating_pane,
// so they're re-exported explicitly (the `*` glob only covers `pub` items).
pub use creation::*;
pub(crate) use creation::{assets_missing_data_url, resolve_frontend_base_url};

// Debounced srv write-through for window position/size (the position-side
// counterpart to `transparency`'s opacity write-through). Windows-only,
// matching its one caller: `wrr::win_event`'s EVENT_OBJECT_LOCATIONCHANGE
// hook (`wrr` itself is only compiled on Windows — see `wrr/mod.rs` — macOS/
// Linux have no equivalent WinEvent-based live position stream today, so
// there is nothing for this module to hook into there). Reagent P0 on
// PR #2302: this module unconditionally uses `windows_sys::HWND` and
// `AppState::label_for_hwnd` (itself `#[cfg(windows)]`), so it must not be
// declared on other platforms — `windows-sys` is only a dependency under
// `[target.'cfg(target_os = "windows")'.dependencies]` in Cargo.toml. Not
// glob-exported — called via the full path
// (`crate::commands::window::position_persist::
// report_position_for_srv_writethrough`) from its one caller.
#[cfg(target_os = "windows")]
pub(crate) mod position_persist;
