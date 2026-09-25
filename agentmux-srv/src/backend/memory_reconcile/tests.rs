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
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(cfg), cwd: CWD, overridden: false },
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
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd: CWD, overridden: false },
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

/// Second review P1: a refill of a missing folder cut short must not turn
/// the files it didn't reach into deletions — here or anywhere else.
#[test]
fn a_refill_cut_short_never_deletes_the_rest() {
    let f = fixture();
    for n in ["MEMORY.md", "a.md", "b.md", "c.md"] {
        f.put(n, n);
    }
    f.run();
    // A second folder (another account) holding the same files.
    let other = tempfile::tempdir().unwrap();
    let other_cfg = other.path().display().to_string();
    f.run_in(&other_cfg);
    std::fs::remove_dir_all(f.dir()).unwrap();

    let partial = f.run_files(2);
    assert!(partial.deferred, "{partial:?}");
    let next = f.run();
    assert_eq!(next.captured, 0, "nothing captured as deleted: {next:?}");
    assert_eq!(record::heads(&f.fs, UID).unwrap().live().count(), 4);
    for n in ["a.md", "b.md", "c.md"] {
        assert_eq!(f.get(n).as_deref(), Some(n));
    }
    assert_eq!(f.run_in(&other_cfg).deleted, 0, "the other folder keeps everything");
}

/// Second review P2: an emptied folder is refilled, not a wholesale deletion.
#[test]
fn an_emptied_folder_is_refilled_not_deleted_everywhere() {
    let f = fixture();
    for n in ["MEMORY.md", "a.md"] {
        f.put(n, n);
    }
    f.run();
    let other = tempfile::tempdir().unwrap();
    let other_cfg = other.path().display().to_string();
    f.run_in(&other_cfg);
    for n in ["MEMORY.md", "a.md"] {
        std::fs::remove_file(f.dir().join(n)).unwrap();
    }
    let r = f.run();
    assert_eq!((r.captured, r.written), (0, 2), "{r:?}");
    assert_eq!(f.run_in(&other_cfg).deleted, 0);
}

/// Second review P2: if the other side of a conflict can't be written, the
/// winner isn't recorded either, so the conflict is raised again next pass.
#[cfg(unix)]
#[test]
fn a_conflict_whose_copy_fails_is_raised_again_not_lost() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.put("notes.md", "base");
    f.run();
    f.change_elsewhere("notes.md", Some("theirs"));
    f.put("notes.md", "mine");
    std::fs::set_permissions(f.dir(), std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(f.dir().join(".probe"), b"").is_ok() {
        std::fs::set_permissions(f.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
        return; // running as root
    }
    let failed = f.run();
    std::fs::set_permissions(f.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(failed.skipped, Some("error"));
    assert_eq!(f.head_body("notes.md").as_deref(), Some("theirs"), "the winner wasn't recorded");

    let again = f.run();
    assert_eq!(again.conflicts, 1, "{again:?}");
    let copies = Fixture::conflict_files(&f.dir());
    assert_eq!(f.get(&copies[0]).as_deref(), Some("theirs"));
}

/// Second review P2 (latent): a head whose name the disk scan doesn't
/// recognise is neither written nor tombstoned.
#[test]
fn a_head_the_disk_scan_skips_is_left_out() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.run();
    f.change_elsewhere("notes.v2.md", Some("x"));
    f.run();
    assert!(f.get("notes.v2.md").is_none());
    f.run();
    assert_eq!(f.head_body("notes.v2.md").as_deref(), Some("x"), "not tombstoned");
}

/// Data-loss review 3, P1: the CLI keys memory by the repository's main
/// checkout, so an agent at a repository root and one in a linked worktree
/// of it (outside it) share one folder. Neither is exclusive: nothing is
/// captured from, written into or deleted from it.
#[test]
fn a_repository_root_and_its_worktree_leave_their_shared_folder_alone() {
    let f = fixture();
    let base = tempfile::tempdir().unwrap();
    let (repo, wt) = crate::backend::claude_layout::tests::repo_with_worktree(base.path());
    let (repo_s, wt_s, cfg) = (repo.to_string_lossy().into_owned(), wt.to_string_lossy().into_owned(), f.cfg());
    let shared = crate::server::native_memory_handlers::memory_dir_for_cwd(&cfg, &repo_s);
    assert_eq!(crate::server::native_memory_handlers::memory_dir_for_cwd(&cfg, &wt_s), shared, "the CLI's folder for both");
    put_in(&shared, "MEMORY.md", "root idx");
    put_in(&shared, "wt-notes.md", "the worktree agent's fact");
    for cwd in [repo_s.as_str(), wt_s.as_str(), &format!("{repo_s}/sub")] {
        let r = reconcile_before_spawn(
            &f.fs,
            &f.store,
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd, overridden: false },
            Duration::from_secs(10),
        );
        assert_eq!(r.skipped, Some("shared directory"), "{cwd}");
    }
    assert!(record::heads(&f.fs, UID).unwrap().files.is_empty(), "nothing captured");
    assert_eq!(std::fs::read_to_string(shared.join("wt-notes.md")).unwrap(), "the worktree agent's fact");
}

/// A subdirectory of a repository without worktrees is keyed by the root
/// too: another agent at the root vetoes it, and it vetoes the root.
#[test]
fn a_subdirectory_agent_shares_the_repository_roots_folder() {
    let f = fixture();
    let base = tempfile::tempdir().unwrap();
    let repo = base.path().canonicalize().unwrap().join("solo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let cfg = f.cfg();
    let (root_s, sub_s) = (repo.to_string_lossy().into_owned(), repo.join("sub").to_string_lossy().into_owned());
    assert_eq!(
        crate::server::native_memory_handlers::memory_dir_for_cwd(&cfg, &sub_s),
        crate::server::native_memory_handlers::memory_dir_for_cwd(&cfg, &root_s)
    );
    let run = |cwd: &str| {
        reconcile_before_spawn(
            &f.fs,
            &f.store,
            &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd, overridden: false },
            Duration::from_secs(10),
        )
    };
    assert_eq!(run(&sub_s).skipped, Some("shared directory"));
}

#[test]
fn an_overridden_memory_folder_is_left_alone() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    let cfg = f.cfg();
    let r = reconcile_before_spawn(
        &f.fs,
        &f.store,
        &SpawnMemory { uid: UID, provider: "claude", config_dir: Some(&cfg), cwd: CWD, overridden: true },
        Duration::from_secs(10),
    );
    assert_eq!(r.skipped, Some("memory folder overridden"));

    std::fs::write(f.config.path().join("settings.json"), r#"{"autoMemoryDirectory":"~/elsewhere"}"#).unwrap();
    assert_eq!(f.run().skipped, Some("memory folder overridden"));
    assert!(record::heads(&f.fs, UID).unwrap().files.is_empty(), "nothing captured");

    let env = std::collections::HashMap::from([("CLAUDE_COWORK_MEMORY_PATH_OVERRIDE".to_string(), "/x".to_string())]);
    assert!(spawn_overrides_memory_dir(&env, &[]));
    assert!(spawn_overrides_memory_dir(&Default::default(), &["--settings".into(), "s.json".into()]));
    assert!(!spawn_overrides_memory_dir(&Default::default(), &["--model".into(), "opus".into()]));
    let env = std::collections::HashMap::from([("CLAUDE_CODE_PROJECT_DIR_NAME".to_string(), "fixed".to_string())]);
    assert!(spawn_overrides_memory_dir(&env, &[]));
}

/// Data-loss review 3, P2: indexing a conflict keeps an index that isn't
/// UTF-8 byte for byte.
#[test]
fn a_non_utf8_index_keeps_its_bytes_when_a_conflict_is_indexed() {
    let f = fixture();
    std::fs::create_dir_all(f.dir()).unwrap();
    let idx: Vec<u8> = b"- caf\xe9 notes (latin-1)\n".to_vec();
    std::fs::write(f.dir().join("MEMORY.md"), &idx).unwrap();
    f.put("notes.md", "base");
    f.run();
    f.change_elsewhere("notes.md", Some("theirs"));
    f.put("notes.md", "mine");
    assert_eq!(f.run().conflicts, 1);
    let now = std::fs::read(f.dir().join("MEMORY.md")).unwrap();
    assert!(now.starts_with(&idx), "{now:?}");
    assert!(now.len() > idx.len(), "the conflict line was added");
}

/// Data-loss review 3, P2: a file is decided against the disk as it is
/// when its turn comes, not a listing from the start of the pass — so a
/// file another pass wrote since is not taken for deleted.
#[test]
fn a_file_written_after_the_listing_is_not_taken_for_deleted() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    f.run();
    let other = tempfile::tempdir().unwrap();
    let other_cfg = other.path().display().to_string();
    f.run_in(&other_cfg);
    put_in(&Fixture::dir_for(&other_cfg), "new.md", "made in the other folder");
    assert_eq!(f.run_in(&other_cfg).captured, 1);
    let dir = f.dir();
    let dir_id = crate::backend::memory_dir_claims::dir_id(&dir);
    assert_eq!(f.run().written, 1, "pass P writes new.md here");
    // Pass Q, which listed the folder before P wrote it, reaches new.md.
    let (mut rep, mut dels) = (Report::default(), Vec::new());
    reconcile_file(&f.fs, UID, &dir, &dir_id, "new.md", &mut rep, &mut dels, true).unwrap();
    assert!(dels.is_empty());
    assert_eq!(f.head_body("new.md").as_deref(), Some("made in the other folder"), "no deletion recorded");
    assert_eq!(f.get("new.md").as_deref(), Some("made in the other folder"));
}

/// Two passes for one agent at once skip rather than race; the lease frees
/// when the pass ends, and a crashed pass's lease expires.
#[test]
fn a_second_pass_while_one_runs_skips() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    let lease = take_pass_lease(&f.fs, UID, Instant::now() + Duration::from_secs(10)).unwrap().unwrap();
    assert_eq!(f.run().skipped, Some("another pass running"));
    release_pass_lease(&f.fs, UID, &lease);
    assert_eq!(f.run().skipped, None);
    assert_eq!(f.run().skipped, None, "a finished pass releases its lease");
    // A lease that has run out is taken over.
    let zone = pass_zone(UID);
    f.fs.zone_txn(&zone, |z| {
        z.put(PASS_LEASE_FILE, &serde_json::to_vec(&PassLease { owner: "crashed".into(), until_ms: 1 }).unwrap())
    })
    .unwrap();
    assert_eq!(f.run().skipped, None);
}

/// Data-loss review 4, P1: channels share `projects/` folders, and an agent
/// in another channel is visible only through its claim — so a spawn that
/// finds its folder shared still claims it. Here B (channel 2, a
/// subdirectory) spawns first; A (channel 1, the repository root, same
/// folder) must then leave B's memory alone.
#[test]
fn an_agent_in_another_channel_sharing_the_folder_keeps_it_shared() {
    let f = fixture();
    let base = tempfile::tempdir().unwrap();
    let repo = base.path().canonicalize().unwrap().join("solo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let (root_s, sub_s, cfg) = (repo.to_string_lossy().into_owned(), repo.join("sub").to_string_lossy().into_owned(), f.cfg());
    let channel2 = Store::open_in_memory().unwrap();
    let mut b = crate::backend::storage::agents::test_agent_def("agent-b", "agent-b", "claude", "agent", 1, "");
    b.working_directory = sub_s.clone();
    channel2.agent_def_insert(&mut b).unwrap();
    let spawn = |store: &Store, uid: &str, cwd: &str| {
        reconcile_before_spawn(
            &f.fs,
            store,
            &SpawnMemory { uid, provider: "claude", config_dir: Some(&cfg), cwd, overridden: false },
            Duration::from_secs(10),
        )
    };
    assert_eq!(spawn(&channel2, "agent-b", &sub_s).skipped, Some("shared directory"));
    let dir = crate::server::native_memory_handlers::memory_dir_for_cwd(&cfg, &root_s);
    put_in(&dir, "MEMORY.md", "b's index");
    put_in(&dir, "b_private.md", "B's own memory");
    assert_eq!(spawn(&f.store, UID, &root_s).skipped, Some("shared directory"));
    assert!(record::heads(&f.fs, UID).unwrap().files.is_empty(), "nothing of B's captured into A's record");
    assert_eq!(std::fs::read_to_string(dir.join("b_private.md")).unwrap(), "B's own memory");
}

/// Data-loss review 4, P2: a conflict in a folder with no MEMORY.md gets an
/// index written, not just recorded — so the next pass doesn't take the
/// index for deleted, and Claude sees the other copy.
#[test]
fn a_conflict_in_a_folder_without_an_index_gets_one() {
    let f = fixture();
    f.put("topic.md", "v1");
    f.run();
    f.change_elsewhere("topic.md", Some("record v2"));
    f.put("topic.md", "disk v2");
    assert_eq!(f.run().conflicts, 1);
    let index = f.get("MEMORY.md").expect("the index is written");
    assert!(index.contains("topic__conflict_"), "{index}");
    let r = f.run();
    assert_eq!((r.captured, r.conflicts, r.deleted), (0, 0, 0), "{r:?}");
    assert!(f.head_body("MEMORY.md").is_some(), "the index is not tombstoned");
}

/// A lease left by a pass that crashed while the clock was far ahead
/// doesn't block every pass until the clock catches up.
#[test]
fn a_lease_from_a_clock_far_ahead_is_taken_over() {
    let f = fixture();
    f.put("MEMORY.md", "idx");
    let far = agentmux_common::time::now_ms() + 3_600_000;
    f.fs.zone_txn(&pass_zone(UID), |z| {
        z.put(PASS_LEASE_FILE, &serde_json::to_vec(&PassLease { owner: "crashed".into(), until_ms: far }).unwrap())
    })
    .unwrap();
    assert_eq!(f.run().skipped, None);
}
