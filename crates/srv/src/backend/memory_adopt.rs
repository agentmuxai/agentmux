// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Adopting an agent's memory from its earlier accounts
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.4, phase M3c).
//!
//! An agent's memory folder is keyed by account, so every account it ever
//! ran under left a folder behind. The agent's own live folder is adopted
//! automatically at first sighting (§2.1.2); every other folder is only
//! **offered**:
//!
//! - [`list`] enumerates the candidates — the same project folder under
//!   every account on this machine, minus the agent's own and any folder
//!   another agent uses — and issues the list a `list_id`. Paths never
//!   leave the server.
//! - [`adopt`] takes `(list_id, index, dir hash)` choices only. It is
//!   reached through the host's confirmation window, never an agent-held
//!   credential (the service gates it on the host secret). A folder that
//!   changed since the list was issued is refused.
//!
//! Merging never replaces the agent's current memory: a file the record
//! already has keeps its head, the adopted bodies are kept as its history;
//! a file it doesn't have is adopted, newest by modification time as the
//! head. The MEMORY.md indexes are unioned line by line, deduplicated by
//! link target, so no adopted topic becomes invisible to Claude. The next
//! spawn writes the result into the agent's folder.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::backend::memory_dir_claims as claims;
use crate::backend::memory_record::{self as record, AppendOutcome, ImportedVersion, NewVersion};
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

const INDEX_FILE: &str = "MEMORY.md";
/// How long an issued list stays valid.
const LIST_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NativeMemoryAdoptionFile")]
pub(crate) struct CandidateFile {
    pub name: String,
    #[ts(type = "number")]
    pub size: u64,
    #[ts(type = "number")]
    pub modified_ms: i64,
    pub sha256: String,
}

/// One folder offered for adoption. No path: the client refers to it by
/// `index` and `dir_hash`.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NativeMemoryAdoptionCandidate")]
pub(crate) struct Candidate {
    pub index: usize,
    /// The account the folder belongs to (its identity folder), `default`
    /// for the unlinked default home, or `held` for files in the agent's own
    /// folder held at first sighting because another agent's record has
    /// the same content (§2.1.2).
    pub account: String,
    /// Changes whenever a file in the folder is added, removed or changed.
    pub dir_hash: String,
    pub files: Vec<CandidateFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NativeMemoryAdoptionList")]
pub(crate) struct AdoptionList {
    pub list_id: String,
    pub agent_uid: String,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Default, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NativeMemoryAdoptReport")]
pub(crate) struct AdoptReport {
    /// Files the record didn't have, now adopted.
    pub files_added: usize,
    /// Other bodies kept as history.
    pub versions_kept: usize,
    /// Lines added to MEMORY.md.
    pub index_lines_added: usize,
}

#[derive(Debug, PartialEq)]
pub(crate) enum AdoptError {
    /// Unknown or expired list, or one issued for another agent.
    UnknownList,
    BadChoice,
    /// A chosen folder changed (or became another agent's) since the list
    /// was issued; the caller shows a fresh list.
    Changed { index: usize },
    /// A reconcile or capture for the agent is running; try again.
    Busy,
    Store(String),
}

impl From<StoreError> for AdoptError {
    fn from(e: StoreError) -> Self {
        AdoptError::Store(e.to_string())
    }
}

/// One listed folder, as the server remembers it.
struct IssuedDir {
    path: PathBuf,
    dir_hash: String,
    account: String,
    /// Only these files (the held ones in the agent's own folder).
    only: Option<Vec<String>>,
}

struct Issued {
    agent_uid: String,
    dirs: Vec<IssuedDir>,
    at: Instant,
}

fn issued() -> &'static Mutex<HashMap<String, Issued>> {
    static ISSUED: std::sync::OnceLock<Mutex<HashMap<String, Issued>>> = std::sync::OnceLock::new();
    ISSUED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Every account's projects folder on this machine: each linked account's
/// (`identities/<acct>/claude/projects`), and the unlinked default home's.
fn account_project_roots(shared: &Path) -> Vec<(String, PathBuf)> {
    let mut roots: Vec<(String, PathBuf)> = std::fs::read_dir(shared.join("identities"))
        .map(|d| {
            d.flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().join("claude").join("projects")))
                .collect()
        })
        .unwrap_or_default();
    roots.sort();
    roots.push(("default".into(), shared.join("providers").join("claude").join("projects")));
    roots
}

/// A folder's memory files, sorted by name — only `only`'s, when given.
/// Unreadable and oversized files are left out.
fn folder_files(dir: &Path, only: Option<&[String]>) -> Vec<(CandidateFile, Vec<u8>)> {
    let mut out: Vec<(CandidateFile, Vec<u8>)> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    crate::server::native_memory_handlers::validate_filename(&name).ok()?;
                    if only.is_some_and(|o| !o.contains(&name)) {
                        return None;
                    }
                    let meta = std::fs::metadata(e.path()).ok().filter(|m| m.is_file())?;
                    if meta.len() as usize > record::MAX_BODY_BYTES {
                        return None;
                    }
                    let body = std::fs::read(e.path()).ok()?;
                    let modified_ms = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_millis() as i64);
                    let file = CandidateFile { name, size: meta.len(), modified_ms, sha256: record::sha256_hex(&body) };
                    Some((file, body))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    out
}

fn dir_hash(files: &[(CandidateFile, Vec<u8>)]) -> String {
    let listing: String = files.iter().map(|(f, _)| format!("{}\t{}\n", f.name, f.sha256)).collect();
    record::sha256_hex(listing.as_bytes())
}

/// The candidates for `agent_uid`, whose own live folder is `own_dir`, among
/// the accounts under `shared`.
pub(crate) fn list_in(fs: &FileStore, mstore: &Store, agent_uid: &str, own_dir: &Path, shared: &Path) -> Result<AdoptionList, StoreError> {
    // `<config>/projects/<name>/memory`: the same `<name>` under every account.
    let name = own_dir.parent().and_then(Path::file_name).map(|n| n.to_os_string());
    let own_id = claims::dir_id(own_dir);
    let mut dirs = Vec::new();
    let mut candidates = Vec::new();
    if let Some(name) = name {
        for (account, root) in account_project_roots(shared) {
            let dir = root.join(&name).join("memory");
            if !dir.is_dir() || claims::dir_id(&dir) == own_id || claims::used_by_another(fs, mstore, agent_uid, &dir)? {
                continue;
            }
            let files = folder_files(&dir, None);
            if files.is_empty() {
                continue;
            }
            let hash = dir_hash(&files);
            candidates.push(Candidate {
                index: candidates.len(),
                account: account.clone(),
                dir_hash: hash.clone(),
                files: files.into_iter().map(|(f, _)| f).collect(),
            });
            dirs.push(IssuedDir { path: dir, dir_hash: hash, account, only: None });
        }
    }
    // Files held in the agent's own folder at first sighting, while they
    // still hold what was held.
    let heads = record::heads(fs, agent_uid)?;
    if let Some(held) = heads.held.get(&own_id) {
        let names: Vec<String> = held.keys().cloned().collect();
        let files: Vec<_> = folder_files(own_dir, Some(&names))
            .into_iter()
            .filter(|(f, _)| held.get(&f.name) == Some(&f.sha256))
            .collect();
        if !files.is_empty() {
            let hash = dir_hash(&files);
            let only = files.iter().map(|(f, _)| f.name.clone()).collect();
            candidates.push(Candidate {
                index: candidates.len(),
                account: "held".into(),
                dir_hash: hash.clone(),
                files: files.into_iter().map(|(f, _)| f).collect(),
            });
            dirs.push(IssuedDir { path: own_dir.to_path_buf(), dir_hash: hash, account: "held".into(), only: Some(only) });
        }
    }
    let list_id = uuid::Uuid::new_v4().simple().to_string();
    let mut map = issued().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, i| i.at.elapsed() < LIST_TTL);
    map.insert(list_id.clone(), Issued { agent_uid: agent_uid.to_string(), dirs, at: Instant::now() });
    Ok(AdoptionList { list_id, agent_uid: agent_uid.to_string(), candidates })
}

/// The candidates for an agent, from its verified memory folder. `None`
/// when the agent has none yet (no spawn, no working directory).
pub(crate) fn list(fs: &FileStore, mstore: &Store, agent: &crate::backend::storage::AgentDefinition) -> Result<Option<AdoptionList>, StoreError> {
    use crate::server::native_memory_handlers::{resolve_memory_dir_by_id, MemoryDirProvenance};
    let Some(own) = resolve_memory_dir_by_id(mstore, agent).filter(|r| r.provenance != MemoryDirProvenance::Unverified) else {
        return Ok(None);
    };
    let Some(shared) = crate::registry::resolve_global_shared_root() else { return Ok(None) };
    list_in(fs, mstore, &agent.id, &own.path, &shared).map(Some)
}

/// The link target of an index line (`[title](target)`), if it has one.
fn link_target(line: &str) -> Option<&str> {
    let start = line.find("](")? + 2;
    let end = line[start..].find(')')? + start;
    Some(line[start..end].trim())
}

/// `base` with every line of `others` it lacks: a linked line when its
/// target is new, any other line when no line of `base` equals it. Returns
/// the union and how many lines were added.
fn union_index(base: &str, others: &[&str]) -> (String, usize) {
    let mut out = base.to_string();
    let mut targets: std::collections::HashSet<String> = base.lines().filter_map(link_target).map(str::to_string).collect();
    let mut plain: std::collections::HashSet<String> = base.lines().map(|l| l.trim().to_string()).collect();
    let mut added = 0;
    for other in others {
        for line in other.lines() {
            let new = match link_target(line) {
                Some(t) => targets.insert(t.to_string()),
                None => !line.trim().is_empty() && plain.insert(line.trim().to_string()),
            };
            if new {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(line);
                out.push('\n');
                plain.insert(line.trim().to_string());
                added += 1;
            }
        }
    }
    (out, added)
}

/// Adopt the chosen folders of list `list_id` into `agent_uid`'s record.
/// Choices are `(index, dir_hash)` from the list; nothing else from the
/// caller is trusted.
pub(crate) fn adopt(
    fs: &FileStore,
    mstore: &Store,
    agent_uid: &str,
    list_id: &str,
    choices: &[(usize, String)],
) -> Result<AdoptReport, AdoptError> {
    let chosen: Vec<(PathBuf, String, Option<Vec<String>>)> = {
        let map = issued().lock().unwrap_or_else(|e| e.into_inner());
        let list = map
            .get(list_id)
            .filter(|l| l.agent_uid == agent_uid && l.at.elapsed() < LIST_TTL)
            .ok_or(AdoptError::UnknownList)?;
        let mut out = Vec::new();
        for (index, hash) in choices {
            let d = list.dirs.get(*index).ok_or(AdoptError::BadChoice)?;
            if &d.dir_hash != hash {
                return Err(AdoptError::BadChoice);
            }
            out.push((d.path.clone(), d.account.clone(), d.only.clone()));
        }
        out
    };
    if chosen.is_empty() {
        return Err(AdoptError::BadChoice);
    }
    let lease = crate::backend::memory_reconcile::take_pass_lease(fs, agent_uid, Instant::now() + Duration::from_secs(5))?
        .ok_or(AdoptError::Busy)?;
    let result = adopt_under_lease(fs, mstore, agent_uid, &chosen, choices);
    crate::backend::memory_reconcile::release_pass_lease(fs, agent_uid, &lease);
    if result.is_ok() {
        issued().lock().unwrap_or_else(|e| e.into_inner()).remove(list_id);
    }
    result
}

fn adopt_under_lease(
    fs: &FileStore,
    mstore: &Store,
    agent_uid: &str,
    chosen: &[(PathBuf, String, Option<Vec<String>>)],
    choices: &[(usize, String)],
) -> Result<AdoptReport, AdoptError> {
    // Read every chosen folder again, now: it must still be what was listed,
    // and still no other agent's (the agent's own folder, for held files, is
    // its own by its claim).
    let mut by_file: BTreeMap<String, Vec<(Vec<u8>, i64, String)>> = BTreeMap::new();
    let mut released: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for ((dir, account, only), (index, hash)) in chosen.iter().zip(choices) {
        let files = folder_files(dir, only.as_deref());
        let others = only.is_none() && claims::used_by_another(fs, mstore, agent_uid, dir)?;
        if &dir_hash(&files) != hash || others {
            return Err(AdoptError::Changed { index: *index });
        }
        if let Some(names) = only {
            released.push((dir.clone(), names.clone()));
        }
        for (f, body) in files {
            by_file.entry(f.name).or_default().push((body, f.modified_ms, account.clone()));
        }
    }
    let heads = record::heads(fs, agent_uid)?;
    let mut report = AdoptReport::default();
    for (name, mut bodies) in by_file {
        bodies.sort_by_key(|(_, mtime, _)| *mtime);
        let history: Vec<ImportedVersion> = bodies
            .iter()
            .map(|(body, mtime, account)| ImportedVersion {
                file: name.clone(),
                body: body.clone(),
                source: "adopted".into(),
                source_detail: format!("earlier account {account}"),
                created_at_ms: *mtime,
            })
            .collect();
        let head = heads.files.get(&name);
        let live_head = head.filter(|h| h.sha256.is_some());
        if name == INDEX_FILE {
            // Kept as history, then unioned into the current index (or the
            // newest adopted one, when the record has none).
            report.versions_kept += record::add_history(fs, agent_uid, &history)?;
            let current = match live_head.and_then(|h| h.sha256.as_deref()) {
                Some(sha) => record::body(fs, agent_uid, sha)?.map(|b| String::from_utf8_lossy(&b).into_owned()),
                None => None,
            };
            let texts: Vec<String> = bodies.iter().rev().map(|(b, _, _)| String::from_utf8_lossy(b).into_owned()).collect();
            let (base, rest) = match &current {
                Some(c) => (c.as_str(), &texts[..]),
                None => (texts[0].as_str(), &texts[1..]),
            };
            let (union, added) = union_index(base, &rest.iter().map(String::as_str).collect::<Vec<_>>());
            if current.as_deref() != Some(union.as_str()) {
                let outcome = record::append_version(
                    fs,
                    agent_uid,
                    NewVersion {
                        file: INDEX_FILE,
                        body: Some(union.as_bytes()),
                        expected_parent: head.map(|h| h.version.as_str()),
                        merged_parent: None,
                        conflicts_with: None,
                        source: "adopted",
                        source_detail: "index union with earlier accounts",
                        project_to: None,
                    },
                )?;
                if !matches!(outcome, AppendOutcome::Appended(_) | AppendOutcome::Unchanged(_)) {
                    return Err(AdoptError::Busy);
                }
                report.index_lines_added += added;
                if current.is_none() {
                    report.files_added += 1;
                }
            }
            continue;
        }
        if live_head.is_some() {
            // The agent's current memory keeps its head.
            report.versions_kept += record::add_history(fs, agent_uid, &history)?;
            continue;
        }
        // New to the record: all but the newest as history, the newest as
        // the head, on whatever the file had (nothing, or a tombstone).
        let (newest, older) = history.split_last().expect("at least one body per file");
        report.versions_kept += record::add_history(fs, agent_uid, older)?;
        let outcome = record::append_version(
            fs,
            agent_uid,
            NewVersion {
                file: &name,
                body: Some(&newest.body),
                expected_parent: head.map(|h| h.version.as_str()),
                merged_parent: None,
                conflicts_with: None,
                source: "adopted",
                source_detail: &newest.source_detail,
                project_to: None,
            },
        )?;
        match outcome {
            AppendOutcome::Appended(_) => report.files_added += 1,
            AppendOutcome::Unchanged(_) => {}
            _ => return Err(AdoptError::Busy),
        }
    }
    for (dir, names) in released {
        record::release_held(fs, agent_uid, &claims::dir_id(&dir), &names)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
