// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `is_self_registration_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::is_self_registration;

/// reagentx P1 on #2698: without this check, `reactive.registrations`'s
/// `remote` list always included this instance's own shared-registry
/// entry (every agent unconditionally writes itself into that same
/// registry), so the "registered elsewhere too" badge fired on every
/// healthy agent — defeating the whole point of the feature.
#[test]
fn same_local_url_is_self() {
    assert!(is_self_registration(
        "http://127.0.0.1:12345",
        "http://127.0.0.1:12345"
    ));
}

#[test]
fn different_local_url_is_not_self() {
    assert!(!is_self_registration(
        "http://127.0.0.1:12345",
        "http://127.0.0.1:54321"
    ));
}
