// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser-pane module.
//!
//! Phase 1 of the split described in `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md`:
//! extract the pure lifecycle state machine into `lifecycle.rs`. The orchestration
//! layer (`BrowserPaneManager`) and CEF-integration layer stay in
//! `crate::browser_panes` for now; subsequent phases move them here.
//!
//! Re-exports the public surface that `browser_panes.rs` and tests consume.

pub mod callbacks;
pub mod creation;
pub mod lifecycle;
#[cfg(target_os = "windows")]
pub mod hwnd;

pub use creation::CreatePaneTask;
pub use lifecycle::{PaneStateMachine, RegisterResult};

// PaneLifecycle is reachable via `crate::pane::lifecycle::PaneLifecycle`
// for tests that need to assert state; no re-export needed.
#[cfg(test)]
pub use lifecycle::PaneLifecycle;

#[cfg(target_os = "windows")]
pub use hwnd::ALLOW_PANE_FOCUS_ONCE;

#[cfg(target_os = "windows")]
pub use hwnd::install_pane_focus_redirect;
