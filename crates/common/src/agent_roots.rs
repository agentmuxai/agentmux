// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent CLIs srv is running, kept on disk for the launcher.
//!
//! On Unix each agent CLI leads its own process group (srv spawns it with
//! `process_group(0)`), so a signal sent to one CLI's group can't reach srv,
//! and srv's teardown of an agent takes the CLI's MCP servers with it. The
//! cost: when srv itself dies without closing its agents (SIGKILL, a crash,
//! a force-kill after its exit deadline), killing srv's group no longer
//! takes the CLIs. This file is how the launcher finds them then.
//!
//! srv writes `<data_dir>/agent-roots.json` (temp file + rename, so a reader
//! never sees half of it) whenever an agent CLI starts or exits, and deletes
//! it on a clean exit. It names srv's pid, its Linux cgroup scope when it
//! has one, and each CLI by pid **and start time**. Once srv has stopped,
//! [`kill_orphans`] ends what is left:
//!
//! - Linux with srv's delegated scope: `cgroup.kill` on each agent cgroup
//!   (`agent-*`) in it, which holds everything that agent started however it
//!   detached.
//! - Everywhere on Unix, and as the fallback: `kill(-pid, SIGKILL)` for each
//!   recorded CLI that is still the same process (its start time matches)
//!   and still leads its own group. A pid alone is never signalled: it may
//!   have been reused since srv wrote it.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name, in srv's data dir next to `srv.lock`.
pub const FILE_NAME: &str = "agent-roots.json";

/// What srv writes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoots {
    /// The srv that wrote it; [`kill_orphans`] acts only for this srv.
    pub srv_pid: u32,
    /// srv's delegated cgroup scope (Linux), holding one `agent-*` cgroup
    /// per agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cgroup: Option<PathBuf>,
    /// Each running agent CLI.
    #[serde(default)]
    pub roots: Vec<Root>,
}

/// One agent CLI: a pid, and when that process started
/// (`process::process_start_time`), so a reused pid is never mistaken for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Root {
    pub pid: u32,
    pub start: u64,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Replace the file atomically: write a temp file beside it, then rename.
pub fn write(data_dir: &Path, roots: &AgentRoots) -> io::Result<()> {
    let target = path(data_dir);
    let tmp = data_dir.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let body = serde_json::to_vec(roots).map_err(io::Error::other)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn read(data_dir: &Path) -> Option<AgentRoots> {
    serde_json::from_slice(&std::fs::read(path(data_dir)).ok()?).ok()
}

/// Delete the file (srv's clean exit). Already gone is fine.
pub fn remove(data_dir: &Path) {
    let _ = std::fs::remove_file(path(data_dir));
}

/// What [`kill_orphans`] did, for the caller's log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// Agent cgroups `cgroup.kill` was written to.
    pub cgroups: usize,
    /// Recorded CLIs that were still running and had their group killed.
    pub groups: usize,
    /// Recorded CLIs that had already exited (or whose pid is now someone
    /// else's): left alone.
    pub gone: usize,
}

/// End the agent CLIs a stopped srv (`srv_pid`) left running, then delete
/// the file. Call only once that srv has exited. `None` when there is no
/// file, or it was written by another srv (left in place).
#[cfg(unix)]
pub fn kill_orphans(data_dir: &Path, srv_pid: u32) -> Option<Cleanup> {
    let roots = read(data_dir)?;
    if roots.srv_pid != srv_pid {
        return None;
    }
    let mut done = Cleanup::default();
    #[cfg(target_os = "linux")]
    if let Some(scope) = roots.cgroup.as_deref().filter(|s| is_srv_scope(s, srv_pid)) {
        done.cgroups = kill_agent_cgroups(scope);
    }
    for root in &roots.roots {
        let leader_alive = crate::process::process_start_time(root.pid) == Some(root.start);
        if (leader_alive && crate::process::leads_own_group(root.pid)) || crate::process::orphaned_group_alive(root.pid) {
            if crate::process::signal_process_group(root.pid, libc::SIGKILL).is_ok() {
                done.groups += 1;
            }
        } else {
            done.gone += 1;
        }
    }
    remove(data_dir);
    Some(done)
}

/// Whether a recorded CLI's group is still there to end: its leader is still
/// the process that was recorded (same start time), or the leader is gone but
/// its helpers still hold the group (`process::orphaned_group_alive`). A pid
/// that now belongs to some other process is not ours.
#[cfg(unix)]
pub fn is_alive(root: &Root) -> bool {
    match crate::process::process_start_time(root.pid) {
        Some(start) => start == root.start,
        None => crate::process::orphaned_group_alive(root.pid),
    }
}

/// Only srv's own scope (`agentmux-srv-<pid>.scope`) under the cgroup
/// mount: the path comes from a file, so it must not name anything else.
#[cfg(target_os = "linux")]
fn is_srv_scope(scope: &Path, srv_pid: u32) -> bool {
    scope.starts_with("/sys/fs/cgroup")
        && !scope.components().any(|c| matches!(c, std::path::Component::ParentDir))
        && scope.file_name().is_some_and(|n| n == format!("agentmux-srv-{srv_pid}.scope").as_str())
}

/// Write `1` to `cgroup.kill` in every `agent-*` cgroup in `scope`; srv's
/// own cgroup and anything else in the scope are left alone. Returns how
/// many were killed. Public for srv's cgroup tests.
#[cfg(target_os = "linux")]
pub fn kill_agent_cgroups(scope: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(scope) else { return 0 };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("agent-") && e.path().is_dir())
        .filter(|e| std::fs::write(e.path().join("cgroup.kill"), "1").is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agent-roots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_then_read_round_trips_and_leaves_no_temp_file() {
        let dir = temp_dir("roundtrip");
        let roots = AgentRoots { srv_pid: 42, cgroup: None, roots: vec![Root { pid: 7, start: 9 }] };
        write(&dir, &roots).unwrap();
        assert_eq!(read(&dir), Some(roots));
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, [FILE_NAME]);
        remove(&dir);
        assert_eq!(read(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_srvs_own_scope_is_accepted() {
        let ok = Path::new("/sys/fs/cgroup/user.slice/user@1000.service/app.slice/agentmux-srv-77.scope");
        assert!(is_srv_scope(ok, 77));
        assert!(!is_srv_scope(ok, 78), "another srv's scope");
        assert!(!is_srv_scope(Path::new("/sys/fs/cgroup/user.slice"), 77));
        assert!(!is_srv_scope(Path::new("/tmp/agentmux-srv-77.scope"), 77));
        assert!(!is_srv_scope(Path::new("/sys/fs/cgroup/../tmp/agentmux-srv-77.scope"), 77));
    }

    /// A CLI that exited while its helper lives on: the group is still ended.
    #[cfg(unix)]
    #[test]
    fn kill_orphans_ends_the_helpers_of_a_cli_that_already_exited() {
        use std::io::BufRead as _;
        use std::os::unix::process::CommandExt as _;
        let dir = temp_dir("leaderless");
        let mut cli = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & echo $!"])
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let start = crate::process::process_start_time(cli.id()).unwrap();
        let roots = AgentRoots { srv_pid: 4242, cgroup: None, roots: vec![Root { pid: cli.id(), start }] };
        write(&dir, &roots).unwrap();
        let mut line = String::new();
        std::io::BufReader::new(cli.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let helper: u32 = line.trim().parse().unwrap();
        cli.wait().unwrap();
        assert!(is_alive(&roots.roots[0]), "kept while the helper holds the group");

        let done = kill_orphans(&dir, 4242).unwrap();
        assert_eq!(done.groups, 1);
        for _ in 0..100 {
            if crate::process::process_start_time(helper).is_none() {
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the helper {helper} survived the launcher's cleanup");
    }

    /// The launcher's fallback after srv died: a recorded CLI still running
    /// is killed with its group; one whose start time no longer matches (a
    /// reused pid) is left alone; a file from another srv is ignored.
    #[cfg(unix)]
    #[test]
    fn kill_orphans_kills_only_the_recorded_process() {
        use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
        let dir = temp_dir("orphans");
        let spawn = || std::process::Command::new("sleep").arg("30").process_group(0).spawn().unwrap();
        let mut cli = spawn();
        let mut bystander = spawn();
        let start = crate::process::process_start_time(cli.id()).unwrap();
        let other_start = crate::process::process_start_time(bystander.id()).unwrap();
        let roots = AgentRoots {
            srv_pid: 4242,
            cgroup: None,
            roots: vec![Root { pid: cli.id(), start }, Root { pid: bystander.id(), start: other_start + 1 }],
        };
        write(&dir, &roots).unwrap();

        assert_eq!(kill_orphans(&dir, 4243), None, "another srv's file");
        assert!(read(&dir).is_some(), "left in place");

        let done = kill_orphans(&dir, 4242).unwrap();
        assert_eq!(done, Cleanup { cgroups: 0, groups: 1, gone: 1 });
        assert_eq!(cli.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert_eq!(bystander.try_wait().unwrap(), None, "a pid whose start time differs is not touched");
        assert_eq!(read(&dir), None, "deleted after cleanup");

        bystander.kill().unwrap();
        bystander.wait().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
