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

/// End a session's whole process group: hang up, then kill what is left.
pub fn end_group(pid: u32) {
    let pgid = -(pid as libc::pid_t);
    // SAFETY: plain signals to a process group this daemon created.
    unsafe {
        libc::kill(pgid, libc::SIGHUP);
    }
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        unsafe {
            libc::kill(pgid, libc::SIGKILL);
        }
    });
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
