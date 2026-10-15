// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A terminal for a session: a PTY with the user's login shell in it, in its
//! own session and process group, so ending it ends everything it started.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub struct Pty {
    /// The terminal's controlling side: read output, write input.
    pub master: File,
    pub child: Child,
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// Start the user's login shell (`$SHELL`, else `/bin/sh`) in a new PTY of
/// `cols` x `rows`, in their home directory.
pub fn spawn_login_shell(cols: u16, rows: u16) -> io::Result<Pty> {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let ws = winsize(cols, rows);
    // SAFETY: openpty writes two valid descriptors on success; null name and
    // termios are allowed. `*mut` pointers: macOS declares them `*mut`, Linux
    // `*const`, and `*mut` coerces to `*const`.
    let mut ws = ws;
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just returned by openpty and are owned here.
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    set_cloexec(master.as_raw_fd());

    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let name = shell.rsplit('/').next().unwrap_or("sh").to_string();
    let mut cmd = Command::new(&shell);
    // A leading '-' in argv[0] is how every shell knows it is a login shell.
    cmd.arg0(format!("-{name}"));
    cmd.env("TERM", "xterm-256color")
        .env("TERM_PROGRAM", "agentmux");
    if let Some(home) = std::env::var_os("HOME") {
        cmd.current_dir(home);
    }
    cmd.stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            // stdin is the PTY's slave side: make it this session's terminal.
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok(Pty { master, child })
}

/// Tell the terminal its new size.
pub fn resize(master: &File, cols: u16, rows: u16) {
    let ws = winsize(cols, rows);
    // SAFETY: TIOCSWINSZ reads a winsize from the pointer.
    unsafe {
        libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws);
    }
}

/// How long [`end_group`] gives a hung-up group before killing what is left.
pub const HANGUP_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// How many [`Ending`]s are still open, for [`wait_for_endings`].
static ENDINGS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
static ENDINGS_DONE: std::sync::Condvar = std::sync::Condvar::new();

/// A session being ended, from the moment it is decided until its group's
/// kill is sent (or none is needed): the daemon must not exit in between.
pub struct Ending(());

impl Ending {
    pub fn start() -> Self {
        *ENDINGS.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        Ending(())
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        *ENDINGS.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        ENDINGS_DONE.notify_all();
    }
}

/// End a session: hang up its shell's process group and every other process
/// in the shell's session (a background job has a group of its own), then
/// kill what is left [`HANGUP_GRACE`] later. `ending` closes once the kill is
/// sent.
pub fn end_group(pid: u32, ending: Ending) {
    let pgid = -(pid as libc::pid_t);
    let leader = agentmux_procstats::snapshot()
        .ok()
        .and_then(|procs| procs.into_iter().find(|p| p.pid == pid))
        .map(|p| p.start_key);
    // SAFETY: plain signals to a process group this daemon created.
    unsafe {
        libc::kill(pgid, libc::SIGHUP);
    }
    signal_session(pid, leader, libc::SIGHUP);
    std::thread::spawn(move || {
        std::thread::sleep(HANGUP_GRACE);
        unsafe {
            libc::kill(pgid, libc::SIGKILL);
        }
        signal_session(pid, leader, libc::SIGKILL);
        drop(ending);
    });
}

/// Send `signal` to every process in the session led by `sid` (the shell
/// `spawn_login_shell` made with `setsid`) outside the shell's own group,
/// which [`end_group`] signals itself.
fn signal_session(sid: u32, leader: Option<u64>, signal: libc::c_int) {
    let Ok(procs) = agentmux_procstats::snapshot() else {
        return;
    };
    // A session's id is its leader's pid, which isn't given to a new process
    // while any process of the session is left. A live process with that pid
    // other than the leader seen when the session was ended means the id was
    // reused since: none of its processes are this session's.
    if procs.iter().any(|p| p.pid == sid && Some(p.start_key) != leader) {
        return;
    }
    let sid = sid as libc::pid_t;
    for p in &procs {
        let pid = p.pid as libc::pid_t;
        // SAFETY: getsid, getpgid and kill on a pid; a process gone since the
        // snapshot fails them harmlessly.
        unsafe {
            if pid != sid && libc::getsid(pid) == sid && libc::getpgid(pid) != sid {
                libc::kill(pid, signal);
            }
        }
    }
}

/// Wait until every [`Ending`] is closed: before the daemon exits, which would
/// otherwise drop the kills still waiting.
pub fn wait_for_endings() {
    let mut open = ENDINGS.lock().unwrap_or_else(|e| e.into_inner());
    while *open > 0 {
        open = ENDINGS_DONE.wait(open).unwrap_or_else(|e| e.into_inner());
    }
}

fn set_cloexec(fd: libc::c_int) {
    // SAFETY: fcntl on a descriptor this process owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}
