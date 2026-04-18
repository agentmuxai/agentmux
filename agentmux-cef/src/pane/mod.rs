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

pub mod lifecycle;

pub use lifecycle::{PaneLifecycle, PaneStateMachine, RegisterResult};
