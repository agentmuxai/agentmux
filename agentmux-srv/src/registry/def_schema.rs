// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Global **agent-definition** registry file format + per-row validation.
//!
//! Sibling of `schema.rs` (which describes the named *instance* record).
//! This record carries a full agent **definition** so the roster can be
//! reconstructed from any channel/version without joining the local
//! channel's SQLite — the foundation of cross-channel agent persistence
//! (`docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`, P0.2).
//!
//! Layering: the `registry` module must NOT depend on `backend::storage`
//! (which already depends on `registry` — that would cycle). So the fields
//! of `AgentDefinition` are mirrored here as a self-contained struct; the
//! `AgentDefinition <-> DefinitionRecordV1` conversion lives in
//! `backend::storage` (the dependent side). Bumping
//! `DEF_MAX_SUPPORTED_SCHEMA` is the additive-evolution path, exactly like
//! the instance record.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Lowest envelope schema this binary will load.
pub const DEF_MIN_SUPPORTED_SCHEMA: u32 = 1;
/// Highest envelope schema this binary will write or read. Bumped per
/// release that adds fields to the definition payload.
pub const DEF_MAX_SUPPORTED_SCHEMA: u32 = 1;

/// Serde default for `container_volumes` — preserves the db invariant that
/// an omitted value is the empty JSON array `"[]"`, not `""` (which would
/// surface via the read-path overlay). (reagent P2 on #1385.)
fn default_container_volumes() -> String {
    "[]".to_string()
}

/// On-disk envelope for a global agent-definition record. Stored at
/// `<shared>/agents/definitions/<id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DefinitionRecord {
    pub schema_version: u32,
    pub data: DefinitionRecordV1,
}

/// A content blob (system prompt / mcp / env / soul / startup) attached to
/// a definition — mirrors a `db_agent_content` row. Carried in the global
/// record so a cross-channel agent launches with its instructions even
/// though `db_agent_content` is per-version. (codex P1 on #1384.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DefContentBlob {
    pub content_type: String,
    pub content: String,
}

/// A skill attached to a definition — mirrors a `db_agent_skills` row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DefSkillBlob {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

/// v1 definition payload — a faithful mirror of `AgentDefinition`'s
/// columns, PLUS the per-definition content + skills blobs (which live in
/// separate per-version tables and must travel with the definition for a
/// cross-channel agent to launch with its instructions). New fields go
/// here under `#[serde(default)]` (so older files still deserialize)
/// paired with a `DEF_MAX_SUPPORTED_SCHEMA` bump.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DefinitionRecordV1 {
    pub id: String,
    #[serde(default)]
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    pub provider: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub provider_flags: String,
    #[serde(default)]
    pub auto_start: i64,
    #[serde(default)]
    pub restart_on_crash: i64,
    #[serde(default)]
    pub idle_timeout_minutes: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
    #[serde(default)]
    pub is_seeded: i64,
    #[serde(default)]
    pub accounts: String,
    #[serde(default)]
    pub parent_id: String,
    #[serde(default)]
    pub branch_label: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub user_hidden: i64,
    #[serde(default)]
    pub container_image: String,
    #[serde(default = "default_container_volumes")]
    pub container_volumes: String,
    #[serde(default)]
    pub container_name: String,
    /// Explicit per-agent opt-in to the CLI's global (ambient) login when
    /// no oauth-class account resolves at spawn (mirrors the SQLite v12
    /// column — SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3).
    /// Additive under `#[serde(default)]` (older files read as 0 =
    /// fail-by-default; older binaries ignore the extra field), same as
    /// the container_* fields — no envelope bump.
    #[serde(default)]
    pub use_ambient_login: i64,
    /// Per-agent opt-in letting a Warden Supervisor watcher agent
    /// auto-continue this agent's session on turn-end (mirrors the SQLite
    /// v17 column —
    /// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md).
    /// Additive under `#[serde(default)]`, same as `use_ambient_login` —
    /// no envelope bump.
    #[serde(default)]
    pub auto_continue_enabled: i64,
    /// System-prompt / mcp / env / soul / startup blobs (db_agent_content),
    /// embedded so a cross-channel agent launches with its instructions.
    #[serde(default)]
    pub content: Vec<DefContentBlob>,
    /// Skills (db_agent_skills) attached to this definition.
    #[serde(default)]
    pub skills: Vec<DefSkillBlob>,
}

#[derive(Debug, Error)]
pub enum DefValidationError {
    #[error("schema_version {version} outside supported [{min}, {max}]")]
    UnsupportedSchema { version: u32, min: u32, max: u32 },
    #[error("filename {filename:?} does not match data.id {id:?}")]
    IdMismatch { filename: String, id: String },
    #[error("required field missing: {0}")]
    MissingField(&'static str),
}

/// Per-row validation. Mirrors `schema::validate`: reject anything that
/// would surface a malformed definition to the roster. Failures are
/// skipped (not auto-fixed), logged, and the file stays on disk for ops
/// triage.
pub fn validate(filename_stem: &str, rec: &DefinitionRecord) -> Result<(), DefValidationError> {
    if rec.schema_version < DEF_MIN_SUPPORTED_SCHEMA || rec.schema_version > DEF_MAX_SUPPORTED_SCHEMA
    {
        return Err(DefValidationError::UnsupportedSchema {
            version: rec.schema_version,
            min: DEF_MIN_SUPPORTED_SCHEMA,
            max: DEF_MAX_SUPPORTED_SCHEMA,
        });
    }
    let d = &rec.data;
    if d.id.is_empty() {
        return Err(DefValidationError::MissingField("id"));
    }
    if d.id != filename_stem {
        return Err(DefValidationError::IdMismatch {
            filename: filename_stem.to_string(),
            id: d.id.clone(),
        });
    }
    if d.name.is_empty() {
        return Err(DefValidationError::MissingField("name"));
    }
    if d.provider.is_empty() {
        return Err(DefValidationError::MissingField("provider"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: 1,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                slug: id.to_string(),
                name: "Demo".to_string(),
                icon: "✦".to_string(),
                provider: "claude".to_string(),
                description: String::new(),
                working_directory: String::new(),
                shell: String::new(),
                provider_flags: String::new(),
                auto_start: 0,
                restart_on_crash: 0,
                idle_timeout_minutes: 0,
                created_at: 1,
                agent_type: "host".to_string(),
                environment: String::new(),
                agent_bus_id: String::new(),
                is_seeded: 0,
                accounts: String::new(),
                parent_id: String::new(),
                branch_label: String::new(),
                updated_at: 1,
                user_hidden: 0,
                container_image: String::new(),
                container_volumes: "[]".to_string(),
                container_name: String::new(),
                use_ambient_login: 0,
                auto_continue_enabled: 0,
                content: Vec::new(),
                skills: Vec::new(),
            },
        }
    }

    #[test]
    fn happy_path() {
        validate("abc", &rec("abc")).unwrap();
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let mut r = rec("abc");
        r.schema_version = 999;
        assert!(matches!(
            validate("abc", &r).unwrap_err(),
            DefValidationError::UnsupportedSchema { .. }
        ));
    }

    #[test]
    fn filename_mismatch_is_rejected() {
        assert!(matches!(
            validate("xyz", &rec("abc")).unwrap_err(),
            DefValidationError::IdMismatch { .. }
        ));
    }

    #[test]
    fn missing_name_is_rejected() {
        let mut r = rec("abc");
        r.data.name = String::new();
        assert!(matches!(
            validate("abc", &r).unwrap_err(),
            DefValidationError::MissingField("name")
        ));
    }

    #[test]
    fn missing_provider_is_rejected() {
        let mut r = rec("abc");
        r.data.provider = String::new();
        assert!(matches!(
            validate("abc", &r).unwrap_err(),
            DefValidationError::MissingField("provider")
        ));
    }

    #[test]
    fn unknown_fields_survive_round_trip() {
        // A future field this binary doesn't know must deserialize cleanly
        // (serde ignores unknown keys) so a newer writer's record still loads.
        let raw = serde_json::json!({
            "schema_version": 1,
            "data": {
                "id": "abc", "name": "Demo", "provider": "claude",
                "future_field": "ignored"
            }
        });
        let parsed: DefinitionRecord = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.data.id, "abc");
        assert_eq!(parsed.data.provider, "claude");
        // Defaulted field absent from JSON now uses the "[]" db invariant.
        assert_eq!(parsed.data.container_volumes, "[]");
    }
}
