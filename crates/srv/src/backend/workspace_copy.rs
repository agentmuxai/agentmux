// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copying one file out of a container agent's workspace, read-only, to a folder of srv's own.
//!
//! The workspace is the agent's to change while it is read, so the file is opened by walking down from the workspace
//! folder without following a link (`widget_beneath`); elsewhere by checking, after the open, that it is the file
//! the path names inside the workspace.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// `file`, which lies under `root` (a canonical path), opened to read.
fn open_beneath(root: &Path, file: &Path) -> Result<std::fs::File, String> {
    #[cfg(unix)]
    return super::widget_beneath::file_beneath(root, file).map_err(|e| format!("{} can't be read from the workspace: {e}", file.display()));
    #[cfg(not(unix))]
    return super::widget_packages::open_within(file, Some(root));
}

/// Copies `file` (under `root`) into the folder `into`, which is made, under its own name, read-only; at most `limit`
/// bytes. Returns the copy's path.
pub fn copy_file(root: &Path, file: &Path, into: &Path, limit: u64) -> Result<PathBuf, String> {
    let name = file.file_name().ok_or_else(|| format!("{} has no file name", file.display()))?;
    let mut source = open_beneath(root, file)?;
    let size = source.metadata().map_err(|e| e.to_string())?.len();
    if size > limit {
        return Err(format!("{} is {} MB, over the {} MB a snapshot can be", file.display(), size.div_ceil(1_000_000), limit / 1_000_000));
    }
    std::fs::create_dir_all(into).map_err(|e| format!("can't make the snapshot folder: {e}"))?;
    let copy = into.join(name);
    let mut out = std::fs::File::create(&copy).map_err(|e| format!("can't write the snapshot: {e}"))?;
    // The file may have grown since it was measured: read no more than the limit, and refuse one that goes past it.
    let written = std::io::copy(&mut (&mut source).take(limit + 1), &mut out).map_err(|e| format!("can't copy {}: {e}", file.display()))?;
    drop(out);
    if written > limit {
        let _ = std::fs::remove_file(&copy);
        return Err(format!("{} grew past the {} MB a snapshot can be", file.display(), limit / 1_000_000));
    }
    let mut permissions = std::fs::metadata(&copy).map_err(|e| e.to_string())?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&copy, permissions).map_err(|e| e.to_string())?;
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws").canonicalize().unwrap_or_else(|_| {
            std::fs::create_dir_all(tmp.path().join("ws")).unwrap();
            tmp.path().join("ws").canonicalize().unwrap()
        });
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "outside").unwrap();
        (tmp, ws)
    }

    #[test]
    fn a_file_is_copied_read_only_under_its_own_name() {
        let (tmp, ws) = workspace();
        let into = tmp.path().join("snap");
        let copy = copy_file(&ws, &ws.join("src/main.rs"), &into, 1000).unwrap();
        assert_eq!(copy, into.join("main.rs"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "fn main() {}");
        assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
    }

    #[test]
    fn a_file_over_the_limit_is_refused_and_leaves_nothing() {
        let (tmp, ws) = workspace();
        std::fs::write(ws.join("big.bin"), vec![0u8; 5000]).unwrap();
        let into = tmp.path().join("snap");
        let err = copy_file(&ws, &ws.join("big.bin"), &into, 1000).unwrap_err();
        assert!(err.contains("a snapshot can be"), "{err}");
        assert!(!into.join("big.bin").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_workspace_is_not_followed() {
        let (tmp, ws) = workspace();
        std::os::unix::fs::symlink(tmp.path().join("secret.txt"), ws.join("link.txt")).unwrap();
        assert!(copy_file(&ws, &ws.join("link.txt"), &tmp.path().join("snap"), 1000).is_err());
        // A folder on the way that is a link is not entered either.
        std::os::unix::fs::symlink(tmp.path(), ws.join("up")).unwrap();
        assert!(copy_file(&ws, &ws.join("up/secret.txt"), &tmp.path().join("snap"), 1000).is_err());
        assert!(!tmp.path().join("snap").exists(), "nothing was made for a refused file");
    }

    #[test]
    fn a_path_outside_the_workspace_is_refused() {
        let (tmp, ws) = workspace();
        assert!(copy_file(&ws, &tmp.path().join("secret.txt"), &tmp.path().join("snap"), 1000).is_err());
    }
}
