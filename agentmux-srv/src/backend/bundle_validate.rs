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
use super::storage::store::Bundle;

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

/// Run every structural check against a bundle.
///
/// `mcp_entries` are the bundle's MCP servers **resolved from
/// `db_bundle_mcp_ref`**, which is authoritative for what a bundle contains
/// (Phase 0b, `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`). They
/// are passed in rather than read off `bundle` for the same reason
/// `export_bundle` takes them: this module does no Store lookups, and reading
/// the inline `bundle.mcp_servers` column would validate data that nothing
/// else consumes any more.
///
/// Pass an empty slice for an unsaved draft — a bundle with no row yet has no
/// bindings to check.
pub fn validate_bundle(bundle: &Bundle, mcp_entries: &[Value]) -> ValidationReport {
    let mut issues = Vec::new();
    validate_instructions_by_provider(bundle, &mut issues);
    validate_context_files(bundle, &mut issues);
    validate_mcp_servers(mcp_entries, &mut issues);
    // `skills` had exactly two checks, malformed JSON and a duplicate id, and
    // the ref table makes both unrepresentable: `db_bundle_skills_ref` has a
    // PRIMARY KEY (bundle_id, skill_id), and every id it yields came from a
    // join against the catalog. There is nothing structural left to check.

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
fn validate_instructions_by_provider(bundle: &Bundle, issues: &mut Vec<ValidationIssue>) {
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
fn validate_context_files(bundle: &Bundle, issues: &mut Vec<ValidationIssue>) {
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
fn validate_mcp_servers(entries: &[Value], issues: &mut Vec<ValidationIssue>) {
    const FIELD: &str = "mcp_servers";
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fixture bundle plus the MCP entries to validate it against.
    ///
    /// The entries come back separately because `validate_bundle` takes them
    /// separately: they are resolved from `db_bundle_mcp_ref`, which is
    /// authoritative for what a bundle contains, and `Bundle` has no inline
    /// `mcp_servers` column any more (Phase 0b,
    /// `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`; columns
    /// retired in `m0031`). `mcp_servers` is still spelled as JSON here
    /// because these tests are about what the checks do with malformed and
    /// colliding entries, and JSON is the readable way to write those.
    fn make_bundle(
        instructions_by_provider: &str,
        context_files: &str,
        mcp_servers: &str,
    ) -> (Bundle, Vec<Value>) {
        let entries: Vec<Value> = serde_json::from_str(mcp_servers).unwrap_or_default();
        let bundle = Bundle {
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
            sort_order: 0,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            is_system: false,
        };
        (bundle, entries)
    }

    #[test]
    fn empty_bundle_is_valid_with_no_issues() {
        let (bundle, mcp) = make_bundle("{}", "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(report.is_valid);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn known_provider_keys_pass() {
        let (bundle, mcp) = make_bundle(r#"{"claude":"x","codex":"y"}"#, "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(report.is_valid, "{:?}", report.issues);
    }

    #[test]
    fn unknown_provider_key_is_an_error() {
        let (bundle, mcp) = make_bundle(r#"{"chatgpt-desktop":"x"}"#, "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.severity == IssueSeverity::Error
            && i.field == "instructions_by_provider"
            && i.message.contains("chatgpt-desktop")));
    }

    #[test]
    fn malformed_instructions_by_provider_json_is_an_error() {
        let (bundle, mcp) = make_bundle("not json", "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "instructions_by_provider"
            && i.message.contains("malformed JSON")));
    }

    #[test]
    fn provider_alias_resolves_like_the_canonical_id() {
        // "claude-code" is a registered alias for "claude" — must pass just
        // like the canonical id does, since export/import both treat them
        // as the same provider (providers::get_provider handles aliases).
        let (bundle, mcp) = make_bundle(r#"{"claude-code":"x"}"#, "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(report.is_valid, "{:?}", report.issues);
    }

    #[test]
    fn multiple_unknown_provider_keys_are_each_reported() {
        let (bundle, mcp) = make_bundle(r#"{"foo":"a","bar":"b"}"#, "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
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
        let (bundle, mcp) = make_bundle("{}", r#"[{"path":"../../etc/passwd","content":"x"}]"#, "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "context_files"
            && i.message.contains("not a safe relative path")));
    }

    #[test]
    fn colliding_context_file_paths_are_an_error() {
        let (bundle, mcp) = make_bundle(
            "{}",
            r#"[{"path":"docs/a.md","content":"one"},{"path":"Docs/A.md","content":"two"}]"#,
            "[]",
        );
        let report = validate_bundle(&bundle, &mcp);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "context_files" && i.message.contains("collides")));
    }

    #[test]
    fn malformed_context_files_json_is_an_error() {
        let (bundle, mcp) = make_bundle("{}", "not json", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "context_files" && i.message.contains("malformed")));
    }

    #[test]
    fn duplicate_mcp_server_name_is_a_warning_not_an_error() {
        let (bundle, mcp) = make_bundle(
            "{}",
            "[]",
            r#"[{"name":"fs","command":"a"},{"name":"FS","command":"b"}]"#,
        );
        let report = validate_bundle(&bundle, &mcp);
        assert!(report.is_valid, "warnings must not flip is_valid: {:?}", report.issues);
        assert!(report.issues.iter().any(|i| i.severity == IssueSeverity::Warning
            && i.field == "mcp_servers"));
    }

    #[test]
    fn non_object_mcp_entry_is_an_error() {
        let (bundle, mcp) = make_bundle("{}", "[]", r#"["not-an-object"]"#);
        let report = validate_bundle(&bundle, &mcp);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.field == "mcp_servers" && i.message.contains("not a JSON object")));
    }

    #[test]
    fn skills_are_no_longer_validated_because_both_checks_became_impossible() {
        // This replaces `duplicate_skill_id_is_a_warning_not_an_error` and
        // `malformed_skills_json_is_an_error`. Those checked the inline
        // `skills` column, which nothing reads now that `db_bundle_skills_ref`
        // is authoritative (Phase 0b). Neither failure it guarded can be
        // represented any more: the ref table has a PRIMARY KEY
        // (bundle_id, skill_id) so an id cannot appear twice, and ids come out
        // of a join against the catalog rather than out of parsed JSON, so
        // there is nothing to be malformed.
        //
        // Kept as a record of where those guarantees went, rather than a
        // silent deletion of two tests. As of `m0031` the column is gone from
        // `Bundle` entirely, so the fixture cannot even express the input the
        // deleted tests used — what remains to assert is that no code path
        // still emits an issue under this field name.
        let (bundle, mcp) = make_bundle("{}", "[]", "[]");
        let report = validate_bundle(&bundle, &mcp);
        assert!(report.is_valid, "{:?}", report.issues);
        assert!(
            !report.issues.iter().any(|i| i.field == "skills"),
            "nothing may report issues against the retired skills column: {:?}",
            report.issues
        );
    }
}
