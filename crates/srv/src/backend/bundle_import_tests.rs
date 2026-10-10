// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of `bundle_import.rs`.

use super::*;

fn file(path: &str, content: &str) -> BundleImportFile {
    BundleImportFile { path: path.to_string(), content: content.to_string() }
}

fn minimal_manifest(components: Value) -> String {
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

#[test]
fn rejects_missing_armory_json() {
    let err = parse_bundle_import(&[]).unwrap_err();
    assert!(err.contains("armory.json"));
}

#[test]
fn rejects_malformed_armory_json() {
    let err = parse_bundle_import(&[file("armory.json", "not json")]).unwrap_err();
    assert!(err.contains("malformed JSON"));
}

#[test]
fn deduplicates_repeated_instruction_references_instead_of_cloning_content_per_reference() {
    // Codex P1, PR #2379 round 3: components.instructions is manifest-
    // controlled and its length isn't bounded by the decompression
    // caps -- those cap file CONTENT, not how many times a manifest
    // can reference the same path. An untrusted manifest repeating one
    // path thousands of times must not clone that content once per
    // repetition.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md", "instructions/AGENTS.md", "instructions/AGENTS.md"],
        }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Be concise.");
    // codex P1, PR #2379 round 6: one bounded summary warning instead
    // of one warning string per duplicate (its own amplification
    // vector -- see the round-6 tests below).
    assert!(result.warnings.iter().any(|w| w.contains("2 duplicate reference(s) skipped")));
}

#[test]
fn deduplicates_repeated_skill_and_mcp_references() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "skills": ["skills/deploy", "skills/deploy"],
            "mcpServers": ["mcp/server.json", "mcp/server.json"],
        }))),
        file("skills/deploy/SKILL.md", "---\nname: \"deploy\"\ndescription: \"d\"\n---\n\nbody"),
        file("mcp/server.json", r#"{"command":"npx","args":["-y","thing"]}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.skills.len(), 1);
    assert_eq!(result.mcp_servers.len(), 1);
    assert!(result.warnings.iter().any(|w| w.contains("components.skills: 1 duplicate reference(s) skipped")));
    assert!(result.warnings.iter().any(|w| w.contains("components.mcpServers: 1 duplicate reference(s) skipped")));
}

#[test]
fn imports_instructions_from_agents_md() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
        }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Be concise.");
    assert_eq!(result.name, "test-bundle");
    assert_eq!(result.description, "A test bundle");
    assert!(result.instructions_by_provider.is_empty());
}

#[test]
fn imports_v02_keyed_object_shape_with_default_and_provider_variants() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": {
                "default": ["instructions/AGENTS.md"],
                "claude": ["instructions/claude/AGENTS.md"],
                "codex": ["instructions/codex/AGENTS.md"],
            },
        }))),
        file("instructions/AGENTS.md", "Be concise."),
        file("instructions/claude/AGENTS.md", "Claude-specific."),
        file("instructions/codex/AGENTS.md", "Codex-specific."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Be concise.");
    assert_eq!(result.instructions_by_provider.get("claude"), Some(&"Claude-specific.".to_string()));
    assert_eq!(result.instructions_by_provider.get("codex"), Some(&"Codex-specific.".to_string()));
    assert_eq!(result.instructions_by_provider.len(), 2);
}

#[test]
fn imports_v02_keyed_object_shape_with_no_default_key() {
    // A bundle that only ever defines provider-specific content, no
    // shared default — must not crash or silently invent a "default".
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": { "claude": ["instructions/claude/AGENTS.md"] },
        }))),
        file("instructions/claude/AGENTS.md", "Claude-only."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "");
    assert_eq!(result.instructions_by_provider.get("claude"), Some(&"Claude-only.".to_string()));
}

#[test]
fn a_context_file_referenced_from_default_and_a_provider_variant_is_deduped_across_both() {
    // reagent P1, PR #2523: dedup used to be local to each
    // components.instructions array, so the same context-file path
    // referenced from both "default" and a provider variant was
    // pushed into context_files TWICE. Must dedupe across the whole
    // components.instructions parse, not per-array.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": {
                "default": ["instructions/AGENTS.md", "instructions/context/shared.md"],
                "claude": ["instructions/claude/AGENTS.md", "instructions/context/shared.md"],
            },
        }))),
        file("instructions/AGENTS.md", "Default."),
        file("instructions/claude/AGENTS.md", "Claude."),
        file("instructions/context/shared.md", "Shared context."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.context_files.len(), 1, "the shared context file must appear exactly once, not once per referencing variant");
    assert_eq!(result.context_files[0].content, "Shared context.");
    assert!(result.warnings.iter().any(|w| w.contains("duplicate reference")));
}

#[test]
fn a_provider_named_context_is_not_misrouted_into_context_files() {
    // reagent P1, PR #2523: the instructions/context/ prefix check
    // applied to every path uniformly regardless of which provider
    // array was being parsed, so a provider literally named "context"
    // — whose exported path is instructions/context/AGENTS.md, per
    // bundle_export.rs's instructions/<provider>/AGENTS.md convention
    // — had its content silently misrouted into context_files instead
    // of instructions_by_provider["context"].
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": {
                "default": ["instructions/AGENTS.md"],
                "context": ["instructions/context/AGENTS.md"],
            },
        }))),
        file("instructions/AGENTS.md", "Default."),
        file("instructions/context/AGENTS.md", "For the 'context' provider."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(
        result.instructions_by_provider.get("context"),
        Some(&"For the 'context' provider.".to_string()),
        "a provider literally named 'context' must land in instructions_by_provider, not context_files"
    );
    assert!(result.context_files.is_empty());
}

#[test]
fn a_non_array_provider_variant_warns_and_is_skipped() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": { "default": ["instructions/AGENTS.md"], "claude": "not-an-array" },
        }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Be concise.");
    assert!(result.instructions_by_provider.is_empty());
    assert!(result.warnings.iter().any(|w| w.contains("instructions.claude") && w.contains("expected an array")));
}

#[test]
fn caps_the_number_of_instruction_provider_variants() {
    let mut instructions = serde_json::Map::new();
    for i in 0..MAX_INSTRUCTION_PROVIDER_VARIANTS + 5 {
        instructions.insert(format!("provider-{i}"), serde_json::json!([]));
    }
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": instructions }))),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("provider variants exceeds the limit")));
}

#[test]
fn project_instructions_are_surfaced_and_never_installed() {
    // Phase 3 §5.2. These entries describe what the SOURCE agent read.
    // The files they name belong to whatever repository this bundle is
    // being imported into, so no import path may write them — unlike
    // memory, there is no agent-scoped variant that does.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
            "projectInstructions": [{
                "path": "CLAUDE.md",
                "file": "instructions/project/CLAUDE.md",
                "contentHash": "abc123",
                "owner": "foreign",
            }],
        }))),
        file("instructions/AGENTS.md", "Be concise."),
        file("instructions/project/CLAUDE.md", "someone else's house rules"),
    ];
    let result = parse_bundle_import(&files).unwrap();

    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("components.projectInstructions") && w.contains("never installed")),
        "the import must say it is not applying them: {:?}",
        result.warnings
    );
    // And nothing about them reaches the bundle that gets written.
    assert!(
        !result.instructions.contains("house rules"),
        "another repository's instructions must not become this bundle's"
    );
    assert!(
        !result.context_files.iter().any(|cf| cf.content.contains("house rules")),
        "nor arrive as a context file"
    );
}

#[test]
fn a_memory_component_is_warned_about_and_ignored_by_the_agent_less_parser() {
    // ABF v0.2 §2.3: parse_bundle_import (shared by bundle.import and
    // bundle.import.preview/.commit, all agent-less) must not silently
    // drop a components.memory key — it needs bundle.import_for_agent
    // instead, and the response should say so.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
            "memory": ["memory/MEMORY.md"],
        }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("components.memory") && w.contains("bundle.import_for_agent")));
}

#[test]
fn no_memory_warning_when_the_component_is_absent() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
        }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(!result.warnings.iter().any(|w| w.contains("components.memory")));
}

fn manifest_with_project_instructions(entries: Value) -> Vec<BundleImportFile> {
    vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
            "projectInstructions": entries,
        }))),
        file("instructions/AGENTS.md", "Be concise."),
        file("instructions/project/CLAUDE.md", "# House rules\n"),
    ]
}

#[test]
fn project_instructions_are_readable_after_import() {
    // Codex P1, PR #3163: the warning alone told you a component existed
    // and nothing else, and `agent.project_instructions` can't fill the
    // gap — it scans the IMPORTING agent's working directory, not the
    // source machine's. Reading is not applying.
    let files = manifest_with_project_instructions(serde_json::json!([{
        "path": "CLAUDE.md",
        "file": "instructions/project/CLAUDE.md",
        "contentHash": "abc123",
        "owner": "foreign",
    }]));
    let result = parse_bundle_import(&files).unwrap();

    assert_eq!(result.project_instructions.len(), 1);
    let entry = &result.project_instructions[0];
    assert_eq!(entry.path, "CLAUDE.md");
    assert_eq!(entry.owner, "foreign");
    assert_eq!(entry.content_hash, "abc123");
    assert!(entry.content.contains("House rules"), "the content must be readable");

    // Readable, still not applied: the warning stays, and nothing about
    // this entry leaks into the fields an import actually writes.
    assert!(result
        .warnings
        .iter()
        .any(|w| w == PROJECT_INSTRUCTIONS_NOT_APPLIED_WARNING));
    assert!(
        !result.instructions.contains("House rules"),
        "a project instruction must never become the bundle's own instructions"
    );
    assert!(
        !result.context_files.iter().any(|cf| cf.content.contains("House rules")),
        "nor a context file"
    );
}

#[test]
fn project_instruction_entries_are_capped_and_report_the_overflow() {
    let entries: Vec<Value> = (0..MAX_IMPORTED_PROJECT_INSTRUCTIONS + 5)
        .map(|i| serde_json::json!({
            "path": format!("CLAUDE{i}.md"),
            "file": "instructions/project/CLAUDE.md",
            "owner": "foreign",
        }))
        .collect();
    let files = manifest_with_project_instructions(serde_json::json!(entries));
    let result = parse_bundle_import(&files).unwrap();

    assert_eq!(result.project_instructions.len(), MAX_IMPORTED_PROJECT_INSTRUCTIONS);
    assert!(
        result.warnings.iter().any(|w| w.contains("exceeds the")),
        "truncating a list silently is what the rest of this parser refuses to do: {:?}",
        result.warnings
    );
}

#[test]
fn a_project_instruction_cannot_read_the_accounts_requirements_file() {
    // The same allowlist components.instructions is held to: a manifest is
    // not a trustworthy inventory of its own bundle, so this must not
    // become a second route to read accounts/requirements.json back out.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "projectInstructions": [{
                "path": "CLAUDE.md",
                "file": "accounts/requirements.json",
                "owner": "foreign",
            }],
        }))),
        file("accounts/requirements.json", "[{\"id\":\"x\",\"credentialProvider\":\"y\"}]"),
    ];
    let result = parse_bundle_import(&files).unwrap();

    assert!(result.project_instructions.is_empty());
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("projectInstructions") && w.contains("requirements file")));
}

#[test]
fn a_non_canonical_project_instruction_reference_still_resolves() {
    // reagent P2, PR #3163 — the third component to need this (codex P2,
    // PR #2379 round 4 for instructions; reagent P2 round 6 for
    // accounts/requirements.json). A bundle not produced by this exact
    // exporter can spell the reference differently; reporting it "not
    // found" when the content is present defeats the whole point.
    let files = manifest_with_project_instructions(serde_json::json!([{
        "path": "CLAUDE.md",
        "file": ".\\instructions\\project\\.\\CLAUDE.md",
        "owner": "foreign",
    }]));
    let result = parse_bundle_import(&files).unwrap();

    assert_eq!(
        result.project_instructions.len(),
        1,
        "a non-canonical spelling must resolve, got warnings: {:?}",
        result.warnings
    );
    assert_eq!(result.project_instructions[0].file, "instructions/project/CLAUDE.md");
    assert!(result.project_instructions[0].content.contains("House rules"));
}

#[test]
fn a_non_canonical_accounts_reference_is_still_rejected_from_project_instructions() {
    // The allowlist is checked on the normalized path, so normalizing the
    // reference above can't become a way around it.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "projectInstructions": [{
                "path": "CLAUDE.md",
                "file": "./accounts/./requirements.json",
                "owner": "foreign",
            }],
        }))),
        file("accounts/requirements.json", "{\"requirements\":[]}"),
    ];
    let result = parse_bundle_import(&files).unwrap();

    assert!(result.project_instructions.is_empty());
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("projectInstructions") && w.contains("requirements file")));
}

#[test]
fn a_project_instruction_referencing_a_missing_file_warns_rather_than_vanishing() {
    let files = vec![file("armory.json", &minimal_manifest(serde_json::json!({
        "projectInstructions": [{
            "path": "CLAUDE.md",
            "file": "instructions/project/GONE.md",
            "owner": "foreign",
        }],
    })))];
    let result = parse_bundle_import(&files).unwrap();

    assert!(result.project_instructions.is_empty());
    assert!(result
        .warnings
        .iter()
        .any(|w| w.contains("GONE.md") && w.contains("not found")));
}

#[test]
fn imports_context_files_separately_from_instructions() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md", "instructions/context/notes.md"],
        }))),
        file("instructions/AGENTS.md", "Main instructions."),
        file("instructions/context/notes.md", "Extra context."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Main instructions.");
    assert_eq!(result.context_files, vec![ImportedContextFile {
        id: 0,
        path: "notes.md".to_string(),
        content: "Extra context.".to_string(),
    }]);
}

#[test]
fn warns_and_skips_a_missing_referenced_file_rather_than_failing() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
        }))),
        // instructions/AGENTS.md deliberately absent
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "");
    assert!(result.warnings.iter().any(|w| w.contains("not found")));
}

#[test]
fn imports_a_skill_round_tripped_through_render_skill_md() {
    let rendered = super::super::agent_config::render_skill_md(
        "deploy-checklist",
        "Runs the checklist",
        "1. Test\n2. Deploy",
    );
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "skills": ["skills/deploy-checklist"],
        }))),
        file("skills/deploy-checklist/SKILL.md", &rendered),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.skills.len(), 1);
    assert_eq!(result.skills[0].source_dir, "skills/deploy-checklist");
    assert_eq!(result.skills[0].slug, "deploy-checklist");
    assert_eq!(result.skills[0].description, "Runs the checklist");
    assert_eq!(result.skills[0].content, "1. Test\n2. Deploy");
    assert!(result.skipped_skills.is_empty());
}

#[test]
fn skips_a_skill_whose_skill_md_is_missing() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "skills": ["skills/ghost"],
        }))),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.skills.is_empty());
    assert_eq!(result.skipped_skills, vec!["skills/ghost".to_string()]);
}

#[test]
fn skips_a_skill_with_malformed_frontmatter() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "skills": ["skills/broken"],
        }))),
        file("skills/broken/SKILL.md", "not frontmatter at all"),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.skills.is_empty());
    assert_eq!(result.skipped_skills, vec!["skills/broken".to_string()]);
    assert!(result.warnings.iter().any(|w| w.contains("malformed SKILL.md")));
}

#[test]
fn imports_mcp_servers_still_containing_placeholders() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "mcpServers": ["mcp/github.server.json"],
        }))),
        file("mcp/github.server.json", r#"{"type":"stdio","command":"gh-mcp","env":{"GITHUB_TOKEN":"${GITHUB_TOKEN}"}}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.mcp_servers.len(), 1);
    assert_eq!(result.mcp_servers[0].source_path, "mcp/github.server.json");
    assert_eq!(result.mcp_servers[0].config["env"]["GITHUB_TOKEN"], "${GITHUB_TOKEN}");
}

#[test]
fn parses_account_requirements_read_only() {
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "accounts": "accounts/requirements.json",
        }))),
        file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","credentialProvider":"github","kind":"api-key","env":"GITHUB_TOKEN","optional":false}]}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.requirements, vec![AccountRequirement {
        id: "gh-main".to_string(),
        provider: "github".to_string(),
        kind: "api-key".to_string(),
        env: "GITHUB_TOKEN".to_string(),
        optional: false,
    }]);
}

#[test]
fn a_v01_bundle_still_carrying_the_old_provider_key_still_imports() {
    // ABF v0.2, §2.1: "provider" was renamed to "credentialProvider" on
    // export, but a bundle exported by a v0.1 build (or hand-authored
    // against the old spec) still uses the old key — must not break.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "accounts": "accounts/requirements.json",
        }))),
        file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","provider":"github","kind":"api-key","env":"GITHUB_TOKEN","optional":false}]}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.requirements, vec![AccountRequirement {
        id: "gh-main".to_string(),
        provider: "github".to_string(),
        kind: "api-key".to_string(),
        env: "GITHUB_TOKEN".to_string(),
        optional: false,
    }]);
}

#[test]
fn rejects_any_accounts_file_other_than_requirements_json() {
    // §4.3.5 / the report's original invariant: never read anything
    // else under accounts/, even if present and well-formed.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({}))),
        file("accounts/secrets.json", r#"{"github_token":"ghp_should_never_be_read"}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("accounts/secrets.json") && w.contains("rejected")));
}

#[test]
fn a_rejected_accounts_file_referenced_by_a_component_is_never_surfaced_in_the_result() {
    // Codex P1, PR #2379: a rejected accounts/ file must be excluded
    // from lookups entirely, not merely warned about — otherwise a
    // malicious bundle whose components.instructions REFERENCES
    // accounts/secrets.json would still get that content folded into
    // the imported bundle's instructions, defeating the whole
    // accounts/ allowlist. This is the exact attack shape: point a
    // legitimate-looking component path at the rejected file.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["accounts/secrets.json"],
        }))),
        file("accounts/secrets.json", "GITHUB_TOKEN=ghp_should_never_leak_into_a_bundle"),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(
        !result.instructions.contains("ghp_should_never_leak_into_a_bundle"),
        "rejected accounts/ content leaked into instructions: {:?}",
        result.instructions
    );
    assert!(result.warnings.iter().any(|w| w.contains("accounts/secrets.json") && w.contains("rejected")));
    // The reference itself now resolves to "not found" (the file was
    // excluded from lookups), which is the correct, safe outcome —
    // not a crash, not a silent success with leaked content.
    assert!(result.warnings.iter().any(|w| w.contains("accounts/secrets.json") && w.contains("not found")));
}

#[test]
fn rejects_an_other_case_accounts_path_the_same_as_the_canonical_lowercase_one() {
    // reagent P1, PR #2379 round 4: sanitize_context_relative_path
    // never case-folds, so a literal lowercase `starts_with("accounts/")`
    // check let `ACCOUNTS/secrets.json` sail through unrejected.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["ACCOUNTS/secrets.json"],
        }))),
        file("ACCOUNTS/secrets.json", "GITHUB_TOKEN=ghp_should_never_leak_case_variant"),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(!result.instructions.contains("ghp_should_never_leak_case_variant"));
    assert!(result.warnings.iter().any(|w| w.contains("ACCOUNTS/secrets.json") && w.contains("rejected")));
}

#[test]
fn resolves_a_non_canonical_manifest_reference_against_the_normalized_file_path() {
    // codex P2, PR #2379 round 4: by_path's keys are normalized (round
    // 4's earlier accounts/ fix), so a manifest reference using a
    // valid but non-canonical spelling of the SAME path must be
    // normalized identically before the lookup, or a genuinely present
    // file is reported "not found".
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["./instructions/AGENTS.md"],
        }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Be concise.");
    assert!(result.warnings.iter().all(|w| !w.contains("not found")), "unexpected warnings: {:?}", result.warnings);
}

#[test]
fn caps_the_number_of_account_requirements_accepted_from_a_single_bundle() {
    // codex P1, PR #2379 round 4: an unbounded requirements array lets
    // a small (well within the per-entry size cap) accounts/requirements.json
    // drive an unbounded number of downstream per-provider account
    // lookups in the RPC handler.
    let many: Vec<_> = (0..MAX_ACCOUNT_REQUIREMENTS + 50)
        .map(|i| serde_json::json!({
            "id": format!("req-{i}"), "provider": "github", "kind": "api-key",
            "env": "GITHUB_TOKEN", "optional": false,
        }))
        .collect();
    let requirements_doc = serde_json::json!({ "requirements": many }).to_string();
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "accounts": "accounts/requirements.json",
        }))),
        file("accounts/requirements.json", &requirements_doc),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.requirements.len(), MAX_ACCOUNT_REQUIREMENTS);
    assert!(result.warnings.iter().any(|w| w.contains("exceeds the limit")));
}

#[test]
fn resolves_a_non_canonical_components_accounts_reference() {
    // reagent P2, PR #2379 round 6: components.accounts was the one
    // remaining manifest-reference lookup still comparing the raw,
    // un-normalized string -- the same bug class round 4 fixed for
    // instructions/skills/mcpServers.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "accounts": "./accounts/requirements.json",
        }))),
        file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","provider":"github","kind":"api-key","env":"GITHUB_TOKEN","optional":false}]}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.requirements.len(), 1);
    assert!(result.warnings.iter().all(|w| !w.contains("is not accounts/requirements.json")));
}

#[test]
fn requirements_json_is_not_readable_through_components_instructions_or_mcp_servers() {
    // codex P1, PR #2379 round 6: accounts/requirements.json is
    // intentionally in by_path for the dedicated parser above, but
    // must never be reachable through the generic component lookups
    // -- otherwise its raw content leaks into instructions (and later
    // an export's AGENTS.md) or an mcpServers entry.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["accounts/requirements.json"],
            "mcpServers": ["accounts/requirements.json"],
        }))),
        file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","provider":"github","kind":"api-key","env":"GITHUB_TOKEN_SECRET_LEAK","optional":false}]}"#),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(!result.instructions.contains("GITHUB_TOKEN_SECRET_LEAK"));
    assert!(result.mcp_servers.is_empty());
    assert!(result.warnings.iter().any(|w| w.contains("components.instructions") && w.contains("not readable as instructions")));
    assert!(result.warnings.iter().any(|w| w.contains("components.mcpServers") && w.contains("not readable as an MCP server config")));
}

#[test]
fn duplicate_reference_warnings_are_bounded_to_one_summary_per_component_category() {
    // codex P1, PR #2379 round 6: dedup already avoids cloning
    // CONTENT per duplicate (round 3), but pushing one warning STRING
    // per duplicate was itself unbounded -- a permitted 10 MB
    // manifest repeating one short path hundreds of thousands of
    // times could allocate hundreds of megabytes of warning text.
    let many_dupes: Vec<Value> = (0..1_000).map(|_| serde_json::json!("instructions/AGENTS.md")).collect();
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_dupes }))),
        file("instructions/AGENTS.md", "Be concise."),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "Be concise.");
    assert_eq!(
        result.warnings.iter().filter(|w| w.contains("duplicate reference(s) skipped")).count(),
        1,
        "expected exactly one summary warning, got: {:?}",
        result.warnings
    );
    assert!(result.warnings.iter().any(|w| w.contains("999 duplicate reference(s) skipped")));
}

#[test]
fn unzip_counts_bytes_read_before_a_decompression_error_toward_the_aggregate() {
    // codex P1, PR #2379 round 6: read_to_end can leave real
    // decompressed bytes in `buf` even when it ultimately returns an
    // error (e.g. a forged CRC on an otherwise-valid entry) -- that
    // decompression work happened regardless, and must count.
    // Directly exercises the ordering fix (check_entry_size runs
    // before the read-error branch) since forging a CRC mismatch
    // through the public ZipWriter API isn't practical.
    let mut total: u64 = 0;
    let mut warnings = WarningSink::new(WarningBudget::unbounded());
    // Simulates: read_to_end returned Err, but buf still holds
    // MAX_ENTRY_UNCOMPRESSED_BYTES bytes of real decompressed data.
    let keep = check_entry_size("corrupt.md", MAX_ENTRY_UNCOMPRESSED_BYTES, &mut total, &mut warnings);
    assert_eq!(keep, Ok(true), "bytes read before the error must still count");
    assert_eq!(total, MAX_ENTRY_UNCOMPRESSED_BYTES);
}

#[test]
fn components_accounts_non_string_value_gets_an_explicit_warning() {
    // reagent P2, PR #2379 round 7: every other component category
    // warns on a malformed value; this one used to silently drop it.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "accounts": ["not", "a", "string"],
        }))),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.requirements.is_empty());
    assert!(result.warnings.iter().any(|w| w.contains("components.accounts") && w.contains("non-string value")));
}

#[test]
fn caps_malformed_component_entries_before_they_can_amplify_into_unbounded_warnings() {
    // codex P1, PR #2379 round 7: round 6 bounded the "duplicate
    // reference" warning specifically; a manifest filled with
    // non-string junk hits a DIFFERENT (still unbounded, until this
    // fix) warning path for the same amplification effect.
    // capped_component_array truncates the array itself before any
    // per-entry processing, so this must produce at most one
    // "exceeds the limit" warning plus MAX_ENTRY_COUNT "non-string
    // entry" warnings -- not one per array element.
    let many_junk: Vec<Value> = (0..MAX_ENTRY_COUNT + 500).map(|i| serde_json::json!(i)).collect();
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_junk }))),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("components.instructions") && w.contains("exceeds the limit")));
    let non_string_warnings = result.warnings.iter().filter(|w| w.contains("non-string entry skipped")).count();
    assert_eq!(non_string_warnings, MAX_ENTRY_COUNT, "expected exactly the capped count, got {non_string_warnings}");
}

#[test]
fn caps_the_number_of_skills_actually_imported_from_a_single_bundle() {
    // codex P1, PR #2379 round 7: MAX_ENTRY_COUNT bounds how many
    // components.skills entries are even looked at, but that's still
    // far too high a ceiling for the RPC handler's write side --
    // each imported skill becomes a separate synchronous Store
    // transaction creating a permanent global row.
    let n = MAX_IMPORTED_SKILLS + 50;
    let mut manifest_skills: Vec<Value> = Vec::new();
    let mut skill_files: Vec<BundleImportFile> = Vec::new();
    for i in 0..n {
        let dir = format!("skills/s{i}");
        manifest_skills.push(serde_json::json!(dir));
        skill_files.push(file(
            &format!("{dir}/SKILL.md"),
            &format!("---\nname: \"s{i}\"\ndescription: \"d\"\n---\n\nbody"),
        ));
    }
    let mut files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({ "skills": manifest_skills }))),
    ];
    files.extend(skill_files);
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.skills.len(), MAX_IMPORTED_SKILLS);
    assert_eq!(result.skipped_skills.len(), 50);
    assert!(result.warnings.iter().any(|w| w.contains("exceeds the import limit")));
}

#[test]
fn reads_every_abf_version_and_its_hint() {
    let v01 = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
        "name": "a", "version": "0.1.0", "provider": "claude", "model": "anthropic",
        "components": { "instructions": ["instructions/AGENTS.md"] },
    });
    let v02 = serde_json::json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
        "name": "a", "version": "0.1.0", "provider": "codex", "model": null,
        "components": { "instructions": { "default": ["instructions/AGENTS.md"] } },
    });
    let v03 = serde_json::json!({
        "$schema": SCHEMA_V0_3,
        "name": "a", "version": "3.1.0", "suggestedFor": { "provider": "gemini", "vendor": "google" },
        "components": { "instructions": { "default": ["instructions/AGENTS.md"] } },
    });
    for (manifest_name, manifest, version, provider, model) in [
        (LEGACY_MANIFEST_FILE, v01, "0.1", "claude", "anthropic"),
        (LEGACY_MANIFEST_FILE, v02, "0.2", "codex", ""),
        (MANIFEST_FILE, v03, "0.3", "gemini", "google"),
    ] {
        assert_eq!(abf_format_version(&manifest), Some(version));
        let files = vec![
            file(manifest_name, &manifest.to_string()),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let r = parse_bundle_import(&files).unwrap();
        assert_eq!((r.provider.as_str(), r.model.as_str()), (provider, model), "v{version}");
        assert_eq!(r.instructions.trim(), "Be concise.", "v{version}");
        assert!(r.warnings.is_empty(), "v{version}: {:?}", r.warnings);
    }
}

#[test]
fn bundle_json_wins_over_armory_json_with_a_warning() {
    let manifest = |name: &str| serde_json::json!({ "$schema": SCHEMA_V0_3, "name": name, "components": {} }).to_string();
    let r = parse_bundle_import(&[file(LEGACY_MANIFEST_FILE, &manifest("old")), file(MANIFEST_FILE, &manifest("new"))]).unwrap();
    assert_eq!(r.name, "new");
    assert!(r.warnings.iter().any(|w| w.contains("armory.json: ignored")), "{:?}", r.warnings);
}

#[test]
fn a_bundle_with_no_manifest_names_both_files() {
    let err = parse_bundle_import(&[file("instructions/AGENTS.md", "x")]).unwrap_err();
    assert!(err.contains("bundle.json") && err.contains("armory.json"), "{err}");
}

#[test]
fn the_content_version_never_warns() {
    // `version` is the bundle's own; an exporter's "0.1.0" bumped to
    // "2.0.0" by its author is not a format problem.
    for version in ["0.1", "0.1.0", "0.10.0", "2.0.0", "not semver"] {
        let manifest = serde_json::to_string(&serde_json::json!({
            "$schema": SCHEMA_V0_3,
            "name": "test-bundle",
            "version": version,
            "components": {},
        }))
        .unwrap();
        let result = parse_bundle_import(&[file(MANIFEST_FILE, &manifest)]).unwrap();
        assert!(result.warnings.is_empty(), "{version:?}: {:?}", result.warnings);
    }
}

#[test]
fn a_raw_files_duplicate_path_resolves_to_the_first_occurrence_with_a_warning() {
    // reagent P2, PR #2379 round 5: unzip_bundle_import already warns
    // on a duplicate zip entry; the raw files list reaches by_path
    // construction directly, bypassing that pass, so it previously
    // silently resolved last-write-wins with no warning at all.
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
        }))),
        file("instructions/AGENTS.md", "first"),
        file("instructions/AGENTS.md", "second -- should never be used"),
    ];
    let result = parse_bundle_import(&files).unwrap();
    assert_eq!(result.instructions, "first");
    assert!(result.warnings.iter().any(|w| w.contains("instructions/AGENTS.md") && w.contains("duplicate path")));
}

// ── raw `files` intake path now shares the zip path's size/count caps
// (reagent P1, PR #2379 round 5) ──────────────────────────────────

#[test]
fn enforce_raw_files_caps_rejects_more_entries_than_the_count_cap() {
    let files: Vec<BundleImportFile> = (0..=MAX_ENTRY_COUNT)
        .map(|i| file(&format!("f{i}.md"), "x"))
        .collect();
    let err = enforce_raw_files_caps(files).unwrap_err();
    assert!(err.contains("entries exceeds the limit"));
}

#[test]
fn enforce_raw_files_caps_skips_a_single_oversized_entry_with_a_warning() {
    let oversized = "a".repeat(MAX_ENTRY_UNCOMPRESSED_BYTES as usize + 1);
    let files = vec![file("armory.json", "{}"), file("big.md", &oversized)];
    let (kept, warnings) = enforce_raw_files_caps(files).unwrap();
    assert!(kept.iter().any(|f| f.path == "armory.json"));
    assert!(!kept.iter().any(|f| f.path == "big.md"));
    assert!(warnings.iter().any(|w| w.contains("big.md") && w.contains("exceeds the per-entry limit")));
}

#[test]
fn enforce_raw_files_caps_rejects_the_whole_import_over_the_aggregate_cap() {
    let each = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 0.9) as usize);
    let files = vec![
        file("a.md", &each), file("b.md", &each), file("c.md", &each),
        file("d.md", &each), file("e.md", &each), file("f.md", &each),
    ];
    let err = enforce_raw_files_caps(files).unwrap_err();
    assert!(err.contains("aggregate decompressed size exceeds"));
}

#[test]
fn enforce_raw_files_caps_counts_a_discarded_oversized_entrys_bytes_toward_the_aggregate() {
    // codex P1, PR #2379 round 5: an entry too large to KEEP must
    // still count toward the aggregate budget -- otherwise many such
    // entries can each force per-entry-cap-sized work while never
    // tripping the aggregate limit. Five entries just over the
    // per-entry cap (~10.5MB each) sum to ~52.5MB > the 50MB
    // aggregate cap, even though every single one is discarded rather
    // than kept.
    let oversized = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 1.05) as usize);
    let files = vec![
        file("a.md", &oversized), file("b.md", &oversized), file("c.md", &oversized),
        file("d.md", &oversized), file("e.md", &oversized),
    ];
    let err = enforce_raw_files_caps(files).unwrap_err();
    assert!(err.contains("aggregate decompressed size exceeds"));
}

#[test]
fn check_entry_size_accounts_bytes_toward_the_aggregate_even_when_the_entry_is_discarded() {
    // codex P1, PR #2379 round 5: directly exercises the shared
    // choke point both unzip_bundle_import (after its backstop read)
    // and enforce_raw_files_caps call -- an entry too large to KEEP
    // must still count toward the aggregate budget first, or many
    // discarded oversized entries could each force per-entry-cap-
    // sized work while the aggregate cap never trips. (A zip-level
    // reproduction would need to forge a declared size that
    // UNDERSTATES real content while still reaching this function --
    // the `zip` crate reads declared size from the central directory,
    // not the local file header this module can cheaply patch, so
    // the invariant is verified directly at its actual implementation
    // site instead.)
    let mut total: u64 = 0;
    let mut warnings = WarningSink::new(WarningBudget::unbounded());
    let oversized = MAX_ENTRY_UNCOMPRESSED_BYTES + 1;
    for i in 0..5 {
        let name = format!("f{i}.md");
        let result = check_entry_size(&name, oversized, &mut total, &mut warnings);
        if i < 4 {
            assert_eq!(result, Ok(false), "entry {i} should be discarded (not an error) with total={total}");
        } else {
            assert!(result.is_err(), "5th entry should trip the aggregate cap now that all 5 discarded entries' bytes were counted (total={total})");
        }
    }
    assert!(warnings.into_vec().iter().filter(|w| w.contains("exceeds the per-entry limit")).count() >= 4);
}

#[test]
fn bounds_an_oversized_manifest_name_at_parse_time() {
    // Phase 3 spec §3.1, round 13: name must be bounded at the parse
    // source (not at a later response boundary) so preview and commit
    // -- which independently re-parse -- always converge on the exact
    // same canonical value.
    let oversized_name = "n".repeat(MAX_BUNDLE_NAME_CHARS + 50);
    let manifest = serde_json::to_string(&serde_json::json!({
        "name": oversized_name,
        "version": "0.1.0",
        "components": {},
    })).unwrap();
    let files = vec![file("armory.json", &manifest)];
    let result_a = parse_bundle_import(&files).unwrap();
    let result_b = parse_bundle_import(&files).unwrap();
    assert_eq!(result_a.name.chars().count(), MAX_BUNDLE_NAME_CHARS);
    assert_eq!(result_a.name, result_b.name, "two independent parses of the same bytes must converge on the identical truncated name");
    assert!(result_a.warnings.iter().any(|w| w.contains("name exceeds") && w.contains("truncated")));
}

#[test]
fn warns_on_an_unknown_schema_but_does_not_fail() {
    let manifest = serde_json::to_string(&serde_json::json!({
        "$schema": "https://example.com/some-other-format/v9/bundle.schema.json",
        "name": "test-bundle",
        "components": {},
    }))
    .unwrap();
    let result = parse_bundle_import(&[file(MANIFEST_FILE, &manifest)]).unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("$schema names no ABF version")));
}

#[test]
fn parse_skill_md_round_trips_special_characters() {
    let rendered = super::super::agent_config::render_skill_md(
        "weird-name",
        "Has a colon: and \"quotes\"",
        "body\nwith\nnewlines",
    );
    let parsed = parse_skill_md(&rendered).expect("parses");
    assert_eq!(parsed.slug, "weird-name");
    assert_eq!(parsed.description, "Has a colon: and \"quotes\"");
    assert_eq!(parsed.content, "body\nwith\nnewlines");
}

#[test]
fn parse_skill_md_rejects_non_matching_shape() {
    assert!(parse_skill_md("not frontmatter at all").is_none());
    assert!(parse_skill_md("---\nname: \"x\"\n---\n\nbody").is_none()); // missing description
    assert!(parse_skill_md("").is_none());
}

// ── zip round-trip ──────────────────────────────────────────────

#[test]
fn unzips_a_bundle_exported_as_zip() {
    use crate::backend::storage::store::Bundle;

    let bundle = Bundle {
        id: "b1".to_string(),
        name: "Zip Roundtrip".to_string(),
        description: "desc".to_string(),
        is_blank: false,
        is_global: false,
        provider: String::new(),
        model: String::new(),
        instructions: "Be helpful.".to_string(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        created_at: 0,
        updated_at: 0,
        is_system: false,
    };
    let export = super::super::bundle_export::export_bundle(&bundle, &[]);
    let zip_bytes = super::super::bundle_export::zip_bundle_export(&export).unwrap();

    let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(warnings.is_empty());
    assert!(files.iter().any(|f| f.path == "bundle.json"));
    assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md" && f.content == "Be helpful."));

    let parsed = parse_bundle_import(&files).unwrap();
    assert_eq!(parsed.instructions, "Be helpful.");
}

#[test]
fn unzip_rejects_a_non_zip_input() {
    let err = unzip_bundle_import(b"not a zip file").unwrap_err();
    assert!(err.contains("not a valid zip archive"));
}

/// Build a zip archive directly from `(path, content)` pairs — no
/// `bundle_export` involvement, so these tests cover a hand-built
/// `.abf` independent of the exporter's own wrapping convention.
fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (path, content) in entries {
        writer.start_file(*path, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    buf.into_inner()
}

#[test]
fn unzips_a_flat_bundle_with_no_wrapping_directory() {
    // reagent P1, PR #2379: the spec's §2 explicitly requires an
    // unwrapped directory tree (zipped directly, no bundle-slug
    // folder) to round-trip just like the exporter's own wrapped
    // form. Nested paths must survive UNCHANGED — no stripping.
    let zip_bytes = build_zip(&[
        ("armory.json", "{}"),
        ("instructions/AGENTS.md", "Be concise."),
        ("skills/deploy/SKILL.md", "---\nname: \"deploy\"\ndescription: \"d\"\n---\n\nbody"),
    ]);
    let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(warnings.is_empty());
    assert!(files.iter().any(|f| f.path == "armory.json"));
    assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md" && f.content == "Be concise."));
    assert!(files.iter().any(|f| f.path == "skills/deploy/SKILL.md"));
}

#[test]
fn unzips_a_wrapped_bundle_with_a_directory_other_than_the_exporters_own_slug() {
    // The wrapper-detection must work for ANY consistent single root
    // name, not just names zip_bundle_export happens to produce.
    let zip_bytes = build_zip(&[
        ("my-hand-built-bundle/armory.json", "{}"),
        ("my-hand-built-bundle/instructions/AGENTS.md", "Be concise."),
    ]);
    let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(warnings.is_empty());
    assert!(files.iter().any(|f| f.path == "armory.json"));
    assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md"));
}

#[test]
fn does_not_strip_when_entries_disagree_on_a_common_wrapper() {
    // An inconsistent archive (some entries wrapped, some not) must
    // not have a wrapper GUESSED at — every entry keeps its raw path,
    // which then fails components.* lookups with a clear "not found"
    // warning downstream rather than silently importing wrong content.
    let zip_bytes = build_zip(&[
        ("root-a/armory.json", "{}"),
        ("root-b/instructions/AGENTS.md", "Be concise."),
    ]);
    let (files, _warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(files.iter().any(|f| f.path == "root-a/armory.json"));
    assert!(files.iter().any(|f| f.path == "root-b/instructions/AGENTS.md"));
}

#[test]
fn full_import_of_a_flat_hand_built_abf_finds_every_component() {
    // End-to-end: unzip -> parse, for the unwrapped case specifically
    // (the exact scenario reagent's finding said silently broke).
    let manifest = minimal_manifest(serde_json::json!({
        "instructions": ["instructions/AGENTS.md"],
        "skills": ["skills/deploy"],
    }));
    let zip_bytes = build_zip(&[
        ("armory.json", &manifest),
        ("instructions/AGENTS.md", "Be concise."),
        ("skills/deploy/SKILL.md", "---\nname: \"deploy\"\ndescription: \"d\"\n---\n\nbody"),
    ]);
    let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(warnings.is_empty());
    let parsed = parse_bundle_import(&files).unwrap();
    assert_eq!(parsed.instructions, "Be concise.");
    assert_eq!(parsed.skills.len(), 1);
    assert!(parsed.warnings.is_empty(), "unexpected warnings: {:?}", parsed.warnings);
}

// ── decompression size bounds (Codex P1, PR #2379) ────────────────
//
// Content is a repeated single character so it compresses to a few KB
// in the actual test zip regardless of its declared/logical size —
// these tests exercise the real cap logic without moving tens of MB
// through the test binary.

#[test]
fn skips_a_single_entry_exceeding_the_per_entry_cap() {
    let oversized = "a".repeat(MAX_ENTRY_UNCOMPRESSED_BYTES as usize + 1);
    let zip_bytes = build_zip(&[
        ("armory.json", "{}"),
        ("instructions/AGENTS.md", &oversized),
    ]);
    let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(files.iter().any(|f| f.path == "armory.json"));
    assert!(!files.iter().any(|f| f.path == "instructions/AGENTS.md"));
    assert!(warnings.iter().any(|w| w.contains("instructions/AGENTS.md") && w.contains("exceeds the per-entry limit")));
}

#[test]
fn accepts_an_entry_right_at_the_per_entry_cap() {
    let exactly_at_cap = "a".repeat(MAX_ENTRY_UNCOMPRESSED_BYTES as usize);
    let zip_bytes = build_zip(&[
        ("armory.json", "{}"),
        ("instructions/AGENTS.md", &exactly_at_cap),
    ]);
    let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
    assert!(warnings.is_empty());
    assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md"));
}

#[test]
fn rejects_the_whole_import_when_aggregate_size_exceeds_the_total_cap() {
    // Six entries at ~90% of the per-entry cap each -- individually
    // well under MAX_ENTRY_UNCOMPRESSED_BYTES, but summing to ~5.4x
    // MAX_TOTAL_UNCOMPRESSED_BYTES's actual ratio (6 * 0.9*10MB = 54MB
    // > the 50MB aggregate cap). Real content, so this exercises the
    // aggregate check regardless of whether it's keyed on declared or
    // actual bytes -- see the dedicated "lying declared size" test
    // below for the distinction that matters (reagent P1, round 2).
    let each = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 0.9) as usize);
    let zip_bytes = build_zip(&[
        ("armory.json", "{}"),
        ("instructions/context/a.md", &each),
        ("instructions/context/b.md", &each),
        ("instructions/context/c.md", &each),
        ("instructions/context/d.md", &each),
        ("instructions/context/e.md", &each),
        ("instructions/context/f.md", &each),
    ]);
    let err = unzip_bundle_import(&zip_bytes).unwrap_err();
    assert!(err.contains("aggregate decompressed size exceeds"));
}

#[test]
fn aggregate_cap_is_enforced_against_actual_bytes_even_when_declared_size_understates_them() {
    // reagent P1, PR #2379 round 2: the aggregate check must accumulate
    // ACTUAL decompressed bytes, not the zip's own declared/attacker-
    // controlled `entry.size()` -- otherwise many entries that each lie
    // with a tiny declared size while really containing content up to
    // the per-entry cap would pass both checks and still exhaust
    // memory. `ZipWriter`'s public API always writes a correct
    // declared size for real content, so this is verified by patching
    // the archive's declared-size fields post-write to a tiny value
    // while leaving the real (large) compressed/uncompressed payload
    // bytes untouched -- exactly the "lying" shape the finding
    // describes. The local file header's uncompressed-size field sits
    // at a fixed offset (22) from the start of each entry's header.
    let each = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 0.9) as usize);
    let mut zip_bytes = build_zip(&[
        ("armory.json", "{}"),
        ("instructions/context/a.md", &each),
        ("instructions/context/b.md", &each),
        ("instructions/context/c.md", &each),
        ("instructions/context/d.md", &each),
        ("instructions/context/e.md", &each),
        ("instructions/context/f.md", &each),
    ]);
    // Local file header signature: 0x04034b50, little-endian bytes
    // 50 4B 03 04. Uncompressed size is the u32 at header offset 22.
    let sig = [0x50u8, 0x4b, 0x03, 0x04];
    let mut i = 0usize;
    let mut patched = 0;
    while i + 26 <= zip_bytes.len() {
        if zip_bytes[i..i + 4] == sig {
            zip_bytes[i + 22..i + 26].copy_from_slice(&1u32.to_le_bytes());
            patched += 1;
        }
        i += 1;
    }
    assert!(patched >= 6, "expected to patch every local file header, patched {patched}");
    let err = unzip_bundle_import(&zip_bytes).unwrap_err();
    assert!(
        err.contains("aggregate decompressed size exceeds"),
        "declared-size lie should not bypass the aggregate cap: {err:?}"
    );
}

#[test]
fn rejects_an_archive_with_more_entries_than_the_count_cap() {
    // reagent P2, PR #2379 round 2: per-entry/aggregate byte caps alone
    // don't bound the CPU cost of iterating + sanitizing + hashing an
    // archive with an enormous number of small, individually-valid
    // entries.
    let mut entries: Vec<(String, String)> = (0..=MAX_ENTRY_COUNT)
        .map(|i| (format!("instructions/context/f{i}.md"), "x".to_string()))
        .collect();
    entries.push(("armory.json".to_string(), "{}".to_string()));
    let refs: Vec<(&str, &str)> = entries.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let zip_bytes = build_zip(&refs);
    let err = unzip_bundle_import(&zip_bytes).unwrap_err();
    assert!(err.contains("entries exceeds the limit"));
}

// ── Phase 3 shared helpers (SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md) ──

#[test]
fn warning_budget_bounds_accumulation_during_parsing_itself() {
    // codex P2, PR #2381, round 11: the cap must live at the parser's
    // own warning-push sites, not just at a later response-boundary
    // projection -- called directly here (not through an RPC response
    // serializer) to prove the returned Vec is bounded regardless of
    // any downstream projection.
    let many_junk: Vec<Value> = (0..5_000).map(|i| serde_json::json!(i)).collect();
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_junk }))),
    ];
    let result = parse_bundle_import_with_budget(&files, WarningBudget::bounded(50, 40)).unwrap();
    assert!(result.warnings.len() <= 51, "expected at most 50 warnings plus one summary, got {}", result.warnings.len());
    assert!(result.warnings.iter().any(|w| w.contains("more warning(s) not shown")));
    for w in &result.warnings {
        // truncate_display appends "..." after taking max_chars, so the
        // hard ceiling is max_len + 3, not max_len exactly.
        assert!(w.chars().count() <= 43, "warning exceeded the budget's max length: {w:?}");
    }
}

#[test]
fn warning_budget_unbounded_matches_todays_existing_route_behavior() {
    // parse_bundle_import (no budget arg) must behave identically to
    // parse_bundle_import_with_budget(files, WarningBudget::unbounded())
    // -- the existing bundle.import route's real, already-shipped
    // behavior must not change.
    let many_junk: Vec<Value> = (0..500).map(|i| serde_json::json!(i)).collect();
    let files = vec![
        file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_junk.clone() }))),
    ];
    let a = parse_bundle_import(&files).unwrap();
    let b = parse_bundle_import_with_budget(&files, WarningBudget::unbounded()).unwrap();
    assert_eq!(a.warnings, b.warnings);
    assert_eq!(a.warnings.len(), 500);
}

#[test]
fn duplicate_in_bundle_slugs_flags_only_slugs_appearing_more_than_once() {
    let skills = vec![
        ParsedSkill { source_dir: "skills/a".to_string(), slug: "code-review".to_string(), description: String::new(), content: String::new() },
        ParsedSkill { source_dir: "skills/b".to_string(), slug: "code-review".to_string(), description: String::new(), content: String::new() },
        ParsedSkill { source_dir: "skills/c".to_string(), slug: "unique".to_string(), description: String::new(), content: String::new() },
    ];
    let dupes = duplicate_in_bundle_slugs(&skills);
    assert!(dupes.contains("code-review"));
    assert!(!dupes.contains("unique"));
    assert_eq!(dupes.len(), 1);
}

#[test]
fn classify_skill_collision_prioritizes_global_catalog_over_intra_bundle_duplicate() {
    // Phase 3 spec §3.1: pass 1 (global catalog) takes priority over
    // pass 2 (intra-bundle duplicate) when both apply.
    let mut global: HashSet<String> = HashSet::new();
    global.insert("taken".to_string());
    let mut dupes: HashSet<String> = HashSet::new();
    dupes.insert("taken".to_string());
    dupes.insert("dupe-only".to_string());
    assert_eq!(classify_skill_collision("taken", &global, &dupes), "name_conflict");
    assert_eq!(classify_skill_collision("dupe-only", &global, &dupes), "duplicate_in_bundle");
    assert_eq!(classify_skill_collision("free", &global, &dupes), "none");
}

#[test]
fn truncate_display_truncates_only_when_over_the_cap() {
    assert_eq!(truncate_display("short", 100), "short");
    let long = "a".repeat(150);
    let truncated = truncate_display(&long, 100);
    assert_eq!(truncated.chars().count(), 103); // 100 chars + "..."
    assert!(truncated.ends_with("..."));
}

#[test]
fn bounded_instructions_preview_reports_truncation_and_true_total_length() {
    let long = "x".repeat(MAX_INSTRUCTIONS_PREVIEW_CHARS + 500);
    let (preview, truncated, total) = bounded_instructions_preview(&long);
    assert!(truncated);
    assert_eq!(total, MAX_INSTRUCTIONS_PREVIEW_CHARS + 500);
    assert_eq!(preview.chars().count(), MAX_INSTRUCTIONS_PREVIEW_CHARS);

    let short = "short instructions";
    let (preview2, truncated2, total2) = bounded_instructions_preview(short);
    assert!(!truncated2);
    assert_eq!(total2, short.chars().count());
    assert_eq!(preview2, short);
}

#[test]
fn mcp_server_display_never_returns_the_full_config() {
    let config = serde_json::json!({
        "name": "github",
        "command": "npx",
        "args": ["-y", "gh-mcp"],
        "env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" },
    });
    let display = mcp_server_display(&config);
    assert_eq!(display["name"], "github");
    assert_eq!(display["command"], "npx");
    assert!(display.get("args").is_none());
    assert!(display.get("env").is_none());
}

#[test]
fn mcp_server_display_bounds_an_oversized_name_or_command() {
    let oversized = "n".repeat(MAX_MCP_DISPLAY_FIELD_CHARS + 50);
    let config = serde_json::json!({ "name": oversized, "command": "npx" });
    let display = mcp_server_display(&config);
    let name = display["name"].as_str().unwrap();
    assert!(name.chars().count() <= MAX_MCP_DISPLAY_FIELD_CHARS + 3);
}

#[test]
fn mcp_server_display_falls_back_to_null_when_fields_absent_or_wrong_type() {
    let config = serde_json::json!({ "command": 123 });
    let display = mcp_server_display(&config);
    assert!(display["name"].is_null());
    assert!(display["command"].is_null());
}

#[test]
fn content_digest_raw_bytes_differs_by_mode_for_identical_bytes() {
    // Phase 3 spec §3.0.5, round 7: file_path and zip_base64 both
    // canonicalize to the same raw zip bytes -- without a mode tag
    // mixed into the hash domain, a file_path preview would be
    // satisfiable by a zip_base64 commit of the same underlying
    // archive, defeating the round-6 same-mode-required fix.
    let bytes = b"identical zip bytes";
    let a = content_digest_raw_bytes(ImportInputMode::FilePath, bytes);
    let b = content_digest_raw_bytes(ImportInputMode::ZipBase64, bytes);
    assert_ne!(a, b);
    // Same mode, same bytes -> identical digest, deterministically.
    assert_eq!(a, content_digest_raw_bytes(ImportInputMode::FilePath, bytes));
}

#[test]
fn content_digest_files_is_order_independent_for_genuinely_equivalent_inputs() {
    let a = vec![file("armory.json", "{}"), file("instructions/AGENTS.md", "Be concise.")];
    let b = vec![file("instructions/AGENTS.md", "Be concise."), file("armory.json", "{}")];
    assert_eq!(content_digest_files(&a), content_digest_files(&b));
}

#[test]
fn content_digest_files_changes_when_reordering_changes_the_first_wins_outcome() {
    // codex P1, PR #2381, round 6: a naive raw-input sort would make
    // two differently-ordered request bodies hash identically even
    // when the parser's own first-wins rule would actually import
    // DIFFERENT content from each (whichever happened to be first).
    let a = vec![file("instructions/AGENTS.md", "first wins"), file("instructions/AGENTS.md", "second, discarded")];
    let b = vec![file("instructions/AGENTS.md", "second, discarded"), file("instructions/AGENTS.md", "first wins")];
    assert_ne!(
        content_digest_files(&a),
        content_digest_files(&b),
        "reordering which entry wins the normalize-then-first-wins reduction must change the digest"
    );
}
