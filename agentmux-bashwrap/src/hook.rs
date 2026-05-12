// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `hook` subcommand.
//!
//! Reads a PreToolUse JSON event on stdin (the contract Claude Code
//! uses for hooks: see `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`
//! §5). If the event is a Bash invocation whose `command` hasn't
//! already been wrapped, emit an `updatedInput.command` that invokes
//! `agentmux-bashwrap exec` with the original command base64-encoded
//! into argv. Idempotent — if the command is already wrapped, pass
//! through with no rewrite.
//!
//! All errors degrade to a pass-through response so a hook failure
//! never blocks Claude's tool execution.

use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};

/// Subset of the PreToolUse payload we care about. Extra fields are
/// ignored (serde default).
#[derive(Deserialize)]
struct PreToolUseInput {
    tool_name: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default)]
    tool_input: Value,
}

const WRAPPER_BINARY: &str = "agentmux-bashwrap";
const WRAPPED_PREFIX: &str = "agentmux-bashwrap exec";

pub fn run_pretooluse_bash() -> Result<()> {
    // Read entire stdin. Hook payloads are small.
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;

    let response = build_response(&buf);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", response)?;
    Ok(())
}

/// Build the hook response for a PreToolUse payload. Public for
/// testability — callers in tests feed in a payload string and assert
/// the response shape without going through stdin.
pub fn build_response(stdin_payload: &str) -> Value {
    let input: PreToolUseInput = match serde_json::from_str(stdin_payload) {
        Ok(i) => i,
        Err(_) => {
            // Malformed hook input → pass through. Claude proceeds
            // with native Bash, no streaming for this call.
            return passthrough();
        }
    };

    if input.tool_name != "Bash" {
        return passthrough();
    }

    let command = input
        .tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Idempotence: if the command already invokes our wrapper, don't
    // re-wrap. Defends against weird hook re-fire scenarios.
    if command.starts_with(WRAPPED_PREFIX) {
        return passthrough();
    }

    let b64 = URL_SAFE_NO_PAD.encode(command.as_bytes());
    let wrapped = format!(
        "{} exec --tool-id={} --b64-cmd={}",
        WRAPPER_BINARY,
        shell_quote(&input.tool_use_id),
        b64
    );

    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "command": wrapped
            }
        }
    })
}

fn passthrough() -> Value {
    // Empty object → Claude Code treats as "no opinion", proceeds.
    json!({})
}

/// Quote a value for embedding in a shell argv position.
///
/// The tool-use-id is provider-generated and currently always opaque
/// ASCII, but be defensive: if it contains a non-safe char, escape
/// per the host shell's rules.
///
/// - **Unix shells** (bash/sh/zsh): single-quote wrap with `'\''`
///   escape for embedded single quotes.
/// - **Windows `cmd.exe /C`**: cmd doesn't interpret single quotes
///   as quoting; we use double quotes and escape embedded `"` per
///   cmd.exe rules. This is the shell Claude Code's Bash tool
///   invokes on Win32.
///
/// Pure-alphanumeric ids return unchanged on every platform.
fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return s.to_string();
    }
    #[cfg(windows)]
    {
        // cmd.exe: double-quote wrap, escape `"` as `""`. Reasonable
        // approximation — full cmd.exe quoting is famously gnarly,
        // but for opaque-ID-with-weird-char defense this is enough.
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    }
    #[cfg(not(windows))]
    {
        // bash/sh/zsh: single-quote wrap, escape `'` as `'\''`.
        let escaped = s.replace('\'', r"'\''");
        format!("'{}'", escaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(cmd: &str, id: &str) -> String {
        json!({
            "tool_name": "Bash",
            "tool_use_id": id,
            "tool_input": { "command": cmd },
            "session_id": "sess-1",
            "hook_event_name": "PreToolUse"
        })
        .to_string()
    }

    #[test]
    fn rewrites_simple_bash_command() {
        let resp = build_response(&payload("echo hi", "toolu_abc"));
        let cmd = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.starts_with("agentmux-bashwrap exec"));
        assert!(cmd.contains("--tool-id=toolu_abc"));
        // base64("echo hi") = ZWNobyBoaQ
        assert!(cmd.contains("--b64-cmd=ZWNobyBoaQ"));
        assert_eq!(
            resp["hookSpecificOutput"]["permissionDecision"].as_str(),
            Some("allow")
        );
    }

    #[test]
    fn preserves_multi_line_and_quotes_via_base64() {
        let cmd = r#"echo "hello" && echo 'multi
line' && cat $HOME/.env"#;
        let resp = build_response(&payload(cmd, "toolu_x"));
        let wrapped = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        // The wrapped form must round-trip — pull the b64 back and decode.
        let b64 = wrapped
            .split("--b64-cmd=")
            .nth(1)
            .expect("b64 arg present");
        let decoded = URL_SAFE_NO_PAD.decode(b64.as_bytes()).unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        assert_eq!(decoded, cmd);
    }

    #[test]
    fn is_no_op_on_already_wrapped_command() {
        let cmd =
            "agentmux-bashwrap exec --tool-id=toolu_y --b64-cmd=ZWNobyBoaQ";
        let resp = build_response(&payload(cmd, "toolu_y"));
        assert!(
            resp.as_object().map(|m| m.is_empty()).unwrap_or(false),
            "should pass through (empty object), got {}",
            resp
        );
    }

    #[test]
    fn passes_through_non_bash_tools() {
        let payload = json!({
            "tool_name": "Read",
            "tool_use_id": "toolu_r",
            "tool_input": { "file_path": "x.txt" }
        })
        .to_string();
        let resp = build_response(&payload);
        assert!(
            resp.as_object().map(|m| m.is_empty()).unwrap_or(false),
            "non-Bash should pass through"
        );
    }

    #[test]
    fn passes_through_malformed_input() {
        let resp = build_response("not json");
        assert!(
            resp.as_object().map(|m| m.is_empty()).unwrap_or(false),
            "malformed → passthrough"
        );
    }

    #[test]
    fn rewrites_with_empty_b64_when_command_field_missing() {
        // Codex P2 round 3: the previous name `passes_through_when_
        // command_field_missing` lied about behavior, and the inline
        // comment claimed `bash -c ""` would "fail at wrapper start
        // time" — which is wrong; an empty bash command exits 0
        // silently. Renamed + comment corrected to describe what
        // actually happens.
        //
        // Behavior under test: when `tool_input.command` is missing,
        // we still emit a rewrite (with `--b64-cmd=` of an empty
        // string). The wrapper then runs `bash -c ""` which exits 0
        // immediately. Net effect: a no-op Bash call streams a "0
        // chars" system chunk + an empty body, and Claude sees an
        // empty tool_result. Not catastrophic; the empty input was
        // already going to do nothing.
        let payload = json!({
            "tool_name": "Bash",
            "tool_use_id": "toolu_z",
            "tool_input": {}
        })
        .to_string();
        let resp = build_response(&payload);
        let cmd = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        // The b64 of an empty string is the empty string, so we
        // expect `--b64-cmd=` immediately followed by whatever ends
        // the command (whitespace or EOL). Match the prefix.
        assert!(
            cmd.contains("--b64-cmd=") && (cmd.ends_with("--b64-cmd=") || cmd.contains("--b64-cmd= ")),
            "expected empty b64-cmd argument, got: {}",
            cmd
        );
    }

    #[test]
    fn shell_quote_passes_safe_ids_unchanged() {
        assert_eq!(shell_quote("toolu_abc-123"), "toolu_abc-123");
    }

    #[cfg(not(windows))]
    #[test]
    fn shell_quote_wraps_unsafe_ids_unix() {
        let q = shell_quote("weird id");
        assert_eq!(q, "'weird id'");
    }

    #[cfg(not(windows))]
    #[test]
    fn shell_quote_escapes_embedded_single_quotes_unix() {
        // Standard bash idiom: `'` → `'\''` (close, escaped, reopen).
        // For `o'malley` the result is `'o'\''malley'`.
        let q = shell_quote("o'malley");
        assert_eq!(q, r"'o'\''malley'");
    }

    #[cfg(windows)]
    #[test]
    fn shell_quote_wraps_unsafe_ids_windows() {
        let q = shell_quote("weird id");
        assert_eq!(q, "\"weird id\"");
    }

    #[cfg(windows)]
    #[test]
    fn shell_quote_escapes_embedded_double_quotes_windows() {
        // cmd.exe `""` is the escape for a literal `"` inside a
        // double-quoted argument. For `a"b` the result is `"a""b"`.
        let q = shell_quote("a\"b");
        assert_eq!(q, "\"a\"\"b\"");
    }
}
