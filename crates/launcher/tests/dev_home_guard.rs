// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A dev launcher started by hand, with no isolated data home, refuses with one line and creates
//! nothing (task #2553). `CARGO_BIN_EXE_*` lives in `target/debug`, so it counts as a dev build.

use std::process::{Command, Stdio};

fn hand_start(home: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agentmux-launcher"))
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("AGENTMUX_DEV_HOME_MODE")
        .env_remove("AGENTMUX_HOME_OVERRIDE")
        .env_remove("AGENTMUX_DEV_USE_REAL_HOME")
        .env_remove("AGENTMUX_DATA_HOME")
        .stdin(Stdio::null())
        .output()
        .expect("run agentmux-launcher")
}

#[test]
fn a_hand_started_dev_launcher_refuses_and_creates_no_folder() {
    let home = tempfile::tempdir().expect("temp home");
    let out = hand_start(home.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert_eq!(stderr.trim().lines().count(), 1, "one plain line: {stderr}");
    assert!(
        stderr.contains("task dev") && stderr.contains("AGENTMUX_DEV_USE_REAL_HOME"),
        "{stderr}"
    );
    let left: Vec<_> = std::fs::read_dir(home.path()).unwrap().collect();
    assert!(
        left.is_empty(),
        "the refused start left {left:?} under the home"
    );
}

/// Where the OS answers for the home folder (Windows) `HOME` can't stand in for it; there the
/// unit tests in `agentmux_common::dev_home_guard` cover this case.
#[cfg(not(windows))]
#[test]
fn naming_the_real_home_without_the_switch_is_refused_too() {
    let home = tempfile::tempdir().expect("temp home");
    let real = home.path().join(".agentmux");
    let out = Command::new(env!("CARGO_BIN_EXE_agentmux-launcher"))
        .env("HOME", home.path())
        .env("AGENTMUX_HOME_OVERRIDE", &real)
        .env_remove("AGENTMUX_DEV_HOME_MODE")
        .env_remove("AGENTMUX_DEV_USE_REAL_HOME")
        .stdin(Stdio::null())
        .output()
        .expect("run agentmux-launcher");
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!real.exists(), "nothing created under the named home");
}
