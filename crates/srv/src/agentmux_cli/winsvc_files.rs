// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The package of AgentMux Headless as the installer of the Windows service reads, holds and copies it
//! (docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md, B.1): every file hashed, each held open so that nothing can
//! change or replace it while the elevated step runs, and a copy that is hashed again at its destination and must give the
//! digest of the package. Plain file work, so the same code and tests run on every platform; only the holding (a share mode)
//! is Windows's.

#![cfg_attr(not(windows), allow(dead_code))]

use std::fs::{File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::winsvc_plan::{self as plan, Entry};

/// The most files a package may have. A real one has a few thousand (the web page's assets); more than this is not one.
const MAX_FILES: usize = 50_000;
/// The deepest folder a package may have.
const MAX_DEPTH: usize = 24;

/// A package folder read: each file's path, size and SHA-256, and a handle to each that stays open (on Windows without
/// the right to write, rename or delete it for anyone else) until the package is dropped.
pub struct Package {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    _held: Vec<File>,
}

impl Package {
    /// [`plan::tree_digest`] of the files read.
    pub fn digest(&self) -> String {
        plan::tree_digest(&self.entries)
    }
}

/// A link of any kind (a symbolic link or a junction): what a copy would follow out of the package.
fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// Open a file for reading and, on Windows, let nobody else write, rename or delete it while it is open.
fn open_held(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

/// The size and SHA-256 of what is left to read of an open file.
fn hash_of(file: &mut File) -> std::io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, hex::encode(hasher.finalize())))
}

/// The files under `root`, as paths relative to it with forward slashes. A link, or anything that is neither a file nor a
/// folder, is refused: the package is the files in it and nothing a link reaches.
fn list_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut todo: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((dir, prefix, depth)) = todo.pop() {
        if depth > MAX_DEPTH {
            return Err(format!("{} is nested more than {MAX_DEPTH} folders deep: this is not an AgentMux Headless package", dir.display()));
        }
        let read = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        for item in read {
            let item = item.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let name = item.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{}", name) };
            let meta = std::fs::symlink_metadata(item.path()).map_err(|e| format!("cannot read {}: {e}", item.path().display()))?;
            if is_link(&meta) {
                return Err(format!("{} is a link: a package holds files, not links to them, so this is not one to install from", item.path().display()));
            }
            if meta.is_dir() {
                todo.push((item.path(), rel, depth + 1));
            } else if meta.is_file() {
                out.push(rel);
            } else {
                return Err(format!("{} is not a file or a folder", item.path().display()));
            }
            if out.len() > MAX_FILES {
                return Err(format!("{} holds more than {MAX_FILES} files: this is not an AgentMux Headless package", root.display()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Read the package at `root`: hash every file, and, when `hold`, keep each file open without the right for others to
/// write, rename or delete it until the result is dropped. A package that lacks the programs or the web page is refused.
pub fn read_package(root: &Path, hold: bool) -> Result<Package, String> {
    let rels = list_files(root)?;
    let lower: Vec<String> = rels.iter().map(|r| r.to_lowercase()).collect();
    let missing = plan::missing_from_package(&lower);
    if !missing.is_empty() {
        return Err(format!("{} is not an AgentMux Headless package (no {})", root.display(), missing.join(", ")));
    }
    let (mut entries, mut held) = (Vec::with_capacity(rels.len()), Vec::new());
    for rel in rels {
        let path = root.join(&rel);
        let mut file = open_held(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let (size, sha256) = hash_of(&mut file).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        entries.push(Entry { rel, size, sha256 });
        if hold {
            held.push(file);
        }
    }
    Ok(Package { root: root.to_path_buf(), entries, _held: held })
}

/// Copy each file of `pkg` into `dest` (made if it is not there), keeping its place under the package.
pub fn copy_package(pkg: &Package, dest: &Path) -> Result<(), String> {
    for entry in &pkg.entries {
        let (from, to) = (pkg.root.join(&entry.rel), dest.join(&entry.rel));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))?;
    }
    Ok(())
}

/// Hash what is at `dest` and check it is the package that has the digest `expected`: the same files, each with the same
/// bytes, and no other. This reads the destination, never the source (B.1).
pub fn verify_copy(dest: &Path, expected: &str) -> Result<(), String> {
    let copy = read_package(dest, false).map_err(|e| format!("the copy cannot be verified: {e}"))?;
    if copy.digest() != expected {
        return Err(format!("the files copied to {} are not the files that were checked: nothing is installed", dest.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &Path) {
        for (rel, text) in [("bin/agentmux.exe", "exe"), ("bin/agentmux-srv.exe", "srv"), ("web/index.html", "<html>"), ("web/a/b.js", "1"), ("VERSION", "0.60.38")] {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    #[test]
    fn a_package_is_read_file_by_file_and_has_one_digest() {
        let tmp = tempfile::tempdir().unwrap();
        package(tmp.path());
        let pkg = read_package(tmp.path(), true).unwrap();
        let rels: Vec<&str> = pkg.entries.iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(rels, vec!["VERSION", "bin/agentmux-srv.exe", "bin/agentmux.exe", "web/a/b.js", "web/index.html"]);
        let exe = pkg.entries.iter().find(|e| e.rel == "bin/agentmux.exe").unwrap();
        assert_eq!((exe.size, exe.sha256.clone()), (3, hex::encode(Sha256::digest(b"exe"))));
        assert!(plan::digest_is_valid(&pkg.digest()));
        assert_eq!(read_package(tmp.path(), false).unwrap().digest(), pkg.digest(), "the same files, the same digest");
    }

    #[test]
    fn a_package_without_its_programs_or_page_is_refused_in_words() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        std::fs::write(tmp.path().join("bin/agentmux.exe"), "x").unwrap();
        let said = read_package(tmp.path(), false).err().unwrap();
        assert!(said.contains("is not an AgentMux Headless package") && said.contains("bin/agentmux-srv.exe, web/index.html"), "{said}");
    }

    #[test]
    fn a_verified_copy_is_the_package_and_any_change_to_it_is_found_by_hashing_the_destination() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        package(src.path());
        let pkg = read_package(src.path(), true).unwrap();
        let want = pkg.digest();
        copy_package(&pkg, dst.path()).unwrap();
        verify_copy(dst.path(), &want).unwrap();
        // A byte changed in the copy, a file added to it, a file left out of it: each is not the package.
        std::fs::write(dst.path().join("web/index.html"), "<html!").unwrap();
        assert!(verify_copy(dst.path(), &want).unwrap_err().contains("not the files that were checked"));
        std::fs::write(dst.path().join("web/index.html"), "<html>").unwrap();
        verify_copy(dst.path(), &want).unwrap();
        std::fs::write(dst.path().join("bin/extra.dll"), "x").unwrap();
        assert!(verify_copy(dst.path(), &want).is_err());
        std::fs::remove_file(dst.path().join("bin/extra.dll")).unwrap();
        std::fs::remove_file(dst.path().join("web/a/b.js")).unwrap();
        assert!(verify_copy(dst.path(), &want).is_err());
    }

    #[test]
    fn the_source_changed_after_it_was_hashed_gives_another_digest_than_the_one_checked() {
        let tmp = tempfile::tempdir().unwrap();
        package(tmp.path());
        let first = read_package(tmp.path(), false).unwrap().digest();
        std::fs::write(tmp.path().join("bin/agentmux.exe"), "swapped").unwrap();
        assert_ne!(read_package(tmp.path(), false).unwrap().digest(), first);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_a_package_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        package(tmp.path());
        std::os::unix::fs::symlink("/etc", tmp.path().join("web/etc")).unwrap();
        let said = read_package(tmp.path(), false).err().unwrap();
        assert!(said.contains("is a link"), "{said}");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_that_is_held_cannot_be_changed_renamed_or_deleted_by_anyone_else() {
        let tmp = tempfile::tempdir().unwrap();
        package(tmp.path());
        let pkg = read_package(tmp.path(), true).unwrap();
        let exe = tmp.path().join("bin/agentmux.exe");
        assert!(std::fs::write(&exe, "swapped").is_err(), "written");
        assert!(std::fs::rename(&exe, tmp.path().join("bin/other.exe")).is_err(), "renamed");
        assert!(std::fs::remove_file(&exe).is_err(), "deleted");
        assert!(std::fs::read(&exe).is_ok(), "still read by the elevated step");
        drop(pkg);
        std::fs::write(&exe, "free again").unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_inside_a_package_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        package(tmp.path());
        let elsewhere = tempfile::tempdir().unwrap();
        agentmux_common::data_paths::ensure_history_link(&tmp.path().join("web/out"), elsewhere.path()).unwrap();
        let said = read_package(tmp.path(), false).err().unwrap();
        assert!(said.contains("is a link"), "{said}");
    }
}
