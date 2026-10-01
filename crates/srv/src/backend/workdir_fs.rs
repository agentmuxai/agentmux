// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one way to write into an agent's working directory
//! (docs/specs/SPEC_WORKDIR_SAFE_WRITES_2026_10_01.md).
//!
//! An agent's workdir isn't wholly AgentMux's: it can hold the user's own
//! files, a cloned repo, and symlinks. Every write AgentMux makes into it —
//! `CLAUDE.md` and the other instructions files, `.claude/*`, `.mcp.json`,
//! skill files, manifests, backups — goes through [`Workdir`], which:
//!
//! - keeps the path inside the workdir, lexically (`safe_join_within_base`)
//!   and through symlinked folders (`verify_no_symlink_escape`);
//! - never writes through a symlinked file, dangling or not (a dangling link
//!   passes the folder check, and `fs::write` would follow it);
//! - replaces files atomically (temp file in the same folder, `sync_all`,
//!   rename), so a crash or a full disk leaves the old file or the new one,
//!   never half of each. A rename replaces a link *entry* rather than
//!   following it, so a link appearing between the check and the write
//!   still can't send the write outside.
//!
//! A refusal is an `io::ErrorKind::PermissionDenied` naming the path and
//! the reason; callers keep their own policy (fail the launch, or log and
//! go on). `scripts/check-workdir-writes.mjs` keeps raw filesystem writes
//! out of the files that write into workdirs.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// How many times an atomic replace retries a rename that Windows refused
/// (the target held open without delete-sharing), and how long it waits
/// between tries.
const RENAME_TRIES: u32 = 5;
const RENAME_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(40);

/// An agent's working directory, resolved once.
#[derive(Debug, Clone)]
pub struct Workdir {
    base: PathBuf,
    canonical: PathBuf,
}

fn refused(path: &Path, why: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("refused to write {}: {why}", path.display()))
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

impl Workdir {
    /// The workdir at `base`, which must already exist (callers create it).
    pub fn open(base: &Path) -> io::Result<Workdir> {
        let canonical = base.canonicalize()?;
        Ok(Workdir { base: base.to_path_buf(), canonical })
    }

    /// The workdir's own path, as given to [`Workdir::open`].
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// `rel` inside the workdir, or a refusal: lexically outside it, under a
    /// folder that links out of it, or a symlink itself (dangling or not).
    pub fn resolve(&self, rel: &str) -> io::Result<PathBuf> {
        let path = crate::backend::base::safe_join_within_base(&self.base, rel)
            .map_err(|e| refused(&self.base.join(rel), e))?;
        crate::backend::base::verify_no_symlink_escape(&path, &self.canonical).map_err(|e| refused(&path, e))?;
        if is_symlink(&path) {
            return Err(refused(&path, "it is a symlink"));
        }
        Ok(path)
    }

    /// The parent folder of a resolved path, created inside the workdir.
    fn ensure_parent(&self, path: &Path) -> io::Result<()> {
        let Some(parent) = path.parent() else { return Ok(()) };
        if parent.as_os_str().is_empty() || parent.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(parent)?;
        // A folder that appeared as a link between the check and the create
        // would only now be visible: check again before anything is written.
        crate::backend::base::verify_no_symlink_escape(path, &self.canonical).map_err(|e| refused(path, e))
    }

    /// Replace `rel` with `bytes`, atomically. Parent folders are created
    /// inside the workdir. `owner_only` makes the file 0600 on Unix (for
    /// files holding keys, such as `.mcp.json`).
    pub fn write(&self, rel: &str, bytes: &[u8], owner_only: bool) -> io::Result<()> {
        let path = self.resolve(rel)?;
        self.ensure_parent(&path)?;
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let tmp = path.with_file_name(format!(".{file_name}.{}.agentmux-tmp", uuid::Uuid::new_v4()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        if owner_only {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = owner_only;
        let written = (|| {
            let mut f = opts.open(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()
        })();
        let result = written.and_then(|()| rename_with_retry(&tmp, &path));
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// Create `rel` with `bytes` only if nothing is there: `Ok(true)` when
    /// created, `Ok(false)` when it already existed. A failed write removes
    /// the partial file, so it can't pass for a complete one later.
    pub fn create_new(&self, rel: &str, bytes: &[u8]) -> io::Result<bool> {
        let path = self.resolve(rel)?;
        self.ensure_parent(&path)?;
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) => return Err(e),
        };
        if let Err(e) = f.write_all(bytes).and_then(|()| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok(true)
    }

    /// Append `bytes` to an existing `rel`: a true append, never a rewrite,
    /// so an edit racing the caller's earlier read is kept.
    pub fn append(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.resolve(rel)?;
        std::fs::OpenOptions::new().append(true).open(&path)?.write_all(bytes)
    }

    /// Remove `rel`. The path is checked like a write, so a link to outside
    /// the workdir is refused rather than removed.
    pub fn remove_file(&self, rel: &str) -> io::Result<()> {
        let path = self.resolve(rel)?;
        std::fs::remove_file(path)
    }

    /// Remove the empty folder `rel`.
    pub fn remove_dir(&self, rel: &str) -> io::Result<()> {
        let path = self.resolve(rel)?;
        std::fs::remove_dir(path)
    }

    /// Open `rel` for read/write, creating it if absent and never truncating
    /// it: for lock files.
    pub fn open_lock_file(&self, rel: &str) -> io::Result<File> {
        let path = self.resolve(rel)?;
        self.ensure_parent(&path)?;
        #[allow(clippy::suspicious_open_options)]
        std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
    }
}

/// `fs::rename`, retried briefly when Windows refuses to replace a file
/// another process holds open; never falls back to a non-atomic write.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if tries + 1 < RENAME_TRIES && e.kind() == io::ErrorKind::PermissionDenied => {
                tries += 1;
                std::thread::sleep(RENAME_RETRY_DELAY);
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir() -> (tempfile::TempDir, Workdir) {
        let dir = tempfile::tempdir().unwrap();
        let wd = Workdir::open(dir.path()).unwrap();
        (dir, wd)
    }

    /// `link` → `target`; false where the OS won't create links (Windows
    /// without the privilege), in which case the caller skips.
    fn symlink_file(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        return std::os::windows::fs::symlink_file(target, link).is_ok();
    }

    fn symlink_dir(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        return std::os::windows::fs::symlink_dir(target, link).is_ok();
    }

    fn is_refusal(r: io::Result<impl std::fmt::Debug>) -> bool {
        matches!(r, Err(e) if e.kind() == io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn writes_create_parents_and_replace_whole() {
        let (dir, wd) = workdir();
        wd.write(".claude/skills/x/SKILL.md", b"one", false).unwrap();
        wd.write(".claude/skills/x/SKILL.md", b"two", false).unwrap();
        assert_eq!(std::fs::read(dir.path().join(".claude/skills/x/SKILL.md")).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join(".claude/skills/x"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".agentmux-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp file left behind");
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_writes_are_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let (dir, wd) = workdir();
        wd.write(".mcp.json", b"{}", true).unwrap();
        let mode = std::fs::metadata(dir.path().join(".mcp.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn paths_outside_the_workdir_are_refused() {
        let (_dir, wd) = workdir();
        assert!(is_refusal(wd.write("../escape.md", b"x", false)));
        assert!(is_refusal(wd.write("/etc/x", b"x", false)));
        assert!(is_refusal(wd.write("", b"x", false)));
    }

    #[test]
    fn a_symlinked_file_is_never_written_through_live_or_dangling() {
        let (dir, wd) = workdir();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("target.md");
        std::fs::write(&target, b"theirs").unwrap();
        if !symlink_file(&target, &dir.path().join("CLAUDE.md")) {
            return;
        }
        assert!(is_refusal(wd.write("CLAUDE.md", b"ours", false)));
        assert!(is_refusal(wd.append("CLAUDE.md", b"ours")));
        assert_eq!(std::fs::read(&target).unwrap(), b"theirs");

        let gone = outside.path().join("gone.md");
        if symlink_file(&gone, &dir.path().join("AGENTS.md")) {
            assert!(is_refusal(wd.write("AGENTS.md", b"ours", false)));
            assert!(is_refusal(wd.create_new("AGENTS.md", b"ours")));
            assert!(!gone.exists(), "nothing created at a dangling link's target");
        }
    }

    #[test]
    fn a_folder_linking_outside_is_refused() {
        let (dir, wd) = workdir();
        let outside = tempfile::tempdir().unwrap();
        if !symlink_dir(outside.path(), &dir.path().join(".claude")) {
            return;
        }
        assert!(is_refusal(wd.write(".claude/settings.json", b"{}", false)));
        assert!(is_refusal(wd.open_lock_file(".claude/.lock")));
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none(), "nothing written outside");
    }

    #[test]
    fn create_new_never_overwrites() {
        let (dir, wd) = workdir();
        assert!(wd.create_new(".claude/marker.json", b"first").unwrap());
        assert!(!wd.create_new(".claude/marker.json", b"second").unwrap());
        assert_eq!(std::fs::read(dir.path().join(".claude/marker.json")).unwrap(), b"first");
    }

    #[test]
    fn append_adds_to_the_end() {
        let (dir, wd) = workdir();
        std::fs::write(dir.path().join("CLAUDE.md"), b"mine\n").unwrap();
        wd.append("CLAUDE.md", b"@import\n").unwrap();
        assert_eq!(std::fs::read(dir.path().join("CLAUDE.md")).unwrap(), b"mine\n@import\n");
        assert!(wd.append("missing.md", b"x").is_err(), "append never creates");
    }

    #[test]
    fn removing_refuses_a_link_and_removes_a_file() {
        let (dir, wd) = workdir();
        wd.write(".claude/commands/a.md", b"x", false).unwrap();
        wd.remove_file(".claude/commands/a.md").unwrap();
        wd.remove_dir(".claude/commands").unwrap();
        assert!(!dir.path().join(".claude/commands").exists());

        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("keep.md");
        std::fs::write(&target, b"keep").unwrap();
        if symlink_file(&target, &dir.path().join("link.md")) {
            assert!(is_refusal(wd.remove_file("link.md")));
            assert!(target.exists());
        }
    }

    #[test]
    fn a_lock_file_is_created_and_never_truncated() {
        let (dir, wd) = workdir();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".claude/.lock"), b"held").unwrap();
        let _f = wd.open_lock_file(".claude/.lock").unwrap();
        assert_eq!(std::fs::read(dir.path().join(".claude/.lock")).unwrap(), b"held");
        let _g = wd.open_lock_file(".claude/new.lock").unwrap();
        assert!(dir.path().join(".claude/new.lock").exists());
    }
}
