// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Crate-wide test-only helpers shared across `#[cfg(test)]` modules.
//!
//! `ISOLATED_AUTH_ENV_LOCK` guards every test that mutates the process-global
//! `AGENTMUX_ISOLATED_AUTH`/`AGENTMUX_INSTANCE_DIR` env vars. Before this
//! existed, `registry::paths`, `migrations::runner`, and
//! `migrations::m0011_shared_store_backfill` each declared their own
//! module-local `Mutex<()>` — serializing tests *within* a module but not
//! *across* them. Cargo's default test runner executes all of a crate's
//! tests in one process with many threads, so those three modules' tests
//! could still interleave: one test clears the flag while another (holding
//! only its own module's lock) is mid-assertion on it, producing
//! nondeterministic failures (reagent/codex on PR #2318). Every test that
//! touches these env vars must acquire THIS lock instead of a local one.

pub(crate) static ISOLATED_AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
