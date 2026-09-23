// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::*;
use crate::backend::bundle_import as bi;
use crate::server::tests::test_state;

fn manifest(components: serde_json::Value) -> String {
    serde_json::to_string(&serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
        "name": "test-bundle",
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

fn skill_md(name: &str, description: &str, body: &str) -> String {
    crate::backend::agent_config::render_skill_md(name, description, body)
}

#[tokio::test]
async fn preview_returns_parsed_bundle_with_digest() {
    let state = test_state();
    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md", "instructions/context/notes.md"],
            "skills": ["skills/deploy"],
        }))),
        entry("instructions/AGENTS.md", "Be concise."),
        entry("instructions/context/notes.md", "Extra context."),
        entry("skills/deploy/SKILL.md", &skill_md("deploy", "Runs the checklist", "1. Test\n2. Deploy")),
    ];
    let req = PreviewReq { file_path: None, zip_base64: None, files: Some(files) };
    let resp = bundle_import_preview_impl(&state.id_store, &state.identity_store, &state.mstore, req).await.unwrap();

    assert_eq!(resp.name, "test-bundle");
    assert_eq!(resp.instructions_preview, "Be concise.");
    assert_eq!(resp.context_files[0].id, 0);
    assert_eq!(resp.context_files[0].display_path, "notes.md");
    assert_eq!(resp.skills[0].source_dir, "skills/deploy");
    assert_eq!(resp.skills[0].slug, "deploy");
    assert_eq!(resp.skills[0].collision, "none");
    assert!(resp.content_digest.len() > 0);
    assert_eq!(resp.name_collision, false);
}

#[tokio::test]
async fn preview_flags_name_conflict_against_existing_global_skill() {
    let state = test_state();
    // Seed an existing global skill named "deploy".
    state
        .identity_store
        .skill_upsert_unique_global(&crate::backend::storage::Skill {
            id: "existing-1".to_string(),
            name: "deploy".to_string(),
            trigger: "deploy".to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: "pre-existing".to_string(),
            content: "pre-existing body".to_string(),
            is_global: true,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
        entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
    ];
    let req = PreviewReq { file_path: None, zip_base64: None, files: Some(files) };
    let resp = bundle_import_preview_impl(&state.id_store, &state.identity_store, &state.mstore, req).await.unwrap();
    assert_eq!(resp.skills[0].collision, "name_conflict");
}

#[tokio::test]
async fn preview_flags_duplicate_in_bundle_when_two_parsed_skills_share_a_slug() {
    // Phase 3 spec §3.1, codex P1 round 2: two skills within the same
    // bundle sharing a slug that ISN'T yet global must both be flagged,
    // not silently passed as "none".
    let state = test_state();
    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({
            "skills": ["skills/code-review-v2", "skills/code-review-old"],
        }))),
        entry("skills/code-review-v2/SKILL.md", &skill_md("code-review", "new", "body-new")),
        entry("skills/code-review-old/SKILL.md", &skill_md("code-review", "old", "body-old")),
    ];
    let req = PreviewReq { file_path: None, zip_base64: None, files: Some(files) };
    let resp = bundle_import_preview_impl(&state.id_store, &state.identity_store, &state.mstore, req).await.unwrap();
    let skills = resp.skills;
    assert_eq!(skills.len(), 2);
    assert!(skills.iter().all(|s| s.collision == "duplicate_in_bundle"));
}

#[tokio::test]
async fn commit_rejects_on_digest_mismatch_and_writes_nothing() {
    let state = test_state();
    let files = vec![entry("armory.json", &manifest(serde_json::json!({})))];
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: "not-the-real-digest".to_string(),
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![],
        include_mcp_servers: vec![],
    };
    let err = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req)
        .await
        .unwrap_err();
    assert!(err.contains("digest mismatch"));
    assert!(state.id_store.bundle_list().unwrap().iter().all(|b| b.name != "test-bundle"));
}

#[tokio::test]
async fn commit_applies_bundle_name_override_not_parsed_name() {
    // codex P2, PR #2381 round 11: bundle_name must actually be
    // substituted for Bundle.name, never silently ignored.
    let state = test_state();
    let files = vec![entry("armory.json", &manifest(serde_json::json!({})))];
    let digest = bi::content_digest_files(&files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect::<Vec<_>>());
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: Some("Renamed Bundle".to_string()),
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![],
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    let bundle_id = resp.bundle_id;
    let saved = state.id_store.bundle_get(&bundle_id).unwrap().unwrap();
    assert_eq!(saved.name, "Renamed Bundle");
}

#[tokio::test]
async fn commit_bounds_an_oversized_bundle_name_override() {
    // reagentx P2, PR #2382 round 3: unlike parsed.name (bounded at
    // parse time), req.bundle_name had no length cap of its own before
    // being used verbatim as Bundle.name.
    let state = test_state();
    let files = vec![entry("armory.json", &manifest(serde_json::json!({})))];
    let digest = bi::content_digest_files(&files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect::<Vec<_>>());
    let oversized_name = "n".repeat(bi::MAX_BUNDLE_NAME_CHARS + 500);
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: Some(oversized_name),
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![],
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    let bundle_id = resp.bundle_id;
    let saved = state.id_store.bundle_get(&bundle_id).unwrap().unwrap();
    assert_eq!(saved.name.chars().count(), bi::MAX_BUNDLE_NAME_CHARS);
}

#[tokio::test]
async fn commit_dedupes_repeated_source_dirs_in_include_skills_first_occurrence_wins() {
    // reagentx P1, PR #2382 round 3: a client repeating the same
    // source_dir with a different import_as each time must not drive
    // one Store write per repetition.
    let state = test_state();
    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
        entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);
    let include_skills: Vec<SkillSelection> = (0..50)
        .map(|i| SkillSelection { source_dir: "skills/deploy".to_string(), import_as: Some(format!("deploy-{i}")) })
        .collect();
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills,
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    let imported = resp.imported_skill_ids;
    assert_eq!(imported.len(), 1, "expected only the first occurrence of the repeated source_dir to be written");
    let saved_skill = state.identity_store.skill_get(&imported[0]).unwrap().unwrap();
    assert_eq!(saved_skill.name, "deploy-0");
}

#[tokio::test]
async fn commit_caps_include_skills_at_max_imported_skills() {
    // reagentx P1, PR #2382 round 3: an include_skills array longer
    // than MAX_IMPORTED_SKILLS must not drive more than that many
    // skill_upsert_unique_global write attempts. Distinct-but-bogus
    // source_dirs (each a cheap no-op "continue") occupy the first
    // MAX_IMPORTED_SKILLS positions; three genuinely resolvable
    // selections are placed AFTER that boundary. If the cap truncates
    // the selection list itself (not just deduping), those three are
    // silently dropped -- proving the cap applies before resolution,
    // not just as an incidental side effect of the dedup fix.
    let state = test_state();
    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({
            "skills": ["skills/a", "skills/b", "skills/c"],
        }))),
        entry("skills/a/SKILL.md", &skill_md("a", "d", "body")),
        entry("skills/b/SKILL.md", &skill_md("b", "d", "body")),
        entry("skills/c/SKILL.md", &skill_md("c", "d", "body")),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);

    let mut include_skills: Vec<SkillSelection> = (0..bi::MAX_IMPORTED_SKILLS)
        .map(|i| SkillSelection { source_dir: format!("skills/nonexistent-{i}"), import_as: None })
        .collect();
    include_skills.push(SkillSelection { source_dir: "skills/a".to_string(), import_as: None });
    include_skills.push(SkillSelection { source_dir: "skills/b".to_string(), import_as: None });
    include_skills.push(SkillSelection { source_dir: "skills/c".to_string(), import_as: None });

    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills,
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    assert!(
        resp.imported_skill_ids.is_empty(),
        "the three real selections beyond the MAX_IMPORTED_SKILLS boundary must be dropped by the cap, not imported"
    );
}

#[tokio::test]
async fn commit_selects_context_files_by_id_not_display_path() {
    let state = test_state();
    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({
            "instructions": ["instructions/context/a.md", "instructions/context/b.md"],
        }))),
        entry("instructions/context/a.md", "content A"),
        entry("instructions/context/b.md", "content B"),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);
    // Only select id 1 (b.md) -- verify a.md (id 0) is excluded.
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![1],
        include_skills: vec![],
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    let bundle_id = resp.bundle_id;
    let saved = state.id_store.bundle_get(&bundle_id).unwrap().unwrap();
    assert!(saved.context_files.contains("content B"));
    assert!(!saved.context_files.contains("content A"));
}

#[tokio::test]
async fn commit_skips_colliding_skill_left_with_an_empty_rename() {
    // §4.1 point 4: never silently sent through under its original,
    // known-conflicting slug.
    let state = test_state();
    state
        .identity_store
        .skill_upsert_unique_global(&crate::backend::storage::Skill {
            id: "existing-1".to_string(),
            name: "deploy".to_string(),
            trigger: "deploy".to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: "pre-existing".to_string(),
            content: "pre-existing body".to_string(),
            is_global: true,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
        entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![SkillSelection { source_dir: "skills/deploy".to_string(), import_as: None }],
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    assert!(resp.imported_skill_ids.is_empty());
    assert_eq!(resp.skipped_skills[0], "deploy");
}

#[tokio::test]
async fn commit_imports_colliding_skill_under_a_non_empty_rename() {
    let state = test_state();
    state
        .identity_store
        .skill_upsert_unique_global(&crate::backend::storage::Skill {
            id: "existing-1".to_string(),
            name: "deploy".to_string(),
            trigger: "deploy".to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: "pre-existing".to_string(),
            content: "pre-existing body".to_string(),
            is_global: true,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
        entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![SkillSelection {
            source_dir: "skills/deploy".to_string(),
            import_as: Some("deploy-team-x".to_string()),
        }],
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    assert_eq!(resp.imported_skill_ids.len(), 1);
    let imported_id = &resp.imported_skill_ids[0];
    let saved_skill = state.identity_store.skill_get(&imported_id).unwrap().unwrap();
    assert_eq!(saved_skill.name, "deploy-team-x");
}

#[tokio::test]
async fn commit_bounds_an_oversized_import_as_in_the_already_exists_warning() {
    // codex P2, PR #2382 round 2: effective_slug (from caller-supplied
    // import_as) has no length bound before reaching the "already
    // exists" warning push -- unlike a parsed skill.slug, which is at
    // least implicitly bounded by the per-entry decompression cap.
    // StoreError::Other's own message text ALSO embeds the identical
    // unbounded value a second time.
    let state = test_state();
    let oversized_name = "n".repeat(bi::MAX_DISPLAY_FIELD_CHARS + 500);
    state
        .identity_store
        .skill_upsert_unique_global(&crate::backend::storage::Skill {
            id: "existing-1".to_string(),
            name: "deploy".to_string(),
            trigger: "deploy".to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: "pre-existing".to_string(),
            content: "pre-existing body".to_string(),
            is_global: true,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();
    state
        .identity_store
        .skill_upsert_unique_global(&crate::backend::storage::Skill {
            id: "existing-2".to_string(),
            name: oversized_name.clone(),
            trigger: oversized_name.clone(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: "pre-existing".to_string(),
            content: "pre-existing body".to_string(),
            is_global: true,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({ "skills": ["skills/deploy"] }))),
        entry("skills/deploy/SKILL.md", &skill_md("deploy", "d", "body")),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![SkillSelection {
            source_dir: "skills/deploy".to_string(),
            import_as: Some(oversized_name),
        }],
        include_mcp_servers: vec![],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    assert!(resp.imported_skill_ids.is_empty());
    let warnings = resp.warnings;
    assert!(!warnings.is_empty(), "expected an already-exists warning");
    for w in warnings {
        let s = w.as_str();
        assert!(
            s.chars().count() <= bi::MAX_DISPLAY_FIELD_CHARS + 3 + 40,
            "warning not bounded ({} chars): {s:?}",
            s.chars().count()
        );
    }
    let skipped = &resp.skipped_skills[0];
    assert!(skipped.chars().count() <= bi::MAX_DISPLAY_FIELD_CHARS + 3);
}

#[tokio::test]
async fn commit_persists_raw_mcp_config_not_the_source_path_wrapper() {
    // Phase 3 spec §3.0, round 2: every write site touching
    // parsed.mcp_servers must project to .config before serializing.
    let state = test_state();
    let files = vec![
        entry("armory.json", &manifest(serde_json::json!({ "mcpServers": ["mcp/github.server.json"] }))),
        entry("mcp/github.server.json", r#"{"command":"npx","args":["-y","gh-mcp"]}"#),
    ];
    let bi_files: Vec<bi::BundleImportFile> =
        files.iter().map(|f| bi::BundleImportFile { path: f.path.clone(), content: f.content.clone() }).collect();
    let digest = bi::content_digest_files(&bi_files);
    let req = CommitReq {
        file_path: None,
        zip_base64: None,
        files: Some(files),
        expected_content_digest: digest,
        bundle_name: None,
        include_instructions: false,
        include_context_files: vec![],
        include_skills: vec![],
        include_mcp_servers: vec!["mcp/github.server.json".to_string()],
    };
    let resp = bundle_import_commit_impl(&state.id_store, &state.identity_store, &state.mstore, &state.broker, req).await.unwrap();
    let bundle_id = resp.bundle_id;
    let saved = state.id_store.bundle_get(&bundle_id).unwrap().unwrap();
    let mcp_servers: serde_json::Value = serde_json::from_str(&saved.mcp_servers).unwrap();
    assert_eq!(mcp_servers[0]["command"], "npx");
    assert!(mcp_servers[0].get("source_path").is_none(), "must not persist the {{source_path, config}} wrapper");
}

#[tokio::test]
async fn read_abf_file_path_rejects_a_missing_path() {
    let err = read_abf_file_path("C:\\definitely\\not\\a\\real\\path.abf").unwrap_err();
    assert!(err.contains("failed to open") || err.contains("failed to stat"));
}

#[tokio::test]
async fn read_abf_file_path_rejects_a_directory() {
    let dir = std::env::temp_dir();
    let err = read_abf_file_path(dir.to_str().unwrap()).unwrap_err();
    assert!(err.contains("not a regular file") || err.contains("failed to open"));
}

#[tokio::test]
async fn read_abf_file_path_reads_a_small_file_correctly() {
    let path = std::env::temp_dir().join(format!("abf-test-{}.bin", std::process::id()));
    std::fs::write(&path, b"hello abf").unwrap();
    let result = read_abf_file_path(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(result.unwrap(), b"hello abf".to_vec());
}

#[tokio::test]
async fn read_abf_file_path_rejects_a_file_over_the_size_cap_via_sparse_file() {
    // Verified via a sparse/pre-allocated file (metadata-based rejection,
    // before any read) rather than actually writing 100MB+ to disk.
    let path = std::env::temp_dir().join(format!("abf-test-oversized-{}.bin", std::process::id()));
    {
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_ABF_FILE_SIZE_BYTES + 1).unwrap();
    }
    let result = read_abf_file_path(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    let err = result.unwrap_err();
    assert!(err.contains("exceeds the limit"), "expected a size-limit error, got: {err}");
}

#[cfg(unix)]
#[tokio::test]
async fn read_abf_file_path_rejects_a_symlink() {
    let target = std::env::temp_dir().join(format!("abf-symlink-target-{}.bin", std::process::id()));
    let link = std::env::temp_dir().join(format!("abf-symlink-{}.bin", std::process::id()));
    std::fs::write(&target, b"real content").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let result = read_abf_file_path(link.to_str().unwrap());
    std::fs::remove_file(&target).ok();
    std::fs::remove_file(&link).ok();
    assert!(result.is_err(), "opening a symlink via file_path must fail, not silently follow it");
}

#[test]
fn bound_warnings_for_response_caps_the_combined_list() {
    let many: Vec<String> = (0..500).map(|i| format!("warning {i}")).collect();
    let (bounded, truncated) = bound_warnings_for_response(many);
    assert!(truncated);
    assert!(bounded.len() <= 201);
    assert!(bounded.last().unwrap().contains("not shown"));
}

#[test]
fn bound_warnings_for_response_leaves_a_short_list_untouched() {
    let few = vec!["a".to_string(), "b".to_string()];
    let (bounded, truncated) = bound_warnings_for_response(few.clone());
    assert!(!truncated);
    assert_eq!(bounded, few);
}

#[test]
fn resolve_import_input_rejects_when_zero_or_multiple_inputs_given() {
    let budget = bi::WarningBudget::unbounded();
    let none = resolve_import_input(None, None, None, budget).unwrap_err();
    assert!(none.contains("exactly one"));
    let both = resolve_import_input(Some("x".to_string()), Some("y".to_string()), None, budget).unwrap_err();
    assert!(both.contains("exactly one"));
}
