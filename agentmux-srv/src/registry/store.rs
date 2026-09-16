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

/// Which records an operation is allowed to touch.
///
/// The registry has two ways to say "this record belongs to X": the file
/// key (`<instance_id>.json`) and the record's own `definition_id`. Since
/// the agent-concept consolidation those are the same value for an agent —
/// but only for records `m0026_registry_agent_id_rekey` re-keyed, and that
/// pass no-ops once `db_agent_instances` is dropped (schema v32), so a real
/// install still holds launch-keyed records where they differ. Matching
/// both is what makes Delete actually remove an agent's row
/// ([`Registry::hard_delete_for_agent`]).
///
/// It is also what makes a TEMPLATE dangerous: a template's id is the
/// `definition_id` of every legacy launch record for the real agents
/// launched from it, so a wide match would delete/retire/rename THEIR
/// records along with the template. Hence this type rather than a `bool`
/// or a per-call-site `if` — ReAgent found the same omission at four
/// separate call sites on PR #3262 (rounds 3 and 4), which is a sign the
/// decision belongs in the signature, not in each caller's memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordScope {
    /// Only the record whose FILE KEY is this id. The correct scope for a
    /// template: its own record, and nothing belonging to its launches.
    FileKeyOnly,
    /// Every record for this agent — file key or `definition_id`.
    Agent,
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
        self.upsert_unlocked(rec)
    }

    /// [`Self::upsert`] without taking `write_lock`, for callers already
    /// holding it. `write_lock` is a plain `Mutex` — re-entering it from a
    /// locked section would deadlock, not nest.
    fn upsert_unlocked(&self, rec: &NamedAgentRecord) -> Result<(), RegistryError> {
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

    /// Set `instance_name` on every ACTIVE record for `agent_id`. Returns
    /// how many files changed.
    ///
    /// Every record, not just the canonical one, because
    /// `listrecentsessions` dedupes rows by `(definition_id, instance_name)`
    /// — renaming one of an agent's records and not the others splits it
    /// into two picker rows, one under each name. (An agent can hold more
    /// than one record for the same reason
    /// [`Self::hard_delete_for_agent`] exists.)
    ///
    /// Active tree only: `upsert` writes to the active path, so touching a
    /// retired record here would resurrect a deliberately-forgotten agent.
    /// A retired record keeps its old name until `unretire_for_agent`
    /// brings it back.
    pub fn set_instance_name_for_agent(
        &self,
        agent_id: &str,
        scope: RecordScope,
        instance_name: &str,
    ) -> Result<usize, RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut renamed = 0usize;
        for path in records_for_agent(&self.root, agent_id, scope)? {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(mut rec) = serde_json::from_slice::<NamedAgentRecord>(&bytes) else {
                continue;
            };
            // An unnamed record fails validation on READ, so it is already
            // invisible to the picker — but `upsert` doesn't validate, so
            // one can sit on disk, and this scan reads raw JSON (matching on
            // `definition_id` is the whole point). Naming it here would
            // "repair" it into a visible row for an agent that never had
            // one. Pinned by
            // `set_instance_name_for_agent_leaves_an_unnamed_record_invisible`.
            if rec.data.instance_name.is_empty() || rec.data.instance_name == instance_name {
                continue;
            }
            rec.data.instance_name = instance_name.to_string();
            self.upsert_unlocked(&rec)?;
            renamed += 1;
        }
        Ok(renamed)
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

    /// Hard-delete by FILE KEY only (drops both active and retired files),
    /// returning how many files were removed.
    ///
    /// The narrow counterpart of [`Self::hard_delete_for_agent`]. Deleting a
    /// TEMPLATE needs this one: a template's id can legitimately appear as
    /// the `definition_id` of a legacy, never-re-keyed launch record
    /// belonging to a real agent launched from it, and matching on
    /// `definition_id` would take that agent's record along with the
    /// template (ReAgent P1 round 3 on PR #3262). A template's own record,
    /// if it has one, is file-keyed, so this is the complete sweep for one.
    pub fn hard_delete(&self, instance_id: &str) -> Result<usize, RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = 0usize;
        for p in [self.active_path(instance_id), self.retired_path(instance_id)] {
            match std::fs::remove_file(&p) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(removed)
    }

    /// Drop every record — active AND retired — that belongs to the agent
    /// `agent_id`, matching on either the record's file key (its instance
    /// id) or its `definition_id`. Returns how many files were removed.
    ///
    /// **Why this exists alongside [`Self::hard_delete`].** That one keys
    /// only on the file name. Since the agent-concept consolidation an
    /// agent's instance id and definition id are the same value, so the two
    /// agree — but only for records this tree has been re-keyed to.
    /// `m0026_registry_agent_id_rekey` does that re-keying by reading
    /// `db_agent_instances`, and returns a no-op the moment that legacy
    /// table is gone (v32 dropped it). An install that consolidated before
    /// m0026 could run therefore still holds one *launch*-keyed file per
    /// agent, in addition to the correctly-keyed one.
    ///
    /// Deleting such an agent removed only the correctly-keyed file,
    /// leaving the launch-keyed one active with no definition behind it —
    /// which `listrecentsessions` still renders as a row (no provider, so
    /// no icon, and `"(missing definition)"`). The delete looked like it
    /// half-worked: the icon vanished, the card stayed.
    ///
    /// Both keys are checked rather than just `definition_id` because
    /// callers legitimately hold either one: a consolidated agent id, or
    /// the launch id of a registry-only row this channel's SQLite has never
    /// seen. Matching both is a strict superset of [`Self::hard_delete`],
    /// and a cross-key collision would require the same uuid to name two
    /// different agents.
    pub fn hard_delete_for_agent(
        &self,
        agent_id: &str,
        scope: RecordScope,
    ) -> Result<usize, RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = 0usize;
        for dir in [self.root.clone(), self.root.join("retired")] {
            for path in records_for_agent(&dir, agent_id, scope)? {
                match std::fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(removed)
    }

    /// Retire every active record for `agent_id`, matched the same way
    /// [`Self::hard_delete_for_agent`] matches. A launch-keyed record left
    /// active by the file-keyed [`Self::retire`] is exactly the "'Forget
    /// agent' retires the NEW key while the stale file stays active, so the
    /// forgotten agent reappears" failure `m0026_registry_agent_id_rekey`'s
    /// own doc comment predicted.
    pub fn retire_for_agent(&self, agent_id: &str, scope: RecordScope) -> Result<usize, RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.move_for_agent(agent_id, scope, self.root.clone(), self.root.join("retired"))
    }

    /// Inverse of [`Self::retire_for_agent`].
    pub fn unretire_for_agent(&self, agent_id: &str, scope: RecordScope) -> Result<usize, RegistryError> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.move_for_agent(agent_id, scope, self.root.join("retired"), self.root.clone())
    }

    /// Move an agent's records between the active and retired trees,
    /// keeping each file's own name (the instance id) so the record stays
    /// addressable by [`Self::get`]. Caller holds `write_lock`.
    fn move_for_agent(
        &self,
        agent_id: &str,
        scope: RecordScope,
        from_dir: PathBuf,
        to_dir: PathBuf,
    ) -> Result<usize, RegistryError> {
        let mut moved = 0usize;
        for path in records_for_agent(&from_dir, agent_id, scope)? {
            let Some(name) = path.file_name() else {
                continue;
            };
            rename_atomic(&path, &to_dir.join(name))?;
            moved += 1;
        }
        Ok(moved)
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

/// Paths of the `*.json` files under `dir` whose record belongs to
/// `agent_id` — by file key (instance id) or by `definition_id`. A missing
/// directory yields nothing (the `retired/` tree is created on open, but a
/// caller may point at a tree that predates it).
///
/// Unparseable files are matched on their file key alone and otherwise
/// skipped, the same way [`Registry::list_active`] skips them: a record
/// this can't read is also one that can never surface as a row, and
/// removing it on a guess would destroy whatever newer-schema binary
/// authored it. The file-key match is kept so an explicitly-addressed
/// delete still behaves like [`Registry::hard_delete`] did.
fn records_for_agent(
    dir: &Path,
    agent_id: &str,
    scope: RecordScope,
) -> Result<Vec<PathBuf>, RegistryError> {
    let mut out = Vec::new();
    if scope == RecordScope::FileKeyOnly {
        let p = dir.join(format!("{agent_id}.json"));
        if p.is_file() {
            out.push(p);
        }
        return Ok(out);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some(agent_id) {
            out.push(path);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(rec) = serde_json::from_slice::<NamedAgentRecord>(&bytes) else {
            continue;
        };
        if rec.data.definition_id == agent_id {
            out.push(path);
        }
    }
    Ok(out)
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
