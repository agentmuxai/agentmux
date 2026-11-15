// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn items_of(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("- {prefix} item {i:03} with some evidence attached to it\n")).collect()
}

/// A summary in the order the summarizer writes it: the Done lists first, the
/// identifiers last, so a cut from the end loses the identifiers first.
fn summary(done: usize) -> String {
    format!(
        "## Current goal\nShip the fix\n## Last user request\n\"merge it\" IN PROGRESS\n\
         ## Done (verified)\n{}## Done (unverified)\n{}## In flight when last seen\n- PR #7368 waiting for CI\n\
         ## Waiting on the user\n- nothing\n## Decisions and why\n{}## User preferences and standing instructions\n- no acks\n\
         ## Exact identifiers\n- agentmuxai/agentmux#7368\n- crates/srv/src/backend/continuity.rs\n",
        items_of("verified", done),
        items_of("unverified", done),
        items_of("decision", 3),
    )
}

#[test]
fn a_summary_within_the_cap_is_unchanged() {
    let s = summary(2);
    let (out, dropped) = shorten(&s, s.chars().count());
    assert_eq!(out, s);
    assert_eq!(dropped, 0);
}

#[test]
fn a_long_summary_keeps_what_is_in_flight_and_the_identifiers_whole() {
    let s = summary(120);
    let cap = 3_000;
    assert!(s.chars().count() > cap * 3);
    let (out, dropped) = shorten(&s, cap);
    assert!(dropped > 0);
    assert!(out.chars().count() <= cap, "{} > {cap}", out.chars().count());
    for kept in [
        "- PR #7368 waiting for CI",
        "\"merge it\" IN PROGRESS",
        "Ship the fix",
        "- nothing",
        "- no acks",
        "- agentmuxai/agentmux#7368",
        "- crates/srv/src/backend/continuity.rs",
    ] {
        assert!(out.contains(kept), "lost {kept:?}:\n{out}");
    }
}

#[test]
fn the_unverified_list_goes_before_the_verified_one_and_the_newest_items_stay() {
    let s = summary(40);
    // Room for the unverified list to be emptied but not the verified one.
    let (out, _) = shorten(&s, s.chars().count() - 1_200);
    let (verified, unverified) = (out.find("## Done (verified)").unwrap(), out.find("## Done (unverified)").unwrap());
    let verified_body = &out[verified..unverified];
    assert!(verified_body.contains("verified item 000"), "the verified list was cut first:\n{out}");
    let unverified_body = &out[unverified..out.find("## In flight").unwrap()];
    assert!(unverified_body.contains("unverified item 039"), "the newest was dropped:\n{out}");
    assert!(!unverified_body.contains("unverified item 000"), "the oldest was kept:\n{out}");
    assert!(unverified_body.contains("older items omitted)"), "{out}");
}

#[test]
fn the_notice_counts_every_item_a_section_lost() {
    let s = summary(60);
    let (out, dropped) = shorten(&s, 2_500);
    let noted: usize = out.lines().filter_map(omitted_count).sum();
    assert_eq!(noted, dropped, "{out}");
}

#[test]
fn shortening_again_adds_to_the_earlier_count() {
    let s = summary(80);
    let (once, first) = shorten(&s, 4_000);
    let (twice, second) = shorten(&once, 2_500);
    let noted: usize = twice.lines().filter_map(omitted_count).sum();
    assert_eq!(noted, first + second, "{twice}");
}

#[test]
fn protected_sections_over_the_cap_are_kept_whole_rather_than_cut() {
    let big = items_of("identifier", 200);
    let s = format!("## Current goal\ng\n## Last user request\nr\n## Done (verified)\n{}## Exact identifiers\n{big}", items_of("v", 30));
    let (out, _) = shorten(&s, 500);
    assert!(out.chars().count() > 500, "the cap was kept by cutting a protected section");
    assert!(out.trim_end().ends_with(big.trim_end()), "the identifiers were cut");
    assert!(!out.contains("v item 000"));
}

#[test]
fn a_wrapped_item_goes_and_stays_with_its_first_line() {
    let s = "## Last user request\nr\n## Done (verified)\n- old one\n  wrapped detail of old\n- new one\n  wrapped detail of new\n";
    let (out, dropped) = shorten(s, 100);
    assert_eq!(dropped, 1);
    assert!(out.contains("- new one\n  wrapped detail of new"));
    assert!(!out.contains("wrapped detail of old"), "{out}");
}

#[test]
fn a_numbered_list_is_cut_by_item() {
    let s = "## Last user request
r
## Done (unverified)
1. first item, with some text
2. second item, with some text
3. third item, with some text
";
    let (out, dropped) = shorten(s, s.chars().count() - 30);
    assert!(dropped >= 1);
    assert!(out.contains("3. third item"), "{out}");
    assert!(!out.contains("1. first item"), "{out}");
}

#[test]
fn the_packet_order_puts_where_the_work_stands_first_and_the_done_lists_last() {
    let out = in_packet_order(&summary(2));
    let at = |h: &str| out.find(h).unwrap_or_else(|| panic!("missing {h}"));
    let order = [
        "## Current goal",
        "## Last user request",
        "## In flight",
        "## Waiting on the user",
        "## Exact identifiers",
        "## Decisions and why",
        "## User preferences",
        "## Done (unverified)",
        "## Done (verified)",
    ];
    for pair in order.windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{} should come before {}\n{out}", pair[0], pair[1]);
    }
    assert_eq!(out.chars().filter(|c| *c == '\n').count() + 1, summary(2).trim_end().lines().count());
}

#[test]
fn a_section_the_order_does_not_name_comes_after_the_named_ones_in_written_order() {
    let s = "## Zeta notes\nz\n## Done (verified)\n- d\n## Current goal\ng\n## Alpha notes\na\n";
    let out = in_packet_order(s);
    let at = |h: &str| out.find(h).unwrap();
    assert!(at("## Current goal") < at("## Done (verified)"));
    assert!(at("## Done (verified)") < at("## Zeta notes"));
    assert!(at("## Zeta notes") < at("## Alpha notes"));
}

#[test]
fn text_without_sections_is_left_alone() {
    assert_eq!(in_packet_order("just a line\nand another"), "just a line\nand another");
    let (out, dropped) = shorten("just a line", 3);
    assert_eq!((out.as_str(), dropped), ("just a line", 0));
}
