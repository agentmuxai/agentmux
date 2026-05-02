// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser-pane module.
//!
//! Phase H.1.d/e (PR #5) deleted the in-memory `PaneStateMachine`; pane
//! lifecycle now lives only in the host reducer (`HostState.panes`).
//! `RegisterResult` moved to `crate::reducer`.

pub mod callbacks;
pub mod creation;
#[cfg(target_os = "windows")]
pub mod hwnd;

pub use creation::CreatePaneTask;

#[cfg(target_os = "windows")]
pub use hwnd::ALLOW_PANE_FOCUS_ONCE;

#[cfg(target_os = "windows")]
pub use hwnd::install_pane_focus_redirect;
