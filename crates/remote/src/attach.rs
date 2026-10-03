// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `attach`, run by srv as `ssh -T host -- agentmux-remote attach ...`: start
//! the daemon if it is not running, send the attach line, then relay bytes
//! between this process's stdio (the SSH channel) and the daemon's socket.
//! The frames themselves pass through untouched. When the SSH channel drops,
//! stdin ends, the daemon sees the connection close, and the session stays.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::daemon::socket_path;
use agentmux_remote::frame::PROTOCOL;

/// Connect to the daemon, starting it (detached, in its own session, so it
/// outlives this SSH channel) if none is running.
pub fn connect_or_start(base: &Path) -> io::Result<UnixStream> {
    let sock = socket_path(base);
    if let Ok(s) = UnixStream::connect(&sock) {
        return Ok(s);
    }
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(home) = std::env::var_os("AGENTMUX_REMOTE_HOME") {
        cmd.env("AGENTMUX_REMOTE_HOME", home);
    }
    // SAFETY: setsid only, between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(&sock) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Attach and relay until either side closes. The exit code is 0 for a clean
/// end and 1 when the daemon could not be reached.
pub fn run(base: &Path, session: &str, offset: u64, cols: u16, rows: u16) -> i32 {
    let mut stream = match connect_or_start(base) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("agentmux-remote: cannot reach the session daemon: {e}");
            return 1;
        }
    };
    let line = format!("AMR {PROTOCOL} ATTACH {session} {offset} {cols} {rows}\n");
    if stream.write_all(line.as_bytes()).is_err() {
        return 1;
    }
    let mut to_daemon = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return 1,
    };
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if to_daemon.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        // The SSH channel closed: tell the daemon this client is gone.
        let _ = to_daemon.shutdown(std::net::Shutdown::Write);
    });
    let mut stdout = io::stdout().lock();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if stdout
                    .write_all(&buf[..n])
                    .and_then(|_| stdout.flush())
                    .is_err()
                {
                    break;
                }
            }
        }
    }
    0
}

/// Send one control line (`END`, `LIST`) and print the answer.
pub fn control(base: &Path, line: &str) -> i32 {
    let mut stream = match UnixStream::connect(socket_path(base)) {
        Ok(s) => s,
        Err(_) => {
            // No daemon: no sessions.
            return 0;
        }
    };
    if stream
        .write_all(format!("AMR {PROTOCOL} {line}\n").as_bytes())
        .is_err()
    {
        return 1;
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let _ = io::stdout().write_all(&answer);
    0
}
