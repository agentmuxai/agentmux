// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tar copy utilities for streaming file transfers over channels.
//! Port of Go's `pkg/util/tarcopy/tarcopy.go`.
//!
//! Provides path normalization with directory traversal protection
//! and single-file mode for tar streams.


use std::path::{Path, PathBuf};

/// Custom flag to indicate that the source is a single file.
pub const SINGLE_FILE: &str = "singlefile";

/// File chunk size for tar streaming (matches Go's wshrpc.FileChunkSize).
pub const FILE_CHUNK_SIZE: usize = 64 * 1024;

/// Tar entry metadata extracted from a tar header.
#[derive(Debug, Clone)]
pub struct TarEntryMeta {
    /// Entry name/path within the tar.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Whether this is a single-file tar (from PAX records).
    pub single_file: bool,
    /// Modification time as Unix timestamp.
    pub mod_time: i64,
    /// File mode/permissions.
    pub mode: u32,
}

/// Errors from tar copy operations.
#[derive(Debug, Clone, PartialEq)]
pub enum TarCopyError {
    /// Path contains directory traversal sequences.
    PathTraversal(String),
    /// Attempted to write multiple files in single-file mode.
    MultipleSingleFiles,
    /// Invalid tar path.
    InvalidPath(String),
}

impl std::fmt::Display for TarCopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathTraversal(path) => write!(f, "invalid tar path containing directory traversal: {}", path),
            Self::MultipleSingleFiles => write!(f, "attempting to write multiple files to a single file tar stream"),
            Self::InvalidPath(path) => write!(f, "invalid tar path: {}", path),
        }
    }
}

impl std::error::Error for TarCopyError {}

/// Fix and normalize a path by removing a prefix and checking for directory traversal.
///
/// Matches Go's `fixPath`: strips prefix, cleans, strips leading separators,
/// then checks for remaining `..` components.
///
/// Returns an empty string if the cleaned path is empty or equals root.
/// Returns an error if the path contains `..` after cleaning.
pub fn fix_path(path: &str, prefix: &str) -> Result<String, TarCopyError> {
    // Remove prefix
    let stripped = path.strip_prefix(prefix).unwrap_or(path);

    // Clean the path (equivalent to filepath.Clean)
    let cleaned = clean_path(stripped);

    // Remove leading separator (matches Go's TrimPrefix for "/" and "\\")
    let result = cleaned.trim_start_matches('/').trim_start_matches('\\');

    // "." means empty/root path after cleaning
    let result = if result == "." { "" } else { result };

    // Check for directory traversal
    if result.contains("..") {
        return Err(TarCopyError::PathTraversal(path.to_string()));
    }

    Ok(result.to_string())
}

/// Clean a path, resolving `.` and `..` components and normalizing separators.
/// Matches Go's `filepath.Clean` behavior.
fn clean_path(path: &str) -> String {
    // Normalize separators to /
    let normalized = path.replace('\\', "/");
    let is_absolute = normalized.starts_with('/');
    let mut components: Vec<&str> = Vec::new();

    for part in normalized.split('/') {
        match part {
            "" | "." => {} // skip empty and current dir
            ".." => {
                // Only pop if last component is a real directory (not "..")
                if !components.is_empty() && *components.last().unwrap() != ".." {
                    components.pop();
                } else if !is_absolute {
                    // For relative paths, keep the ".."
                    components.push("..");
                }
                // For absolute paths, ".." at root is just ignored
            }
            s => {
                components.push(s);
            }
        }
    }

    if components.is_empty() {
        if is_absolute {
            "/".to_string()
        } else {
            ".".to_string()
        }
    } else if is_absolute {
        format!("/{}", components.join("/"))
    } else {
        components.join("/")
    }
}

/// State tracker for single-file mode in tar source operations.
#[derive(Debug, Default)]
pub struct SingleFileTracker {
    flag_set: bool,
}

impl SingleFileTracker {
    pub fn new() -> Self {
        Self { flag_set: false }
    }

    /// Mark as single file. Returns error if already marked.
    pub fn mark_single_file(&mut self) -> Result<(), TarCopyError> {
        if self.flag_set {
            return Err(TarCopyError::MultipleSingleFiles);
        }
        self.flag_set = true;
        Ok(())
    }

    /// Check if single-file mode has been set.
    pub fn is_single_file(&self) -> bool {
        self.flag_set
    }
}

/// Validate a tar entry name for directory traversal.
pub fn validate_tar_name(name: &str) -> Result<(), TarCopyError> {
    if name.contains("..") {
        return Err(TarCopyError::PathTraversal(name.to_string()));
    }
    Ok(())
}

/// Construct a destination path from a base directory and tar entry name.
/// Ensures the resulting path is within the base directory.
pub fn safe_join(base: &Path, name: &str) -> Result<PathBuf, TarCopyError> {
    validate_tar_name(name)?;

    let joined = base.join(name);

    // Verify the canonical path stays within base
    // (this is a defense-in-depth check)
    let joined_str = joined.to_string_lossy();
    let base_str = base.to_string_lossy();
    if !joined_str.starts_with(base_str.as_ref()) {
        return Err(TarCopyError::PathTraversal(name.to_string()));
    }

    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- fix_path Tests ----

    #[test]
    fn test_fix_path_simple() {
        assert_eq!(fix_path("/root/file.txt", "/root").unwrap(), "file.txt");
    }

    #[test]
    fn test_fix_path_with_subdir() {
        assert_eq!(fix_path("/root/sub/file.txt", "/root").unwrap(), "sub/file.txt");
    }

    #[test]
    fn test_fix_path_root_becomes_empty() {
        assert_eq!(fix_path("/root", "/root").unwrap(), "");
    }

    #[test]
    fn test_fix_path_no_prefix_match() {
        assert_eq!(fix_path("/other/file.txt", "/root").unwrap(), "other/file.txt");
    }

    #[test]
    fn test_fix_path_traversal_resolved_by_clean() {
        // Go's filepath.Clean resolves ".." so /root/../etc/passwd → /etc/passwd
        // After prefix strip and clean, no ".." remains → allowed
        let result = fix_path("/root/../etc/passwd", "/root").unwrap();
        assert_eq!(result, "etc/passwd");
    }

    #[test]
    fn test_fix_path_raw_traversal_rejected() {
        // A path that has ".." remaining after clean is rejected
        // This happens when ".." appears at the beginning and can't be resolved further
        // e.g. "../../etc/passwd" with no prefix → clean doesn't resolve leading ".."
        assert!(fix_path("../../etc/passwd", "").is_err());
    }

    #[test]
    fn test_fix_path_empty() {
        assert_eq!(fix_path("", "").unwrap(), "");
    }

    #[test]
    fn test_fix_path_dot_segments() {
        assert_eq!(fix_path("/root/./file.txt", "/root").unwrap(), "file.txt");
    }

    // ---- clean_path Tests ----

    #[test]
    fn test_clean_path_simple() {
        assert_eq!(clean_path("a/b/c"), "a/b/c");
    }

    #[test]
    fn test_clean_path_dot() {
        assert_eq!(clean_path("a/./b"), "a/b");
    }

    #[test]
    fn test_clean_path_double_dot() {
        assert_eq!(clean_path("a/b/../c"), "a/c");
    }

    #[test]
    fn test_clean_path_trailing_slash() {
        assert_eq!(clean_path("a/b/"), "a/b");
    }

    #[test]
    fn test_clean_path_backslash() {
        assert_eq!(clean_path("a\\b\\c"), "a/b/c");
    }

    // ---- SingleFileTracker Tests ----

    #[test]
    fn test_single_file_tracker_first() {
        let mut tracker = SingleFileTracker::new();
        assert!(!tracker.is_single_file());
        tracker.mark_single_file().unwrap();
        assert!(tracker.is_single_file());
    }

    #[test]
    fn test_single_file_tracker_double() {
        let mut tracker = SingleFileTracker::new();
        tracker.mark_single_file().unwrap();
        assert!(tracker.mark_single_file().is_err());
    }

    // ---- validate_tar_name Tests ----

    #[test]
    fn test_validate_normal_name() {
        assert!(validate_tar_name("dir/file.txt").is_ok());
    }

    #[test]
    fn test_validate_traversal_name() {
        assert!(validate_tar_name("../etc/passwd").is_err());
    }

    #[test]
    fn test_validate_embedded_traversal() {
        assert!(validate_tar_name("dir/../../../etc/passwd").is_err());
    }

    // ---- safe_join Tests ----

    #[test]
    fn test_safe_join_normal() {
        let base = Path::new("/tmp/extract");
        let result = safe_join(base, "file.txt").unwrap();
        assert_eq!(result, PathBuf::from("/tmp/extract/file.txt"));
    }

    #[test]
    fn test_safe_join_subdir() {
        let base = Path::new("/tmp/extract");
        let result = safe_join(base, "sub/file.txt").unwrap();
        assert_eq!(result, PathBuf::from("/tmp/extract/sub/file.txt"));
    }

    #[test]
    fn test_safe_join_traversal() {
        let base = Path::new("/tmp/extract");
        assert!(safe_join(base, "../etc/passwd").is_err());
    }

    // ---- TarEntryMeta Tests ----

    #[test]
    fn test_tar_entry_meta() {
        let meta = TarEntryMeta {
            name: "test.txt".to_string(),
            size: 1024,
            is_dir: false,
            single_file: true,
            mod_time: 1700000000,
            mode: 0o644,
        };
        assert_eq!(meta.name, "test.txt");
        assert_eq!(meta.size, 1024);
        assert!(!meta.is_dir);
        assert!(meta.single_file);
    }

    // ---- TarCopyError Display ----

    #[test]
    fn test_error_display_traversal() {
        let err = TarCopyError::PathTraversal("../bad".into());
        assert!(err.to_string().contains("directory traversal"));
    }

    #[test]
    fn test_error_display_multiple() {
        let err = TarCopyError::MultipleSingleFiles;
        assert!(err.to_string().contains("multiple files"));
    }
}
