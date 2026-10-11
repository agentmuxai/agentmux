// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The real binary as Claude Code's shell prefix, started the way the CLI
//! starts a stdio MCP server: `agentmux-bashwrap <program>`, one argument.
//! The program gets this process's stdin and its stdout comes back byte for
//! byte, the way the `agentmux` MCP server's JSON-RPC must. Without this,
//! v0.59.19 refused `agentmux-mcp` as an unknown subcommand and agents came
//! up without their AgentMux tools
//! (docs/reports/REPORT_AGENT_MCP_DOWN_THROUGH_SHELL_PREFIX_2026_10_11.md).
//!
//! No test harness: this binary is also the "MCP server" (an echo of stdin),
//! and the harness would print its own lines on stdout in that role.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

const CHILD_ENV: &str = "BASHWRAP_TEST_ECHO_CHILD";
const REQUEST: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n";

fn main() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let mut input = Vec::new();
        std::io::stdin().read_to_end(&mut input).unwrap();
        std::io::stdout().write_all(&input).unwrap();
        std::process::exit(7);
    }

    let me = std::env::current_exe().unwrap();
    let me = me.to_str().unwrap();
    if me.contains(char::is_whitespace) {
        // The CLI joins a server's command line unquoted; a path with a space
        // is a command line, not a program, and bash would split it.
        println!("skipped: the test binary's path has a space in it ({me})");
        return;
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_agentmux-bashwrap"))
        .arg(me)
        .env(CHILD_ENV, "1")
        .env("CLAUDE_CODE_SHELL_PREFIX", env!("CARGO_BIN_EXE_agentmux-bashwrap"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the real agentmux-bashwrap binary");
    child.stdin.take().unwrap().write_all(REQUEST).unwrap();
    let out = child.wait_with_output().unwrap();

    assert_eq!(
        out.stdout,
        REQUEST,
        "stdout must be the server's bytes, nothing added; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(7), "the server's exit code comes back");
    println!("ok: a one-word program runs through the prefix with clean stdio");
}
