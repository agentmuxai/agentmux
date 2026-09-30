// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Registry file format + per-row validation.
//!
//! Bumping `MAX_SUPPORTED_SCHEMA` is the additive-evolution path:
//! readers of the previous bound still skip-and-log new files; old
//! disk files keep validating because the v1 reader stays intact.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Lowest envelope schema this binary will load. Bumped only with a
/// deprecation cycle (see SPEC §6).
pub const MIN_SUPPORTED_SCHEMA: u32 = 1;
/// Highest envelope schema this binary will write or read. Bumped
/// per release that adds fields.
pub const MAX_SUPPORTED_SCHEMA: u32 = 3;

/// On-disk envelope. The `data` field's shape is gated by
/// `schema_version`; readers should match on the version before
/// projecting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedAgentRecord {
    pub schema_version: u32,
    pub data: NamedAgentRecordV1,
}

/// v1 payload. Add new optional fields here under `#[serde(default)]`
/// and bump `MAX_SUPPORTED_SCHEMA` — old readers will skip the new
/// version, new readers fill defaults for old files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedAgentRecordV1 {
    pub instance_id: String,
    pub instance_name: String,
    pub definition_id: String,
    /// Legacy Identity-bundle id — `db_identity_bundles` was dropped in
    /// Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md, so this
    /// field is now vestigial (opaque pass-through only; credential
    /// resolution is `db_agent_identity_links`-only). None = unbound
    /// (= ambient creds).
    pub identity_id: Option<String>,
    /// FK to `db_bundles.id`. None = unbound (= vanilla CLI).
    pub memory_id: Option<String>,
    /// Provider CLI session id for `--resume` (e.g. a Claude Code session
    /// uuid). `None` = no session wired yet (fresh stub / legacy record).
    /// Added in schema v2 so a global/migrated record can resume the
    /// agent's conversation across channels without a current-channel
    /// SQLite join. Old (v1) files deserialize this as `None`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Path **relative to [`source_agents_base`]** (or, for legacy records
    /// without one, the reader's current channel agents dir) — never
    /// absolute. Keeps the record portable across machines where the home
    /// dir differs.
    pub working_dir: String,
    /// Absolute path of the agents dir that [`working_dir`] is relative to —
    /// i.e. the channel/dev instance the agent actually lives in
    /// (`channels/<ch>/agents` or `<dev-instance>/agents`). Added in schema
    /// v3 (cross-channel persistence P0.4): the registry is global, so a row
    /// surfaced in a DIFFERENT channel must reconstruct its absolute
    /// `working_directory` against the SOURCE base, not the reader's current
    /// channel. `None` for legacy (v1/v2) records — the reader then falls
    /// back to its current channel agents dir, matching pre-P0.4 behavior.
    /// Old binaries deserialize this as `None`.
    #[serde(default)]
    pub source_agents_base: Option<String>,
    pub created_at_ms: i64,
    pub last_launched_at_ms: i64,
    pub created_by_version: String,
    pub last_launched_by_version: String,
}

impl NamedAgentRecordV1 {
    /// Lowest envelope schema that can faithfully represent this payload.
    /// Climbs only when a higher-version-only field is populated, so an
    /// older binary keeps reading records that don't use any newer feature —
    /// only records that actually need the newer schema are hidden from it.
    /// Writers should stamp the record with this rather than always using
    /// `MAX_SUPPORTED_SCHEMA`.
    ///
    /// - v3 when `source_agents_base` is set (cross-channel reconstruction)
    /// - v2 when `session_id` is set (cross-channel resume)
    /// - v1 otherwise
    pub fn min_schema_version(&self) -> u32 {
        if self.source_agents_base.is_some() {
            3
        } else if self.session_id.is_some() {
            2
        } else {
            1
        }
    }
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("schema_version {version} outside supported [{min}, {max}]")]
    UnsupportedSchema {
        version: u32,
        min: u32,
        max: u32,
    },
    #[error("filename UUID {filename:?} does not match data.instance_id {instance_id:?}")]
    IdMismatch {
        filename: String,
        instance_id: String,
    },
    #[error("working_dir {0:?} is not a safe relative subpath of agents/")]
    UnsafeWorkingDir(String),
    #[error("required field missing: {0}")]
    MissingField(&'static str),
}

/// Per-row validation. Fails fast on anything that would let a
/// malformed file be returned to the launch modal. Validation
/// failures are skipped (not auto-fixed), logged, and the file stays
/// on disk for ops triage.
pub fn validate(filename_stem: &str, rec: &NamedAgentRecord) -> Result<(), ValidationError> {
    if rec.schema_version < MIN_SUPPORTED_SCHEMA || rec.schema_version > MAX_SUPPORTED_SCHEMA {
        return Err(ValidationError::UnsupportedSchema {
            version: rec.schema_version,
            min: MIN_SUPPORTED_SCHEMA,
            max: MAX_SUPPORTED_SCHEMA,
        });
    }
    let d = &rec.data;
    if d.instance_id.is_empty() {
        return Err(ValidationError::MissingField("instance_id"));
    }
    if d.instance_id != filename_stem {
        return Err(ValidationError::IdMismatch {
            filename: filename_stem.to_string(),
            instance_id: d.instance_id.clone(),
        });
    }
    if d.instance_name.is_empty() {
        return Err(ValidationError::MissingField("instance_name"));
    }
    if d.definition_id.is_empty() {
        return Err(ValidationError::MissingField("definition_id"));
    }
    if d.working_dir.is_empty() {
        return Err(ValidationError::MissingField("working_dir"));
    }
    if !is_safe_relative_subpath(&d.working_dir) {
        return Err(ValidationError::UnsafeWorkingDir(d.working_dir.clone()));
    }
    Ok(())
}

fn is_safe_relative_subpath(s: &str) -> bool {
    let p = std::path::Path::new(s);
    if p.is_absolute() {
        return false;
    }
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return false,
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1_record(id: &str) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: 1,
            data: NamedAgentRecordV1 {
                instance_id: id.to_string(),
                instance_name: "demo".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "demo-0512a".to_string(),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "0.33.822".to_string(),
                last_launched_by_version: "0.33.822".to_string(),
            },
        }
    }

    #[test]
    fn happy_path() {
        let r = v1_record("abc");
        validate("abc", &r).unwrap();
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let mut r = v1_record("abc");
        r.schema_version = 999;
        let err = validate("abc", &r).unwrap_err();
        assert!(matches!(err, ValidationError::UnsupportedSchema { .. }));
    }

    #[test]
    fn filename_mismatch_is_rejected() {
        let r = v1_record("abc");
        assert!(matches!(
            validate("xyz", &r).unwrap_err(),
            ValidationError::IdMismatch { .. }
        ));
    }

    #[test]
    fn absolute_workdir_is_rejected() {
        let mut r = v1_record("abc");
        r.data.working_dir = if cfg!(windows) {
            "C:\\tmp\\evil".to_string()
        } else {
            "/tmp/evil".to_string()
        };
        assert!(matches!(
            validate("abc", &r).unwrap_err(),
            ValidationError::UnsafeWorkingDir(_)
        ));
    }

    #[test]
    fn dotdot_workdir_is_rejected() {
        let mut r = v1_record("abc");
        r.data.working_dir = "../../sneaky".to_string();
        assert!(matches!(
            validate("abc", &r).unwrap_err(),
            ValidationError::UnsafeWorkingDir(_)
        ));
    }

    #[test]
    fn missing_required_field_is_rejected() {
        let mut r = v1_record("abc");
        r.data.instance_name = String::new();
        assert!(matches!(
            validate("abc", &r).unwrap_err(),
            ValidationError::MissingField("instance_name")
        ));
    }

    #[test]
    fn session_id_drives_min_schema_version() {
        let mut r = v1_record("abc");
        assert_eq!(r.data.min_schema_version(), 1, "session-less record is v1");
        r.data.session_id = Some("sess-xyz".to_string());
        assert_eq!(r.data.min_schema_version(), 2, "session-wired record is v2");
        // A v2 record validates under the bumped MAX_SUPPORTED_SCHEMA.
        r.schema_version = 2;
        validate("abc", &r).unwrap();
    }

    #[test]
    fn source_agents_base_drives_min_schema_version_v3() {
        let mut r = v1_record("abc");
        assert_eq!(r.data.min_schema_version(), 1);
        r.data.source_agents_base = Some("/home/u/.agentmux/channels/stable/agents".to_string());
        assert_eq!(
            r.data.min_schema_version(),
            3,
            "a source-anchored record needs v3"
        );
        // v3 takes precedence even with session_id also set.
        r.data.session_id = Some("sess-1".to_string());
        assert_eq!(r.data.min_schema_version(), 3);
        // Validates under the bumped MAX_SUPPORTED_SCHEMA.
        r.schema_version = 3;
        validate("abc", &r).unwrap();
    }

    #[test]
    fn unknown_future_field_round_trips() {
        // A v3 record written by a newer binary with an unknown field must
        // still deserialize (serde ignores unknowns) so this reader can load
        // it — the additive-evolution contract.
        let raw = serde_json::json!({
            "schema_version": 3,
            "data": {
                "instance_id": "abc", "instance_name": "demo",
                "definition_id": "claude-code", "identity_id": null,
                "memory_id": null, "working_dir": "demo-0",
                "source_agents_base": "/h/.agentmux/channels/x/agents",
                "created_at_ms": 1, "last_launched_at_ms": 1,
                "created_by_version": "0.45.0", "last_launched_by_version": "0.45.0",
                "future_field": "ignored"
            }
        });
        let parsed: NamedAgentRecord = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.data.instance_id, "abc");
        assert_eq!(
            parsed.data.source_agents_base.as_deref(),
            Some("/h/.agentmux/channels/x/agents")
        );
    }
}
