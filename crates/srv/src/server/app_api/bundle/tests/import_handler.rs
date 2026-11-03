// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bundle.import`, called as `bundle_import(&state, req)` without the engine.

use super::super::*;
use crate::server::tests::test_state;

fn manifest(name: &str, components: serde_json::Value) -> String {
    serde_json::to_string(&serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
        "name": name,
        "version": "0.1.0",
        "description": "A test bundle",
        "components": components,
        "metadata": {},
    }))
    .unwrap()
}

fn entry(path: &str, content: &str) -> FileEntry {
    FileEntry { path: path.to_string(), content: content.to_string() }
}

fn files_req(files: Vec<FileEntry>) -> BundleImportReq {
    BundleImportReq { zip_base64: None, files: Some(files) }
}

fn bundle_with_skill(bundle: &str, skill: &str) -> Vec<FileEntry> {
    vec![
        entry(
            "armory.json",
            &manifest(bundle, serde_json::json!({ "instructions": ["instructions/AGENTS.md"], "skills": [format!("skills/{skill}")] })),
        ),
        entry("instructions/AGENTS.md", "Be concise."),
        entry(
            &format!("skills/{skill}/SKILL.md"),
            &crate::backend::agent_config::render_skill_md(skill, "Runs the checklist", "1. Test\n2. Deploy"),
        ),
    ]
}

fn deploy_bundle() -> Vec<FileEntry> {
    bundle_with_skill("direct-bundle", "deploy")
}

/// Exactly one of `zip_base64` / `files` is accepted, and a bad archive is named.
#[tokio::test]
async fn bundle_import_wants_exactly_one_intake_shape() {
    let state = test_state();
    let both = BundleImportReq { zip_base64: Some(String::new()), files: Some(vec![]) };
    assert_eq!(
        bundle_import(&state, both).unwrap_err(),
        "bundle.import: exactly one of zip_base64/files must be present, got both"
    );
    assert_eq!(
        bundle_import(&state, BundleImportReq::default()).unwrap_err(),
        "bundle.import: exactly one of zip_base64/files must be present, got neither"
    );
    let bad = BundleImportReq { zip_base64: Some("not base64!".to_string()), files: None };
    assert!(bundle_import(&state, bad).unwrap_err().starts_with("bundle.import: invalid zip_base64"));
}

/// A raw `files` list becomes a bundle row (never global) bound to a new global skill.
#[tokio::test]
async fn bundle_import_creates_the_bundle_and_its_skill() {
    let state = test_state();
    let done = bundle_import(&state, files_req(deploy_bundle())).unwrap();

    assert_eq!(done.imported_skill_ids.len(), 1);
    assert!(done.skipped_skills.is_empty());
    let skill = state.identity_store.skill_get(&done.imported_skill_ids[0]).unwrap().unwrap();
    assert_eq!(skill.name, "deploy");
    assert!(skill.is_global);

    let bundle = state.id_store.bundle_get(&done.bundle_id).unwrap().unwrap();
    assert_eq!(bundle.name, "direct-bundle");
    assert!(!bundle.is_global, "an import must not start injecting into every agent");
    assert_eq!(bundle.skills, serde_json::to_string(&done.imported_skill_ids).unwrap());
}

/// A second bundle carrying a skill that already exists skips that skill, with
/// a warning, and still makes the bundle.
#[tokio::test]
async fn bundle_import_skips_a_skill_that_already_exists() {
    let state = test_state();
    bundle_import(&state, files_req(deploy_bundle())).unwrap();
    let again = bundle_import(&state, files_req(bundle_with_skill("second-bundle", "deploy"))).unwrap();

    assert!(again.imported_skill_ids.is_empty());
    assert_eq!(again.skipped_skills, vec!["deploy".to_string()]);
    assert!(again.warnings.iter().any(|w| w.contains("already exists")), "{:?}", again.warnings);
    assert!(state.id_store.bundle_get(&again.bundle_id).unwrap().is_some());
}

/// A file list with no manifest fails before anything is written.
#[tokio::test]
async fn bundle_import_writes_nothing_when_the_files_do_not_parse() {
    let state = test_state();
    let before = state.id_store.bundle_list().unwrap().len();
    let err = bundle_import(&state, files_req(vec![entry("README.md", "no manifest")])).unwrap_err();
    assert!(err.starts_with("bundle.import:"), "{err}");
    assert_eq!(state.id_store.bundle_list().unwrap().len(), before);
}

/// When the bundle row cannot be written (its name is taken), the skills this
/// import just created are deleted again rather than left as orphans.
#[tokio::test]
async fn bundle_import_rolls_back_its_skills_when_the_bundle_cannot_be_saved() {
    let state = test_state();
    bundle_import(&state, files_req(deploy_bundle())).unwrap();
    let err = bundle_import(&state, files_req(bundle_with_skill("direct-bundle", "lint"))).unwrap_err();

    assert!(err.starts_with("bundle.import:"), "{err}");
    assert!(!err.contains("rollback"), "the rollback itself succeeded: {err}");
    let globals = state.mstore.skill_list_global(&state.identity_store).unwrap();
    assert!(globals.iter().all(|s| s.skill.name != "lint"), "the lint skill must not survive a failed import");
}
