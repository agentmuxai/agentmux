// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `bundle.import.commit` writes into the bundle row and reports back, beyond the selection
//! rules `import_preview_commit.rs` covers.

use super::super::*;
use crate::backend::bundle_import as bi;
use crate::server::tests::test_state;

fn entry(path: &str, content: &str) -> FileEntry {
    FileEntry { path: path.to_string(), content: content.to_string() }
}

fn manifest_with(extra: serde_json::Value) -> String {
    let mut m = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
        "name": "commit-bundle",
        "version": "0.1.0",
        "description": "Commit test",
        "components": {},
    });
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    serde_json::to_string(&m).unwrap()
}

fn commit_req(files: Vec<FileEntry>, include_instructions: bool) -> CommitReq {
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    CommitReq {
        include_widgets: vec![],
        file_path: None,
        zip_base64: None,
        expected_content_digest: bi::content_digest_files(&bi_files),
        files: Some(files),
        bundle_name: None,
        include_instructions,
        include_context_files: vec![],
        include_skills: vec![],
    }
}

async fn commit(state: &AppState, req: CommitReq) -> Result<crate::backend::rpc_types::BundleImportCommitResponse, String> {
    bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await
}

fn instruction_files() -> Vec<FileEntry> {
    vec![
        entry(
            "armory.json",
            &manifest_with(serde_json::json!({
                "provider": "claude",
                "model": "opus",
                "components": { "instructions": { "default": ["instructions/AGENTS.md"], "claude": ["instructions/CLAUDE.md"] } },
            })),
        ),
        entry("instructions/AGENTS.md", "Be concise."),
        entry("instructions/CLAUDE.md", "Use the tools."),
    ]
}

#[tokio::test]
async fn commit_carries_instructions_and_provider_variants_when_selected() {
    let state = test_state();
    let resp = commit(&state, commit_req(instruction_files(), true)).await.unwrap();
    let saved = state.id_store.bundle_get(&resp.bundle_id).unwrap().unwrap();
    assert_eq!(saved.instructions, "Be concise.");
    let by_provider: std::collections::HashMap<String, String> = serde_json::from_str(&saved.instructions_by_provider).unwrap();
    assert_eq!(by_provider.get("claude").map(String::as_str), Some("Use the tools."));
}

#[tokio::test]
async fn commit_leaves_instructions_out_when_not_selected_but_keeps_provider_and_model() {
    let state = test_state();
    let resp = commit(&state, commit_req(instruction_files(), false)).await.unwrap();
    let saved = state.id_store.bundle_get(&resp.bundle_id).unwrap().unwrap();
    assert_eq!(saved.instructions, "");
    assert_eq!(saved.instructions_by_provider, "{}");
    assert_eq!(saved.provider, "claude");
    assert_eq!(saved.model, "opus");
    assert!(!saved.is_global && !saved.is_system && !saved.is_blank);
    assert_eq!(saved.mcp_servers, "[]");
}

#[tokio::test]
async fn commit_response_starts_with_no_widgets_and_a_bounded_clean_warning_list() {
    let state = test_state();
    let resp = commit(&state, commit_req(instruction_files(), true)).await.unwrap();
    assert!(resp.widgets.is_empty());
    assert!(resp.imported_skill_ids.is_empty());
    assert!(resp.skipped_skills.is_empty());
    assert!(!resp.warnings_truncated);
}

#[tokio::test]
async fn commit_splits_requirements_into_resolved_and_unresolved_by_matching_accounts() {
    let state = test_state();
    state.id_store.identity_upsert(&crate::identity::oauth_tokens::tests::oauth_account("acc-have", "have-provider", "cid")).unwrap();
    let requirements = serde_json::json!({ "requirements": [
        { "id": "r-have", "credentialProvider": "have-provider", "env": "HAVE_TOKEN" },
        { "id": "r-miss", "credentialProvider": "missing-provider", "env": "MISS_TOKEN" },
    ]});
    let files = vec![
        entry("armory.json", &manifest_with(serde_json::json!({ "components": { "accounts": "accounts/requirements.json" } }))),
        entry("accounts/requirements.json", &requirements.to_string()),
    ];
    let resp = commit(&state, commit_req(files, false)).await.unwrap();
    assert_eq!(resp.resolved_requirement_ids, vec!["r-have".to_string()]);
    assert_eq!(resp.unresolved_requirements.len(), 1);
    let missing = &resp.unresolved_requirements[0];
    assert_eq!((missing.id.as_str(), missing.provider.as_str(), missing.env.as_str()), ("r-miss", "missing-provider", "MISS_TOKEN"));
    assert_eq!(missing.match_count, 0);
}

#[tokio::test]
async fn commit_with_an_unparseable_bundle_writes_nothing() {
    let state = test_state();
    let files = vec![entry("armory.json", "{ not json")];
    let before = state.id_store.bundle_list().unwrap().len();
    let err = commit(&state, commit_req(files, false)).await.unwrap_err();
    assert!(err.starts_with("bundle.import.commit: "), "{err}");
    assert_eq!(state.id_store.bundle_list().unwrap().len(), before);
}
