// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `parse_bundle_import_with_budget`, one function each, in the order they ran: the
//! manifest, then each component category (instructions, skills, MCP servers, account requirements,
//! the memory note, project instructions). Moved out of it without a change in behaviour, so every
//! warning is still pushed in the same order. A child module of `bundle_import.rs`, whose private
//! helpers and caps these use.

use super::*;

type Components<'a> = Option<&'a serde_json::Map<String, Value>>;
type FilesByPath<'a> = HashMap<String, &'a str>;

/// The manifest file the bundle carries and its parsed JSON. Structural failures reject the import.
pub(super) fn read_manifest(by_path: &FilesByPath, warnings: &mut WarningSink) -> Result<(&'static str, Value), String> {
    let manifest_name = manifest_file_name(|name| by_path.contains_key(name))
        .ok_or_else(|| format!("{MANIFEST_FILE}: missing from bundle (or {LEGACY_MANIFEST_FILE}, before ABF v0.3)"))?;
    if manifest_name == MANIFEST_FILE && by_path.contains_key(LEGACY_MANIFEST_FILE) {
        warnings.push(format!("{LEGACY_MANIFEST_FILE}: ignored; this bundle also has {MANIFEST_FILE}"));
    }
    let manifest_raw = by_path[manifest_name];
    let manifest: Value = serde_json::from_str(manifest_raw)
        .map_err(|e| format!("{manifest_name}: malformed JSON ({e})"))?;
    Ok((manifest_name, manifest))
}

/// What the manifest says about the bundle itself.
pub(super) struct ManifestHeader {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) provider: String,
    pub(super) model: String,
}

/// The bundle's name, description and the provider and model it was made for (a hint only).
pub(super) fn manifest_header(manifest: &Value, manifest_name: &str, warnings: &mut WarningSink) -> ManifestHeader {
    // Phase 3 spec §3.1, round 13: bound at the parse source, not at a
    // later response boundary — `name` is re-submitted verbatim as
    // `bundle_name` when a user doesn't edit the preview's suggested value
    // (§3.2, round 11), so `preview` and `commit` (which independently
    // re-parse) must converge on the identical canonical value
    // deterministically. There is no separate "full" name anywhere for a
    // display-only truncation to lose.
    let raw_name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("imported-bundle");
    if raw_name.chars().count() > MAX_BUNDLE_NAME_CHARS {
        warnings.push(format!(
            "{manifest_name}: name exceeds {MAX_BUNDLE_NAME_CHARS} characters; truncated"
        ));
    }
    let name = bound_bundle_name(raw_name);
    let description = manifest
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Who the bundle was made for, a hint only: v0.3's `suggestedFor`
    // (`vendor` is what `model` always held), or v0.1–v0.2's top-level
    // `provider` / `model`. Absent or non-string reads as "not set."
    let hint = |v03: &str, legacy: &str| {
        manifest
            .get("suggestedFor")
            .and_then(|s| s.get(v03))
            .or_else(|| manifest.get(legacy))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let provider = hint("provider", "provider");
    let model = hint("vendor", "model");
    ManifestHeader { name, description, provider, model }
}

/// Record a missing or unrecognised `$schema` as a warning; it never blocks the import.
pub(super) fn warn_on_schema(manifest: &Value, manifest_name: &str, warnings: &mut WarningSink) {
    // §4.3.2: schema/version are recorded (via warnings, since ParsedBundleImport
    // has no dedicated field for them yet — no schema registry exists to
    // validate against) but never block the import.
    if manifest.get("$schema").is_none() {
        warnings.push(format!("{manifest_name}: no $schema field present"));
    } else if abf_format_version(manifest).is_none() {
        warnings.push(format!("{manifest_name}: $schema names no ABF version this AgentMux knows; reading it anyway"));
    }
    // `version` is the bundle's own content version, which says nothing
    // about the format (that's `$schema`, above), so it's never checked.
}

pub(super) struct ParsedInstructions {
    pub(super) instructions: String,
    pub(super) instructions_by_provider: HashMap<String, String>,
    pub(super) context_files: Vec<ImportedContextFile>,
}

pub(super) fn parse_instructions(components: Components, by_path: &FilesByPath, warnings: &mut WarningSink) -> ParsedInstructions {
    // ------------------------------------------------------------------
    // instructions — every path in components.instructions, concatenated
    // in manifest order for AGENTS.md-shaped entries; instructions/context/*
    // paths become context_files entries instead.
    // ------------------------------------------------------------------
    let mut context_files: Vec<ImportedContextFile> = Vec::new();
    let mut instructions = String::new();
    let mut instructions_by_provider: HashMap<String, String> = HashMap::new();
    // ABF v0.2 §2.2: components.instructions is EITHER a flat array (v0.1
    // shape, treated as an implicit "default") OR an object keyed by
    // provider id (v0.2 shape, "default" plus zero or more provider-scoped
    // variants). No merge decision happens here — every variant is stored
    // verbatim; selecting one at launch time is a separate, not-yet-built
    // materializer's job (see the spec's non-goals).
    // Shared across every components.instructions array parsed below
    // (default + every provider variant) — see
    // parse_instruction_component_paths's doc comment for why this must
    // NOT be reset per-array (reagent P1, PR #2523).
    let mut seen_instruction_paths: HashSet<String> = HashSet::new();
    let mut duplicate_instruction_refs: u32 = 0;
    match components.and_then(|c| c.get("instructions")) {
        Some(Value::Array(arr)) => {
            instructions = parse_instruction_component_paths(
                arr, "instructions", by_path, &mut context_files, true,
                &mut seen_instruction_paths, &mut duplicate_instruction_refs, warnings,
            );
        }
        Some(Value::Object(obj)) => {
            if obj.len() > MAX_INSTRUCTION_PROVIDER_VARIANTS {
                warnings.push(format!(
                    "components.instructions: {} provider variants exceeds the limit ({MAX_INSTRUCTION_PROVIDER_VARIANTS}); only the first {MAX_INSTRUCTION_PROVIDER_VARIANTS} are used",
                    obj.len()
                ));
            }
            // "default" must be parsed FIRST, regardless of the manifest's
            // own key order (serde_json::Map without the preserve_order
            // feature iterates alphabetically — "claude" < "default" —
            // not insertion order). The shared dedup set means whichever
            // array reaches a given instructions/context/* path first
            // decides whether it's diverted; parsing a provider variant
            // first would let it silently claim a shared context-file
            // path as plain instructions text before "default" ever gets
            // a chance to divert it correctly.
            let mut ordered: Vec<(&String, &Value)> = Vec::with_capacity(obj.len());
            if let Some(default_entry) = obj.get_key_value("default") {
                ordered.push(default_entry);
            }
            ordered.extend(obj.iter().filter(|(k, _)| *k != "default"));
            for (key, val) in ordered.into_iter().take(MAX_INSTRUCTION_PROVIDER_VARIANTS) {
                let label = format!("instructions.{key}");
                let Some(arr) = val.as_array() else {
                    warnings.push(format!("components.{label}: expected an array; skipped"));
                    continue;
                };
                // Only the "default" array diverts instructions/context/*
                // references into context_files — a provider variant's
                // array never does (reagent P1, PR #2523; see this
                // function's own doc comment for the reserved-word
                // collision this closes).
                let divert_context_files = key == "default";
                let joined = parse_instruction_component_paths(
                    arr, &label, by_path, &mut context_files, divert_context_files,
                    &mut seen_instruction_paths, &mut duplicate_instruction_refs, warnings,
                );
                if key == "default" {
                    instructions = joined;
                } else {
                    instructions_by_provider.insert(key.clone(), joined);
                }
            }
        }
        Some(_) => warnings.push("components.instructions: expected an array or object; skipped".to_string()),
        None => {}
    }
    if duplicate_instruction_refs > 0 {
        warnings.push(format!(
            "components.instructions: {duplicate_instruction_refs} duplicate reference(s) skipped"
        ));
    }
    ParsedInstructions { instructions, instructions_by_provider, context_files }
}

pub(super) fn parse_skills(components: Components, by_path: &FilesByPath, warnings: &mut WarningSink) -> (Vec<ParsedSkill>, Vec<String>) {
    // ------------------------------------------------------------------
    // skills — every directory in components.skills, reading <dir>/SKILL.md
    // ------------------------------------------------------------------
    let mut skills: Vec<ParsedSkill> = Vec::new();
    let mut skipped_skills: Vec<String> = Vec::new();
    let mut seen_skill_dirs: HashSet<String> = HashSet::new();
    let mut duplicate_skill_refs: u32 = 0;
    let dirs = capped_component_array(
        components.and_then(|c| c.get("skills")).and_then(|v| v.as_array()).map(|v| v.as_slice()),
        "skills",
        warnings,
    );
    for dir_val in dirs {
        let Some(raw_dir) = dir_val.as_str() else {
            warnings.push("components.skills: non-string entry skipped".to_string());
            continue;
        };
        // codex P2, PR #2379 round 4: same normalization as
        // components.instructions above.
        let Some(dir) = sanitize_context_relative_path(raw_dir) else {
            warnings.push(format!("components.skills: \"{raw_dir}\" is not a safe path; skipped"));
            continue;
        };
        // codex P1, PR #2379 round 6: bounded the same way as
        // components.instructions above.
        if !seen_skill_dirs.insert(dir.clone()) {
            duplicate_skill_refs += 1;
            continue;
        }
        let skill_md_path = format!("{}/SKILL.md", dir.trim_end_matches('/'));
        let Some(content) = by_path.get(skill_md_path.as_str()) else {
            warnings.push(format!("components.skills: \"{skill_md_path}\" not found; skipped"));
            skipped_skills.push(dir.clone());
            continue;
        };
        match parse_skill_md(content) {
            Some(mut skill) => {
                skill.source_dir = dir.clone();
                skills.push(skill);
            }
            None => {
                warnings.push(format!("{skill_md_path}: malformed SKILL.md frontmatter; skipped"));
                skipped_skills.push(dir.clone());
            }
        }
    }
    if duplicate_skill_refs > 0 {
        warnings.push(format!("components.skills: {duplicate_skill_refs} duplicate reference(s) skipped"));
    }
    // codex P1, PR #2379 round 7: bounds the RPC handler's WRITE side --
    // see MAX_IMPORTED_SKILLS's doc comment. The truncated skills still
    // count as "skipped" for reporting purposes, matching every other
    // skip reason in this loop.
    if skills.len() > MAX_IMPORTED_SKILLS {
        warnings.push(format!(
            "components.skills: {} skills exceeds the import limit ({MAX_IMPORTED_SKILLS}); only the first {MAX_IMPORTED_SKILLS} are imported",
            skills.len()
        ));
        skipped_skills.extend(skills.split_off(MAX_IMPORTED_SKILLS).into_iter().map(|s| s.slug));
    }
    (skills, skipped_skills)
}

pub(super) fn parse_mcp_servers(components: Components, by_path: &FilesByPath, warnings: &mut WarningSink) -> Vec<ParsedMcpServer> {
    // ------------------------------------------------------------------
    // mcp servers — every path in components.mcpServers, parsed as JSON
    // verbatim (still containing ${VAR} placeholders; resolution is the
    // RPC handler's job, §4.5).
    // ------------------------------------------------------------------
    let mut mcp_servers: Vec<ParsedMcpServer> = Vec::new();
    let mut seen_mcp_paths: HashSet<String> = HashSet::new();
    let mut duplicate_mcp_refs: u32 = 0;
    let paths = capped_component_array(
        components.and_then(|c| c.get("mcpServers")).and_then(|v| v.as_array()).map(|v| v.as_slice()),
        "mcpServers",
        warnings,
    );
    for path_val in paths {
        let Some(raw_path) = path_val.as_str() else {
            warnings.push("components.mcpServers: non-string entry skipped".to_string());
            continue;
        };
        // codex P2, PR #2379 round 4: same normalization as
        // components.instructions above.
        let Some(path) = sanitize_context_relative_path(raw_path) else {
            warnings.push(format!("components.mcpServers: \"{raw_path}\" is not a safe path; skipped"));
            continue;
        };
        // codex P1, PR #2379 round 6: same rejection as
        // components.instructions above -- requirements.json's raw
        // content is valid JSON and would otherwise parse cleanly
        // into an mcpServers entry, leaking it the same way.
        if is_requirements_json(&path) {
            warnings.push(format!(
                "components.mcpServers: \"{path}\" is the accounts/ requirements file; not readable as an MCP server config"
            ));
            continue;
        }
        // codex P1, PR #2379 round 6: bounded the same way as
        // components.instructions above.
        if !seen_mcp_paths.insert(path.clone()) {
            duplicate_mcp_refs += 1;
            continue;
        }
        let Some(content) = by_path.get(path.as_str()) else {
            warnings.push(format!("components.mcpServers: \"{path}\" not found; skipped"));
            continue;
        };
        match serde_json::from_str::<Value>(content) {
            Ok(config) => mcp_servers.push(ParsedMcpServer { source_path: path.clone(), config }),
            Err(e) => warnings.push(format!("{path}: malformed JSON ({e}); skipped")),
        }
    }
    if duplicate_mcp_refs > 0 {
        warnings.push(format!("components.mcpServers: {duplicate_mcp_refs} duplicate reference(s) skipped"));
    }
    mcp_servers
}

pub(super) fn parse_account_requirements(components: Components, by_path: &FilesByPath, warnings: &mut WarningSink) -> Vec<AccountRequirement> {
    // ------------------------------------------------------------------
    // accounts/requirements.json — read-only input to §4.5, never written
    // anywhere as-is.
    // ------------------------------------------------------------------
    let mut requirements: Vec<AccountRequirement> = Vec::new();
    if let Some(accounts_val) = components.and_then(|c| c.get("accounts")) {
        // reagent P2, PR #2379 round 7: every other component category
        // (instructions/skills/mcpServers non-string entries, an
        // unrecognized `version`) pushes an explicit warning when the
        // manifest's value has the wrong shape — this one silently
        // dropped a non-string `accounts` value with no warning at all,
        // inconsistent with the module's own "warn, don't silently drop"
        // philosophy.
        if let Some(raw_req_path) = accounts_val.as_str() {
            // reagent P2, PR #2379 round 6: this was the one remaining
            // manifest-reference lookup still comparing/using the raw,
            // un-normalized string -- the same bug class round 4 fixed for
            // components.instructions/skills/mcpServers, just missed here. A
            // valid non-canonical spelling (e.g. "./accounts/requirements.json")
            // failed the literal equality check, silently dropping legitimate
            // account requirements.
            let req_path = sanitize_context_relative_path(raw_req_path).filter(|p| is_requirements_json(p));
            if let Some(req_path) = &req_path {
                if let Some(content) = by_path.get(req_path.as_str()) {
                    #[derive(Deserialize)]
                    struct RequirementsDoc {
                        #[serde(default)]
                        requirements: Vec<AccountRequirement>,
                    }
                    match serde_json::from_str::<RequirementsDoc>(content) {
                        Ok(doc) => {
                            // codex P1, PR #2379 round 4: unbounded, the RPC
                            // handler's per-requirement account lookup becomes a
                            // synchronous store query per row — a permitted 10 MB
                            // JSON entry can hold tens/hundreds of thousands of
                            // (duplicate or distinct) requirements and keep that
                            // handler busy for a prolonged time. Bounding here
                            // protects the parse step itself; the handler
                            // separately dedupes by provider so its actual query
                            // count stays low even at this cap.
                            if doc.requirements.len() > MAX_ACCOUNT_REQUIREMENTS {
                                warnings.push(format!(
                                    "accounts/requirements.json: {} requirements exceeds the limit ({MAX_ACCOUNT_REQUIREMENTS}); only the first {MAX_ACCOUNT_REQUIREMENTS} are used",
                                    doc.requirements.len()
                                ));
                                requirements = doc.requirements.into_iter().take(MAX_ACCOUNT_REQUIREMENTS).collect();
                            } else {
                                requirements = doc.requirements;
                            }
                        }
                        Err(e) => warnings.push(format!("accounts/requirements.json: malformed JSON ({e}); ignored")),
                    }
                } else {
                    warnings.push("components.accounts references accounts/requirements.json, but it's not present in the bundle".to_string());
                }
            } else {
                warnings.push(format!(
                    "components.accounts: \"{raw_req_path}\" is not accounts/requirements.json; ignored per the accounts/ allowlist"
                ));
            }
        } else {
            warnings.push("components.accounts: non-string value skipped".to_string());
        }
    }
    requirements
}

pub(super) fn warn_memory_component(components: Components, warnings: &mut WarningSink) {
    // ABF v0.2 §2.3: `parse_bundle_import`/`parse_bundle_import_with_budget`
    // are called by every agent-LESS import path (bundle.import,
    // bundle.import.preview, bundle.import.commit) AND by
    // `bundle_import_for_agent_impl` (which reuses this same parser for
    // everything except memory, which it handles separately downstream).
    // A components.memory key gets a warning here so the three agent-less
    // callers can tell the memory component was present but requires
    // bundle.import_for_agent, not that it was missing from the source
    // bundle. reagent P1, PR #2527: `bundle_import_for_agent_impl` DOES
    // handle memory, so this warning is actively misleading there —
    // it must filter this exact string out of `parsed.warnings` before
    // merging into its own response (see MEMORY_COMPONENT_IGNORED_WARNING's
    // doc comment for why it's a shared constant, not a duplicated
    // literal, so the push site and the filter site can't drift apart).
    if components.and_then(|c| c.get("memory")).is_some() {
        warnings.push(MEMORY_COMPONENT_IGNORED_WARNING.to_string());
    }
}

pub(super) fn parse_project_instructions(components: Components, by_path: &FilesByPath, warnings: &mut WarningSink) -> Vec<ImportedProjectInstruction> {
    // Unlike memory, this warning has no filter site anywhere: NO import path
    // installs project instructions, agent-scoped or not. Those files belong
    // to the repository the bundle is being imported INTO, and writing them
    // would be exactly what SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md
    // exists to prevent — a portability feature turning into a back door
    // (SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md §5.2).
    //
    // The entries themselves ARE read back (Codex P1, PR #3163): a warning
    // saying "this bundle recorded some instructions" with no way to see them
    // is not surfacing anything, and `agent.project_instructions` can't fill
    // the gap because it scans the IMPORTING agent's working directory.
    // Reading is not applying — nothing downstream writes these.
    let mut project_instructions: Vec<ImportedProjectInstruction> = Vec::new();
    if let Some(raw) = components.and_then(|c| c.get("projectInstructions")) {
        warnings.push(PROJECT_INSTRUCTIONS_NOT_APPLIED_WARNING.to_string());
        match raw.as_array() {
            Some(entries) => {
                if entries.len() > MAX_IMPORTED_PROJECT_INSTRUCTIONS {
                    warnings.push(format!(
                        "components.projectInstructions: {} entries exceeds the \
                         {MAX_IMPORTED_PROJECT_INSTRUCTIONS}-entry cap; only the \
                         first {MAX_IMPORTED_PROJECT_INSTRUCTIONS} were read",
                        entries.len()
                    ));
                }
                for entry in entries.iter().take(MAX_IMPORTED_PROJECT_INSTRUCTIONS) {
                    if let Some(parsed) = project_instruction_entry(entry, by_path, warnings) {
                        project_instructions.push(parsed);
                    }
                }
            }
            None => warnings.push(
                "components.projectInstructions: not an array; ignored".to_string(),
            ),
        }
    }
    project_instructions
}

/// One `components.projectInstructions` entry, or `None` (with a warning) when it can't be read.
fn project_instruction_entry(entry: &Value, by_path: &FilesByPath, warnings: &mut WarningSink) -> Option<ImportedProjectInstruction> {
    let Some(obj) = entry.as_object() else {
        warnings.push(
            "components.projectInstructions: non-object entry skipped"
                .to_string(),
        );
        return None;
    };
    let str_field = |k: &str| {
        obj.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
    };
    let raw_file = str_field("file");
    if raw_file.is_empty() {
        warnings.push(
            "components.projectInstructions: entry has no \"file\"; skipped"
                .to_string(),
        );
        return None;
    }
    // Normalize the manifest's OWN reference the same way
    // `by_path`'s keys are normalized, exactly as every other
    // manifest-reference lookup in this file does (codex P2,
    // PR #2379 round 4; reagent P2, round 6; reagent P2 again
    // on PR #3163 — this is the third component to need it).
    // Without it a valid non-canonical spelling
    // (`./instructions/project/CLAUDE.md`, backslashes) misses
    // the lookup and gets reported as "not found" even though
    // the content is right there.
    let Some(file_path) = sanitize_context_relative_path(&raw_file) else {
        warnings.push(format!(
            "components.projectInstructions: \"{raw_file}\" is not a \
             safe path; skipped"
        ));
        return None;
    };
    // The accounts/ allowlist applies here for the same reason
    // it applies to components.instructions: a manifest is not
    // a trustworthy inventory of its own bundle, and a
    // projectInstructions entry must not become a second way to
    // read accounts/requirements.json back out as content.
    // Checked on the NORMALIZED path so a non-canonical
    // spelling can't slip past it either.
    if is_requirements_json(&file_path) {
        warnings.push(format!(
            "components.projectInstructions: \"{raw_file}\" is the \
             accounts/ requirements file; not readable as instructions"
        ));
        return None;
    }
    let Some(content) = by_path.get(file_path.as_str()) else {
        warnings.push(format!(
            "components.projectInstructions: \"{raw_file}\" not found \
             among the bundle's files; skipped"
        ));
        return None;
    };
    Some(ImportedProjectInstruction {
        path: str_field("path"),
        file: file_path,
        content_hash: str_field("contentHash"),
        owner: str_field("owner"),
        content: content.to_string(),
    })
}
