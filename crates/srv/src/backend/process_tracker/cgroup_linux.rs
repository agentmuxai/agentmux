// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux tracker: one cgroup v2 per agent, under a systemd user scope srv
//! puts itself in at startup with `Delegate=yes`, so srv may create and
//! manage cgroups below it.
//!
//! A cgroup holds every descendant of what it was given, whatever they do:
//! `setsid`, double forks and reparenting don't leave it (process groups and
//! sessions can't hold an agent's tree: Claude Code and bashwrap each start
//! new sessions). `cgroup.kill` ends all of them at once.
//! See `docs/analysis/agent-spawned-process-tracking-2026-10-07.md`.

use std::path::{Path, PathBuf};

use super::{TrackedProcess, TrackerHandle, TrackingConfidence};

const CGROUP_FS: &str = "/sys/fs/cgroup";

/// srv's delegated scope directory, or `None` when it couldn't get one (no
/// cgroup v2, no systemd user manager): trackers then fall back.
static ROOT: tokio::sync::OnceCell<Option<PathBuf>> = tokio::sync::OnceCell::const_new();

/// The directory per-agent cgroups are created in, once [`init`] succeeded.
pub fn root() -> Option<&'static Path> {
    ROOT.get().and_then(|r| r.as_deref())
}

/// Move srv into its own delegated systemd user scope
/// (`agentmux-srv-<pid>.scope`). Call once at startup, before any agent
/// spawns. Idempotent; never fails startup.
pub async fn init() {
    ROOT.get_or_init(|| async {
        match setup().await {
            Ok(dir) => {
                tracing::info!(cgroup = %dir.display(), "[process-tracker] agent processes tracked by cgroup");
                Some(dir)
            }
            Err(e) => {
                tracing::warn!(error = %e, "[process-tracker] no delegated cgroup; agent process tracking falls back");
                None
            }
        }
    })
    .await;
}

async fn setup() -> Result<PathBuf, String> {
    if !Path::new(CGROUP_FS).join("cgroup.controllers").exists() {
        return Err("cgroup v2 is not mounted at /sys/fs/cgroup".into());
    }
    let unit = format!("agentmux-srv-{}.scope", std::process::id());
    if !own_cgroup()?.ends_with(&format!("/{unit}")) {
        start_transient_scope(&unit).await?;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !own_cgroup()?.ends_with(&format!("/{unit}")) {
            if std::time::Instant::now() >= until {
                return Err(format!("systemd did not move srv into {unit}"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    let dir = Path::new(CGROUP_FS).join(own_cgroup()?.trim_start_matches('/'));
    // Prove delegation: a child cgroup can be made and removed.
    let probe = dir.join("probe");
    std::fs::create_dir(&probe).map_err(|e| format!("create a cgroup in {}: {e}", dir.display()))?;
    let _ = std::fs::remove_dir(&probe);
    Ok(dir)
}

/// This process's cgroup v2 path (`/user.slice/…`).
fn own_cgroup() -> Result<String, String> {
    let text = std::fs::read_to_string("/proc/self/cgroup").map_err(|e| format!("read /proc/self/cgroup: {e}"))?;
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(str::to_string)
        .ok_or_else(|| "no cgroup v2 entry in /proc/self/cgroup".into())
}

/// Ask the user's systemd manager for a transient scope holding this
/// process, delegated to us (what `systemd-run --user --scope -p
/// Delegate=yes` does).
async fn start_transient_scope(unit: &str) -> Result<(), String> {
    use zbus::zvariant::Value;
    let conn = zbus::Connection::session().await.map_err(|e| format!("session bus: {e}"))?;
    let props: Vec<(&str, Value)> = vec![
        ("Description", Value::from("AgentMux srv and the agents it runs")),
        ("PIDs", Value::from(vec![std::process::id()])),
        ("Delegate", Value::from(true)),
    ];
    let aux: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();
    conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StartTransientUnit",
        &(unit, "fail", props, aux),
    )
    .await
    .map_err(|e| format!("StartTransientUnit {unit}: {e}"))?;
    Ok(())
}

/// One agent's cgroup.
pub struct CgroupTracker {
    dir: PathBuf,
}

impl CgroupTracker {
    pub fn new(root: &Path, block_id: &str) -> Result<Self, String> {
        let name: String = block_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let dir = root.join(format!("agent-{name}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("create {}: {e}", dir.display())),
        }
        Ok(Self { dir })
    }

    fn pids(&self) -> Vec<u32> {
        pids_in(&self.dir)
    }
}

fn pids_in(dir: &Path) -> Vec<u32> {
    std::fs::read_to_string(dir.join("cgroup.procs"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn populated(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("cgroup.events"))
        .map(|t| t.lines().any(|l| l == "populated 1"))
        .unwrap_or(false)
}

/// Kill everything in `dir`: `cgroup.kill` (kernel 5.14+), else SIGKILL each
/// member until none is left.
fn kill_all(dir: &Path) {
    if std::fs::write(dir.join("cgroup.kill"), "1").is_ok() {
        return;
    }
    for _ in 0..5 {
        let pids = pids_in(dir);
        if pids.is_empty() {
            return;
        }
        for pid in pids {
            signal(pid, libc::SIGKILL);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn signal(pid: u32, sig: libc::c_int) {
    // SAFETY: kill(2) with a pid read from cgroup.procs and a constant signal.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

impl TrackerHandle for CgroupTracker {
    fn assign_process(&self, pid: u32) -> Result<(), String> {
        std::fs::write(self.dir.join("cgroup.procs"), pid.to_string())
            .map_err(|e| format!("move pid {pid} into {}: {e}", self.dir.display()))
    }

    fn list_members(&self) -> Vec<TrackedProcess> {
        let pids = self.pids();
        if pids.is_empty() {
            return Vec::new();
        }
        let list: Vec<sysinfo::Pid> = pids.iter().map(|p| sysinfo::Pid::from_u32(*p)).collect();
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&list),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always).with_memory(),
        );
        sys.processes()
            .iter()
            .map(|(pid, p)| {
                let cmd: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
                TrackedProcess {
                    pid: pid.as_u32(),
                    command: if cmd.is_empty() { p.name().to_string_lossy().into_owned() } else { cmd.join(" ") },
                    rss_bytes: p.memory(),
                    started_at_ms: p.start_time() * 1000,
                    parent_pid: p.parent().map(|pp| pp.as_u32()),
                    exe: cmd.first().cloned().unwrap_or_else(|| p.name().to_string_lossy().into_owned()),
                }
            })
            .collect()
    }

    fn kill_tree(&self) {
        kill_all(&self.dir);
    }

    fn kill_pid(&self, pid: u32) -> bool {
        if !self.pids().contains(&pid) {
            return false;
        }
        signal(pid, libc::SIGKILL);
        true
    }

    fn confidence(&self) -> TrackingConfidence {
        TrackingConfidence::High
    }

    fn terminate(&self) -> bool {
        for pid in self.pids() {
            signal(pid, libc::SIGTERM);
        }
        true
    }

    fn member_count(&self) -> usize {
        self.pids().len()
    }

    fn spawn_target(&self) -> Option<PathBuf> {
        Some(self.dir.join("cgroup.procs"))
    }
}

impl Drop for CgroupTracker {
    /// The agent is gone: end whatever it left and remove its cgroup, which
    /// the kernel allows only once it's empty.
    fn drop(&mut self) {
        let dir = self.dir.clone();
        kill_all(&dir);
        std::thread::spawn(move || {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while populated(&dir) && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if let Err(e) = std::fs::remove_dir(&dir) {
                tracing::warn!(cgroup = %dir.display(), error = %e, "[process-tracker] could not remove an agent's cgroup");
            }
        });
    }
}

/// Make a child join `procs` (a `cgroup.procs` file) before it execs, so it
/// is tracked before it runs any code of its own and before it can fork.
pub fn join_before_exec(cmd: &mut tokio::process::Command, procs: PathBuf) {
    let Ok(path) = std::ffi::CString::new(procs.into_os_string().into_encoded_bytes()) else { return };
    // SAFETY: the hook only calls open/write/close, which are
    // async-signal-safe, on memory allocated before the fork.
    unsafe {
        cmd.pre_exec(move || {
            let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
            if fd >= 0 {
                // "0" moves the writing process: this child.
                libc::write(fd, b"0".as_ptr().cast(), 1);
                libc::close(fd);
            }
            // Failing to join is not fatal: the parent assigns the PID right
            // after spawn as well.
            Ok(())
        });
    }
}

/// [`join_before_exec`] for a PTY spawn (portable-pty has no pre-exec hook):
/// run the command through `sh`, which joins and then `exec`s it, so the
/// program starts already in the cgroup, as the same process.
pub fn join_before_exec_pty(cmd: &mut portable_pty::CommandBuilder, procs: PathBuf) {
    let argv = cmd.get_argv_mut();
    if argv.is_empty() {
        return; // the user's default shell: never an agent
    }
    let mut wrapped: Vec<std::ffi::OsString> = vec![
        "/bin/sh".into(),
        "-c".into(),
        r#"echo 0 > "$0" 2>/dev/null; exec "$@""#.into(),
        procs.into_os_string(),
    ];
    wrapped.append(argv);
    *argv = wrapped;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A delegated scope for this test binary, or `None` where there is no
    /// systemd user manager (CI containers): the tests then skip.
    async fn test_root() -> Option<&'static Path> {
        init().await;
        root()
    }

    fn sh(script: &str, procs: Option<PathBuf>) -> tokio::process::Child {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", script]).kill_on_drop(true);
        if let Some(p) = procs {
            join_before_exec(&mut cmd, p);
        }
        cmd.spawn().expect("spawn sh")
    }

    async fn wait_for(mut f: impl FnMut() -> bool) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !f() {
            if std::time::Instant::now() >= until {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        true
    }

    /// The case process groups can't hold: a `setsid` daemon, a double
    /// fork, and a plain child all stay in the agent's cgroup, are listed,
    /// and end with `kill_tree`.
    #[tokio::test]
    async fn every_descendant_is_tracked_and_killed_however_it_detaches() {
        let Some(root) = test_root().await else {
            eprintln!("skipped: no delegated cgroup here");
            return;
        };
        let tracker = CgroupTracker::new(root, "test-detaching-descendants").unwrap();
        let mut child = sh(
            "setsid sleep 3001 & (sleep 3002 &); sleep 3003 & echo started; wait",
            tracker.spawn_target(),
        );
        assert!(wait_for(|| tracker.member_count() >= 4).await, "members: {:?}", tracker.pids());
        let listed: Vec<String> = tracker.list_members().into_iter().map(|p| p.command).collect();
        for s in ["sleep 3001", "sleep 3002", "sleep 3003"] {
            assert!(listed.iter().any(|c| c == s), "{s} not listed in {listed:?}");
        }
        tracker.kill_tree();
        assert!(wait_for(|| !populated(&tracker.dir)).await, "left: {:?}", tracker.pids());
        let _ = child.wait().await;
    }

    /// `terminate` asks politely: a process that exits on SIGTERM is gone
    /// without a kill.
    #[tokio::test]
    async fn terminate_sends_sigterm_to_every_member() {
        let Some(root) = test_root().await else { return };
        let tracker = CgroupTracker::new(root, "test-terminate").unwrap();
        let _child = sh("setsid sleep 3011 & sleep 3012", tracker.spawn_target());
        assert!(wait_for(|| tracker.member_count() >= 2).await);
        tracker.terminate();
        assert!(wait_for(|| tracker.member_count() == 0).await, "left: {:?}", tracker.pids());
    }

    /// Dropping the tracker (the agent's block is gone) ends what's left and
    /// removes the cgroup.
    #[tokio::test]
    async fn drop_kills_what_is_left_and_removes_the_cgroup() {
        let Some(root) = test_root().await else { return };
        let tracker = CgroupTracker::new(root, "test-drop").unwrap();
        let dir = tracker.dir.clone();
        let _child = sh("trap '' TERM; setsid sleep 3021 & sleep 3022", tracker.spawn_target());
        assert!(wait_for(|| pids_in(&dir).len() >= 2).await);
        drop(tracker);
        assert!(wait_for(|| !dir.exists()).await, "cgroup still there: {:?}", pids_in(&dir));
    }

    /// A PTY-style spawn wrapped by `join_before_exec_pty` starts in the
    /// cgroup as the same process, with its own argv.
    #[tokio::test]
    async fn a_wrapped_pty_command_joins_before_it_runs() {
        let Some(root) = test_root().await else { return };
        let tracker = CgroupTracker::new(root, "test-pty-wrap").unwrap();
        let mut b = portable_pty::CommandBuilder::new("sleep");
        b.arg("3041");
        join_before_exec_pty(&mut b, tracker.spawn_target().unwrap());
        let argv: Vec<String> = b.get_argv().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let mut child = tokio::process::Command::new(&argv[0]).args(&argv[1..]).kill_on_drop(true).spawn().unwrap();
        let pid = child.id().unwrap();
        assert!(wait_for(|| tracker.pids() == vec![pid]).await, "members: {:?}", tracker.pids());
        let listed = tracker.list_members();
        assert_eq!(listed.first().map(|p| p.command.as_str()), Some("sleep 3041"), "exec'd in place");
        tracker.kill_tree();
        let _ = child.wait().await;
    }

    /// `assign_process` after spawn tracks a child spawned without the
    /// pre-exec hook (the PTY path), and `kill_pid` only touches members.
    #[tokio::test]
    async fn assign_after_spawn_and_kill_pid_only_members() {
        let Some(root) = test_root().await else { return };
        let tracker = CgroupTracker::new(root, "test-assign").unwrap();
        let mut child = sh("sleep 3031", None);
        let pid = child.id().unwrap();
        tracker.assign_process(pid).unwrap();
        assert_eq!(tracker.pids(), vec![pid]);
        assert!(!tracker.kill_pid(1), "pid 1 is not a member");
        assert!(tracker.kill_pid(pid));
        let _ = child.wait().await;
        assert!(wait_for(|| tracker.member_count() == 0).await);
    }
}
