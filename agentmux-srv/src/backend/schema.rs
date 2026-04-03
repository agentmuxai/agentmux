// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Schema path resolution with `.json` extension fallback.
//! Port of Go's `pkg/schema/schema.go`.
//!
//! Provides file path resolution for JSON schema files, trying the
//! exact path first, then falling back to appending `.json`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The content type for JSON schema responses.
pub const SCHEMA_CONTENT_TYPE: &str = "application/schema+json";

/// Resolve a schema file path, trying the exact path first, then with `.json` extension.
///
/// Returns `Some(path)` if a file exists at the exact path or with `.json` appended.
/// Returns `None` if neither exists.
pub fn resolve_schema_path(base_dir: &Path, name: &str) -> Option<PathBuf> {
    // Prevent directory traversal
    if name.contains("..") {
        return None;
    }

    let exact = base_dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }

    // Try with .json extension
    let with_ext = base_dir.join(format!("{}.json", name));
    if with_ext.is_file() {
        return Some(with_ext);
    }

    None
}

/// Get the schema directory path from the app path.
pub fn get_schema_dir(app_path: &Path) -> PathBuf {
    app_path.join("schema")
}

/// Check if a schema directory exists and is valid.
pub fn schema_dir_exists(app_path: &Path) -> bool {
    let schema_dir = get_schema_dir(app_path);
    schema_dir.is_dir()
}

/// Normalize a request path for schema lookup.
/// Strips leading slashes and ensures no directory traversal.
pub fn normalize_schema_request(path: &str) -> Option<String> {
    let stripped = path.trim_start_matches('/');
    if stripped.contains("..") {
        return None;
    }
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "schema_test_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let schema_dir = dir.join("schema");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&schema_dir).unwrap();
        schema_dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir.parent().unwrap_or(dir));
    }

    #[test]
    fn test_resolve_exact_path() {
        let dir = setup_test_dir();
        fs::write(dir.join("test.json"), "{}").unwrap();

        let result = resolve_schema_path(&dir, "test.json");
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("test.json"));

        cleanup(&dir);
    }

    #[test]
    fn test_resolve_with_json_fallback() {
        let dir = setup_test_dir();
        fs::write(dir.join("test.json"), "{}").unwrap();

        let result = resolve_schema_path(&dir, "test");
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("test.json"));

        cleanup(&dir);
    }

    #[test]
    fn test_resolve_not_found() {
        let dir = setup_test_dir();

        let result = resolve_schema_path(&dir, "missing");
        assert!(result.is_none());

        cleanup(&dir);
    }

    #[test]
    fn test_resolve_traversal_rejected() {
        let dir = setup_test_dir();

        let result = resolve_schema_path(&dir, "../etc/passwd");
        assert!(result.is_none());

        cleanup(&dir);
    }

    #[test]
    fn test_normalize_schema_request() {
        assert_eq!(normalize_schema_request("/foo/bar"), Some("foo/bar".into()));
        assert_eq!(normalize_schema_request("foo"), Some("foo".into()));
        assert_eq!(normalize_schema_request("/"), None);
        assert_eq!(normalize_schema_request(""), None);
        assert_eq!(normalize_schema_request("/../bad"), None);
    }

    #[test]
    fn test_get_schema_dir() {
        let app_path = Path::new("/opt/agentmux");
        assert_eq!(get_schema_dir(app_path), PathBuf::from("/opt/agentmux/schema"));
    }

    #[test]
    fn test_schema_dir_exists() {
        let dir = setup_test_dir();
        let app_path = dir.parent().unwrap();
        assert!(schema_dir_exists(app_path));
        cleanup(&dir);
    }

    #[test]
    fn test_schema_dir_not_exists() {
        let app_path = Path::new("/nonexistent/path");
        assert!(!schema_dir_exists(app_path));
    }

    #[test]
    fn test_schema_content_type() {
        assert_eq!(SCHEMA_CONTENT_TYPE, "application/schema+json");
    }
}
