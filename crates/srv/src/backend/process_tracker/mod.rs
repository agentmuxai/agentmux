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
//! The API is a platform-agnostic trait, one impl per platform. Terminals a
//! person types in get the no-op `StubTracker` everywhere but Windows (see
//! `AgentProcessRegistry::ensure_tracker_kind`).
//!
//! | Platform | Impl            | Mechanism                                  | Confidence |
//! |----------|-----------------|--------------------------------------------|------------|
//! | Windows  | `JobObjectTracker` | `CreateJobObject` + `AssignProcessToJobObject` + `TerminateJobObject` | high       |
//! | Linux    | `CgroupTracker` | cgroup v2 per agent in srv's delegated user scope; `cgroup.procs` / `cgroup.kill` | high |
//! | Linux, no delegated cgroup; macOS | `ScanTracker` | process table: descendants of the CLI + the inherited `AGENTMUX_BLOCKID` env tag | best effort |
//! | other    | `StubTracker`   | no-op                                                          | none       |
//!
//! The frontend's swarm panel surfaces the confidence level so users know
//! when tracking may miss escaped descendants.
//!
//! See `docs/analysis/agent-spawned-process-tracking-2026-10-07.md`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub mod registry;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod cgroup_linux;

pub mod scan;

// `stub` is defined inline below; there is no `stub.rs`. A file-form
// `pub mod stub;` here would collide with it (E0428) and break `task dev`.

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
    /// The executable alone (path or name), which [`agent_started`]
    /// classifies by: `command` may be a whole argv (`/bin/bash -c npm
    /// test`). Empty means `command` is the executable.
    #[serde(skip)]
    pub exe: String,
}

impl TrackedProcess {
    fn image(&self) -> String {
        image_name(if self.exe.is_empty() { &self.command } else { &self.exe })
    }
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
        p.image().as_str(),
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
        .filter(|p| !roots.contains(&p.pid) && p.image() != "conhost")
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
    /// single Job Object query; the planned Linux and macOS impls would
    /// read `cgroup.procs` and scan sysctl respectively.
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

    /// Run every process in this tree at below-normal CPU priority (`on`), or
    /// back at normal. Descendants inherit it and cannot raise it themselves.
    /// Windows only (a Job Object priority-class limit); a no-op elsewhere.
    /// See `docs/analysis/ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26.md` §8.3.
    fn set_below_normal_priority(&self, _on: bool) -> Result<(), String> {
        Ok(())
    }

    /// Ask every member to exit (SIGTERM): the graceful step a teardown
    /// takes before [`kill_tree`](Self::kill_tree). `false` where the
    /// platform has no graceful signal for a whole tree yet (Windows), so
    /// there is nothing to wait for.
    fn terminate(&self) -> bool {
        false
    }

    /// How many processes the tree holds right now, the agent's own CLI and
    /// plumbing included: what a teardown waits to reach zero.
    fn member_count(&self) -> usize {
        self.list_members().len()
    }

    /// The tree's PIDs alone, plumbing included, without the per-process
    /// enrichment `list_members` does: what Tower reads every refresh
    /// (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §5).
    fn member_pids(&self) -> Vec<u32> {
        self.list_members().into_iter().map(|p| p.pid).collect()
    }

    /// CPU time the whole tree has used, in nanoseconds, **including members
    /// that have exited** (a Job Object's accounting, a cgroup's `cpu.stat`),
    /// so a burst of short-lived build processes isn't lost between two
    /// samples. `None` where the platform keeps no such account.
    fn cpu_time_ns(&self) -> Option<u64> {
        None
    }

    /// Where a child about to be spawned joins the tree before it execs
    /// (Linux: the cgroup's `cgroup.procs`), so nothing it forks can slip
    /// out between spawn and [`assign_process`](Self::assign_process).
    /// `None`: only the after-spawn assignment.
    fn spawn_target(&self) -> Option<std::path::PathBuf> {
        None
    }
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
/// `agent`: the block runs an agent, not a terminal a person types in (see
/// `AgentProcessRegistry::ensure_tracker_kind`). Every Unix tells them apart;
/// Windows gives both a Job Object.
pub fn new_tracker(block_id: &str, agent: bool) -> Arc<dyn TrackerHandle> {
    #[cfg(windows)]
    {
        match windows::JobObjectTracker::new(block_id) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                tracing::error!(
                    block_id = %block_id,
                    error = %e,
                    "[process-tracker] JobObjectTracker init failed — falling back to best-effort tracking"
                );
                if agent {
                    Arc::new(scan::ScanTracker::new(block_id))
                } else {
                    Arc::new(stub::StubTracker)
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(root) = cgroup_linux::root().filter(|_| agent) {
            match cgroup_linux::CgroupTracker::new(root, block_id) {
                Ok(t) => return Arc::new(t),
                Err(e) => tracing::warn!(block_id = %block_id, error = %e, "[process-tracker] cgroup tracker init failed"),
            }
        }
        if agent {
            return Arc::new(scan::ScanTracker::new(block_id));
        }
        Arc::new(stub::StubTracker)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        if agent {
            return Arc::new(scan::ScanTracker::new(block_id));
        }
        Arc::new(stub::StubTracker)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (block_id, agent);
        Arc::new(stub::StubTracker)
    }
}

pub mod stub {
    //! No-op tracker used on unsupported platforms, and on Windows when
    //! `JobObjectTracker::new` fails (e.g. the process is not elevated enough
    //! to create a job object). All operations succeed silently;
    //! `list_members` always returns empty. Confidence reports `None` so the
    //! UI can inform the user that tracking is disabled.

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
            exe: String::new(),
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

    /// Linux and macOS report the whole argv as `command`; the shell is
    /// recognised from `exe`.
    #[test]
    fn a_shell_reported_with_its_argv_is_still_a_shell() {
        let argv = |pid, parent, exe: &str, command: &str| TrackedProcess { exe: exe.to_string(), ..proc(pid, parent, command) };
        let members = vec![
            argv(10, Some(1), "/usr/bin/node", "node /usr/lib/claude/cli.js --input-format stream-json"),
            argv(20, Some(10), "/bin/bash", "/bin/bash -c npm test"),
            argv(21, Some(20), "/usr/bin/node", "node /repo/node_modules/.bin/vitest"),
        ];
        assert_eq!(pids(&agent_started(members, &HashSet::from([10]))), vec![20, 21]);
    }

    #[test]
    fn parent_pid_cycle_terminates() {
        let members = vec![proc(70, Some(71), r"C:\x\a.exe"), proc(71, Some(70), r"C:\x\b.exe")];
        assert!(agent_started(members, &HashSet::new()).is_empty());
    }
}
