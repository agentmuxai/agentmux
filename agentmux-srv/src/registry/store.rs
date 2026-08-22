// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Registry` — file-per-agent CRUD on `<root>/<uuid>.json` with
//! a sibling `retired/<uuid>.json` tombstone tree.
//!
//! Concurrency: cross-process safety comes from filesystem atomic
//! rename. The internal `Mutex` only serializes partial-merge writes
//! from threads inside the same `srv` so a v1 binary touching
//! `last_launched_at_ms` doesn't race itself.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;
use thiserror::Error;

use super::atomic::{rename_atomic, write_atomic};
use super::schema::{validate, NamedAgentRecord, ValidationError, MAX_SUPPORTED_SCHEMA};

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),
}

pub struct Registry {
    root: PathBuf,
    write_lock: Mutex<()>,
}

impl Registry {
    /// Open or create the registry rooted at `root`. Ensures the
    /// active dir and `retired/` subdir both exist.
    pub fn open(root: PathBuf) -> Result<Self, RegistryError> {
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

    /// Resolved `<shared_home>/agents/` — one level above `root`.
    /// Used by callers that need to express working-directory paths
    /// as relative subpaths under the shared agents tree. Returns
    /// `None` if the registry root has no parent (only happens in
    /// pathological filesystem-root setups; production always nests
    /// under `~/.agentmux/agents/registry`).
    pub fn agents_root(&self) -> Option<&Path> {
        self.root.parent()
    }

    fn active_path(&self, instance_id: &str) -> PathBuf {
        self.root.join(format!("{instance_id}.json"))
    }

    fn retired_path(&self, instance_id: &str) -> PathBuf {
        self.root.join("retired").join(format!("{instance_id}.json"))
    }

    /// Insert or update a record. If the file already exists, unknown
    /// top-level + `data` fields are preserved (forward-compat with
    /// future schemas that add columns this binary doesn't know).
    ///
    /// Forward-compat invariant (spec §6): never write a higher-schema
    /// row into a lower schema, never overwrite a corrupt/unparseable
    /// file. Both cases skip the mirror with a warning — the on-disk
    /// file stays intact for the binary that authored it (or for ops
    /// triage). Skipping is `Ok(())`: SQLite remains authoritative.
    pub fn upsert(&self, rec: &NamedAgentRecord) -> Result<(), RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.active_path(&rec.data.instance_id);
        let bytes = match std::fs::read(&path) {
            Ok(existing) => match merge_for_write(&existing, rec)? {
                Some(b) => b,
                None => return Ok(()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => to_pretty(rec)?,
            Err(e) => return Err(e.into()),
        };
        write_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Move record into `retired/` (soft delete — keeps the working
    /// dir intact, drops it from the launch-modal dropdown). Idempotent.
    pub fn retire(&self, instance_id: &str) -> Result<(), RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let from = self.active_path(instance_id);
        if !from.exists() {
            return Ok(());
        }
        rename_atomic(&from, &self.retired_path(instance_id))?;
        Ok(())
    }

    /// Move record back from `retired/` to active. Idempotent.
    pub fn unretire(&self, instance_id: &str) -> Result<(), RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let from = self.retired_path(instance_id);
        if !from.exists() {
            return Ok(());
        }
        rename_atomic(&from, &self.active_path(instance_id))?;
        Ok(())
    }

    /// Hard-delete (drops both active and retired files). Mirrors
    /// SQLite `instance_delete`.
    pub fn hard_delete(&self, instance_id: &str) -> Result<(), RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        for p in [self.active_path(instance_id), self.retired_path(instance_id)] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Whether an active record exists for `instance_id`. Doesn't
    /// validate — useful for migration idempotency checks. Use
    /// [`Self::exists_anywhere`] when retired records should also
    /// count (e.g. so migration doesn't resurrect a hidden agent).
    pub fn exists(&self, instance_id: &str) -> bool {
        self.active_path(instance_id).exists()
    }

    /// Read a single active record by id, without scanning the whole
    /// tree. `Ok(None)` when there's no active file for `instance_id`
    /// (never checks `retired/`). Callers that need to patch a single
    /// field (e.g. propagating a fresh `session_id`) must read-modify-
    /// upsert via this rather than constructing a partial record —
    /// `upsert`'s merge overwrites every field present in the struct,
    /// so a partially-populated record would clobber the rest with
    /// defaults.
    pub fn get(&self, instance_id: &str) -> Result<Option<NamedAgentRecord>, RegistryError> {
        let path = self.active_path(instance_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_and_validate(&path, instance_id)?))
    }

    /// Whether a record exists in either active or retired. Used by
    /// migration to skip already-tombstoned records (avoid
    /// resurrecting a user's deliberate "Forget agent" via a
    /// per-version SQLite row that still has `display_hidden = 0`).
    pub fn exists_anywhere(&self, instance_id: &str) -> bool {
        self.active_path(instance_id).exists() || self.retired_path(instance_id).exists()
    }

    /// Read every valid active record. Invalid files are skipped +
    /// logged. PR A doesn't wire this into the RPC path; included so
    /// PR B can swap reads over without further restructuring.
    pub fn list_active(&self) -> Result<Vec<NamedAgentRecord>, RegistryError> {
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
                        "registry: skipping invalid record"
                    );
                }
            }
        }
        Ok(out)
    }
}

fn read_and_validate(path: &Path, stem: &str) -> Result<NamedAgentRecord, RegistryError> {
    let bytes = std::fs::read(path)?;
    let rec: NamedAgentRecord = serde_json::from_slice(&bytes)?;
    validate(stem, &rec)?;
    Ok(rec)
}

fn to_pretty(rec: &NamedAgentRecord) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(rec)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Merge an in-memory record into an on-disk file's raw JSON,
/// preserving fields beyond this binary's struct shape.
///
/// Returns `Ok(None)` when the writer must refuse the merge (corrupt
/// JSON, missing `schema_version`, or `schema_version` above
/// `MAX_SUPPORTED_SCHEMA`). The caller treats `None` as a skip and
/// leaves the on-disk file intact.
fn merge_for_write(
    existing: &[u8],
    rec: &NamedAgentRecord,
) -> Result<Option<Vec<u8>>, RegistryError> {
    let on_disk: Value = match serde_json::from_slice(existing) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "registry: existing file is unparseable JSON — refusing to overwrite (may be a newer schema)"
            );
            return Ok(None);
        }
    };
    // Use `try_from` so any number above `u32::MAX` is treated as
    // unparseable rather than wrapping into the supported range. A
    // wrap would let an oversized envelope downgrade-bypass the
    // forward-compat guard below.
    let on_disk_version = on_disk
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok());
    match on_disk_version {
        Some(v) if v > MAX_SUPPORTED_SCHEMA => {
            tracing::warn!(
                on_disk = v,
                writer_max = MAX_SUPPORTED_SCHEMA,
                "registry: on-disk schema_version > writer max — refusing downgrade"
            );
            return Ok(None);
        }
        Some(_) => {}
        None => {
            tracing::warn!(
                "registry: existing file lacks schema_version — refusing to overwrite"
            );
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

/// Overwrite `target`'s top-level keys with `updates`' keys. Keys in
/// `target` that aren't in `updates` are preserved. Recurses into
/// the `data` sub-object so future fields inside `data` also survive.
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
