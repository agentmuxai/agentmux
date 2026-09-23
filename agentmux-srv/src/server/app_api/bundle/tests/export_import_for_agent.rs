// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! ABF v0.2 §2.3 — `bundle.export_for_agent`/`bundle.import_for_agent`.
//! Uses a real temp directory for the native-memory filesystem (these
//! handlers genuinely touch disk, unlike the pure bundle_export.rs/
//! bundle_import.rs modules), mirroring native_memory_handlers.rs's own
//! test fixtures.

use super::super::*;
use crate::server::tests::test_state;

/// Insert an AgentDefinition with a real working_directory + a
/// CLAUDE_CONFIG_DIR env pointing at `config_dir` (must be a per-test
/// temp dir — an empty/shared value would resolve to the real
/// ~/.agentmux/shared/providers/claude/, writing test fixtures into
/// the developer's actual home directory, exactly the trap
/// native_memory_handlers.rs's own test helper's doc comment warns
/// about).
fn make_agent(state: &AppState, id: &str, working_directory: &str, config_dir: &std::path::Path) {
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": id,
        "slug": id,
        "name": id,
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "working_directory": working_directory,
        "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: id.to_string(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", config_dir.display()),
            updated_at: 0,
        })
        .unwrap();
}

fn make_bundle(state: &AppState, id: &str, instructions: &str) -> crate::backend::storage::store::Bundle {
    let bundle = crate::backend::storage::store::Bundle {
        id: id.to_string(),
        name: format!("Bundle {id}"),
        description: String::new(),
        is_blank: false,
        is_global: false,
        provider: String::new(),
        model: String::new(),
        instructions: instructions.to_string(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        created_at: 0,
        updated_at: 0,
        is_system: false,
    };
    state.id_store.bundle_upsert(&bundle).unwrap();
    bundle
}

/// Bind an MCP server to a bundle through the ref table, the way the
/// Armory does.
fn bind_mcp(state: &AppState, bundle_id: &str, name: &str, config: &str) {
    let server = crate::backend::storage::McpServer {
        id: format!("srv-{name}"),
        name: name.to_string(),
        transport: "stdio".to_string(),
        config: config.to_string(),
        is_global: false,
        created_at: 1,
        updated_at: 1,
    };
    state
        .mstore
        .bundle_mcp_upsert_unique(&state.identity_store, &state.id_store, bundle_id, &server, true)
        .unwrap();
}

/// Bind a skill to a bundle through the ref table.
fn bind_skill(state: &AppState, bundle_id: &str, name: &str) {
    let skill = crate::backend::storage::Skill {
        id: format!("skill-{name}"),
        name: name.to_string(),
        trigger: name.to_string(),
        skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
        description: String::new(),
        content: "body".to_string(),
        is_global: false,
        created_at: 1,
        updated_at: 1,
    };
    state
        .mstore
        .bundle_skill_upsert_unique(&state.identity_store, &state.id_store, bundle_id, &skill, true)
        .unwrap();
}

#[tokio::test]
async fn project_instructions_resolves_the_authenticated_slug_not_a_uuid() {
    // ReAgent P0 on #3156: check_s1 authenticates a SLUG, while
    // agent_def_get queries by UUID — so looking the agent up directly
    // returned None for every real caller. The existing test helper sets
    // id == slug, which could never have caught it, so this one keeps them
    // deliberately different.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();

    let mut def: crate::backend::storage::AgentDefinition =
        serde_json::from_value(serde_json::json!({
            "id": "11111111-2222-3333-4444-555555555555",
            "slug": "agent-slug",
            "name": "Agent Slug",
            "icon": "robot",
            "provider": "claude",
            "description": "",
            "working_directory": work.path().to_str().unwrap(),
            "created_at": 1,
        }))
        .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: def.id.clone(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", config_dir.path().display()),
            updated_at: 0,
        })
        .unwrap();

    // The slug is what an authenticated agent actually sends.
    let resolved = resolve_agent_for_s1(&state, "agent-slug")
        .expect("the authenticated slug must resolve");
    assert_eq!(resolved.id, def.id, "must find the definition behind the slug");

    // And the UUID still works, for a caller that legitimately holds one.
    let by_id = resolve_agent_for_s1(&state, &def.id).expect("a definition id must still work");
    assert_eq!(by_id.slug, "agent-slug");
}

#[tokio::test]
async fn export_carries_ref_bound_components_the_inline_columns_never_had() {
    // The Phase 0b regression. Before this, binding a skill or server in
    // the Armory wrote only a ref row while export read only the inline
    // column, so this bundle exported as empty skills/ and mcp/
    // directories with no warning — in a format advertised as a backup.
    let state = test_state();
    let bundle = make_bundle(&state, "bundle-1", "Be helpful.");
    assert_eq!(bundle.skills, "[]", "fixture must leave the inline columns empty");
    assert_eq!(bundle.mcp_servers, "[]");

    bind_skill(&state, "bundle-1", "Deploy");
    bind_mcp(&state, "bundle-1", "github", r#"{"command":"gh-mcp","env":{"GITHUB_TOKEN":"tok"}}"#);

    let result = bundle_export_impl(
        &state.id_store,
        &state.mstore,
        &state.identity_store,
        ExportReq { id: "bundle-1".to_string(), format: String::new() },
    )
    .unwrap();

    let paths: Vec<String> = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        paths.iter().any(|p| p.starts_with("skills/")),
        "ref-bound skill must be exported: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "mcp/github.server.json"),
        "ref-bound MCP server must be exported: {paths:?}"
    );

    // And the secret is still redacted on the way out — resolving from a
    // different source must not bypass the redaction pass.
    let server_file = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "mcp/github.server.json")
        .unwrap();
    let content = server_file["content"].as_str().unwrap();
    assert!(!content.contains("tok"), "secret must not survive export: {content}");
}

#[tokio::test]
async fn deleting_a_bundle_takes_its_component_refs_with_it() {
    // No FK reaches from the ref tables to db_bundles — they live in
    // different physical databases — so nothing else removes these, and a
    // later bundle reusing the id would inherit components nobody chose.
    let state = test_state();
    make_bundle(&state, "bundle-1", "Be helpful.");
    bind_skill(&state, "bundle-1", "Deploy");
    bind_mcp(&state, "bundle-1", "github", r#"{"command":"gh-mcp"}"#);

    let before = resolve_bundle_components(&state.mstore, &state.identity_store, "bundle-1").unwrap();
    assert_eq!(before.skills.len(), 1);
    assert_eq!(before.mcp_entries.len(), 1);

    assert!(state.id_store.bundle_delete("bundle-1").unwrap());
    // Through the shared helper both delete RPCs call — `bundle.delete`
    // here and `deletememory`, the one the Armory actually uses. Fixing
    // only one left the product path orphaning refs (Codex, PR #3153).
    purge_bundle_component_refs(&state.mstore, "bundle-1");
    let (skills, mcp) = state.mstore.bundle_unbind_all_components("bundle-1").unwrap();
    assert_eq!(
        (skills, mcp), (0, 0),
        "the purge must have already removed both refs"
    );

    // Re-create a bundle with the same id: it must start empty.
    make_bundle(&state, "bundle-1", "Reused id.");
    let after = resolve_bundle_components(&state.mstore, &state.identity_store, "bundle-1").unwrap();
    assert!(
        after.skills.is_empty() && after.mcp_entries.is_empty(),
        "a reused bundle id must not inherit the old bundle's components"
    );
}

#[tokio::test]
async fn validate_reports_a_malformed_bound_component_instead_of_passing() {
    // Codex on #3153: the resolver drops a bound server whose config will
    // not parse and says so in a warning. Discarding that left validate
    // reporting is_valid for exactly the component it exists to catch —
    // the entry is absent from the resolved list, so nothing downstream
    // could have seen it.
    let state = test_state();
    make_bundle(&state, "bundle-1", "Be helpful.");
    bind_mcp(&state, "bundle-1", "broken", "not json at all");

    let report = crate::server::app_api::bundle_validate_impl(
        &state.mstore,
        &state.identity_store,
        json!({ "id": "bundle-1", "name": "Bundle bundle-1" }),
    )
    .unwrap();

    assert!(
        !report.is_valid,
        "a bound component that cannot load must not validate clean: {report:?}"
    );
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.field == "mcp_servers" && i.message.contains("broken")),
        "the issue must name the server: {:?}",
        report.issues
    );
}

#[tokio::test]
async fn export_carries_project_instructions_with_their_owner() {
    // The last piece of Phase 3: an exported bundle records what the
    // source agent was reading, with enough to tell whose file each was.
    let state = test_state();
    let work = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    std::fs::write(work.path().join("CLAUDE.md"), "# House rules\n\nBe careful.\n").unwrap();

    let result = bundle_export_for_agent_impl(
        &state.id_store,
        &state.mstore,
        &state.identity_store,
        ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        },
    )
    .await
    .unwrap();

    let files = result["files"].as_array().unwrap();
    let carried = files
        .iter()
        .find(|f| f["path"] == "instructions/project/CLAUDE.md")
        .expect("the repository's own CLAUDE.md must be carried");
    assert!(carried["content"].as_str().unwrap().contains("House rules"));

    let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
    let entries = manifest["components"]["projectInstructions"].as_array().unwrap();
    let entry = entries.iter().find(|e| e["path"] == "CLAUDE.md").unwrap();
    assert_eq!(
        entry["owner"], "foreign",
        "an unmarked repository file is the repository's — this field is what keeps import from installing it"
    );
    assert!(!entry["contentHash"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn export_carries_a_readable_but_empty_instruction_file() {
    // Codex P2 on #3163: skipping on `content.is_empty()` conflated "the
    // file is there and says nothing" with "there is no file", which is
    // the exact ambiguity this component exists to remove. An empty
    // CLAUDE.md is a real fact about what the agent reads.
    let state = test_state();
    let work = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    std::fs::write(work.path().join("CLAUDE.md"), "").unwrap();

    let result = bundle_export_for_agent_impl(
        &state.id_store,
        &state.mstore,
        &state.identity_store,
        ExportForAgentReq {
            bundle_id: "bundle-1".to_string(),
            agent_id: "agent-1".to_string(),
            format: String::new(),
        },
    )
    .await
    .unwrap();

    let files = result["files"].as_array().unwrap();
    assert!(
        files.iter().any(|f| f["path"] == "instructions/project/CLAUDE.md"),
        "an empty-but-present CLAUDE.md must still be recorded"
    );
    let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
    let entries = manifest["components"]["projectInstructions"].as_array().unwrap();
    assert!(
        entries.iter().any(|e| e["path"] == "CLAUDE.md"),
        "present-and-empty must be distinguishable from absent, got: {entries:?}"
    );
}

/// Build a resolver result by hand. The two conditions below (a
/// case-only path collision, and unreadable/oversized files) are not
/// reachable through the filesystem on this repo's own dev platform —
/// Windows folds case, so the collision cannot be staged at all — so the
/// splice function is exercised directly rather than not at all.
fn instruction_file(
    path: &str,
    content: &str,
) -> crate::backend::project_instructions::ProjectInstructionFile {
    crate::backend::project_instructions::ProjectInstructionFile {
        path: path.to_string(),
        exists: true,
        size_bytes: content.len() as u64,
        content_hash: "hash".to_string(),
        owner: crate::backend::project_instructions::InstructionOwner::Foreign,
        content: content.to_string(),
        truncated: false,
        error: None,
    }
}

fn empty_export() -> crate::backend::bundle_export::BundleExport {
    crate::backend::bundle_export::BundleExport {
        root_slug: "b".to_string(),
        files: vec![crate::backend::bundle_export::BundleExportFile {
            path: "armory.json".to_string(),
            content: "{\"components\":{}}".to_string(),
        }],
        skipped_skills: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn case_only_different_instruction_paths_do_not_overwrite_each_other() {
    // Codex P2 on #3163. Scanned directories (Copilot's
    // `.github/instructions/`) make two paths differing only in case
    // reachable on a case-sensitive source filesystem; extracted on
    // Windows or macOS one silently clobbers the other.
    let mut export = empty_export();
    let files = vec![
        instruction_file(".github/instructions/A.instructions.md", "first"),
        instruction_file(".github/instructions/a.instructions.md", "second"),
    ];
    splice_project_instructions_component(&mut export, &files).unwrap();

    let carried: Vec<_> = export
        .files
        .iter()
        .filter(|f| f.path.to_lowercase().starts_with("instructions/project/"))
        .collect();
    assert_eq!(carried.len(), 1, "the second must be skipped, not written alongside");
    assert!(
        export.warnings.iter().any(|w| w.contains("normalizes to the same path")),
        "the skip must be reported, not silent: {:?}",
        export.warnings
    );
}

#[test]
fn unreadable_and_truncated_instruction_files_are_not_exported() {
    // The other half of the empty-file fix: `content.is_empty()` was doing
    // two jobs, and only one of them was correct. These are the cases that
    // genuinely have nothing to archive.
    let mut export = empty_export();
    let mut unreadable = instruction_file("CLAUDE.md", "");
    unreadable.error = Some("permission denied".to_string());
    let mut oversized = instruction_file("AGENTS.md", "");
    oversized.truncated = true;
    let mut absent = instruction_file("GEMINI.md", "");
    absent.exists = false;

    splice_project_instructions_component(&mut export, &[unreadable, oversized, absent])
        .unwrap();

    assert!(
        !export.files.iter().any(|f| f.path.starts_with("instructions/project/")),
        "nothing readable means nothing to carry"
    );
}

#[tokio::test]
async fn a_scanned_file_deleted_after_launch_is_reported_removed() {
    // Codex on #3162: the resolver stops returning a scanned path once the
    // file is gone, so iterating current files only made it vanish from
    // the response instead of reporting `removed` — and the stale row
    // meant recreating it would later read as `unchanged`.
    use crate::backend::storage::project_instructions::{
        classify_change, InstructionChange, ProjectInstructionObservation,
    };

    let state = test_state();
    let work = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());

    // Recorded at a previous launch, and since deleted from disk.
    state
        .mstore
        .project_instructions_record(
            "agent-1",
            &[ProjectInstructionObservation {
                path: ".github/instructions/gone.instructions.md".to_string(),
                content_hash: "h1".to_string(),
                size_bytes: 3,
                owner: "foreign".to_string(),
                existed: true,
                observed_at: 1,
            }],
        )
        .unwrap();

    let current = crate::backend::project_instructions::resolve_project_instructions(
        "copilot",
        work.path().to_str().unwrap(),
    );
    assert!(
        !current.iter().any(|f| f.path.ends_with("gone.instructions.md")),
        "precondition: the resolver no longer returns a deleted scanned file"
    );

    let previous = state.mstore.project_instructions_list("agent-1").unwrap();
    let orphan = previous
        .iter()
        .find(|p| p.path.ends_with("gone.instructions.md"))
        .unwrap();
    assert_eq!(
        classify_change(Some(orphan), false, ""),
        InstructionChange::Removed,
        "a prior observation with no current file is a removal"
    );
}

#[tokio::test]
async fn a_foreign_instruction_file_changing_between_launches_is_visible() {
    // The point of tracking: a repository's own CLAUDE.md can be the
    // majority of what an agent is told, and until now nothing recorded
    // enough about one to notice it had changed.
    use crate::backend::storage::project_instructions::InstructionChange;

    let state = test_state();
    let work = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", work.path().to_str().unwrap(), config_dir.path());
    let agent = state.mstore.agent_def_get("agent-1").unwrap().unwrap();

    std::fs::write(work.path().join("CLAUDE.md"), "original rules").unwrap();

    // First launch: nothing recorded yet.
    let first = crate::backend::project_instructions::resolve_project_instructions(
        &agent.provider,
        work.path().to_str().unwrap(),
    );
    let claude = first.iter().find(|f| f.path == "CLAUDE.md").unwrap();
    assert_eq!(
        crate::backend::storage::project_instructions::classify_change(
            None, claude.exists, &claude.content_hash
        ),
        InstructionChange::FirstSeen
    );
    crate::server::app_api::agent_open::observe_project_instructions(
        &state.mstore,
        &agent,
        work.path().to_str().unwrap(),
    );

    // The repository changes underneath.
    std::fs::write(work.path().join("CLAUDE.md"), "somebody edited this").unwrap();

    let second = crate::backend::project_instructions::resolve_project_instructions(
        &agent.provider,
        work.path().to_str().unwrap(),
    );
    let claude = second.iter().find(|f| f.path == "CLAUDE.md").unwrap();
    let recorded = state.mstore.project_instructions_list("agent-1").unwrap();
    let prev = recorded.iter().find(|p| p.path == "CLAUDE.md");
    assert_eq!(
        crate::backend::storage::project_instructions::classify_change(
            prev, claude.exists, &claude.content_hash
        ),
        InstructionChange::Modified,
        "an edit between launches must be detectable"
    );

    // Observing again settles it.
    crate::server::app_api::agent_open::observe_project_instructions(
        &state.mstore,
        &agent,
        work.path().to_str().unwrap(),
    );
    let recorded = state.mstore.project_instructions_list("agent-1").unwrap();
    let prev = recorded.iter().find(|p| p.path == "CLAUDE.md");
    assert_eq!(
        crate::backend::storage::project_instructions::classify_change(
            prev, claude.exists, &claude.content_hash
        ),
        InstructionChange::Unchanged
    );
}

#[tokio::test]
async fn a_bound_mcp_server_with_unparseable_config_warns_instead_of_vanishing() {
    // The guarantee that moved here from bundle_export.rs when the
    // renderer stopped parsing MCP JSON: malformed component data warns,
    // it does not silently disappear.
    let state = test_state();
    make_bundle(&state, "bundle-1", "Be helpful.");
    bind_mcp(&state, "bundle-1", "broken", "not json at all");

    let components = resolve_bundle_components(&state.mstore, &state.identity_store, "bundle-1").unwrap();
    assert!(components.mcp_entries.is_empty(), "unparseable config must not be exported");
    assert!(
        components.warnings.iter().any(|w| w.contains("broken") && w.contains("invalid config")),
        "expected a warning naming the server, got: {:?}",
        components.warnings
    );
}

#[tokio::test]
async fn resolving_ignores_catalog_rows_this_bundle_is_not_bound_to() {
    // `managed_list` returns globals alongside bound rows; only the bound
    // ones are this bundle's contents.
    let state = test_state();
    make_bundle(&state, "bundle-1", "Be helpful.");
    make_bundle(&state, "bundle-2", "Other.");
    bind_mcp(&state, "bundle-2", "elsewhere", r#"{"command":"x"}"#);

    let components = resolve_bundle_components(&state.mstore, &state.identity_store, "bundle-1").unwrap();
    assert!(
        components.mcp_entries.is_empty(),
        "another bundle's server must not leak in: {:?}",
        components.mcp_entries
    );
    assert!(components.skills.is_empty());
}

#[tokio::test]
async fn export_for_agent_includes_normal_components_and_native_memory() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    // Written directly to the live FS, bypassing the mirror entirely —
    // proves the export path's own refresh (not a pre-existing mirror
    // row) is what picks this up.
    let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "Learned fact.").unwrap();

    let result = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-1".to_string(),
        agent_id: "agent-1".to_string(),
        format: String::new(),
    }).await.unwrap();

    let files = result["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "instructions/AGENTS.md" && f["content"] == "Be helpful."));
    let memory_file = files.iter().find(|f| f["path"] == "memory/MEMORY.md")
        .expect("expected memory/MEMORY.md in the export — live-FS refresh must have picked it up");
    assert_eq!(memory_file["content"], "Learned fact.");

    let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
    let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
    assert_eq!(manifest["components"]["memory"], json!(["memory/MEMORY.md"]));

    // The refresh must also have durably mirrored it, not just read it
    // for this one export.
    assert_eq!(
        state.id_store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(),
        Some("Learned fact.".to_string())
    );
}

#[tokio::test]
async fn export_for_agent_omits_memory_component_when_agent_has_none() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    let result = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-1".to_string(),
        agent_id: "agent-1".to_string(),
        format: String::new(),
    }).await.unwrap();

    let files = result["files"].as_array().unwrap();
    assert!(!files.iter().any(|f| f["path"].as_str().unwrap_or("").starts_with("memory/")));
    let manifest_file = files.iter().find(|f| f["path"] == "armory.json").unwrap();
    let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
    assert!(manifest["components"].get("memory").is_none());
}

#[tokio::test]
async fn export_for_agent_errors_for_an_unknown_bundle_or_agent() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    let err = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "no-such-bundle".to_string(),
        agent_id: "agent-1".to_string(),
        format: String::new(),
    }).await.unwrap_err();
    assert!(err.contains("no bundle"));

    let err = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-1".to_string(),
        agent_id: "no-such-agent".to_string(),
        format: String::new(),
    }).await.unwrap_err();
    assert!(err.contains("no agent"));
}

/// Point an agent DEFINITION's own bundle at `bundle_id`.
///
/// Uses the dedicated setter, not `agent_def_update`: that UPDATE
/// deliberately does not list `default_memory_id` among its columns
/// (`storage/agents.rs:1188-1197`), so mutating `AgentDefinition.memory_id`
/// and calling it would silently no-op.
fn bind_definition_to_bundle(state: &AppState, agent_id: &str, bundle_id: &str) {
    assert!(
        state
            .mstore
            .agent_def_set_memory_id_if_empty(agent_id, bundle_id)
            .unwrap(),
        "test setup: binding {agent_id} to {bundle_id} must apply"
    );
}

#[tokio::test]
async fn agent_less_export_warns_when_a_bound_agent_has_memory() {
    // SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md §3.1/§5.1:
    // import announces a dropped memory component; export must too.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");
    bind_definition_to_bundle(&state, "agent-1", "bundle-1");
    state
        .id_store
        .agent_native_memory_upsert("agent-1", "MEMORY.md", "a fact", None, "/x", 6, 0)
        .unwrap();

    assert!(
        bound_agent_has_native_memory(&state.id_store, &state.mstore, "bundle-1"),
        "definition-bound agent with memory must be detected"
    );

    // The contract that matters is at the RPC boundary, not the helper.
    let result = bundle_export_impl(
        &state.id_store,
        &state.mstore,
        &state.identity_store,
        ExportReq { id: "bundle-1".to_string(), format: String::new() },
    )
    .unwrap();
    let warnings = result["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w == MEMORY_NOT_EXPORTED_WARNING),
        "bundle.export must announce the memory it is not carrying: {warnings:?}"
    );
    // ...and still not carry it: this path has no agent to read from.
    let files = result["files"].as_array().unwrap();
    assert!(!files.iter().any(|f| f["path"].as_str().unwrap_or("").starts_with("memory/")));
}

#[tokio::test]
async fn agent_less_export_is_silent_without_memory_or_without_a_binding() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");
    make_bundle(&state, "bundle-2", "Other.");
    bind_definition_to_bundle(&state, "agent-1", "bundle-1");

    // Bound, but the agent has no memory at all.
    assert!(!bound_agent_has_native_memory(&state.id_store, &state.mstore, "bundle-1"));

    state
        .id_store
        .agent_native_memory_upsert("agent-1", "MEMORY.md", "a fact", None, "/x", 6, 0)
        .unwrap();

    // Now it has memory — but bundle-2 is bound to nobody, so exporting
    // bundle-2 loses nothing and must stay quiet.
    assert!(!bound_agent_has_native_memory(&state.id_store, &state.mstore, "bundle-2"));

    // The blank-singleton sentinel must never warn, or nearly every
    // export would.
    assert!(!bound_agent_has_native_memory(&state.id_store, &state.mstore, ""));

    // And the RPC stays quiet for the unbound bundle.
    let result = bundle_export_impl(
        &state.id_store,
        &state.mstore,
        &state.identity_store,
        ExportReq { id: "bundle-2".to_string(), format: String::new() },
    )
    .unwrap();
    let warnings = result["warnings"].as_array().unwrap();
    assert!(
        !warnings.iter().any(|w| w == MEMORY_NOT_EXPORTED_WARNING),
        "exporting a bundle nobody is bound to loses nothing: {warnings:?}"
    );
}

#[tokio::test]
async fn agent_less_export_warns_for_an_instance_scoped_binding_too() {
    // The definition's own bundle and a launch's bundle are different
    // columns; a launch pointed at another bundle still carries the
    // definition's memory.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");
    make_bundle(&state, "bundle-2", "Other.");
    bind_definition_to_bundle(&state, "agent-1", "bundle-1");
    state
        .id_store
        .agent_native_memory_upsert("agent-1", "MEMORY.md", "a fact", None, "/x", 6, 0)
        .unwrap();

    let inst: crate::backend::storage::AgentInstance =
        serde_json::from_value(serde_json::json!({
            "id": "inst-1",
            "definition_id": "agent-1",
            "memory_id": "bundle-2",
            "status": "running",
            "started_at": 1,
            "created_at": 1,
        }))
        .unwrap();
    state.mstore.instance_create(&inst).unwrap();

    assert!(
        bound_agent_has_native_memory(&state.id_store, &state.mstore, "bundle-2"),
        "a launch pointed at bundle-2 still carries agent-1's memory"
    );
}

fn abf_files_with_memory(instructions: &str, memory_filename: &str, memory_content: &str) -> Vec<FileEntry> {
    let manifest = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
        "name": "imported-bundle",
        "version": "0.1.0",
        "description": "",
        "components": {
            "instructions": { "default": ["instructions/AGENTS.md"] },
            "memory": [format!("memory/{memory_filename}")],
        },
        "metadata": {},
    });
    vec![
        FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
        FileEntry { path: "instructions/AGENTS.md".to_string(), content: instructions.to_string() },
        FileEntry { path: format!("memory/{memory_filename}"), content: memory_content.to_string() },
    ]
}

/// An ABF carrying one skill and one MCP server.
fn abf_files_with_components() -> Vec<FileEntry> {
    let manifest = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
        "name": "imported-bundle",
        "version": "0.1.0",
        "description": "",
        "components": {
            "instructions": { "default": ["instructions/AGENTS.md"] },
            "skills": ["skills/deploy"],
            "mcpServers": ["mcp/github.server.json"],
        },
        "metadata": {},
    });
    vec![
        FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
        FileEntry { path: "instructions/AGENTS.md".to_string(), content: "Be helpful.".to_string() },
        FileEntry {
            path: "skills/deploy/SKILL.md".to_string(),
            // Frontmatter values are JSON-quoted — that is what
            // `render_skill_md` writes and what `parse_skill_md` requires.
            content: "---\nname: \"deploy\"\ndescription: \"Ship it\"\n---\n\nSteps.".to_string(),
        },
        FileEntry {
            path: "mcp/github.server.json".to_string(),
            content: r#"{"name":"github","command":"gh-mcp","env":{"GITHUB_TOKEN":"${GITHUB_TOKEN}"}}"#.to_string(),
        },
    ]
}

#[tokio::test]
async fn an_imported_bundle_exports_the_components_it_arrived_with() {
    // Codex P1 on #3152: once export reads only the ref tables, an import
    // that writes only the inline columns produces a bundle that exports
    // empty — and whose MCP servers never reach a spawned agent, since
    // launch reads the refs too. The round trip is the contract.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    let result = bundle_import_for_agent_impl(
        &state.id_store,
        &state.identity_store,
        &state.mstore,
        ImportForAgentReq {
            agent_id: "agent-1".to_string(),
            file_path: None,
            zip_base64: None,
            files: Some(abf_files_with_components()),
        },
    )
    .await
    .unwrap();

    let bundle_id = result["bundle_id"]
        .as_str()
        .or_else(|| result["id"].as_str())
        .expect("import must report the bundle it created")
        .to_string();

    let components = resolve_bundle_components(&state.mstore, &state.identity_store, &bundle_id).unwrap();
    assert_eq!(
        components.skills.len(),
        1,
        "imported skill must be bound to the bundle. import result: {result}"
    );
    assert_eq!(
        components.mcp_entries.len(),
        1,
        "imported MCP server must be bound to the bundle: {:?}",
        components.warnings
    );

    // And it round-trips out again.
    let exported = bundle_export_impl(
        &state.id_store,
        &state.mstore,
        &state.identity_store,
        ExportReq { id: bundle_id, format: String::new() },
    )
    .unwrap();
    let paths: Vec<String> = exported["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(paths.iter().any(|p| p.starts_with("skills/")), "got {paths:?}");
    assert!(paths.iter().any(|p| p == "mcp/github.server.json"), "got {paths:?}");
}

#[tokio::test]
async fn import_for_agent_rejects_a_target_with_existing_memory() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    state.id_store.agent_native_memory_upsert("agent-1", "MEMORY.md", "already here", None, "/x", 5, 0).unwrap();

    let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Imported fact.");
    let err = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-1".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap_err();
    assert!(err.contains("already has"));

    // Rejected up front — the pre-existing row must survive untouched.
    assert_eq!(
        state.id_store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(),
        Some("already here".to_string())
    );
}

#[tokio::test]
async fn import_for_agent_rejects_a_target_with_an_unmirrored_live_memory_file() {
    // reagent P0, PR #2527: a file written directly to the live FS
    // (never viewed through Stash, so never mirrored) must still be
    // detected by the "zero existing memory" guard -- otherwise the
    // write loop would silently overwrite it via fs::rename.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "Never mirrored, but real.").unwrap();
    // Confirm the premise: nothing in the mirror yet.
    assert!(state.id_store.agent_native_memory_list_meta("agent-1").unwrap().is_empty());

    let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Would-be overwrite.");
    let err = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-1".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap_err();
    assert!(err.contains("already has"));

    // The live file must survive untouched.
    assert_eq!(
        std::fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap(),
        "Never mirrored, but real."
    );
}

#[tokio::test]
async fn import_for_agent_fails_fast_when_agent_has_no_resolvable_memory_dir_but_bundle_has_memory() {
    // reagent P2, PR #2527: must fail BEFORE creating any skill/bundle
    // rows, not silently succeed with memory_files_written: 0 -- this
    // RPC exists specifically to transfer memory.
    //
    // The trigger narrowed in PR #2901: a blank `working_directory` alone
    // no longer reaches this guard, because it now resolves to the same
    // default `agent.open` substitutes (~/.agentmux/agents/<name-slug>) --
    // which serves this guard's OWN stated purpose better than erroring
    // did, since the memory is written where the agent will actually read
    // it instead of being refused. The guard remains for the genuinely
    // unresolvable case, which needs a blank NAME too (no name => no
    // derivable default), exercised here.
    let state = test_state();
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "agent-no-workdir",
        "slug": "agent-no-workdir",
        "name": "",
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "working_directory": "",
        "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();

    let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Some fact.");
    let err = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-no-workdir".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap_err();
    assert!(err.contains("no working directory"));

    // Nothing should have been created.
    assert!(state.id_store.bundle_list().unwrap().iter().all(|b| b.is_blank));
}

#[tokio::test]
async fn export_for_agent_warns_when_a_memory_file_is_truncated() {
    // reagent P2, PR #2527: a file over the native-memory size cap
    // must be flagged, not silently exported partial with no signal.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    // One byte over the 10 MiB cap.
    let oversized = "x".repeat(10 * 1024 * 1024 + 1);
    std::fs::write(memory_dir.join("MEMORY.md"), &oversized).unwrap();

    let result = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-1".to_string(),
        agent_id: "agent-1".to_string(),
        format: String::new(),
    }).await.unwrap();

    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(
        warnings.iter().any(|w| w.contains("MEMORY.md") && w.contains("truncated")),
        "expected a truncation warning, got: {warnings:?}"
    );
}

#[tokio::test]
async fn export_for_agent_repeats_the_truncation_warning_on_an_unchanged_oversized_file() {
    // reagent P2, PR #2527 (second round): the truncation check used
    // to run only inside the "changed since last mirror" branch, so
    // a SECOND export of the same still-oversized, unchanged file
    // silently stopped warning even though the exported content was
    // still truncated every time.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());
    make_bundle(&state, "bundle-1", "Be helpful.");

    let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let oversized = "x".repeat(10 * 1024 * 1024 + 1);
    std::fs::write(memory_dir.join("MEMORY.md"), &oversized).unwrap();

    // First export mirrors it (and warns).
    let _ = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-1".to_string(),
        agent_id: "agent-1".to_string(),
        format: String::new(),
    }).await.unwrap();

    // Second export: file on disk is byte-for-byte unchanged (same
    // size+mtime), so the mirror-refresh's "unchanged" fast path
    // applies — the warning must still fire.
    let result = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-1".to_string(),
        agent_id: "agent-1".to_string(),
        format: String::new(),
    }).await.unwrap();
    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(
        warnings.iter().any(|w| w.contains("MEMORY.md") && w.contains("truncated")),
        "truncation warning must repeat on a second export of the same unchanged oversized file, got: {warnings:?}"
    );
}

#[tokio::test]
async fn import_for_agent_writes_memory_to_both_live_fs_and_mirror() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Imported fact.");
    let result = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-1".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap();

    assert_eq!(result["memory_files_written"], 1);
    assert!(result["bundle_id"].as_str().unwrap().len() > 0);

    // Live FS.
    let memory_dir = config_dir.path().join("projects").join("-work-proj").join("memory");
    let on_disk = std::fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap();
    assert_eq!(on_disk, "Imported fact.");

    // Mirror.
    assert_eq!(
        state.id_store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(),
        Some("Imported fact.".to_string())
    );

    // The bundle row itself was also created.
    let bundle_id = result["bundle_id"].as_str().unwrap();
    let bundle = state.id_store.bundle_get(bundle_id).unwrap().unwrap();
    assert_eq!(bundle.instructions, "Be helpful.");
}

#[tokio::test]
async fn import_for_agent_does_not_falsely_claim_memory_was_ignored() {
    // reagent P1, PR #2527: parse_bundle_import's "components.memory:
    // present but ignored" warning is correct for bundle.import/.
    // preview/.commit, but bundle_import_for_agent_impl DOES handle
    // memory (as this same test's success asserts) — it must not
    // also carry that misleading warning into its own response.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    let files = abf_files_with_memory("Be helpful.", "MEMORY.md", "Imported fact.");
    let result = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-1".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap();

    assert_eq!(result["memory_files_written"], 1);
    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(
        !warnings.iter().any(|w| w.contains("present but ignored")),
        "memory was actually processed; the 'ignored' warning must not appear: {warnings:?}"
    );
}

#[test]
fn bundle_import_for_agent_lock_is_keyed_by_agent_id() {
    // Direct test of the lock mechanism itself, since the join-based
    // test below can't rigorously prove the lock (as opposed to
    // incidental single-threaded-runtime serialization) is what
    // makes concurrent calls behave — bundle_import_for_agent_impl's
    // body has no other .await points, so it already serializes on a
    // CURRENT_THREAD runtime regardless of the lock. Production runs
    // multi-threaded, where that incidental serialization doesn't
    // apply and the lock is load-bearing.
    let lock_a1 = bundle_import_for_agent_lock("agent-1");
    let lock_a2 = bundle_import_for_agent_lock("agent-1");
    assert!(Arc::ptr_eq(&lock_a1, &lock_a2), "the same agent_id must return the same lock instance");

    let lock_b = bundle_import_for_agent_lock("agent-2");
    assert!(!Arc::ptr_eq(&lock_a1, &lock_b), "different agent_ids must not share a lock");
}

#[tokio::test]
async fn concurrent_imports_for_the_same_agent_do_not_both_succeed() {
    // reagent P2, PR #2527: without the per-agent lock, two concurrent
    // bundle.import_for_agent calls for the same agent could both
    // pass the "zero existing rows" check before either writes. The
    // lock serializes them fully — one must complete (and its memory
    // row must exist) before the other's own zero-rows check runs,
    // so the second is guaranteed to see the first's write and fail.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    let files_a = abf_files_with_memory("A.", "MEMORY.md", "From import A.");
    let files_b = abf_files_with_memory("B.", "MEMORY.md", "From import B.");

    let (result_a, result_b) = tokio::join!(
        bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(), file_path: None, zip_base64: None, files: Some(files_a),
        }),
        bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
            agent_id: "agent-1".to_string(), file_path: None, zip_base64: None, files: Some(files_b),
        }),
    );

    let outcomes = [result_a.is_ok(), result_b.is_ok()];
    assert_eq!(
        outcomes.iter().filter(|ok| **ok).count(),
        1,
        "exactly one of two concurrent imports for the same agent must succeed, got {outcomes:?}"
    );
    if let Err(e) = if result_a.is_err() { &result_a } else { &result_b } {
        assert!(e.contains("already has"), "the losing import must fail with the existing-memory guard, got: {e}");
    }
}

#[tokio::test]
async fn import_for_agent_rejects_an_unsafe_memory_filename() {
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    // A manifest referencing a path outside memory/ conventions —
    // validate_memory_filename must reject it, not write it.
    let manifest = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
        "name": "imported-bundle",
        "version": "0.1.0",
        "description": "",
        "components": { "memory": ["memory/../escape.md"] },
        "metadata": {},
    });
    let files = vec![
        FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
        FileEntry { path: "memory/../escape.md".to_string(), content: "malicious".to_string() },
    ];
    let result = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-1".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap();
    assert_eq!(result["memory_files_written"], 0);
    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(
        warnings.iter().any(|w| w.contains("not a valid memory filename")),
        "got: {warnings:?}"
    );
}

#[tokio::test]
async fn import_for_agent_warns_when_a_declared_memory_path_has_no_matching_content() {
    // reagent P1, PR #2527 (third round): a components.memory entry
    // with no matching file among the bundle's contents used to
    // silently continue with no warning, undercounting
    // memory_files_written with zero signal to the caller — this RPC
    // exists specifically to transfer memory, so every skip must warn.
    let state = test_state();
    let config_dir = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-1", "/work/proj", config_dir.path());

    let manifest = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
        "name": "imported-bundle",
        "version": "0.1.0",
        "description": "",
        "components": { "memory": ["memory/MISSING.md"] },
        "metadata": {},
    });
    // Deliberately no "memory/MISSING.md" entry in files.
    let files = vec![
        FileEntry { path: "armory.json".to_string(), content: manifest.to_string() },
    ];
    let result = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-1".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap();
    assert_eq!(result["memory_files_written"], 0);
    let warnings: Vec<&str> = result["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(
        warnings.iter().any(|w| w.contains("MISSING.md") && w.contains("not found")),
        "got: {warnings:?}"
    );
}

#[tokio::test]
async fn round_trip_export_then_import_into_a_fresh_agent_preserves_memory() {
    let state = test_state();
    let config_a = tempfile::tempdir().unwrap();
    let config_b = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-a", "/work/a", config_a.path());
    make_agent(&state, "agent-b", "/work/b", config_b.path());
    make_bundle(&state, "bundle-src", "Shared instructions.");

    let memory_dir_a = config_a.path().join("projects").join("-work-a").join("memory");
    std::fs::create_dir_all(&memory_dir_a).unwrap();
    std::fs::write(memory_dir_a.join("MEMORY.md"), "Agent A's learned fact.").unwrap();

    let exported = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-src".to_string(),
        agent_id: "agent-a".to_string(),
        format: String::new(),
    }).await.unwrap();

    let files: Vec<FileEntry> = exported["files"].as_array().unwrap().iter()
        .map(|f| FileEntry {
            path: f["path"].as_str().unwrap().to_string(),
            content: f["content"].as_str().unwrap().to_string(),
        })
        .collect();

    let imported = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-b".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap();
    assert_eq!(imported["memory_files_written"], 1);

    assert_eq!(
        state.id_store.agent_native_memory_read("agent-b", "MEMORY.md").unwrap(),
        Some("Agent A's learned fact.".to_string())
    );
    let memory_dir_b = config_b.path().join("projects").join("-work-b").join("memory");
    assert_eq!(
        std::fs::read_to_string(memory_dir_b.join("MEMORY.md")).unwrap(),
        "Agent A's learned fact."
    );
}

// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.3/§7.5 step 6:
// provider/model round-trip through export -> import, same as the
// memory-files round-trip above.
#[tokio::test]
async fn export_then_import_carries_provider_and_model_through() {
    let state = test_state();
    let config_a = tempfile::tempdir().unwrap();
    let config_b = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-a", "/work/a", config_a.path());
    make_agent(&state, "agent-b", "/work/b", config_b.path());
    let mut bundle = make_bundle(&state, "bundle-src", "Shared instructions.");
    bundle.provider = "claude".to_string();
    bundle.model = "anthropic".to_string();
    state.id_store.bundle_upsert(&bundle).unwrap();

    let exported = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-src".to_string(),
        agent_id: "agent-a".to_string(),
        format: String::new(),
    }).await.unwrap();

    let manifest_file = exported["files"].as_array().unwrap().iter()
        .find(|f| f["path"] == "armory.json")
        .expect("armory.json must be present");
    let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
    assert_eq!(manifest["provider"], "claude");
    assert_eq!(manifest["model"], "anthropic");

    let files: Vec<FileEntry> = exported["files"].as_array().unwrap().iter()
        .map(|f| FileEntry {
            path: f["path"].as_str().unwrap().to_string(),
            content: f["content"].as_str().unwrap().to_string(),
        })
        .collect();
    let imported = bundle_import_for_agent_impl(&state.id_store, &state.identity_store, &state.mstore, ImportForAgentReq {
        agent_id: "agent-b".to_string(),
        file_path: None,
        zip_base64: None,
        files: Some(files),
    }).await.unwrap();
    let new_bundle_id = imported["bundle_id"].as_str().expect("import response must include bundle_id");
    let new_bundle = state.id_store.bundle_get(new_bundle_id).unwrap().unwrap();
    assert_eq!(new_bundle.provider, "claude");
    assert_eq!(new_bundle.model, "anthropic");
}

#[tokio::test]
async fn export_of_an_unbound_bundle_omits_provider_and_model_rather_than_exporting_empty_strings() {
    let state = test_state();
    let config_a = tempfile::tempdir().unwrap();
    make_agent(&state, "agent-a", "/work/a", config_a.path());
    make_bundle(&state, "bundle-unbound", "Shared instructions."); // provider/model left empty

    let exported = bundle_export_for_agent_impl(&state.id_store, &state.mstore, &state.identity_store, ExportForAgentReq {
        bundle_id: "bundle-unbound".to_string(),
        agent_id: "agent-a".to_string(),
        format: String::new(),
    }).await.unwrap();

    let manifest_file = exported["files"].as_array().unwrap().iter()
        .find(|f| f["path"] == "armory.json")
        .expect("armory.json must be present");
    let manifest: serde_json::Value = serde_json::from_str(manifest_file["content"].as_str().unwrap()).unwrap();
    assert!(manifest["provider"].is_null(), "unset provider must not export as an empty string");
    assert!(manifest["model"].is_null(), "unset model must not export as an empty string");
}

// docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
// §3.3 — bundle.export_for_agent_with_history.

mod with_history_tests {
    use super::*;
    use crate::backend::history::adapter::{DiscoveredFile, HistoryAdapter, HistoryError, HistorySession, SessionMeta};
    use crate::backend::history::index::SessionIndex;
    use crate::backend::history::HistoryService;

    /// Tags every discovered file with a fixed identity_id -- enough to
    /// exercise the agent_id -> identity_id -> sessions chain without a
    /// real filesystem scan.
    struct MockAdapter {
        files: Vec<DiscoveredFile>,
        identity_id: String,
    }
    impl HistoryAdapter for MockAdapter {
        fn provider(&self) -> &str {
            "mock"
        }
        fn discover_files(&self) -> Result<Vec<DiscoveredFile>, HistoryError> {
            Ok(self.files.iter().map(|f| DiscoveredFile { file_path: f.file_path.clone(), mtime_ms: f.mtime_ms }).collect())
        }
        fn extract_meta(&self, file_path: &str) -> Result<Option<SessionMeta>, HistoryError> {
            let id = std::path::Path::new(file_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(Some(SessionMeta {
                session_id: id,
                file_path: file_path.to_string(),
                provider: "mock".to_string(),
                model: String::new(),
                slug: String::new(),
                working_directory: "/proj".to_string(),
                created_at: 0,
                modified_at: 0,
                message_count: 0,
                first_user_message: String::new(),
                file_size_bytes: 0,
                git_branch: String::new(),
                total_tokens: 0,
                subagent_count: 0,
                identity_id: self.identity_id.clone(),
            }))
        }
        fn parse_file(&self, _: &str) -> Result<Option<HistorySession>, HistoryError> {
            Ok(None)
        }
    }

    fn link_agent_to_identity(state: &AppState, agent_id: &str, account_id: &str) {
        state
            .id_store
            .identity_upsert(&crate::backend::storage::store::IdentityAccount {
                id: account_id.to_string(),
                name: format!("claude-{account_id}"),
                provider: "claude".to_string(),
                kind: "pat".to_string(),
                display_name: String::new(),
                secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: String::new() },
                context: serde_json::json!({}),
                status: "unknown".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        state.id_store.agent_identity_link(agent_id, account_id, "claude").unwrap();
    }

    fn unzip_paths(zip_base64: &str) -> Vec<String> {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(zip_base64).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect()
    }

    /// The `armory.json` manifest out of an exported zip.
    fn unzip_manifest(zip_base64: &str) -> serde_json::Value {
        use base64::Engine as _;
        use std::io::Read as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(zip_base64).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let name = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .find(|n| n.ends_with("armory.json"))
            .expect("export must contain armory.json");
        let mut content = String::new();
        archive.by_name(&name).unwrap().read_to_string(&mut content).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[tokio::test]
    async fn includes_the_agents_sessions_alongside_the_normal_bundle_files() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        link_agent_to_identity(&state, "agent-1", "acct-mine");

        let session_dir = tempfile::tempdir().unwrap();
        let session_path = session_dir.path().join("sess-1.jsonl");
        std::fs::write(&session_path, "{\"role\":\"user\"}\n").unwrap();

        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: session_path.to_string_lossy().into_owned(), mtime_ms: 1 }],
                identity_id: "acct-mine".to_string(),
            })],
            vec![session_dir.path().to_path_buf()],
        );
        let history_service = HistoryService::from_index(index);

        let result = bundle_export_for_agent_with_history_impl(
            state.id_store.clone(),
            state.identity_store.clone(),
            &state.mstore,
            std::sync::Arc::new(history_service),
            ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
        )
        .await
        .unwrap();

        assert_eq!(result["history_session_count"], 1);
        let paths = unzip_paths(result["zip_base64"].as_str().unwrap());
        assert!(paths.iter().any(|p| p.ends_with("instructions/AGENTS.md")), "base bundle files must still be present: {paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("history/mock/sess-1.jsonl")), "session must be included under history/: {paths:?}");

        // SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md §3.5: the
        // files above were in the archive but invisible to anything reading
        // components.* to learn what the archive holds.
        let manifest = unzip_manifest(result["zip_base64"].as_str().unwrap());
        assert_eq!(
            manifest["components"]["history"],
            json!(["history/mock/sess-1.jsonl"]),
            "history must be registered in the manifest, not just zipped"
        );
    }

    #[tokio::test]
    async fn omits_the_history_component_when_no_session_was_included() {
        // Absence already means "none"; an empty array would assert
        // "this archive has an empty history", which is a different claim.
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        // No identity link -> no sessions discovered.

        let index = SessionIndex::with_isolated_roots(vec![], vec![]);
        let history_service = HistoryService::from_index(index);

        let result = bundle_export_for_agent_with_history_impl(
            state.id_store.clone(),
            state.identity_store.clone(),
            &state.mstore,
            std::sync::Arc::new(history_service),
            ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
        )
        .await
        .unwrap();

        assert_eq!(result["history_session_count"], 0);
        let manifest = unzip_manifest(result["zip_base64"].as_str().unwrap());
        assert!(
            manifest["components"].get("history").is_none(),
            "no sessions must leave the history component absent: {}",
            manifest["components"]
        );
    }

    #[tokio::test]
    async fn succeeds_with_zero_sessions_when_the_agent_has_no_linked_identity() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        // No link_agent_to_identity call -- agent has no bound account.

        let history_service = HistoryService::from_index(SessionIndex::with_isolated_roots(vec![], vec![]));

        let result = bundle_export_for_agent_with_history_impl(
            state.id_store.clone(),
            state.identity_store.clone(),
            &state.mstore,
            std::sync::Arc::new(history_service),
            ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
        )
        .await
        .unwrap();

        assert_eq!(result["history_session_count"], 0);
        let paths = unzip_paths(result["zip_base64"].as_str().unwrap());
        assert!(!paths.iter().any(|p| p.contains("history/")), "no history/ entries when nothing is linked: {paths:?}");
    }

    #[tokio::test]
    async fn warns_and_continues_when_a_transcript_file_cannot_be_read() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        link_agent_to_identity(&state, "agent-1", "acct-mine");

        // Points at a file that will never exist on disk.
        let missing_path = std::env::temp_dir().join("amx-nonexistent-session-xyz.jsonl");
        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: missing_path.to_string_lossy().into_owned(), mtime_ms: 1 }],
                identity_id: "acct-mine".to_string(),
            })],
            vec![],
        );
        // Note: refresh() would normally skip a file it can't stat via
        // discover_files' own is_dir/exists checks in the real adapter,
        // but MockAdapter unconditionally reports it as discovered so
        // extract_meta -- and therefore the export's own read -- is what
        // actually exercises the missing-file path here.
        let history_service = HistoryService::from_index(index);

        let result = bundle_export_for_agent_with_history_impl(
            state.id_store.clone(),
            state.identity_store.clone(),
            &state.mstore,
            std::sync::Arc::new(history_service),
            ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
        )
        .await
        .unwrap();

        assert_eq!(result["history_session_count"], 0, "unreadable session must not count as included");
        let warnings = result["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap_or("").contains("failed to read session")),
            "must warn about the unreadable session: {warnings:?}"
        );
    }

    // reagentx P1 on PR #2613: an oversized transcript must be skipped
    // (never truncated -- a partial JSONL file can fail or misparse on
    // the receiving end) and warned about, not silently included or
    // read into memory unbounded.
    #[tokio::test]
    async fn skips_and_warns_on_a_transcript_over_the_size_limit() {
        let state = test_state();
        let config_dir = tempfile::tempdir().unwrap();
        make_agent(&state, "agent-1", "/work/proj", config_dir.path());
        make_bundle(&state, "bundle-1", "Be helpful.");
        link_agent_to_identity(&state, "agent-1", "acct-mine");

        let session_dir = tempfile::tempdir().unwrap();
        let oversized_path = session_dir.path().join("sess-huge.jsonl");
        {
            // Sparse file -- claims MAX_HISTORY_SESSION_FILE_SIZE_BYTES + 1
            // bytes via metadata without actually writing/allocating
            // that much, matching read_abf_file_path's own sparse-file
            // test technique above.
            let file = std::fs::File::create(&oversized_path).unwrap();
            file.set_len(MAX_HISTORY_SESSION_FILE_SIZE_BYTES + 1).unwrap();
        }

        let index = SessionIndex::with_isolated_roots(
            vec![Box::new(MockAdapter {
                files: vec![DiscoveredFile { file_path: oversized_path.to_string_lossy().into_owned(), mtime_ms: 1 }],
                identity_id: "acct-mine".to_string(),
            })],
            vec![session_dir.path().to_path_buf()],
        );
        let history_service = HistoryService::from_index(index);

        let result = bundle_export_for_agent_with_history_impl(
            state.id_store.clone(),
            state.identity_store.clone(),
            &state.mstore,
            std::sync::Arc::new(history_service),
            ExportForAgentWithHistoryReq { bundle_id: "bundle-1".to_string(), agent_id: "agent-1".to_string() },
        )
        .await
        .unwrap();

        assert_eq!(result["history_session_count"], 0, "oversized session must not count as included");
        let paths = unzip_paths(result["zip_base64"].as_str().unwrap());
        assert!(!paths.iter().any(|p| p.contains("sess-huge")), "oversized session must not be in the zip: {paths:?}");
        let warnings = result["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap_or("").contains("exceeds") && w.as_str().unwrap_or("").contains("sess-huge")),
            "must warn about the size limit: {warnings:?}"
        );
    }

    // reagentx P1, third review round: a per-session cap alone doesn't
    // bound an agent with many sessions each just under it. These
    // exercise the pure sessions_within_total_budget decision directly
    // with tiny numbers -- MAX_TOTAL_HISTORY_BYTES is 500 MiB, real
    // enough files to trigger it would make this test slow and wasteful
    // for no extra confidence over testing the same logic with small
    // inputs.
    #[test]
    fn total_budget_includes_everything_when_under_the_cap() {
        let (count, stopped) = sessions_within_total_budget(&[10, 10, 10], 100);
        assert_eq!(count, 3);
        assert!(!stopped);
    }

    #[test]
    fn total_budget_stops_at_the_first_session_that_would_exceed_the_cap() {
        // Most-recent-first order: [10, 10, 10] with cap 25 -- the
        // third one (cumulative 30) doesn't fit, so only the first two
        // (most recent) are included.
        let (count, stopped) = sessions_within_total_budget(&[10, 10, 10], 25);
        assert_eq!(count, 2, "must include the two most-recent sessions, not just any two that individually fit");
        assert!(stopped);
    }

    #[test]
    fn total_budget_includes_nothing_when_the_first_session_alone_exceeds_the_cap() {
        let (count, stopped) = sessions_within_total_budget(&[50, 10], 25);
        assert_eq!(count, 0);
        assert!(stopped);
    }

    #[test]
    fn total_budget_handles_an_empty_list() {
        let (count, stopped) = sessions_within_total_budget(&[], 100);
        assert_eq!(count, 0);
        assert!(!stopped);
    }

    #[test]
    fn total_budget_includes_a_session_that_exactly_fills_the_remaining_cap() {
        // 10 + 15 == 25 exactly -- must NOT be treated as exceeding.
        let (count, stopped) = sessions_within_total_budget(&[10, 15], 25);
        assert_eq!(count, 2);
        assert!(!stopped);
    }
}
