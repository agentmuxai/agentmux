// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `classify`'s order and fallbacks: which cause wins when a failure's text names more than one,
//! what each class says, and what the stderr tail keeps.

use super::*;
use serde_json::json;

fn class_of(stderr: &str) -> FailureClass {
    classify(Some(1), None, stderr, None).code
}

#[test]
fn a_turn_cap_named_in_the_text_is_the_turn_limit() {
    for stderr in ["Reached max turns (10)", "error: --max-turns exceeded", "exceeded the maximum number of turns"] {
        let f = classify(Some(1), None, stderr, None);
        assert_eq!(f.code, FailureClass::MaxTurns, "{stderr}");
        assert_eq!(f.title, "Hit the turn limit");
        assert!(!f.retryable);
    }
}

#[test]
fn the_plain_billing_words_are_a_usage_limit_that_a_retry_cannot_fix() {
    for stderr in ["quota exceeded for this key", "there is a billing problem", "insufficient funds", "you are out of credits"] {
        let f = classify(Some(1), None, stderr, None);
        assert_eq!(f.code, FailureClass::UsageLimit, "{stderr}");
        assert_eq!(f.title, "Usage limit reached");
        assert!(!f.retryable, "{stderr}");
    }
}

#[test]
fn a_signal_or_a_turn_cap_frame_wins_over_whatever_the_text_says() {
    let killed = classify(Some(1), Some(9), "Rate limited · overloaded · 401", None);
    assert_eq!(killed.code, FailureClass::Killed);
    assert_eq!((killed.signal, killed.title.as_str()), (Some(9), "Agent process was killed"));
    assert!(killed.detail.contains("out-of-memory killer"));

    let frame = json!({ "type": "result", "is_error": true, "subtype": "error_max_turns" });
    let capped = classify(Some(137), Some(9), "overloaded", Some(&frame));
    assert_eq!(capped.code, FailureClass::MaxTurns, "the frame's subtype is checked first");
}

#[test]
fn an_overload_is_not_taken_for_a_rate_limit_and_both_are_retryable() {
    let overloaded = classify(Some(1), None, "HTTP 529 overloaded_error", None);
    assert_eq!((overloaded.code, overloaded.retryable), (FailureClass::Overloaded, true));
    assert_eq!(overloaded.title, "API temporarily overloaded");
    let limited = classify(Some(1), None, "error 429 · overloaded", None);
    assert_eq!(limited.code, FailureClass::RateLimited, "the rate limit is checked before the overload");
    assert_eq!(limited.title, "Rate-limited by the API");
}

#[test]
fn a_rejected_login_wins_over_a_context_message_and_a_plan_limit_wins_over_both() {
    assert_eq!(class_of("401 Unauthorized · prompt is too long"), FailureClass::Auth);
    assert_eq!(class_of("prompt is too long · You've hit your weekly limit"), FailureClass::UsageLimit);
    assert_eq!(class_of("context window exceeded"), FailureClass::ContextExceeded);
    assert_eq!(class_of("the context is too long and the network dropped"), FailureClass::ContextExceeded);
}

#[test]
fn a_usage_limit_is_checked_before_the_network_words() {
    assert_eq!(class_of("quota exceeded after a network timeout"), FailureClass::UsageLimit);
    assert_eq!(class_of("timeout waiting for the response"), FailureClass::Network);
}

#[test]
fn the_spawn_gate_refusals_are_not_retryable_and_say_how_to_get_out() {
    let account = classify(Some(1), None, "no credentials for claude: Bind an account for this provider in Connectors → Accounts.", None);
    assert_eq!((account.code, account.title.as_str(), account.retryable), (FailureClass::Auth, "No account linked", false));
    assert!(account.detail.contains("Sign in to link one"), "{}", account.detail);
    let deleted = classify(Some(1), None, "this agent was deleted, so this pane cannot start", None);
    assert_eq!((deleted.code, deleted.title.as_str(), deleted.retryable), (FailureClass::AgentDeleted, "Agent was deleted", false));
}

#[test]
fn the_fallbacks_say_what_is_known() {
    let frame = json!({ "type": "result", "is_error": true, "subtype": "error_during_execution", "result": "something odd happened" });
    let reported = classify(Some(0), None, "", Some(&frame));
    assert_eq!((reported.code, reported.title.as_str(), reported.retryable), (FailureClass::UnknownNonZero, "Agent reported an error", false));
    assert!(reported.stderr_tail.contains("something odd happened"), "the frame's text is part of the evidence");

    let silent = classify(Some(0), None, "", None);
    assert_eq!((silent.code, silent.title.as_str()), (FailureClass::NoOutput, "Agent produced no output"));
    let failed = classify(Some(2), None, "boom", None);
    assert_eq!((failed.code, failed.title.as_str(), failed.exit_code), (FailureClass::UnknownNonZero, "Agent failed", Some(2)));
    assert_eq!(failed.stderr_tail, "boom");
    // No exit code and no signal: nothing to go on, so it is not a clean exit.
    assert_eq!(classify(None, None, "", None).code, FailureClass::UnknownNonZero);
}

#[test]
fn the_tail_keeps_the_last_lines_of_stderr_and_the_frames_text_after_them() {
    let stderr = (0..40).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
    let f = classify(Some(2), None, &stderr, None);
    let lines: Vec<&str> = f.stderr_tail.lines().collect();
    assert_eq!((lines.len(), lines[0], lines[23]), (24, "line16", "line39"));

    let frame = json!({ "type": "result", "is_error": true, "result": "the frame said this" });
    let with_frame = classify(Some(2), None, "stderr said that", Some(&frame));
    assert_eq!(with_frame.stderr_tail, "stderr said that\nthe frame said this");
}

#[test]
fn a_failure_explains_itself_with_its_evidence_and_tail() {
    let f = classify(Some(2), None, "boom", None);
    assert_eq!(f.explain(), "Agent failed — The agent exited with a non-zero status and no recognized error. The raw stderr tail is included below. [exit 2]\n--- claude stderr (tail) ---\nboom");
}
