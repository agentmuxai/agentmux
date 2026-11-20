// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Workspace-root detection for LSP. Walks up from a file's directory
// looking for project-marker files (`.git`, `Cargo.toml`, `package.json`,
// `go.mod`, `pyproject.toml`). Falls back to the file's parent dir if
// none found.

use std::path::{Path, PathBuf};

use crate::backend::fs_ops::container_folders::Place;

/// Whether the language server for `file`, rooted at `root`, runs on the host: only when no container agent
/// can write the file or the root. `place` answers for each, as it is written and as it resolves (links
/// followed). A server on the host is for the person's own folders; where a folder's owner can't be told,
/// none is started either.
pub fn runs_on_host(file: &Path, root: &Path, place: &dyn Fn(&str) -> Place) -> bool {
    [file, root].into_iter().all(|path| matches!(place(&path.to_string_lossy()), Place::Host))
}

/// Whether a language server for `file`, rooted at `root`, is started on the host: [`runs_on_host`] says so
/// of the file and the root, and no folder a container agent can write lies beneath the root (a server
/// works with everything beneath its root, not only the file). Reads the disk and the store.
pub fn served_on_host(file: &Path, root: &Path) -> bool {
    use crate::backend::fs_ops::container_folders::{holds_a_container_agents_folder, place};
    runs_on_host(file, root, &place) && !holds_a_container_agents_folder(&root.to_string_lossy())
}

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

    use crate::backend::fs_ops::container_folders::{self, place};

    /// A temp folder holding `ws`, counted as bound writable into a container, and `own`, the person's.
    /// Each holds a project (`package.json`) with one source file.
    fn an_agents_folder_and_the_persons() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let (ws, own) = (tmp.path().join("ws"), tmp.path().join("own"));
        for project in [ws.join("proj"), own.join("proj")] {
            fs::create_dir_all(project.join("src")).unwrap();
            fs::write(project.join("package.json"), "{}").unwrap();
            fs::write(project.join("src").join("a.ts"), "").unwrap();
        }
        container_folders::add_for_test(&ws, &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        (tmp, ws, own)
    }

    fn link_dir(to: &Path, at: &Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(to, at).is_ok();
        #[cfg(windows)]
        return std::os::windows::fs::symlink_dir(to, at).is_ok();
    }

    fn link_file(to: &Path, at: &Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(to, at).is_ok();
        #[cfg(windows)]
        return std::os::windows::fs::symlink_file(to, at).is_ok();
    }

    #[test]
    fn a_language_server_runs_on_the_host_for_a_folder_no_container_agent_can_write() {
        let (_tmp, _ws, own) = an_agents_folder_and_the_persons();
        let file = own.join("proj").join("src").join("a.ts");
        let root = detect_workspace_root(&file);
        assert_eq!(root, own.join("proj"));
        assert!(runs_on_host(&file, &root, &place));
    }

    #[test]
    fn no_language_server_runs_on_the_host_for_a_file_in_a_container_agents_folder() {
        let (_tmp, ws, _own) = an_agents_folder_and_the_persons();
        let file = ws.join("proj").join("src").join("a.ts");
        let root = detect_workspace_root(&file);
        assert_eq!(root, ws.join("proj"));
        assert!(!runs_on_host(&file, &root, &place));
        // The folder itself, and a file that is not there yet, are the agent's too.
        assert!(!runs_on_host(&ws, &ws, &place));
        assert!(!runs_on_host(&ws.join("new.ts"), &ws, &place));
    }

    #[test]
    fn no_language_server_runs_on_the_host_while_a_folders_owner_cannot_be_told() {
        let (_tmp, _ws, own) = an_agents_folder_and_the_persons();
        let file = own.join("proj").join("src").join("a.ts");
        assert!(!runs_on_host(&file, &own.join("proj"), &|_| Place::Unknown));
        // One of the two not known is enough.
        let file_text = file.to_string_lossy().into_owned();
        assert!(!runs_on_host(&file, &own.join("proj"), &|asked| if asked == file_text { Place::Host } else { Place::Unknown }));
    }

    #[test]
    fn a_project_folder_a_container_agent_can_write_gets_no_server_for_a_file_outside_it() {
        let (_tmp, ws, own) = an_agents_folder_and_the_persons();
        let file = own.join("proj").join("src").join("a.ts");
        assert!(matches!(place(&file.to_string_lossy()), Place::Host));
        assert!(!runs_on_host(&file, &ws.join("proj"), &place));
    }

    /// A folder outside the agent's that is a link into it: the file and the root are written outside, and
    /// both resolve inside.
    #[test]
    fn a_linked_folder_that_resolves_into_a_container_agents_folder_gets_no_server() {
        let (tmp, ws, _own) = an_agents_folder_and_the_persons();
        let link = tmp.path().join("link");
        if !link_dir(&ws.join("proj"), &link) {
            return;
        }
        let file = link.join("src").join("a.ts");
        let root = detect_workspace_root(&file);
        assert_eq!(root, link, "the root is found as the path is written");
        assert!(!runs_on_host(&file, &root, &place));
    }

    /// A file outside the agent's folder that is a link to one inside it: its own project folder is the
    /// person's, and the file resolves inside.
    #[test]
    fn a_linked_file_that_resolves_into_a_container_agents_folder_gets_no_server() {
        let (_tmp, ws, own) = an_agents_folder_and_the_persons();
        let file = own.join("proj").join("src").join("linked.ts");
        if !link_file(&ws.join("proj").join("src").join("a.ts"), &file) {
            return;
        }
        let root = detect_workspace_root(&file);
        assert_eq!(root, own.join("proj"));
        assert!(matches!(place(&root.to_string_lossy()), Place::Host));
        assert!(!runs_on_host(&file, &root, &place));
    }

    /// The person's project `own/proj` with one package of it, `packages/agent`, bound writable into a container.
    fn a_project_holding_an_agents_folder(own: &Path) -> PathBuf {
        let package = own.join("proj").join("packages").join("agent");
        fs::create_dir_all(&package).unwrap();
        container_folders::add_for_test(&package, &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        package
    }

    #[test]
    fn a_project_with_a_container_agents_folder_beneath_it_gets_no_server() {
        let (_tmp, _ws, own) = an_agents_folder_and_the_persons();
        let file = own.join("proj").join("src").join("a.ts");
        let root = detect_workspace_root(&file);
        assert!(served_on_host(&file, &root), "the person's project, before a folder beneath it is an agent's");
        a_project_holding_an_agents_folder(&own);
        // The file and the root are the person's; the folder beneath the root is not.
        assert!(runs_on_host(&file, &root, &place));
        assert!(!served_on_host(&file, &root));
    }

    #[test]
    fn a_container_agents_folder_beside_a_project_leaves_its_server_on_the_host() {
        let (tmp, ws, own) = an_agents_folder_and_the_persons();
        // `ws` is beside `own`; so is a folder whose name starts with the project's.
        let longer = own.join("proj-agent");
        fs::create_dir_all(&longer).unwrap();
        container_folders::add_for_test(&longer, &format!("lsp-{}", uuid::Uuid::new_v4().simple()), "/workspace");
        let file = own.join("proj").join("src").join("a.ts");
        assert!(served_on_host(&file, &detect_workspace_root(&file)));
        assert!(!served_on_host(&ws.join("proj").join("src").join("a.ts"), &ws.join("proj")));
        assert!(!served_on_host(&tmp.path().join("loose.ts"), tmp.path()), "the folder above both holds the agent's");
    }

    /// The project is reached through a link: the root is written outside, and resolves to the folder that
    /// holds the agent's.
    #[test]
    fn a_linked_project_with_a_container_agents_folder_beneath_it_gets_no_server() {
        let (tmp, _ws, own) = an_agents_folder_and_the_persons();
        a_project_holding_an_agents_folder(&own);
        let link = tmp.path().join("link");
        if !link_dir(&own.join("proj"), &link) {
            return;
        }
        let file = link.join("src").join("a.ts");
        let root = detect_workspace_root(&file);
        assert_eq!(root, link, "the root is found as the path is written");
        assert!(runs_on_host(&file, &root, &place));
        assert!(!served_on_host(&file, &root));
    }

    /// A link inside the agent's folder that points out of it: the path as written is the agent's.
    #[test]
    fn a_link_out_of_a_container_agents_folder_is_still_the_agents_path() {
        let (_tmp, ws, own) = an_agents_folder_and_the_persons();
        let link = ws.join("out");
        if !link_dir(&own.join("proj"), &link) {
            return;
        }
        let file = link.join("src").join("a.ts");
        assert!(!runs_on_host(&file, &detect_workspace_root(&file), &place));
    }
}
