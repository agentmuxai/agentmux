// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser-pane module.
//!
//! Phase H.1.d/e (PR #5) deleted the in-memory `PaneStateMachine`; pane
//! lifecycle now lives only in the host reducer (`HostState.browser_panes`).
//! `RegisterResult` moved to `crate::reducer`.

pub mod auth;
pub mod callbacks;
pub mod creation;
pub mod label;
pub mod leaving;
pub mod open_timing;
pub mod page_close;
pub mod identity;
pub mod profile_carry;
pub mod size_badge;
pub mod trace;
#[cfg(target_os = "windows")]
pub mod hwnd;
#[cfg(target_os = "windows")]
pub mod wrapper;
pub mod creation_views;
pub mod side_button_cdp;
pub mod signin_notice;

pub use creation::CreateBrowserPaneTask;
