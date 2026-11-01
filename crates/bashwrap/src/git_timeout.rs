// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A `git` command gets a timeout long enough to finish.
//!
//! AgentMux has Claude Code move a Bash call to the background once it has run for
//! `BASH_DEFAULT_TIMEOUT_MS` (30 s), so the agent can take the next message. The CLI does not move a
//! command that contains `git`: it stops it at the timeout. A `git push`, a clone or a commit with
//! slow hooks would be killed at 30 s, so the `PreToolUse` hook gives such a call a timeout of its own
//! when the model set none. The hook answers `updatedInput` only: no permission decision, so the
//! user's `allow`, `ask` and `deny` rules still apply to the call.

use serde_json::{json, Value};

/// The timeout a `git` command gets when the model set none: long enough for a push or a clone.
pub const GIT_TIMEOUT_MS: u64 = 5 * 60 * 1000;

/// The CLI's default Bash timeout, which AgentMux sets at launch (`backend/bash_background.rs` in srv).
const DEFAULT_ENV: &str = "BASH_DEFAULT_TIMEOUT_MS";

/// Whether `command` contains `git` as a word, which is what makes the CLI stop it at the timeout. A
/// path like `~/git/x` counts (a few more calls get a longer timeout than need it); `digit` and
/// `gitleaks` do not.
pub fn mentions_git(command: &str) -> bool {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = command.as_bytes();
    command.match_indices("git").any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + 3).copied();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// The hook's `updatedInput` answer for this Bash call, or `None` to leave it alone: when the call
/// mentions `git`, sets no `timeout` and is not a background call, and the CLI's default timeout
/// (`default_ms`, from `BASH_DEFAULT_TIMEOUT_MS`) is shorter than [`GIT_TIMEOUT_MS`]. Every other field
/// the model set is carried through, as `updatedInput` replaces the whole input.
pub fn response(tool_input: &Value, default_ms: Option<u64>) -> Option<Value> {
    let fields = tool_input.as_object()?;
    let command = fields.get("command")?.as_str()?;
    let sets_timeout = fields.get("timeout").is_some_and(|t| !t.is_null());
    let background = fields.get("run_in_background").and_then(Value::as_bool).unwrap_or(false);
    if sets_timeout || background || !mentions_git(command) || !default_ms.is_some_and(|d| d < GIT_TIMEOUT_MS) {
        return None;
    }
    let mut updated = fields.clone();
    updated.insert("timeout".to_string(), json!(GIT_TIMEOUT_MS));
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "updatedInput": updated } }))
}

/// [`response`] with the default read from this process's environment (the CLI's, which it inherits).
pub fn response_from_env(tool_input: &Value) -> Option<Value> {
    response(tool_input, std::env::var(DEFAULT_ENV).ok().and_then(|v| v.trim().parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_found_as_a_word_and_not_inside_another() {
        for yes in ["git push", "cd x && git commit -m y", "echo hi; git status", "(git fetch)", "git", "sudo git pull", "~/git/x", "git-lfs pull", "FOO=1 git log"] {
            assert!(mentions_git(yes), "{yes}");
        }
        for no in ["echo digit", "gitleaks detect", "cargo build", "legit_tool run", "mygit pull", "git2 x", ""] {
            assert!(!mentions_git(no), "{no}");
        }
    }

    #[test]
    fn a_git_call_with_no_timeout_gets_five_minutes_and_keeps_its_other_fields() {
        let input = json!({ "command": "git push origin x", "description": "Push the branch", "run_in_background": false });
        let out = response(&input, Some(30_000)).expect("answered");
        let hso = &out["hookSpecificOutput"];
        assert_eq!(hso["hookEventName"], "PreToolUse");
        assert_eq!(hso["updatedInput"]["timeout"], 300_000);
        assert_eq!(hso["updatedInput"]["command"], "git push origin x");
        assert_eq!(hso["updatedInput"]["description"], "Push the branch");
        assert!(hso.get("permissionDecision").is_none(), "the user's permission rules must still decide");
    }

    #[test]
    fn nothing_changes_when_the_model_set_a_timeout_or_it_is_a_background_call_or_not_git() {
        assert!(response(&json!({ "command": "git push", "timeout": 600_000 }), Some(30_000)).is_none());
        assert!(response(&json!({ "command": "git push", "timeout": 1_000 }), Some(30_000)).is_none(), "the model's own shorter timeout stands");
        assert!(response(&json!({ "command": "git clone x", "run_in_background": true }), Some(30_000)).is_none());
        assert!(response(&json!({ "command": "cargo build" }), Some(30_000)).is_none());
        assert!(response(&json!({ "command": "git push", "timeout": null }), Some(30_000)).is_some(), "a null timeout is none");
    }

    #[test]
    fn it_acts_only_while_the_default_is_shorter_than_the_git_timeout() {
        let input = json!({ "command": "git push" });
        assert!(response(&input, None).is_none(), "no default set: the CLI's own 2 minutes stand");
        assert!(response(&input, Some(GIT_TIMEOUT_MS)).is_none());
        assert!(response(&input, Some(GIT_TIMEOUT_MS + 1)).is_none());
        assert!(response(&input, Some(30_000)).is_some());
    }

    #[test]
    fn a_malformed_input_is_left_alone() {
        assert!(response(&json!("git push"), Some(30_000)).is_none());
        assert!(response(&json!({ "command": 7 }), Some(30_000)).is_none());
        assert!(response(&json!({}), Some(30_000)).is_none());
    }
}
