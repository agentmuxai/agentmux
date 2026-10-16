// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who gets a window's keyboard and pointer: the app's own content or one
//! browser pane's page. `machine` is the pure state machine, `registry` holds
//! one per window, `windows` is the Windows adapter that feeds it and runs its
//! effects, `views` the macOS and Linux one, and `watch` the Windows input
//! diagnostics. docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md.

// Some events and queries are used by one platform's adapter only.
#[allow(dead_code)]
pub mod machine;
#[allow(dead_code)]
pub mod registry;
// Built everywhere so its shared part is checked on Windows; called on macOS and Linux.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub mod views;
#[cfg(target_os = "windows")]
pub mod watch;
#[cfg(target_os = "windows")]
pub mod windows;
