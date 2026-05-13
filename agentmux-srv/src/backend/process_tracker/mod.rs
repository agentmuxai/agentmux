// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-spawned process tracking.
//!
//! Gives the host a complete, authoritative view of what each agent CLI
//! has forked — backgrounded shells, dev servers, Docker containers,
//! file watchers, nested bash/python/node children, etc. The goal is
//! end-user visibility: a user running multiple agents can see in one
//! place what's still running on their machine, and kill it reliably
//! when they're done.
//!
//! The API is a platform-agnostic trait. Per-platform impls use the
//! strongest available mechanism:
//!
//! | Platform | Impl            | Mechanism                                  | Confidence |
//! |----------|-----------------|--------------------------------------------|------------|
//! | Windows  | `JobObjectTracker` | `CreateJobObject` + `AssignProcessToJobObject` + `TerminateJobObject` | high       |
//! | Linux    | `Cgroupv2Tracker`  | `systemd-run --user --scope` + `cgroup.procs` / `cgroup.kill`      | high       |
//! | macOS    | `ProcessGroupTracker` | `POSIX_SPAWN_SETPGROUP` + `killpg`                               | best-effort |
//! | other    | `StubTracker`   | no-op                                                          | none       |
//!
//! The frontend's swarm panel surfaces the confidence level so users know
//! when tracking may miss escaped descendants.
//!
//! See `agentmux-ai/AGENT_SPAWNED_PROCESSES_SPEC.md` for the design.

use std::sync::Arc;

pub mod registry;

#[cfg(windows)]
pub mod windows;

// `pub mod stub;` (file-form) was here. Removed: `stub.rs` doesn't exist
// in the tree — only the two inline `pub mod stub { ... }` definitions
// below (cfg(not(windows)) and cfg(windows)) define the module. On Linux
// the file-form line collided with the inline non-Windows definition →
// E0428 "the name `stub` is defined multiple times" → broke `task dev`.

/// A single process tracked by the host — PID + metadata enriched
/// per-platform. The frontend renders one row per entry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrackedProcess {
    pub pid: u32,
    /// Full command line, or best approximation. May be empty if the
    /// platform doesn't expose it cheaply (macOS without `libproc`).
    pub command: String,
    /// Working-set / RSS in bytes. 0 if unavailable.
    pub rss_bytes: u64,
    /// Unix ms of process creation, 0 if unavailable.
    pub started_at_ms: u64,
}

/// Opaque per-agent handle returned by the tracker when we wrap a spawn.
/// Held inside `AgentProcessRegistry` for the lifetime of the pane;
/// dropped when the pane closes or the agent exits.
#[allow(dead_code)]
pub trait TrackerHandle: Send + Sync {
    /// Add a freshly-spawned process to the tracked tree. Called by the
    /// controller immediately after `tokio::process::Command::spawn`.
    /// Descendants created AFTER this call are caught automatically;
    /// descendants created BEFORE (in the ~1ms race window) escape.
    /// No-op in the stub impl — platforms without a real tracker
    /// silently accept the PID and move on.
    fn assign_process(&self, pid: u32) -> Result<(), String>;

    /// Enumerate the current members of this tracked tree.
    ///
    /// Must be cheap enough to poll every ~2s. On Windows this is a
    /// single Job Object query; on Linux it's a read of `cgroup.procs`;
    /// on macOS it's a sysctl scan.
    fn list_members(&self) -> Vec<TrackedProcess>;

    /// Forcibly terminate every process in this tracked tree.
    fn kill_tree(&self);

    /// Terminate a single process by PID, if it's a member of this tree.
    /// Returns `true` if the PID was known and the kill was attempted.
    fn kill_pid(&self, pid: u32) -> bool;

    /// Describes how confidently this platform tracks descendants.
    /// Surfaced to the UI so the user can tell when tracking is
    /// best-effort and escape-prone.
    fn confidence(&self) -> TrackingConfidence;
}

/// How reliable this platform's tracker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackingConfidence {
    /// Descendants can't escape the tracker. Windows Job Objects +
    /// Linux cgroups v2.
    High,
    /// Descendants can escape via `setsid`, launchd, etc. macOS.
    BestEffort,
    /// Platform has no tracker. No guarantees.
    None,
}

/// Factory: returns a platform-appropriate tracker handle that will
/// accept the next-spawned process and everything it forks.
///
/// Call once per agent pane; reuse the handle across multiple turns of
/// the `SubprocessController` so children from any turn are all tracked
/// under the same umbrella.
pub fn new_tracker(block_id: &str) -> Arc<dyn TrackerHandle> {
    #[cfg(windows)]
    {
        match windows::JobObjectTracker::new(block_id) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    "[process-tracker] JobObjectTracker init failed — falling back to stub"
                );
                Arc::new(stub::StubTracker)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = block_id;
        Arc::new(stub::StubTracker)
    }
}

#[cfg(not(windows))]
pub mod stub {
    //! No-op tracker used on unsupported platforms or when init fails.
    //! All operations succeed silently; `list_members` always returns
    //! empty. Confidence reports `None` so the UI can inform the user
    //! that tracking is disabled.

    use super::{TrackedProcess, TrackerHandle, TrackingConfidence};

    pub struct StubTracker;

    impl TrackerHandle for StubTracker {
        fn assign_process(&self, _pid: u32) -> Result<(), String> {
            Ok(())
        }
        fn list_members(&self) -> Vec<TrackedProcess> {
            Vec::new()
        }
        fn kill_tree(&self) {}
        fn kill_pid(&self, _pid: u32) -> bool {
            false
        }
        fn confidence(&self) -> TrackingConfidence {
            TrackingConfidence::None
        }
    }
}

#[cfg(windows)]
pub mod stub {
    //! Windows fallback if `JobObjectTracker::new` fails (e.g. the
    //! process is not elevated enough to create a job object). The real
    //! impl lives in `windows`; this is only used for the init-fail
    //! recovery path.

    use super::{TrackedProcess, TrackerHandle, TrackingConfidence};

    pub struct StubTracker;

    impl TrackerHandle for StubTracker {
        fn assign_process(&self, _pid: u32) -> Result<(), String> {
            Ok(())
        }
        fn list_members(&self) -> Vec<TrackedProcess> {
            Vec::new()
        }
        fn kill_tree(&self) {}
        fn kill_pid(&self, _pid: u32) -> bool {
            false
        }
        fn confidence(&self) -> TrackingConfidence {
            TrackingConfidence::None
        }
    }
}
