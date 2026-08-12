// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `DefinitionStore` — file-per-definition CRUD on `<root>/<id>.json`
//! with a sibling `retired/<id>.json` tombstone tree.
//!
//! Sibling of `store.rs` (the named-*instance* `Registry`). Same on-disk
//! discipline: atomic rename for cross-process safety, a forward-compat
//! merge that preserves unknown fields, refuses schema downgrades, and
//! never clobbers an unparseable file. Holds the GLOBAL agent-definition
//! roster so any channel/version sees the same agents (P0.2 of
//! `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`).
//!
//! The forward-compat merge helpers below intentionally parallel the
//! private ones in `store.rs`. They are kept separate (not shared) so
//! this addition is fully isolated from the in-production instance
//! registry; a later cleanup can extract a generic file-store both use.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;
use thiserror::Error;

use super::atomic::{rename_atomic, write_atomic};
use super::def_schema::{validate, DefValidationError, DefinitionRecord, DEF_MAX_SUPPORTED_SCHEMA};

#[derive(Debug, Error)]
pub enum DefStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation: {0}")]
    Validation(#[from] DefValidationError),
}

pub struct DefinitionStore {
    root: PathBuf,
    write_lock: Mutex<()>,
}

impl DefinitionStore {
    /// Open or create the store rooted at `root`. Ensures the active dir
    /// and `retired/` subdir both exist.
    pub fn open(root: PathBuf) -> Result<Self, DefStoreError> {
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(root.join("retired"))?;
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn active_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn retired_path(&self, id: &str) -> PathBuf {
        self.root.join("retired").join(format!("{id}.json"))
    }

    /// Insert or update a definition. Preserves unknown top-level + `data`
    /// fields on an existing file (forward-compat); refuses to overwrite a
    /// corrupt or higher-schema file.
    pub fn upsert(&self, rec: &DefinitionRecord) -> Result<(), DefStoreError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.active_path(&rec.data.id);
        let bytes = match std::fs::read(&path) {
            Ok(existing) => match merge_for_write(&existing, rec)? {
                Some(b) => b,
                None => return Ok(()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Don't resurrect a tombstoned definition: a missing active
                // file plus an existing `retired/<id>.json` means the agent
                // was deliberately deleted (possibly in another channel). A
                // stale cross-channel mirror calling `upsert` must NOT recreate
                // it; the caller must `unretire` first to bring it back.
                // (codex P1 on #1384.)
                if self.retired_path(&rec.data.id).exists() {
                    return Ok(());
                }
                to_pretty(rec)?
            }
            Err(e) => return Err(e.into()),
        };
        write_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Soft-delete → move into `retired/` (tombstone). Idempotent. The
    /// tombstone prevents another channel's migration from resurrecting an
    /// agent the user deliberately deleted.
    pub fn retire(&self, id: &str) -> Result<(), DefStoreError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let from = self.active_path(id);
        if !from.exists() {
            return Ok(());
        }
        rename_atomic(&from, &self.retired_path(id))?;
        Ok(())
    }

    /// Move a record back from `retired/` to active. Idempotent.
    pub fn unretire(&self, id: &str) -> Result<(), DefStoreError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let from = self.retired_path(id);
        if !from.exists() {
            return Ok(());
        }
        rename_atomic(&from, &self.active_path(id))?;
        Ok(())
    }

    /// Hard-delete (drops both active and retired files). Mirrors SQLite
    /// `agent_def_delete`. Use [`Self::retire`] when a resurrection-proof
    /// tombstone is wanted instead.
    pub fn hard_delete(&self, id: &str) -> Result<(), DefStoreError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        for p in [self.active_path(id), self.retired_path(id)] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn exists(&self, id: &str) -> bool {
        self.active_path(id).exists()
    }

    pub fn exists_anywhere(&self, id: &str) -> bool {
        self.active_path(id).exists() || self.retired_path(id).exists()
    }

    /// Read a single active definition record by id. `None` if absent;
    /// invalid/corrupt files surface as an error (caller logs + falls back).
    pub fn get(&self, id: &str) -> Result<Option<DefinitionRecord>, DefStoreError> {
        match std::fs::read(self.active_path(id)) {
            Ok(bytes) => {
                let rec: DefinitionRecord = serde_json::from_slice(&bytes)?;
                validate(id, &rec)?;
                Ok(Some(rec))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Read every valid active definition. Invalid files are skipped +
    /// logged (left on disk for ops triage).
    pub fn list_active(&self) -> Result<Vec<DefinitionRecord>, DefStoreError> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match read_and_validate(&path, stem) {
                Ok(rec) => out.push(rec),
                Err(e) => {
                    tracing::warn!(
                        file = %path.display(),
                        error = %e,
                        "def registry: skipping invalid record"
                    );
                }
            }
        }
        Ok(out)
    }
}

fn read_and_validate(path: &Path, stem: &str) -> Result<DefinitionRecord, DefStoreError> {
    let bytes = std::fs::read(path)?;
    let rec: DefinitionRecord = serde_json::from_slice(&bytes)?;
    validate(stem, &rec)?;
    Ok(rec)
}

fn to_pretty(rec: &DefinitionRecord) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(rec)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Merge an in-memory record into an on-disk file's raw JSON, preserving
/// fields beyond this binary's struct shape. Returns `Ok(None)` (skip,
/// leave file intact) on corrupt JSON, missing `schema_version`, or
/// `schema_version` above `DEF_MAX_SUPPORTED_SCHEMA`. Parallels
/// `store.rs::merge_for_write` (see module docs).
fn merge_for_write(
    existing: &[u8],
    rec: &DefinitionRecord,
) -> Result<Option<Vec<u8>>, DefStoreError> {
    let on_disk: Value = match serde_json::from_slice(existing) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "def registry: existing file is unparseable JSON — refusing to overwrite"
            );
            return Ok(None);
        }
    };
    // `try_from` so a number above u32::MAX is treated as unparseable
    // rather than wrapping past the forward-compat guard.
    let on_disk_version = on_disk
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok());
    match on_disk_version {
        Some(v) if v > DEF_MAX_SUPPORTED_SCHEMA => {
            tracing::warn!(
                on_disk = v,
                writer_max = DEF_MAX_SUPPORTED_SCHEMA,
                "def registry: on-disk schema_version > writer max — refusing downgrade"
            );
            return Ok(None);
        }
        Some(_) => {}
        None => {
            tracing::warn!("def registry: existing file lacks schema_version — refusing to overwrite");
            return Ok(None);
        }
    }
    let mut merged = on_disk;
    let updates = serde_json::to_value(rec)?;
    merge_known(&mut merged, &updates);
    let mut bytes = serde_json::to_vec_pretty(&merged)?;
    bytes.push(b'\n');
    Ok(Some(bytes))
}

/// Overwrite `target`'s top-level keys with `updates`' keys, preserving
/// keys absent from `updates`; recurses into `data`.
fn merge_known(target: &mut Value, updates: &Value) {
    let (Some(t), Some(u)) = (target.as_object_mut(), updates.as_object()) else {
        *target = updates.clone();
        return;
    };
    for (k, v) in u {
        if k == "data" {
            if let Some(t_data) = t.get_mut("data") {
                merge_known(t_data, v);
                continue;
            }
        }
        t.insert(k.clone(), v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::DefinitionRecordV1;

    fn store() -> (tempfile::TempDir, DefinitionStore) {
        let tmp = tempfile::tempdir().unwrap();
        let s = DefinitionStore::open(tmp.path().join("definitions")).unwrap();
        (tmp, s)
    }

    fn rec(id: &str, name: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: 1,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                slug: id.to_string(),
                name: name.to_string(),
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
    fn upsert_then_list() {
        let (_t, s) = store();
        s.upsert(&rec("aaa", "Demo")).unwrap();
        let listed = s.list_active().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].data.name, "Demo");
    }

    #[test]
    fn upsert_update_replaces_known_fields() {
        let (_t, s) = store();
        s.upsert(&rec("aaa", "Demo")).unwrap();
        s.upsert(&rec("aaa", "Renamed")).unwrap();
        let listed = s.list_active().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].data.name, "Renamed");
    }

    #[test]
    fn retire_then_unretire_round_trips() {
        let (_t, s) = store();
        s.upsert(&rec("aaa", "Demo")).unwrap();
        s.retire("aaa").unwrap();
        assert!(s.list_active().unwrap().is_empty());
        assert!(s.exists_anywhere("aaa"));
        s.unretire("aaa").unwrap();
        assert_eq!(s.list_active().unwrap().len(), 1);
    }

    #[test]
    fn hard_delete_removes_both_paths() {
        let (_t, s) = store();
        s.upsert(&rec("aaa", "Demo")).unwrap();
        s.retire("aaa").unwrap();
        s.upsert(&rec("bbb", "Demo2")).unwrap();
        s.hard_delete("aaa").unwrap();
        s.hard_delete("bbb").unwrap();
        assert!(!s.exists_anywhere("aaa"));
        assert!(!s.exists_anywhere("bbb"));
    }

    #[test]
    fn unknown_field_survives_older_writer_update() {
        let (_t, s) = store();
        // Write a record carrying a future field directly to disk.
        let path = s.root().join("aaa.json");
        let raw = serde_json::json!({
            "schema_version": 1,
            "data": {
                "id": "aaa", "name": "Demo", "provider": "claude",
                "tags": ["keep-me"]
            }
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
        // An older binary updates a known field.
        s.upsert(&rec("aaa", "Renamed")).unwrap();
        let on_disk: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk.pointer("/data/tags"), Some(&serde_json::json!(["keep-me"])));
        assert_eq!(on_disk.pointer("/data/name"), Some(&serde_json::json!("Renamed")));
    }

    #[test]
    fn refuses_to_downgrade_higher_schema_on_disk() {
        let (_t, s) = store();
        let path = s.root().join("aaa.json");
        let v999 = serde_json::json!({
            "schema_version": 999,
            "data": { "id": "aaa", "future_only": "field" }
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&v999).unwrap()).unwrap();
        s.upsert(&rec("aaa", "Renamed")).unwrap();
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after.pointer("/schema_version"), Some(&serde_json::json!(999)));
        assert_eq!(after.pointer("/data/future_only"), Some(&serde_json::json!("field")));
    }

    #[test]
    fn upsert_does_not_resurrect_a_tombstoned_definition() {
        let (_t, s) = store();
        s.upsert(&rec("aaa", "Demo")).unwrap();
        s.retire("aaa").unwrap();
        // A stale cross-channel mirror upserts the same id.
        s.upsert(&rec("aaa", "Resurrected")).unwrap();
        // Tombstone respected: still not active, not resurrected.
        assert!(
            s.list_active().unwrap().is_empty(),
            "tombstoned definition must not be resurrected by upsert"
        );
        assert!(s.exists_anywhere("aaa"), "tombstone file still present");
        // An explicit unretire is required to bring it back.
        s.unretire("aaa").unwrap();
        assert_eq!(s.list_active().unwrap().len(), 1);
    }
}
