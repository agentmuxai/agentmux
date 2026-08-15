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

use std::collections::{HashMap, HashSet};

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
    /// Non-fatal problems encountered while exporting: malformed source
    /// JSON (`context_files`/`mcp_servers`) that had to be treated as
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

/// Flag names (case/dash/underscore-insensitive, `-`/`--` prefix optional)
/// whose value is credential-shaped in common CLI/MCP server invocations —
/// a curated allowlist rather than a broad substring match (e.g. `--keymap`
/// must NOT trigger this). Used for BOTH `args` flag names AND `url` query
/// param names — a single shared list so the two can never drift apart
/// again (reagent P1, PR #2333: the two lists started separate and the
/// query-param one was missing several names the args one already had).
const SECRET_NAMES: &[&str] = &[
    "key", "api-key", "apikey", "api_key",
    "token", "access-token", "access_token", "auth-token", "auth_token",
    "bearer-token", "bearer_token",
    "secret", "secret-key", "secret_key", "client-secret", "client_secret",
    "password", "passwd",
];

fn secret_placeholder(name: &str) -> String {
    let normalized = name.trim_start_matches('-').to_uppercase().replace('-', "_");
    format!("${{{normalized}}}")
}

fn is_secret_name(name: &str) -> bool {
    let normalized = name.trim_start_matches('-').to_lowercase();
    SECRET_NAMES.contains(&normalized.as_str())
}

/// HTTP header names whose value is credential-shaped when embedded in a
/// `"HeaderName: value"` string passed as a CLI flag's argument (see
/// [`redact_header_value`]) — a separate curated list from `SECRET_NAMES`
/// because header names (`authorization`, `cookie`) and CLI flag names
/// (`api-key`, `token`) are different vocabularies that happen to overlap
/// only partially.
const SECRET_HEADER_NAMES: &[&str] = &[
    "authorization", "x-api-key", "x-auth-token", "proxy-authorization", "cookie",
];

fn is_secret_header_name(name: &str) -> bool {
    SECRET_HEADER_NAMES.contains(&name.trim().to_lowercase().as_str())
}

fn is_header_flag(flag: &str) -> bool {
    matches!(
        flag.trim_start_matches('-').to_lowercase().as_str(),
        "header" | "headers" | "h"
    )
}

/// Redact a `"HeaderName: value"` string -- the shape a `--header`/`-H`
/// flag's argument takes in common CLI/MCP server invocations -- when the
/// header name is secret-shaped, e.g. turning
/// `"Authorization: Bearer <token>"` into `"Authorization: ${AUTHORIZATION}"`.
/// The flag name itself (`--header`) is never secret-shaped, so the
/// `is_secret_name` flag check above can never catch this: the secret is
/// smuggled inside the *value*, not signaled by the flag (Codex + reagent
/// P0, PR #2333, flagged across multiple review rounds). Returns the
/// rebuilt string plus the header name to record in `requirements.json`,
/// or `None` if `value` isn't `name: value`-shaped or the name isn't a
/// recognized secret-bearing header.
fn redact_header_value(value: &str) -> Option<(String, String)> {
    let (header_name, _header_value) = value.split_once(':')?;
    let header_name = header_name.trim();
    if header_name.is_empty() || !is_secret_header_name(header_name) {
        return None;
    }
    Some((
        format!("{header_name}: {}", secret_placeholder(header_name)),
        header_name.to_string(),
    ))
}

/// Redact userinfo (`scheme://user:pass@host`) and known secret-bearing
/// query params from a URL string. Hand-rolled string scanning rather than
/// a `url`-crate parse (no such dependency in this workspace, matching
/// this module's existing style — see `sanitize_context_relative_path`) —
/// deliberately conservative: only touches the exact shapes below, leaving
/// anything it doesn't recognize unchanged rather than risking a malformed
/// rewrite of a URL this scan doesn't fully understand. Returns the
/// redacted URL plus the "landing" names for each thing redacted, so the
/// caller can generate a matching `requirements.json` entry per name
/// (reagent P2, PR #2333: redaction and the declared requirement must
/// never disagree, the same invariant already enforced for headers).
fn redact_url_credentials(url: &str) -> (String, Vec<String>) {
    let mut result = url.to_string();
    let mut redacted_names = Vec::new();

    if let Some(scheme_end) = result.find("://") {
        let after_scheme = scheme_end + 3;
        // The authority section (the only place userinfo can legally
        // appear) ends at the first '/', '?', or '#' -- the complete set
        // of authority-terminating delimiters per RFC 3986 §3.2. An `@`
        // at or after that boundary is inside the path/query/fragment,
        // not userinfo. Finding the boundary once up front (rather than
        // excluding one delimiter at a time as edge cases surface) is the
        // robust fix: this handles '/', '?', AND '#' — and any future
        // reader can see the fix's completeness at a glance instead of
        // wondering "what about the next delimiter" (reagent P0 + P2, PR
        // #2333: a bare '@' after either '?' or '#' was each independently
        // misread as userinfo, and the '?' case additionally blinded the
        // query-param redaction pass by consuming the delimiter itself).
        let authority_end = result[after_scheme..]
            .find(['/', '?', '#'])
            .map(|p| after_scheme + p)
            .unwrap_or(result.len());
        if let Some(at_offset) = result[after_scheme..authority_end].find('@') {
            let userinfo_end = after_scheme + at_offset;
            result.replace_range(after_scheme..userinfo_end, "${URL_CREDENTIALS}");
            redacted_names.push("url_credentials".to_string());
        }
    }

    if let Some(query_start) = result.find('?') {
        let (base, query_with_qmark) = result.split_at(query_start);
        let query = &query_with_qmark[1..];
        let mut changed = false;
        let new_pairs: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, _)) if is_secret_name(key) => {
                    changed = true;
                    redacted_names.push(key.to_string());
                    format!("{key}={}", secret_placeholder(key))
                }
                _ => pair.to_string(),
            })
            .collect();
        if changed {
            result = format!("{base}?{}", new_pairs.join("&"));
        }
    }

    (result, redacted_names)
}

/// Redact secret-shaped values out of an MCP server config entry before it
/// is written into a shareable bundle export. `env` and `headers` are the
/// two structured fields AgentMux's own MCP config editors accept literal
/// secret values into (e.g. `env.GITHUB_TOKEN`, `headers.Authorization` on
/// an HTTP/SSE-transport server) — every value under either key is
/// replaced with a `${VAR_NAME}`-style placeholder so the exported
/// `.server.json` never contains a real credential, matching ABF's
/// "declare, don't bundle secrets" principle that `accounts/requirements.json`
/// already follows (security finding, Codex P1 x2, PR #2325). `args` (CLI
/// flag/value pairs) and `url` (userinfo/query-param credentials) are
/// ALSO scanned for the same reason — a credential passed as
/// `--api-key <secret>` or `https://user:pass@host` is just as real a leak
/// as one in `env`/`headers` (Codex + reagent P1, PR #2333). Any other
/// field passes through unchanged.
///
/// Returns the redacted entry PLUS every "landing" name that was actually
/// redacted (env/header keys, arg flag names, url credential markers), so
/// the caller derives `requirements.json` directly from what was redacted
/// here instead of re-scanning separately — the two can never disagree by
/// construction (reagent P2, PR #2333).
fn redact_mcp_entry(entry: &Value) -> (Value, Vec<String>) {
    let mut redacted = entry.clone();
    let mut redacted_names: Vec<String> = Vec::new();
    if let Some(obj) = redacted.as_object_mut() {
        for field in ["env", "headers"] {
            if let Some(Value::Object(map)) = obj.get_mut(field) {
                for (key, value) in map.iter_mut() {
                    *value = json!(format!("${{{key}}}"));
                    redacted_names.push(key.clone());
                }
            }
        }
        if let Some(Value::Array(args)) = obj.get_mut("args") {
            let mut i = 0;
            while i < args.len() {
                let Some(s) = args[i].as_str().map(|s| s.to_string()) else {
                    i += 1;
                    continue;
                };
                if let Some((flag, value)) = s.split_once('=') {
                    // "--flag=value" form: redact just the value portion.
                    if is_secret_name(flag) {
                        args[i] = json!(format!("{flag}={}", secret_placeholder(flag)));
                        redacted_names.push(flag.trim_start_matches('-').to_string());
                    } else if is_header_flag(flag) {
                        if let Some((redacted_value, header_name)) = redact_header_value(value) {
                            args[i] = json!(format!("{flag}={redacted_value}"));
                            redacted_names.push(header_name);
                        }
                    }
                    i += 1;
                } else if is_secret_name(&s) && i + 1 < args.len() {
                    // "--flag value" form: redact the NEXT element, leave
                    // the flag itself (it's not a secret) untouched.
                    args[i + 1] = json!(secret_placeholder(&s));
                    redacted_names.push(s.trim_start_matches('-').to_string());
                    i += 2;
                } else if is_header_flag(&s) && i + 1 < args.len() {
                    // "--header value" form, where the secret is embedded
                    // in the value as "HeaderName: <secret>", not signaled
                    // by the flag name -- e.g.
                    // args: ["--header", "Authorization: Bearer <tok>"].
                    if let Some(value_str) = args[i + 1].as_str() {
                        if let Some((redacted_value, header_name)) =
                            redact_header_value(value_str)
                        {
                            args[i + 1] = json!(redacted_value);
                            redacted_names.push(header_name);
                        }
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
        if let Some(url_str) = obj.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()) {
            let (redacted_url, url_names) = redact_url_credentials(&url_str);
            if redacted_url != url_str {
                obj.insert("url".to_string(), json!(redacted_url));
                redacted_names.extend(url_names);
            }
        }
    }
    (redacted, redacted_names)
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
    let mut warnings = Vec::new();

    let mut manifest_instructions: Vec<String> = Vec::new();
    let mut manifest_skills: Vec<String> = Vec::new();
    let mut manifest_mcp: Vec<String> = Vec::new();

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
    // required (armory.json's own object key order is not meaningful), but
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

    // ------------------------------------------------------------------
    // mcp/<slug>.server.json + inferred accounts/requirements.json
    // ------------------------------------------------------------------
    let mcp_entries: Vec<Value> =
        parse_json_field_or_warn(&bundle.mcp_servers, "mcp_servers", &mut warnings);
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
        let (redacted_entry, redacted_names) = redact_mcp_entry(entry);
        let pretty = serde_json::to_string_pretty(&redacted_entry).unwrap_or_else(|_| "{}".to_string());
        files.push(BundleExportFile {
            path: path.clone(),
            content: pretty,
        });
        manifest_mcp.push(path);

        // Requirements are derived DIRECTLY from what redact_mcp_entry
        // actually redacted (env/header keys, arg flag names, url
        // credential markers) -- not re-scanned separately here -- so the
        // exported placeholder and the declared requirement can never
        // disagree by construction. This invariant was broken twice
        // already by re-scanning only a subset of fields (reagent P2, PR
        // #2325 for headers, PR #2333 for args/url); deriving from the
        // single redaction pass closes it for good.
        let mut seen_names: HashSet<&str> = HashSet::new();
        for name in &redacted_names {
            if !seen_names.insert(name.as_str()) {
                continue; // e.g. the same flag name appearing twice in args
            }
            requirements.push(json!({
                "id": format!("{slug}-{name}"),
                // ABF v0.2 (SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_
                // NATIVE_MEMORY_2026_08_10.md §2.1): renamed from "provider"
                // to disambiguate from the harness/model-vendor "provider"
                // concept components.instructions now uses (§2.2) — this one
                // means "which credential/account service", matching
                // db_accounts.provider, not a coding harness.
                "credentialProvider": slug,
                "kind": "api-key",
                // "Where it lands" per the requirements.json schema
                // (research report §5.3) -- an env var name, header name,
                // arg flag name, or url_credentials for a userinfo/query
                // redaction; the resolver substitutes into whichever the
                // exported .server.json's redacted `${NAME}` placeholder
                // sits under.
                "env": name,
                "optional": false,
            }));
        }
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
        // ABF v0.2 §2.4: bumped from v0.1 — components.instructions's shape
        // itself changed (flat array -> keyed object, §2.2). $schema is the
        // ABF FORMAT version, distinct from the "version" field below
        // (the bundle's own content version, which has nothing to do with
        // which ABF shape produced this file).
        "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json",
        "name": root_slug,
        // Bundles have no native version concept (no `version` column) --
        // ABF requires one, so this is an export-time default the user is
        // expected to bump before actually publishing the bundle anywhere.
        "version": "0.1.0",
        "description": bundle.description,
        // ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.3/§7.5 step
        // 6: harness + vendor, readonly-once-set on the source bundle (see
        // `check_provider_model_immutable` in `server/app_api/bundle.rs`) —
        // carried through export so a re-imported ABF is self-describing
        // about what it needs to run, not silently reset to unbound.
        // Omitted (not just empty-stringed) when the source bundle itself
        // has none set yet, so older/still-unbound bundles don't export a
        // misleadingly-present-but-empty field.
        "provider": if bundle.provider.is_empty() { Value::Null } else { json!(bundle.provider) },
        "model": if bundle.model.is_empty() { Value::Null } else { json!(bundle.model) },
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
        warnings,
    }
}

/// Parse a `db_bundles` JSON-array column (`context_files`/`mcp_servers`,
/// and — via the `bundle.export` RPC handler in `app_api/bundle.rs` —
/// `skills`), treating a blank/whitespace-only value as "genuinely no
/// data" (not an error) but pushing a warning to `warnings` for anything
/// non-blank that fails to parse, rather than silently discarding it via
/// `unwrap_or_default()` — an export that quietly loses data defeats its
/// own backup/portability purpose (reagent P1, PR #2333).
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
            instructions_by_provider: "{}".to_string(),
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
    fn requirement_entries_use_credential_provider_not_provider() {
        // ABF v0.2, §2.1: "provider" was ambiguous with the newer harness/
        // model-vendor sense components.instructions now uses. Exported
        // requirements must use the disambiguated key.
        let mcp_servers = r#"[{"name":"github","type":"stdio","command":"gh-mcp","env":{"GITHUB_TOKEN":""}}]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);

        let req_file = export
            .files
            .iter()
            .find(|f| f.path == "accounts/requirements.json")
            .unwrap();
        assert!(req_file.content.contains("\"credentialProvider\""));
        assert!(
            !req_file.content.contains("\"provider\""),
            "must not emit the old, ambiguous key alongside the new one: {}",
            req_file.content
        );
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

        // reagent P2, PR #2325: requirements.json must be inferred from BOTH
        // env and headers keys -- a header-only-authenticated server's
        // redacted `${Authorization}` placeholder must have a matching
        // requirement entry telling an importer a credential is needed there.
        let req_file = export
            .files
            .iter()
            .find(|f| f.path == "accounts/requirements.json")
            .unwrap();
        assert!(req_file.content.contains("GITHUB_TOKEN"));
        assert!(req_file.content.contains("Authorization"));
    }

    #[test]
    fn no_requirements_file_when_no_env_vars_present() {
        let mcp_servers = r#"[{"name":"local-tool","type":"stdio","command":"local-tool"}]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(!export.files.iter().any(|f| f.path == "accounts/requirements.json"));
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
    fn malformed_mcp_servers_json_warns_instead_of_silently_dropping_data() {
        let bundle = make_bundle("", "[]", "not an array at all", "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(
            export.warnings.iter().any(|w| w.contains("mcp_servers") && w.contains("malformed")),
            "expected a warning about malformed mcp_servers, got: {:?}",
            export.warnings
        );
    }

    #[test]
    fn blank_context_files_and_mcp_servers_produce_no_warning() {
        // A genuinely empty/unset field is not an error -- must not warn.
        let bundle = make_bundle("", "", "", "[]");
        let export = export_bundle(&bundle, &[]);
        assert!(export.warnings.is_empty(), "blank fields must not warn: {:?}", export.warnings);
    }

    #[test]
    fn parse_json_field_or_warn_direct_unit_test() {
        // reagent P1, PR #2333: `bundle.export`'s RPC handler
        // (app_api/bundle.rs) reuses this exact helper for `bundle.skills`,
        // which previously had the same unwrap_or_default() silent-loss bug
        // already fixed here for context_files/mcp_servers.
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
    fn infers_a_requirement_for_a_header_only_authenticated_server() {
        // reagent P2, PR #2325: a server authenticated ENTIRELY via headers
        // (no env at all) previously produced no requirements.json -- its
        // redacted `${Authorization}` placeholder in the exported
        // .server.json had nothing telling an importer a credential is
        // needed there.
        let mcp_servers = r#"[{
            "name": "notion",
            "type": "http",
            "url": "https://mcp.notion.com",
            "headers": {"Authorization": "Bearer realToken789"}
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);

        let req_file = export
            .files
            .iter()
            .find(|f| f.path == "accounts/requirements.json")
            .expect("a header-only-authenticated server must still infer a requirement");
        assert!(req_file.content.contains("Authorization"));
        assert!(
            !req_file.content.to_lowercase().contains("realtoken789"),
            "must never contain a real secret value"
        );
    }

    #[test]
    fn redacts_credentials_from_args_flag_equals_value_form() {
        // Codex + reagent P1, PR #2333: a real credential passed as
        // "--api-key=<secret>" in the runtime args array previously
        // exported verbatim.
        let mcp_servers = r#"[{
            "name": "linear",
            "type": "stdio",
            "command": "linear-mcp",
            "args": ["--api-key=lin_realSecretAbc123", "--verbose"]
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/linear.server.json").unwrap();
        assert!(!server_file.content.contains("lin_realSecretAbc123"));
        assert!(server_file.content.contains("${API_KEY}"));
        assert!(server_file.content.contains("--verbose"), "unrelated flags must survive untouched");

        // reagent P2, PR #2333: an args-derived redaction must infer a
        // matching requirement too, not just env/headers ones.
        let req_file = export.files.iter().find(|f| f.path == "accounts/requirements.json").unwrap();
        assert!(req_file.content.contains("api-key"));
    }

    #[test]
    fn redacts_credentials_from_args_flag_space_value_form() {
        let mcp_servers = r#"[{
            "name": "custom",
            "type": "stdio",
            "command": "custom-mcp",
            "args": ["--token", "realSecretXyz789", "--port", "8080"]
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/custom.server.json").unwrap();
        assert!(!server_file.content.contains("realSecretXyz789"));
        assert!(server_file.content.contains("${TOKEN}"));
        // "--port 8080" is not a secret flag -- must survive untouched.
        assert!(server_file.content.contains("8080"));

        let req_file = export.files.iter().find(|f| f.path == "accounts/requirements.json").unwrap();
        assert!(req_file.content.contains("token"));
    }

    #[test]
    fn unrelated_args_flags_are_never_redacted() {
        // "--keymap" contains neither "key" as a whole segment nor any
        // other curated secret-flag name -- must not false-positive.
        let mcp_servers = r#"[{
            "name": "editor",
            "type": "stdio",
            "command": "editor-mcp",
            "args": ["--keymap=vim", "--theme", "dark"]
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/editor.server.json").unwrap();
        assert!(server_file.content.contains("--keymap=vim"));
        assert!(server_file.content.contains("dark"));
    }

    #[test]
    fn redacts_userinfo_and_secret_query_params_from_url() {
        // Codex + reagent P1, PR #2333: a credential embedded in the `url`
        // field (userinfo or a secret-bearing query param) previously
        // exported verbatim.
        let mcp_servers = r#"[{
            "name": "remote",
            "type": "http",
            "url": "https://admin:realPass456@mcp.example.com/api?api_key=realKeyAbc&region=us"
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/remote.server.json").unwrap();
        assert!(!server_file.content.contains("realPass456"));
        assert!(!server_file.content.contains("realKeyAbc"));
        assert!(server_file.content.contains("${URL_CREDENTIALS}@mcp.example.com"));
        assert!(server_file.content.contains("api_key=${API_KEY}"));
        // Non-secret query params must survive untouched.
        assert!(server_file.content.contains("region=us"));

        // reagent P2, PR #2333: requirements.json must be derived from
        // the SAME redaction pass, not a separate env/headers-only scan.
        let req_file = export
            .files
            .iter()
            .find(|f| f.path == "accounts/requirements.json")
            .expect("url-embedded credentials must still infer requirements");
        assert!(req_file.content.contains("url_credentials"));
        assert!(req_file.content.contains("api_key"));
    }

    #[test]
    fn query_param_redaction_uses_the_same_allowlist_as_args() {
        // reagent P1, PR #2333: is_secret_query_param's list previously
        // omitted names already recognized for args (client_secret,
        // auth_token, bearer_token, secret_key) -- the two lists could
        // silently drift apart. Now backed by one shared SECRET_NAMES list.
        let mcp_servers = r#"[{
            "name": "oauth-server",
            "type": "http",
            "url": "https://api.example.com/mcp?client_secret=realClientSecret123&auth_token=realAuthToken456"
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/oauth-server.server.json").unwrap();
        assert!(!server_file.content.contains("realClientSecret123"));
        assert!(!server_file.content.contains("realAuthToken456"));
        assert!(server_file.content.contains("client_secret=${CLIENT_SECRET}"));
        assert!(server_file.content.contains("auth_token=${AUTH_TOKEN}"));
    }

    #[test]
    fn userinfo_detection_does_not_swallow_the_query_string_delimiter() {
        // reagent P0, PR #2333: a bare "@" appearing INSIDE the query
        // string (not real userinfo) was previously misdetected as
        // userinfo, and the replacement consumed the "?" along with it --
        // blinding the query-param redaction pass to a real secret sitting
        // right after it. No path, no real userinfo, just an "@" in a
        // param value.
        let mcp_servers = r#"[{
            "name": "svc",
            "type": "http",
            "url": "https://svc.example.com?a=b@c&api_key=realSecretShouldBeRedacted"
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/svc.server.json").unwrap();
        assert!(
            !server_file.content.contains("realSecretShouldBeRedacted"),
            "the real secret must still be redacted even with a bare '@' earlier in the query string: {}",
            server_file.content
        );
        assert!(server_file.content.contains("api_key=${API_KEY}"));
        // No genuine userinfo here -- must not fabricate a ${URL_CREDENTIALS}.
        assert!(!server_file.content.contains("URL_CREDENTIALS"));
    }

    #[test]
    fn userinfo_detection_excludes_the_fragment_delimiter() {
        // reagent P2, PR #2333: a bare "@" appearing after "#" (inside the
        // URL fragment, no path/query present) was previously misdetected
        // as userinfo -- the authority section ends at the FIRST of '/',
        // '?', OR '#', and the old check only excluded the first two.
        let mcp_servers = r#"[{
            "name": "svc",
            "type": "http",
            "url": "https://mcp.example.com#note@example.com"
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/svc.server.json").unwrap();
        assert!(
            server_file.content.contains("mcp.example.com#note@example.com"),
            "no genuine userinfo present (the '@' is inside the fragment) -- host must not be mangled: {}",
            server_file.content
        );
        assert!(!server_file.content.contains("URL_CREDENTIALS"));
    }

    #[test]
    fn redacts_secret_header_value_embedded_in_args_flag_space_value_form() {
        // Codex + reagent P0, PR #2333: a credential smuggled inside a
        // "--header"/"-H" flag's value as "HeaderName: <secret>" -- the
        // flag name itself is not secret-shaped, so is_secret_name never
        // catches it; the secret only becomes visible once the value is
        // itself parsed as a header.
        let mcp_servers = r#"[{
            "name": "remote",
            "type": "stdio",
            "command": "remote-mcp",
            "args": ["--header", "Authorization: Bearer realSecretToken789", "--verbose"]
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/remote.server.json").unwrap();
        assert!(!server_file.content.contains("realSecretToken789"));
        assert!(server_file.content.contains("Authorization: ${AUTHORIZATION}"));
        assert!(server_file.content.contains("--verbose"), "unrelated flags must survive untouched");

        let req_file = export.files.iter().find(|f| f.path == "accounts/requirements.json").unwrap();
        assert!(req_file.content.contains("Authorization"));
    }

    #[test]
    fn redacts_secret_header_value_embedded_in_args_flag_equals_value_form() {
        let mcp_servers = r#"[{
            "name": "remote2",
            "type": "stdio",
            "command": "remote-mcp",
            "args": ["--header=Authorization: Bearer realSecretToken456"]
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/remote2.server.json").unwrap();
        assert!(!server_file.content.contains("realSecretToken456"));
        assert!(server_file.content.contains("--header=Authorization: ${AUTHORIZATION}"));
    }

    #[test]
    fn non_secret_header_values_in_args_are_never_redacted() {
        let mcp_servers = r#"[{
            "name": "remote3",
            "type": "stdio",
            "command": "remote-mcp",
            "args": ["--header", "X-Request-Id: abc123", "-H", "Content-Type: application/json"]
        }]"#;
        let bundle = make_bundle("", "[]", mcp_servers, "[]");
        let export = export_bundle(&bundle, &[]);
        let server_file = export.files.iter().find(|f| f.path == "mcp/remote3.server.json").unwrap();
        assert!(server_file.content.contains("X-Request-Id: abc123"));
        assert!(server_file.content.contains("Content-Type: application/json"));
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
        // ABF v0.2 §2.2: components.instructions is the keyed-object shape
        // ("default" + provider variants), not a flat array.
        assert!(manifest["components"]["instructions"]["default"].as_array().unwrap().len() == 2);
        assert!(manifest["components"]["skills"].as_array().unwrap().len() == 1);
        assert!(manifest["components"]["mcpServers"].as_array().unwrap().len() == 1);
        assert_eq!(manifest["components"]["accounts"], "accounts/requirements.json");
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

        let manifest_file = export.files.iter().find(|f| f.path == "armory.json").unwrap();
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
        let manifest_file = export.files.iter().find(|f| f.path == "armory.json").unwrap();
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

        let manifest_file = export.files.iter().find(|f| f.path == "armory.json").unwrap();
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
