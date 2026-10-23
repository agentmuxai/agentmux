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
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

use super::atomic::{rename_atomic, write_atomic};
use super::def_slug_index::{SlugEntry, SlugIndex};
use super::def_schema::{validate, DefValidationError, DefinitionRecord, DEF_MAX_SUPPORTED_SCHEMA};

#[derive(Debug, Error)]
pub enum DefStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation: {0}")]
    Validation(#[from] DefValidationError),
    #[error("definition {id} is being written by another process (waited {waited_ms} ms); try again")]
    Locked { id: String, waited_ms: u64 },
}

pub struct DefinitionStore {
    root: PathBuf,
    /// How long a writer waits for a record's lock ([`Self::with_record_locked`]).
    lock_timeout: Duration,
    /// Every record's slug, parsed once per change ([`Self::slug_index`]).
    slugs: SlugIndex,
}

impl DefinitionStore {
    /// Open or create the store rooted at `root`. Ensures the active dir
    /// and `retired/` subdir both exist.
    pub fn open(root: PathBuf) -> Result<Self, DefStoreError> {
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(root.join("retired"))?;
        Ok(Self {
            root,
            lock_timeout: super::def_record_lock::LOCK_TIMEOUT,
            slugs: SlugIndex::default(),
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

    /// Run `f` with record `id` locked against every other writer, in this
    /// process or another (`def_record_lock`): a read, merge and write inside
    /// it can't interleave with another one. Every write below takes it;
    /// a rename that has to read and rewrite the record takes it too. Waits
    /// up to the store's lock timeout, then fails with
    /// [`DefStoreError::Locked`] having written nothing.
    pub fn with_record_locked<T>(
        &self,
        id: &str,
        f: impl FnOnce(&LockedRecord<'_>) -> Result<T, DefStoreError>,
    ) -> Result<T, DefStoreError> {
        let lock = super::def_record_lock::lock_record(&self.root, id, self.lock_timeout)?;
        let out = f(&LockedRecord { store: self, id: id.to_string() });
        drop(lock);
        out
    }

    /// The host-wide slug allocation lock, held until the guard drops. A
    /// new agent's slug is picked against every slug the shared store holds
    /// and then published by mirroring its record: whoever inserts holds this
    /// across both, so two channels or instances creating agents at once
    /// can't both pick the same free slug. Same bounded wait as a record's
    /// lock; [`DefStoreError::Locked`] names it.
    pub fn lock_slug_allocation(&self) -> Result<SlugAllocationLock, DefStoreError> {
        let file = super::def_record_lock::lock_record(&self.root, super::def_record_lock::SLUG_ALLOCATION, self.lock_timeout)?;
        Ok(SlugAllocationLock { _file: file })
    }

    #[cfg(test)]
    pub(crate) fn set_lock_timeout(&mut self, timeout: Duration) {
        self.lock_timeout = timeout;
    }

    /// Insert or update a definition. Preserves unknown top-level + `data`
    /// fields on an existing file (forward-compat); refuses to overwrite a
    /// corrupt or higher-schema file. An existing record's slug and former
    /// slugs are kept ([`keep_slug_and_former_slugs`]): a rename is not an
    /// update. Holds the record's lock across the read, merge and write.
    pub fn upsert(&self, rec: &DefinitionRecord) -> Result<(), DefStoreError> {
        self.with_record_locked(&rec.data.id, |locked| locked.upsert(rec))
    }

    /// [`Self::upsert`]'s body; the caller holds the record's lock.
    fn upsert_locked(&self, rec: &DefinitionRecord) -> Result<(), DefStoreError> {
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
        self.slugs.invalidate(&rec.data.id);
        Ok(())
    }

    /// Soft-delete → move into `retired/` (tombstone). Idempotent. The
    /// tombstone prevents another channel's migration from resurrecting an
    /// agent the user deliberately deleted.
    pub fn retire(&self, id: &str) -> Result<(), DefStoreError> {
        self.with_record_locked(id, |_| {
            let from = self.active_path(id);
            if from.exists() {
                rename_atomic(&from, &self.retired_path(id))?;
                self.slugs.invalidate(id);
            }
            Ok(())
        })
    }

    /// Move a record back from `retired/` to active. Idempotent.
    pub fn unretire(&self, id: &str) -> Result<(), DefStoreError> {
        self.with_record_locked(id, |_| {
            let from = self.retired_path(id);
            if from.exists() {
                rename_atomic(&from, &self.active_path(id))?;
                self.slugs.invalidate(id);
            }
            Ok(())
        })
    }

    /// Hard-delete (drops both active and retired files). Mirrors SQLite
    /// `agent_def_delete`. Use [`Self::retire`] when a resurrection-proof
    /// tombstone is wanted instead.
    pub fn hard_delete(&self, id: &str) -> Result<(), DefStoreError> {
        self.with_record_locked(id, |_| {
            for p in [self.active_path(id), self.retired_path(id)] {
                match std::fs::remove_file(&p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            self.slugs.invalidate(id);
            Ok(())
        })
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

    /// Every valid active record's slug and former slugs, by id: what
    /// [`Self::list_active`] would give, without reading a file that hasn't
    /// changed since the last call (`def_slug_index`). For lookups that run
    /// per message.
    pub fn slug_index(&self) -> Result<std::sync::Arc<std::collections::HashMap<String, SlugEntry>>, DefStoreError> {
        Ok(self.slugs.all(&self.root)?)
    }

    /// One active record's slug and former slugs, read from its file now,
    /// for a caller that must not act on a guess (deleting the agent). `None`
    /// only when there is no active file. A file that can't be read, or whose
    /// slug fields can't be (`def_slug_index::slug_fields`), is an error. A
    /// record this build otherwise can't load (a newer schema, a missing name
    /// or provider) still answers when its slug fields can be read.
    pub fn slug_of_strict(&self, id: &str) -> Result<Option<SlugEntry>, DefStoreError> {
        let bytes = match std::fs::read(self.active_path(id)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        super::def_slug_index::slug_fields(id, &bytes)
            .map(Some)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{id}.json: {why}")).into())
    }

    /// Every active record's slug and former slugs, for allocating a new
    /// slug: [`Self::slug_index`]'s cache, but every record counts, newer
    /// schemas included, and anything that can't be read (the directory, a
    /// file, a record's slug fields) is an error rather than skipped.
    pub fn slug_index_strict(&self) -> Result<std::collections::HashMap<String, SlugEntry>, DefStoreError> {
        self.slugs
            .all_strict(&self.root)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, why).into())
    }

    /// One active record's slug and former slugs, from the same cache;
    /// stats only that record's file.
    pub fn slug_of(&self, id: &str) -> Result<Option<SlugEntry>, DefStoreError> {
        Ok(self.slugs.one(&self.root, id)?)
    }
}

/// Held slug allocation ([`DefinitionStore::lock_slug_allocation`]).
pub struct SlugAllocationLock {
    _file: std::fs::File,
}

/// One definition record, locked by [`DefinitionStore::with_record_locked`].
pub struct LockedRecord<'a> {
    store: &'a DefinitionStore,
    id: String,
}

impl LockedRecord<'_> {
    /// The active record, as [`DefinitionStore::get`] reads it.
    pub fn get(&self) -> Result<Option<DefinitionRecord>, DefStoreError> {
        self.store.get(&self.id)
    }

    /// [`DefinitionStore::upsert`] of this record, under the lock already held.
    pub fn upsert(&self, rec: &DefinitionRecord) -> Result<(), DefStoreError> {
        if rec.data.id != self.id {
            return Err(DefValidationError::IdMismatch { filename: self.id.clone(), id: rec.data.id.clone() }.into());
        }
        self.store.upsert_locked(rec)
    }

    /// The active file's bytes, as they are on disk; `None` if there is none.
    pub fn raw(&self) -> Result<Option<Vec<u8>>, DefStoreError> {
        match std::fs::read(self.store.active_path(&self.id)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Replace the active file with `bytes`, as they are: the one write that
    /// may change a non-empty slug, which [`Self::upsert`] never does. For
    /// the fork slug migration's rename and its undo
    /// (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` §4 Phase 2, S1). `bytes`
    /// must be a record of this id and a schema this build writes; nothing is
    /// written otherwise, and a missing active file (a deleted agent) is not
    /// recreated.
    pub fn replace_raw(&self, bytes: &[u8]) -> Result<(), DefStoreError> {
        let path = self.store.active_path(&self.id);
        if !path.exists() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}.json is not active", self.id)).into());
        }
        let value: Value = serde_json::from_slice(bytes)?;
        let version = value.get("schema_version").and_then(Value::as_u64).unwrap_or(0);
        if version == 0 || version > u64::from(DEF_MAX_SUPPORTED_SCHEMA) {
            return Err(DefValidationError::UnsupportedSchema {
                version: version as u32,
                min: super::def_schema::DEF_MIN_SUPPORTED_SCHEMA,
                max: DEF_MAX_SUPPORTED_SCHEMA,
            }
            .into());
        }
        if value.pointer("/data/id").and_then(Value::as_str) != Some(self.id.as_str()) {
            let id = value.pointer("/data/id").and_then(Value::as_str).unwrap_or("").to_string();
            return Err(DefValidationError::IdMismatch { filename: self.id.clone(), id }.into());
        }
        write_atomic(&path, bytes)?;
        self.store.slugs.invalidate(&self.id);
        Ok(())
    }
}

/// A definition record as [`DefinitionStore`] writes one after a merge:
/// pretty JSON and a trailing newline.
pub fn record_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_and_validate(path: &Path, stem: &str) -> Result<DefinitionRecord, DefStoreError> {
    parse_and_validate(&std::fs::read(path)?, stem)
}

pub(super) fn parse_and_validate(bytes: &[u8], stem: &str) -> Result<DefinitionRecord, DefStoreError> {
    let rec: DefinitionRecord = serde_json::from_slice(bytes)?;
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
    let mut updates = serde_json::to_value(rec)?;
    keep_slug_and_former_slugs(&merged, &mut updates);
    merge_known(&mut merged, &updates);
    let mut bytes = serde_json::to_vec_pretty(&merged)?;
    bytes.push(b'\n');
    Ok(Some(bytes))
}

/// A shared definition's slug is never changed by an update
/// (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.2). Every channel and version
/// keeps its own copy of the slug in its `objects.db`, and the copy a writer
/// mirrors back may be one this record has moved on from (a former slug), or
/// a local collision suffix. So a non-empty slug on disk is kept whatever the
/// update carries, and its `former_slugs` are kept too, with any new ones the
/// update carries appended. An empty slug on disk takes the update's.
fn keep_slug_and_former_slugs(on_disk: &Value, updates: &mut Value) {
    let disk_slug = on_disk
        .pointer("/data/slug")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());
    let disk_former: Vec<Value> = on_disk
        .pointer("/data/former_slugs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(data) = updates.get_mut("data").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(disk_slug) = disk_slug {
        let incoming = data.get("slug").and_then(Value::as_str).unwrap_or("").to_string();
        if incoming != disk_slug {
            let retired = disk_former
                .iter()
                .any(|f| f.get("slug").and_then(Value::as_str) == Some(incoming.as_str()));
            let def_id = data.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            tracing::warn!(
                def_id = %def_id,
                shared_slug = disk_slug,
                refused_slug = %incoming,
                refused_is_former = retired,
                "def registry: an update carried a different slug; the shared record's is kept"
            );
            data.insert("slug".to_string(), Value::String(disk_slug.to_string()));
        }
    }
    if !disk_former.is_empty() {
        let mut all = disk_former;
        for f in data.get("former_slugs").and_then(Value::as_array).into_iter().flatten() {
            if !all.iter().any(|g| g.get("slug") == f.get("slug")) {
                all.push(f.clone());
            }
        }
        data.insert("former_slugs".to_string(), Value::Array(all));
    }
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
                former_slugs: Vec::new(),
                content: Vec::new(),
                skills: Vec::new(),
            },
        }
    }

    fn former(slug: &str, retired_at: i64) -> crate::registry::FormerSlug {
        crate::registry::FormerSlug { slug: slug.to_string(), retired_at }
    }

    /// P0.2: a channel whose `objects.db` still holds a slug the shared
    /// record has retired mirrors an edit back. The edit lands; the slug and
    /// the former slugs don't change.
    #[test]
    fn an_update_cannot_put_back_a_former_slug() {
        let (_t, s) = store();
        let mut renamed = rec("aaa", "Loap #3");
        renamed.data.slug = "loap-3".to_string();
        renamed.data.former_slugs = vec![former("loap-loap-2-loap-3", 5)];
        s.upsert(&renamed).unwrap();

        let mut stale = rec("aaa", "Loap #3 (edited)");
        stale.data.slug = "loap-loap-2-loap-3".to_string();
        s.upsert(&stale).unwrap();

        let got = s.get("aaa").unwrap().unwrap();
        assert_eq!(got.data.name, "Loap #3 (edited)", "the edit itself still mirrors");
        assert_eq!(got.data.slug, "loap-3");
        assert_eq!(got.data.former_slugs, vec![former("loap-loap-2-loap-3", 5)]);
    }

    /// Any other slug is refused the same way (a local collision suffix), and
    /// an empty slug on disk takes the update's.
    #[test]
    fn an_update_keeps_the_shared_slug_unless_it_is_empty() {
        let (_t, s) = store();
        let mut first = rec("aaa", "Foo");
        first.data.slug = "foo".to_string();
        s.upsert(&first).unwrap();
        let mut suffixed = first.clone();
        suffixed.data.slug = "foo-2".to_string();
        s.upsert(&suffixed).unwrap();
        assert_eq!(s.get("aaa").unwrap().unwrap().data.slug, "foo");

        let mut blank = rec("bbb", "Bar");
        blank.data.slug = String::new();
        s.upsert(&blank).unwrap();
        blank.data.slug = "bar".to_string();
        s.upsert(&blank).unwrap();
        assert_eq!(s.get("bbb").unwrap().unwrap().data.slug, "bar");
    }

    /// Former slugs are never dropped: neither by a writer that doesn't know
    /// the field (an older build, whose update has none) nor by one carrying
    /// an older copy of the list; new entries are appended.
    #[test]
    fn former_slugs_only_grow() {
        let (_t, s) = store();
        let path = s.root().join("aaa.json");
        let raw = serde_json::json!({
            "schema_version": 1,
            "data": {
                "id": "aaa", "name": "Demo", "provider": "claude", "slug": "c",
                "former_slugs": [{ "slug": "a", "retired_at": 1 }, { "slug": "b", "retired_at": 2 }]
            }
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

        s.upsert(&rec("aaa", "Renamed")).unwrap();
        let got = s.get("aaa").unwrap().unwrap();
        assert_eq!(got.data.former_slugs, vec![former("a", 1), former("b", 2)]);

        let mut partial = got.clone();
        partial.data.former_slugs = vec![former("a", 1), former("z", 9)];
        s.upsert(&partial).unwrap();
        let got = s.get("aaa").unwrap().unwrap();
        assert_eq!(got.data.former_slugs, vec![former("a", 1), former("b", 2), former("z", 9)]);
        assert_eq!(got.data.slug, "c");
    }

    /// Backdate a record's file, as an index built a while after the last
    /// write sees it (out of the racy window).
    fn age(s: &DefinitionStore, id: &str, secs: u64) {
        let file = std::fs::File::options().write(true).open(s.root().join(format!("{id}.json"))).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs)).unwrap();
    }

    fn with_slug(id: &str, slug: &str) -> DefinitionRecord {
        let mut r = rec(id, "Demo");
        r.data.slug = slug.to_string();
        r
    }

    /// Once warm, a lookup parses nothing: not for the whole index, and not
    /// for one record.
    #[test]
    fn a_warm_slug_index_parses_no_file() {
        let (_t, s) = store();
        for i in 0..5 {
            let id = format!("id-{i}");
            s.upsert(&with_slug(&id, &format!("slug-{i}"))).unwrap();
            age(&s, &id, 60);
        }
        let first = s.slug_index().unwrap();
        assert_eq!(first.len(), 5);
        assert_eq!(s.slugs.parses(), 5);
        for _ in 0..20 {
            assert_eq!(s.slug_index().unwrap().get("id-3").map(|e| e.slug.as_str()), Some("slug-3"));
            assert_eq!(s.slug_of("id-1").unwrap().map(|e| e.slug), Some("slug-1".to_string()));
        }
        assert_eq!(s.slugs.parses(), 5, "nothing reparsed");
        assert!(std::sync::Arc::ptr_eq(&first, &s.slug_index().unwrap()), "the same map is handed out");
    }

    /// A write through this store invalidates its record: the next lookup
    /// reparses that file only. A retired record leaves the index.
    #[test]
    fn an_in_process_write_invalidates_the_slug_index() {
        let (_t, s) = store();
        for id in ["aaa", "bbb"] {
            s.upsert(&with_slug(id, id)).unwrap();
            age(&s, id, 60);
        }
        s.slug_index().unwrap();
        let warm = s.slugs.parses();

        // A same-size edit, then the old timestamp put back: the file's
        // stamp is unchanged, so only the invalidation can trigger a reparse.
        let path = s.root().join("aaa.json");
        let before = std::fs::metadata(&path).unwrap();
        let mut edited = with_slug("aaa", "aaa");
        edited.data.name = "Dema".to_string();
        s.upsert(&edited).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(before.modified().unwrap()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before.len());
        s.slug_index().unwrap();
        assert_eq!(s.slugs.parses(), warm + 1, "only the written record is reparsed");

        let mut renamed = with_slug("aaa", "aaa");
        renamed.data.former_slugs = vec![former("aaa-old", 1)];
        s.upsert(&renamed).unwrap();
        s.retire("bbb").unwrap();
        let index = s.slug_index().unwrap();
        assert_eq!(index["aaa"].former, vec!["aaa-old".to_string()]);
        assert!(!index.contains_key("bbb"));
    }

    /// Another process's write is picked up from the file's stamp: a new
    /// file, a changed one, and a removed one. A same-size rewrite within
    /// the racy window of the last parse is caught too.
    #[test]
    fn an_out_of_process_change_reaches_the_slug_index() {
        let (_t, s) = store();
        s.upsert(&with_slug("aaa", "loap-loap-2")).unwrap();
        age(&s, "aaa", 60);
        assert_eq!(s.slug_index().unwrap()["aaa"].slug, "loap-loap-2");

        // Another instance renames it and adds an agent, writing the files directly.
        let mut moved = with_slug("aaa", "loap-2");
        moved.data.former_slugs = vec![former("loap-loap-2", 1)];
        std::fs::write(s.root().join("aaa.json"), serde_json::to_vec_pretty(&moved).unwrap()).unwrap();
        age(&s, "aaa", 30);
        std::fs::write(s.root().join("ccc.json"), serde_json::to_vec_pretty(&with_slug("ccc", "c")).unwrap()).unwrap();
        let index = s.slug_index().unwrap();
        assert_eq!(index["aaa"].slug, "loap-2");
        assert_eq!(index["aaa"].former, vec!["loap-loap-2".to_string()]);
        assert_eq!(s.slug_of("ccc").unwrap().map(|e| e.slug), Some("c".to_string()));

        std::fs::remove_file(s.root().join("ccc.json")).unwrap();
        assert!(!s.slug_index().unwrap().contains_key("ccc"));
        assert_eq!(s.slug_of("ccc").unwrap(), None);

        // Racy: parsed just after a write, then rewritten with the same size
        // and the same timestamp. The stamp alone can't tell; the index
        // doesn't trust it yet.
        let path = s.root().join("ddd.json");
        std::fs::write(&path, serde_json::to_vec(&with_slug("ddd", "d-one")).unwrap()).unwrap();
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(s.slug_of("ddd").unwrap().unwrap().slug, "d-one");
        std::fs::write(&path, serde_json::to_vec(&with_slug("ddd", "d-two")).unwrap()).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(stamp).unwrap();
        assert_eq!(s.slug_of("ddd").unwrap().unwrap().slug, "d-two");
    }

    /// P0.2 across stores: two `DefinitionStore`s on one directory, as two
    /// channels have. While one holds the record's lock for a rename (the
    /// slug and a former slug written directly, as Phase 2's will be), the
    /// other's update waits, then merges onto the renamed record: the slug is
    /// not reverted and the former slug is not dropped.
    #[test]
    fn an_update_from_another_store_waits_for_the_record_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("definitions");
        let a = std::sync::Arc::new(DefinitionStore::open(root.clone()).unwrap());
        let b = DefinitionStore::open(root).unwrap();
        a.upsert(&with_slug("aaa", "loap-loap-2")).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let holder = {
            let a = a.clone();
            std::thread::spawn(move || {
                a.with_record_locked("aaa", |locked| {
                    let mut rec = locked.get()?.unwrap();
                    tx.send(()).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    rec.data.slug = "loap-2".to_string();
                    rec.data.former_slugs = vec![former("loap-loap-2", 1)];
                    Ok(std::fs::write(a.root().join("aaa.json"), serde_json::to_vec_pretty(&rec)?)?)
                })
            })
        };
        rx.recv().unwrap();
        let started = std::time::Instant::now();
        let mut stale = with_slug("aaa", "loap-loap-2");
        stale.data.name = "Edited".to_string();
        b.upsert(&stale).unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(200), "the update waited for the lock");
        holder.join().unwrap().unwrap();

        let got = b.get("aaa").unwrap().unwrap();
        assert_eq!(got.data.name, "Edited");
        assert_eq!(got.data.slug, "loap-2");
        assert_eq!(got.data.former_slugs, vec![former("loap-loap-2", 1)]);
    }

    /// A writer that can't get the lock in time fails with `Locked` and
    /// writes nothing.
    #[test]
    fn a_lock_timeout_fails_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("definitions");
        let a = std::sync::Arc::new(DefinitionStore::open(root.clone()).unwrap());
        let mut b = DefinitionStore::open(root).unwrap();
        b.set_lock_timeout(std::time::Duration::from_millis(100));
        a.upsert(&with_slug("aaa", "aaa")).unwrap();
        let before = std::fs::read(a.root().join("aaa.json")).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let holder = {
            let a = a.clone();
            std::thread::spawn(move || {
                a.with_record_locked("aaa", |_| {
                    tx.send(()).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    Ok(())
                })
            })
        };
        rx.recv().unwrap();
        let mut edit = with_slug("aaa", "aaa");
        edit.data.name = "Changed".to_string();
        assert!(matches!(b.upsert(&edit), Err(DefStoreError::Locked { .. })));
        assert!(matches!(b.retire("aaa"), Err(DefStoreError::Locked { .. })));
        assert_eq!(std::fs::read(a.root().join("aaa.json")).unwrap(), before, "nothing written");
        holder.join().unwrap().unwrap();
    }

    /// The strict index fails when the directory can't be listed or a
    /// record's slug fields can't be read, and reads a newer schema's.
    #[test]
    fn the_strict_slug_index_fails_closed_and_reads_newer_schemas() {
        let (_t, s) = store();
        s.upsert(&with_slug("aaa", "a")).unwrap();
        let future = r#"{"schema_version":99,"data":{"id":"bbb","slug":"b","former_slugs":[{"slug":"b-old"}]}}"#;
        std::fs::write(s.root().join("bbb.json"), future).unwrap();
        let strict = s.slug_index_strict().unwrap();
        assert_eq!(strict["bbb"].former, vec!["b-old".to_string()]);
        assert!(!s.slug_index().unwrap().contains_key("bbb"), "the lenient index lists what list_active does");

        for bad in [r#"{"data":{"id":"ccc","former_slugs":"x"}}"#, "{", r#"{"data":{"id":"other"}}"#] {
            std::fs::write(s.root().join("ccc.json"), bad).unwrap();
            assert!(s.slug_index_strict().is_err(), "{bad}");
        }
        std::fs::remove_file(s.root().join("ccc.json")).unwrap();
        assert!(s.slug_index_strict().is_ok());

        std::fs::remove_dir_all(s.root()).unwrap();
        assert!(s.slug_index_strict().is_err(), "an unlistable directory");
    }

    /// Slug allocation is one host-wide lock: a second store waits for it,
    /// then fails with `Locked`; once released it is free again.
    #[test]
    fn slug_allocation_is_exclusive_and_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("definitions");
        let a = DefinitionStore::open(root.clone()).unwrap();
        let mut b = DefinitionStore::open(root).unwrap();
        b.set_lock_timeout(std::time::Duration::from_millis(100));
        let held = a.lock_slug_allocation().unwrap();
        assert!(matches!(b.lock_slug_allocation(), Err(DefStoreError::Locked { .. })));
        drop(held);
        assert!(b.lock_slug_allocation().is_ok());
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
