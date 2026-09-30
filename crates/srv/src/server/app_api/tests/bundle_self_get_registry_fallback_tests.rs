// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `bundle_self_get_registry_fallback_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;
use crate::server::tests::test_state;
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

#[tokio::test]
async fn falls_back_to_the_registrys_own_bound_bundle_when_no_local_instance_row_exists() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let state = test_state();
    let bundle: crate::backend::storage::bundles::Bundle =
        serde_json::from_value(serde_json::json!({
            "id": "bundle-agenty-test",
            "name": "AgentY's real bundle",
        }))
        .unwrap();
    state.id_store.bundle_upsert(&bundle).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let prev = std::env::var_os("AGENTMUX_HOME_OVERRIDE");
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());

    let registry_dir = tmp.path().join("shared").join("agents").join("registry");
    let registry = crate::registry::Registry::open(registry_dir).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-agenty".to_string(),
                instance_name: "AgentY".to_string(),
                definition_id: "def-agenty".to_string(),
                identity_id: None,
                memory_id: Some("bundle-agenty-test".to_string()),
                session_id: None,
                working_dir: "agenty-0629j".to_string(),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    // No `db_agents` row for "agenty" exists in `state.mstore` — this
    // must resolve entirely through the registry fallback.
    let resp = bundle_self_get_impl(&state, "agenty").await;

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_HOME_OVERRIDE", v),
        None => std::env::remove_var("AGENTMUX_HOME_OVERRIDE"),
    }

    let resp = resp.expect("bundle.self.get must succeed via the registry fallback");
    assert_eq!(resp["id"], "bundle-agenty-test");
    assert_eq!(resp["is_blank"], false);
}
