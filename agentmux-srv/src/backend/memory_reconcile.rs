// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reconcile an agent's memory record with the provider folder its spawn is
//! about to use (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.3, M3b).
//!
//! Runs before the provider process starts, so a new account, working
//! directory or channel starts its first session WITH the agent's memory.
//! Only a directory proven exclusive (`memory_dir_claims`) is touched.
//!
//! Per file, comparing disk, the record's head, and the version last
//! projected into this directory:
//! - disk equals the head → record the projection, nothing else (rule 0:
//!   covers a crash between a write and its record);
//! - the record changed, disk didn't → write the head to disk;
//! - disk changed, the record didn't → capture it as a new version;
//! - both changed → conflict: disk's version becomes the head, marked
//!   `conflicts_with` the record's, which is written beside it as
//!   `<stem>__conflict_<short>.md` with an index line in MEMORY.md;
//! - the first time the record meets an exclusive directory, its files are
//!   adopted together as the baseline.
//!
//! The whole pass has a time budget. Past it, the spawn proceeds and the
//! rest waits for the next spawn; deletions are applied only by a pass that
//! finished everything else.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::backend::memory_dir_claims::{self as claims, Exclusivity};
use crate::backend::memory_record::{self as record, AppendOutcome, Heads, NewVersion};
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// The whole pass, before the spawn proceeds without it.
pub(crate) const RECONCILE_BUDGET: Duration = Duration::from_secs(1);
const INDEX_FILE: &str = "MEMORY.md";
const CONFLICT_MARK: &str = "__conflict_";

/// What a spawn's memory directory is.
pub(crate) struct SpawnMemory<'a> {
    pub uid: &'a str,
    pub provider: &'a str,
    pub config_dir: Option<&'a str>,
    pub cwd: &'a str,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub skipped: Option<&'static str>,
    pub adopted: usize,
    pub written: usize,
    pub captured: usize,
    pub conflicts: usize,
    pub deleted: usize,
    /// The budget ran out; the rest waits for the next spawn.
    pub deferred: bool,
}

/// Reconcile before a spawn. Never fails the spawn: every error is logged
/// and reported as skipped.
pub(crate) fn reconcile_before_spawn(fs: &FileStore, mstore: &Store, m: &SpawnMemory<'_>, budget: Duration) -> Report {
    let started = Instant::now();
    match reconcile_inner(fs, mstore, m, started, budget) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(uid = m.uid, error = %e, "memory reconcile skipped");
            Report { skipped: Some("error"), ..Default::default() }
        }
    }
}

fn reconcile_inner(fs: &FileStore, mstore: &Store, m: &SpawnMemory<'_>, started: Instant, budget: Duration) -> Result<Report, StoreError> {
    let mut report = Report::default();
    if m.provider != "claude" {
        report.skipped = Some("provider");
        return Ok(report);
    }
    if m.cwd.trim().is_empty() {
        report.skipped = Some("no working directory");
        return Ok(report);
    }
    let dir = crate::server::native_memory_handlers::memory_dir_for_cwd(m.config_dir.unwrap_or(""), m.cwd);
    if let Exclusivity::Shared(reason) = claims::check_and_claim(fs, mstore, m.uid, &dir, m.cwd)? {
        tracing::info!(uid = m.uid, dir = %dir.display(), ?reason, "memory reconcile: shared directory left as is");
        report.skipped = Some("shared directory");
        return Ok(report);
    }
    let dir_id = claims::dir_id(&dir);
    let heads = record::heads(fs, m.uid)?;
    let disk = read_disk(&dir);

    if heads.files.is_empty() {
        adopt_baseline(fs, m.uid, &dir_id, &disk, &mut report)?;
        return Ok(report);
    }

    let names: BTreeSet<&String> = heads.files.keys().chain(disk.iter().map(|(n, _)| n)).collect();
    let mut deletions: Vec<(String, String)> = Vec::new();
    for name in names {
        if started.elapsed() > budget {
            report.deferred = true;
            break;
        }
        let on_disk = disk.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice());
        reconcile_file(fs, m.uid, &dir, &dir_id, &heads, name, on_disk, &mut report, &mut deletions)?;
    }
    if !report.deferred {
        for (name, version) in deletions {
            let path = dir.join(&name);
            if std::fs::remove_file(&path).is_ok() {
                record::record_projected(fs, m.uid, &dir_id, &name, &version, heads.projected.get(&dir_id).and_then(|p| p.get(&name)).map(String::as_str))?;
                report.deleted += 1;
            }
        }
    }
    Ok(report)
}

/// The directory's memory files: `*.md`, valid names, never conflict files.
fn read_disk(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, Vec<u8>)> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (!name.contains(CONFLICT_MARK) && crate::server::native_memory_handlers::validate_filename(&name).is_ok())
                .then_some(name)
        })
        .filter_map(|name| {
            let body = crate::backend::native_memory_drift::read_memory_file_lossy(&dir.join(&name)).ok()?;
            Some((name, body.into_bytes()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The first time the record meets this agent's own exclusive directory:
/// every file in it, index and topics together, becomes the baseline.
fn adopt_baseline(fs: &FileStore, uid: &str, dir_id: &str, disk: &[(String, Vec<u8>)], report: &mut Report) -> Result<(), StoreError> {
    for (name, body) in disk {
        let outcome = record::append_version(
            fs,
            uid,
            NewVersion {
                file: name,
                body: Some(body),
                expected_parent: None,
                merged_parent: None,
                conflicts_with: None,
                source: "adopted",
                source_detail: "first sighting, own spawn dir",
                project_to: Some(dir_id),
            },
        )?;
        if matches!(outcome, AppendOutcome::Appended(_)) {
            report.adopted += 1;
        }
    }
    Ok(())
}

/// The version that `version` of `name` holds, from the log.
fn sha_of(fs: &FileStore, uid: &str, name: &str, version: &str) -> Result<Option<Option<String>>, StoreError> {
    Ok(record::history(fs, uid, name)?.into_iter().find(|v| v.version == version).map(|v| v.sha256))
}

#[allow(clippy::too_many_arguments)]
fn reconcile_file(
    fs: &FileStore,
    uid: &str,
    dir: &Path,
    dir_id: &str,
    heads: &Heads,
    name: &str,
    on_disk: Option<&[u8]>,
    report: &mut Report,
    deletions: &mut Vec<(String, String)>,
) -> Result<(), StoreError> {
    let head = heads.files.get(name);
    let head_sha = head.and_then(|h| h.sha256.clone());
    let disk_sha = on_disk.map(record::sha256_hex);
    let projected = heads.projected.get(dir_id).and_then(|p| p.get(name)).cloned();

    // Rule 0: disk already holds the head (or both are absent).
    if disk_sha == head_sha {
        if let (Some(h), true) = (head, projected.as_deref() != head.map(|h| h.version.as_str())) {
            record::record_projected(fs, uid, dir_id, name, &h.version, projected.as_deref())?;
        }
        return Ok(());
    }

    let projected_sha = match &projected {
        Some(p) => sha_of(fs, uid, name, p)?.flatten(),
        None => None,
    };
    let disk_changed = match &projected {
        Some(_) => disk_sha != projected_sha,
        // Never projected here: anything on disk is news to the record.
        None => disk_sha.is_some(),
    };
    let record_changed = match (&projected, head) {
        (Some(p), Some(h)) => &h.version != p,
        (None, Some(h)) => h.sha256.is_some(),
        (_, None) => false,
    };

    match (disk_changed, record_changed) {
        // The record changed: bring disk to the head.
        (false, true) => {
            let h = head.expect("record_changed implies a head");
            match &h.sha256 {
                Some(sha) => {
                    let body = record::body(fs, uid, sha)?
                        .ok_or_else(|| StoreError::Other(format!("memory record: body {sha} missing")))?;
                    write_atomic(dir, name, &body)?;
                    record::record_projected(fs, uid, dir_id, name, &h.version, projected.as_deref())?;
                    report.written += 1;
                }
                None => deletions.push((name.to_string(), h.version.clone())),
            }
        }
        // Disk changed: capture it on top of what this directory had.
        (true, false) => {
            let outcome = record::append_version(
                fs,
                uid,
                NewVersion {
                    file: name,
                    body: on_disk,
                    expected_parent: head.map(|h| h.version.as_str()),
                    merged_parent: None,
                    conflicts_with: None,
                    source: "provider",
                    source_detail: "",
                    project_to: Some(dir_id),
                },
            )?;
            if matches!(outcome, AppendOutcome::Appended(_)) {
                report.captured += 1;
            }
        }
        // Both changed: disk keeps its file and becomes the head; the
        // record's version goes beside it.
        (true, true) => {
            let h = head.expect("record_changed implies a head");
            let Some(disk_body) = on_disk else {
                // Deleted here, changed there: keep the record's version on
                // disk rather than lose it.
                if let Some(sha) = &h.sha256 {
                    if let Some(body) = record::body(fs, uid, sha)? {
                        write_atomic(dir, name, &body)?;
                        record::record_projected(fs, uid, dir_id, name, &h.version, projected.as_deref())?;
                        report.written += 1;
                    }
                }
                return Ok(());
            };
            let outcome = record::append_version(
                fs,
                uid,
                NewVersion {
                    file: name,
                    body: Some(disk_body),
                    expected_parent: Some(&h.version),
                    merged_parent: None,
                    conflicts_with: Some(&h.version),
                    source: "provider",
                    source_detail: "conflict",
                    project_to: Some(dir_id),
                },
            )?;
            if let (AppendOutcome::Appended(_), Some(sha)) = (outcome, &h.sha256) {
                if let Some(theirs) = record::body(fs, uid, sha)? {
                    let conflict_name = conflict_file_name(name, &h.version);
                    write_atomic(dir, &conflict_name, &theirs)?;
                    add_index_line(fs, uid, dir, dir_id, &conflict_name, name)?;
                    report.conflicts += 1;
                }
            }
        }
        (false, false) => {}
    }
    Ok(())
}

/// `<stem>__conflict_<short>.md`, the stem cut so the whole name stays
/// within `validate_filename`'s 200-character stem.
pub(crate) fn conflict_file_name(name: &str, version: &str) -> String {
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let short: String = version.trim_start_matches("v_").chars().take(8).collect();
    let suffix = format!("{CONFLICT_MARK}{short}");
    let keep = 200usize.saturating_sub(suffix.len()).min(stem.len());
    format!("{}{suffix}.md", &stem[..keep])
}

/// Add a line pointing at a conflict file to MEMORY.md — Claude loads only
/// the index, so an unindexed file would never be seen — recorded as a
/// version so the next pass doesn't take it for a provider write.
fn add_index_line(fs: &FileStore, uid: &str, dir: &Path, dir_id: &str, conflict_name: &str, of: &str) -> Result<(), StoreError> {
    let path = dir.join(INDEX_FILE);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current.contains(conflict_name) {
        return Ok(());
    }
    let mut next = current.clone();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&format!(
        "- [{of} — another copy]({conflict_name}) — AgentMux kept both versions of {of}; merge them and delete this file\n"
    ));
    write_atomic(dir, INDEX_FILE, next.as_bytes())?;
    let heads = record::heads(fs, uid)?;
    let head = heads.files.get(INDEX_FILE).map(|h| h.version.clone());
    record::append_version(
        fs,
        uid,
        NewVersion {
            file: INDEX_FILE,
            body: Some(next.as_bytes()),
            expected_parent: head.as_deref(),
            merged_parent: None,
            conflicts_with: None,
            source: "agentmux",
            source_detail: "conflict index line",
            project_to: Some(dir_id),
        },
    )?;
    Ok(())
}

fn write_atomic(dir: &Path, name: &str, body: &[u8]) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir).map_err(|e| StoreError::Other(format!("memory reconcile: mkdir: {e}")))?;
    let tmp: PathBuf = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, body).map_err(|e| StoreError::Other(format!("memory reconcile: write: {e}")))?;
    std::fs::rename(&tmp, dir.join(name)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StoreError::Other(format!("memory reconcile: rename: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        fs: FileStore,
        store: Store,
        config: tempfile::TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    const UID: &str = "agent-rec";
    const CWD: &str = "/work/rec";

    fn fixture() -> Fixture {
        let guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::agents::test_agent_def(UID, UID, "claude", "agent", 1, "");
        def.working_directory = CWD.to_string();
        store.agent_def_insert(&mut def).unwrap();
        Fixture { fs: FileStore::open_in_memory().unwrap(), store, config: tempfile::tempdir().unwrap(), _guard: guard }
    }

    impl Fixture {
        fn config_dir(&self) -> String {
            self.config.path().display().to_string()
        }
        fn dir(&self) -> PathBuf {
            crate::server::native_memory_handlers::memory_dir_for_cwd(&self.config_dir(), CWD)
        }
        fn run(&self) -> Report {
            let cfg = self.config_dir();
            reconcile_before_spawn(
                &self.fs,
                &self.store,
                &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd: CWD },
                Duration::from_secs(10),
            )
        }
        fn put(&self, name: &str, body: &str) {
            std::fs::create_dir_all(self.dir()).unwrap();
            std::fs::write(self.dir().join(name), body).unwrap();
        }
        fn get(&self, name: &str) -> Option<String> {
            std::fs::read_to_string(self.dir().join(name)).ok()
        }
    }

    #[test]
    fn first_sighting_adopts_the_folder_and_a_second_pass_is_quiet() {
        let f = fixture();
        f.put("MEMORY.md", "- [t](topic.md)\n");
        f.put("topic.md", "fact");
        let r = f.run();
        assert_eq!(r.adopted, 2);
        let h = record::heads(&f.fs, UID).unwrap();
        assert_eq!(h.files.len(), 2);
        assert_eq!(f.run(), Report::default(), "nothing changed: nothing to do");
    }

    #[test]
    fn a_new_account_folder_is_filled_from_the_record() {
        let f = fixture();
        f.put("MEMORY.md", "index");
        f.run();
        // The agent moves to a new account: a new, empty config dir.
        let new_config = tempfile::tempdir().unwrap();
        let cfg = new_config.path().display().to_string();
        let r = reconcile_before_spawn(
            &f.fs,
            &f.store,
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd: CWD },
            Duration::from_secs(10),
        );
        assert_eq!(r.written, 1);
        let new_dir = crate::server::native_memory_handlers::memory_dir_for_cwd(&cfg, CWD);
        assert_eq!(std::fs::read_to_string(new_dir.join("MEMORY.md")).unwrap(), "index");
    }

    #[test]
    fn a_provider_write_is_captured_and_a_record_change_is_written() {
        let f = fixture();
        f.put("MEMORY.md", "v1");
        f.run();
        f.put("MEMORY.md", "v2 by claude");
        assert_eq!(f.run().captured, 1);
        let h = record::heads(&f.fs, UID).unwrap();
        let sha = h.files["MEMORY.md"].sha256.clone().unwrap();
        assert_eq!(record::body(&f.fs, UID, &sha).unwrap().unwrap(), b"v2 by claude");

        // A change from elsewhere (another machine, another account).
        let head = h.files["MEMORY.md"].version.clone();
        record::append_version(&f.fs, UID, NewVersion {
            file: "MEMORY.md", body: Some(b"v3 elsewhere"), expected_parent: Some(&head), merged_parent: None,
            conflicts_with: None, source: "sync", source_detail: "", project_to: None,
        }).unwrap();
        assert_eq!(f.run().written, 1);
        assert_eq!(f.get("MEMORY.md").as_deref(), Some("v3 elsewhere"));
    }

    #[test]
    fn both_changed_keeps_disk_and_writes_the_other_beside_it_once() {
        let f = fixture();
        f.put("MEMORY.md", "- [n](notes.md)\n");
        f.put("notes.md", "base");
        f.run();
        let head = record::heads(&f.fs, UID).unwrap().files["notes.md"].version.clone();
        record::append_version(&f.fs, UID, NewVersion {
            file: "notes.md", body: Some(b"theirs"), expected_parent: Some(&head), merged_parent: None,
            conflicts_with: None, source: "sync", source_detail: "", project_to: None,
        }).unwrap();
        f.put("notes.md", "mine");

        let r = f.run();
        assert_eq!(r.conflicts, 1);
        assert_eq!(f.get("notes.md").as_deref(), Some("mine"), "disk keeps its file");
        let conflict = std::fs::read_dir(f.dir()).unwrap().flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.contains(CONFLICT_MARK)).expect("the other version beside it");
        assert_eq!(f.get(&conflict).as_deref(), Some("theirs"));
        assert!(crate::server::native_memory_handlers::validate_filename(&conflict).is_ok(), "{conflict}");
        assert!(f.get("MEMORY.md").unwrap().contains(&conflict), "indexed, so Claude sees it");
        let r2 = f.run();
        assert_eq!((r2.conflicts, r2.captured), (0, 0), "converged: not raised again");
    }

    #[test]
    fn a_tombstone_deletes_only_after_a_complete_pass() {
        let f = fixture();
        f.put("gone.md", "x");
        f.run();
        let head = record::heads(&f.fs, UID).unwrap().files["gone.md"].version.clone();
        record::append_version(&f.fs, UID, NewVersion {
            file: "gone.md", body: None, expected_parent: Some(&head), merged_parent: None,
            conflicts_with: None, source: "sync", source_detail: "", project_to: None,
        }).unwrap();
        let cfg = f.config_dir();
        let partial = reconcile_before_spawn(
            &f.fs, &f.store,
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd: CWD },
            Duration::ZERO,
        );
        assert!(partial.deferred);
        assert!(f.get("gone.md").is_some(), "a partial pass writes no tombstones");
        assert_eq!(f.run().deleted, 1);
        assert!(f.get("gone.md").is_none());
    }

    #[test]
    fn a_shared_directory_is_left_exactly_as_it_is() {
        let f = fixture();
        let mut other = crate::backend::storage::agents::test_agent_def("agent-other", "agent-other", "claude", "agent", 1, "");
        other.working_directory = CWD.to_string();
        f.store.agent_def_insert(&mut other).unwrap();
        // Same working dir, and the other agent's env names the same config dir.
        f.store.agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: "agent-other".into(), content_type: "env".into(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", f.config_dir()), updated_at: 0,
        }).unwrap();
        f.store.agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: UID.into(), content_type: "env".into(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", f.config_dir()), updated_at: 0,
        }).unwrap();
        f.put("MEMORY.md", "someone's");
        let r = f.run();
        assert_eq!(r.skipped, Some("shared directory"));
        assert!(record::heads(&f.fs, UID).unwrap().files.is_empty(), "nothing captured");
    }

    #[test]
    fn conflict_names_stay_valid_for_long_stems() {
        let long = format!("{}.md", "a".repeat(200));
        let name = conflict_file_name(&long, "v_0123456789abcdef");
        assert!(crate::server::native_memory_handlers::validate_filename(&name).is_ok(), "{name}");
        assert!(name.contains("__conflict_01234567"));
    }
}
