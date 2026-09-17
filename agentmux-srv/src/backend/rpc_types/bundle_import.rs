// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `bundle.import.preview` and `bundle.import.commit`.
//!
//! These did not exist before: both handlers built their responses as ad-hoc
//! `json!` literals, so there was nothing for the bindings generator to read
//! and the frontend's matching declarations were hand-maintained in
//! `srv-types.d.ts` with no mechanical link to the server. Authoring them here
//! is what lets `scripts/check-rpc-bindings.sh` see this surface at all.
//!
//! Two shapes below were NOT in the hand-written declarations and are now
//! visible to the frontend for the first time — see
//! `BundleImportPreviewResponse::project_instructions`.

use serde::{Deserialize, Serialize};

/// One `*.md` context file offered for selection in the preview.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportContextFilePreview {
    /// 0-based index within this parse — the stable selection key
    /// `include_context_files` uses at commit. Never the display path, which
    /// may be truncated.
    #[ts(type = "number")]
    pub id: usize,
    pub display_path: String,
    #[ts(type = "number")]
    pub size_bytes: usize,
}

/// One skill directory offered for selection.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportSkillPreview {
    /// Stable selection key — never the (possibly truncated) slug.
    pub source_dir: String,
    pub slug: String,
    pub description: String,
    /// Produced by `classify_skill_collision`, which returns a
    /// `&'static str` from a closed set. ts-rs emits a bare `string` for a
    /// Rust `String`, which would silently drop the union the hand-written
    /// declaration carried, so the override keeps it.
    #[ts(type = "\"none\" | \"name_conflict\" | \"duplicate_in_bundle\"")]
    pub collision: String,
}

/// The deliberately-narrow view of an MCP server config shown in the preview.
/// Both fields are genuinely nullable rather than optional: `mcp_server_display`
/// writes an explicit JSON `null` when the key is absent from the config, so
/// the key is always present on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportMcpServerDisplay {
    pub name: Option<String>,
    pub command: Option<String>,
}

/// One MCP server offered for selection. Carries only `display`, never the
/// full config — a bundle's server config can hold secrets.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportMcpServerPreview {
    /// Stable selection key.
    pub source_path: String,
    pub display: BundleImportMcpServerDisplay,
}

/// One account requirement the bundle declares, with how many local accounts
/// match it. `resolved` is exactly `match_count == 1` — zero means nothing to
/// bind, more than one means the choice is ambiguous.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportRequirementPreview {
    pub id: String,
    pub provider: String,
    pub env: String,
    pub resolved: bool,
    #[ts(type = "number")]
    pub match_count: usize,
}

/// One project-instruction snapshot (`CLAUDE.md` and friends) carried by the
/// bundle.
///
/// NOTE: the preview response has always included this list, but the
/// hand-written `BundleImportPreviewResponse` in `srv-types.d.ts` did not
/// declare it and no frontend code reads it — so it has been shipping on the
/// wire, invisible to the client. Generating the response type surfaces it
/// rather than silently dropping it; whether the UI should use it is a
/// product question, not a refactor one.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportProjectInstructionPreview {
    /// Working-directory-relative path on the SOURCE machine.
    pub path: String,
    /// Where the snapshot lives inside the bundle.
    pub file: String,
    pub content_hash: String,
    /// `"agentmux"` or `"foreign"`, verbatim from the manifest — deliberately
    /// not narrowed to a union, because an unrecognized value from a
    /// future/hand-written bundle must survive to the UI as-is.
    pub owner: String,
    pub content_preview: String,
    pub content_truncated: bool,
    #[ts(type = "number")]
    pub content_total_chars: usize,
}

/// Response for `bundle.import.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportPreviewResponse {
    pub project_instructions: Vec<BundleImportProjectInstructionPreview>,
    pub name: String,
    pub description: String,
    pub instructions_preview: String,
    pub instructions_truncated: bool,
    #[ts(type = "number")]
    pub instructions_total_chars: usize,
    pub context_files: Vec<BundleImportContextFilePreview>,
    pub skills: Vec<BundleImportSkillPreview>,
    pub mcp_servers: Vec<BundleImportMcpServerPreview>,
    pub requirements: Vec<BundleImportRequirementPreview>,
    pub warnings: Vec<String>,
    pub warnings_truncated: bool,
    /// Soft and informational: `bundle_upsert` has no name uniqueness
    /// constraint, so this never blocks an import.
    pub name_collision: bool,
    /// Required back at commit as `expected_content_digest` — proves the file
    /// has not changed since preview.
    pub content_digest: String,
}

/// A requirement the commit could not bind, with the ambiguity count that
/// explains why (0 = nothing matched, >1 = ambiguous).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportUnresolvedRequirement {
    pub id: String,
    pub provider: String,
    pub env: String,
    #[ts(type = "number")]
    pub match_count: usize,
}

/// Response for `bundle.import.commit`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportCommitResponse {
    pub bundle_id: String,
    pub imported_skill_ids: Vec<String>,
    pub skipped_skills: Vec<String>,
    pub resolved_requirement_ids: Vec<String>,
    pub unresolved_requirements: Vec<BundleImportUnresolvedRequirement>,
    pub warnings: Vec<String>,
    pub warnings_truncated: bool,
}

/// One in-memory file of an unpacked bundle, for the `files` input mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportFileEntry {
    pub path: String,
    pub content: String,
}

/// One skill selected for import, optionally renamed to dodge a collision.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BundleImportSkillSelection {
    /// Matches `BundleImportSkillPreview::source_dir` — the stable key, not
    /// the slug.
    pub source_dir: String,
    #[serde(default)]
    #[ts(optional)]
    pub import_as: Option<String>,
}

/// Request for `bundle.import.preview`.
///
/// All three inputs are optional ALTERNATIVES, not a required path plus
/// extras: `resolve_import_input` accepts a `file_path` on disk, a
/// base64 zip, or an already-unpacked `files` list. The hand-written stub
/// declared this as `{ file_path: string }`, so two of the three input modes
/// were unreachable through the typed client even though the server has always
/// supported them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandBundleImportPreviewData {
    #[serde(default)]
    #[ts(optional)]
    pub file_path: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub zip_base64: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub files: Option<Vec<BundleImportFileEntry>>,
}

/// Request for `bundle.import.commit`. Same three-way input as preview, plus
/// the selections and the digest that proves the source has not changed since.
///
/// Every field is `#[serde(default)]`, so all of them are omittable on the
/// wire. The non-`Option` ones (`expected_content_digest`,
/// `include_instructions`, the three `include_*` lists) are generated as
/// REQUIRED because ts-rs cannot express an optional property for a
/// non-`Option` field — which is fine and arguably better here: silently
/// defaulting `expected_content_digest` to `""` on a commit would skip the
/// staleness check the digest exists to enforce, so callers should be made to
/// pass it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandBundleImportCommitData {
    #[serde(default)]
    #[ts(optional)]
    pub file_path: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub zip_base64: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub files: Option<Vec<BundleImportFileEntry>>,
    #[serde(default)]
    pub expected_content_digest: String,
    #[serde(default)]
    #[ts(optional)]
    pub bundle_name: Option<String>,
    #[serde(default)]
    pub include_instructions: bool,
    /// Indices into the preview's `context_files`, by `id`.
    #[serde(default)]
    #[ts(type = "number[]")]
    pub include_context_files: Vec<usize>,
    #[serde(default)]
    pub include_skills: Vec<BundleImportSkillSelection>,
    /// `source_path` values from the preview's `mcp_servers`.
    #[serde(default)]
    pub include_mcp_servers: Vec<String>,
}

// Request-shape tests for the two `bundle.import.*` commands.
//
// These matter more than usual here, because both request types were PRIVATE
// to the handler file before this change and the frontend's copies were
// hand-written against nothing. The preview one was wrong: it declared only
// `file_path`, so two of the three input modes the server accepts were
// unreachable through the typed client.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    // All three inputs are ALTERNATIVES. `resolve_import_input` picks whichever
    // is present, so each must parse on its own.
    #[test]
    fn preview_accepts_each_of_the_three_input_modes() {
        let by_path: CommandBundleImportPreviewData =
            serde_json::from_value(json!({"file_path": "/tmp/b.zip"})).expect("file_path mode");
        assert_eq!(by_path.file_path.as_deref(), Some("/tmp/b.zip"));

        let by_zip: CommandBundleImportPreviewData =
            serde_json::from_value(json!({"zip_base64": "UEsDBA=="})).expect("zip_base64 mode");
        assert!(by_zip.file_path.is_none() && by_zip.zip_base64.is_some());

        let by_files: CommandBundleImportPreviewData = serde_json::from_value(json!({
            "files": [{"path": "armory.json", "content": "{}"}]
        }))
        .expect("files mode");
        assert_eq!(by_files.files.as_ref().map(Vec::len), Some(1));
    }

    // Every field is `#[serde(default)]`, so an empty object parses. That is
    // the server's existing behaviour and worth pinning: it means a malformed
    // call fails later, in `resolve_import_input`, with a domain error rather
    // than a deserialization error.
    #[test]
    fn preview_accepts_an_empty_object() {
        let empty: CommandBundleImportPreviewData =
            serde_json::from_value(json!({})).expect("all three inputs are optional");
        assert!(empty.file_path.is_none() && empty.zip_base64.is_none() && empty.files.is_none());
    }

    #[test]
    fn commit_accepts_the_full_selection_payload() {
        let req: CommandBundleImportCommitData = serde_json::from_value(json!({
            "file_path": "/tmp/b.zip",
            "expected_content_digest": "sha256:abc",
            "bundle_name": "renamed",
            "include_instructions": true,
            "include_context_files": [0, 2],
            "include_skills": [{"source_dir": "skills/deploy", "import_as": "deploy2"}],
            "include_mcp_servers": ["mcp/github.json"],
        }))
        .expect("commit must accept the full selection payload");
        assert_eq!(req.include_context_files, vec![0, 2]);
        assert_eq!(req.include_skills[0].import_as.as_deref(), Some("deploy2"));
    }

    // `import_as` is the optional half of a skill selection — omitting it means
    // "import under the original slug", which is the common case.
    #[test]
    fn commit_accepts_a_skill_selection_without_a_rename() {
        let req: CommandBundleImportCommitData = serde_json::from_value(json!({
            "expected_content_digest": "sha256:abc",
            "include_skills": [{"source_dir": "skills/deploy"}],
        }))
        .expect("import_as must be optional");
        assert!(req.include_skills[0].import_as.is_none());
    }

    // The generated binding marks `expected_content_digest` REQUIRED even
    // though serde defaults it, because ts-rs cannot express an optional
    // property for a non-`Option` field. That asymmetry is deliberate here:
    // defaulting it to "" server-side would skip the staleness check, so
    // callers should be made to pass it. This pins the wire half.
    #[test]
    fn commit_still_parses_without_a_digest_even_though_callers_must_send_one() {
        let req: CommandBundleImportCommitData =
            serde_json::from_value(json!({})).expect("every commit field has a serde default");
        assert_eq!(req.expected_content_digest, "");
        assert!(!req.include_instructions);
    }
}
