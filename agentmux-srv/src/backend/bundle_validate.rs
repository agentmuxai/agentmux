// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Structural Armory Bundle Format (ABF) validator — Armory UI-alignment
//! pass (docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md).
//!
//! Pure functions only — no I/O, no Store access, same convention as
//! `bundle_export.rs` / `bundle_import.rs`. Checks a bundle's raw JSON
//! columns for the classes of bug `bundle_export.rs`'s own review cycle
//! found and fixed one at a time (unknown provider keys, unsafe/colliding
//! paths, malformed JSON) — surfaced proactively here, before export time,
//! rather than only as an export warning after the fact.
//!
//! This is advisory only: a bundle can be saved with validation errors
//! present. The `bundle.validate` RPC is a read-only, on-demand check the
//! Armory bundle editor's "Validate" button calls.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::bundle_export::{parse_json_field_or_warn, sanitize_context_relative_path, ContextFileEntry};
use super::providers;
use super::storage::store::Memory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    /// Which bundle field/component this issue is about — one of
    /// "instructions_by_provider", "context_files", "mcp_servers", "skills".
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationReport {
    /// True when `issues` contains no `Error`-severity entries. Warnings
    /// alone do not affect this — they're surfaced but non-blocking.
    pub is_valid: bool,
    pub issues: Vec<ValidationIssue>,
}

/// Run every structural check against a bundle's JSON-encoded columns.
pub fn validate_bundle(bundle: &Memory) -> ValidationReport {
    let mut issues = Vec::new();
    validate_instructions_by_provider(bundle, &mut issues);
    validate_context_files(bundle, &mut issues);
    validate_mcp_servers(bundle, &mut issues);
    validate_skills(bundle, &mut issues);

    let is_valid = !issues.iter().any(|i| i.severity == IssueSeverity::Error);
    ValidationReport { is_valid, issues }
}

fn push_error(issues: &mut Vec<ValidationIssue>, field: &str, message: String) {
    issues.push(ValidationIssue {
        severity: IssueSeverity::Error,
        field: field.to_string(),
        message,
    });
}

fn push_warning(issues: &mut Vec<ValidationIssue>, field: &str, message: String) {
    issues.push(ValidationIssue {
        severity: IssueSeverity::Warning,
        field: field.to_string(),
        message,
    });
}

/// Every key must resolve to a known harness provider (`providers::get_provider`
/// handles both canonical ids and aliases). Unlike `context_files`'
/// arbitrary user-supplied paths, provider keys are always exact matches
/// against `providers.rs`'s hardcoded registry/alias strings — none of
/// which contain `/` or `.` — so a path-normalization collision between two
/// *valid* keys structurally cannot happen here; no collision check needed.
fn validate_instructions_by_provider(bundle: &Memory, issues: &mut Vec<ValidationIssue>) {
    const FIELD: &str = "instructions_by_provider";
    if bundle.instructions_by_provider.trim().is_empty() {
        return;
    }
    let parsed: HashMap<String, String> = match serde_json::from_str(&bundle.instructions_by_provider) {
        Ok(v) => v,
        Err(e) => {
            push_error(issues, FIELD, format!("malformed JSON: {e}"));
            return;
        }
    };
    // Sorted for deterministic message ordering across runs.
    let mut sorted: Vec<&String> = parsed.keys().collect();
    sorted.sort();
    for key in sorted {
        if providers::get_provider(key).is_none() {
            push_error(issues, FIELD, format!("\"{key}\" is not a known harness provider"));
        }
    }
}

/// Mirrors `bundle_export.rs::export_bundle`'s own context-file handling:
/// each path must survive `sanitize_context_relative_path` (no absolute
/// paths, no `..` traversal, no drive letters), and no two entries may
/// normalize to the same output path case-insensitively (the most common
/// export/extract targets have case-insensitive filesystems).
fn validate_context_files(bundle: &Memory, issues: &mut Vec<ValidationIssue>) {
    const FIELD: &str = "context_files";
    let mut warnings = Vec::new();
    let entries: Vec<ContextFileEntry> =
        parse_json_field_or_warn(&bundle.context_files, FIELD, &mut warnings);
    for w in warnings {
        push_error(issues, FIELD, w);
    }

    let mut used_paths: HashSet<String> = HashSet::new();
    for entry in entries {
        match sanitize_context_relative_path(&entry.path) {
            None => push_error(
                issues,
                FIELD,
                format!(
                    "\"{}\" is not a safe relative path (absolute, contains \"..\", \
                     a drive letter, or empty)",
                    entry.path
                ),
            ),
            Some(safe) => {
                if !used_paths.insert(safe.to_lowercase()) {
                    push_error(
                        issues,
                        FIELD,
                        format!(
                            "\"{}\" collides with another context file after path \
                             normalization",
                            entry.path
                        ),
                    );
                }
            }
        }
    }
}

/// Each entry must be a JSON object (the shape `export_bundle` writes to
/// `mcp/<slug>.server.json`). Export auto-dedupes colliding display names
/// into distinct slugs (`unique_skill_slug`), so a duplicate `name` can't
/// actually collide on disk — flagged as a warning rather than an error
/// since it's very likely a copy-paste mistake, not a structural break.
fn validate_mcp_servers(bundle: &Memory, issues: &mut Vec<ValidationIssue>) {
    const FIELD: &str = "mcp_servers";
    let mut warnings = Vec::new();
    let entries: Vec<Value> = parse_json_field_or_warn(&bundle.mcp_servers, FIELD, &mut warnings);
    for w in warnings {
        push_error(issues, FIELD, w);
    }

    let mut seen_names: HashSet<String> = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if !entry.is_object() {
            push_error(issues, FIELD, format!("entry {} is not a JSON object", index + 1));
            continue;
        }
        if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
            let key = name.trim().to_lowercase();
            if !key.is_empty() && !seen_names.insert(key) {
                push_warning(
                    issues,
                    FIELD,
                    format!(
                        "\"{name}\" is used by more than one server entry — each gets a \
                         distinct slug on export, but this is likely a mistake"
                    ),
                );
            }
        }
    }
}

/// `skills` is just a JSON array of skill ids — the only structural checks
/// possible without a Store lookup (which this module deliberately never
/// does) are malformed JSON and an id listed more than once.
fn validate_skills(bundle: &Memory, issues: &mut Vec<ValidationIssue>) {
    const FIELD: &str = "skills";
    let mut warnings = Vec::new();
    let ids: Vec<String> = parse_json_field_or_warn(&bundle.skills, FIELD, &mut warnings);
    for w in warnings {
        push_error(issues, FIELD, w);
    }

    let mut seen: HashSet<&str> = HashSet::new();
    for id in &ids {
        if !seen.insert(id.as_str()) {
            push_warning(issues, FIELD, format!("skill id \"{id}\" is listed more than once"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bundle(
        instructions_by_provider: &str,
        context_files: &str,
        mcp_servers: &str,
        skills: &str,
    ) -> Memory {
        Memory {
            id: "bundle-1".to_string(),
            name: "Backend Dev Bundle".to_string(),
            description: "Backend dev conventions".to_string(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: "Be terse.".to_string(),
            instructions_by_provider: instructions_by_provider.to_string(),
            context_files: context_files.to_string(),
            mcp_servers: mcp_servers.to_string(),
            skills: skills.to_string(),
            sort_order: 0,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            is_system: false,
        }
    }

    #[test]
    fn empty_bundle_is_valid_with_no_issues() {
        let bundle = make_bundle("{}", "[]", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(report.is_valid);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn known_provider_keys_pass() {
        let bundle = make_bundle(r#"{"claude":"x","codex":"y"}"#, "[]", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(report.is_valid, "{:?}", report.issues);
    }

    #[test]
    fn unknown_provider_key_is_an_error() {
        let bundle = make_bundle(r#"{"chatgpt-desktop":"x"}"#, "[]", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.severity == IssueSeverity::Error
            && i.field == "instructions_by_provider"
            && i.message.contains("chatgpt-desktop")));
    }

    #[test]
    fn malformed_instructions_by_provider_json_is_an_error() {
        let bundle = make_bundle("not json", "[]", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "instructions_by_provider"
            && i.message.contains("malformed JSON")));
    }

    #[test]
    fn provider_alias_resolves_like_the_canonical_id() {
        // "claude-code" is a registered alias for "claude" — must pass just
        // like the canonical id does, since export/import both treat them
        // as the same provider (providers::get_provider handles aliases).
        let bundle = make_bundle(r#"{"claude-code":"x"}"#, "[]", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(report.is_valid, "{:?}", report.issues);
    }

    #[test]
    fn multiple_unknown_provider_keys_are_each_reported() {
        let bundle = make_bundle(r#"{"foo":"a","bar":"b"}"#, "[]", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        let provider_errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.field == "instructions_by_provider")
            .collect();
        assert_eq!(provider_errors.len(), 2, "{:?}", report.issues);
    }

    #[test]
    fn unsafe_context_file_path_is_an_error() {
        let bundle = make_bundle("{}", r#"[{"path":"../../etc/passwd","content":"x"}]"#, "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "context_files"
            && i.message.contains("not a safe relative path")));
    }

    #[test]
    fn colliding_context_file_paths_are_an_error() {
        let bundle = make_bundle(
            "{}",
            r#"[{"path":"docs/a.md","content":"one"},{"path":"Docs/A.md","content":"two"}]"#,
            "[]",
            "[]",
        );
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "context_files" && i.message.contains("collides")));
    }

    #[test]
    fn malformed_context_files_json_is_an_error() {
        let bundle = make_bundle("{}", "not json", "[]", "[]");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "context_files" && i.message.contains("malformed")));
    }

    #[test]
    fn duplicate_mcp_server_name_is_a_warning_not_an_error() {
        let bundle = make_bundle(
            "{}",
            "[]",
            r#"[{"name":"fs","command":"a"},{"name":"FS","command":"b"}]"#,
            "[]",
        );
        let report = validate_bundle(&bundle);
        assert!(report.is_valid, "warnings must not flip is_valid: {:?}", report.issues);
        assert!(report.issues.iter().any(|i| i.severity == IssueSeverity::Warning
            && i.field == "mcp_servers"));
    }

    #[test]
    fn non_object_mcp_entry_is_an_error() {
        let bundle = make_bundle("{}", "[]", r#"["not-an-object"]"#, "[]");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "mcp_servers" && i.message.contains("not a JSON object")));
    }

    #[test]
    fn duplicate_skill_id_is_a_warning_not_an_error() {
        let bundle = make_bundle("{}", "[]", "[]", r#"["skill-1","skill-1"]"#);
        let report = validate_bundle(&bundle);
        assert!(report.is_valid, "{:?}", report.issues);
        assert!(report.issues.iter().any(|i| i.severity == IssueSeverity::Warning && i.field == "skills"));
    }

    #[test]
    fn malformed_skills_json_is_an_error() {
        let bundle = make_bundle("{}", "[]", "[]", "not json");
        let report = validate_bundle(&bundle);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "skills" && i.message.contains("malformed")));
    }
}
