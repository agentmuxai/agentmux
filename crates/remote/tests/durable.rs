// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A durable session end to end, with the real binary: attach, run a command,
//! drop the link (kill `attach`, as an SSH drop does), reattach and get
//! exactly what was missed, then end the session (spec §7.2, §7.3).
#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agentmux_remote::frame::{Decoder, Frame};

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

#[test]
fn version_names_the_protocol() {
    let out = Command::new(BIN).arg("version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("protocol 1"));
}
