// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The editor's named file and scratch functions, called directly with a temp dir as the home folder (no RPC
//! engine, no real home).

use std::path::Path;

use super::file_io;
use super::scratch;
use crate::backend::rpc_types::{
    CommandWriteEditorFileData, CreateEditorDirReq, CreateEditorFileReq, CreateScratchFileReq, DeleteEditorFileReq,
    ListEditorDirReq, MoveScratchFileReq,
};

fn write_cmd(path: &Path, content: &str) -> CommandWriteEditorFileData {
    CommandWriteEditorFileData {
        path: path.to_string_lossy().into_owned(),
        content: content.to_string(),
        encoding: None,
        bom: None,
        line_ending: None,
        connection: None,
        block_id: None,
    }
}

fn scratch_meta(dir: &Path, id: &str, created_at: u64, saved_to: Option<&str>) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{id}.md")), "").unwrap();
    let meta = serde_json::json!({ "scratch_id": id, "created_at": created_at, "display_name": id, "saved_to": saved_to });
    std::fs::write(dir.join(format!("{id}.md.meta")), meta.to_string()).unwrap();
}

#[test]
fn a_new_name_is_one_plain_component() {
    for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
        assert!(!file_io::is_plain_name(bad), "{bad:?}");
    }
    assert!(file_io::is_plain_name("notes.md"));
    assert!(file_io::is_plain_name(".gitignore"));
}

#[test]
fn a_write_over_the_size_limit_is_refused_before_anything_else() {
    let at_limit = write_cmd(Path::new("/x"), &"a".repeat(10_000_000));
    assert!(file_io::check_write_size(&at_limit).is_ok());
    let over = write_cmd(Path::new("/x"), &"a".repeat(10_000_001));
    assert_eq!(file_io::check_write_size(&over).unwrap_err(), "Content too large (>10MB)");
}

#[test]
fn a_write_lands_under_home_and_never_outside_it() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    let inside = home.path().join("note.txt");
    file_io::write_local_file(home.path(), &write_cmd(&inside, "hello")).unwrap();
    assert_eq!(std::fs::read_to_string(&inside).unwrap(), "hello");

    let elsewhere = outside.path().join("note.txt");
    let err = file_io::write_local_file(home.path(), &write_cmd(&elsewhere, "no")).unwrap_err();
    assert!(err.contains("outside home directory"), "{err}");
    assert!(!elsewhere.exists());
}

#[test]
fn a_lossy_write_is_refused_and_leaves_the_file_alone() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("latin.txt");
    std::fs::write(&path, "kept").unwrap();
    let mut cmd = write_cmd(&path, "snowman \u{2603}");
    cmd.encoding = Some("windows-1252".to_string());
    let err = file_io::write_local_file(home.path(), &cmd).unwrap_err();
    assert!(err.contains("can't be represented"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
}

#[test]
fn a_file_reads_back_with_its_format() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("crlf.txt");
    std::fs::write(&path, "a\r\nb\r\n").unwrap();
    let read = file_io::read_local_file(&path.to_string_lossy()).unwrap();
    assert_eq!(read.line_ending, "crlf");
    assert!(!read.had_decode_errors);
    assert!(!read.read_only);
    assert!(file_io::read_local_file(&home.path().join("missing").to_string_lossy()).unwrap_err().starts_with("readeditorfile:"));
}

#[test]
fn a_listing_puts_folders_first_then_names_without_regard_to_case() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("zeta")).unwrap();
    std::fs::create_dir(home.path().join("Alpha")).unwrap();
    std::fs::write(home.path().join("beta.txt"), "bb").unwrap();
    std::fs::write(home.path().join("Apple.txt"), "a").unwrap();

    let listing = file_io::list_dir(&ListEditorDirReq { path: home.path().to_string_lossy().into_owned() }).unwrap();
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Alpha", "zeta", "Apple.txt", "beta.txt"]);
    assert_eq!(listing.entries[0].size, None);
    assert_eq!(listing.entries[3].size, Some(2));
}

#[test]
fn a_new_file_or_folder_needs_a_plain_unused_name_under_home() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file_req = |parent: &Path, name: &str| CreateEditorFileReq {
        parent_path: parent.to_string_lossy().into_owned(),
        name: name.to_string(),
    };
    let dir_req = |parent: &Path, name: &str| CreateEditorDirReq {
        parent_path: parent.to_string_lossy().into_owned(),
        name: name.to_string(),
    };

    let made = file_io::create_file(home.path(), &file_req(home.path(), "a.txt")).unwrap();
    assert!(Path::new(&made.file_path).is_file());
    assert!(file_io::create_file(home.path(), &file_req(home.path(), "a.txt")).unwrap_err().contains("already exists"));
    assert!(file_io::create_file(home.path(), &file_req(home.path(), "a/b")).unwrap_err().contains("plain filename"));
    assert!(file_io::create_file(home.path(), &file_req(outside.path(), "a.txt")).unwrap_err().contains("outside home"));

    let dir = file_io::create_dir(home.path(), &dir_req(home.path(), "sub")).unwrap();
    assert!(Path::new(&dir.dir_path).is_dir());
    assert!(file_io::create_dir(home.path(), &dir_req(home.path(), "sub")).unwrap_err().contains("already exists"));
    assert!(file_io::create_dir(home.path(), &dir_req(outside.path(), "sub")).unwrap_err().contains("outside home"));
}

#[test]
fn a_delete_keeps_a_full_folder_unless_recursive_and_never_removes_home() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("full");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("f"), "x").unwrap();
    let req = |path: &Path, recursive: bool| DeleteEditorFileReq { path: path.to_string_lossy().into_owned(), recursive };

    assert!(file_io::delete_file(home.path(), &req(&folder, false)).unwrap_err().contains("not empty"));
    assert!(folder.exists());
    assert!(file_io::delete_file(home.path(), &req(home.path(), true)).unwrap_err().contains("outside or is the home"));
    file_io::delete_file(home.path(), &req(&folder, true)).unwrap();
    assert!(!folder.exists());
}

#[cfg(unix)]
#[test]
fn deleting_a_symlink_unlinks_it_and_leaves_its_target() {
    let home = tempfile::tempdir().unwrap();
    let target = home.path().join("target.txt");
    std::fs::write(&target, "keep").unwrap();
    let link = home.path().join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    file_io::delete_file(home.path(), &DeleteEditorFileReq { path: link.to_string_lossy().into_owned(), recursive: false })
        .unwrap();
    assert!(std::fs::symlink_metadata(&link).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
}

#[test]
fn a_scratch_file_is_minted_once_then_a_second_call_gets_another() {
    let home = tempfile::tempdir().unwrap();
    let req = || CreateScratchFileReq { display_name: Some("Draft".to_string()), exclude_scratch_ids: None };

    let first = scratch::create_scratch_file(home.path(), req()).unwrap();
    assert_eq!(first.display_name, "Draft");
    assert!(Path::new(&first.file_path).is_file());
    let dir = Path::new(&first.file_path).parent().unwrap().to_path_buf();
    assert!(dir.join(format!("{}.md.meta", first.scratch_id)).is_file());
    assert!(dir.join(format!("{}.md.claim", first.scratch_id)).is_file());

    let second = scratch::create_scratch_file(home.path(), req()).unwrap();
    assert_ne!(first.scratch_id, second.scratch_id, "a buffer claimed by this process is not handed out again");
}

#[test]
fn an_unclaimed_scratch_buffer_is_handed_out_again_unless_excluded() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".agentmux").join("cache").join("scratch");
    let now = agentmux_common::time::now_ms_u64();
    scratch_meta(&dir, "old-one", now - 5_000, None);
    scratch_meta(&dir, "new-one", now - 1_000, None);

    let excluded = CreateScratchFileReq { display_name: None, exclude_scratch_ids: Some(vec!["new-one".to_string()]) };
    assert_eq!(scratch::create_scratch_file(home.path(), excluded).unwrap().scratch_id, "old-one");

    let open = CreateScratchFileReq { display_name: None, exclude_scratch_ids: None };
    assert_eq!(scratch::create_scratch_file(home.path(), open).unwrap().scratch_id, "new-one");
}

#[test]
fn the_scan_skips_saved_buffers_and_prunes_those_over_thirty_days_old() {
    let dir = tempfile::tempdir().unwrap();
    let now = 100 * 24 * 60 * 60 * 1000_u64;
    scratch_meta(dir.path(), "fresh", now - 1_000, None);
    scratch_meta(dir.path(), "saved", now - 1_000, Some("/somewhere"));
    scratch_meta(dir.path(), "stale", now - 31 * 24 * 60 * 60 * 1000, None);

    let found = scratch::scan_candidates(dir.path(), None, now);
    let ids: Vec<&str> = found.iter().map(|c| c.scratch_id.as_str()).collect();
    assert_eq!(ids, ["fresh"]);
    assert!(!dir.path().join("stale.md").exists() && !dir.path().join("stale.md.meta").exists());
    assert!(dir.path().join("saved.md").exists(), "a saved buffer is skipped, not pruned");
}

#[test]
fn a_claim_from_an_earlier_run_is_replaced_and_one_from_this_run_is_respected() {
    let dir = tempfile::tempdir().unwrap();
    let now = 5_000_u64;
    scratch_meta(dir.path(), "a", now, None);
    let candidates = || scratch::scan_candidates(dir.path(), None, now);

    std::fs::write(dir.path().join("a.md.claim"), "token-of-a-dead-run").unwrap();
    let got = scratch::claim_most_recent(dir.path(), candidates(), "this-run").unwrap();
    assert_eq!(got.0, "a");
    assert_eq!(std::fs::read_to_string(dir.path().join("a.md.claim")).unwrap(), "this-run");

    assert!(scratch::claim_most_recent(dir.path(), candidates(), "this-run").is_none());
}

#[test]
fn saving_a_scratch_buffer_moves_it_and_removes_its_sidecars() {
    let home = tempfile::tempdir().unwrap();
    let made = scratch::create_scratch_file(home.path(), CreateScratchFileReq::default()).unwrap();
    std::fs::write(&made.file_path, "draft").unwrap();
    let dest = home.path().join("docs").join("saved.md");

    let moved = scratch::move_scratch_file(
        home.path(),
        &MoveScratchFileReq { scratch_id: made.scratch_id.clone(), destination_path: dest.to_string_lossy().into_owned() },
    )
    .unwrap();
    assert!(moved.file_path.ends_with("saved.md"));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "draft");
    let dir = Path::new(&made.file_path).parent().unwrap();
    for ext in ["md", "md.meta", "md.claim"] {
        assert!(!dir.join(format!("{}.{ext}", made.scratch_id)).exists(), "{ext}");
    }
}

#[test]
fn saving_a_scratch_buffer_refuses_a_bad_id_a_dotdot_path_and_a_place_outside_home() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let made = scratch::create_scratch_file(home.path(), CreateScratchFileReq::default()).unwrap();
    let req = |id: &str, dest: &Path| MoveScratchFileReq {
        scratch_id: id.to_string(),
        destination_path: dest.to_string_lossy().into_owned(),
    };

    assert!(scratch::move_scratch_file(home.path(), &req("../x", &home.path().join("d"))).unwrap_err().contains("invalid scratch_id"));
    assert!(scratch::move_scratch_file(home.path(), &req("nope", &home.path().join("d"))).unwrap_err().contains("not found"));
    assert!(scratch::move_scratch_file(home.path(), &req(&made.scratch_id, &outside.path().join("d")))
        .unwrap_err()
        .contains("only inside your home folder"));
    let dotdot = home.path().join("a").join("..").join("..").join("evil");
    assert!(scratch::move_scratch_file(home.path(), &req(&made.scratch_id, &dotdot)).unwrap_err().contains("“..”"));
    assert!(Path::new(&made.file_path).exists(), "a refused save leaves the buffer in place");
}

#[test]
fn saving_a_scratch_buffer_never_replaces_a_file_and_follows_the_name_rules() {
    let home = tempfile::tempdir().unwrap();
    let made = scratch::create_scratch_file(home.path(), CreateScratchFileReq::default()).unwrap();
    std::fs::write(&made.file_path, "draft").unwrap();
    let req = |dest: &Path| MoveScratchFileReq { scratch_id: made.scratch_id.clone(), destination_path: dest.to_string_lossy().into_owned() };

    let taken = home.path().join("taken.md");
    std::fs::write(&taken, "keep").unwrap();
    let err = scratch::move_scratch_file(home.path(), &req(&taken)).unwrap_err();
    assert_eq!(err, "Something named “taken.md” already exists here.");
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "keep");

    // A name rule is refused before any folder is made for it.
    let err = scratch::move_scratch_file(home.path(), &req(&home.path().join("new-folder").join(".."))).unwrap_err();
    assert!(err.contains("“..”"), "{err}");
    #[cfg(windows)]
    {
        let err = scratch::move_scratch_file(home.path(), &req(&home.path().join("new-folder").join("nul.txt"))).unwrap_err();
        assert!(err.contains("reserved by Windows"), "{err}");
        assert!(!home.path().join("new-folder").exists());
    }
    assert_eq!(std::fs::read_to_string(&made.file_path).unwrap(), "draft", "every refusal leaves the buffer in place");
}

#[test]
fn a_save_that_fails_part_way_leaves_no_file_and_one_that_meets_a_file_leaves_it() {
    struct Fails;
    impl std::io::Read for Fails {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.md");
    let err = scratch::copy_to_new_file(&mut Fails, &dest).unwrap_err();
    assert_eq!(scratch::save_error(&err, "out.md"), "There isn't enough space to save it there.");
    assert!(!dest.exists(), "nothing half-written is left");

    std::fs::write(&dest, "keep").unwrap();
    let err = scratch::copy_to_new_file(&mut "new".as_bytes(), &dest).unwrap_err();
    assert_eq!(scratch::save_error(&err, "out.md"), "Something named “out.md” already exists here.");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "keep", "created exclusively: never replaced, never removed");

    let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    assert_eq!(scratch::save_error(&denied, "out.md"), "You don't have permission to save there.");
}
