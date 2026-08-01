// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Armory Bundle Format (ABF) importer — Phase 2 of
//! `docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md`.
//! Inverse of `bundle_export.rs`: takes a bundle's files (either a raw
//! `[{path, content}]` list or an unpacked `.abf` zip archive) and
//! validates + parses them into data ready for the `bundle.import` RPC
//! handler to write to the Store.
//!
//! Pure functions only — no I/O, no Store access, no account resolution
//! (account resolution needs `db_accounts`, which only the RPC handler has
//! access to; see the spec's §4.5). Mirrors `bundle_export.rs`'s shape and
//! quality bar deliberately: the same "warn, don't silently drop" and
//! "reject the whole thing on structural failure, but never partially
//! import on a per-entry problem" philosophy, since the two modules are
//! meant to round-trip against each other.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::bundle_export::sanitize_context_relative_path;

/// One file read from an import source (zip or raw list), path relative
/// to the bundle root. Mirrors `bundle_export::BundleExportFile`.
#[derive(Debug, Clone)]
pub struct BundleImportFile {
    pub path: String,
    pub content: String,
}

/// A skill parsed out of a `skills/<slug>/SKILL.md` file — the inverse of
/// `agent_config::render_skill_md`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ParsedSkill {
    /// The slug from SKILL.md's own `name` field (Agent Skills spec:
    /// must match the parent directory) — reused as both the imported
    /// skill's display name and its `trigger`. Agent-skill-type skills
    /// don't materialize a slash-command file from `trigger` (see
    /// `agent_config.rs`'s `buildConfigFiles`), so this is a safe,
    /// deterministic default rather than a meaningful distinct value.
    pub slug: String,
    pub description: String,
    pub content: String,
}

/// One `accounts/requirements.json` entry — mirrors the shape
/// `bundle_export.rs` writes (research report §5.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountRequirement {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub optional: bool,
}

/// One `instructions/context/*` file, in the `[{path, content}]` shape
/// `db_bundles.context_files` stores.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportedContextFile {
    pub path: String,
    pub content: String,
}

/// Parsed, validated bundle content ready for the RPC handler to resolve
/// accounts against and write to the Store. `mcp_servers` still contains
/// whatever `${VAR}`-style placeholders the exporter's redaction left in
/// place — account resolution (§4.5, RPC-handler-side) substitutes real
/// account bindings where exactly one match is found and leaves the rest
/// untouched.
#[derive(Debug, Clone, Serialize)]
pub struct ParsedBundleImport {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub context_files: Vec<ImportedContextFile>,
    pub mcp_servers: Vec<Value>,
    pub skills: Vec<ParsedSkill>,
    pub skipped_skills: Vec<String>,
    pub requirements: Vec<AccountRequirement>,
    pub warnings: Vec<String>,
}

/// Parse and validate a bundle's files into [`ParsedBundleImport`].
/// Structural failures (missing/malformed `armory.json`) reject the whole
/// import — `Err` — since there is nothing safe to partially write.
/// Per-entry problems (a missing referenced file, an unsafe path, a
/// malformed SKILL.md) degrade to a warning and that entry is skipped —
/// matches `bundle_export.rs`'s own philosophy, and lets a lossy import
/// still produce a usable bundle rather than an all-or-nothing failure on
/// e.g. one corrupt skill among five good ones.
pub fn parse_bundle_import(files: &[BundleImportFile]) -> Result<ParsedBundleImport, String> {
    let by_path: HashMap<&str, &str> = files
        .iter()
        .map(|f| (f.path.as_str(), f.content.as_str()))
        .collect();

    let manifest_raw = by_path
        .get("armory.json")
        .ok_or_else(|| "armory.json: missing from bundle".to_string())?;
    let manifest: Value = serde_json::from_str(manifest_raw)
        .map_err(|e| format!("armory.json: malformed JSON ({e})"))?;

    let mut warnings: Vec<String> = Vec::new();

    // §4.3.5: reject anything under accounts/ other than requirements.json
    // outright — never read, never write, never even acknowledged beyond a
    // warning. Checked against every file actually present, not just what
    // the manifest references, since a malicious/malformed bundle's
    // `components` object is not a trustworthy inventory of its own
    // contents.
    for file in files {
        if file.path.starts_with("accounts/") && file.path != "accounts/requirements.json" {
            warnings.push(format!(
                "{}: rejected — only accounts/requirements.json is ever read from the accounts/ directory",
                file.path
            ));
        }
    }

    let name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("imported-bundle")
        .to_string();
    let description = manifest
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // §4.3.2: schema/version are recorded (via warnings, since ParsedBundleImport
    // has no dedicated field for them yet — no schema registry exists to
    // validate against) but never block the import.
    if manifest.get("$schema").is_none() {
        warnings.push("armory.json: no $schema field present".to_string());
    }
    match manifest.get("version").and_then(|v| v.as_str()) {
        Some(v) if v.starts_with("0.1") => {}
        Some(other) => warnings.push(format!(
            "armory.json: version \"{other}\" is not a recognized ABF v0.1.x version; importing anyway"
        )),
        None => warnings.push("armory.json: no version field present".to_string()),
    }

    let components = manifest.get("components").and_then(|v| v.as_object());

    // ------------------------------------------------------------------
    // instructions — every path in components.instructions, concatenated
    // in manifest order for AGENTS.md-shaped entries; instructions/context/*
    // paths become context_files entries instead.
    // ------------------------------------------------------------------
    let mut instructions_parts: Vec<String> = Vec::new();
    let mut context_files: Vec<ImportedContextFile> = Vec::new();
    if let Some(paths) = components.and_then(|c| c.get("instructions")).and_then(|v| v.as_array()) {
        for path_val in paths {
            let Some(path) = path_val.as_str() else {
                warnings.push("components.instructions: non-string entry skipped".to_string());
                continue;
            };
            let Some(content) = by_path.get(path) else {
                warnings.push(format!("components.instructions: \"{path}\" not found among the bundle's files; skipped"));
                continue;
            };
            if let Some(rel) = path.strip_prefix("instructions/context/") {
                match sanitize_context_relative_path(rel) {
                    Some(safe_rel) => context_files.push(ImportedContextFile {
                        path: safe_rel,
                        content: content.to_string(),
                    }),
                    None => warnings.push(format!(
                        "{path}: not a safe relative path under instructions/context/; skipped"
                    )),
                }
            } else {
                instructions_parts.push(content.to_string());
            }
        }
    }
    let instructions = instructions_parts.join("\n\n---\n\n");

    // ------------------------------------------------------------------
    // skills — every directory in components.skills, reading <dir>/SKILL.md
    // ------------------------------------------------------------------
    let mut skills: Vec<ParsedSkill> = Vec::new();
    let mut skipped_skills: Vec<String> = Vec::new();
    if let Some(dirs) = components.and_then(|c| c.get("skills")).and_then(|v| v.as_array()) {
        for dir_val in dirs {
            let Some(dir) = dir_val.as_str() else {
                warnings.push("components.skills: non-string entry skipped".to_string());
                continue;
            };
            let skill_md_path = format!("{}/SKILL.md", dir.trim_end_matches('/'));
            let Some(content) = by_path.get(skill_md_path.as_str()) else {
                warnings.push(format!("components.skills: \"{skill_md_path}\" not found; skipped"));
                skipped_skills.push(dir.to_string());
                continue;
            };
            match parse_skill_md(content) {
                Some(skill) => skills.push(skill),
                None => {
                    warnings.push(format!("{skill_md_path}: malformed SKILL.md frontmatter; skipped"));
                    skipped_skills.push(dir.to_string());
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // mcp servers — every path in components.mcpServers, parsed as JSON
    // verbatim (still containing ${VAR} placeholders; resolution is the
    // RPC handler's job, §4.5).
    // ------------------------------------------------------------------
    let mut mcp_servers: Vec<Value> = Vec::new();
    if let Some(paths) = components.and_then(|c| c.get("mcpServers")).and_then(|v| v.as_array()) {
        for path_val in paths {
            let Some(path) = path_val.as_str() else {
                warnings.push("components.mcpServers: non-string entry skipped".to_string());
                continue;
            };
            let Some(content) = by_path.get(path) else {
                warnings.push(format!("components.mcpServers: \"{path}\" not found; skipped"));
                continue;
            };
            match serde_json::from_str::<Value>(content) {
                Ok(v) => mcp_servers.push(v),
                Err(e) => warnings.push(format!("{path}: malformed JSON ({e}); skipped")),
            }
        }
    }

    // ------------------------------------------------------------------
    // accounts/requirements.json — read-only input to §4.5, never written
    // anywhere as-is.
    // ------------------------------------------------------------------
    let mut requirements: Vec<AccountRequirement> = Vec::new();
    if let Some(req_path) = components.and_then(|c| c.get("accounts")).and_then(|v| v.as_str()) {
        if req_path != "accounts/requirements.json" {
            warnings.push(format!(
                "components.accounts: \"{req_path}\" is not accounts/requirements.json; ignored per the accounts/ allowlist"
            ));
        } else if let Some(content) = by_path.get(req_path) {
            #[derive(Deserialize)]
            struct RequirementsDoc {
                #[serde(default)]
                requirements: Vec<AccountRequirement>,
            }
            match serde_json::from_str::<RequirementsDoc>(content) {
                Ok(doc) => requirements = doc.requirements,
                Err(e) => warnings.push(format!("accounts/requirements.json: malformed JSON ({e}); ignored")),
            }
        } else {
            warnings.push("components.accounts references accounts/requirements.json, but it's not present in the bundle".to_string());
        }
    }

    Ok(ParsedBundleImport {
        name,
        description,
        instructions,
        context_files,
        mcp_servers,
        skills,
        skipped_skills,
        requirements,
        warnings,
    })
}

/// Parse a SKILL.md file (`render_skill_md`'s exact output shape, and
/// nothing more general — this is not a YAML parser, just the inverse of
/// the one writer this codebase has). Expects exactly:
/// `---\nname: "<json-string>"\ndescription: "<json-string>"\n---\n\n<body>`.
/// Returns `None` for anything that doesn't match — a real Agent Skills
/// SKILL.md from elsewhere in the ecosystem may use additional frontmatter
/// fields or different quoting; broadening this to a general YAML parser
/// is a follow-up, not silently guessed at here (mirrors the exporter's
/// own documented `server.json` scope decision).
fn parse_skill_md(content: &str) -> Option<ParsedSkill> {
    let rest = content.strip_prefix("---\n")?;
    let (frontmatter, body) = rest.split_once("\n---\n\n")?;
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in frontmatter.lines() {
        let (key, value) = line.split_once(": ")?;
        let parsed: String = serde_json::from_str(value).ok()?;
        match key {
            "name" => name = Some(parsed),
            "description" => description = Some(parsed),
            _ => {} // unrecognized frontmatter field — ignore, don't fail
        }
    }
    Some(ParsedSkill {
        slug: name?,
        // Required, not defaulted: render_skill_md always writes both
        // fields (falling back to a placeholder string, never omitting
        // the key), so a real exported SKILL.md always has both present.
        // A file missing `description` entirely isn't this writer's
        // output shape — reject it the same as a missing `name`.
        description: description?,
        content: body.to_string(),
    })
}

/// Unpack a `.abf` zip archive into a flat file list, applying the same
/// path-safety check as everywhere else in this module to every entry
/// name before it's trusted (§4.3.4) — a zip's own internal paths are
/// exactly as untrusted as any other part of the archive's content.
/// Directory entries are skipped; anything that fails the safety check is
/// dropped with a warning rather than surfaced as a file.
pub fn unzip_bundle_import(zip_bytes: &[u8]) -> Result<(Vec<BundleImportFile>, Vec<String>), String> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("not a valid zip archive: {e}"))?;
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip archive: failed to read entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry.name().to_string();
        // `zip_bundle_export` wraps every entry in a single top-level
        // `<root_slug>/` directory (mirrors the spec's §5.1 on-disk
        // layout, where the bundle-slug directory IS the archive root) —
        // strip that one wrapping component before treating the rest as
        // the bundle-relative path. An entry with no `/` at all (no
        // wrapping directory) is kept as-is rather than rejected, so a
        // hand-built `.abf` without the convention still round-trips.
        let relative_name = raw_name.split_once('/').map(|(_, rest)| rest).unwrap_or(&raw_name);
        let Some(safe_name) = sanitize_context_relative_path(relative_name) else {
            warnings.push(format!("{raw_name}: not a safe path; skipped"));
            continue;
        };
        if !seen.insert(safe_name.clone()) {
            warnings.push(format!("{safe_name}: duplicate entry in archive; first occurrence kept"));
            continue;
        }
        let mut content = String::new();
        if let Err(e) = entry.read_to_string(&mut content) {
            warnings.push(format!("{safe_name}: not valid UTF-8 text; skipped ({e})"));
            continue;
        }
        out.push(BundleImportFile { path: safe_name, content });
    }
    Ok((out, warnings))
}

#[cfg(test)]
mod tests {
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
        assert_eq!(result.mcp_servers[0]["env"]["GITHUB_TOKEN"], "${GITHUB_TOKEN}");
    }

    #[test]
    fn parses_account_requirements_read_only() {
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
    fn warns_on_unrecognized_version_but_does_not_fail() {
        let manifest = serde_json::to_string(&serde_json::json!({
            "name": "test-bundle",
            "version": "2.0.0",
            "components": {},
        })).unwrap();
        let files = vec![file("armory.json", &manifest)];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("version")));
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
        use crate::backend::storage::store::Memory;

        let bundle = Memory {
            id: "b1".to_string(),
            name: "Zip Roundtrip".to_string(),
            description: "desc".to_string(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: "Be helpful.".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        let export = super::super::bundle_export::export_bundle(&bundle, &[]);
        let zip_bytes = super::super::bundle_export::zip_bundle_export(&export).unwrap();

        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(warnings.is_empty());
        assert!(files.iter().any(|f| f.path == "armory.json"));
        assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md" && f.content == "Be helpful."));

        let parsed = parse_bundle_import(&files).unwrap();
        assert_eq!(parsed.instructions, "Be helpful.");
    }

    #[test]
    fn unzip_rejects_a_non_zip_input() {
        let err = unzip_bundle_import(b"not a zip file").unwrap_err();
        assert!(err.contains("not a valid zip archive"));
    }
}
