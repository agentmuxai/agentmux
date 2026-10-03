// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::collections::HashSet;

fn list(store: &Mutex<CursorStore>, path: &str, cursor: Option<String>, limit: Option<u32>, now: Instant) -> FsListResult {
    list_page_in(store, &FsListReq { path: path.to_string(), cursor, limit }, now)
}

fn dir_with_files(n: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..n {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "x").unwrap();
    }
    dir
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Try to create a file symlink; Windows needs Developer Mode or elevation,
/// so a test that needs one is skipped where it can't be made.
fn try_symlink_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_file(target, link).is_ok();
}

// ── Listing ──────────────────────────────────────────────────────────────

#[test]
fn listing_pages_through_a_folder_with_a_cursor() {
    let dir = dir_with_files(25);
    let store = Mutex::new(CursorStore::default());
    let now = Instant::now();

    let p1 = list(&store, &s(dir.path()), None, Some(10), now);
    assert!(p1.error.is_none(), "{:?}", p1.error);
    assert_eq!(p1.entries.len(), 10);
    let c1 = p1.cursor.clone().expect("more to come");

    let p2 = list(&store, &s(dir.path()), Some(c1.clone()), Some(10), now);
    assert_eq!(p2.entries.len(), 10);
    assert_eq!(p2.path, p1.path, "every page names the canonical folder");
    let c2 = p2.cursor.clone().expect("more to come");

    let p3 = list(&store, &s(dir.path()), Some(c2), Some(10), now);
    assert_eq!(p3.entries.len(), 5);
    assert!(p3.cursor.is_none(), "the last page has no cursor");

    let names: HashSet<String> = [p1, p2, p3].iter().flat_map(|p| p.entries.iter().map(|e| e.name.clone())).collect();
    assert_eq!(names.len(), 25, "every entry exactly once");
    assert_eq!(store.lock().unwrap().len(), 0, "a finished listing holds no cursor");

    // A used cursor is consumed: asking for the same page again is expired.
    let again = list(&store, &s(dir.path()), Some(c1), Some(10), now);
    assert_eq!(again.error.unwrap().kind, FsErrorKind::Expired);
}

#[test]
fn a_page_that_exactly_fills_the_limit_ends_without_a_cursor() {
    let dir = dir_with_files(10);
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, Some(10), Instant::now());
    assert_eq!(p.entries.len(), 10);
    assert!(p.cursor.is_none(), "nothing left, so no empty extra page");
}

#[test]
fn an_idle_cursor_expires() {
    let dir = dir_with_files(5);
    let store = Mutex::new(CursorStore::default());
    let now = Instant::now();
    let p1 = list(&store, &s(dir.path()), None, Some(2), now);
    let cursor = p1.cursor.unwrap();

    let late = now + CURSOR_TTL + Duration::from_secs(1);
    let p2 = list(&store, &s(dir.path()), Some(cursor), Some(2), late);
    let err = p2.error.expect("expired");
    assert_eq!(err.kind, FsErrorKind::Expired);
    assert!(p2.entries.is_empty() && p2.cursor.is_none());
}

#[test]
fn an_unknown_cursor_is_expired_not_an_rpc_error() {
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, "~", Some("no-such-cursor".into()), None, Instant::now());
    assert_eq!(p.error.unwrap().kind, FsErrorKind::Expired);
}

#[test]
fn the_sweep_drops_idle_cursors() {
    let dir = dir_with_files(5);
    let store = Mutex::new(CursorStore::default());
    let now = Instant::now();
    let _ = list(&store, &s(dir.path()), None, Some(1), now);
    let _ = list(&store, &s(dir.path()), None, Some(1), now);
    assert_eq!(store.lock().unwrap().len(), 2);
    store.lock().unwrap().sweep(now + Duration::from_secs(5));
    assert_eq!(store.lock().unwrap().len(), 2, "not idle long enough");
    store.lock().unwrap().sweep(now + CURSOR_TTL + Duration::from_secs(1));
    assert_eq!(store.lock().unwrap().len(), 0);
}

#[test]
fn a_zero_limit_still_makes_progress() {
    let dir = dir_with_files(3);
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, Some(0), Instant::now());
    assert_eq!(p.entries.len(), 1);
}

#[test]
fn a_folder_that_cannot_be_opened_is_an_answer_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let store = Mutex::new(CursorStore::default());

    let missing = list(&store, &s(&dir.path().join("nope")), None, None, Instant::now());
    assert_eq!(missing.error.as_ref().unwrap().kind, FsErrorKind::NotFound);
    assert!(missing.entries.is_empty() && missing.cursor.is_none());

    let file = dir.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    let not_dir = list(&store, &s(&file), None, None, Instant::now());
    assert_eq!(not_dir.error.unwrap().kind, FsErrorKind::NotADirectory);

    let relative = list(&store, "some/relative/dir", None, None, Instant::now());
    assert_eq!(relative.error.unwrap().kind, FsErrorKind::Other);
}

#[test]
fn listed_paths_are_in_display_form() {
    let dir = dir_with_files(1);
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, None, Instant::now());
    assert!(!p.path.starts_with(r"\\?\"), "verbatim prefix leaked: {}", p.path);
    assert!(Path::new(&p.path).is_absolute());
}

#[test]
fn entries_describe_files_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.bin"), [0u8; 42]).unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, None, Instant::now());
    let by_name = |n: &str| p.entries.iter().find(|e| e.name == n).unwrap().clone();

    let file = by_name("data.bin");
    assert!(!file.is_dir && !file.is_symlink);
    assert_eq!(file.size, Some(42));
    assert!(file.mtime.is_some());
    assert!(file.error.is_none());

    let sub = by_name("sub");
    assert!(sub.is_dir);
    assert_eq!(sub.size, None, "a folder has no size");
}

#[test]
fn a_broken_link_is_a_row_with_an_error_and_the_page_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fine.txt"), "x").unwrap();
    if !try_symlink_file(&dir.path().join("gone.txt"), &dir.path().join("dangling")) {
        return;
    }
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, None, Instant::now());
    assert!(p.error.is_none(), "one bad entry must not fail the folder");
    assert_eq!(p.entries.len(), 2);

    let link = p.entries.iter().find(|e| e.name == "dangling").unwrap();
    assert!(link.is_symlink);
    assert!(link.error.is_some(), "a dangling link carries an error");
    assert!(link.size.is_none() && link.mtime.is_none());
    assert!(link.link_target.as_deref().unwrap_or("").ends_with("gone.txt"));

    let fine = p.entries.iter().find(|e| e.name == "fine.txt").unwrap();
    assert!(fine.error.is_none());
}

#[test]
fn a_dot_name_is_hidden() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "x").unwrap();
    std::fs::write(dir.path().join("visible"), "x").unwrap();
    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, None, Instant::now());
    assert!(p.entries.iter().find(|e| e.name == ".env").unwrap().hidden);
    assert!(!p.entries.iter().find(|e| e.name == "visible").unwrap().hidden);
}

#[cfg(windows)]
#[test]
fn the_windows_hidden_and_system_attributes_hide() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
    let dir = tempfile::tempdir().unwrap();
    let set = |name: &str, attrs: u32| {
        let p = dir.path().join(name);
        std::fs::write(&p, "x").unwrap();
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated UTF-16 path that outlives the call.
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), attrs) }, 0);
    };
    set("hidden.txt", FILE_ATTRIBUTE_HIDDEN);
    set("system.txt", FILE_ATTRIBUTE_SYSTEM);
    std::fs::write(dir.path().join("plain.txt"), "x").unwrap();

    let store = Mutex::new(CursorStore::default());
    let p = list(&store, &s(dir.path()), None, None, Instant::now());
    let hidden = |n: &str| p.entries.iter().find(|e| e.name == n).unwrap().hidden;
    assert!(hidden("hidden.txt"));
    assert!(hidden("system.txt"));
    assert!(!hidden("plain.txt"));

    // Let the temp dir clean itself up.
    for n in ["hidden.txt", "system.txt"] {
        let wide: Vec<u16> = dir.path().join(n).as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: as above. 0x80 = FILE_ATTRIBUTE_NORMAL.
        unsafe { SetFileAttributesW(wide.as_ptr(), 0x80) };
    }
}

// ── Names ────────────────────────────────────────────────────────────────

#[test]
fn name_rules_table() {
    // (name, valid everywhere, valid on Windows)
    let table: &[(&str, bool, bool)] = &[
        ("report.txt", true, true),
        (".gitignore", true, true),
        ("with space.md", true, true),
        ("日本語.txt", true, true),
        ("", false, false),
        ("   ", false, false),
        (".", false, false),
        ("..", false, false),
        ("a/b", false, false),
        ("a\\b", false, false),
        ("nul\0byte", false, false),
        ("trailing.", true, false),
        ("trailing ", true, false),
        ("CON", true, false),
        ("con.txt", true, false),
        ("Nul.tar.gz", true, false),
        ("com1", true, false),
        ("LPT9.log", true, false),
        ("com¹", true, false),
        ("conin$", true, false),
        ("console", true, true),
        ("com10", true, true),
        ("what?", true, false),
        ("a:b", true, false),
        ("a<b", true, false),
        ("pipe|name", true, false),
        ("star*", true, false),
        ("quote\"d", true, false),
        ("tab\there", true, false),
    ];
    for &(name, unix_ok, windows_ok) in table {
        assert_eq!(validate_name_rules(name, false).is_ok(), unix_ok, "non-Windows rules for {name:?}");
        assert_eq!(validate_name_rules(name, true).is_ok(), windows_ok, "Windows rules for {name:?}");
    }
    assert!(validate_name_rules(&"x".repeat(256), false).is_err(), "too long");
    assert!(validate_name_rules(&"x".repeat(255), false).is_ok());
}

// ── Mutation safety ──────────────────────────────────────────────────────

/// An absolute path for the protection table, on a drive on Windows.
fn abs(p: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:{}", p.replace('/', "\\")))
    } else {
        PathBuf::from(p)
    }
}

#[test]
fn protected_paths_are_refused() {
    let protect = ProtectedPaths::for_test(
        &abs("/users/me"),
        &abs("/users/me/.agentmux"),
        &[&abs("/opt/agentmux")],
        &[&abs("/usr"), &abs("/windows")],
        &[&abs("/tmp")],
    );
    let ok = |p: &str| protect.check(&abs(p)).is_ok();

    assert!(!ok("/"), "a filesystem root");
    assert!(!ok("/users/me"), "the home folder itself");
    assert!(!ok("/users"), "a folder containing home");
    assert!(ok("/users/me/notes.txt"));
    assert!(ok("/users/me/projects/app/src"));

    assert!(!ok("/users/me/.agentmux"), "the data folder");
    assert!(!ok("/users/me/.agentmux/db/store.db"), "AgentMux state");
    assert!(!ok("/users/me/.agentmux/agents"), "the workspaces folder");
    assert!(!ok("/users/me/.agentmux/agents/korp"), "a workspace itself");
    assert!(ok("/users/me/.agentmux/agents/korp/notes.md"), "inside a workspace");

    assert!(!ok("/opt/agentmux/bin/agentmux-srv"), "the installation");
    assert!(!ok("/opt"), "a folder containing the installation");

    assert!(!ok("/usr/lib/libc.so"), "a system folder");
    assert!(!ok("/windows/system32"), "a system folder");
    assert!(ok("/tmp/scratch.txt"), "temp is exempt");
    assert!(ok("/srv/data/file"), "an ordinary folder outside home");
}

#[cfg(windows)]
#[test]
fn protection_ignores_case_and_verbatim_prefixes_on_windows() {
    let protect = ProtectedPaths::for_test(&abs("/users/me"), &abs("/users/me/.agentmux"), &[], &[&abs("/windows")], &[]);
    assert!(protect.check(Path::new(r"C:\USERS\ME")).is_err());
    assert!(protect.check(Path::new(r"\\?\C:\Users\Me")).is_err());
    assert!(protect.check(Path::new(r"\\?\c:\WINDOWS\System32\drivers")).is_err());
    assert!(protect.check(Path::new(r"\\?\C:\Users\Me\.AgentMux\Agents\korp\x.txt")).is_ok());
}

/// A WSL distro's files through `\\wsl.localhost` (`remote::wsl_fs`): its own
/// system and home folders are protected, and the rest of it can be changed.
#[test]
fn a_wsl_distro_has_its_own_protected_folders() {
    let protect = ProtectedPaths::for_test(&abs("/users/me"), &abs("/users/me/.agentmux"), &[], &[&abs("/windows")], &[]);
    let ok = |p: &str| protect.check(Path::new(p)).is_ok();
    assert!(!ok(r"\\wsl.localhost\Ubuntu\usr\bin\ls"), "a distro's system folder");
    assert!(!ok(r"\\?\UNC\wsl.localhost\Ubuntu\etc"), "in the form canonicalize returns");
    assert!(!ok(r"\\wsl.localhost\Ubuntu\mnt"), "where the drives are mounted");
    assert!(!ok(r"\\wsl.localhost\Ubuntu\home\u"), "a distro user's home folder");
    assert!(!ok(r"\\wsl.localhost\Ubuntu\home"));
    assert!(ok(r"\\wsl.localhost\Ubuntu\home\u\proj\main.rs"));
    assert!(ok(r"\\wsl$\Ubuntu\srv\app"));
}

/// A distro's `/mnt/c` is the C: drive: changing a file there changes the
/// Windows file, so the Windows rules decide, not the distro's.
#[cfg(windows)]
#[test]
fn a_wsl_drive_mount_gets_the_windows_rules() {
    let protect = ProtectedPaths::for_test(&abs("/users/me"), &abs("/users/me/.agentmux"), &[], &[&abs("/windows")], &[]);
    let ok = |p: &str| protect.check(Path::new(p)).is_ok();
    assert!(!ok(r"\\wsl.localhost\Ubuntu\mnt\c\Windows\System32"));
    assert!(!ok(r"\\wsl.localhost\Ubuntu\mnt\c\Users\me"), "the Windows home folder");
    assert!(!ok(r"\\wsl.localhost\Ubuntu\mnt\c"), "a drive root");
    assert!(ok(r"\\wsl.localhost\Ubuntu\mnt\c\Users\me\notes.txt"));
}

#[test]
fn a_root_is_refused_before_anything_else() {
    let err = resolve_entry_path(&s(&abs("/"))).unwrap_err();
    assert_eq!(err, ROOT_REFUSAL);
}

#[test]
fn an_entry_path_keeps_a_links_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real.txt");
    std::fs::write(&target, "x").unwrap();
    let link = dir.path().join("alias.txt");
    if !try_symlink_file(&target, &link) {
        return;
    }
    let resolved = resolve_entry_path(&s(&link)).unwrap();
    assert_eq!(resolved.file_name().unwrap(), "alias.txt", "the link itself, not its target");
}

// ── Mutations (in the temp folder, which the policy allows) ──────────────

#[test]
fn rename_moves_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();

    let err = rename(&FsRenameReq { path: s(&dir.path().join("a.txt")), new_name: "b.txt".into() }).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(dir.path().join("b.txt")).unwrap(), "b");

    let out = rename(&FsRenameReq { path: s(&dir.path().join("a.txt")), new_name: "c.txt".into() }).unwrap();
    assert!(out.new_path.ends_with("c.txt"));
    assert!(!out.new_path.starts_with(r"\\?\"));
    assert_eq!(std::fs::read_to_string(dir.path().join("c.txt")).unwrap(), "a");
    assert!(!dir.path().join("a.txt").exists());

    let bad = rename(&FsRenameReq { path: s(&dir.path().join("c.txt")), new_name: "../escape.txt".into() });
    assert!(bad.is_err(), "a separator in the new name is refused");
}

#[test]
fn a_case_only_rename_changes_the_case() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("readme.md"), "x").unwrap();
    let out = rename(&FsRenameReq { path: s(&dir.path().join("readme.md")), new_name: "README.md".into() }).unwrap();
    assert!(out.new_path.ends_with("README.md"));
    let names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["README.md".to_string()], "exactly one entry, with the new case, and no temp name left");
}

#[test]
fn create_makes_new_entries_and_refuses_taken_names() {
    let dir = tempfile::tempdir().unwrap();
    let parent = s(dir.path());

    let f = create(&FsCreateReq { parent: parent.clone(), name: "new.txt".into(), kind: FsCreateKind::File }).unwrap();
    assert_eq!(std::fs::read(&f.path).unwrap().len(), 0, "an empty file");
    let d = create(&FsCreateReq { parent: parent.clone(), name: "folder".into(), kind: FsCreateKind::Dir }).unwrap();
    assert!(Path::new(&d.path).is_dir());

    std::fs::write(dir.path().join("new.txt"), "keep").unwrap();
    let err = create(&FsCreateReq { parent: parent.clone(), name: "new.txt".into(), kind: FsCreateKind::File }).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(dir.path().join("new.txt")).unwrap(), "keep", "never truncated");

    assert!(create(&FsCreateReq { parent, name: "..".into(), kind: FsCreateKind::Dir }).is_err());
}

#[test]
fn permanent_delete_removes_files_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(sub.join("inner")).unwrap();
    std::fs::write(sub.join("inner").join("g.txt"), "x").unwrap();

    delete_permanently(&s(&file)).unwrap();
    delete_permanently(&s(&sub)).unwrap();
    assert!(!file.exists() && !sub.exists());
    assert!(delete_permanently(&s(&file)).is_err(), "already gone");
}
