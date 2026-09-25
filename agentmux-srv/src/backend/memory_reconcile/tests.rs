// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

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
    fn cfg(&self) -> String {
        self.config.path().display().to_string()
    }
    fn dir_for(cfg: &str) -> PathBuf {
        crate::server::native_memory_handlers::memory_dir_for_cwd(cfg, CWD)
    }
    fn dir(&self) -> PathBuf {
        Self::dir_for(&self.cfg())
    }
    fn run_in(&self, cfg: &str) -> Report {
        reconcile_before_spawn(
            &self.fs,
            &self.store,
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(cfg), cwd: CWD },
            Duration::from_secs(10),
        )
    }
    fn run(&self) -> Report {
        self.run_in(&self.cfg())
    }
    fn run_files(&self, files: usize) -> Report {
        let cfg = self.cfg();
        run(
            &self.fs,
            &self.store,
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd: CWD },
            Budget { deadline: Instant::now() + Duration::from_secs(10), files_left: Some(files) },
        )
    }
    fn put(&self, name: &str, body: &str) {
        put_in(&self.dir(), name, body);
    }
    fn get(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir().join(name)).ok()
    }
    fn head_body(&self, name: &str) -> Option<String> {
        let h = record::heads(&self.fs, UID).unwrap();
        let sha = h.files.get(name)?.sha256.clone()?;
        Some(String::from_utf8(record::body(&self.fs, UID, &sha).unwrap().unwrap()).unwrap())
    }
    /// A change to the record from elsewhere (another machine, account...).
    fn change_elsewhere(&self, name: &str, body: Option<&str>) {
        let head = record::heads(&self.fs, UID).unwrap().files.get(name).map(|h| h.version.clone());
        let out = record::append_version(
            &self.fs,
            UID,
            NewVersion {
                file: name,
                body: body.map(str::as_bytes),
                expected_parent: head.as_deref(),
                merged_parent: None,
                conflicts_with: None,
                source: "sync",
                source_detail: "",
                project_to: None,
            },
        )
        .unwrap();
        assert!(matches!(out, AppendOutcome::Appended(_)), "{out:?}");
    }
    fn conflict_files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(CONFLICT_MARK))
            .collect();
        v.sort();
        v
    }
}

fn put_in(dir: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), body).unwrap();
}

#[test]
fn first_sighting_adopts_the_folder_and_a_second_pass_is_quiet() {
    let f = fixture();
    f.put("MEMORY.md", "- [t](topic.md)\n");
    f.put("topic.md", "fact");
    assert_eq!(f.run().adopted, 2);
    assert_eq!(record::heads(&f.fs, UID).unwrap().files.len(), 2);
    assert_eq!(f.run(), Report::default(), "nothing changed: nothing to do");
}

#[test]
fn a_new_account_folder_is_filled_from_the_record() {
    let f = fixture();
    f.put("MEMORY.md", "index");
    f.run();
    let new_config = tempfile::tempdir().unwrap();
    let cfg = new_config.path().display().to_string();
    assert_eq!(f.run_in(&cfg).written, 1);
    assert_eq!(std::fs::read_to_string(Fixture::dir_for(&cfg).join("MEMORY.md")).unwrap(), "index");
}

#[test]
fn a_provider_write_is_captured_and_a_record_change_is_written() {
    let f = fixture();
    f.put("MEMORY.md", "v1");
    f.run();
    f.put("MEMORY.md", "v2 by claude");
    assert_eq!(f.run().captured, 1);
    assert_eq!(f.head_body("MEMORY.md").as_deref(), Some("v2 by claude"));
    f.change_elsewhere("MEMORY.md", Some("v3 elsewhere"));
    assert_eq!(f.run().written, 1);
    assert_eq!(f.get("MEMORY.md").as_deref(), Some("v3 elsewhere"));
}

#[test]
fn both_changed_keeps_disk_writes_the_other_beside_it_and_indexes_it_once() {
    let f = fixture();
    f.put("MEMORY.md", "- [n](notes.md)\n");
    f.put("notes.md", "base");
    f.run();
    f.change_elsewhere("notes.md", Some("theirs"));
    f.put("notes.md", "mine");

    assert_eq!(f.run().conflicts, 1);
    assert_eq!(f.get("notes.md").as_deref(), Some("mine"), "disk keeps its file");
    let conflicts = Fixture::conflict_files(&f.dir());
    assert_eq!(conflicts.len(), 1);
    assert_eq!(f.get(&conflicts[0]).as_deref(), Some("theirs"));
    assert!(f.get("MEMORY.md").unwrap().contains(&conflicts[0]), "indexed, so Claude sees it");
    assert_eq!(f.head_body("MEMORY.md"), f.get("MEMORY.md"), "the index line is recorded");
    let again = f.run();
    assert_eq!((again.conflicts, again.captured, again.written), (0, 0, 0), "converged");
}

/// Review R2 / M1: a new account folder already holding different files.
/// The agent's own index must survive — never replaced by the foreign one —
/// and both copies must be kept.
#[test]
fn a_new_folder_with_different_files_never_loses_the_agents_own_index() {
    let f = fixture();
    f.put("MEMORY.md", "own-index");
    f.put("API.md", "own-api");
    f.run();
    let new_config = tempfile::tempdir().unwrap();
    let cfg = new_config.path().display().to_string();
    let new_dir = Fixture::dir_for(&cfg);
    put_in(&new_dir, "MEMORY.md", "foreign-index");
    put_in(&new_dir, "API.md", "foreign-api");

    let r = f.run_in(&cfg);
    assert_eq!(r.conflicts, 2, "{r:?}");
    let copies = Fixture::conflict_files(&new_dir);
    let bodies: Vec<String> = copies.iter().map(|c| std::fs::read_to_string(new_dir.join(c)).unwrap()).collect();
    assert!(bodies.contains(&"own-index".to_string()), "{bodies:?}");
    assert!(bodies.contains(&"own-api".to_string()), "{bodies:?}");

    // Back in the original folder, both conflict copies follow it, and the
    // original files are never overwritten by the foreign ones silently.
    let back = f.run();
    assert_eq!(back.conflicts, 0);
    let orig_copies = Fixture::conflict_files(&f.dir());
    assert_eq!(orig_copies.len(), 2, "the conflict copies follow the agent: {orig_copies:?}");
}

/// Review R1 (same folder, MEMORY.md changed elsewhere, a topic conflicts):
/// MEMORY.md is reconciled first and never rebased onto a stale disk copy.
#[test]
fn the_index_is_reconciled_first_and_never_silently_replaced() {
    let f = fixture();
    f.put("MEMORY.md", "idx-v1");
    f.put("API.md", "api-v1");
    f.run();
    f.change_elsewhere("MEMORY.md", Some("idx-remote"));
    f.change_elsewhere("API.md", Some("api-remote"));
    f.put("API.md", "api-local");

    let r = f.run();
    assert_eq!(r.written, 1, "the remote index is written: {r:?}");
    assert_eq!(r.conflicts, 1);
    let idx = f.get("MEMORY.md").unwrap();
    assert!(idx.starts_with("idx-remote"), "{idx}");
    assert!(idx.contains(&Fixture::conflict_files(&f.dir())[0]), "and indexes the conflict copy");
    assert_eq!(f.head_body("MEMORY.md").as_deref(), Some(idx.as_str()));
}

/// Review R3/R4: an unreadable or oversized file is left alone — never
/// captured as deleted, never overwritten.
#[test]
fn an_unreadable_file_is_neither_deleted_nor_overwritten() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("big.md", "small at first");
    f.run();
    f.change_elsewhere("big.md", Some("remote"));
    let huge = "x".repeat(record::MAX_BODY_BYTES + 1);
    f.put("big.md", &huge);

    let r = f.run();
    assert_eq!(r.unreadable, 1);
    assert_eq!(std::fs::metadata(f.dir().join("big.md")).unwrap().len() as usize, huge.len(), "not overwritten");
    assert_eq!(f.head_body("big.md").as_deref(), Some("remote"), "not tombstoned");
}

/// Review R5: a folder removed wholesale is refilled, never tombstoned.
#[test]
fn a_missing_folder_is_refilled_not_deleted_everywhere() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("topic.md", "t");
    f.run();
    std::fs::remove_dir_all(f.dir()).unwrap();

    let r = f.run();
    assert_eq!(r.written, 2, "{r:?}");
    assert_eq!(record::heads(&f.fs, UID).unwrap().live().count(), 2, "nothing tombstoned");
    assert_eq!(f.get("topic.md").as_deref(), Some("t"));
}

/// A deletion from elsewhere removes the file only if it still holds what
/// was projected; a locally modified file survives (review M4).
#[test]
fn a_tombstone_never_deletes_a_locally_modified_file() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("gone.md", "x");
    f.run();
    f.change_elsewhere("gone.md", None);
    f.put("gone.md", "edited here");
    f.run();
    assert_eq!(f.get("gone.md").as_deref(), Some("edited here"));
    assert_eq!(f.head_body("gone.md").as_deref(), Some("edited here"), "it comes back to life");
}

#[test]
fn a_tombstone_deletes_an_unchanged_file_after_a_complete_pass() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("gone.md", "x");
    f.run();
    f.change_elsewhere("gone.md", None);
    assert_eq!(f.run().deleted, 1);
    assert!(f.get("gone.md").is_none());
}

/// Review M3: a pass that runs out of budget AFTER queuing a deletion must
/// not apply it.
#[test]
fn a_partial_pass_never_deletes_even_with_a_deletion_queued() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("a-gone.md", "x");
    f.put("z-last.md", "z");
    f.run();
    f.change_elsewhere("a-gone.md", None);
    // MEMORY.md, then a-gone.md (queued), then out of budget before z-last.md.
    let r = f.run_files(2);
    assert!(r.deferred, "{r:?}");
    assert!(f.get("a-gone.md").is_some(), "a partial pass writes no deletions");
}

/// Review R6: an index kept alive this pass is never removed by a deletion
/// queued before it.
#[test]
fn a_deletion_rechecks_the_record_before_removing() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("notes.md", "base");
    f.run();
    f.change_elsewhere("MEMORY.md", None);
    f.change_elsewhere("notes.md", Some("theirs"));
    f.put("notes.md", "mine");
    f.run();
    let idx = f.get("MEMORY.md");
    let head = f.head_body("MEMORY.md");
    assert_eq!(idx, head, "disk and record agree on the index: {idx:?} vs {head:?}");
}

#[test]
fn a_shared_directory_is_left_exactly_as_it_is() {
    let f = fixture();
    let mut other = crate::backend::storage::agents::test_agent_def("agent-other", "agent-other", "claude", "agent", 1, "");
    other.working_directory = CWD.to_string();
    f.store.agent_def_insert(&mut other).unwrap();
    for id in ["agent-other", UID] {
        f.store
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: id.into(),
                content_type: "env".into(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", f.cfg()),
                updated_at: 0,
            })
            .unwrap();
    }
    f.put("MEMORY.md", "someone's");
    assert_eq!(f.run().skipped, Some("shared directory"));
    assert!(record::heads(&f.fs, UID).unwrap().files.is_empty(), "nothing captured");
}

#[test]
fn conflict_names_stay_valid_for_long_stems() {
    let long = format!("{}.md", "a".repeat(200));
    let name = conflict_file_name(&long, "v_0123456789abcdef");
    assert!(crate::server::native_memory_handlers::validate_filename(&name).is_ok(), "{name}");
    assert!(name.contains("__conflict_01234567"));
}

#[test]
fn non_utf8_bytes_are_recorded_exactly() {
    let f = fixture();
    std::fs::create_dir_all(f.dir()).unwrap();
    std::fs::write(f.dir().join("bin.md"), [0xffu8, 0xfe, b'x']).unwrap();
    f.run();
    let h = record::heads(&f.fs, UID).unwrap();
    let sha = h.files["bin.md"].sha256.clone().unwrap();
    assert_eq!(record::body(&f.fs, UID, &sha).unwrap().unwrap(), vec![0xffu8, 0xfe, b'x']);
}

/// The re-check itself: a file that changed after its deletion was queued
/// (e.g. the provider wrote it mid-pass) is not removed.
#[test]
fn a_queued_deletion_skips_a_file_changed_since() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("gone.md", "x");
    f.run();
    let projected = record::heads(&f.fs, UID).unwrap().projected[&crate::backend::memory_dir_claims::dir_id(&f.dir())]["gone.md"].clone();
    f.change_elsewhere("gone.md", None);
    let tombstone = record::heads(&f.fs, UID).unwrap().files["gone.md"].version.clone();
    f.put("gone.md", "written after the deletion was queued");
    let d = Deletion {
        name: "gone.md".into(),
        tombstone,
        projected: Some(projected),
        expected_sha: record::sha256_hex(b"x"),
    };
    let mut report = Report::default();
    delete_if_unchanged(&f.fs, UID, &f.dir(), &crate::backend::memory_dir_claims::dir_id(&f.dir()), &d, &mut report).unwrap();
    assert_eq!(report.deleted, 0);
    assert!(f.get("gone.md").is_some());
}

/// A file the process can't read (permissions) is left alone.
#[cfg(unix)]
#[test]
fn a_file_without_read_permission_is_not_taken_for_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("locked.md", "secret");
    f.run();
    let path = f.dir().join("locked.md");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        // Running as root: permissions don't stop reads; nothing to test.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    let r = f.run();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(r.unreadable, 1);
    assert_eq!(f.head_body("locked.md").as_deref(), Some("secret"), "not tombstoned");
}
