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
//!   MPS broker (HTTP), and prints the aggregated output on its own
//!   stdout for Claude's native Bash tool to capture as `tool_result`.
//!
//! - `hook` — reads a PreToolUse JSON payload on stdin. When the CLI's
//!   shell prefix is this binary, it records the call for the prefix
//!   process and changes nothing. Otherwise it emits a hook response that
//!   rewrites the command so Claude's native Bash invokes
//!   `agentmux-bashwrap exec` instead, the command base64-encoded into the
//!   rewrite argv so quoting + multi-line bodies survive.
//!
//! - prefix mode (one argument: the CLI's script) — Claude Code's
//!   `CLAUDE_CODE_SHELL_PREFIX`. Runs the script the way `exec` runs a
//!   command, for the call the hook recorded. See `prefix.rs` and
//!   `docs/specs/SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md`.
//!
//! - `precompact` — registered as Claude Code's `PreCompact` hook.
//!   Fires the instant compaction begins; pings the sidecar's MPS
//!   broker with a `compaction_started` event so the UI can show
//!   live status instead of a silent gap. See `precompact.rs` and
//!   `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`.
//!
//! - askpass mode (no subcommand) — `ssh`'s `SSH_ASKPASS` when AgentMux
//!   runs ssh for an agent: shows ssh's prompt to the user through srv and
//!   prints the answer. See `askpass.rs`.
//!
//! - `sessionstart --part N` — registered as Claude Code's `SessionStart`
//!   hook, once per part. Delivers part N of the agent's memory as the
//!   hook's `additionalContext`. See `sessionstart.rs` and
//!   `docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md` §7 P2.
//!
//! See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` for the
//! full design rationale (why command rewrite vs. MCP deny-redirect,
//! why a separate binary vs. extending an MCP server, channel
//! correlation by `tool_use_id`, etc).

use anyhow::Result;
use clap::{Parser, Subcommand};

mod askpass;
mod bash_wrap;
mod hook;
mod precompact;
mod prefix;
mod sessionstart;
#[cfg(test)]
mod test_env_lock;
mod mps_client;

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
    /// to the AgentMux sidecar's MPS broker, and print the aggregated
    /// output on this process's stdout for Claude to capture.
    Exec(bash_wrap::Args),
    /// Read a PreToolUse JSON payload on stdin (from Claude Code) and
    /// emit a hook response that rewrites the command to invoke `exec`.
    Hook,
    /// Registered as Claude Code's `PreCompact` hook. Publishes a
    /// `compaction_started` MPS event and exits 0 with no stdout
    /// output — observe-only, never blocks compaction.
    Precompact(precompact::Args),
    /// Registered as Claude Code's `SessionStart` hook, once per part.
    /// Prints this part of the agent's memory as the hook's
    /// `additionalContext` (or nothing), then acknowledges it.
    Sessionstart(sessionstart::Args),
}

fn main() -> Result<()> {
    init_tracing();

    // ssh's askpass for an agent's SSH command: the prompt is the only
    // argument, so this is decided before clap sees it (askpass.rs).
    if let Some(code) = askpass::run_if_asked() {
        std::process::exit(code);
    }

    // Claude Code's shell prefix: the CLI's script is the only argument, so
    // this too is decided before clap sees it (prefix.rs).
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(script) = prefix::prefix_script(&argv) {
        let rt = tokio::runtime::Runtime::new()?;
        let exit_code = rt.block_on(bash_wrap::run_prefix(script))?;
        std::process::exit(exit_code);
    }

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
        Command::Sessionstart(args) => rt.block_on(sessionstart::run(args)),
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
