// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who gets a window's keyboard and pointer: the app's own content or one
//! browser pane's page. `machine` is the pure state machine, `registry` holds
//! one per window, and `windows` is the Windows adapter that feeds it and runs
//! its effects. macOS and Linux adapters are Phase 4.
//! docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md.

// Until the macOS and Linux adapters exist, only the Windows build calls these.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod machine;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod registry;
#[cfg(target_os = "windows")]
pub mod watch;
#[cfg(target_os = "windows")]
pub mod windows;
