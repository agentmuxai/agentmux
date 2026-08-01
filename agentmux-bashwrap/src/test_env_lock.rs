// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared `std::env` mutation lock for this crate's test modules.
//!
//! `wps_client.rs` and `precompact.rs` both mutate the SAME
//! process-global env vars (`AGENTMUX_LOCAL_URL`, `AGENTMUX_AUTH_KEY`,
//! `AGENTMUX_BLOCKID`) in their `#[cfg(test)]` suites. Cargo runs tests
//! in parallel by default; each module previously held its own private
//! `ENV_LOCK`, which didn't actually synchronize anything cross-module —
//! a `precompact` test could clear an env var while a `wps_client` test
//! was setting/reading it, causing nondeterministic assertion failures
//! (Codex P2, PR #2378 round 6). One shared lock, used by both.

use std::sync::Mutex;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());
