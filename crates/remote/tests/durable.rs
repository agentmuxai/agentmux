// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A durable session end to end, with the real binary: attach, run a command,
//! drop the link (kill `attach`, as an SSH drop does), reattach and get
//! exactly what was missed, then end the session (spec §7.2, §7.3).
#![cfg(unix)]

// This binary must hold this checkout's code (`agentmux_common::checkout`).
agentmux_common::checkout_guard!();

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agentmux_remote::frame::{Decoder, Frame};
use agentmux_remote::fsproto::{PaneFound, PaneFrame, Reply, Request, Splitter};

const BIN: &str = env!("CARGO_BIN_EXE_agentmux-remote");

struct Attached {
    child: Child,
    frames: mpsc::Receiver<Frame>,
}

fn attach(home: &std::path::Path, session: &str, offset: u64) -> Attached {
    let mut child = Command::new(BIN)
        .args([
            "attach",
            "--session",
            session,
            "--offset",
            &offset.to_string(),
            "--cols",
            "100",
            "--rows",
            "30",
        ])
        .env("AGENTMUX_REMOTE_HOME", home)
        .env("SHELL", "/bin/sh")
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("attach runs");
    let mut out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut d = Decoder::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            for f in d.push(&buf[..n]).expect("frames") {
                if tx.send(f).is_err() {
                    return;
                }
            }
        }
    });
    Attached { child, frames: rx }
}

impl Attached {
    fn send(&mut self, f: Frame) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(&f.encode()).unwrap();
        stdin.flush().unwrap();
    }

    /// Frames until `done` says stop; the output bytes seen, with the offset
    /// each Output frame said it started at checked against the running count.
    fn until(&self, mut done: impl FnMut(&Frame, &[u8]) -> bool) -> (Vec<Frame>, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut frames, mut text) = (Vec::new(), Vec::new());
        while Instant::now() < deadline {
            let Ok(f) = self.frames.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            if let Frame::Output { data, .. } = &f {
                text.extend_from_slice(data);
            }
            let stop = done(&f, &text);
            frames.push(f);
            if stop {
                return (frames, text);
            }
        }
        panic!(
            "timed out; frames so far: {frames:?}, text: {:?}",
            String::from_utf8_lossy(&text)
        );
    }
}

fn contains(hay: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(hay).contains(needle)
}

/// Ends the test's session and removes its home however the test ends, so a
/// failed assertion never leaves a shell and a daemon behind.
struct Cleanup(std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new(BIN)
            .args(["end", "--session", "t1"])
            .env("AGENTMUX_REMOTE_HOME", &self.0)
            .output();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_session_survives_a_dropped_link_and_replays_exactly_what_was_missed() {
    let home = std::env::temp_dir().join(format!("amr-test-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup(home.clone());

    // 1. Attach: a new session, then a command and its output.
    let mut a = attach(&home, "t1", 0);
    let (frames, _) = a.until(|f, _| matches!(f, Frame::Hello { .. }));
    assert!(
        matches!(&frames[0], Frame::Hello { session, created: true, .. } if session == "t1"),
        "{frames:?}"
    );
    a.send(Frame::Input(b"echo durable-$((40+2))\n".to_vec()));
    let (_, text) = a.until(|_, t| contains(t, "durable-42"));
    assert!(contains(&text, "durable-42"));

    // 2. The link drops: the session stays.
    a.child.kill().unwrap();
    let _ = a.child.wait();
    std::thread::sleep(Duration::from_millis(300));
    let list = Command::new(BIN)
        .arg("list")
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&list.stdout).starts_with("t1 "),
        "{list:?}"
    );

    // 3. Reattach from 0: everything, including what came while detached.
    let b = attach(&home, "t1", 0);
    let (frames, text) = b.until(|_, t| contains(t, "durable-42"));
    let end = match &frames[0] {
        Frame::Hello {
            created: false,
            end,
            ..
        } => *end,
        other => panic!("expected a Hello for an existing session, got {other:?}"),
    };
    assert!(contains(&text, "durable-42"));
    let mut b = b;
    b.child.kill().unwrap();
    let _ = b.child.wait();
    std::thread::sleep(Duration::from_millis(300));

    // 4. Reattach from the end: nothing replayed, new output starts at `end`.
    let mut c = attach(&home, "t1", end);
    c.until(|f, _| matches!(f, Frame::Hello { .. }));
    c.send(Frame::Input(b"echo second-$((1+1))\n".to_vec()));
    let (frames, text) = c.until(|_, t| contains(t, "second-2"));
    let first_output = frames.iter().find_map(|f| match f {
        Frame::Output { offset, .. } => Some(*offset),
        _ => None,
    });
    assert_eq!(
        first_output,
        Some(end),
        "continues exactly where it left off"
    );
    assert!(!contains(&text, "durable-42"), "already seen: not replayed");

    // 5. Ping, then end the session.
    c.send(Frame::Ping);
    c.until(|f, _| *f == Frame::Pong);
    c.send(Frame::End);
    let _ = c.child.wait();
    std::thread::sleep(Duration::from_millis(500));
    let list = Command::new(BIN)
        .arg("list")
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&list.stdout), "", "ended");
}

/// Ends `.1` and removes the test's home when dropped.
struct EndOnDrop(std::path::PathBuf, &'static str);

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        let _ = Command::new(BIN)
            .args(["end", "--session", self.1])
            .env("AGENTMUX_REMOTE_HOME", &self.0)
            .output();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `PaneProcs` for `session`, asked of a fresh `serve --stdio` as srv asks
/// it for a durable pane's stats badge.
fn pane_procs(home: &std::path::Path, session: &str) -> PaneFrame {
    let mut child = Command::new(BIN)
        .args(["serve", "--stdio"])
        .env("AGENTMUX_REMOTE_HOME", home)
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("serve runs");
    let req = Request::PaneProcs { block_id: String::new(), session: session.into() };
    child.stdin.take().unwrap().write_all(&req.encode(1)).unwrap();
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    let _ = child.wait();
    let bodies = Splitter::new().push(&out).unwrap();
    match Reply::decode(&bodies[0]).unwrap().1 {
        Reply::PaneProcs(f) => f,
        other => panic!("expected PaneProcs, got {other:?}"),
    }
}

/// A durable pane's stats: `serve` asks the daemon for the session's shell
/// and sums it and what it started; an unknown session is the host's totals
/// only.
#[test]
fn serve_measures_a_sessions_shell_through_the_daemon() {
    let home = std::env::temp_dir().join(format!("amr-stats-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = EndOnDrop(home.clone(), "stats1");
    let mut a = attach(&home, "stats1", 0);
    a.until(|f, _| matches!(f, Frame::Hello { .. }));
    a.send(Frame::Input(b"sleep 30 &\n".to_vec()));
    std::thread::sleep(Duration::from_millis(500));

    let f = pane_procs(&home, "stats1");
    assert_eq!(f.found, PaneFound::Session, "{f:?}");
    assert_eq!(f.names.first().map(|n| n.0.as_str()), Some("sh"), "the session's shell first: {f:?}");
    assert!(f.names.iter().any(|(n, _)| n == "sleep"), "and what it started: {f:?}");
    assert!(f.processes >= 2 && f.host_processes > f.processes, "{f:?}");

    let none = pane_procs(&home, "no-such-session");
    assert_eq!((none.found, none.processes), (PaneFound::None, 0));
    assert!(none.host_processes > 1);

    // The shell exits: the session stays listed a while, but its PID is no
    // longer the pane's (it may already be another process's).
    // The background sleep holds the terminal open: end it first.
    a.send(Frame::Input(b"kill %1; exit\n".to_vec()));
    a.until(|f, _| matches!(f, Frame::Exited { .. }));
    let exited = pane_procs(&home, "stats1");
    assert_eq!(exited.found, PaneFound::None, "{exited:?}");
    // attach ends with its session's shell; the cleanup ends the session.
    let _ = a.child.wait();
}

/// Several attaches at once, with no daemon running: one daemon, one session.
#[test]
fn concurrent_attaches_share_one_daemon_and_one_session() {
    let home = std::env::temp_dir().join(format!("amr-race-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let attaches: Vec<Attached> = (0..4).map(|_| attach(&home, "t1", 0)).collect();
    for a in &attaches {
        a.until(|f, _| matches!(f, Frame::Hello { .. }));
    }
    let list = Command::new(BIN)
        .arg("list")
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&list.stdout).into_owned();
    assert_eq!(text.lines().count(), 1, "one session: {text:?}");
    for mut a in attaches {
        let _ = a.child.kill();
        let _ = a.child.wait();
    }
}

/// Whether a daemon holds `home`'s daemon lock, which it keeps for its life.
fn daemon_running(home: &std::path::Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(lock) = std::fs::OpenOptions::new()
        .write(true)
        .open(home.join("run").join("daemon.lock"))
    else {
        return false;
    };
    // SAFETY: flock on a descriptor this test owns; closing it releases it.
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) != 0 }
}

/// `stop` (Remotes' Remove helper) ends every session and the daemon at once,
/// instead of the daemon exiting only after a minute idle (#4513, finding 6).
#[test]
fn stop_ends_every_session_and_the_daemon_at_once() {
    let home = std::env::temp_dir().join(format!("amr-stop-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let mut a = attach(&home, "t1", 0);
    a.until(|f, _| matches!(f, Frame::Hello { .. }));
    let mut b = attach(&home, "t2", 0);
    b.until(|f, _| matches!(f, Frame::Hello { .. }));
    assert!(daemon_running(&home));

    let out = Command::new(BIN)
        .arg("stop")
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");

    // Both attaches end with their sessions, and the daemon goes within
    // seconds, not a minute.
    let deadline = Instant::now() + Duration::from_secs(10);
    for att in [&mut a, &mut b] {
        while att.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "an attach outlived stop");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    while daemon_running(&home) {
        assert!(Instant::now() < deadline, "the daemon outlived stop");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!home.join("run").join("daemon.sock").exists());

    // With no daemon, stop has nothing to do.
    let out = Command::new(BIN)
        .arg("stop")
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

/// An attach already connected when `stop` comes gets no session: one made
/// after `stop` would never be ended, and outlive the daemon.
#[test]
fn an_attach_connected_before_stop_gets_no_session() {
    use std::os::unix::net::UnixStream;
    let home = std::env::temp_dir().join(format!("amr-stop-race-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let a = attach(&home, "t1", 0);
    a.until(|f, _| matches!(f, Frame::Hello { .. }));
    let mut early = UnixStream::connect(home.join("run").join("daemon.sock")).unwrap();

    let out = Command::new(BIN)
        .arg("stop")
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");

    let line = format!("AMR {} ATTACH t2 0 80 24\n", agentmux_remote::frame::PROTOCOL);
    early.write_all(line.as_bytes()).unwrap();
    early.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (mut d, mut buf) = (Decoder::new(), [0u8; 4096]);
    let n = early.read(&mut buf).unwrap();
    let frames = d.push(&buf[..n]).unwrap();
    assert!(matches!(frames.first(), Some(Frame::Error(_))), "{frames:?}");
}

/// Kills the process it holds however the test ends.
struct KillOnDrop(libc::pid_t);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // SAFETY: a plain signal to a process this test started.
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
    }
}

/// Run `stops` stops at once on a daemon whose session runs a process that
/// ignores the hang-up (and, unlike a shell, never notices its terminal
/// close): each succeeds, and the process is killed before the daemon exits.
fn stops_kill_a_process_that_ignores_the_hangup(tag: &str, stops: usize) {
    let home = std::env::temp_dir().join(format!("amr-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let mut a = attach(&home, "t1", 0);
    a.until(|f, _| matches!(f, Frame::Hello { .. }));
    a.send(Frame::Input(b"trap '' HUP; echo \"P\"\"ID=$$=\"; exec sleep 300\n".to_vec()));
    let (_, text) = a.until(|_, t| {
        String::from_utf8_lossy(t)
            .split("PID=")
            .nth(1)
            .is_some_and(|rest| rest.contains('='))
    });
    let text = String::from_utf8_lossy(&text).into_owned();
    let pid: libc::pid_t = text
        .split("PID=")
        .nth(1)
        .and_then(|rest| rest.split('=').next())
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or_else(|| panic!("no pid in {text:?}"));
    let _kill = KillOnDrop(pid);
    // SAFETY: signal 0 only checks that the process exists.
    let alive = || unsafe { libc::kill(pid, 0) == 0 };
    assert!(alive());

    let running: Vec<Child> = (0..stops)
        .map(|_| {
            Command::new(BIN)
                .arg("stop")
                .env("AGENTMUX_REMOTE_HOME", &home)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    // One that comes once the socket is gone finds no daemon: nothing to stop.
    for stop in running {
        let out = stop.wait_with_output().unwrap();
        assert!(out.status.success(), "{out:?}");
        assert!(matches!(String::from_utf8_lossy(&out.stdout).trim(), "ok" | ""), "{out:?}");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon_running(&home) {
        assert!(Instant::now() < deadline, "the daemon outlived stop");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Killed by the time the daemon is gone; reaping by init may lag a little.
    while alive() {
        assert!(Instant::now() < deadline, "a process that ignored SIGHUP outlived stop");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn stop_kills_a_process_that_ignores_the_hangup() {
    stops_kill_a_process_that_ignores_the_hangup("stop-hup", 1);
}

/// A second stop at the same time never exits the daemon before the first
/// one's kills are sent.
#[test]
fn concurrent_stops_still_kill_a_process_that_ignores_the_hangup() {
    stops_kill_a_process_that_ignores_the_hangup("stop-hup-2", 4);
}

/// A background job that ignores the hang-up has a process group of its own
/// (job control), but is in the session: ending the session kills it too.
#[test]
fn ending_a_session_kills_a_background_job_that_ignores_the_hangup() {
    let home = std::env::temp_dir().join(format!("amr-end-job-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let mut a = attach(&home, "t1", 0);
    a.until(|f, _| matches!(f, Frame::Hello { .. }));
    a.send(Frame::Input(b"(trap '' HUP; exec sleep 300) & echo \"P\"\"ID=$!=\"\n".to_vec()));
    let (_, text) = a.until(|_, t| {
        String::from_utf8_lossy(t)
            .split("PID=")
            .nth(1)
            .is_some_and(|rest| rest.contains('='))
    });
    let text = String::from_utf8_lossy(&text).into_owned();
    let pid: libc::pid_t = text
        .split("PID=")
        .nth(1)
        .and_then(|rest| rest.split('=').next())
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or_else(|| panic!("no pid in {text:?}"));
    let _kill = KillOnDrop(pid);
    // SAFETY: signal 0 only checks that the process exists.
    let alive = || unsafe { libc::kill(pid, 0) == 0 };
    assert!(alive());

    let out = Command::new(BIN)
        .args(["end", "--session", "t1"])
        .env("AGENTMUX_REMOTE_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
    // Hung up at once, killed after the grace; reaping by init may lag a little.
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive() {
        assert!(Instant::now() < deadline, "a background job that ignored SIGHUP outlived its session");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn version_names_the_protocol() {
    let out = Command::new(BIN).arg("version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("protocol 1"));
}
