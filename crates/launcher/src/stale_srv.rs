// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reclaim the data dir from an srv that an earlier launcher left behind.
//!
//! srv takes an exclusive `flock` on `<data_dir>/srv.lock` and writes its PID
//! into it (`crates/srv/src/backend/base.rs`, `acquire_data_dir_lock`). If an
//! srv outlives its launcher, every later launch's srv fails on that lock and
//! the app cannot be opened (2026-10-11: macOS "Quit & Reopen" left one
//! behind). An srv still holding the lock whose launcher is gone (it was
//! reparented to launchd/init) is stale: stop it, and start fresh.
//!
//! Only a process that is an `agentmux-srv` binary is stopped, and never one
//! started headless (`--headless` / `AGENTMUX_HEADLESS=1`), which runs on its
//! own by design, and never one whose parent is still an AgentMux launcher:
//! winning the single-instance socket isn't proof enough on its own, since two
//! local builds of one checkout can share a data dir with different sockets.

use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

/// Name of srv's data-dir lock file. Mirrors `SRV_DATA_LOCK_FILE` in
/// `crates/srv/src/backend/base.rs`.
const SRV_LOCK_FILE: &str = "srv.lock";

/// How long a stale srv gets to exit after SIGTERM (it closes its agents
/// first) before its whole process group is killed.
const TERM_GRACE: Duration = Duration::from_secs(10);

/// If `<data_dir>/srv.lock` is held, stop the stale srv holding it. Returns
/// once the lock is free, or the holder can't be stopped (the srv spawn that
/// follows then fails and reports it as before).
pub fn reclaim_stale_srv_lock(data_dir: &Path) {
    let lock_path = data_dir.join(SRV_LOCK_FILE);
    if lock_is_free(&lock_path) {
        return;
    }
    let Some(pid) = std::fs::read_to_string(&lock_path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    else {
        crate::log(&format!(
            "stale-srv: {} is held but names no PID (an srv from before PID recording?); leaving it",
            lock_path.display()
        ));
        return;
    };
    // `comm` is the executable as one field (a full path on macOS, which can
    // hold spaces: `/Volumes/AgentMux 3/...`; the first 15 characters of the
    // name on Linux). Only its file name ever goes into the log: another
    // process' arguments and environment can hold secrets.
    let Some(comm) = ps_field(pid, "comm=", false) else {
        crate::log(&format!("stale-srv: lock names pid {pid}, which isn't running; leaving it"));
        return;
    };
    let exe_name = Path::new(&comm)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !exe_name.starts_with("agentmux-srv") {
        crate::log(&format!("stale-srv: lock names pid {pid} ({exe_name}), which isn't agentmux-srv; leaving it"));
        return;
    }
    let headless_env = ps_field(pid, "command=", true)
        .is_some_and(|with_env| with_env.split_whitespace().any(|w| w == "AGENTMUX_HEADLESS=1" || w == "AGENTMUX_HEADLESS=true"));
    let headless_arg = ps_field(pid, "command=", false)
        .is_some_and(|args| args.split_whitespace().any(|a| a == "--headless"));
    if headless_arg || headless_env {
        crate::log(&format!("stale-srv: pid {pid} is a headless srv using this data dir; leaving it"));
        return;
    }

    // Only an srv whose launcher is really gone: winning this launcher's
    // single-instance socket doesn't prove nobody else owns the data dir (two
    // local builds of one checkout have different socket hashes but the same
    // data dir). An orphan is reparented to launchd/init (PPID 1); one whose
    // parent is still an AgentMux process belongs to a running app.
    let ppid = ps_field(pid, "ppid=", false).and_then(|p| p.trim().parse::<i32>().ok());
    if let Some(ppid) = ppid.filter(|p| *p > 1) {
        let parent = ps_field(ppid, "comm=", false)
            .and_then(|c| Path::new(&c).file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        if parent.to_ascii_lowercase().contains("agentmux") {
            crate::log(&format!(
                "stale-srv: srv pid {pid} still has its launcher (pid {ppid}, {parent}); another instance owns this data dir, leaving it"
            ));
            return;
        }
    }

    crate::log(&format!(
        "stale-srv: srv pid {pid} still holds {} with no launcher; stopping it so this launch can start",
        lock_path.display()
    ));
    unsafe { libc::kill(pid, libc::SIGTERM) };
    if wait_for_free(&lock_path, TERM_GRACE) {
        crate::log(&format!("stale-srv: pid {pid} exited after SIGTERM; lock is free"));
        return;
    }
    // srv leads its own process group (srv_spawner's process_group(0)), which
    // holds its agent CLIs; take them down with it.
    let target = if unsafe { libc::getpgid(pid) } == pid { -pid } else { pid };
    unsafe { libc::kill(target, libc::SIGKILL) };
    if wait_for_free(&lock_path, Duration::from_secs(3)) {
        crate::log(&format!("stale-srv: pid {pid} killed; lock is free"));
    } else {
        crate::log(&format!("stale-srv: lock still held after killing pid {pid}"));
    }
}

/// True when nobody holds the lock (or the file doesn't exist). Takes and
/// immediately releases the lock; the srv spawned next takes it for real.
fn lock_is_free(lock_path: &Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new().read(true).open(lock_path) else {
        return true;
    };
    let fd = file.as_raw_fd();
    if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        true
    } else {
        false
    }
}

fn wait_for_free(lock_path: &Path, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if lock_is_free(lock_path) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    lock_is_free(lock_path)
}

/// One `ps` field for `pid` (`ww`: untruncated; `with_env`: append the
/// process' environment, `ps e`), or `None` if it isn't running. Never log
/// the result: it can hold another process' secrets.
fn ps_field(pid: i32, field: &str, with_env: bool) -> Option<String> {
    let flags = if with_env { "eww" } else { "ww" };
    let out = std::process::Command::new("/bin/ps")
        .args([flags, "-o", field, "-p", &pid.to_string()])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_unheld_lock_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join(SRV_LOCK_FILE);
        assert!(lock_is_free(&lock), "missing file");
        std::fs::write(&lock, "12345").unwrap();
        assert!(lock_is_free(&lock), "file nobody holds");
    }

    #[test]
    fn a_held_lock_is_not_free_and_is_left_alone_without_a_live_srv() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join(SRV_LOCK_FILE);
        let holder = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lock)
            .unwrap();
        assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
        assert!(!lock_is_free(&lock));

        // The lock names this test process, which isn't agentmux-srv: nothing
        // may be signalled, and the lock stays held.
        std::fs::write(&lock, std::process::id().to_string()).unwrap();
        reclaim_stale_srv_lock(dir.path());
        assert!(!lock_is_free(&lock));
    }
}




