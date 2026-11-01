// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The timeout of a Bash call, set by the `PreToolUse` hook so the agent is free after 30 seconds.
//!
//! AgentMux has Claude Code move a Bash call to the background once it has run for
//! `BASH_DEFAULT_TIMEOUT_MS` (30 s), so the agent can take the next message. That default applies only
//! to a call with no `timeout` of its own: a model that sets one (a build, a wait on the build slot)
//! holds its turn for it. So the hook clamps a longer `timeout` to the default.
//!
//! The CLI does not move every command to the background. A command it cannot parse into simple
//! commands is stopped at the timeout instead (exit 143): one with `git` in it, a command or process
//! substitution, a parameter, brace or `$'...'` expansion. Measured on 2.1.218; `$VAR`, `~`, globs,
//! loops, `if`, subshells, `bash -c`, `eval`, heredocs, redirects, pipes, `&&`, `;` and `&` are
//! moved. For a call of that kind the hook does the reverse and gives a timeout long enough to
//! finish when the model set none; a timeout the model chose stands, as it did before the 30 s
//! default. It answers `updatedInput` only, with no permission decision, so the user's `allow`,
//! `ask` and `deny` rules still apply to the call.

use serde_json::{json, Value};

/// The timeout a call the CLI would stop gets when the model set none: long enough for a push or a clone.
pub const LONG_TIMEOUT_MS: u64 = 5 * 60 * 1000;

/// The CLI's default Bash timeout, which AgentMux sets at launch (`backend/bash_background.rs` in srv).
const DEFAULT_ENV: &str = "BASH_DEFAULT_TIMEOUT_MS";

/// Spellings that make the CLI stop a command at the timeout instead of moving it to the background.
/// Wider than what was measured on purpose: a `{` alone, or one inside quotes, is also taken as one, so
/// a call is clamped only when it is known to be moved.
const STOPPED_SPELLINGS: [&str; 6] = ["$(", "$'", "`", "<(", ">(", "{"];

/// Whether `command` contains `git` as a word. A path like `~/git/x` counts; `digit` and `gitleaks` do not.
pub fn mentions_git(command: &str) -> bool {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = command.as_bytes();
    command.match_indices("git").any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + 3).copied();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// Whether the CLI may stop `command` at the timeout rather than move it to the background.
pub fn stopped_at_timeout(command: &str) -> bool {
    mentions_git(command) || STOPPED_SPELLINGS.iter().any(|s| command.contains(s))
}

/// The call's `timeout` in ms, when it sets one.
fn timeout_of(fields: &serde_json::Map<String, Value>) -> Result<Option<u64>, ()> {
    match fields.get("timeout") {
        None | Some(Value::Null) => Ok(None),
        // A float (`600000.0`) is a timeout too; anything else is not ours to read.
        Some(v) => v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)).map(Some).ok_or(()),
    }
}

/// The timeout this call should have, or `None` to leave it as it is.
///
/// - `default_ms`: the CLI's default (`BASH_DEFAULT_TIMEOUT_MS`). Nothing is done without one shorter than
///   [`LONG_TIMEOUT_MS`], so turning the 30 s default off turns this off too.
/// - A call the CLI would stop gets [`LONG_TIMEOUT_MS`] when it sets no `timeout`; one it sets stands.
/// - Any other call that sets a `timeout` longer than the default is clamped to the default, which is what
///   moves it to the background then. A shorter one stands, and so does a call with none (the default applies).
/// - A background call (`run_in_background`) is left alone.
pub fn timeout_for(fields: &serde_json::Map<String, Value>, default_ms: Option<u64>) -> Option<u64> {
    let default_ms = default_ms.filter(|d| *d < LONG_TIMEOUT_MS)?;
    let command = fields.get("command")?.as_str()?;
    if fields.get("run_in_background").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let set = timeout_of(fields).ok()?;
    if stopped_at_timeout(command) {
        return set.is_none().then_some(LONG_TIMEOUT_MS);
    }
    set.filter(|t| *t > default_ms).map(|_| default_ms)
}

/// The hook's `updatedInput` answer for this Bash call, or `None` to leave it alone. Every other field the
/// model set is carried through, as `updatedInput` replaces the whole input.
pub fn response(tool_input: &Value, default_ms: Option<u64>) -> Option<Value> {
    let fields = tool_input.as_object()?;
    let timeout = timeout_for(fields, default_ms)?;
    let mut updated = fields.clone();
    updated.insert("timeout".to_string(), json!(timeout));
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "updatedInput": updated } }))
}

/// [`response`] with the default read from this process's environment (the CLI's, which it inherits).
pub fn response_from_env(tool_input: &Value) -> Option<Value> {
    response(tool_input, std::env::var(DEFAULT_ENV).ok().and_then(|v| v.trim().parse().ok()))
}

#[cfg(test)]
#[path = "bash_timeout_tests.rs"]
mod tests;
