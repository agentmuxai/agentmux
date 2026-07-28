// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Armory Bundle Format (ABF) exporter — Phase 1 of
//! `docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md` /
//! <https://docs.agentmux.ai/abf/>. Serializes a `db_bundles` row (the
//! `Memory` struct — table/UI say "Bundles", the type name predates the
//! rename) plus its referenced skills and inline MCP server configs into
//! the ABF v0.1 on-disk layout:
//!
//! ```text
//! <bundle-slug>/
//! ├── armory.json
//! ├── instructions/
//! │   ├── AGENTS.md
//! │   └── context/…
//! ├── skills/
//! │   └── <skill-slug>/
//! │       └── SKILL.md
//! ├── mcp/
//! │   └── <server-slug>.server.json
//! └── accounts/
//!     └── requirements.json   (only written when inferred requirements exist)
//! ```
//!
//! Pure functions only — no I/O, no Store access. Callers (the `bundle.export`
//! RPC handler) own fetching the `Memory` row and resolving its `skills`
//! id-array into `Skill` rows before calling [`export_bundle`].
//!
//! **`accounts/requirements.json` design note:** the plan this implements
//! says these entries come from "`db_agent_identity_links`-implied needs",
//! but no such link exists at the bundle level — `db_agent_identity_links`
//! is keyed on `(agent_id, provider)`, and bundles are reusable across many
//! agents with no FK to any one of them. Rather than requiring an arbitrary
//! agent context (and risking exporting a real `db_accounts` pointer),
//! requirements are inferred abstractly from the bundle's own inline
//! `mcp_servers` configs: each `env` key on a server config becomes one
//! requirement declaration (name/provider guess, no values, ever) — this
//! matches ABF's own "declare, don't bundle secrets" design and MCP's own
//! `isSecret`/env-placeholder convention (see the research report §3b/§3d).
//!
//! **`mcp/<slug>.server.json` content note:** this writes AgentMux's own
//! runtime MCP config shape (`{type, command, args, env}` — the same object
//! `.mcp.json` uses), NOT the official MCP registry `server.json` schema
//! (`packages[].registry_type`/`identifier`/`environment_variables`, etc.).
//! Real conversion would require fabricating fields this data doesn't
//! contain (registry type/package identifier aren't derivable from a bare
//! stdio command) — worse than an honest runtime-shape export. Deferred to
//! Phase 2 (schema validation), tracked alongside the importer rather than
//! guessed at here (Codex P1, PR #2325 — flagged as a real gap, not
//! disputed; this comment documents the deliberate scope decision).
//! `env` values ARE redacted before being written (see [`redact_mcp_entry`])
//! regardless of this open question — the credential-leak fix does not wait
//! on the schema question.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{json, Value};

use super::agent_config::{render_skill_md, unique_skill_slug, SKILL_TYPE_AGENT_SKILL};
use super::storage::store::{derive_slug, Memory};
use super::storage::Skill;

/// One file within an exported bundle, path relative to the bundle root
/// (e.g. `"armory.json"`, `"instructions/AGENTS.md"`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BundleExportFile {
    pub path: String,
    pub content: String,
}

/// Result of exporting one bundle: every file to write, plus bookkeeping
/// about anything that couldn't be represented in ABF and was left out
/// (surfaced to the caller/UI rather than silently dropped).
#[derive(Debug, Clone, Serialize)]
pub struct BundleExport {
    /// Filesystem-safe slug derived from the bundle's name — the
    /// recommended root directory / zip base name.
    pub root_slug: String,
    pub files: Vec<BundleExportFile>,
    /// Names of skills that were NOT exported because their `skill_type`
    /// isn't `"agent-skill"` (ABF's `skills` component is Agent Skills
    /// (SKILL.md) format specifically — AgentMux's proprietary
    /// slash-command skills have no ABF representation, see Phase 0 /
    /// `SKILL_TYPE_AGENT_SKILL`).
    pub skipped_skills: Vec<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct ContextFileEntry {
    #[serde(default)]
    path: String,
    #[serde(default)]
    content: String,
}

/// Redact secret-shaped values out of an MCP server config entry before it
/// is written into a shareable bundle export. `env` and `headers` are the
/// two fields AgentMux's own MCP config editors accept literal secret
/// values into (e.g. `env.GITHUB_TOKEN`, `headers.Authorization` on an
/// HTTP/SSE-transport server) — every value under either key is replaced
/// with a `${VAR_NAME}`-style placeholder so the exported `.server.json`
/// never contains a real credential, matching ABF's "declare, don't bundle
/// secrets" principle that `accounts/requirements.json` already follows
/// (security finding, Codex P1 x2, PR #2325). Any other field passes
/// through unchanged.
fn redact_mcp_entry(entry: &Value) -> Value {
    let mut redacted = entry.clone();
    if let Some(obj) = redacted.as_object_mut() {
        for field in ["env", "headers"] {
            if let Some(Value::Object(map)) = obj.get_mut(field) {
                for (key, value) in map.iter_mut() {
                    *value = json!(format!("${{{key}}}"));
                }
            }
        }
    }
    redacted
}

/// Validate a context-file's relative path is safe to place under
/// `instructions/context/` in an exported bundle: non-empty, not absolute,
/// no drive letter, no `..` traversal component. Pure string validation
/// (no filesystem access) so [`export_bundle`] can stay a pure function.
/// Returns `None` for anything that fails; callers skip that entry.
fn sanitize_context_relative_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        return None;
    }
    let mut parts = Vec::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => continue,
            ".." => return None,
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Export a bundle + its already-resolved skills into the ABF v0.1 layout.
///
/// `skills` must already be resolved from `bundle.skills` (a JSON array of
/// skill ids) via `Store::skill_get` per id — this function does no
/// lookups. Callers should skip ids that failed to resolve (deleted skills)
/// before calling this; it does not distinguish "missing" from "not
/// passed."
pub fn export_bundle(bundle: &Memory, skills: &[Skill]) -> BundleExport {
    let root_slug = derive_slug(&bundle.name);
    let mut files = Vec::new();
    let mut skipped_skills = Vec::new();

    let mut manifest_instructions: Vec<String> = Vec::new();
    let mut manifest_skills: Vec<String> = Vec::new();
    let mut manifest_mcp: Vec<String> = Vec::new();

    // ------------------------------------------------------------------
    // instructions/AGENTS.md + instructions/context/*
    // ------------------------------------------------------------------
    if !bundle.instructions.trim().is_empty() {
        files.push(BundleExportFile {
            path: "instructions/AGENTS.md".to_string(),
            content: bundle.instructions.clone(),
        });
        manifest_instructions.push("instructions/AGENTS.md".to_string());
    }

    let context_files: Vec<ContextFileEntry> =
        serde_json::from_str(&bundle.context_files).unwrap_or_default();
    for entry in context_files {
        if let Some(safe_path) = sanitize_context_relative_path(&entry.path) {
            let out_path = format!("instructions/context/{safe_path}");
            manifest_instructions.push(out_path.clone());
            files.push(BundleExportFile {
                path: out_path,
                content: entry.content,
            });
        }
    }

    // ------------------------------------------------------------------
    // skills/<slug>/SKILL.md — Agent Skills format only (see doc comment)
    // ------------------------------------------------------------------
    let mut used_skill_slugs: HashSet<String> = HashSet::new();
    for skill in skills {
        if skill.skill_type != SKILL_TYPE_AGENT_SKILL {
            skipped_skills.push(skill.name.clone());
            continue;
        }
        if skill.content.is_empty() {
            skipped_skills.push(skill.name.clone());
            continue;
        }
        let slug = unique_skill_slug(&skill.name, &mut used_skill_slugs);
        files.push(BundleExportFile {
            path: format!("skills/{slug}/SKILL.md"),
            content: render_skill_md(&slug, &skill.description, &skill.content),
        });
        // Manifest references the skill's DIRECTORY, not the SKILL.md file
        // inside it -- per the ABF spec's own on-disk layout example
        // (`"skills": ["skills/deploy-checklist"]`), which lets an importer
        // locate SKILL.md plus any optional scripts/references/assets
        // alongside it (Codex P1, PR #2325).
        manifest_skills.push(format!("skills/{slug}"));
    }

    // ------------------------------------------------------------------
    // mcp/<slug>.server.json + inferred accounts/requirements.json
    // ------------------------------------------------------------------
    let mcp_entries: Vec<Value> = serde_json::from_str(&bundle.mcp_servers).unwrap_or_default();
    let mut used_mcp_slugs: HashSet<String> = HashSet::new();
    let mut requirements: Vec<Value> = Vec::new();
    for (index, entry) in mcp_entries.iter().enumerate() {
        let display_name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("mcp-server-{}", index + 1));
        let slug = unique_skill_slug(&display_name, &mut used_mcp_slugs);
        let path = format!("mcp/{slug}.server.json");
        let redacted_entry = redact_mcp_entry(entry);
        let pretty = serde_json::to_string_pretty(&redacted_entry).unwrap_or_else(|_| "{}".to_string());
        files.push(BundleExportFile {
            path: path.clone(),
            content: pretty,
        });
        manifest_mcp.push(path);

        if let Some(env_obj) = entry.get("env").and_then(|v| v.as_object()) {
            for key in env_obj.keys() {
                requirements.push(json!({
                    "id": format!("{slug}-{key}"),
                    "provider": slug,
                    "kind": "api-key",
                    "env": key,
                    "optional": false,
                }));
            }
        }
    }

    let mut components = serde_json::Map::new();
    if !manifest_instructions.is_empty() {
        components.insert("instructions".to_string(), json!(manifest_instructions));
    }
    if !manifest_skills.is_empty() {
        components.insert("skills".to_string(), json!(manifest_skills));
    }
    if !manifest_mcp.is_empty() {
        components.insert("mcpServers".to_string(), json!(manifest_mcp));
    }
    if !requirements.is_empty() {
        components.insert(
            "accounts".to_string(),
            json!("accounts/requirements.json"),
        );
        let requirements_doc = json!({ "requirements": requirements });
        files.push(BundleExportFile {
            path: "accounts/requirements.json".to_string(),
            content: serde_json::to_string_pretty(&requirements_doc)
                .unwrap_or_else(|_| "{}".to_string()),
        });
    }

    let manifest = json!({
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
        "name": root_slug,
        // Bundles have no native version concept (no `version` column) --
        // ABF requires one, so this is an export-time default the user is
        // expected to bump before actually publishing the bundle anywhere.
        "version": "0.1.0",
        "description": bundle.description,
        "components": Value::Object(components),
        "metadata": {},
    });
    files.push(BundleExportFile {
        path: "armory.json".to_string(),
        content: serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string()),
    });

    BundleExport {
        root_slug,
        files,
        skipped_skills,
    }
}

/// Pack an export's files into a zip archive in memory. First `ZipWriter`
/// usage in this workspace — `tool_store.rs` only ever reads zips. No
/// filesystem access: writes into an in-memory buffer, so this stays as
/// side-effect-free as the exporter itself (the RPC handler decides what
/// to do with the resulting bytes).
pub fn zip_bundle_export(export: &BundleExport) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let mut buf = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for file in &export.files {
        let zip_path = format!("{}/{}", export.root_slug, file.path);
        writer
            .start_file(zip_path, options)
            .map_err(|e| format!("zip_bundle_export: {e}"))?;
        writer
            .write_all(file.content.as_bytes())
            .map_err(|e| format!("zip_bundle_export: {e}"))?;
    }

    writer
        .finish()
        .map_err(|e| format!("zip_bundle_export: {e}"))?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bundle(instructions: &str, context_files: &str, mcp_servers: &str, skills: &str) -> Memory {
        Memory {
            id: "bundle-1".to_string(),
            name: "Backend Dev Bundle".to_string(),
            description: "Backend dev conventions".to_string(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: instructions.to_string(),
            context_files: context_files.to_string(),
            mcp_servers: mcp_servers.to_string(),
            skills: skills.to_string(),
            sort_order: 0,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    fn make_agent_skill(name: &str) -> Skill {
        Skill {
            id: format!("skill-{name}"),
            name: name.to_string(),
            trigger: String::new(),
            skill_type: "agent-skill".to_string(),
            description: format!("{name} description"),
            content: format!("{name} content"),
            is_global: true,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn exports_instructions_as_agents_md() {
        let bundle = make_bundle("Follow repo conventions.", "[]", "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        let f = export.files.iter().find(|f| f.path == "instructions/AGENTS.md").unwrap();
        assert_eq!(f.content, "Follow repo conventions.");
    }

    #[test]
    fn exports_context_files_under_instructions_context() {
        let context_files = r#"[{"path":"docs/readme.md","content":"Readme heading"}]"#;
        let bundle = make_bundle("", context_files, "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        let f = export
            .files
            .iter()
            .find(|f| f.path == "instructions/context/docs/readme.md")
            .expect("expected instructions/context/docs/readme.md");
        assert_eq!(f.content, "Readme heading");
    }

    #[test]
    fn rejects_path_traversal_in_context_files() {
        let context_files = r#"[{"path":"../../etc/passwd","content":"evil"}]"#;
        let bundle = make_bundle("", context_files, "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(export.files.iter().all(|f| !f.content.contains("evil")));
        assert!(!export.files.iter().any(|f| f.path.contains("..")));
    }

    #[test]
    fn exports_agent_skill_format_skills_and_skips_prompt_format() {
        let bundle = make_bundle("", "[]", "[]", r#"["skill-a","skill-b"]"#);
        let mut prompt_skill = make_agent_skill("Slash Skill");
        prompt_skill.skill_type = "prompt".to_string();
        let skills = vec![make_agent_skill("Deploy Checklist"), prompt_skill];

        let export = export_bundle(&bundle, &skills);
        assert!(export
            .files
            .iter()
            .any(|f| f.path == "skills/deploy-checklist/SKILL.md"));
        assert!(!export.files.iter().any(|f| f.path.contains("slash-skill")));
        assert_eq!(export.skipped_skills, vec!["Slash Skill".to_string()]);
    }

    #[test]
    fn exports_mcp_servers_and_infers_requirements_from_env_keys() {
        let mcp_servers = r#"[{"name":"github","type":"stdio","command":"gh-mcp","env":{"GITHUB_TOKEN":""}}]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);

        let server_file = export
            .files
            .iter()
            .find(|f| f.path == "mcp/github.server.json")
            .expect("expected mcp/github.server.json");
        assert!(server_file.content.contains("gh-mcp"));

        let req_file = export
            .files
            .iter()
            .find(|f| f.path == "accounts/requirements.json")
            .expect("expected accounts/requirements.json to be inferred");
        assert!(req_file.content.contains("GITHUB_TOKEN"));
        assert!(!req_file.content.to_lowercase().contains("ghp_"), "must never contain a real secret value");
    }

    #[test]
    fn redacts_real_secret_values_from_the_exported_server_json() {
        // Security finding, Codex P1 x2, PR #2325: the exported .server.json
        // must never contain the literal secret value stored in env/headers,
        // even though requirements.json was already value-free.
        let mcp_servers = r#"[{
            "name": "github",
            "type": "stdio",
            "command": "gh-mcp",
            "env": {"GITHUB_TOKEN": "ghp_realSecretValue123"},
            "headers": {"Authorization": "Bearer realBearerToken456"}
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);

        let server_file = export
            .files
            .iter()
            .find(|f| f.path == "mcp/github.server.json")
            .expect("expected mcp/github.server.json");
        assert!(
            !server_file.content.contains("ghp_realSecretValue123"),
            "exported .server.json must never contain the real env secret value: {}",
            server_file.content
        );
        assert!(
            !server_file.content.contains("realBearerToken456"),
            "exported .server.json must never contain the real header secret value: {}",
            server_file.content
        );
        // Redacted to a placeholder, not silently dropped -- the key/shape survives.
        assert!(server_file.content.contains("${GITHUB_TOKEN}"));
        assert!(server_file.content.contains("${Authorization}"));

        // requirements.json inference is unaffected (it only ever read keys).
        let req_file = export
            .files
            .iter()
            .find(|f| f.path == "accounts/requirements.json")
            .unwrap();
        assert!(req_file.content.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn no_requirements_file_when_no_env_vars_present() {
        let mcp_servers = r#"[{"name":"local-tool","type":"stdio","command":"local-tool"}]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(!export.files.iter().any(|f| f.path == "accounts/requirements.json"));
    }

    #[test]
    fn dedupes_colliding_mcp_server_names() {
        let mcp_servers = r#"[{"name":"Server!!!One"},{"name":"Server One"}]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let paths: HashSet<&str> = export.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains("mcp/server-one.server.json"));
        assert!(paths.contains("mcp/server-one-2.server.json"));
    }

    #[test]
    fn manifest_lists_every_component_and_validates_as_json() {
        let bundle = make_bundle(
            "Instructions",
            r#"[{"path":"a.md","content":"A"}]"#,
            r#"[{"name":"github","env":{"GITHUB_TOKEN":""}}]"#,
            r#"["skill-a"]"#,
        );
        let export = export_bundle(&bundle, &[make_agent_skill("Deploy")]);
        let manifest_file = export.files.iter().find(|f| f.path == "armory.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).expect("armory.json must be valid JSON");
        assert_eq!(manifest["name"], "backend-dev-bundle");
        assert!(manifest["components"]["instructions"].as_array().unwrap().len() == 2);
        assert!(manifest["components"]["skills"].as_array().unwrap().len() == 1);
        assert!(manifest["components"]["mcpServers"].as_array().unwrap().len() == 1);
        assert_eq!(manifest["components"]["accounts"], "accounts/requirements.json");
    }

    #[test]
    fn manifest_references_the_skill_directory_not_the_skill_md_file() {
        // Codex P1, PR #2325: the ABF spec's manifest example references
        // "skills/<slug>" (the directory), not "skills/<slug>/SKILL.md".
        let bundle = make_bundle("", "[]", "[]", r#"["skill-a"]"#);
        let export = export_bundle(&bundle, &[make_agent_skill("Deploy Checklist")]);
        let manifest_file = export.files.iter().find(|f| f.path == "armory.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).unwrap();
        let skills = manifest["components"]["skills"].as_array().unwrap();
        assert_eq!(skills, &vec![json!("skills/deploy-checklist")]);
        // The actual file on disk still lives at the nested SKILL.md path.
        assert!(export.files.iter().any(|f| f.path == "skills/deploy-checklist/SKILL.md"));
    }

    #[test]
    fn empty_bundle_still_produces_a_valid_manifest() {
        let bundle = make_bundle("", "[]", "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        // armory.json is always written, even for a fully empty bundle.
        assert_eq!(export.files.len(), 1);
        let manifest: Value = serde_json::from_str(&export.files[0].content).unwrap();
        assert_eq!(manifest["components"], json!({}));
    }

    #[test]
    fn zip_bundle_export_produces_a_valid_archive_with_all_files() {
        let bundle = make_bundle("Instructions here", "[]", "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        let zip_bytes = zip_bundle_export(&export).expect("zip should succeed");

        let cursor = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid zip archive");
        assert_eq!(archive.len(), export.files.len());

        let mut found = false;
        for i in 0..archive.len() {
            let file = archive.by_index(i).unwrap();
            if file.name() == "backend-dev-bundle/armory.json" {
                found = true;
            }
        }
        assert!(found, "expected backend-dev-bundle/armory.json in the archive");
    }
}
