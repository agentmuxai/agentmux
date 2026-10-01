// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `sessionstart` subcommand — registered as Claude Code's `SessionStart`
//! hook, once per part (`--part 1` … `--part 8`). Delivers the agent's memory
//! (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2).
//!
//! A `SessionStart` hook's `additionalContext` reaches the model without a
//! visible turn, but only up to about 10,000 characters per hook command (the
//! CLI swaps anything longer for a file preview). So the sidecar splits the
//! memory into labelled parts, and each hook command asks for its own:
//!
//! 1. read the hook's stdin (`session_id`, `source`);
//! 2. `POST /api/v1/agent/memory/session-start/part` for part N — the sidecar
//!    decides whether this source gets a delivery at all (`resume` and `fork`
//!    don't) and how many parts there are;
//! 3. print `{"hookSpecificOutput":{"hookEventName":"SessionStart",
//!    "additionalContext":…}}` and flush it;
//! 4. `POST …/session-start/ack`, so the sidecar knows this part reached the
//!    CLI. Once every part is acknowledged the pane gets its notice.
//!
//! Every failure (no env, sidecar down, no part for this number) degrades to
//! exit 0 with nothing printed: a hook must never block or break a session
//! start, and an empty stdout is a hook with nothing to add.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

use crate::mps_client::WpsClient;

const PART_PATH: &str = "/api/v1/agent/memory/session-start/part";
const ACK_PATH: &str = "/api/v1/agent/memory/session-start/ack";

/// The CLI waits for every `SessionStart` hook before the session starts, so
/// a slow or wedged sidecar must not hold it up for long. Composing reads a
/// handful of small files on the same host.
const PART_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// CLI args for `sessionstart`.
#[derive(clap::Parser, Debug)]
pub struct Args {
    /// Which part of the delivery this hook command carries (1-based).
    #[arg(long)]
    pub part: usize,
}

/// The fields of the `SessionStart` stdin payload this hook uses.
#[derive(Deserialize, Default, Debug, PartialEq)]
struct SessionStartInput {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    source: String,
    /// The session's working directory: where the CLI looked for its
    /// startup files (`CLAUDE.md` up the tree, skills, `.mcp.json`).
    #[serde(default)]
    cwd: String,
}

#[derive(Serialize)]
struct PartRequest<'a> {
    block_id: &'a str,
    session_id: &'a str,
    source: &'a str,
    part: usize,
    /// So the pane's card can list the files the CLI loaded by itself.
    cwd: &'a str,
    /// The CLI's config dir (`CLAUDE_CONFIG_DIR`): the user's `CLAUDE.md`.
    config_dir: &'a str,
}

/// Entry point. Always `Ok(())`: see the module doc.
pub async fn run(args: Args) -> Result<()> {
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    let input: SessionStartInput = serde_json::from_str(&buf).unwrap_or_default();

    let Some(client) = WpsClient::from_env() else {
        tracing::debug!(target: "bashwrap", "sessionstart: sidecar env absent, nothing to deliver");
        return Ok(());
    };
    let Some(block_id) = std::env::var("AGENTMUX_BLOCKID").ok().filter(|v| !v.is_empty()) else {
        tracing::debug!(target: "bashwrap", "sessionstart: no AGENTMUX_BLOCKID, nothing to deliver");
        return Ok(());
    };
    let token = std::env::var("AGENTMUX_AGENT_TOKEN").ok();
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_default();
    let req = PartRequest {
        block_id: &block_id,
        session_id: &input.session_id,
        source: &input.source,
        part: args.part,
        cwd: &input.cwd,
        config_dir: &config_dir,
    };

    let reply = match client.post_json(PART_PATH, token.as_deref(), &req, PART_TIMEOUT).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(target: "bashwrap", part = args.part, error = %e, "sessionstart: part request failed");
            return Ok(());
        }
    };
    let Some(text) = reply.get("text").and_then(|t| t.as_str()) else {
        return Ok(());
    };

    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(hook_output(text).as_bytes()).and_then(|_| stdout.flush()).is_err() {
        return Ok(());
    }
    drop(stdout);

    if let Err(e) = client.post_json(ACK_PATH, token.as_deref(), &req, ACK_TIMEOUT).await {
        tracing::warn!(target: "bashwrap", part = args.part, error = %e, "sessionstart: ack failed");
    }
    tracing::info!(target: "bashwrap", part = args.part, source = %input.source, chars = text.chars().count(), "sessionstart: part delivered");
    Ok(())
}

/// The hook's stdout for a part: the shape Claude Code reads a
/// `SessionStart` hook's added context from.
fn hook_output(text: &str) -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": text,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_session_id_source_and_cwd_from_stdin() {
        let input: SessionStartInput = serde_json::from_str(
            r#"{"session_id":"s1","transcript_path":"/t","cwd":"/c","hook_event_name":"SessionStart","source":"compact"}"#,
        )
        .unwrap();
        assert_eq!(input, SessionStartInput { session_id: "s1".into(), source: "compact".into(), cwd: "/c".into() });
    }

    #[test]
    fn a_malformed_payload_degrades_to_empty() {
        let input: SessionStartInput = serde_json::from_str("not json").unwrap_or_default();
        assert_eq!(input, SessionStartInput::default());
    }

    #[test]
    fn prints_the_shape_claude_code_reads() {
        let out: serde_json::Value = serde_json::from_str(&hook_output("[AgentMux memory — part 1 of 1]\nhi")).unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert_eq!(out["hookSpecificOutput"]["additionalContext"], "[AgentMux memory — part 1 of 1]\nhi");
    }

    #[test]
    fn the_request_names_block_session_source_part_and_where_the_cli_runs() {
        let req = PartRequest {
            block_id: "b",
            session_id: "s",
            source: "startup",
            part: 3,
            cwd: "/ws",
            config_dir: "/cfg",
        };
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            serde_json::json!({"block_id": "b", "session_id": "s", "source": "startup", "part": 3, "cwd": "/ws", "config_dir": "/cfg"})
        );
    }
}
