// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The real binary's output for a plain command: nothing but the command's
//! own text after the exit prefix, and a PTY wide enough that width-aware
//! programs don't fold their output at 80 columns. Integration test, like
//! `idle_kill_full_process_tree.rs`, because it needs the built binary.
//! docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §3.5, §6.

#![cfg(unix)]

fn b64(s: &str) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(s.as_bytes())
}

async fn run(cmd: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_agentmux-bashwrap");
    let out = tokio::process::Command::new(bin)
        .args(["exec", "--tool-id=test-pty-output-clean", &format!("--b64-cmd={}", b64(cmd))])
        .output()
        .await
        .expect("run the real agentmux-bashwrap binary");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The command's own lines: the output minus the exit prefix, the warning
/// bashwrap prints when it runs without the app's streaming environment (as
/// on CI), and blank lines.
fn own_lines(out: &str) -> Vec<&str> {
    out.lines()
        .filter(|l| !l.starts_with("<exited ") && !l.starts_with("[bashwrap] ") && !l.trim().is_empty())
        .collect()
}

/// The cursor-position reply ConPTY needs was written on every platform, and
/// a Unix PTY echoed it back as `^[[1;1R` at the start of every result.
#[tokio::test]
async fn a_result_has_no_echoed_cursor_report() {
    let out = run("echo hello").await;
    assert!(!out.contains("1;1R"), "echoed cursor report in the output: {out:?}");
    assert_eq!(own_lines(&out), ["hello"], "{out:?}");
}

/// Programs that size their output to the terminal see 200 columns.
#[tokio::test]
async fn the_pty_is_wider_than_80_columns() {
    let out = run("python3 -c 'import shutil; print(shutil.get_terminal_size().columns)' 2>/dev/null || echo $COLUMNS").await;
    let cols: u32 = own_lines(&out)
        .last()
        .and_then(|l| l.trim().parse().ok())
        .unwrap_or_else(|| panic!("no column count in {out:?}"));
    assert_eq!(cols, 200, "{out:?}");
}
