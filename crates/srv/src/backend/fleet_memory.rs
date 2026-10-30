// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One Global Memory for the fleet
//! (docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md §4.6).
//!
//! A Global Memory entry with scope `fleet` (see `storage::bundle_scope`) is
//! pushed to every proven machine with the remote ops `memory.global.put` and
//! `memory.global.remove`, and a machine lists what it holds with
//! `memory.global.list` so the other side can catch it up. This module is the
//! machine-independent part: what an entry carries over the wire, which write
//! wins, and applying a received write to the local store. The transport that
//! carries the ops is `server::remote_ops`.
//!
//! **Last write wins.** An entry's `updated_at` is stamped by the machine that
//! wrote it and kept as is on every machine that applies it, so all machines
//! agree on it. The newer `updated_at` wins; on a tie the larger content hash
//! wins, so every machine picks the same side. A removal is a write too: it
//! carries the time it happened and wins or loses the same way, so an entry
//! removed on one machine and edited later on another comes back.
//!
//! **What is not carried.** `skills`, `provider` and `model` name things that
//! exist per machine, `sort_order` is a position on one machine's list, and
//! `mcp_servers` is always empty. AgentMux's own entries (system tier, `seed-`
//! ids) and the blank bundle never leave or enter. A write for an entry that is
//! `machine` scope here is skipped, so a machine keeps what its owner kept.
//!
//! **No echo.** A write applied from another machine is recorded with source
//! [`FLEET_SOURCE`]; the push on change (a later PR) does not push those back.

// The remote-ops handlers and the push on change call this in the next PRs, once the transport
// (`server::remote_ops`) has landed; until then only its tests do.
#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::storage::bundles::REMOVE_SOURCE;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::{Bundle, Store};

/// The `db_bundle_versions.source` of a write that came from another machine.
pub const FLEET_SOURCE: &str = "fleet";

/// Same bound as `globalmemory.write`: every agent's launch parses and injects this.
const MAX_INSTRUCTIONS: usize = 1024 * 1024;

/// The hash a removal carries, so a removal and an edit with the same
/// timestamp resolve the same way on every machine.
const REMOVED_HASH: &str = "removed";

fn default_json_object() -> String {
    "{}".to_string()
}

fn default_json_array() -> String {
    "[]".to_string()
}

/// A Global Memory entry as it travels between machines: `memory.global.put`'s params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    #[serde(default = "default_json_object")]
    pub instructions_by_provider: String,
    #[serde(default = "default_json_array")]
    pub context_files: String,
    /// When the writing machine saved it (ms since the epoch).
    pub updated_at: i64,
}

impl FleetEntry {
    /// The entry's content hash: what differs between two copies.
    pub fn hash(&self) -> String {
        content_hash(&self.name, &self.description, &self.instructions, &self.instructions_by_provider, &self.context_files)
    }
}

fn content_hash(name: &str, description: &str, instructions: &str, by_provider: &str, context_files: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [name, description, instructions, by_provider, context_files] {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

/// `memory.global.remove`'s params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetRemoval {
    pub id: String,
    /// When the writing machine removed it.
    pub removed_at: i64,
}

/// What a machine holds for one entry, without its text: `memory.global.list`'s rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetMeta {
    pub id: String,
    pub updated_at: i64,
    pub hash: String,
    pub removed: bool,
}

/// Who a write is attributed to on this machine: the remote agent as
/// `slug@machine`, and its uid when the transport knows it. Never the operator.
#[derive(Debug, Clone, Copy)]
pub struct Actor<'a> {
    pub label: &'a str,
    pub uid: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Created,
    Updated,
    /// The entry had been removed here; the newer write brought it back.
    Restored,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// The same content is already here.
    Same,
    /// What is here is newer.
    Older,
    /// This machine keeps the entry to itself.
    MachineScope,
    /// AgentMux's own entry, the blank bundle, or an id the fleet never carries.
    Protected,
    /// The id belongs to a bundle that is not Global Memory (an agent's own preset).
    NotMemory,
    /// A removal for an entry this machine already removed.
    AlreadyRemoved,
    /// A removal for an entry this machine never had.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Applied(Applied),
    Skipped(Skipped),
}

#[derive(Debug)]
pub enum FleetError {
    /// The request is malformed; nothing was written.
    Invalid(String),
    Store(StoreError),
}

impl std::fmt::Display for FleetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FleetError::Invalid(m) => f.write_str(m),
            FleetError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl From<StoreError> for FleetError {
    fn from(e: StoreError) -> Self {
        FleetError::Store(e)
    }
}

/// Whether a write with `(incoming_at, incoming_hash)` replaces what is here.
fn wins(incoming_at: i64, incoming_hash: &str, local_at: i64, local_hash: &str) -> bool {
    incoming_at > local_at || (incoming_at == local_at && incoming_hash > local_hash)
}

/// An id the fleet never carries: the blank bundle and AgentMux's re-seeded entries.
fn is_fleet_id(id: &str) -> bool {
    !id.is_empty() && id != "blank" && !id.starts_with("seed-")
}

fn validate_id(id: &str) -> Result<(), FleetError> {
    if id.is_empty() || id.len() > 200 {
        return Err(FleetError::Invalid("memory.global: id is required (at most 200 characters)".to_string()));
    }
    Ok(())
}

fn bundle_hash(b: &Bundle) -> String {
    content_hash(&b.name, &b.description, &b.instructions, &b.instructions_by_provider, &b.context_files)
}

/// The entries that follow the fleet, including those removed here since:
/// `memory.global.list`'s answer.
pub fn list_meta(store: &Store) -> Result<Vec<FleetMeta>, StoreError> {
    let machine_only = store.bundle_scope_machine_ids()?;
    let carried = |b: &Bundle| is_fleet_id(&b.id) && !b.is_system && !b.is_blank && !machine_only.contains(&b.id);
    let mut out: Vec<FleetMeta> = store
        .bundle_list_global()?
        .into_iter()
        .filter(carried)
        .map(|b| FleetMeta { hash: bundle_hash(&b), id: b.id, updated_at: b.updated_at, removed: false })
        .collect();
    out.extend(store.bundle_list_removed_global()?.into_iter().filter(carried).map(|b| FleetMeta {
        id: b.id,
        updated_at: b.updated_at,
        hash: REMOVED_HASH.to_string(),
        removed: true,
    }));
    Ok(out)
}

/// One entry to send, if it follows the fleet and is still Global Memory.
pub fn entry_for_push(store: &Store, id: &str) -> Result<Option<FleetEntry>, StoreError> {
    if !is_fleet_id(id) || store.bundle_scope_get(id)? != crate::backend::storage::bundle_scope::SCOPE_FLEET {
        return Ok(None);
    }
    Ok(store.bundle_get(id)?.filter(|b| b.is_global && !b.is_system && !b.is_blank).map(|b| FleetEntry {
        id: b.id,
        name: b.name,
        description: b.description,
        instructions: b.instructions,
        instructions_by_provider: b.instructions_by_provider,
        context_files: b.context_files,
        updated_at: b.updated_at,
    }))
}

/// One thing to send a machine that is behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Push {
    Put { id: String },
    Remove { id: String, removed_at: i64 },
}

/// What to send a machine, given what each side lists, so it catches up: the
/// entries it lacks or holds an older copy of, and the removals it has not
/// seen. Newest first. What the other machine holds newer is that machine's to
/// push when it sees this one.
pub fn to_push(local: &[FleetMeta], remote: &[FleetMeta]) -> Vec<Push> {
    let theirs: HashMap<&str, &FleetMeta> = remote.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut out: Vec<(i64, Push)> = Vec::new();
    for mine in local {
        match theirs.get(mine.id.as_str()) {
            Some(t) if t.hash == mine.hash => continue,
            Some(t) if !wins(mine.updated_at, &mine.hash, t.updated_at, &t.hash) => continue,
            // Nothing to tell a machine that never had an entry that is gone.
            None if mine.removed => continue,
            _ => {}
        }
        let push = if mine.removed {
            Push::Remove { id: mine.id.clone(), removed_at: mine.updated_at }
        } else {
            Push::Put { id: mine.id.clone() }
        };
        out.push((mine.updated_at, push));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, p)| p).collect()
}

/// Apply a write received from another machine.
pub fn apply_put(store: &Store, entry: &FleetEntry, actor: Actor<'_>) -> Result<Outcome, FleetError> {
    validate_id(&entry.id)?;
    if entry.name.trim().is_empty() {
        return Err(FleetError::Invalid("memory.global.put: name is required".to_string()));
    }
    if entry.instructions.len() > MAX_INSTRUCTIONS {
        return Err(FleetError::Invalid(format!(
            "memory.global.put: content too large ({} bytes, max {MAX_INSTRUCTIONS})",
            entry.instructions.len()
        )));
    }
    for (field, value) in [("instructions_by_provider", &entry.instructions_by_provider), ("context_files", &entry.context_files)] {
        if serde_json::from_str::<serde_json::Value>(value).is_err() {
            return Err(FleetError::Invalid(format!("memory.global.put: {field} is not valid JSON")));
        }
    }
    if !is_fleet_id(&entry.id) {
        return Ok(Outcome::Skipped(Skipped::Protected));
    }
    let local = store.bundle_get(&entry.id)?;
    if let Some(b) = &local {
        if b.is_system || b.is_blank {
            return Ok(Outcome::Skipped(Skipped::Protected));
        }
    }
    if store.bundle_scope_get(&entry.id)? != crate::backend::storage::bundle_scope::SCOPE_FLEET {
        return Ok(Outcome::Skipped(Skipped::MachineScope));
    }

    let incoming_hash = entry.hash();
    let (applied, mut bundle) = match local {
        None => (Applied::Created, new_bundle(entry)),
        Some(b) if b.is_global => {
            let local_hash = bundle_hash(&b);
            if local_hash == incoming_hash {
                return Ok(Outcome::Skipped(Skipped::Same));
            }
            if !wins(entry.updated_at, &incoming_hash, b.updated_at, &local_hash) {
                return Ok(Outcome::Skipped(Skipped::Older));
            }
            (Applied::Updated, b)
        }
        Some(b) if store.bundle_is_removed_global(&b.id)? => {
            if !wins(entry.updated_at, &incoming_hash, b.updated_at, REMOVED_HASH) {
                return Ok(Outcome::Skipped(Skipped::Older));
            }
            (Applied::Restored, b)
        }
        Some(_) => return Ok(Outcome::Skipped(Skipped::NotMemory)),
    };
    bundle.name = entry.name.trim().to_string();
    bundle.description = entry.description.clone();
    bundle.instructions = entry.instructions.clone();
    bundle.instructions_by_provider = entry.instructions_by_provider.clone();
    bundle.context_files = entry.context_files.clone();
    bundle.is_global = true;
    bundle.updated_at = entry.updated_at;
    store.bundle_upsert_with_version(&bundle, actor.label, actor.uid, FLEET_SOURCE, &source_detail(actor))?;
    if applied != Applied::Updated {
        append_to_order(store, &bundle.id);
    }
    Ok(Outcome::Applied(applied))
}

/// Apply a removal received from another machine.
pub fn apply_remove(store: &Store, removal: &FleetRemoval, actor: Actor<'_>) -> Result<Outcome, FleetError> {
    validate_id(&removal.id)?;
    if !is_fleet_id(&removal.id) {
        return Ok(Outcome::Skipped(Skipped::Protected));
    }
    let Some(mut bundle) = store.bundle_get(&removal.id)? else {
        return Ok(Outcome::Skipped(Skipped::Unknown));
    };
    if bundle.is_system || bundle.is_blank {
        return Ok(Outcome::Skipped(Skipped::Protected));
    }
    if store.bundle_scope_get(&removal.id)? != crate::backend::storage::bundle_scope::SCOPE_FLEET {
        return Ok(Outcome::Skipped(Skipped::MachineScope));
    }
    if !bundle.is_global {
        return Ok(Outcome::Skipped(if store.bundle_is_removed_global(&removal.id)? {
            Skipped::AlreadyRemoved
        } else {
            Skipped::NotMemory
        }));
    }
    if !wins(removal.removed_at, REMOVED_HASH, bundle.updated_at, &bundle_hash(&bundle)) {
        return Ok(Outcome::Skipped(Skipped::Older));
    }
    bundle.is_global = false;
    bundle.updated_at = removal.removed_at;
    store.bundle_upsert_with_version(&bundle, actor.label, actor.uid, REMOVE_SOURCE, &source_detail(actor))?;
    Ok(Outcome::Applied(Applied::Removed))
}

fn source_detail(actor: Actor<'_>) -> String {
    serde_json::json!({ "fleet_from": actor.label }).to_string()
}

fn new_bundle(entry: &FleetEntry) -> Bundle {
    Bundle {
        id: entry.id.clone(),
        name: entry.name.trim().to_string(),
        description: entry.description.clone(),
        is_blank: false,
        is_global: true,
        provider: String::new(),
        model: String::new(),
        instructions: entry.instructions.clone(),
        instructions_by_provider: entry.instructions_by_provider.clone(),
        context_files: entry.context_files.clone(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        is_system: false,
        created_at: agentmux_common::time::now_ms(),
        updated_at: entry.updated_at,
    }
}

/// Put a new or restored entry last in the Global Memory order, as Memory →
/// Global's "+ New section" does. A failed reorder is logged, not fatal.
fn append_to_order(store: &Store, id: &str) {
    let ids: Result<Vec<String>, StoreError> = store.bundle_list_global().map(|bundles| {
        bundles
            .into_iter()
            .filter(|b| !b.is_system && b.id != id)
            .map(|b| b.id)
            .chain(std::iter::once(id.to_string()))
            .collect()
    });
    match ids.and_then(|ids| store.bundle_reorder(&ids)) {
        Ok(_) => {}
        Err(e) => tracing::warn!(bundle_id = id, error = %e, "fleet memory: append-to-order failed (non-fatal)"),
    }
}

#[cfg(test)]
mod tests;
