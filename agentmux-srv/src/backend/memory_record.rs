// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux's own record of an agent's memory
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.1, phase M3).
//!
//! An agent's memory files follow its provider account and working directory
//! — `$CLAUDE_CONFIG_DIR/projects/<cwd>/memory` — so an account switch or a
//! new working directory used to leave them behind. The record lives in the
//! global transcript store, keyed by the agent's UID (`db_agents.id`), and is
//! the source of truth the provider folder is reconciled against.
//!
//! Zone `agent-uid:<uid>:memory`:
//! - `log.jsonl` — append-only events: a file version (a body by hash, or a
//!   tombstone) or a projection (which version a directory holds);
//! - `blob/<sha256>` — one file per distinct body, written before the log
//!   line that names it, in the same transaction;
//! - `heads.json` — per file, the head version; per directory, the version
//!   last projected there. Rewritten with every append, so current state is
//!   one small read and no tail window has to hold a rarely edited file.
//!
//! Every write is one `zone_txn` — a conditional append against the head the
//! caller expects — so srv processes sharing the store never fork a file's
//! chain. Nothing here touches the provider folder; reconciliation does
//! (phase M3b).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;

const LOG_FILE: &str = "log.jsonl";
const HEADS_FILE: &str = "heads.json";
/// Same cap as a memory file today (`app_api` memory write).
pub(crate) const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// `agent-uid:<uid>:memory`, or `None` for a UID outside the safe zone-name
/// character set.
pub(crate) fn zone_for(agent_uid: &str) -> Option<String> {
    crate::backend::agent_session::is_valid_definition_id(agent_uid).then(|| format!("agent-uid:{agent_uid}:memory"))
}

pub(crate) fn sha256_hex(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

/// One line of `log.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum Event {
    Version(Version),
    /// `dir_id` now holds `version` of `file`. Local to this machine; never
    /// synced.
    Projected { dir_id: String, file: String, version: String, at_ms: i64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Version {
    pub file: String,
    pub version: String,
    /// The version this one replaces; `None` for a file's first version.
    pub parent: Option<String>,
    /// A merge's second parent (spec §2.1.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_parent: Option<String>,
    /// The body's hash; `None` for a tombstone.
    pub sha256: Option<String>,
    /// Set when this version won a conflict against another head, kept in
    /// the record and written beside it as a `__conflict_` file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts_with: Option<String>,
    pub source: String,
    #[serde(default)]
    pub source_detail: String,
    pub created_at_ms: i64,
}

impl Version {
    #[cfg(test)]
    pub(crate) fn is_tombstone(&self) -> bool {
        self.sha256.is_none()
    }
}

/// A file's head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Head {
    pub version: String,
    /// The head's own parent, so a second capture of the same change from
    /// the same parent is recognised as already recorded.
    #[serde(default)]
    pub parent: Option<String>,
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts_with: Option<String>,
    /// Fields a newer build added, kept when this build rewrites the file.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Heads {
    /// Head per file name.
    #[serde(default)]
    pub files: BTreeMap<String, Head>,
    /// Per directory, per file: the version last projected there.
    #[serde(default)]
    pub projected: BTreeMap<String, BTreeMap<String, String>>,
    /// When the agent's per-channel version history was imported
    /// ([`import_history`]); `None` until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at_ms: Option<i64>,
    /// Per directory, per file: content found there at first sighting that
    /// another agent's record already holds — kept out of this record for
    /// the adoption list (spec §2.1.2), and left alone while the file
    /// still holds it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub held: BTreeMap<String, BTreeMap<String, String>>,
    /// Fields a newer build added, kept when this build rewrites the file.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Heads {
    /// The files that currently exist (not tombstoned).
    pub(crate) fn live(&self) -> impl Iterator<Item = (&String, &Head)> {
        self.files.iter().filter(|(_, h)| h.sha256.is_some())
    }
}

/// A version to append.
pub(crate) struct NewVersion<'a> {
    pub file: &'a str,
    /// The new body; `None` records a deletion.
    pub body: Option<&'a [u8]>,
    /// The head the caller based this on (`None`: the file has no versions
    /// yet). A different current head refuses the append.
    pub expected_parent: Option<&'a str>,
    pub merged_parent: Option<&'a str>,
    pub conflicts_with: Option<&'a str>,
    pub source: &'a str,
    pub source_detail: &'a str,
    /// Also record, in the same transaction, that this directory now holds
    /// the new version (a capture from it, or a write through AgentMux to it).
    pub project_to: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AppendOutcome {
    Appended(Version),
    /// This change is already recorded — the head has this content, either
    /// as `expected_parent` itself or as its child (N srvs capturing the same
    /// provider write record it once). Carries the head's version.
    Unchanged(String),
    /// Deleting a file the record never had: nothing to record.
    NothingToRecord,
    /// The file's head isn't `expected_parent`; re-read and decide again.
    StaleParent { head: Option<String> },
}

/// A memory file name: one path component of safe characters.
pub(crate) fn validate_file(file: &str) -> Result<(), StoreError> {
    let ok = !file.is_empty()
        && file.len() <= 255
        && file != "."
        && file != ".."
        && file.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(StoreError::Other(format!("memory record: invalid file name {file:?}")))
    }
}

fn validate_sha(sha256: &str) -> Result<(), StoreError> {
    if sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(StoreError::Other(format!("memory record: invalid hash {sha256:?}")))
    }
}

/// `heads.json`, or empty when the record has none yet. A file that exists
/// but doesn't parse is an error — never silently empty, or the next write
/// would replace every head and projection with just its own.
fn read_heads(bytes: Option<Vec<u8>>) -> Result<Heads, StoreError> {
    match bytes {
        None => Ok(Heads::default()),
        Some(b) => serde_json::from_slice(&b)
            .map_err(|e| StoreError::Other(format!("memory record: heads.json unreadable: {e}"))),
    }
}

fn log_line(event: &Event) -> Result<Vec<u8>, StoreError> {
    let mut line = serde_json::to_vec(event).map_err(|e| StoreError::Other(format!("memory record: encode: {e}")))?;
    line.push(b'\n');
    Ok(line)
}

fn zone_or_err(agent_uid: &str) -> Result<String, StoreError> {
    zone_for(agent_uid).ok_or_else(|| StoreError::Other(format!("memory record: invalid agent uid {agent_uid:?}")))
}

/// The agent's current heads, read as one snapshot sized from the database.
pub(crate) fn heads(fs: &FileStore, agent_uid: &str) -> Result<Heads, StoreError> {
    let zone = zone_or_err(agent_uid)?;
    let mut got = fs.read_files_consistent(&zone, &[HEADS_FILE])?;
    read_heads(got.pop().flatten())
}

/// A body by hash.
pub(crate) fn body(fs: &FileStore, agent_uid: &str, sha256: &str) -> Result<Option<Vec<u8>>, StoreError> {
    validate_sha(sha256)?;
    let zone = zone_or_err(agent_uid)?;
    let mut got = fs.read_files_consistent(&zone, &[&format!("blob/{sha256}")])?;
    Ok(got.pop().flatten())
}

/// Append a version if the file's head is still `expected_parent`.
pub(crate) fn append_version(fs: &FileStore, agent_uid: &str, v: NewVersion<'_>) -> Result<AppendOutcome, StoreError> {
    validate_file(v.file)?;
    if let Some(b) = v.body {
        if b.len() > MAX_BODY_BYTES {
            return Err(StoreError::Other(format!(
                "memory record: {} is {} bytes (max {MAX_BODY_BYTES})",
                v.file,
                b.len()
            )));
        }
    }
    let zone = zone_or_err(agent_uid)?;
    let sha = v.body.map(sha256_hex);
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        let plain = v.merged_parent.is_none() && v.conflicts_with.is_none();
        let head = heads.files.get(v.file);
        let head_version = head.map(|h| h.version.as_str());
        if let Some(h) = head {
            // Already recorded: the head IS this content on this parent, or
            // is its child from the same parent (another srv captured the
            // same change first).
            let same_change = h.sha256 == sha
                && (head_version == v.expected_parent || h.parent.as_deref() == v.expected_parent);
            if plain && same_change {
                let version = h.version.clone();
                if let Some(dir) = v.project_to {
                    set_projected(z, &mut heads, dir, v.file, &version)?;
                }
                return Ok(AppendOutcome::Unchanged(version));
            }
        } else if sha.is_none() {
            return Ok(AppendOutcome::NothingToRecord);
        }
        if head_version != v.expected_parent {
            return Ok(AppendOutcome::StaleParent { head: head_version.map(str::to_string) });
        }
        if let (Some(sha), Some(body)) = (&sha, v.body) {
            z.put_if_absent(&format!("blob/{sha}"), body)?;
        }
        let version = Version {
            file: v.file.to_string(),
            version: format!("v_{}", uuid::Uuid::new_v4().simple()),
            parent: v.expected_parent.map(str::to_string),
            merged_parent: v.merged_parent.map(str::to_string),
            sha256: sha.clone(),
            conflicts_with: v.conflicts_with.map(str::to_string),
            source: v.source.to_string(),
            source_detail: v.source_detail.to_string(),
            created_at_ms: agentmux_common::time::now_ms(),
        };
        z.append_lines(LOG_FILE, &log_line(&Event::Version(version.clone()))?)?;
        let extra = heads.files.get(v.file).map(|h| h.extra.clone()).unwrap_or_default();
        heads.files.insert(
            v.file.to_string(),
            Head {
                version: version.version.clone(),
                parent: version.parent.clone(),
                sha256: sha,
                conflicts_with: version.conflicts_with.clone(),
                extra,
            },
        );
        if let Some(dir) = v.project_to {
            set_projected(z, &mut heads, dir, v.file, &version.version)?;
        }
        write_heads(z, &heads)?;
        Ok(AppendOutcome::Appended(version))
    })
}

fn write_heads(z: &mut crate::backend::storage::filestore::ZoneTxn<'_>, heads: &Heads) -> Result<(), StoreError> {
    z.put(HEADS_FILE, &serde_json::to_vec(heads).map_err(|e| StoreError::Other(e.to_string()))?)
}

/// Point `dir_id`'s projection of `file` at `version` and log it (the caller
/// writes `heads.json`, or it is written here when nothing else changes).
fn set_projected(
    z: &mut crate::backend::storage::filestore::ZoneTxn<'_>,
    heads: &mut Heads,
    dir_id: &str,
    file: &str,
    version: &str,
) -> Result<(), StoreError> {
    let per_dir = heads.projected.entry(dir_id.to_string()).or_default();
    if per_dir.get(file).map(String::as_str) == Some(version) {
        return Ok(());
    }
    per_dir.insert(file.to_string(), version.to_string());
    let event = Event::Projected {
        dir_id: dir_id.to_string(),
        file: file.to_string(),
        version: version.to_string(),
        at_ms: agentmux_common::time::now_ms(),
    };
    z.append_lines(LOG_FILE, &log_line(&event)?)?;
    write_heads(z, heads)
}

/// Record that `dir_id` now holds `version` of `file`, if its projection is
/// still `expected_previous` (`None`: never projected there). Appended only
/// after the file write it describes succeeded. Returns `false`, writing
/// nothing, when another srv projected meanwhile — the caller re-reads.
pub(crate) fn record_projected(
    fs: &FileStore,
    agent_uid: &str,
    dir_id: &str,
    file: &str,
    version: &str,
    expected_previous: Option<&str>,
) -> Result<bool, StoreError> {
    validate_file(file)?;
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        let current = heads.projected.get(dir_id).and_then(|m| m.get(file)).map(String::as_str);
        if current == Some(version) {
            return Ok(true);
        }
        if current != expected_previous {
            return Ok(false);
        }
        set_projected(z, &mut heads, dir_id, file, version)?;
        Ok(true)
    })
}

/// Forget every projection into `dir_id`: the folder no longer holds what
/// was written there (it was removed or emptied), so each file must read as
/// never projected — to be written back — not as deleted.
pub(crate) fn reset_projections(fs: &FileStore, agent_uid: &str, dir_id: &str) -> Result<(), StoreError> {
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        if heads.projected.remove(dir_id).is_some() {
            write_heads(z, &heads)?;
        }
        Ok(())
    })
}

/// Which of `shas` another agent's record holds (a blob in its zone).
pub(crate) fn held_by_other_records(
    fs: &FileStore,
    agent_uid: &str,
    shas: &[String],
) -> Result<std::collections::BTreeSet<String>, StoreError> {
    let own = zone_or_err(agent_uid)?;
    let mut found = std::collections::BTreeSet::new();
    if shas.is_empty() {
        return Ok(found);
    }
    for zone in fs.get_all_zone_ids()? {
        if zone == own || !(zone.starts_with("agent-uid:") && zone.ends_with(":memory")) {
            continue;
        }
        for sha in shas {
            if !found.contains(sha) && fs.stat(&zone, &format!("blob/{sha}"))?.is_some() {
                found.insert(sha.clone());
            }
        }
    }
    Ok(found)
}

/// Keep `files` (name, content hash) found in `dir_id` out of the record,
/// for the adoption list. One transaction.
pub(crate) fn hold_for_adoption(fs: &FileStore, agent_uid: &str, dir_id: &str, files: &[(String, String)]) -> Result<(), StoreError> {
    if files.is_empty() {
        return Ok(());
    }
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        let per_dir = heads.held.entry(dir_id.to_string()).or_default();
        for (file, sha) in files {
            per_dir.insert(file.clone(), sha.clone());
        }
        write_heads(z, &heads)
    })
}

/// Stop holding `files` in `dir_id` — a human adopted them (§2.1.4).
pub(crate) fn release_held(fs: &FileStore, agent_uid: &str, dir_id: &str, files: &[String]) -> Result<(), StoreError> {
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        let Some(per_dir) = heads.held.get_mut(dir_id) else { return Ok(()) };
        for f in files {
            per_dir.remove(f);
        }
        if per_dir.is_empty() {
            heads.held.remove(dir_id);
        }
        write_heads(z, &heads)
    })
}

/// A version from the per-channel history tables, to import.
pub(crate) struct ImportedVersion {
    pub file: String,
    pub body: Vec<u8>,
    pub source: String,
    pub source_detail: String,
    pub created_at_ms: i64,
}

/// Import the agent's earlier history, once: every version goes into the
/// log, oldest first and chained per file, keeping its source and time.
/// History only — no head changes, so a file deleted long ago is never
/// written back; heads still come from the folder. A version whose content
/// the record already has for that file is skipped. Returns how many were
/// imported; `None` if the record already imported its history.
pub(crate) fn import_history(fs: &FileStore, agent_uid: &str, rows: &[ImportedVersion]) -> Result<Option<usize>, StoreError> {
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?)?;
        if heads.imported_at_ms.is_some() {
            return Ok(None);
        }
        let count = log_history(z, rows)?;
        heads.imported_at_ms = Some(agentmux_common::time::now_ms());
        write_heads(z, &heads)?;
        Ok(Some(count))
    })
}

/// Add `rows` to the agent's history — no head changes, content the record
/// already has for that file skipped. For versions kept beside a head the
/// caller doesn't replace (adoption of earlier accounts' memory, §2.1.4).
/// Returns how many were added.
pub(crate) fn add_history(fs: &FileStore, agent_uid: &str, rows: &[ImportedVersion]) -> Result<usize, StoreError> {
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| log_history(z, rows))
}

/// Log `rows` as history-only versions, oldest first and chained per file,
/// skipping content the log already has for that file.
fn log_history(z: &mut crate::backend::storage::filestore::ZoneTxn<'_>, rows: &[ImportedVersion]) -> Result<usize, StoreError> {
    let log = z.read(LOG_FILE)?.unwrap_or_default();
    let mut have: std::collections::BTreeSet<(String, String)> = log
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Event>(line).ok())
        .filter_map(|e| match e {
            Event::Version(v) => v.sha256.map(|sha| (v.file, sha)),
            _ => None,
        })
        .collect();
    let mut sorted: Vec<&ImportedVersion> = rows.iter().collect();
    sorted.sort_by_key(|r| r.created_at_ms);
    let mut last: BTreeMap<&str, String> = BTreeMap::new();
    let mut lines = Vec::new();
    for r in sorted {
        if validate_file(&r.file).is_err() || r.body.len() > MAX_BODY_BYTES {
            continue;
        }
        let sha = sha256_hex(&r.body);
        if !have.insert((r.file.clone(), sha.clone())) {
            continue;
        }
        z.put_if_absent(&format!("blob/{sha}"), &r.body)?;
        let version = Version {
            file: r.file.clone(),
            version: format!("v_{}", uuid::Uuid::new_v4().simple()),
            parent: last.get(r.file.as_str()).cloned(),
            merged_parent: None,
            sha256: Some(sha),
            conflicts_with: None,
            source: r.source.clone(),
            source_detail: r.source_detail.clone(),
            created_at_ms: r.created_at_ms,
        };
        last.insert(&r.file, version.version.clone());
        lines.extend(log_line(&Event::Version(version))?);
    }
    let count = lines.iter().filter(|b| **b == b'\n').count();
    if !lines.is_empty() {
        z.append_lines(LOG_FILE, &lines)?;
    }
    Ok(count)
}

/// Every version of `file`, oldest first — by time, since imported history
/// is logged after the versions it predates.
pub(crate) fn history(fs: &FileStore, agent_uid: &str, file: &str) -> Result<Vec<Version>, StoreError> {
    let zone = zone_or_err(agent_uid)?;
    let log = fs.read_files_consistent(&zone, &[LOG_FILE])?.pop().flatten().unwrap_or_default();
    let mut versions: Vec<Version> = log
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Event>(line).ok())
        .filter_map(|e| match e {
            Event::Version(v) if v.file == file => Some(v),
            _ => None,
        })
        .collect();
    versions.sort_by_key(|v| v.created_at_ms);
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: &str = "93d0dfdd-0000-4000-8000-000000000001";

    fn new<'a>(file: &'a str, body: Option<&'a [u8]>, parent: Option<&'a str>) -> NewVersion<'a> {
        NewVersion {
            file,
            body,
            expected_parent: parent,
            merged_parent: None,
            conflicts_with: None,
            source: "agent",
            source_detail: "",
            project_to: None,
        }
    }

    fn appended(o: AppendOutcome) -> Version {
        match o {
            AppendOutcome::Appended(v) => v,
            other => panic!("expected an append, got {other:?}"),
        }
    }

    #[test]
    fn versions_chain_and_heads_follow() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        let v2 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"two"), Some(&v1.version))).unwrap());
        assert_eq!(v2.parent.as_deref(), Some(v1.version.as_str()));

        let h = heads(&fs, UID).unwrap();
        assert_eq!(h.files["MEMORY.md"].version, v2.version);
        let sha = h.files["MEMORY.md"].sha256.clone().unwrap();
        assert_eq!(body(&fs, UID, &sha).unwrap().as_deref(), Some(&b"two"[..]));
        let hist = history(&fs, UID, "MEMORY.md").unwrap();
        assert_eq!(hist.iter().map(|v| v.version.as_str()).collect::<Vec<_>>(), [v1.version.as_str(), v2.version.as_str()]);
    }

    #[test]
    fn a_stale_parent_is_refused_and_nothing_is_written() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        let _v2 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"two"), Some(&v1.version))).unwrap());

        let stale = append_version(&fs, UID, new("MEMORY.md", Some(b"three"), Some(&v1.version))).unwrap();
        assert!(matches!(stale, AppendOutcome::StaleParent { head: Some(_) }));
        assert_eq!(history(&fs, UID, "MEMORY.md").unwrap().len(), 2);
        assert!(body(&fs, UID, &sha256_hex(b"three")).unwrap().is_none(), "no blob for a refused append");
    }

    #[test]
    fn the_same_content_on_the_same_parent_is_recorded_once() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        let again = append_version(&fs, UID, new("MEMORY.md", Some(b"one"), Some(&v1.version))).unwrap();
        assert_eq!(again, AppendOutcome::Unchanged(v1.version.clone()));
        assert_eq!(history(&fs, UID, "MEMORY.md").unwrap().len(), 1);
    }

    #[test]
    fn a_tombstone_removes_the_file_from_the_live_set() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("topic.md", Some(b"x"), None)).unwrap());
        let t = appended(append_version(&fs, UID, new("topic.md", None, Some(&v1.version))).unwrap());
        assert!(t.is_tombstone());
        assert_eq!(heads(&fs, UID).unwrap().live().count(), 0);
        let never = append_version(&fs, UID, new("never.md", None, None)).unwrap();
        assert_eq!(never, AppendOutcome::NothingToRecord);
    }

    #[test]
    fn projections_are_kept_per_directory() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        assert!(record_projected(&fs, UID, "/a/memory", "MEMORY.md", &v1.version, None).unwrap());
        assert!(record_projected(&fs, UID, "/a/memory", "MEMORY.md", &v1.version, None).unwrap()); // already so
        let v2 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"two"), Some(&v1.version))).unwrap());
        // Another srv projected meanwhile: a stale expectation writes nothing.
        assert!(!record_projected(&fs, UID, "/a/memory", "MEMORY.md", &v2.version, None).unwrap());
        assert!(record_projected(&fs, UID, "/a/memory", "MEMORY.md", &v2.version, Some(&v1.version)).unwrap());
        let h = heads(&fs, UID).unwrap();
        assert_eq!(h.projected["/a/memory"]["MEMORY.md"], v2.version);
        assert!(!h.projected.contains_key("/b/memory"));
    }

    #[test]
    fn oversized_bodies_and_unsafe_uids_are_refused() {
        let fs = FileStore::open_in_memory().unwrap();
        let big = vec![b'x'; MAX_BODY_BYTES + 1];
        assert!(append_version(&fs, UID, new("big.md", Some(&big), None)).is_err());
        assert!(zone_for("../escape").is_none());
        assert!(heads(&fs, "../escape").is_err());
        for bad in ["", ".", "..", "../x.md", "a/b.md", "a\\b.md"] {
            assert!(append_version(&fs, UID, new(bad, Some(b"x"), None)).is_err(), "{bad:?}");
        }
        assert!(body(&fs, UID, "../../etc").is_err());
    }

    /// Two srvs capture the same provider write from the same parent: the
    /// second finds it already recorded, not a conflict.
    #[test]
    fn a_second_capture_of_the_same_change_is_already_recorded() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        let first = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"two"), Some(&v1.version))).unwrap());
        let second = append_version(&fs, UID, new("MEMORY.md", Some(b"two"), Some(&v1.version))).unwrap();
        assert_eq!(second, AppendOutcome::Unchanged(first.version.clone()));
        // A different change from the same parent is still stale.
        let other = append_version(&fs, UID, new("MEMORY.md", Some(b"three"), Some(&v1.version))).unwrap();
        assert!(matches!(other, AppendOutcome::StaleParent { .. }));
    }

    #[test]
    fn a_version_and_its_projection_commit_together() {
        let fs = FileStore::open_in_memory().unwrap();
        let mut nv = new("MEMORY.md", Some(b"one"), None);
        nv.project_to = Some("/a/memory");
        let v1 = appended(append_version(&fs, UID, nv).unwrap());
        assert_eq!(heads(&fs, UID).unwrap().projected["/a/memory"]["MEMORY.md"], v1.version);
    }

    /// An unreadable heads.json is an error, never silently empty — the next
    /// write would otherwise replace every head with its own.
    #[test]
    fn an_unreadable_heads_file_is_an_error_not_empty() {
        let fs = FileStore::open_in_memory().unwrap();
        let zone = zone_for(UID).unwrap();
        appended(append_version(&fs, UID, new("a.md", Some(b"a"), None)).unwrap());
        fs.zone_txn(&zone, |z| z.put(HEADS_FILE, b"{not json")).unwrap();
        assert!(heads(&fs, UID).is_err());
        assert!(append_version(&fs, UID, new("b.md", Some(b"b"), None)).is_err());
    }

    /// Fields a newer build added survive this build's rewrite.
    #[test]
    fn unknown_heads_fields_survive_a_rewrite() {
        let fs = FileStore::open_in_memory().unwrap();
        let zone = zone_for(UID).unwrap();
        let v1 = appended(append_version(&fs, UID, new("a.md", Some(b"a"), None)).unwrap());
        let mut h: serde_json::Value = serde_json::to_value(heads(&fs, UID).unwrap()).unwrap();
        h["future"] = serde_json::json!(1);
        h["files"]["a.md"]["future_file_field"] = serde_json::json!("x");
        fs.zone_txn(&zone, |z| z.put(HEADS_FILE, &serde_json::to_vec(&h).unwrap())).unwrap();
        appended(append_version(&fs, UID, new("a.md", Some(b"a2"), Some(&v1.version))).unwrap());
        let after: serde_json::Value = serde_json::to_value(heads(&fs, UID).unwrap()).unwrap();
        assert_eq!(after["future"], 1);
        assert_eq!(after["files"]["a.md"]["future_file_field"], "x");
    }

    /// Two handles on one database stand in for two srv processes appending
    /// to the same file from the same parent: exactly one wins.
    #[test]
    fn two_processes_from_the_same_parent_never_fork_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filestore.db");
        let a = FileStore::open(&path).unwrap();
        let b = FileStore::open(&path).unwrap();
        let v1 = appended(append_version(&a, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        let from_a = append_version(&a, UID, new("MEMORY.md", Some(b"from a"), Some(&v1.version))).unwrap();
        let from_b = append_version(&b, UID, new("MEMORY.md", Some(b"from b"), Some(&v1.version))).unwrap();
        assert!(matches!(from_a, AppendOutcome::Appended(_)));
        assert!(matches!(from_b, AppendOutcome::StaleParent { .. }));
        assert_eq!(history(&b, UID, "MEMORY.md").unwrap().len(), 2);
    }
}
