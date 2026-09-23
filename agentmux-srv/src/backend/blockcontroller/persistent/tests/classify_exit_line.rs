// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::*;
use crate::agents::failure::FailureClass;

#[test]
fn json_result_frame_with_overloaded_text_is_surfaced() {
    let line = r#"{"type":"result","is_error":true,"result":"Overloaded"}"#;
    let failure = classify_exit_line(Some(1), line).expect("overloaded should surface");
    assert_eq!(failure.code, FailureClass::Overloaded);
    assert!(failure.retryable);
}

#[test]
fn json_result_frame_with_rate_limit_text_is_surfaced() {
    let line = r#"{"type":"result","is_error":true,"result":"429 rate limited, please retry"}"#;
    let failure = classify_exit_line(Some(1), line).expect("rate-limited should surface");
    assert_eq!(failure.code, FailureClass::RateLimited);
    assert!(failure.retryable);
}

#[test]
fn non_json_raw_text_falls_back_to_keyword_matching() {
    // FlushErrorLine can carry an earlier held-back turn's raw line,
    // not guaranteed to be well-formed JSON — the fallback path must
    // still classify it from the raw text.
    let line = "connection error: rate limited (429)";
    let failure = classify_exit_line(Some(1), line).expect("raw-text 429 should surface");
    assert_eq!(failure.code, FailureClass::RateLimited);
}

#[test]
fn unrecognized_json_error_is_suppressed() {
    // A real error frame, but with no keyword classify() recognizes —
    // must not produce a low-confidence UnknownNonZero banner.
    let line = r#"{"type":"result","is_error":true,"result":"something unusual happened"}"#;
    assert_eq!(classify_exit_line(Some(1), line), None);
}

#[test]
fn unrecognized_raw_text_is_suppressed() {
    let line = "some unrelated noise flushed from an earlier turn";
    assert_eq!(classify_exit_line(Some(1), line), None);
}

#[test]
fn auth_error_is_surfaced_even_though_not_retryable() {
    // Non-retryable classes still have recovery-banner value (Login
    // Again / Armory actions) — only UnknownNonZero is suppressed.
    let line = r#"{"type":"result","is_error":true,"result":"invalid api key (401)"}"#;
    let failure = classify_exit_line(Some(1), line).expect("auth errors should still surface");
    assert_eq!(failure.code, FailureClass::Auth);
    assert!(!failure.retryable);
}

#[test]
fn no_exit_code_still_classifies_from_line_content() {
    // Mid-generation flush sites (a superseded spawn generation, a
    // SessionCaptured resolution) have no fresh exit code for the line
    // being flushed — classification must still work from content alone.
    let line = r#"{"type":"result","is_error":true,"result":"Overloaded"}"#;
    let failure = classify_exit_line(None, line).expect("overloaded should surface without an exit code");
    assert_eq!(failure.code, FailureClass::Overloaded);
}
