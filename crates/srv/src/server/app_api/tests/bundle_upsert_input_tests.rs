// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `bundle_upsert_input_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::bundle::normalize_bundle_upsert_input;
use serde_json::json;

// The frontend types `bundle.validate` as `BundleValidateInput` (id
// OPTIONAL) but `upsertmemory`/`upsertsystemmemory` as
// `BundleUpsertInput` (id REQUIRED). That asymmetry is not a style choice,
// and reagent flagged a version of this PR that got it wrong -- it falls
// straight out of which handlers normalize their payload first:
//
//   * bundle_validate_impl runs normalize_bundle_upsert_input, which fills
//     a missing id with "", so an unsaved draft validates.
//   * upsertmemory / upsertsystemmemory deserialize Bundle straight from
//     the payload, and Bundle::id has no serde default.
//
// If either half of this ever changes, the frontend types are wrong, so
// pin both halves here.
#[test]
fn validate_accepts_a_draft_with_no_id_but_upsert_does_not() {
    use crate::backend::storage::store::Bundle;

    let draft = json!({ "name": "unsaved draft" });

    let validated: Bundle =
        serde_json::from_value(normalize_bundle_upsert_input(draft.clone()))
            .expect("bundle.validate must accept a draft with no id");
    assert!(validated.id.is_empty(), "a draft id normalizes to the empty string");

    assert!(
        serde_json::from_value::<Bundle>(draft).is_err(),
        "upsertmemory does NOT normalize, so the same payload must be rejected there"
    );
}

#[test]
fn fills_missing_or_null_id_with_empty_string() {
    let no_id = normalize_bundle_upsert_input(json!({ "name": "p" }));
    assert_eq!(no_id["id"], json!(""));

    let null_id = normalize_bundle_upsert_input(json!({ "id": null, "name": "p" }));
    assert_eq!(null_id["id"], json!(""));

    // A real id is preserved untouched.
    let kept = normalize_bundle_upsert_input(json!({ "id": "abc", "name": "p" }));
    assert_eq!(kept["id"], json!("abc"));
}

#[test]
fn encodes_array_fields_to_json_strings() {
    let out = normalize_bundle_upsert_input(json!({
        "name": "p",
        "context_files": [{ "path": "a.md", "content": "x" }],
        "mcp_servers": [],
        "skills": ["s1", "s2"],
    }));
    // serde_json::Value maps serialize keys in sorted order.
    assert_eq!(out["context_files"], json!("[{\"content\":\"x\",\"path\":\"a.md\"}]"));
    assert_eq!(out["mcp_servers"], json!("[]"));
    assert_eq!(out["skills"], json!("[\"s1\",\"s2\"]"));
}

#[test]
fn leaves_string_fields_untouched() {
    let out = normalize_bundle_upsert_input(json!({
        "name": "p",
        "context_files": "[]",
        "skills": "[\"already\"]",
    }));
    assert_eq!(out["context_files"], json!("[]"));
    assert_eq!(out["skills"], json!("[\"already\"]"));
}
