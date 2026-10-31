// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Files pane confined to one folder: a container agent's folder, shown to the person.
//!
//! A container agent's `/workspace` is a folder of the host that the agent can change while a window reads
//! it, so a link or junction it swaps in later could make a Files pane show the person's own files, which
//! the agent could then read back from a screenshot of the pane. So such a pane carries `files:root`, the
//! canonical host folder it was opened on, and srv checks it on the requests the pane makes that name it
//! (`fs.list`, `fs.watch`, and the changes that carry the pane's block id): the path is resolved again at
//! each access, links and junctions included, and refused unless it lies inside the root. The pane lists
//! and navigates; it changes nothing, and the view hides previews, open, reveal, terminal and places.
//!

use std::path::{Path, PathBuf};

use crate::backend::rpc_types::{FsError, FsErrorKind, FsListResult};
use crate::server::AppState;

/// The block meta key that confines a Files pane. Written when the pane is opened; a client may not write it.
pub(crate) const FILES_ROOT_META_KEY: &str = "files:root";

/// The folder `block_id`'s Files pane is confined to, if it is.
pub(crate) fn root_of(state: &AppState, block_id: &str) -> Option<PathBuf> {
    let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten()?;
    let root = crate::backend::obj::meta_get_string(&block.meta, FILES_ROOT_META_KEY, "");
    (!root.trim().is_empty()).then(|| PathBuf::from(root))
}

/// `path` as it is now (links and junctions resolved), if it lies inside `root`.
pub(crate) fn confine(root: &Path, path: &str) -> Result<PathBuf, String> {
    let canonical = crate::server::browser_uploads::canonical;
    let root = canonical(root).map_err(|_| "The folder this pane shows can't be read any more.".to_string())?;
    let real = canonical(Path::new(path.trim())).map_err(|_| "This folder doesn't exist any more.".to_string())?;
    if real.starts_with(&root) {
        Ok(real)
    } else {
        Err(format!(
            "This pane shows only {} from an agent's workspace, and {} is outside it: a link in the workspace leads out of it. Nothing outside the workspace is shown here.",
            root.display(),
            real.display()
        ))
    }
}

/// The refusal for a change asked of a confined pane.
pub(crate) const READ_ONLY: &str = "This folder is shown read-only from an agent's workspace: nothing in it can be changed from this pane.";

/// Refuses a change from the pane `block_id` when it is confined.
pub(crate) fn refuse_change(state: &AppState, block_id: Option<&str>) -> Result<(), String> {
    match block_id {
        Some(block) if root_of(state, block).is_some() => Err(READ_ONLY.to_string()),
        _ => Ok(()),
    }
}

/// A listing that failed because the folder lies outside the pane's root.
pub(crate) fn outside_listing(path: &str, message: String) -> FsListResult {
    FsListResult {
        path: path.to_string(),
        entries: Vec::new(),
        cursor: None,
        error: Some(FsError { kind: FsErrorKind::PermissionDenied, message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> (PathBuf, PathBuf, PathBuf) {
        let top = std::env::temp_dir().join(format!("agentmux-froot-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
        let root = top.join("ws");
        let outside = top.join("private");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "the person's own file").unwrap();
        (top, root, outside)
    }

    #[test]
    fn a_folder_inside_the_root_is_allowed_and_one_outside_is_refused() {
        let (top, root, outside) = folders();
        assert!(confine(&root, &root.join("src").to_string_lossy()).is_ok());
        assert!(confine(&root, &root.to_string_lossy()).is_ok());
        assert!(confine(&root, &outside.to_string_lossy()).unwrap_err().contains("outside it"));
        assert!(confine(&root, &root.join("..").join("private").to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(top);
    }

    /// The folder was inside the root when the pane opened and a link was put in its place after.
    #[test]
    fn a_symlink_swapped_in_after_the_pane_opened_is_refused() {
        let (top, root, outside) = folders();
        let dir = root.join("src");
        assert!(confine(&root, &dir.to_string_lossy()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, &dir).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, &dir).is_ok();
        if linked {
            let err = confine(&root, &dir.to_string_lossy()).unwrap_err();
            assert!(err.contains("outside it"), "{err}");
        } else {
            eprintln!("symlinks are not allowed here: the swapped-symlink check was skipped");
        }
        let _ = std::fs::remove_dir_all(top);
    }

    /// A Windows junction needs no privilege: made with `mklink /J`.
    #[cfg(windows)]
    #[test]
    fn a_junction_swapped_in_after_the_pane_opened_is_refused() {
        let (top, root, outside) = folders();
        let dir = root.join("src");
        assert!(confine(&root, &dir.to_string_lossy()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &dir.to_string_lossy(), &outside.to_string_lossy()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(made, "mklink /J failed");
        let err = confine(&root, &dir.to_string_lossy()).unwrap_err();
        assert!(err.contains("outside it"), "{err}");
        let _ = std::fs::remove_dir_all(top);
    }

    #[test]
    fn a_missing_folder_is_not_reported_as_outside() {
        let (top, root, _outside) = folders();
        let err = confine(&root, &root.join("gone").to_string_lossy()).unwrap_err();
        assert!(err.contains("doesn't exist"), "{err}");
        let _ = std::fs::remove_dir_all(top);
    }
}
