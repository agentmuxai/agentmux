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

/// The path Windows reports for the process with this id. A process this user may not look at, or one that is gone,
/// has none.
#[cfg(windows)]
fn image_of(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: the handle is closed on every path; the buffer is as long as the length it is told.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 4096];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(not(any(unix, windows)))]
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

/// There is no signal to send on Windows: `agentmux stop` asks the server to shut down over its signed channel.
#[cfg(not(unix))]
pub fn terminate(_pid: u32) -> bool {
    false
}

/// End the process at once (`TerminateProcess`), for `stop --force` after the server did not leave in time. Only a
/// process that is the server is ended: the id is looked at again here.
#[cfg(windows)]
pub fn kill(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE};
    if !is_server(pid) {
        return false;
    }
    // SAFETY: the handle is closed on every path.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        // Exit code 0: the end was asked for, so the supervisor and the logon task that run it see a clean end and the task's
        // restart on failure does not start the server again. A crash has its own code.
        let ended = TerminateProcess(handle, 0) != 0;
        CloseHandle(handle);
        ended
    }
}

#[cfg(not(any(unix, windows)))]
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

/// On Windows: its own process group (a Ctrl+C or Ctrl+Break in the terminal that started it does not reach it) and no
/// console window, so it keeps running after the terminal closes.
#[cfg(windows)]
pub fn detach(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
pub fn detach(_cmd: &mut std::process::Command) {}

/// Before the server is started to outlive this command: this command's own input, output and error handles are made
/// not inheritable. A program started with a handle table inherits every inheritable handle of its parent, whatever
/// it is given as its own input and output, so a server started by a command whose output is a pipe (a script, a CI
/// step) would keep the pipe open for as long as it runs, and the reader of the pipe would never see the end.
#[cfg(windows)]
pub fn keep_std_handles_private() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    let handles = [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()];
    for handle in handles {
        if !handle.is_null() {
            // SAFETY: a handle this process holds; if it is not one the call fails and nothing changes.
            unsafe { SetHandleInformation(handle as HANDLE, HANDLE_FLAG_INHERIT, 0) };
        }
    }
}

#[cfg(not(windows))]
pub fn keep_std_handles_private() {}

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

    #[test]
    fn this_test_binary_is_not_the_server() {
        assert!(!is_server(std::process::id()));
    }

    /// A command's own input, output and error are not handed on to a server it starts.
    #[cfg(windows)]
    #[test]
    fn the_commands_own_handles_are_not_inherited_by_the_server_it_starts() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
        keep_std_handles_private();
        for handle in [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()] {
            let mut flags = 0u32;
            // SAFETY: a handle this process holds, and a place for the flags.
            if !handle.is_null() && unsafe { GetHandleInformation(handle as HANDLE, &mut flags) } != 0 {
                assert_eq!(flags & HANDLE_FLAG_INHERIT, 0, "a standard handle is still inheritable");
            }
        }
    }

    /// The system names the image of a live process: this test program's own file, found by its id.
    #[cfg(windows)]
    #[test]
    fn windows_names_the_image_of_a_live_process() {
        let image = image_of(std::process::id()).expect("an image for this process");
        let exe = std::env::current_exe().unwrap();
        assert!(image.to_lowercase().ends_with(&exe.file_name().unwrap().to_string_lossy().to_lowercase()), "{image} is not {}", exe.display());
        assert!(image_of(u32::MAX - 1).is_none());
    }

    /// Only a process whose image is the server's is ended: this test program is not, and is still here afterwards.
    #[cfg(windows)]
    #[test]
    fn a_process_that_is_not_the_server_is_never_ended() {
        assert!(!kill(std::process::id()));
        assert!(!kill(u32::MAX - 1));
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
