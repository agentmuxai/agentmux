// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Armory Bundle Format (ABF) exporter — Phase 1 of
//! `docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md` /
//! <https://docs.agentmux.ai/abf/>. Serializes a `db_bundles` row (the
//! `Bundle` struct — table/UI say "Bundles", the type name predates the
//! rename) plus its referenced skills into the ABF on-disk layout:
//!
//! ```text
//! <bundle-slug>/
//! ├── bundle.json
//! ├── instructions/
//! │   ├── AGENTS.md
//! │   └── context/…
//! └── skills/
//!     └── <skill-slug>/
//!         └── SKILL.md
//! ```
//!
//! Pure functions only — no I/O, no Store access. Callers (the `bundle.export`
//! RPC handler) own fetching the `Bundle` row and resolving its `skills`
//! id-array into `Skill` rows before calling [`export_bundle`].
//!
//! **No `mcp/` or `accounts/`.** A bundle carries no MCP servers; they belong
//! to Connectors (`SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md` §3.1).
//! `accounts/requirements.json` was inferred from those servers' `env` keys,
//! so it went with them, as did the redaction of secrets out of their configs.
//! An importer still reads both from older archives.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{json, Value};

use super::agent_config::{render_skill_md, unique_skill_slug, SKILL_TYPE_AGENT_SKILL};
use super::storage::store::{derive_slug, Bundle};
use super::storage::Skill;

/// One file within an exported bundle, path relative to the bundle root
/// (e.g. `"bundle.json"`, `"instructions/AGENTS.md"`).
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
    /// Non-fatal problems encountered while exporting: malformed source
    /// JSON (`context_files`, `instructions_by_provider`) that had to be treated as
    /// empty, or a context-file path that collided with an earlier one
    /// after normalization and was skipped rather than silently
    /// overwriting it. Never blocks the export — surfaced to the
    /// caller/UI instead of being silently swallowed, since a backup tool
    /// losing data without saying so defeats its own purpose (reagent P1,
    /// PR #2333).
    pub warnings: Vec<String>,
}

// `pub(crate)` so `bundle_validate.rs` can parse `context_files` the exact
// same way export does — a single shared shape means the two can never
// silently disagree on what a context-file entry looks like.
#[derive(Debug, serde::Deserialize, Default)]
pub(crate) struct ContextFileEntry {
    #[serde(default)]
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) content: String,
}

/// Validate a relative path is safe to place under a bundle export/import
/// root: non-empty, not absolute, no drive letter, no `..` traversal
/// component. Pure string validation (no filesystem access) so
/// [`export_bundle`] can stay a pure function. Returns `None` for anything
/// that fails; callers skip that entry.
///
/// `pub(crate)` (not private) so `bundle_import.rs` can reuse the exact
/// same check on the way IN — an untrusted `.abf` file needs this defense
/// at least as much as export needs it on the way out, and a single shared
/// implementation means the two can never drift apart on what counts as
/// safe (see `docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md`
/// §4.3.4).
pub(crate) fn sanitize_context_relative_path(path: &str) -> Option<String> {
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

/// Export a bundle + its already-resolved components into the ABF layout.
///
/// **This function does no store lookups and reads no component column off
/// `bundle`.** `skills` must be resolved by the caller from
/// `db_bundle_skills_ref`, which is authoritative for what a bundle contains
/// (Phase 0b of SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md).
/// Callers skip ids that failed to resolve before calling; this does not
/// distinguish "missing" from "not passed".
pub fn export_bundle(bundle: &Bundle, skills: &[Skill]) -> BundleExport {
    let root_slug = derive_slug(&bundle.name);
    let mut files = Vec::new();
    let mut skipped_skills = Vec::new();
    let mut warnings = Vec::new();

    let mut manifest_instructions: Vec<String> = Vec::new();
    let mut manifest_skills: Vec<String> = Vec::new();

    // ------------------------------------------------------------------
    // instructions/AGENTS.md (default) + instructions/<provider>/AGENTS.md
    // (ABF v0.2 §2.2) + instructions/context/*
    // ------------------------------------------------------------------
    if !bundle.instructions.trim().is_empty() {
        files.push(BundleExportFile {
            path: "instructions/AGENTS.md".to_string(),
            content: bundle.instructions.clone(),
        });
        manifest_instructions.push("instructions/AGENTS.md".to_string());
    }

    let instructions_by_provider: HashMap<String, String> = parse_json_field_or_warn(
        &bundle.instructions_by_provider,
        "instructions_by_provider",
        &mut warnings,
    );
    // manifest_instructions_by_provider preserves insertion order isn't
    // required (the manifest's own object key order is not meaningful), but
    // BTreeMap gives deterministic output ordering across export calls,
    // which matters for reproducible zip byte content.
    let mut manifest_instructions_by_provider: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    // reagent P2, PR #2523: two distinct raw keys (e.g. "claude" and
    // "./claude") can sanitize to the SAME output path — without a
    // collision check, both push a BundleExportFile at the identical path
    // (last one written silently wins, nondeterministically, since
    // HashMap iteration order isn't guaranteed) and the manifest lists
    // that path twice. Iterate raw keys sorted first (deterministic which
    // one "wins" a collision, not just which one happens to iterate
    // last), and skip + warn on the second and further collisions rather
    // than silently duplicating.
    let mut sorted_providers: Vec<(&String, &String)> = instructions_by_provider.iter().collect();
    sorted_providers.sort_by(|a, b| a.0.cmp(b.0));
    let mut seen_safe_providers: HashSet<String> = HashSet::new();
    for (provider, content) in sorted_providers {
        if content.trim().is_empty() {
            continue;
        }
        // Provider keys land directly in a file path — sanitize the same
        // way every other manifest-adjacent path segment in this module
        // is, since a key that arrived via import (bundle_import.rs stores
        // whatever key string a manifest declared, unvalidated against the
        // known provider list) could otherwise smuggle a traversal segment
        // into instructions/<provider>/AGENTS.md on a later re-export.
        let Some(safe_provider) = sanitize_context_relative_path(provider) else {
            warnings.push(format!(
                "instructions_by_provider: \"{provider}\" is not a safe path segment; skipped"
            ));
            continue;
        };
        if !seen_safe_providers.insert(safe_provider.clone()) {
            warnings.push(format!(
                "instructions_by_provider: \"{provider}\" normalizes to the same path as an earlier key (instructions/{safe_provider}/AGENTS.md); skipped to avoid overwriting it"
            ));
            continue;
        }
        let out_path = format!("instructions/{safe_provider}/AGENTS.md");
        files.push(BundleExportFile {
            path: out_path.clone(),
            content: content.clone(),
        });
        manifest_instructions_by_provider
            .entry(safe_provider)
            .or_default()
            .push(out_path);
    }

    let context_files: Vec<ContextFileEntry> = parse_json_field_or_warn(
        &bundle.context_files,
        "context_files",
        &mut warnings,
    );
    // Compared case-INSENSITIVELY (stores the lowercased form) because the
    // most common export/extract targets (Windows, macOS default) have
    // case-insensitive filesystems -- "Docs/A.md" and "docs/a.md" collide
    // on extraction there even though they're distinct paths byte-for-byte
    // (reagent P2, PR #2333). The path actually written to `files` keeps
    // its original case; only the collision check is case-folded.
    let mut used_context_paths: HashSet<String> = HashSet::new();
    for entry in context_files {
        if let Some(safe_path) = sanitize_context_relative_path(&entry.path) {
            let out_path = format!("instructions/context/{safe_path}");
            // Two distinct source paths can normalize to the same output
            // (e.g. "docs/a.md" and "docs/./a.md", or a case-only
            // difference) -- without this check the second silently
            // overwrites the first's `files` entry, and the zip archive
            // ends up with the same duplicate risk (reagent P2, PR #2333).
            if !used_context_paths.insert(out_path.to_lowercase()) {
                warnings.push(format!(
                    "context_files: \"{}\" normalizes to the same path as an \
                     earlier entry ({out_path}); skipped to avoid overwriting it",
                    entry.path
                ));
                continue;
            }
            manifest_instructions.push(out_path.clone());
            files.push(BundleExportFile {
                path: out_path,
                content: entry.content,
            });
        } else {
            // Rejected by sanitization (absolute path, ".." traversal, a
            // drive-letter colon, or an empty path) -- must warn the same
            // way the collision case right above does, or a backup export
            // can silently lose a context file with no signal anywhere in
            // the returned `warnings` (Codex + reagent P2, PR #2333 --
            // flagged twice, unaddressed in the prior push).
            warnings.push(format!(
                "context_files: \"{}\" is not a safe relative path (absolute, \
                 contains \"..\", a drive letter, or empty); skipped",
                entry.path
            ));
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

    let mut components = serde_json::Map::new();
    // ABF v0.2 §2.2: components.instructions is always the keyed-object
    // shape on export ("default" plus zero or more provider variants) —
    // the importer accepts both this and the v0.1 flat-array shape for
    // backward compatibility, but new exports only ever emit the v0.2
    // shape.
    if !manifest_instructions.is_empty() || !manifest_instructions_by_provider.is_empty() {
        let mut instructions_obj = serde_json::Map::new();
        if !manifest_instructions.is_empty() {
            instructions_obj.insert("default".to_string(), json!(manifest_instructions));
        }
        for (provider, paths) in &manifest_instructions_by_provider {
            instructions_obj.insert(provider.clone(), json!(paths));
        }
        components.insert("instructions".to_string(), Value::Object(instructions_obj));
    }
    if !manifest_skills.is_empty() {
        components.insert("skills".to_string(), json!(manifest_skills));
    }


    // Who the bundle was made for: a hint, never enforced (ABF v0.3,
    // SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §2.1). The row's `model`
    // has always held a vendor, so that's its name here. Omitted when unset.
    let mut suggested_for = serde_json::Map::new();
    if !bundle.provider.is_empty() {
        suggested_for.insert("provider".into(), json!(bundle.provider));
    }
    if !bundle.model.is_empty() {
        suggested_for.insert("vendor".into(), json!(bundle.model));
    }
    let mut manifest = json!({
        // The ABF FORMAT version (Agent Bundle Format v0.3), distinct from
        // the "version" field below (the bundle's own content version).
        "$schema": crate::backend::bundle_import::SCHEMA_V0_3,
        "name": root_slug,
        // Bundles have no native version concept (no `version` column) --
        // ABF requires one, so this is an export-time default the user is
        // expected to bump before actually publishing the bundle anywhere.
        "version": "0.1.0",
        "description": bundle.description,
        "components": Value::Object(components),
        "metadata": {},
    });
    if !suggested_for.is_empty() {
        manifest["suggestedFor"] = Value::Object(suggested_for);
    }
    files.push(BundleExportFile {
        path: crate::backend::bundle_import::MANIFEST_FILE.to_string(),
        content: serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string()),
    });

    BundleExport {
        root_slug,
        files,
        skipped_skills,
        warnings,
    }
}

/// Parse a `db_bundles` JSON column (`context_files`,
/// `instructions_by_provider`), treating a blank/whitespace-only value as
/// "genuinely no data" (not an error) but pushing a warning to `warnings`
/// for anything non-blank that fails to parse, rather than silently
/// discarding it via `unwrap_or_default()` — an export that quietly loses
/// data defeats its own backup/portability purpose (#2333).
pub(crate) fn parse_json_field_or_warn<T: serde::de::DeserializeOwned + Default>(
    raw: &str,
    field_name: &str,
    warnings: &mut Vec<String>,
) -> T {
    if raw.trim().is_empty() {
        return T::default();
    }
    match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("{field_name}: malformed JSON, treated as empty ({e})"));
            T::default()
        }
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

    fn make_bundle(instructions: &str, context_files: &str, mcp_servers: &str, skills: &str) -> Bundle {
        Bundle {
            id: "bundle-1".to_string(),
            name: "Backend Dev Bundle".to_string(),
            description: "Backend dev conventions".to_string(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: instructions.to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: context_files.to_string(),
            mcp_servers: mcp_servers.to_string(),
            skills: skills.to_string(),
            sort_order: 0,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            is_system: false,
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
        // Codex + reagent P2, PR #2333: a rejected entry must not just
        // vanish -- an export used as a backup can silently lose a context
        // file with no signal anywhere in `warnings` otherwise.
        assert!(
            export.warnings.iter().any(|w| w.contains("../../etc/passwd")),
            "expected a warning naming the rejected path, got: {:?}",
            export.warnings
        );
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
    fn malformed_context_files_json_warns_instead_of_silently_dropping_data() {
        // reagent P1, PR #2333: previously unwrap_or_default() silently
        // treated malformed context_files as empty, with no signal to the
        // caller that data was lost -- defeats the exporter's stated
        // backup/portability guarantee.
        let bundle = make_bundle("", "{not valid json", "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(
            export.warnings.iter().any(|w| w.contains("context_files") && w.contains("malformed")),
            "expected a warning about malformed context_files, got: {:?}",
            export.warnings
        );
    }

    #[test]
    fn blank_context_files_produce_no_warning() {
        // A genuinely empty/unset field is not an error -- must not warn.
        let bundle = make_bundle("", "", "", "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(export.warnings.is_empty(), "blank fields must not warn: {:?}", export.warnings);
    }

    #[test]
    fn parse_json_field_or_warn_direct_unit_test() {
        // PR #2333: `bundle.export`'s RPC handler used to parse the inline
        // `bundle.skills` column with this exact helper (until #3152 made the
        // ref tables authoritative), which previously had the same
        // unwrap_or_default() silent-loss bug already fixed here for
        // context_files.
        let mut warnings = Vec::new();
        let blank: Vec<String> = parse_json_field_or_warn("", "skills", &mut warnings);
        assert!(blank.is_empty());
        assert!(warnings.is_empty(), "blank must not warn");

        let malformed: Vec<String> = parse_json_field_or_warn("not json", "skills", &mut warnings);
        assert!(malformed.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("skills") && warnings[0].contains("malformed"));

        let mut warnings2 = Vec::new();
        let valid: Vec<String> = parse_json_field_or_warn(r#"["a","b"]"#, "skills", &mut warnings2);
        assert_eq!(valid, vec!["a".to_string(), "b".to_string()]);
        assert!(warnings2.is_empty());
    }

    #[test]
    fn colliding_context_file_paths_are_deduped_with_a_warning() {
        // reagent P2, PR #2333: two distinct source paths that normalize to
        // the same output (e.g. a redundant "./" component) previously
        // silently overwrote one `files` entry with the other.
        let context_files = r#"[
            {"path":"docs/a.md","content":"first"},
            {"path":"docs/./a.md","content":"second, would silently clobber the first"}
        ]"#;
        let bundle = make_bundle("", context_files, "[]", "[]");
        let export = export_bundle(&bundle, &[]);

        let matches: Vec<_> = export
            .files
            .iter()
            .filter(|f| f.path == "instructions/context/docs/a.md")
            .collect();
        assert_eq!(matches.len(), 1, "must not produce duplicate file entries for the same path");
        assert_eq!(matches[0].content, "first", "the first entry must win, not be silently overwritten");
        assert!(
            export.warnings.iter().any(|w| w.contains("docs/./a.md")),
            "expected a warning naming the skipped duplicate, got: {:?}",
            export.warnings
        );
    }

    #[test]
    fn colliding_context_file_paths_case_insensitive() {
        // reagent P2, PR #2333: "Docs/A.md" and "docs/a.md" are distinct
        // byte-for-byte but collide on extraction on the most common
        // export targets (Windows, macOS default case-insensitive
        // filesystems) -- must be caught the same way an exact-match
        // collision is.
        let context_files = r#"[
            {"path":"Docs/A.md","content":"first"},
            {"path":"docs/a.md","content":"second, would collide on a case-insensitive filesystem"}
        ]"#;
        let bundle = make_bundle("", context_files, "[]", "[]");
        let export = export_bundle(&bundle, &[]);

        let matches: Vec<_> = export
            .files
            .iter()
            .filter(|f| f.path.to_lowercase() == "instructions/context/docs/a.md")
            .collect();
        assert_eq!(matches.len(), 1, "must not produce case-only-different duplicate entries");
        assert_eq!(matches[0].content, "first");
        assert!(export.warnings.iter().any(|w| w.contains("docs/a.md")));
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
        let manifest_file = export.files.iter().find(|f| f.path == "bundle.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).expect("bundle.json must be valid JSON");
        assert_eq!(manifest["name"], "backend-dev-bundle");
        // ABF v0.2 §2.2: components.instructions is the keyed-object shape
        // ("default" + provider variants), not a flat array.
        assert!(manifest["components"]["instructions"]["default"].as_array().unwrap().len() == 2);
        assert!(manifest["components"]["skills"].as_array().unwrap().len() == 1);
    }

    /// A bundle carries no MCP servers, so a leftover inline column exports
    /// no `mcp/` files, no `mcpServers` and no inferred `accounts/`
    /// (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md §3.1).
    #[test]
    fn exports_no_mcp_servers_and_no_inferred_requirements() {
        let bundle = make_bundle("", "[]", r#"[{"name":"github","env":{"GITHUB_TOKEN":"ghp_x"}}]"#, "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(!export.files.iter().any(|f| f.path.starts_with("mcp/") || f.path.starts_with("accounts/")));
        let manifest: Value = serde_json::from_str(&export.files.iter().find(|f| f.path == "bundle.json").unwrap().content).unwrap();
        assert!(manifest["components"].get("mcpServers").is_none());
        assert!(manifest["components"].get("accounts").is_none());
        assert!(!export.files.iter().any(|f| f.content.contains("ghp_x")));
    }

    #[test]
    fn exports_a_provider_scoped_instruction_variant() {
        let mut bundle = make_bundle("Default instructions.", "[]", "[]", "[]");
        bundle.instructions_by_provider =
            r#"{"claude":"Claude-specific override.","codex":"Codex-specific override."}"#.to_string();
        let export = export_bundle(&bundle, &[]);

        let claude_file = export.files.iter().find(|f| f.path == "instructions/claude/AGENTS.md")
            .expect("expected instructions/claude/AGENTS.md");
        assert_eq!(claude_file.content, "Claude-specific override.");
        let codex_file = export.files.iter().find(|f| f.path == "instructions/codex/AGENTS.md")
            .expect("expected instructions/codex/AGENTS.md");
        assert_eq!(codex_file.content, "Codex-specific override.");

        let manifest_file = export.files.iter().find(|f| f.path == "bundle.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).unwrap();
        assert_eq!(manifest["components"]["instructions"]["default"], json!(["instructions/AGENTS.md"]));
        assert_eq!(manifest["components"]["instructions"]["claude"], json!(["instructions/claude/AGENTS.md"]));
        assert_eq!(manifest["components"]["instructions"]["codex"], json!(["instructions/codex/AGENTS.md"]));
    }

    #[test]
    fn a_blank_provider_variant_is_omitted_entirely() {
        // An empty-string variant (e.g. left over from a UI field that was
        // added then cleared) must not produce an empty instructions file
        // or an empty manifest entry.
        let mut bundle = make_bundle("Default.", "[]", "[]", "[]");
        bundle.instructions_by_provider = r#"{"claude":"   "}"#.to_string();
        let export = export_bundle(&bundle, &[]);
        assert!(!export.files.iter().any(|f| f.path.starts_with("instructions/claude/")));
        let manifest_file = export.files.iter().find(|f| f.path == "bundle.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).unwrap();
        assert!(manifest["components"]["instructions"].get("claude").is_none());
    }

    #[test]
    fn colliding_provider_keys_after_sanitization_do_not_overwrite_each_other() {
        // reagent P2, PR #2523: "claude" and "./claude" both sanitize to
        // the same output path — the second one must be skipped with a
        // warning, not silently overwrite the first (or worse, produce a
        // manifest listing the same path twice with ambiguous content).
        let mut bundle = make_bundle("Default.", "[]", "[]", "[]");
        bundle.instructions_by_provider =
            r#"{"claude":"First.","./claude":"Second."}"#.to_string();
        let export = export_bundle(&bundle, &[]);

        let claude_files: Vec<_> = export.files.iter().filter(|f| f.path == "instructions/claude/AGENTS.md").collect();
        assert_eq!(claude_files.len(), 1, "must not produce two files at the same path");
        // "./claude" sorts before "claude" byte-wise ('.' < 'c'), so it's
        // processed first and wins; "claude" is the one skipped as a
        // duplicate. The exact winner is an implementation detail — what
        // matters is that it's deterministic and there's only one.
        assert_eq!(claude_files[0].content, "Second.");

        let manifest_file = export.files.iter().find(|f| f.path == "bundle.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).unwrap();
        assert_eq!(
            manifest["components"]["instructions"]["claude"],
            json!(["instructions/claude/AGENTS.md"]),
            "must not list the same path twice"
        );
        assert!(export.warnings.iter().any(|w| w.contains("normalizes to the same path")));
    }

    #[test]
    fn manifest_references_the_skill_directory_not_the_skill_md_file() {
        // Codex P1, PR #2325: the ABF spec's manifest example references
        // "skills/<slug>" (the directory), not "skills/<slug>/SKILL.md".
        let bundle = make_bundle("", "[]", "[]", r#"["skill-a"]"#);
        let export = export_bundle(&bundle, &[make_agent_skill("Deploy Checklist")]);
        let manifest_file = export.files.iter().find(|f| f.path == "bundle.json").unwrap();
        let manifest: Value = serde_json::from_str(&manifest_file.content).unwrap();
        let skills = manifest["components"]["skills"].as_array().unwrap();
        assert_eq!(skills, &vec![json!("skills/deploy-checklist")]);
        // The actual file on disk still lives at the nested SKILL.md path.
        assert!(export.files.iter().any(|f| f.path == "skills/deploy-checklist/SKILL.md"));
    }

    #[test]
    fn writes_an_abf_v0_3_manifest_with_a_suggested_for_hint() {
        let mut bundle = make_bundle("Be concise.", "[]", "[]", "[]");
        bundle.provider = "codex".into();
        bundle.model = "openai".into();
        let export = export_bundle(&bundle, &[]);
        let manifest_file = export.files.iter().find(|f| f.path == "bundle.json").expect("bundle.json");
        assert!(!export.files.iter().any(|f| f.path == "armory.json"));
        let manifest: Value = serde_json::from_str(&manifest_file.content).unwrap();
        assert_eq!(manifest["$schema"], "https://docs.agentmux.ai/schemas/agent-bundle/v0.3/bundle.schema.json");
        assert_eq!(manifest["suggestedFor"], json!({ "provider": "codex", "vendor": "openai" }));
        assert!(manifest.get("provider").is_none() && manifest.get("model").is_none());
    }

    #[test]
    fn omits_the_hint_when_the_bundle_has_none() {
        let export = export_bundle(&make_bundle("x", "[]", "[]", "[]"), &[]);
        let manifest: Value = serde_json::from_str(&export.files.iter().find(|f| f.path == "bundle.json").unwrap().content).unwrap();
        assert!(manifest.get("suggestedFor").is_none());
    }

    #[test]
    fn a_v0_3_export_round_trips_through_import() {
        let mut bundle = make_bundle("Be concise.", "[]", "[]", "[]");
        bundle.provider = "gemini".into();
        bundle.model = "google".into();
        let export = export_bundle(&bundle, &[]);
        let files: Vec<crate::backend::bundle_import::BundleImportFile> = export
            .files
            .iter()
            .map(|f| crate::backend::bundle_import::BundleImportFile { path: f.path.clone(), content: f.content.clone() })
            .collect();
        let parsed = crate::backend::bundle_import::parse_bundle_import(&files).unwrap();
        assert_eq!((parsed.provider.as_str(), parsed.model.as_str()), ("gemini", "google"));
        assert!(parsed.instructions.contains("Be concise."));
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    }

    #[test]
    fn empty_bundle_still_produces_a_valid_manifest() {
        let bundle = make_bundle("", "[]", "[]", "[]");
        let export = export_bundle(&bundle, &[]);
        // bundle.json is always written, even for a fully empty bundle.
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
            if file.name() == "backend-dev-bundle/bundle.json" {
                found = true;
            }
        }
        assert!(found, "expected backend-dev-bundle/bundle.json in the archive");
    }
}
