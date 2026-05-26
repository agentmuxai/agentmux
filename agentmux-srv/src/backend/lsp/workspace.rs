// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Workspace-root detection for LSP. Walks up from a file's directory
// looking for project-marker files (`.git`, `Cargo.toml`, `package.json`,
// `go.mod`, `pyproject.toml`). Falls back to the file's parent dir if
// none found.

use std::path::{Path, PathBuf};

/// Project-marker files we treat as a workspace root.
/// Order doesn't matter — we return the *closest* ancestor that has any
/// of these (walking up from the file's dir).
const MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    "deno.json",
    "deno.jsonc",
    "tsconfig.json",
];

pub fn detect_workspace_root(file_path: &Path) -> PathBuf {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        for marker in MARKERS {
            if dir.join(marker).exists() {
                return dir.to_path_buf();
            }
        }
        current = dir.parent();
    }
    // Fall back to the file's parent (or "." if even that's missing).
    file_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn falls_back_to_parent_dir_when_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("loose.txt");
        fs::write(&file, "").unwrap();
        // No markers anywhere — root is the temp dir
        assert_eq!(detect_workspace_root(&file), tmp.path());
    }

    #[test]
    fn finds_git_root_up_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("a/b/c");
        fs::create_dir_all(&sub).unwrap();
        let file = sub.join("file.rs");
        fs::write(&file, "").unwrap();
        assert_eq!(detect_workspace_root(&file), root);
    }

    #[test]
    fn finds_cargo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("Cargo.toml"), "").unwrap();
        let sub = root.join("src");
        fs::create_dir(&sub).unwrap();
        let file = sub.join("main.rs");
        fs::write(&file, "").unwrap();
        assert_eq!(detect_workspace_root(&file), root);
    }

    #[test]
    fn closest_ancestor_wins_when_nested_markers() {
        // Outer Cargo.toml, inner package.json — file inside inner should
        // resolve to inner.
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path();
        fs::write(outer.join("Cargo.toml"), "").unwrap();
        let inner = outer.join("frontend");
        fs::create_dir(&inner).unwrap();
        fs::write(inner.join("package.json"), "").unwrap();
        let file = inner.join("src/index.ts");
        fs::create_dir(inner.join("src")).unwrap();
        fs::write(&file, "").unwrap();
        assert_eq!(detect_workspace_root(&file), inner);
    }
}
