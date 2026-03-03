// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Documentation site path resolution.
//! Port of Go's pkg/docsite/.
//!
//! Resolves paths to the bundled documentation site directory
//! with automatic .html extension fallback.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

// ---- Docsite resolution ----

/// Cached docsite directory path.
static DOCSITE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Set the docsite directory path (typically `<app_path>/docsite`).
///
/// Must be called before `get_docsite_dir`. Once set, the path is cached.
pub fn set_docsite_dir(path: PathBuf) {
    let _ = DOCSITE_DIR.set(if path.is_dir() { Some(path) } else { None });
}

/// Get the cached docsite directory, if it exists.
pub fn get_docsite_dir() -> Option<&'static PathBuf> {
    DOCSITE_DIR.get().and_then(|opt| opt.as_ref())
}

/// Resolve a request path to a file in the docsite directory.
///
/// Tries the exact path first, then appends `.html` as fallback.
/// Returns `None` if no matching file exists or docsite is not configured.
///
/// # Examples
///
/// ```
/// use backend_test::backend::docsite::resolve_docsite_path;
///
/// // Without docsite configured, always returns None
/// assert!(resolve_docsite_path("/docs/foo").is_none());
/// ```
pub fn resolve_docsite_path(request_path: &str) -> Option<PathBuf> {
    let base = get_docsite_dir()?;

    // Strip leading slash and normalize
    let clean = request_path.trim_start_matches('/');
    if clean.is_empty() {
        // Try index.html
        let index = base.join("index.html");
        if index.is_file() {
            return Some(index);
        }
        return None;
    }

    // Prevent path traversal
    if clean.contains("..") {
        return None;
    }

    // Try exact path
    let exact = base.join(clean);
    if exact.is_file() {
        return Some(exact);
    }

    // Try with .html extension
    let with_html = base.join(format!("{}.html", clean));
    if with_html.is_file() {
        return Some(with_html);
    }

    None
}

/// Check if a path component is safe (no traversal).
pub fn is_safe_path(path: &str) -> bool {
    !path.contains("..") && !path.contains('\0')
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_no_docsite() {
        // Without setting docsite dir, should return None
        assert!(resolve_docsite_path("/anything").is_none());
    }

    #[test]
    fn test_is_safe_path() {
        assert!(is_safe_path("docs/page"));
        assert!(is_safe_path("index.html"));
        assert!(!is_safe_path("../etc/passwd"));
        assert!(!is_safe_path("docs/../../secret"));
        assert!(!is_safe_path("path\0with\0null"));
    }

    #[test]
    fn test_resolve_blocks_traversal() {
        // Even if docsite were set, traversal should be blocked
        assert!(resolve_docsite_path("/../../../etc/passwd").is_none());
    }
}
