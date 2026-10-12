// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reclaim the data dir from an srv that an earlier launcher left behind.
//!
//! srv takes an exclusive `flock` on `<data_dir>/srv.lock` and writes its PID
//! into it (`crates/srv/src/backend/base.rs`, `acquire_data_dir_lock`). If an
//! srv outlives its launcher, every later launch's srv fails on that lock and
//! the app cannot be opened (2026-10-11: macOS "Quit & Reopen" left one
//! behind). By the time the launcher spawns srv it has already won its
//! single-instance socket, so no live launcher owns this data dir: an srv still
//! holding the lock is stale. Stop it, and start fresh.
//!
//! Only a process that is an `agentmux-srv` binary is stopped, and never one
//! started headless (`--headless` / `AGENTMUX_HEADLESS=1`), which runs on its
//! own by design.

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
    let Some(command) = process_command(pid) else {
        crate::log(&format!("stale-srv: lock names pid {pid}, which isn't running; leaving it"));
        return;
    };
    if !command.contains("agentmux-srv") {
        crate::log(&format!("stale-srv: lock names pid {pid}, which isn't agentmux-srv ({command}); leaving it"));
        return;
    }
    if command.contains("--headless") || command.contains("AGENTMUX_HEADLESS=1") {
        crate::log(&format!("stale-srv: pid {pid} is a headless srv using this data dir; leaving it"));
        return;
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

/// The process' command line with its environment (`ps eww`), or `None` if it
/// isn't running.
fn process_command(pid: i32) -> Option<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(["eww", "-o", "command=", "-p", &pid.to_string()])
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
