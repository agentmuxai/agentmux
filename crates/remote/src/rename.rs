// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A rename that never replaces what is already at the new name.
//!
//! `std::fs::rename` replaces an existing destination on every platform, so "check that the name is free, then
//! rename" lets something created in between be replaced, and its content is gone. [`rename_no_replace`] makes
//! the check and the rename one step:
//!
//! - Linux: `renameat2` with `RENAME_NOREPLACE`.
//! - macOS: `renamex_np` with `RENAME_EXCL`.
//! - Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`.
//!
//! An old kernel, or a filesystem that has no such rename (NFS, some FUSE mounts, FAT and exFAT volumes on
//! macOS), is renamed in steps whose first one takes the new name and fails if it is taken:
//!
//! - A folder: an empty folder is made at the new name, and the folder is renamed over it. A rename over a
//!   folder that got something in it meanwhile fails, so nothing put there is lost.
//! - Anything else: a hard link is made at the new name, then the old name is removed.
//! - Where hard links can't be made either: an empty file is made at the new name, exclusively, and the file
//!   is renamed over it. Whatever was there already, and whatever anything that does not overwrite files
//!   creates there meanwhile, is kept. Only a program that overwrites that empty file in the instant before
//!   the rename loses what it wrote: such a filesystem has nothing that closes that last gap.
//!
//! Either way the caller gets `AlreadyExists` for a name that is taken, and the person is told the name
//! already exists.
//!
//! srv renames local files through this too, so a host and this computer follow one rule.

use std::io;
use std::path::Path;

/// Rename `from` to `to`, failing with `AlreadyExists` instead of replacing an existing `to` (a file, a
/// folder, even an empty one, or a link, even a dangling one).
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        move_no_replace(from, to)
    }
    #[cfg(unix)]
    {
        match one_step(from, to) {
            Some(result) => result,
            None => in_steps(from, to),
        }
    }
}

#[cfg(windows)]
fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // Declared here rather than taken from a crate: the helper stays std and libc only.
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
    let (f, t) = (wide(from), wide(to));
    // Flags 0: without MOVEFILE_REPLACE_EXISTING an existing destination fails with ERROR_ALREADY_EXISTS,
    // which std maps to AlreadyExists.
    // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call.
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn c_path(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::CString::new(p.as_os_str().as_bytes())?)
}

/// The system's own no-replace rename. `None` where this kernel or this filesystem has none.
#[cfg(target_os = "linux")]
fn one_step(from: &Path, to: &Path) -> Option<io::Result<()>> {
    // The kernel's value, written out: libc does not name it for every C library.
    const RENAME_NOREPLACE: libc::c_uint = 1;
    let (f, t) = match (c_path(from), c_path(to)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
    };
    // The system call itself rather than the C library's wrapper, which glibc has only from 2.28 on.
    // SAFETY: both pointers are NUL-terminated strings that outlive the call.
    let r = unsafe {
        libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), RENAME_NOREPLACE)
    };
    if r == 0 {
        return Some(Ok(()));
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        // A kernel before 3.15, or a filesystem without RENAME_NOREPLACE.
        Some(libc::ENOSYS) | Some(libc::EINVAL) => None,
        _ => Some(Err(err)),
    }
}

#[cfg(target_os = "macos")]
fn one_step(from: &Path, to: &Path) -> Option<io::Result<()>> {
    let (f, t) = match (c_path(from), c_path(to)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
    };
    // SAFETY: both pointers are NUL-terminated strings that outlive the call.
    if unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) } == 0 {
        return Some(Ok(()));
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        // A filesystem without RENAME_EXCL (FAT and exFAT, some network volumes).
        Some(libc::ENOTSUP) | Some(libc::EINVAL) => None,
        _ => Some(Err(err)),
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn one_step(_from: &Path, _to: &Path) -> Option<io::Result<()>> {
    None
}

/// The rename where the system has no one-step one: the new name is taken first, by a step that fails if it
/// is taken already (the module's doc says which, and what each keeps).
#[cfg(unix)]
fn in_steps(from: &Path, to: &Path) -> io::Result<()> {
    // Not followed: a link to a folder is a link, and is renamed as a file is.
    if std::fs::symlink_metadata(from)?.is_dir() {
        return dir_by_claim(from, to);
    }
    match by_link(from, to) {
        // The name is taken, or there is nothing to rename: the answer, whatever the filesystem can do.
        Err(e) if matches!(e.kind(), io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound) => Err(e),
        // No hard link could be made here (the filesystem has none, or this user may not link this file).
        // Any other failure shows again below, as the claim's own or the rename's.
        Err(_) => by_placeholder(from, to),
        Ok(()) => Ok(()),
    }
}

/// A file (or a link, or anything that is not a folder) by a hard link at the new name, which fails if the
/// name is taken, then the old name's removal.
#[cfg(unix)]
fn by_link(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::hard_link(from, to)?;
    if let Err(e) = std::fs::remove_file(from) {
        // Both names now: take the new one away again rather than leave the file under two.
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// A folder by an empty folder made at the new name, which fails if the name is taken, and a rename over
/// it. A rename over a folder that is no longer empty fails, so what was put in it meanwhile stays.
#[cfg(unix)]
fn dir_by_claim(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir(to)?;
    std::fs::rename(from, to).map_err(|e| {
        // Only an empty folder goes: one that was filled meanwhile is someone else's now.
        let _ = std::fs::remove_dir(to);
        match e.raw_os_error() {
            Some(libc::ENOTEMPTY) | Some(libc::EEXIST) => io::Error::new(io::ErrorKind::AlreadyExists, e),
            _ => e,
        }
    })
}

/// A file by an empty file made at the new name, exclusively, which fails if the name is taken, and a
/// rename over it. For a filesystem with neither a no-replace rename nor hard links.
#[cfg(unix)]
fn by_placeholder(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let claim = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    // Still the empty file made here, and not something written or moved over it since.
    let still_the_claim = || match (claim.metadata(), std::fs::symlink_metadata(to)) {
        (Ok(made), Ok(there)) => made.len() == 0 && made.dev() == there.dev() && made.ino() == there.ino(),
        _ => false,
    };
    if !still_the_claim() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "the new name was taken meanwhile"));
    }
    std::fs::rename(from, to).inspect_err(|_| {
        if still_the_claim() {
            let _ = std::fs::remove_file(to);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Barrier};

    type Rename = fn(&Path, &Path) -> io::Result<()>;

    /// Every way a file is renamed on this platform: the whole function, and on Unix each of the steps it
    /// falls back to, which no filesystem a test runs on would reach.
    fn file_ways() -> Vec<(&'static str, Rename)> {
        let mut ways: Vec<(&'static str, Rename)> = vec![("rename_no_replace", rename_no_replace)];
        #[cfg(unix)]
        ways.extend([
            ("in_steps", in_steps as Rename),
            ("by_link", by_link as Rename),
            ("by_placeholder", by_placeholder as Rename),
        ]);
        ways
    }

    fn dir_ways() -> Vec<(&'static str, Rename)> {
        let mut ways: Vec<(&'static str, Rename)> = vec![("rename_no_replace", rename_no_replace)];
        #[cfg(unix)]
        ways.extend([("in_steps", in_steps as Rename), ("dir_by_claim", dir_by_claim as Rename)]);
        ways
    }

    #[test]
    fn a_file_moves_to_a_free_name_with_its_content() {
        for (way, rename) in file_ways() {
            let dir = tempfile::tempdir().unwrap();
            let (a, b) = (dir.path().join("a"), dir.path().join("b"));
            fs::write(&a, "one").unwrap();
            rename(&a, &b).unwrap_or_else(|e| panic!("{way}: {e}"));
            assert!(!a.exists(), "{way}");
            assert_eq!(fs::read_to_string(&b).unwrap(), "one", "{way}");
        }
    }

    #[test]
    fn a_taken_name_is_refused_and_both_files_keep_their_content() {
        for (way, rename) in file_ways() {
            let dir = tempfile::tempdir().unwrap();
            let (a, b) = (dir.path().join("a"), dir.path().join("b"));
            fs::write(&a, "one").unwrap();
            fs::write(&b, "two").unwrap();
            let err = rename(&a, &b).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{way}: {err}");
            assert_eq!(fs::read_to_string(&a).unwrap(), "one", "{way}");
            assert_eq!(fs::read_to_string(&b).unwrap(), "two", "{way}");
        }
    }

    #[test]
    fn a_file_is_not_renamed_over_a_folder_even_an_empty_one() {
        for (way, rename) in file_ways() {
            let dir = tempfile::tempdir().unwrap();
            let (a, b) = (dir.path().join("a"), dir.path().join("b"));
            fs::write(&a, "one").unwrap();
            fs::create_dir(&b).unwrap();
            let err = rename(&a, &b).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{way}: {err}");
            assert!(a.is_file() && b.is_dir(), "{way}");
        }
    }

    #[test]
    fn a_missing_file_is_not_found_and_leaves_nothing_at_the_new_name() {
        for (way, rename) in file_ways() {
            let dir = tempfile::tempdir().unwrap();
            let (a, b) = (dir.path().join("a"), dir.path().join("b"));
            let err = rename(&a, &b).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound, "{way}: {err}");
            assert!(fs::symlink_metadata(&b).is_err(), "{way}: something was left at the new name");
        }
    }

    #[test]
    fn a_folder_moves_with_what_is_in_it_and_never_over_another_folder() {
        for (way, rename) in dir_ways() {
            let dir = tempfile::tempdir().unwrap();
            let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
            fs::create_dir(&a).unwrap();
            fs::write(a.join("inside"), "one").unwrap();
            // An empty folder is in the way as much as a full one: a plain rename would replace it.
            fs::create_dir(&b).unwrap();
            let err = rename(&a, &b).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{way}: {err}");
            assert!(a.join("inside").is_file() && b.is_dir(), "{way}");

            rename(&a, &c).unwrap_or_else(|e| panic!("{way}: {e}"));
            assert!(!a.exists(), "{way}");
            assert_eq!(fs::read_to_string(c.join("inside")).unwrap(), "one", "{way}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_stays_a_link_and_a_dangling_link_takes_its_name() {
        use std::os::unix::fs::symlink;
        // Not through the empty file: a link made that way is what the test's filesystem never needs.
        for (way, rename) in [("rename_no_replace", rename_no_replace as Rename), ("by_link", by_link as Rename)] {
            let dir = tempfile::tempdir().unwrap();
            let (a, b, link, dangling) =
                (dir.path().join("a"), dir.path().join("b"), dir.path().join("link"), dir.path().join("dangling"));
            fs::write(&a, "one").unwrap();
            symlink(&a, &link).unwrap();
            symlink(dir.path().join("nowhere"), &dangling).unwrap();

            let err = rename(&a, &dangling).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{way}: {err}");
            assert!(fs::symlink_metadata(&dangling).unwrap().file_type().is_symlink(), "{way}");

            rename(&link, &b).unwrap_or_else(|e| panic!("{way}: {e}"));
            assert!(fs::symlink_metadata(&b).unwrap().file_type().is_symlink(), "{way}");
            assert_eq!(fs::read_to_string(&a).unwrap(), "one", "{way}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_filled_while_it_is_renamed_over_keeps_what_was_put_in_it() {
        // The claim's own folder, filled before the rename: the state a rename in steps can meet.
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(b.join("theirs"), "two").unwrap();
        let err = fs::rename(&a, &b).unwrap_err();
        assert!(
            matches!(err.raw_os_error(), Some(libc::ENOTEMPTY) | Some(libc::EEXIST)),
            "a rename over a folder with something in it must fail: {err}"
        );
        assert_eq!(fs::read_to_string(b.join("theirs")).unwrap(), "two");
    }

    /// Two renames to one new name, started together, many times over: one wins, the other is told the name
    /// is taken, and each file's content is where its rename's answer says it is.
    #[test]
    fn two_renames_to_one_name_started_together_have_one_winner_and_lose_nothing() {
        for (way, rename) in file_ways() {
            let dir = tempfile::tempdir().unwrap();
            for round in 0..100 {
                let (a, b, to) = (dir.path().join("a"), dir.path().join("b"), dir.path().join(format!("to{round}")));
                fs::write(&a, "one").unwrap();
                fs::write(&b, "two").unwrap();
                let start = Arc::new(Barrier::new(2));
                let run = |from: std::path::PathBuf| {
                    let (start, to) = (start.clone(), to.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        rename(&from, &to)
                    })
                };
                let (ra, rb) = (run(a.clone()), run(b.clone()));
                let (ra, rb) = (ra.join().unwrap(), rb.join().unwrap());
                let (won, lost, lost_at, lost_text, won_text) = match (&ra, &rb) {
                    (Ok(()), Err(e)) => (&a, e, &b, "two", "one"),
                    (Err(e), Ok(())) => (&b, e, &a, "one", "two"),
                    other => panic!("{way} round {round}: exactly one must win: {other:?}"),
                };
                assert_eq!(lost.kind(), io::ErrorKind::AlreadyExists, "{way} round {round}: {lost}");
                assert!(!won.exists(), "{way} round {round}: the winner's old name is still there");
                assert_eq!(fs::read_to_string(&to).unwrap(), won_text, "{way} round {round}");
                assert_eq!(fs::read_to_string(lost_at).unwrap(), lost_text, "{way} round {round}");
                fs::remove_file(lost_at).unwrap();
            }
        }
    }

    /// A file made at the new name while a rename to it is under way: whichever comes first, the other is
    /// refused, and the file that was made is never replaced.
    #[test]
    fn a_file_made_at_the_new_name_while_a_rename_is_under_way_is_never_replaced() {
        for (way, rename) in file_ways() {
            let dir = tempfile::tempdir().unwrap();
            for round in 0..100 {
                let (a, to) = (dir.path().join("a"), dir.path().join(format!("to{round}")));
                fs::write(&a, "renamed").unwrap();
                let start = Arc::new(Barrier::new(2));
                let renamer = {
                    let (start, a, to) = (start.clone(), a.clone(), to.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        rename(&a, &to)
                    })
                };
                let maker = {
                    let (start, to) = (start.clone(), to.clone());
                    std::thread::spawn(move || {
                        use std::io::Write;
                        start.wait();
                        fs::OpenOptions::new().write(true).create_new(true).open(&to)?.write_all(b"made")
                    })
                };
                let (renamed, made) = (renamer.join().unwrap(), maker.join().unwrap());
                match (&renamed, &made) {
                    (Ok(()), Err(e)) => {
                        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{way} round {round}: {e}");
                        assert_eq!(fs::read_to_string(&to).unwrap(), "renamed", "{way} round {round}");
                        assert!(!a.exists(), "{way} round {round}");
                    }
                    (Err(e), Ok(())) => {
                        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{way} round {round}: {e}");
                        assert_eq!(fs::read_to_string(&to).unwrap(), "made", "{way} round {round}");
                        assert_eq!(fs::read_to_string(&a).unwrap(), "renamed", "{way} round {round}");
                    }
                    other => panic!("{way} round {round}: exactly one must get the name: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn two_folders_renamed_to_one_name_together_have_one_winner_and_keep_their_files() {
        for (way, rename) in dir_ways() {
            let dir = tempfile::tempdir().unwrap();
            for round in 0..50 {
                let (a, b, to) = (dir.path().join("a"), dir.path().join("b"), dir.path().join(format!("to{round}")));
                for (d, text) in [(&a, "one"), (&b, "two")] {
                    fs::create_dir(d).unwrap();
                    fs::write(d.join("inside"), text).unwrap();
                }
                let start = Arc::new(Barrier::new(2));
                let run = |from: std::path::PathBuf| {
                    let (start, to) = (start.clone(), to.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        rename(&from, &to)
                    })
                };
                let (ra, rb) = (run(a.clone()), run(b.clone()));
                let (ra, rb) = (ra.join().unwrap(), rb.join().unwrap());
                let (lost, lost_at, lost_text, won_text) = match (&ra, &rb) {
                    (Ok(()), Err(e)) => (e, &b, "two", "one"),
                    (Err(e), Ok(())) => (e, &a, "one", "two"),
                    other => panic!("{way} round {round}: exactly one must win: {other:?}"),
                };
                assert_eq!(lost.kind(), io::ErrorKind::AlreadyExists, "{way} round {round}: {lost}");
                assert_eq!(fs::read_to_string(to.join("inside")).unwrap(), won_text, "{way} round {round}");
                assert_eq!(fs::read_to_string(lost_at.join("inside")).unwrap(), lost_text, "{way} round {round}");
                fs::remove_dir_all(lost_at).unwrap();
            }
        }
    }
}
