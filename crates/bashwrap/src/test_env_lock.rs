// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared `std::env` mutation lock for this crate's test modules.
//!
//! `mps_client.rs` and `precompact.rs` both mutate the SAME
//! process-global env vars (`AGENTMUX_LOCAL_URL`, `AGENTMUX_AUTH_KEY`,
//! `AGENTMUX_BLOCKID`) in their `#[cfg(test)]` suites. Cargo runs tests
//! in parallel by default; each module previously held its own private
//! `ENV_LOCK`, which didn't actually synchronize anything cross-module —
//! a `precompact` test could clear an env var while a `mps_client` test
//! was setting/reading it, causing nondeterministic assertion failures
//! (Codex P2, PR #2378 round 6). One shared lock, used by both.

use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Unique-per-call scratch directory under the OS temp dir, so parallel
/// test runs (cargo's default) never share state. Mirrors the marker-tag
/// pattern `run_via_pty_kills_idle_child_and_returns_promptly` already
/// uses for the same reason.
pub(crate) fn unique_temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentmux-bashwrap-test-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create unique temp dir");
    dir
}

/// Save-and-restore guard for an env var, so tests can freely
/// set/remove it and always leave the ambient environment exactly as
/// they found it, even on an early return or panic mid-test — every
/// cwd-state-key test needs this for at least `AGENTMUX_AGENT_ID` and
/// `AGENTMUX_INSTANCE_SLUG` simultaneously, so a bespoke prev/restore
/// dance per test (the pattern used elsewhere in this file for a single
/// var) would otherwise multiply combinatorially.
pub(crate) struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    pub(crate) fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }

    pub(crate) fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
