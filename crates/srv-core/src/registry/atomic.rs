// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Atomic file write + rename helpers. All registry mutations route
//! through these so readers never observe a half-written file.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Name prefix of the sibling temp file [`write_atomic`] writes before it renames.
const TEMP_PREFIX: &str = ".registry-tmp-";

/// How old a leftover temp file must be before [`sweep_stale_temps`] removes it.
/// A write holds its temp file for milliseconds (a write, an fsync, a rename), so a
/// file this old belongs to a write that was cut short, never to one in flight in
/// another process sharing the folder.
pub const STALE_TEMP_AGE: Duration = Duration::from_secs(600);

/// Whether `path` is a temp file [`write_atomic`] made: not a record, whatever
/// its `.json` ending says.
pub fn is_temp_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(TEMP_PREFIX))
}

/// Remove the temp files in `dir` that an interrupted write left behind: only
/// files named like [`write_atomic`]'s, and only those last changed more than
/// `min_age` ago. Returns how many were removed. A folder that can't be read or
/// a file that can't be removed is logged and left, never an error: this runs
/// at start and must not stop it.
pub fn sweep_stale_temps(dir: &Path, min_age: Duration) -> usize {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(dir = %dir.display(), error = %e, "registry: could not look for leftover temp files");
            }
            return 0;
        }
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_temp_file(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let old_enough = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= min_age);
        if !old_enough {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                removed += 1;
                tracing::info!(file = %path.display(), "registry: removed a temp file left by an interrupted write");
            }
            Err(e) => tracing::warn!(file = %path.display(), error = %e, "registry: could not remove a leftover temp file"),
        }
    }
    removed
}

/// Write `bytes` to `path` atomically: sibling temp file → fsync →
/// rename over the target. Rename is atomic on every supported
/// filesystem when source and destination live on the same volume,
/// which is guaranteed here (sibling temp).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write_atomic: path has no parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(".json")
        .tempfile_in(parent)?;
    tmp.as_file_mut().write_all(bytes)?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Atomic rename. Used for retire/unretire (same-directory tree).
pub fn rename_atomic(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_parent_and_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested").join("file.json");
        write_atomic(&target, b"hello").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
    }

    #[test]
    fn write_atomic_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a.json");
        write_atomic(&target, b"v1").unwrap();
        write_atomic(&target, b"v2-longer").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"v2-longer");
    }

    fn aged(path: &Path, secs: u64) {
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn sweep_removes_only_old_files_named_like_write_atomics_temp() {
        let dir = tempfile::tempdir().unwrap();
        let old_tmp = dir.path().join(".registry-tmp-old.json");
        let fresh_tmp = dir.path().join(".registry-tmp-fresh.json");
        let record = dir.path().join("abc.json");
        let other_hidden = dir.path().join(".something-else.json");
        for p in [&old_tmp, &fresh_tmp, &record, &other_hidden] {
            std::fs::write(p, b"{}").unwrap();
        }
        for p in [&old_tmp, &record, &other_hidden] {
            aged(p, 3600);
        }
        assert_eq!(sweep_stale_temps(dir.path(), STALE_TEMP_AGE), 1);
        assert!(!old_tmp.exists(), "an old temp file is removed");
        assert!(fresh_tmp.exists(), "a write that may be in flight is left");
        assert!(record.exists(), "a record is never removed, however old");
        assert!(other_hidden.exists(), "only the temp pattern is removed");
    }

    #[test]
    fn sweep_of_a_missing_folder_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sweep_stale_temps(&dir.path().join("nope"), STALE_TEMP_AGE), 0);
    }

    #[test]
    fn is_temp_file_matches_the_name_write_atomic_gives() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.json");
        let tmp = tempfile::Builder::new().prefix(TEMP_PREFIX).suffix(".json").tempfile_in(dir.path()).unwrap();
        assert!(is_temp_file(tmp.path()));
        assert!(!is_temp_file(&target));
    }

    #[test]
    fn rename_atomic_moves_file() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from.json");
        let to = tmp.path().join("retired").join("from.json");
        std::fs::write(&from, b"x").unwrap();
        rename_atomic(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"x");
    }

    #[test]
    fn rename_atomic_does_not_create_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("missing.json");
        let to = tmp.path().join("retired.json");
        let err = rename_atomic(&from, &to).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
