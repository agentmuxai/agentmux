// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux's own record of Global Memory
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.6, phase M4).
//!
//! Global Memory lives in `db_bundles` (`is_global = 1`) in the channel's
//! identity store: per channel on an isolated channel, `shared/store.db`
//! otherwise. This record, in the global transcript store, is what it can
//! later follow agents from — into a new isolated channel's import step, and
//! to other machines (M5). `db_bundles` stays the cache every reader uses;
//! nothing here changes what an agent receives.
//!
//! Zone `global-memory:<scope>`, `<scope>` being `shared` for channels that
//! share their identity store and `channel:<id>` for an isolated one — the
//! same boundary isolation draws today, so an isolated test channel's edits
//! never reach the real record.
//! - `log.jsonl`: `entry` events (a version of an entry, by its stable id —
//!   the bundle id; a tombstone when it stops being Global Memory) and
//!   `order` events (the full id list).
//! - `blob/<sha>`: `{name, instructions}` per distinct version.
//! - `heads.json`: each entry's head, and the current order.
//!
//! **Write-through.** The store's Global Memory write methods tell an
//! observer after each change ([`attach`]); the observer only queues the
//! change, and a worker thread reads the committed row and records it —
//! never blocking a write. The record only appends when content differs, so
//! several srv processes on one store record each change once. The source is
//! the latest `db_bundle_versions` row's when that matches the content.
//!
//! **Older builds** write `db_bundles` without it; each start compares every
//! entry with the record and records what differs as `legacy-build` (as
//! `baseline` the first time).

use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::{GlobalMemoryChange, Store};

const LOG_FILE: &str = "log.jsonl";
const HEADS_FILE: &str = "heads.json";

/// This channel's Global Memory scope: `shared`, or `channel:<id>` when it
/// is isolated.
pub(crate) fn scope() -> String {
    if agentmux_common::isolated_auth_enabled() {
        format!("channel:{}", crate::backend::reactive::registry::local_channel_id())
    } else {
        "shared".to_string()
    }
}

fn zone(scope: &str) -> String {
    // A channel id is free text; keep the zone name to safe characters.
    if scope.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')) {
        format!("global-memory:{scope}")
    } else {
        format!("global-memory:h{}", &crate::backend::memory_record::sha256_hex(scope.as_bytes())[..32])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntryVersion {
    pub entry_id: String,
    pub version: String,
    pub parent: Option<String>,
    /// `content_hash(name, instructions)`; `None` for a tombstone.
    pub sha256: Option<String>,
    pub name: String,
    #[serde(default)]
    pub is_system: bool,
    pub source: String,
    #[serde(default)]
    pub source_detail: String,
    #[serde(default)]
    pub written_by: String,
    #[serde(default)]
    pub written_by_uid: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Entry(EntryVersion),
    Order { ids: Vec<String>, at_ms: i64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntryHead {
    pub version: String,
    pub sha256: Option<String>,
    pub name: String,
    #[serde(default)]
    pub is_system: bool,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Heads {
    #[serde(default)]
    pub entries: BTreeMap<String, EntryHead>,
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct Body {
    name: String,
    instructions: String,
}

fn read_heads(bytes: Option<Vec<u8>>) -> Result<Heads, StoreError> {
    match bytes {
        None => Ok(Heads::default()),
        Some(b) => serde_json::from_slice(&b)
            .map_err(|e| StoreError::Other(format!("global memory record: heads.json unreadable: {e}"))),
    }
}

fn line(event: &Event) -> Result<Vec<u8>, StoreError> {
    let mut l = serde_json::to_vec(event).map_err(|e| StoreError::Other(e.to_string()))?;
    l.push(b'\n');
    Ok(l)
}

pub(crate) fn heads(fs: &FileStore, scope: &str) -> Result<Heads, StoreError> {
    read_heads(fs.read_files_consistent(&zone(scope), &[HEADS_FILE])?.pop().flatten())
}

/// An entry's `(name, instructions)` at a version's hash.
pub(crate) fn body(fs: &FileStore, scope: &str, sha256: &str) -> Result<Option<(String, String)>, StoreError> {
    let got = fs.read_files_consistent(&zone(scope), &[&format!("blob/{sha256}")])?.pop().flatten();
    Ok(got.and_then(|b| serde_json::from_slice::<Body>(&b).ok()).map(|b| (b.name, b.instructions)))
}

/// Every version of an entry, oldest first.
pub(crate) fn history(fs: &FileStore, scope: &str, entry_id: &str) -> Result<Vec<EntryVersion>, StoreError> {
    let log = fs.read_files_consistent(&zone(scope), &[LOG_FILE])?.pop().flatten().unwrap_or_default();
    Ok(log
        .split(|b| *b == b'\n')
        .filter_map(|l| serde_json::from_slice::<Event>(l).ok())
        .filter_map(|e| match e {
            Event::Entry(v) if v.entry_id == entry_id => Some(v),
            _ => None,
        })
        .collect())
}

/// Who wrote what an entry now holds: the latest version row, when it holds
/// this content.
fn attribution(store: &Store, id: &str, sha: &str) -> (String, String, String, String) {
    let latest = store.bundle_version_list(id).ok().and_then(|v| v.into_iter().next());
    match latest {
        Some(v) if v.content_hash == sha => (v.source, v.source_detail, v.written_by, v.written_by_uid),
        _ => ("agentmux".into(), String::new(), String::new(), String::new()),
    }
}

/// Record entry `id` as the store now has it, if the record differs: a new
/// version while it is Global Memory, a tombstone once it isn't. `source`
/// overrides the attribution (the startup comparison). Returns whether
/// anything was recorded.
pub(crate) fn sync_entry(fs: &FileStore, store: &Store, scope: &str, id: &str, source: Option<&str>) -> Result<bool, StoreError> {
    let row = store.bundle_get(id)?.filter(|b| b.is_global);
    let wanted = row.as_ref().map(|b| (crate::backend::storage::bundle_versions::content_hash(&b.name, &b.instructions), b));
    let (src, detail, by, by_uid) = match (&wanted, source) {
        (_, Some(s)) => (s.to_string(), String::new(), String::new(), String::new()),
        (Some((sha, _)), None) => attribution(store, id, sha),
        (None, None) => ("agentmux".into(), "removed from Global Memory".into(), String::new(), String::new()),
    };
    fs.zone_txn(&zone(scope), |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        let head = heads.entries.get(id);
        let same = match (&wanted, head) {
            (Some((sha, b)), Some(h)) => h.sha256.as_deref() == Some(sha.as_str()) && h.is_system == b.is_system,
            (None, Some(h)) => h.sha256.is_none(),
            (None, None) => true,
            (Some(_), None) => false,
        };
        if same {
            return Ok(false);
        }
        let (sha, name, is_system) = match &wanted {
            Some((sha, b)) => {
                let blob = serde_json::to_vec(&Body { name: b.name.clone(), instructions: b.instructions.clone() })
                    .map_err(|e| StoreError::Other(e.to_string()))?;
                z.put_if_absent(&format!("blob/{sha}"), &blob)?;
                (Some(sha.clone()), b.name.clone(), b.is_system)
            }
            None => (None, head.map(|h| h.name.clone()).unwrap_or_default(), head.is_some_and(|h| h.is_system)),
        };
        let v = EntryVersion {
            entry_id: id.to_string(),
            version: format!("g_{}", uuid::Uuid::new_v4().simple()),
            parent: head.map(|h| h.version.clone()),
            sha256: sha.clone(),
            name: name.clone(),
            is_system,
            source: src.clone(),
            source_detail: detail.clone(),
            written_by: by.clone(),
            written_by_uid: by_uid.clone(),
            created_at_ms: agentmux_common::time::now_ms(),
        };
        z.append_lines(LOG_FILE, &line(&Event::Entry(v.clone()))?)?;
        let extra = head.map(|h| h.extra.clone()).unwrap_or_default();
        heads.entries.insert(id.to_string(), EntryHead { version: v.version, sha256: sha, name, is_system, extra });
        z.put(HEADS_FILE, &serde_json::to_vec(&heads).map_err(|e| StoreError::Other(e.to_string()))?)?;
        Ok(true)
    })
}

/// Record the Global Memory order if it differs from the record's.
pub(crate) fn sync_order(fs: &FileStore, store: &Store, scope: &str) -> Result<bool, StoreError> {
    let ids: Vec<String> = store.bundle_list_global()?.into_iter().map(|b| b.id).collect();
    fs.zone_txn(&zone(scope), |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        if heads.order == ids {
            return Ok(false);
        }
        z.append_lines(LOG_FILE, &line(&Event::Order { ids: ids.clone(), at_ms: agentmux_common::time::now_ms() })?)?;
        heads.order = ids.clone();
        z.put(HEADS_FILE, &serde_json::to_vec(&heads).map_err(|e| StoreError::Other(e.to_string()))?)?;
        Ok(true)
    })
}

/// At start: record every entry and the order as the store has them —
/// `baseline` the first time, `legacy-build` for what an older build (or a
/// change the worker never reached) left different. Returns how many
/// entries were recorded.
pub(crate) fn sync_all(fs: &FileStore, store: &Store, scope: &str) -> Result<usize, StoreError> {
    let first = heads(fs, scope)?.entries.is_empty();
    let source = if first { "baseline" } else { "legacy-build" };
    let mut ids: HashSet<String> = store.bundle_list_global()?.into_iter().map(|b| b.id).collect();
    ids.extend(heads(fs, scope)?.entries.into_iter().filter(|(_, h)| h.sha256.is_some()).map(|(id, _)| id));
    let mut n = 0;
    for id in ids {
        if sync_entry(fs, store, scope, &id, Some(source))? {
            n += 1;
        }
    }
    sync_order(fs, store, scope)?;
    Ok(n)
}

/// What the worker is asked to do.
enum Job {
    Change(GlobalMemoryChange),
    SyncAll,
}

fn process(fs: &FileStore, store: &Store, scope: &str, job: Job) {
    let r = match job {
        Job::Change(GlobalMemoryChange::Entry(id)) => sync_entry(fs, store, scope, &id, None).map(|_| ()),
        Job::Change(GlobalMemoryChange::Order) => sync_order(fs, store, scope).map(|_| ()),
        Job::SyncAll => sync_all(fs, store, scope).map(|n| {
            if n > 0 {
                tracing::info!(scope, recorded = n, "global memory record: start-up comparison recorded entries");
            }
        }),
    };
    if let Err(e) = r {
        tracing::warn!(scope, error = %e, "global memory record: not recorded; the next start compares again");
    }
}

/// Handle for queueing the start-up comparison once seeding is done.
pub(crate) struct Recorder {
    tx: mpsc::Sender<Job>,
}

impl Recorder {
    /// Queue the start-up comparison (after this start's own seeding, so
    /// seeded entries keep their seeder's attribution).
    pub(crate) fn sync_all(&self) {
        let _ = self.tx.send(Job::SyncAll);
    }
}

/// Start recording `store`'s Global Memory into `fs`: attach the observer
/// and start the worker thread.
pub(crate) fn attach(fs: Arc<FileStore>, store: Arc<Store>) -> Recorder {
    let (tx, rx) = mpsc::channel::<Job>();
    let scope = scope();
    let worker_store = store.clone();
    let spawned = std::thread::Builder::new().name("global-memory-record".into()).spawn(move || {
        while let Ok(job) = rx.recv() {
            process(&fs, &worker_store, &scope, job);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "global memory record: worker not started; nothing is recorded this run");
    }
    let observer_tx = tx.clone();
    store.set_global_memory_observer(Arc::new(move |change| {
        let _ = observer_tx.send(Job::Change(change));
    }));
    Recorder { tx }
}

#[cfg(test)]
mod tests;
