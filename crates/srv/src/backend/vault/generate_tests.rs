// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn req(passwordrules: Option<&str>) -> RulesRequest {
    RulesRequest { passwordrules: passwordrules.map(str::to_string), ..Default::default() }
}

#[test]
fn the_default_is_twenty_characters_with_every_class_and_no_look_alikes() {
    let r = rules(&RulesRequest::default()).unwrap();
    assert_eq!(r.length, DEFAULT_LENGTH);
    for _ in 0..200 {
        let v = generate(&r).unwrap();
        assert_eq!(v.chars().count(), DEFAULT_LENGTH);
        for set in [LOWER, UPPER, DIGIT, SPECIAL] {
            assert!(v.chars().any(|c| set.contains(c)), "{set} missing");
        }
        assert!(!v.chars().any(|c| LOOK_ALIKES.contains(c)));
    }
}

#[test]
fn a_sites_passwordrules_replace_the_default_classes() {
    let r = rules(&req(Some("minlength: 24; maxlength: 30; required: lower; required: digit; allowed: [-_]"))).unwrap();
    assert_eq!(r.length, 24, "the default length is raised to the site's minimum");
    for _ in 0..100 {
        let v = generate(&r).unwrap();
        assert!(v.chars().all(|c| LOWER.contains(c) || DIGIT.contains(c) || "-_".contains(c)), "{} has a character the site doesn't allow", v.as_str());
        assert!(v.chars().any(|c| LOWER.contains(c)) && v.chars().any(|c| DIGIT.contains(c)));
    }
}

#[test]
fn a_required_list_needs_one_character_from_any_of_its_classes() {
    let r = rules(&req(Some("required: upper, digit; allowed: lower; minlength: 22"))).unwrap();
    assert_eq!(r.required.len(), 1);
    for _ in 0..100 {
        let v = generate(&r).unwrap();
        assert!(v.chars().any(|c| UPPER.contains(c) || DIGIT.contains(c)));
    }
}

#[test]
fn max_consecutive_is_kept() {
    let r = rules(&req(Some("allowed: lower; minlength: 20; max-consecutive: 1"))).unwrap();
    for _ in 0..100 {
        let v: Vec<char> = generate(&r).unwrap().chars().collect();
        assert!(v.windows(2).all(|w| w[0] != w[1]));
    }
}

#[test]
fn a_fields_maxlength_caps_the_length() {
    let r = rules(&RulesRequest { maxlength: Some(16), ..Default::default() }).unwrap();
    assert_eq!(r.length, 16);
    assert!(bits(&r) >= MIN_BITS);
}

#[test]
fn rules_that_leave_too_few_bits_are_refused_and_say_why() {
    let e = rules(&req(Some("allowed: digit; maxlength: 12"))).unwrap_err();
    assert!(e.contains("bits"), "{e}");
    let e = rules(&RulesRequest { maxlength: Some(8), ..Default::default() }).unwrap_err();
    assert!(e.contains("bits"), "{e}");
}

#[test]
fn contradictory_lengths_are_refused() {
    assert!(rules(&req(Some("minlength: 40; maxlength: 20"))).is_err());
}

#[test]
fn malformed_rules_are_refused_without_quoting_a_value() {
    assert!(rules(&req(Some("minlength: lots"))).is_err());
    assert!(rules(&req(Some("required: [0O1lI]"))).is_err(), "a set of only look-alikes holds nothing usable");
    assert!(rules(&req(Some("nonsense"))).is_err());
}

#[test]
fn unknown_rules_are_ignored_as_browsers_do() {
    assert!(rules(&req(Some("minlength: 20; frobnicate: 3"))).is_ok());
}

#[test]
fn words_are_from_the_list_and_joined_by_hyphens() {
    let r = rules(&RulesRequest { words: Some(6), ..Default::default() }).unwrap();
    let v = generate(&r).unwrap();
    let words: Vec<&str> = v.split('-').collect();
    assert_eq!(words.len(), 6);
    assert!(words.iter().all(|w| super::super::wordlist::WORDS.contains(w)));
    assert!(rules(&RulesRequest { words: Some(3), ..Default::default() }).is_err());
}

#[test]
fn words_that_a_fields_maxlength_would_cut_are_refused() {
    let short = rules(&RulesRequest { words: Some(6), maxlength: Some(16), ..Default::default() }).unwrap_err();
    assert!(short.contains("maxlength of 16"), "{short}");
    assert!(rules(&RulesRequest { words: Some(6), maxlength: Some(128), ..Default::default() }).is_ok());
    let with_rules = RulesRequest { words: Some(6), passwordrules: Some("maxlength: 16".into()), ..Default::default() };
    assert!(rules(&with_rules).is_err(), "a site's rules don't apply to words, so the two aren't combined");
}

#[test]
fn every_character_is_about_equally_likely() {
    // Chi-square over 100,000 characters from a 29-character alphabet: with no bias the
    // statistic stays well under 80 (28 degrees of freedom; p < 1e-6 above it).
    let r = rules(&req(Some("allowed: lower, [2345]; minlength: 20"))).unwrap();
    let alphabet: Vec<char> = r.allowed.chars().collect();
    let mut counts = vec![0usize; alphabet.len()];
    let mut total = 0;
    while total < 100_000 {
        for c in generate(&r).unwrap().chars() {
            counts[alphabet.iter().position(|a| *a == c).unwrap()] += 1;
            total += 1;
        }
    }
    let expected = total as f64 / alphabet.len() as f64;
    let chi: f64 = counts.iter().map(|&n| (n as f64 - expected).powi(2) / expected).sum();
    assert!(chi < 80.0, "chi-square {chi:.1} over {} characters", alphabet.len());
}

#[test]
fn two_values_differ() {
    let r = Rules::default();
    assert_ne!(generate(&r).unwrap().as_str(), generate(&r).unwrap().as_str());
}
