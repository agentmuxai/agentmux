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
    let mut warnings: Vec<String> = Vec::new();

    // §4.3.5: reject anything under accounts/ other than requirements.json
    // outright — never read, never write, never even acknowledged beyond a
    // warning. Checked against every file actually present, not just what
    // the manifest references, since a malicious/malformed bundle's
    // `components` object is not a trustworthy inventory of its own
    // contents.
    //
    // Codex P1, PR #2379: a rejected file must be excluded from `by_path`
    // entirely, not merely warned about — otherwise a malicious bundle
    // whose `components.instructions`/`components.mcpServers` REFERENCES
    // `accounts/secrets.json` would still have that content looked up and
    // folded into the imported bundle by the code below (and, on a later
    // re-export of that same bundle, written unredacted straight into
    // `instructions/AGENTS.md`) — defeating the accounts/ allowlist this
    // exact loop exists to enforce.
    // Codex P1, PR #2379 (round 2): the accounts/ allowlist check above
    // only ever saw a ZIP entry's already-normalized path (unzip_bundle_import
    // runs every entry through `sanitize_context_relative_path` first). The
    // raw `files` RPC input skips that step entirely, so a spelling like
    // `./accounts/secrets.json` or `accounts\secrets.json` doesn't match the
    // literal `starts_with("accounts/")` check and sails through unrejected
    // — while a manifest `components.*` entry using the exact same raw
    // spelling still resolves it via `by_path.get(path)` below. Normalize
    // every file's path the same way regardless of source (zip or raw
    // list) before the allowlist check and before it becomes a lookup key,
    // so both intake paths enforce the same rule on the same canonical form.
    let mut by_path: HashMap<String, &str> = HashMap::new();
    for f in files {
        let Some(safe_path) = sanitize_context_relative_path(&f.path) else {
            warnings.push(format!("{}: not a safe path; skipped", f.path));
            continue;
        };
        if safe_path.starts_with("accounts/") && safe_path != "accounts/requirements.json" {
            warnings.push(format!(
                "{safe_path}: rejected — only accounts/requirements.json is ever read from the accounts/ directory"
            ));
            continue;
        }
        by_path.insert(safe_path, f.content.as_str());
    }

    let manifest_raw = by_path
        .get("armory.json")
        .ok_or_else(|| "armory.json: missing from bundle".to_string())?;
    let manifest: Value = serde_json::from_str(manifest_raw)
        .map_err(|e| format!("armory.json: malformed JSON ({e})"))?;

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

/// Per-entry decompressed-size cap. Generous for any legitimate
/// instructions/SKILL.md/context file (10 MB of text is already an
/// absurdly large single bundle component) while bounding how much a
/// single malicious entry can force into memory.
const MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 10 * 1024 * 1024;

/// Aggregate decompressed-size cap across the whole archive. Bounds a
/// zip-bomb-style archive built from many medium-sized entries that would
/// each individually pass [`MAX_ENTRY_UNCOMPRESSED_BYTES`].
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 50 * 1024 * 1024;

/// Maximum number of entries an archive may contain. A legitimate bundle
/// (instructions, a handful of context files, a handful of skills, one
/// manifest) never comes close to this; it exists purely to bound the CPU
/// cost of iterating + path-sanitizing + hashing every entry, independent
/// of the per-entry/aggregate byte caps below (reagent P2, PR #2379 round
/// 2) — a crafted archive of many tiny/valid entries passes both size caps
/// while still forcing unbounded iteration work.
const MAX_ENTRY_COUNT: usize = 10_000;

/// Unpack a `.abf` zip archive into a flat file list, applying the same
/// path-safety check as everywhere else in this module to every entry
/// name before it's trusted (§4.3.4) — a zip's own internal paths are
/// exactly as untrusted as any other part of the archive's content.
/// Directory entries are skipped; anything that fails the safety check is
/// dropped with a warning rather than surfaced as a file.
///
/// Codex P1, PR #2379: also bounds decompressed size, per-entry and in
/// aggregate — `read_to_string` alone has no size limit, so an untrusted
/// `.abf` containing a highly compressed entry (a classic zip-bomb shape;
/// DEFLATE alone permits ratios past 1000:1) could otherwise exhaust the
/// server process's memory through `bundle.import`. An oversized single
/// entry is skipped with a warning (matches this function's existing
/// per-entry-problem philosophy); an oversized AGGREGATE fails the whole
/// import — letting a partially-capped import through silently would be
/// more confusing than useful, and hitting the aggregate cap at all
/// already indicates a genuinely abusive archive rather than one bad file.
pub fn unzip_bundle_import(zip_bytes: &[u8]) -> Result<(Vec<BundleImportFile>, Vec<String>), String> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("not a valid zip archive: {e}"))?;

    if archive.len() > MAX_ENTRY_COUNT {
        return Err(format!(
            "zip archive: {} entries exceeds the limit ({MAX_ENTRY_COUNT}) — rejecting the whole import",
            archive.len()
        ));
    }

    // First pass: collect every non-dir entry's raw name, to DETECT
    // whether every single one shares one common wrapping directory
    // (`zip_bundle_export`'s convention: `<root_slug>/armory.json`, etc.)
    // before deciding whether to strip a path component at all.
    //
    // reagent P1, PR #2379: the previous version unconditionally stripped
    // the first `/`-delimited segment of EVERY entry, on the assumption a
    // wrapper always exists. A hand-built `.abf` zipped directly from the
    // loose directory tree (no wrapping folder) — which this spec's §2
    // explicitly requires an importer to also accept — has entries like
    // `instructions/AGENTS.md` already bundle-relative; blindly stripping
    // silently mangled it to `AGENTS.md`, breaking every `components.*`
    // path lookup in `parse_bundle_import` (each degrades to a "not
    // found" warning and the content is dropped) with no error surfaced.
    // Only strip when EVERY entry agrees on the same first segment.
    let mut raw_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip archive: failed to read entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        raw_names.push(entry.name().to_string());
    }
    let common_wrapper: Option<&str> = raw_names.first().and_then(|first| {
        let candidate = first.split_once('/').map(|(root, _)| root)?;
        raw_names
            .iter()
            .all(|n| n.split_once('/').map(|(root, _)| root == candidate).unwrap_or(false))
            .then_some(candidate)
    });
    let strip_len: Option<usize> = common_wrapper.map(|w| w.len() + 1); // +1 for the '/'

    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip archive: failed to read entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry.name().to_string();
        let relative_name = match strip_len {
            Some(n) if raw_name.len() > n => &raw_name[n..],
            _ => raw_name.as_str(),
        };
        let Some(safe_name) = sanitize_context_relative_path(relative_name) else {
            warnings.push(format!("{raw_name}: not a safe path; skipped"));
            continue;
        };
        if !seen.insert(safe_name.clone()) {
            warnings.push(format!("{safe_name}: duplicate entry in archive; first occurrence kept"));
            continue;
        }

        // Declared size check first — cheap, and avoids decompressing at
        // all for an entry that's already too large per its own metadata.
        // This is ONLY a cheap pre-filter: `entry.size()` is the zip's own
        // declared/attacker-controlled uncompressed size, not a verified
        // value, so it must never feed the aggregate cap below (reagent
        // P1, PR #2379 round 2) — an archive of many entries that each lie
        // with a tiny declared size while actually containing content up
        // to the per-entry cap would otherwise pass both checks here while
        // still exhausting memory once decompressed.
        let declared_size = entry.size();
        if declared_size > MAX_ENTRY_UNCOMPRESSED_BYTES {
            warnings.push(format!(
                "{safe_name}: declared uncompressed size ({declared_size} bytes) exceeds the per-entry limit ({MAX_ENTRY_UNCOMPRESSED_BYTES} bytes); skipped"
            ));
            continue;
        }

        // Hard backstop on the actual read, independent of the declared
        // size above: read at most one byte past the cap so an entry
        // whose real decompressed content exceeds what it declared is
        // still caught (rather than trusting zip metadata alone).
        let mut limited = entry.by_ref().take(MAX_ENTRY_UNCOMPRESSED_BYTES + 1);
        let mut buf: Vec<u8> = Vec::new();
        if let Err(e) = limited.read_to_end(&mut buf) {
            warnings.push(format!("{safe_name}: failed to read entry: {e}"));
            continue;
        }
        if buf.len() as u64 > MAX_ENTRY_UNCOMPRESSED_BYTES {
            warnings.push(format!(
                "{safe_name}: actual decompressed size exceeds the per-entry limit ({MAX_ENTRY_UNCOMPRESSED_BYTES} bytes), despite a smaller declared size; skipped"
            ));
            continue;
        }

        // Aggregate cap enforced against ACTUAL bytes read, not the
        // declared size — the only value that can't be lied about by the
        // archive's own metadata.
        total_uncompressed = total_uncompressed.saturating_add(buf.len() as u64);
        if total_uncompressed > MAX_TOTAL_UNCOMPRESSED_BYTES {
            return Err(format!(
                "zip archive: aggregate decompressed size exceeds the total limit ({MAX_TOTAL_UNCOMPRESSED_BYTES} bytes) — rejecting the whole import"
            ));
        }
        let content = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("{safe_name}: not valid UTF-8 text; skipped ({e})"));
                continue;
            }
        };
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
}
