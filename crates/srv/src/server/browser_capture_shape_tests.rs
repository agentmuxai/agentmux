// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn no_shape_asked_for_is_no_shape() {
    assert!(Shape::new(None, None).unwrap().is_none());
    assert!(Shape::new(Some("  "), Some("")).unwrap().is_none());
}

#[test]
fn a_value_has_the_prefix_and_the_pattern_whole_or_it_is_refused() {
    let s = Shape::new(Some("sk_live_"), Some("[A-Za-z0-9_]{20,40}")).unwrap().unwrap();
    assert!(s.matches("sk_live_abcdefghijkl1234"));
    assert!(s.matches("  sk_live_abcdefghijkl1234\n"), "trimmed");
    assert!(!s.matches("pk_live_abcdefghijkl1234"), "the prefix");
    assert!(!s.matches("sk_live_ab"), "the pattern: too short");
    assert!(!s.matches("sk_live_abcdefghijkl1234 and more"), "the pattern is the whole value, not a part of it");
    let only_pattern = Shape::new(None, Some("[0-9a-f]{32}")).unwrap().unwrap();
    assert!(only_pattern.matches(&"ab".repeat(16)) && !only_pattern.matches(&"ab".repeat(17)));
    let alternatives = Shape::new(None, Some("sk_a|sk_b")).unwrap().unwrap();
    assert!(alternatives.matches("sk_b") && !alternatives.matches("sk_bx"), "alternatives are grouped, not split around the anchors");
}

#[test]
fn a_shape_that_cannot_be_used_says_what_to_fix_without_a_value() {
    assert!(Shape::new(None, Some("(")).unwrap_err().contains("not a valid regular expression"));
    assert!(Shape::new(Some("two words"), None).unwrap_err().contains("one word"));
    assert!(Shape::new(None, Some(&"a".repeat(201))).unwrap_err().contains("at most 200"));
    assert!(Shape::new(Some(&"a".repeat(65)), None).is_err());
}

#[test]
fn a_pane_that_has_had_five_values_refused_is_refused_a_shape_for_a_while() {
    let pane = "pane-shape-probe-test";
    assert!(check_not_probing(pane).is_ok());
    for _ in 0..MISSES - 1 {
        note_miss(pane);
    }
    assert!(check_not_probing(pane).is_ok(), "four misses are still a choice between values");
    note_miss(pane);
    let err = check_not_probing(pane).unwrap_err();
    assert!(err.contains("not for testing what one says"), "{err}");
    assert!(check_not_probing("another-pane").is_ok(), "per pane");
}
