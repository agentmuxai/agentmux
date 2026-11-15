// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container agent's `/workspace` as a folder of the host.
//!
//! A container agent's machine is its container, whose `/workspace` is a folder of the host. A route
//! that takes a path from it (`widgets/install`) names the folder as the agent sees it, and srv maps
//! that to the host's, after resolving symlinks, and refuses anything outside: a host path outside the
//! workspace would reach the user's `~/.ssh` or `~/.aws`.

use std::path::{Path, PathBuf};

use crate::server::AppState;

/// Where the container's `/workspace` is on the host: the block's `cmd:cwd`, as the agent was started in it.
pub(crate) fn workspace_of(state: &AppState, block_id: &str) -> Option<PathBuf> {
    let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten()?;
    let cwd = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
    if cwd.trim().is_empty() {
        return None;
    }
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(&cwd);
    Some(std::path::absolute(&expanded).unwrap_or_else(|_| PathBuf::from(expanded)))
}

/// `raw` as a path on the host, inside `workspace`. `/workspace/...` is the container's name for it;
/// a relative path is relative to it; a host path is accepted if it is inside it. Symlinks are resolved first.
pub(crate) fn inside(workspace: &Path, raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let joined = host_spelling(workspace, raw);
    let root = crate::server::browser_uploads::canonical(workspace).map_err(|e| format!("the agent's workspace can't be read: {e}"))?;
    let real = crate::server::browser_uploads::canonical(&joined).map_err(|_| format!("no such file or folder: {raw}"))?;
    if real.starts_with(&root) {
        Ok(real.to_string_lossy().into_owned())
    } else {
        Err(format!("{raw} is outside this agent's workspace. A container agent can use only files in /workspace: copy the file there first."))
    }
}

/// `raw` as the host spells it, with nothing resolved yet: `/workspace/...` and a relative path lie
/// under `workspace`, and a host path stands as given.
fn host_spelling(workspace: &Path, raw: &str) -> PathBuf {
    match raw.strip_prefix("/workspace") {
        Some("") => workspace.to_path_buf(),
        Some(rest) if rest.starts_with('/') => workspace.join(rest.trim_start_matches('/')),
        _ if Path::new(raw).is_absolute() => PathBuf::from(raw),
        _ => workspace.join(raw),
    }
}

/// The entry `raw` names, itself: a link is not followed, because a rename acts on the name in its
/// folder. The folder is resolved (links included) and must lie inside `workspace`. Returns the entry
/// on the host, and the container's name for its folder (`/workspace/...`).
///
/// The folder is checked now and renamed in a moment later, as every route that takes a workspace
/// path does: the entry handed on is spelled through the resolved folder, with no link left in it.
pub(crate) fn entry_inside(workspace: &Path, raw: &str) -> Result<(PathBuf, String), String> {
    let raw = raw.trim();
    let joined = host_spelling(workspace, raw);
    let outside =
        || format!("{raw} is outside this agent's workspace. A container agent can rename only what is in /workspace: move the file there first.");
    // No name of its own (`/workspace/a/..`): not an entry of a folder.
    let (Some(folder), Some(name)) = (joined.parent(), joined.file_name()) else {
        return Err(outside());
    };
    let root = crate::server::browser_uploads::canonical(workspace).map_err(|e| format!("the agent's workspace can't be read: {e}"))?;
    let folder = crate::server::browser_uploads::canonical(folder).map_err(|_| format!("no such file or folder: {raw}"))?;
    // The workspace folder itself has its folder outside, so it is refused here too.
    let Ok(below) = folder.strip_prefix(&root) else {
        return Err(outside());
    };
    let entry = folder.join(name);
    if entry.symlink_metadata().is_err() {
        return Err(format!("no such file or folder: {raw}"));
    }
    let mut named = String::from("/workspace");
    for part in below.components() {
        named.push('/');
        named.push_str(&part.as_os_str().to_string_lossy());
    }
    Ok((entry, named))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("agentmux-cws-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("secret.txt"), "outside").unwrap();
        (root, ws)
    }

    #[test]
    fn the_containers_name_for_its_workspace_is_the_hosts_folder() {
        let (root, ws) = workspace();
        let want = crate::server::browser_uploads::canonical(&ws.join("src/main.rs")).unwrap().to_string_lossy().into_owned();
        for given in ["/workspace/src/main.rs", "src/main.rs", &ws.join("src/main.rs").to_string_lossy()] {
            assert_eq!(inside(&ws, given).as_deref(), Ok(want.as_str()), "{given}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_path_outside_the_workspace_is_refused_however_it_is_written() {
        let (root, ws) = workspace();
        let outside = root.join("secret.txt");
        for given in [
            "/workspace/../secret.txt".to_string(),
            "../secret.txt".to_string(),
            outside.to_string_lossy().into_owned(),
            "/etc/passwd".to_string(),
            "/workspacefoo".to_string(),
            "~/.ssh/id_rsa".to_string(),
        ] {
            let err = inside(&ws, &given).expect_err(&given);
            assert!(err.contains("outside this agent's workspace") || err.contains("no such file"), "{given}: {err}");
        }
        // A symlink inside the workspace that points out is outside.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, ws.join("link.txt")).unwrap();
            assert!(inside(&ws, "/workspace/link.txt").unwrap_err().contains("outside this agent's workspace"));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_entry_is_named_in_its_resolved_folder_and_the_container_names_that_folder() {
        let (root, ws) = workspace();
        let real = crate::server::browser_uploads::canonical(&ws).unwrap();
        for given in ["/workspace/src/main.rs", "src/main.rs", &ws.join("src/main.rs").to_string_lossy()] {
            assert_eq!(entry_inside(&ws, given), Ok((real.join("src").join("main.rs"), "/workspace/src".to_string())), "{given}");
        }
        // A folder is an entry too; one at the top is in /workspace.
        assert_eq!(entry_inside(&ws, "/workspace/src"), Ok((real.join("src"), "/workspace".to_string())));
        assert!(entry_inside(&ws, "/workspace/src/gone.rs").unwrap_err().contains("no such file"));
        for given in ["/workspace", "/workspace/..", "/workspace/src/..", "../secret.txt", "/workspace/../secret.txt"] {
            let err = entry_inside(&ws, given).expect_err(given);
            assert!(err.contains("outside this agent's workspace"), "{given}: {err}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
