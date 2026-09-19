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

use std::collections::{HashMap, HashSet};
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
    /// Parent PID, if the platform exposes it. Drives
    /// [`agent_started`]'s ancestry walk.
    pub parent_pid: Option<u32>,
}

/// Lowercased executable name without directory or `.exe` suffix —
/// `C:\Program Files\Git\bin\bash.exe` → `bash`.
fn image_name(command: &str) -> String {
    let base = command.rsplit(['\\', '/']).next().unwrap_or(command);
    let base = base.to_ascii_lowercase();
    base.strip_suffix(".exe").map(str::to_string).unwrap_or(base)
}

/// Interpreters an agent runs its tool commands through (Claude Code's Bash
/// tool spawns `bash.exe -c ...` directly under `claude.exe`; Codex/Gemini
/// use `powershell`/`cmd`).
fn is_shell(p: &TrackedProcess) -> bool {
    matches!(
        image_name(&p.command).as_str(),
        "bash" | "sh" | "dash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh" | "nu"
    )
}

/// Filter a tracker's raw membership down to the processes the agent
/// started through its tools — what the Swarm list, the pane-close
/// confirmation and the process events should count.
///
/// The raw job also holds the agent's own plumbing: the CLI itself
/// (`roots` — every PID handed to `assign_process`), its `conhost.exe`, and
/// MCP servers such as `agentmux-mcp.exe` (direct non-shell children of the
/// root, plus their own conhosts/children). Counting those made every idle
/// agent report ~3 processes and every pane close ask for confirmation.
///
/// Rule: drop roots and `conhost`; keep a process iff walking its parents
/// inside the job passes through a shell before reaching a root. A root
/// that is itself a shell (terminal blocks) counts as that shell. A chain
/// that hits a parent no longer in the job — a launching shell that exited
/// (`npm run dev &`), or a root that exited (a terminal's shell, a
/// respawned CLI) — is kept: those orphans are exactly what the user needs
/// to see.
pub fn agent_started(members: Vec<TrackedProcess>, roots: &HashSet<u32>) -> Vec<TrackedProcess> {
    let by_pid: HashMap<u32, &TrackedProcess> = members.iter().map(|p| (p.pid, p)).collect();
    let keep: HashSet<u32> = members
        .iter()
        .filter(|p| !roots.contains(&p.pid) && image_name(&p.command) != "conhost")
        .filter(|p| {
            let mut cur = *p;
            // Bounded walk: a PID-reuse cycle can't spin forever.
            for _ in 0..=members.len() {
                if is_shell(cur) {
                    return true;
                }
                let Some(parent) = cur.parent_pid else { return true };
                if roots.contains(&parent) {
                    // A root that has exited leaves an orphan like any other
                    // missing parent — keep it (ReAgent P1 on #3430).
                    return by_pid.get(&parent).is_none_or(|r| is_shell(r));
                }
                match by_pid.get(&parent) {
                    Some(next) => cur = next,
                    None => return true,
                }
            }
            false
        })
        .map(|p| p.pid)
        .collect();
    members.into_iter().filter(|p| keep.contains(&p.pid)).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: Option<u32>, command: &str) -> TrackedProcess {
        TrackedProcess {
            pid,
            command: command.to_string(),
            rss_bytes: 0,
            started_at_ms: 0,
            parent_pid: parent,
        }
    }

    fn pids(v: &[TrackedProcess]) -> Vec<u32> {
        let mut p: Vec<u32> = v.iter().map(|p| p.pid).collect();
        p.sort();
        p
    }

    /// The live tree of an idle Claude agent (observed after #3425): nothing
    /// in it was started by the agent.
    fn idle_claude() -> Vec<TrackedProcess> {
        vec![
            proc(10, Some(1), r"C:\Users\u\.local\bin\claude.exe"),
            proc(11, Some(10), r"C:\Windows\System32\conhost.exe"),
            proc(12, Some(10), r"C:\Program Files\AgentMux\agentmux-mcp.exe"),
            proc(13, Some(12), r"C:\Windows\System32\conhost.exe"),
        ]
    }

    #[test]
    fn idle_agent_has_no_agent_started_processes() {
        assert!(agent_started(idle_claude(), &HashSet::from([10])).is_empty());
    }

    #[test]
    fn bash_tool_command_and_its_descendants_count() {
        let mut members = idle_claude();
        members.push(proc(20, Some(10), r"C:\Program Files\Git\bin\bash.exe"));
        members.push(proc(21, Some(20), r"C:\Program Files\nodejs\node.exe"));
        members.push(proc(22, Some(21), r"C:\Program Files\nodejs\node.exe"));
        assert_eq!(pids(&agent_started(members, &HashSet::from([10]))), vec![20, 21, 22]);
    }

    #[test]
    fn mcp_server_and_its_children_do_not_count() {
        let mut members = idle_claude();
        members.push(proc(30, Some(10), r"C:\Program Files\nodejs\node.exe"));
        members.push(proc(31, Some(30), r"C:\Program Files\nodejs\node.exe"));
        assert!(agent_started(members, &HashSet::from([10])).is_empty());
    }

    #[test]
    fn orphan_whose_launching_shell_exited_still_counts() {
        // `npm run dev &` — the bash that launched it is gone, the server lives on.
        let mut members = idle_claude();
        members.push(proc(40, Some(39), r"C:\Program Files\nodejs\node.exe"));
        assert_eq!(pids(&agent_started(members, &HashSet::from([10]))), vec![40]);
    }

    #[test]
    fn orphan_of_an_exited_root_still_counts() {
        // A terminal's shell (root 80) backgrounded a server, then exited.
        let members = vec![proc(81, Some(80), r"C:\Program Files\nodejs\node.exe")];
        assert_eq!(pids(&agent_started(members, &HashSet::from([80]))), vec![81]);
    }

    #[test]
    fn terminal_block_rooted_at_a_shell_counts_its_direct_children() {
        let members = vec![
            proc(50, Some(1), r"C:\Program Files\PowerShell\7\pwsh.exe"),
            proc(51, Some(50), r"C:\Windows\System32\conhost.exe"),
            proc(52, Some(50), r"C:\Program Files\nodejs\node.exe"),
        ];
        assert_eq!(pids(&agent_started(members, &HashSet::from([50]))), vec![52]);
    }

    #[test]
    fn respawned_agent_excludes_every_root() {
        // Old and new CLI PIDs were both assigned; neither is agent-started.
        let mut members = idle_claude();
        members.push(proc(60, Some(1), r"C:\Users\u\.local\bin\claude.exe"));
        assert!(agent_started(members, &HashSet::from([10, 60])).is_empty());
    }

    #[test]
    fn parent_pid_cycle_terminates() {
        let members = vec![proc(70, Some(71), r"C:\x\a.exe"), proc(71, Some(70), r"C:\x\b.exe")];
        assert!(agent_started(members, &HashSet::new()).is_empty());
    }
}
