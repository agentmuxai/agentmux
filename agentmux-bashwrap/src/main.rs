// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! agentmux-bashwrap — streaming bash wrapper for AgentMux agents.
//!
//! Two subcommands:
//!
//! - `exec` — runs a user-supplied command inside an owned PTY,
//!   streams stdout/stderr line-by-line to the AgentMux sidecar's
//!   WPS broker (HTTP), and prints the aggregated output on its own
//!   stdout for Claude's native Bash tool to capture as `tool_result`.
//!
//! - `hook` — reads a PreToolUse JSON payload on stdin and emits a
//!   hook response that rewrites the command so Claude's native
//!   Bash invokes `agentmux-bashwrap exec` instead. The original
//!   command is base64-encoded into the rewrite argv so quoting +
//!   multi-line bodies survive.
//!
//! See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` for the
//! full design rationale (why command rewrite vs. MCP deny-redirect,
//! why a separate binary vs. extending an MCP server, channel
//! correlation by `tool_use_id`, etc).

use anyhow::Result;
use clap::{Parser, Subcommand};

mod bash_wrap;
mod hook;
mod wps_client;

#[derive(Parser)]
#[command(name = "agentmux-bashwrap", version)]
#[command(about = "AgentMux streaming bash wrapper + PreToolUse hook helper")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a bash command inside an owned PTY, stream its stdout/stderr
    /// to the AgentMux sidecar's WPS broker, and print the aggregated
    /// output on this process's stdout for Claude to capture.
    Exec(bash_wrap::Args),
    /// Read a PreToolUse JSON payload on stdin (from Claude Code) and
    /// emit a hook response that rewrites the command to invoke `exec`.
    Hook,
}

fn main() -> Result<()> {
    // Logs go to stderr so they don't pollute the stdout channel that
    // Claude reads as `tool_result.content`. RUST_LOG controls level;
    // default `warn` keeps noise minimal in production.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    match cli.command {
        Command::Exec(args) => {
            // Propagate the inner command's exit code as our own
            // process exit. Without this the wrapper always exited 0
            // and Claude's native Bash tool saw success for every
            // wrapped command regardless of the actual outcome —
            // codex P1 on PR #804.
            let exit_code = rt.block_on(bash_wrap::run(args))?;
            std::process::exit(exit_code);
        }
        Command::Hook => hook::run_pretooluse_bash(),
    }
}
