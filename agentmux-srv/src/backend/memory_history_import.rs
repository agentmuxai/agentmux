// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one-time import of an agent's earlier memory history into its record
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1, phase M3b).
//!
//! Before the record, each channel kept its own history of an agent's memory
//! files (`db_agent_native_memory_versions`, in that channel's identity
//! store), so an agent's past was split across every channel it ran in. At
//! the agent's first reconcile that proves its folder exclusive, the rows
//! from every channel store on this machine are imported into the record,
//! deduplicated by content.
//!
//! Rows from `agent_inferred` (the m0024 backfill) and `external_fs_write`
//! (drift) were attributed to agents by a registry lookup that could name
//! the wrong agent, so they are **unproven**: imported only when the file
//! of that name in the agent's own proven folder holds that exact content.
//! The rest wait for the adoption list (M3c).
//!
//! History only: nothing here changes a head or touches the folder.

use std::path::PathBuf;
use std::time::Instant;

use crate::backend::memory_record::{self as record, ImportedVersion};
use crate::backend::storage::agent_native_memory_versions::NativeMemoryVersion;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// Sources written by a path that knew which agent it was writing for.
const PROVEN_SOURCES: &[&str] = &["agent", "armory-ui", "human", "revert", "jekt"];

/// Every store on this machine that may hold per-channel memory history:
/// each channel's and dev instance's identity store, and the stable
/// channel's (the shared store).
fn history_store_paths() -> Vec<PathBuf> {
    let Some(shared) = crate::registry::resolve_global_shared_root() else {
        return Vec::new();
    };
    history_store_paths_under(&shared)
}

fn history_store_paths_under(shared: &std::path::Path) -> Vec<PathBuf> {
    let mut paths = vec![shared.join("store.db")];
    if let Some(home) = shared.parent() {
        let children = |dir: PathBuf| -> Vec<PathBuf> {
            std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
        };
        for channel in children(home.join("channels")) {
            paths.push(channel.join("identity-store.db"));
        }
        // A dev instance's store sits at `dev/<branch>/<clone>/`, or at
        // `dev/<branch>/` itself in the older layout without a clone id.
        for branch in children(home.join("dev")) {
            paths.push(branch.join("identity-store.db"));
            for instance in children(branch) {
                paths.push(instance.join("identity-store.db"));
            }
        }
    }
    paths.retain(|p| p.is_file());
    paths
}

/// `uid`'s rows from every history store, or `None` if `deadline` passed
/// before they were all read (the import then waits for a later pass). A
/// store that can't be opened or read is skipped.
fn gather(uid: &str, stores: &[PathBuf], deadline: Instant) -> Option<Vec<NativeMemoryVersion>> {
    let mut rows = Vec::new();
    for path in stores {
        if Instant::now() > deadline {
            return None;
        }
        let Ok(store) = Store::open_read_only(path) else { continue };
        match store.agent_native_memory_versions_for_agent(uid) {
            Ok(r) => rows.extend(r),
            Err(e) => tracing::debug!(store = %path.display(), error = %e, "memory import: history unreadable; skipped"),
        }
    }
    Some(rows)
}

/// The rows to import: proven ones, and unproven ones whose content is what
/// the agent's own folder holds under that name (`disk_sha`).
fn select(rows: Vec<NativeMemoryVersion>, disk_sha: impl Fn(&str) -> Option<String>) -> Vec<ImportedVersion> {
    rows.into_iter()
        .filter(|r| crate::server::native_memory_handlers::validate_filename(&r.filename).is_ok())
        .filter(|r| {
            PROVEN_SOURCES.contains(&r.source.as_str())
                || disk_sha(&r.filename).as_deref() == Some(record::sha256_hex(r.content.as_bytes()).as_str())
        })
        .map(|r| ImportedVersion {
            file: r.filename,
            body: r.content.into_bytes(),
            source_detail: if r.source_detail.is_empty() {
                format!("imported {}", r.id)
            } else {
                format!("imported {}; {}", r.id, r.source_detail)
            },
            source: r.source,
            created_at_ms: r.created_at,
        })
        .collect()
}

/// Import `uid`'s earlier history, once, from `stores` (every history store
/// on the machine when `None`). `disk_sha` gives the content hash of a file
/// in the agent's proven folder. Returns how many versions were imported,
/// or `None` when there was nothing to do (already imported, or out of
/// time — retried next pass).
pub(crate) fn import_once(
    fs: &FileStore,
    uid: &str,
    stores: Option<&[PathBuf]>,
    disk_sha: impl Fn(&str) -> Option<String>,
    deadline: Instant,
) -> Result<Option<usize>, StoreError> {
    if record::heads(fs, uid)?.imported_at_ms.is_some() {
        return Ok(None);
    }
    let found;
    let stores = match stores {
        Some(s) => s,
        None => {
            found = history_store_paths();
            &found
        }
    };
    let Some(rows) = gather(uid, stores, deadline) else {
        return Ok(None);
    };
    record::import_history(fs, uid, &select(rows, disk_sha))
}

#[cfg(test)]
mod tests;
