// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server's process, as `agentmux` sees it. A process id alone is never enough to signal: a
//! `server.json` left by a crash can name an id the system has given to another program since. So
//! `stop` signals only a process whose image is `agentmux-srv`, found by asking the system about
//! that id, and never a process found by name.

use agentmux_common::win32::NoWindow;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// The server's file name, without a directory or an extension.
pub const SERVER_IMAGE: &str = "agentmux-srv";

/// Is `image` (a path, or a bare name) the server's? The base name must be `agentmux-srv`, with
/// `.exe` allowed, and a Linux kernel's ` (deleted)` mark after a replaced file ignored.
pub fn names_server(image: &str) -> bool {
    let image = image.trim().trim_end_matches(" (deleted)");
    let base = image.rsplit(['/', '\\']).next().unwrap_or(image);
    base == SERVER_IMAGE || base == "agentmux-srv.exe"
}

/// The image of the process with this id: the path the system reports for it.
#[cfg(target_os = "linux")]
fn image_of(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn image_of(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).no_window().output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(not(unix))]
fn image_of(_pid: u32) -> Option<String> {
    None
}

/// Is this process alive, and is it the server?
pub fn is_server(pid: u32) -> bool {
    image_of(pid).is_some_and(|image| names_server(&image))
}

/// Ask the server to shut down cleanly (SIGTERM). True when the signal was sent.
#[cfg(unix)]
pub fn terminate(pid: u32) -> bool {
    signal(pid, libc::SIGTERM)
}

/// End it at once (SIGKILL), for `stop --force` after the server did not leave in time.
#[cfg(unix)]
pub fn kill(pid: u32) -> bool {
    signal(pid, libc::SIGKILL)
}

#[cfg(unix)]
fn signal(pid: u32, sig: libc::c_int) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    // A pid of 0 or below would signal a whole process group: never.
    pid > 0 && unsafe { libc::kill(pid, sig) } == 0
}

#[cfg(not(unix))]
pub fn terminate(_pid: u32) -> bool {
    false
}

#[cfg(not(unix))]
pub fn kill(_pid: u32) -> bool {
    false
}

/// Make the child leave this process's session, so it keeps running after the command that started it
/// exits and after its terminal closes.
#[cfg(unix)]
pub fn detach(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: `setsid` is async-signal-safe and the closure touches nothing else.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub fn detach(_cmd: &mut std::process::Command) {}

/// Does the server answer `/health` on this loopback port? A plain request over a socket: nothing
/// is sent but the one line, and nothing but the status is read.
pub fn health_ok(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(800)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    stream.read_exact(&mut head).is_ok() && head.starts_with(b"HTTP/1.1 200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn only_the_server_image_is_the_server() {
        for yes in [
            "/home/u/.agentmux/server/0.60.26/bin/agentmux-srv",
            "agentmux-srv",
            "/opt/agentmux-srv (deleted)",
            "C:\\x\\agentmux-srv.exe",
        ] {
            assert!(names_server(yes), "{yes}");
        }
        for no in ["", "/usr/bin/node", "/x/agentmux-srv-old", "/x/agentmux", "/x/agentmux-srv/node", "/x/my-agentmux-srv"] {
            assert!(!names_server(no), "{no}");
        }
    }

    #[test]
    fn a_pid_of_nothing_is_no_server() {
        assert!(!is_server(u32::MAX - 1));
    }

    #[cfg(unix)]
    #[test]
    fn this_test_binary_is_not_the_server() {
        assert!(!is_server(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn a_pid_that_would_mean_a_group_is_never_signalled() {
        assert!(!terminate(0));
        assert!(!kill(0));
        assert!(!terminate(u32::MAX));
    }

    fn answer_once(reply: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply);
            }
        });
        port
    }

    #[test]
    fn health_is_a_200_and_nothing_else() {
        assert!(health_ok(answer_once(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")));
        assert!(!health_ok(answer_once(b"HTTP/1.1 503 Service Unavailable\r\n\r\n")));
        assert!(!health_ok(answer_once(b"not http at all, just a lot of bytes")));
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(!health_ok(closed));
    }
}
