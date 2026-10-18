// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The arguments of `agentmux-bashwrap exec`, also built by prefix mode for a
//! call it runs (`prefix.rs`). Moved out of `bash_wrap.rs`, which runs the call.

use clap::Parser;

/// CLI args for `exec`. `command` carried as base64 to sidestep every
/// quoting concern in the shell that invokes us.
#[derive(Parser, Debug)]
pub struct Args {
    /// Tool-use id from Claude's `tool_use` event; threaded back as
    /// the MPS subject suffix so the frontend correlates chunks with
    /// the matching ToolNode.
    #[arg(long)]
    pub tool_id: String,

    /// URL-safe base64 (no padding) of the original command string.
    #[arg(long)]
    pub b64_cmd: String,

    /// Optional block id — used as the `block:<id>` MPS scope so
    /// chunks only reach subscribers watching this block. When
    /// omitted, chunks publish unscoped.
    #[arg(long)]
    pub block_id: Option<String>,

    /// True when the CLI's own Bash tool call was made with
    /// `run_in_background: true` (threaded through by
    /// `hook.rs::build_response` from the PreToolUse payload's
    /// `tool_input.run_in_background`). A declared-background task may
    /// legitimately produce zero output forever (a GUI window, a dev
    /// server once its logs go quiet) — the idle-kill heuristic exists for
    /// commands that SHOULD exit and are stuck, not for this class. See
    /// `effective_idle_timeout` below, issue #2491, and
    /// docs/retro/RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md.
    #[arg(long, default_value_t = false)]
    pub declared_background: bool,

    /// The Claude Code subagent that made the call (the PreToolUse
    /// payload's `agent_id`); absent for the main conversation. Its working
    /// directory is kept apart from the main agent's, so a subagent's `cd`
    /// never moves where the main agent's next command runs
    /// (`CwdState::load`).
    #[arg(long)]
    pub agent: Option<String>,

    /// Set by prefix mode when two agents ran the very same command at once
    /// and this call's agent can't be told: no agent's saved directory is
    /// used or written (`cwd_state::CwdState::for_call`).
    #[arg(skip)]
    pub agent_unknown: bool,
}
