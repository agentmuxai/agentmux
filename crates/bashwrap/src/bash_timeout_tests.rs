// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const DEFAULT: Option<u64> = Some(30_000);

fn timeout(input: Value, default_ms: Option<u64>) -> Option<u64> {
    timeout_for(input.as_object().expect("an object"), default_ms)
}

#[test]
fn git_is_found_as_a_word_and_not_inside_another() {
    for yes in ["git push", "cd x && git commit -m y", "echo hi; git status", "(git fetch)", "git", "sudo git pull", "~/git/x", "git-lfs pull", "FOO=1 git log"] {
        assert!(mentions_git(yes), "{yes}");
    }
    for no in ["echo digit", "gitleaks detect", "cargo build", "legit_tool run", "mygit pull", "git2 x", ""] {
        assert!(!mentions_git(no), "{no}");
    }
}

/// The measured table (Claude Code 2.1.218): what the CLI stops at the timeout and what it moves.
#[test]
fn the_commands_the_cli_stops_are_told_from_the_ones_it_moves() {
    for stopped in [
        "git push origin x",
        "echo $(date) && make",
        "X=1; echo ${X} && make",
        "echo `date` && make",
        "diff <(echo a) <(echo a); make",
        "echo {a,b} && make",
        "echo {1..3}; make",
        "echo $'a\\tb' && make",
        "curl -d '{\"a\":1}' http://x",
    ] {
        assert!(stopped_at_timeout(stopped), "{stopped}");
    }
    for moved in [
        "cargo build --release",
        "cd ~/.agentmux && make",
        "echo $HOME > /dev/null && make 2>&1",
        "ls *.py && make",
        "for i in 1; do make; done",
        "if true; then make; fi",
        "(make)",
        "bash -c 'make'",
        "eval \"make\"",
        "cat <<< hi && make",
        "make <<'EOF'\nhi\nEOF",
        "make & wait",
        "make | tail -n 5",
        "make || echo failed",
    ] {
        assert!(!stopped_at_timeout(moved), "{moved}");
    }
}

#[test]
fn a_call_that_waits_longer_than_the_default_is_clamped_to_it() {
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 600_000 }), DEFAULT), Some(30_000));
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 30_001 }), DEFAULT), Some(30_000));
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 600_000.0 }), DEFAULT), Some(30_000), "a float is a timeout too");
}

#[test]
fn a_call_with_a_short_timeout_or_none_is_left_alone() {
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 10_000 }), DEFAULT), None, "the model's shorter timeout stands");
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 30_000 }), DEFAULT), None);
    assert_eq!(timeout(json!({ "command": "cargo build" }), DEFAULT), None, "the CLI's default applies");
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": null }), DEFAULT), None);
}

#[test]
fn a_call_the_cli_would_stop_gets_five_minutes_only_when_it_set_no_timeout() {
    for command in ["git push", "echo $(date) && make", "echo {a,b}"] {
        assert_eq!(timeout(json!({ "command": command }), DEFAULT), Some(LONG_TIMEOUT_MS), "{command}");
        assert_eq!(timeout(json!({ "command": command, "timeout": null }), DEFAULT), Some(LONG_TIMEOUT_MS));
        for chosen in [5_000, 60_000, LONG_TIMEOUT_MS, 600_000] {
            assert_eq!(timeout(json!({ "command": command, "timeout": chosen }), DEFAULT), None, "a timeout the model chose stands: {command} {chosen}");
        }
    }
}

#[test]
fn a_background_call_is_left_alone() {
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 600_000, "run_in_background": true }), DEFAULT), None);
    assert_eq!(timeout(json!({ "command": "git clone x", "run_in_background": true }), DEFAULT), None);
    assert_eq!(timeout(json!({ "command": "cargo build", "timeout": 600_000, "run_in_background": false }), DEFAULT), Some(30_000));
}

#[test]
fn nothing_is_done_without_a_short_default() {
    let call = json!({ "command": "git push", "timeout": 600_000 });
    let build = json!({ "command": "cargo build", "timeout": 600_000 });
    for default_ms in [None, Some(LONG_TIMEOUT_MS), Some(LONG_TIMEOUT_MS + 1)] {
        assert_eq!(timeout(call.clone(), default_ms), None);
        assert_eq!(timeout(build.clone(), default_ms), None);
    }
    assert_eq!(timeout(build, Some(45_000)), Some(45_000), "the clamp is the default in force");
}

#[test]
fn a_malformed_input_is_left_alone() {
    assert!(response(&json!("git push"), DEFAULT).is_none());
    assert!(response(&json!({ "command": 7 }), DEFAULT).is_none());
    assert!(response(&json!({}), DEFAULT).is_none());
    assert!(response(&json!({ "command": "cargo build", "timeout": "soon" }), DEFAULT).is_none());
    assert!(response(&json!({ "command": "git push", "timeout": -5 }), DEFAULT).is_none());
}

#[test]
fn the_answer_keeps_every_field_and_decides_no_permission() {
    let input = json!({ "command": "cargo build", "description": "Build the app", "timeout": 600_000, "run_in_background": false });
    let out = response(&input, DEFAULT).expect("answered");
    let hso = &out["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "PreToolUse");
    assert_eq!(hso["updatedInput"]["timeout"], 30_000);
    assert_eq!(hso["updatedInput"]["command"], "cargo build");
    assert_eq!(hso["updatedInput"]["description"], "Build the app");
    assert_eq!(hso["updatedInput"]["run_in_background"], false);
    assert!(hso.get("permissionDecision").is_none(), "the user's permission rules must still decide");
}
