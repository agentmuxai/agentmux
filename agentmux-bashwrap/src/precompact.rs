// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `precompact` subcommand.
//!
//! Registered as Claude Code's `PreCompact` hook (see
//! `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
//! §4.2 / Tier 1). Fires synchronously the instant compaction begins —
//! the only real-time "it's happening now" signal available, since
//! the `compact_boundary` stream-json frame (`agentmux-srv`'s
//! `claude.rs` translator) only arrives *after* compaction finishes.
//!
//! Contract corrections vs `PreToolUse` (see `hook.rs`, whose code
//! *pattern* this mirrors but whose payload *shape* it does not):
//!
//! - `PreCompact`'s stdin payload carries only the common hook fields
//!   (`session_id`, `transcript_path`, `cwd`, `permission_mode`,
//!   `hook_event_name`) — there is **no `trigger` field on stdin**.
//!   Claude Code requires a `matcher` (`"manual"` | `"auto"`) in
//!   `settings.json` for `PreCompact` and provides no confirmed
//!   wildcard, so `agent_config.rs` registers two separate hook
//!   entries, each invoking this binary with a different static
//!   `--trigger=` argv value baked into the command string. That's
//!   the only way this binary learns which trigger fired.
//! - This is observe-only: unlike `PreToolUse`'s `passthrough()`
//!   (which prints `{}` to mean "no opinion"), `PreCompact` must exit
//!   0 with **no stdout output at all** — not even `{}`. Any stdout
//!   here would be interpreted as hook output Claude Code has to
//!   parse; silence is the correct "no opinion" signal for this hook.
//!
//! Every failure mode (malformed stdin, missing WPS env, unreachable
//! sidecar) degrades to the same outcome: exit 0, nothing printed. A
//! hook must never block or error the user's Claude session — this
//! is best-effort observability only, matching the "degrade silently"
//! philosophy already documented in `wps_client.rs`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Read;

use crate::wps_client::WpsClient;

/// Which `PreCompact` matcher fired. Baked into argv per hook entry
/// (see the module doc comment) rather than read from stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Trigger {
    Manual,
    Auto,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Trigger::Manual => "manual",
            Trigger::Auto => "auto",
        }
    }
}

/// CLI args for `precompact`.
#[derive(clap::Parser, Debug)]
pub struct Args {
    /// Which `PreCompact` matcher fired — `manual` or `auto`. Always
    /// present: `agent_config.rs` registers one static-argv hook
    /// entry per matcher value, so this is never inferred at runtime.
    #[arg(long, value_enum)]
    pub trigger: Trigger,
}

/// Subset of the `PreCompact` stdin payload we care about. Every
/// other field (`transcript_path`, `cwd`, `permission_mode`,
/// `hook_event_name`) is ignored — serde drops unknown fields by
/// default, and there's nothing actionable in them for this hook.
#[derive(Deserialize, Default)]
struct PreCompactInput {
    #[serde(default)]
    session_id: String,
}

/// Payload published on the `compaction_started` WPS event. camelCase
/// on the wire, matching `agentmux-srv`'s `AgentEvent` convention
/// (`types.rs`) so the frontend never has to special-case this event.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompactionStartedPayload<'a> {
    trigger: &'a str,
    session_id: &'a str,
    started_at: String,
}

/// Entry point. Always returns `Ok(())` — every failure mode degrades
/// silently per the module doc comment; there is no error case this
/// hook should ever surface to Claude Code's stdout.
pub async fn run(args: Args) -> Result<()> {
    let mut buf = String::new();
    // Best-effort stdin read. A read failure just means we publish
    // with an empty session_id instead of blocking the hook.
    let _ = std::io::stdin().read_to_string(&mut buf);
    let session_id = parse_session_id(&buf);

    let Some(client) = WpsClient::from_env() else {
        tracing::debug!(
            target: "bashwrap",
            "precompact: WPS env absent, skipping publish"
        );
        return Ok(());
    };

    // Same fallback as `exec`: prefer an explicit CLI arg (precompact
    // has none) then AGENTMUX_BLOCKID, inherited from the same spawn
    // chain agentmux-srv sets up for the whole Claude subprocess tree.
    let block_id = std::env::var("AGENTMUX_BLOCKID")
        .ok()
        .filter(|v| !v.is_empty());

    let payload = build_payload(args.trigger, &session_id);

    tracing::info!(
        target: "bashwrap",
        trigger = args.trigger.as_str(),
        session_id = %session_id,
        block_id = %block_id.as_deref().unwrap_or(""),
        "precompact: publishing compaction_started"
    );

    if let Err(e) = client
        .publish_compaction_started(block_id.as_deref(), &payload)
        .await
    {
        // Best-effort observability — never fatal, never surfaced to
        // Claude Code. Just log for local debugging.
        tracing::warn!(target: "bashwrap", error = %e, "precompact: publish failed");
    }

    Ok(())
}

/// Parse `session_id` out of the `PreCompact` stdin payload. Malformed
/// or empty input yields an empty string rather than erroring — there
/// is nothing useful to do with the trigger info alone without a
/// session id, but publishing a best-effort ping is still better than
/// silently doing nothing.
fn parse_session_id(stdin_payload: &str) -> String {
    serde_json::from_str::<PreCompactInput>(stdin_payload)
        .map(|i| i.session_id)
        .unwrap_or_default()
}

fn build_payload(trigger: Trigger, session_id: &str) -> CompactionStartedPayload<'_> {
    CompactionStartedPayload {
        trigger: trigger.as_str(),
        session_id,
        started_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::sync::Mutex;

    // Mirrors wps_client.rs's ENV_LOCK — std::env mutation races under
    // cargo test's default parallel execution.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        std::env::remove_var("AGENTMUX_LOCAL_URL");
        std::env::remove_var("AGENTMUX_AUTH_KEY");
        std::env::remove_var("AGENTMUX_BLOCKID");
    }

    // ── argv parsing ──────────────────────────────────────────────

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn parses_trigger_manual() {
        let cli = TestCli::parse_from(["precompact", "--trigger=manual"]);
        assert_eq!(cli.args.trigger, Trigger::Manual);
    }

    #[test]
    fn parses_trigger_auto() {
        let cli = TestCli::parse_from(["precompact", "--trigger=auto"]);
        assert_eq!(cli.args.trigger, Trigger::Auto);
    }

    #[test]
    fn rejects_unknown_trigger_value() {
        let result = TestCli::try_parse_from(["precompact", "--trigger=sometimes"]);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_trigger() {
        let result = TestCli::try_parse_from(["precompact"]);
        assert!(result.is_err());
    }

    // ── stdin session_id parsing ──────────────────────────────────

    #[test]
    fn parses_session_id_from_full_precompact_payload() {
        // Real PreCompact stdin shape: only the common hook fields,
        // no `trigger`.
        let payload = serde_json::json!({
            "session_id": "sess-abc123",
            "transcript_path": "/tmp/sess-abc123.jsonl",
            "cwd": "/home/user/project",
            "permission_mode": "default",
            "hook_event_name": "PreCompact"
        })
        .to_string();
        assert_eq!(parse_session_id(&payload), "sess-abc123");
    }

    #[test]
    fn malformed_stdin_yields_empty_session_id() {
        assert_eq!(parse_session_id("not json"), "");
        assert_eq!(parse_session_id(""), "");
    }

    #[test]
    fn missing_session_id_field_yields_empty_string() {
        assert_eq!(parse_session_id(r#"{"cwd":"/tmp"}"#), "");
    }

    // ── payload shape ──────────────────────────────────────────────

    #[test]
    fn build_payload_manual_shape() {
        let payload = build_payload(Trigger::Manual, "sess-1");
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["trigger"], "manual");
        assert_eq!(v["sessionId"], "sess-1");
        assert!(v["startedAt"].as_str().unwrap().contains('T'), "expects RFC3339");
    }

    #[test]
    fn build_payload_auto_shape() {
        let payload = build_payload(Trigger::Auto, "sess-2");
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["trigger"], "auto");
        assert_eq!(v["sessionId"], "sess-2");
    }

    #[test]
    fn build_payload_camel_cases_wire_fields() {
        let payload = build_payload(Trigger::Manual, "");
        let v = serde_json::to_value(&payload).unwrap();
        let obj = v.as_object().unwrap();
        assert!(obj.contains_key("sessionId"));
        assert!(obj.contains_key("startedAt"));
        assert!(!obj.contains_key("session_id"), "must be camelCase on the wire");
    }

    // ── env-driven degrade-gracefully behavior ──────────────────────

    #[test]
    fn from_env_none_without_wps_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        // Mirrors run()'s early-return path: no client is built, so
        // run() would return Ok(()) without attempting a publish.
        assert!(WpsClient::from_env().is_none());
        clear_env();
    }

    #[test]
    fn block_id_falls_back_to_env_when_unset() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("AGENTMUX_BLOCKID", "block-42");
        let block_id = std::env::var("AGENTMUX_BLOCKID")
            .ok()
            .filter(|v| !v.is_empty());
        assert_eq!(block_id.as_deref(), Some("block-42"));
        clear_env();
    }

    #[test]
    fn block_id_empty_env_treated_as_absent() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("AGENTMUX_BLOCKID", "");
        let block_id = std::env::var("AGENTMUX_BLOCKID")
            .ok()
            .filter(|v| !v.is_empty());
        assert_eq!(block_id, None);
        clear_env();
    }
}
