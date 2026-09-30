// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `agent_zoom_seed_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::parse_seed_zoom;

#[test]
fn seeds_valid_non_default_zoom() {
    assert_eq!(parse_seed_zoom("1.3"), Some(1.3));
    assert_eq!(parse_seed_zoom("0.5"), Some(0.5));
    assert_eq!(parse_seed_zoom("2"), Some(2.0));
    assert_eq!(parse_seed_zoom("  1.4  "), Some(1.4)); // trims
}

#[test]
fn rejects_default_out_of_range_and_garbage() {
    assert_eq!(parse_seed_zoom("1.0"), None, "default seeds nothing");
    assert_eq!(parse_seed_zoom("1"), None, "default seeds nothing");
    assert_eq!(parse_seed_zoom("2.5"), None, "above range");
    assert_eq!(parse_seed_zoom("0.4"), None, "below range");
    assert_eq!(parse_seed_zoom("abc"), None, "unparseable");
    assert_eq!(parse_seed_zoom(""), None, "empty");
}
