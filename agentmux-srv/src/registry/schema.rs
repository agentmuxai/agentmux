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
pub const MAX_SUPPORTED_SCHEMA: u32 = 1;

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
    /// FK to `db_identities.id`. None = unbound (= ambient creds).
    pub identity_id: Option<String>,
    /// FK to `db_memories.id`. None = unbound (= vanilla CLI).
    pub memory_id: Option<String>,
    /// Path **relative to `<shared_home>/agents/`** — never absolute.
    /// Keeps the record portable across machines where the home dir
    /// differs.
    pub working_dir: String,
    pub created_at_ms: i64,
    pub last_launched_at_ms: i64,
    pub created_by_version: String,
    pub last_launched_by_version: String,
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
                working_dir: "demo-0512a".to_string(),
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
        r.data.working_dir = "..\\..\\sneaky".to_string();
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
}
