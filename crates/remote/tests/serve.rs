// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-remote serve --stdio` as srv runs it: the real binary, spoken to
//! over its stdin and stdout.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use agentmux_remote::fsproto::{Reply, Request, Splitter, PROTOCOL};

#[test]
fn serve_answers_over_stdio_until_its_input_closes() {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentmux-remote"))
        .args(["serve", "--stdio"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for (i, r) in [
        Request::Hello,
        Request::Write {
            path: "~/note.txt".into(),
            data: b"remote".to_vec(),
        },
        Request::Read {
            path: "note.txt".into(),
            offset: 0,
            len: 64,
        },
    ]
    .iter()
    .enumerate()
    {
        stdin.write_all(&r.encode(i as u32)).unwrap();
    }
    // Closing its input ends it.
    drop(stdin);
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    assert!(child.wait().unwrap().success());

    let replies: Vec<Reply> = Splitter::new()
        .push(&out)
        .unwrap()
        .iter()
        .map(|b| Reply::decode(b).unwrap().1)
        .collect();
    assert!(matches!(&replies[0], Reply::Hello { protocol, .. } if *protocol == PROTOCOL));
    assert_eq!(replies[1], Reply::Done);
    assert_eq!(
        replies[2],
        Reply::Read {
            data: b"remote".to_vec(),
            eof: true
        }
    );
    assert_eq!(
        std::fs::read(home.path().join("note.txt")).unwrap(),
        b"remote"
    );
}
