// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn always_covers_this_agent_and_these_two_commands_only() {
    let k = grant_key("Agent2-6", "gh secret set X", "gh api /user");
    assert_ne!(k, grant_key("Agent2-6", "gh secret set X", "gh api /octocat"), "another verify asks again");
    assert_ne!(k, grant_key("Agent1", "gh secret set X", "gh api /user"), "another agent asks again");
    assert_ne!(k, super::super::shell_secrets::grant_key("Agent2-6", "value"), "a command grant isn't a rotation grant");
}

#[test]
fn the_prompt_names_both_commands() {
    assert_eq!(shown_commands("set-it", ""), "set: set-it");
    assert_eq!(shown_commands("set-it", "check-it"), "set: set-it\nverify: check-it");
}

#[test]
fn only_an_exit_code_of_zero_is_success() {
    assert!(Ran { exit_code: Some(0), output: vec![] }.ok());
    assert!(!Ran { exit_code: Some(1), output: vec![] }.ok());
    assert!(!Ran { exit_code: None, output: vec![] }.ok(), "stopped, or killed by a signal");
}
