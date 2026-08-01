// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Suppress console windows on Windows — bashwrap communicates via pipes and
// HTTP, never via a console. Without this attribute every Bash tool call
// produces a blank transparent window (Windows auto-creates one for any
// CUI-subsystem process unless CREATE_NO_WINDOW is passed by the spawner,
// which Claude Code's hook mechanism does not do).
#![cfg_attr(windows, windows_subsystem = "windows")]

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
//! - `precompact` — registered as Claude Code's `PreCompact` hook.
//!   Fires the instant compaction begins; pings the sidecar's WPS
//!   broker with a `compaction_started` event so the UI can show
//!   live status instead of a silent gap. See `precompact.rs` and
//!   `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`.
//!
//! See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` for the
//! full design rationale (why command rewrite vs. MCP deny-redirect,
//! why a separate binary vs. extending an MCP server, channel
//! correlation by `tool_use_id`, etc).

use anyhow::Result;
use clap::{Parser, Subcommand};

mod bash_wrap;
mod hook;
mod precompact;
#[cfg(test)]
mod test_env_lock;
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
    /// Registered as Claude Code's `PreCompact` hook. Publishes a
    /// `compaction_started` WPS event and exits 0 with no stdout
    /// output — observe-only, never blocks compaction.
    Precompact(precompact::Args),
}

fn main() -> Result<()> {
    init_tracing();

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
        Command::Precompact(args) => rt.block_on(precompact::run(args)),
    }
}

/// Initialize tracing to write to `~/.agentmux/logs/bashwrap-debug.log`
/// at INFO by default. Writing to a file instead of stderr means:
///
/// 1. Diagnostics survive bash's stdio capture — Claude's tool_result
///    `stderr` field is empty for successful commands, so anything we
///    write to stderr is lost to us as developers.
/// 2. We don't pollute the model's tool_result with bashwrap-internal
///    noise (env snapshot, publish attempts, etc.).
/// 3. We can tail the file from outside the agentmux process tree.
///
/// Falls back to stderr at WARN level if the file can't be opened.
fn init_tracing() {
    let log_path = dirs::home_dir()
        .map(|h| h.join(".agentmux").join("logs").join("bashwrap-debug.log"));
    if let Some(path) = log_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(file) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
        {
            let writer = std::sync::Mutex::new(file);
            let _ = tracing_subscriber::fmt()
                .with_writer(writer)
                .with_ansi(false)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .try_init();
            return;
        }
    }
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();
}
