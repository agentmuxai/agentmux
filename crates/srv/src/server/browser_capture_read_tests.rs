// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use agentmux_common::api_types::UiAutomationAuth;

fn request() -> CaptureRequest {
    CaptureRequest {
        auth: UiAutomationAuth { agent_id: "Agent2-6".into(), ts_secs: 0, sig: String::new() },
        pane: None,
        ref_: None,
        card: None,
        into: None,
        entry: None,
        download: false,
        timeout_ms: None,
        reason: None,
        near: None,
        prefix: None,
        pattern: None,
        copy: false,
    }
}

#[test]
fn a_capture_names_a_reference_or_the_label_beside_the_text_and_not_both() {
    let by_ref = CaptureRequest { ref_: Some("e4".into()), ..request() };
    assert_eq!(what_to_read(&by_ref).unwrap().0, "e4");
    let by_label = CaptureRequest { near: Some("  Client secret ".into()), ..request() };
    assert_eq!(what_to_read(&by_label).unwrap().0, "", "the host finds it and gives it a reference");
    let both = CaptureRequest { ref_: Some("e4".into()), near: Some("Client secret".into()), ..request() };
    assert!(what_to_read(&both).unwrap_err().contains("one or the other"));
    let neither = what_to_read(&request()).unwrap_err();
    assert!(neither.contains("near") && neither.contains("ref"), "{neither}");
}

#[test]
fn a_copy_button_is_a_reference_and_a_label_is_not_a_button() {
    let copy = CaptureRequest { ref_: Some("e9".into()), copy: true, ..request() };
    assert!(what_to_read(&copy).is_ok());
    let no_ref = what_to_read(&CaptureRequest { copy: true, ..request() }).unwrap_err();
    assert!(no_ref.contains("copy button"), "{no_ref}");
    let by_label = CaptureRequest { near: Some("Key".into()), copy: true, ..request() };
    assert!(what_to_read(&by_label).is_err());
    let both = CaptureRequest { ref_: Some("e9".into()), copy: true, download: true, ..request() };
    assert!(what_to_read(&both).unwrap_err().contains("not both"));
}

#[test]
fn the_shape_the_agent_gave_is_checked_and_a_refusal_never_quotes_the_value() {
    let req = CaptureRequest { ref_: Some("e4".into()), prefix: Some("sk_live_".into()), pattern: Some("[A-Za-z0-9_]{20,40}".into()), ..request() };
    let (_, shape) = what_to_read(&req).unwrap();
    assert!(check_shape(shape.as_ref(), "sk_live_abcdefghijkl1234", "pane-read-test-a").is_ok());
    let err = check_shape(shape.as_ref(), "pk_test_hunter2hunter2hunter2", "pane-read-test-a").unwrap_err();
    assert!(err.contains("nothing was saved") && !err.contains("hunter2") && !err.contains("sk_live_"), "{err}");
    assert!(check_shape(None, "anything", "pane-read-test-a").is_ok(), "no shape, no check");
    let bad = CaptureRequest { ref_: Some("e4".into()), pattern: Some("(".into()), ..request() };
    assert!(what_to_read(&bad).is_err());
}

#[test]
fn a_pane_that_keeps_missing_is_refused_a_shape() {
    let req = CaptureRequest { ref_: Some("e4".into()), prefix: Some("sk_".into()), ..request() };
    let (_, shape) = what_to_read(&req).unwrap();
    let pane = "pane-read-test-probe";
    for _ in 0..5 {
        assert!(super::super::browser_capture_shape::check_not_probing(pane).is_ok());
        assert!(check_shape(shape.as_ref(), "nope", pane).is_err());
    }
    assert!(super::super::browser_capture_shape::check_not_probing(pane).is_err());
}
