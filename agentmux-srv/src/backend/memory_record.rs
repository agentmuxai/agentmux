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
    pub(crate) fn is_tombstone(&self) -> bool {
        self.sha256.is_none()
    }
}

/// A file's head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Head {
    pub version: String,
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts_with: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Heads {
    /// Head per file name.
    pub files: BTreeMap<String, Head>,
    /// Per directory, per file: the version last projected there.
    pub projected: BTreeMap<String, BTreeMap<String, String>>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AppendOutcome {
    Appended(Version),
    /// The head already has this content with this parent: nothing written
    /// (N srvs capturing the same provider write record it once).
    Unchanged(String),
    /// The file's head isn't `expected_parent`; re-read and decide again.
    StaleParent { head: Option<String> },
}

fn read_heads(bytes: Option<Vec<u8>>) -> Heads {
    bytes.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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
    Ok(read_heads(got.pop().flatten()))
}

/// A body by hash.
pub(crate) fn body(fs: &FileStore, agent_uid: &str, sha256: &str) -> Result<Option<Vec<u8>>, StoreError> {
    let zone = zone_or_err(agent_uid)?;
    let mut got = fs.read_files_consistent(&zone, &[&format!("blob/{sha256}")])?;
    Ok(got.pop().flatten())
}

/// Append a version if the file's head is still `expected_parent`.
pub(crate) fn append_version(fs: &FileStore, agent_uid: &str, v: NewVersion<'_>) -> Result<AppendOutcome, StoreError> {
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
        let mut heads = read_heads(z.read(HEADS_FILE)?);
        let head = heads.files.get(v.file);
        let head_version = head.map(|h| h.version.as_str());
        if head_version != v.expected_parent {
            return Ok(AppendOutcome::StaleParent { head: head_version.map(str::to_string) });
        }
        if let Some(h) = head {
            if h.sha256 == sha && v.merged_parent.is_none() && v.conflicts_with.is_none() {
                return Ok(AppendOutcome::Unchanged(h.version.clone()));
            }
        } else if sha.is_none() {
            // Deleting a file the record never had: nothing to record.
            return Ok(AppendOutcome::Unchanged(String::new()));
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
        heads.files.insert(
            v.file.to_string(),
            Head { version: version.version.clone(), sha256: sha, conflicts_with: version.conflicts_with.clone() },
        );
        z.put(HEADS_FILE, &serde_json::to_vec(&heads).map_err(|e| StoreError::Other(e.to_string()))?)?;
        Ok(AppendOutcome::Appended(version))
    })
}

/// Record that `dir_id` now holds `version` of `file`. Appended only after
/// the file write it describes succeeded.
pub(crate) fn record_projected(
    fs: &FileStore,
    agent_uid: &str,
    dir_id: &str,
    file: &str,
    version: &str,
) -> Result<(), StoreError> {
    let zone = zone_or_err(agent_uid)?;
    fs.zone_txn(&zone, |z| {
        let mut heads = read_heads(z.read(HEADS_FILE)?);
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
        z.put(HEADS_FILE, &serde_json::to_vec(&heads).map_err(|e| StoreError::Other(e.to_string()))?)
    })
}

/// Every version of `file`, oldest first.
pub(crate) fn history(fs: &FileStore, agent_uid: &str, file: &str) -> Result<Vec<Version>, StoreError> {
    let zone = zone_or_err(agent_uid)?;
    let log = fs.read_files_consistent(&zone, &[LOG_FILE])?.pop().flatten().unwrap_or_default();
    Ok(log
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Event>(line).ok())
        .filter_map(|e| match e {
            Event::Version(v) if v.file == file => Some(v),
            _ => None,
        })
        .collect())
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
        assert_eq!(never, AppendOutcome::Unchanged(String::new()));
    }

    #[test]
    fn projections_are_kept_per_directory() {
        let fs = FileStore::open_in_memory().unwrap();
        let v1 = appended(append_version(&fs, UID, new("MEMORY.md", Some(b"one"), None)).unwrap());
        record_projected(&fs, UID, "/a/memory", "MEMORY.md", &v1.version).unwrap();
        record_projected(&fs, UID, "/a/memory", "MEMORY.md", &v1.version).unwrap(); // no-op
        let h = heads(&fs, UID).unwrap();
        assert_eq!(h.projected["/a/memory"]["MEMORY.md"], v1.version);
        assert!(!h.projected.contains_key("/b/memory"));
    }

    #[test]
    fn oversized_bodies_and_unsafe_uids_are_refused() {
        let fs = FileStore::open_in_memory().unwrap();
        let big = vec![b'x'; MAX_BODY_BYTES + 1];
        assert!(append_version(&fs, UID, new("big.md", Some(&big), None)).is_err());
        assert!(zone_for("../escape").is_none());
        assert!(heads(&fs, "../escape").is_err());
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
