// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Atomic file write + rename helpers. All registry mutations route
//! through these so readers never observe a half-written file.

use std::io::Write;
use std::path::Path;

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
        .prefix(".registry-tmp-")
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
