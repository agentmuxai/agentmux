// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who gets a window's keyboard and pointer: the app's own content or one
//! browser pane's page. `machine` is the pure state machine; the platform
//! adapters that feed it and run its effects come in later phases.
//! docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md.

// Phase 1 builds the machine alone; the adapter (Phase 2) is its first caller.
#[allow(dead_code)]
pub mod machine;
