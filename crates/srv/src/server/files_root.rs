// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Files pane confined to one folder: a container agent's folder, shown to the person.
//!
//! A container agent's `/workspace` is a folder of the host that the agent can change while a window reads
//! it, so a link or junction it swaps in later could make a Files pane show the person's own files, which
//! the agent could then read back from a screenshot of the pane. So such a pane carries `files:root`, the
//! canonical host folder it was opened on, and srv checks it on the requests the pane makes that name it
//! (the listing, the watch, the preview stream, opening in the Editor, and the changes): the path is
//! resolved again at each access, links and junctions included, and refused unless it lies inside the
//! root. The pane lists, navigates, previews and opens files in the Editor; it changes nothing.
//!

use std::path::{Path, PathBuf};

use crate::backend::rpc_types::{CommandPaneOpenData, FsError, FsErrorKind, FsListResult};
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
    let real = canonical(Path::new(path.trim())).map_err(|_| "That doesn't exist any more.".to_string())?;
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

/// `path` as the pane `block_id` may use it: for a confined pane, resolved now and inside its root (else
/// the reason); for any other, as asked.
pub(crate) async fn bound_path(state: &AppState, block_id: Option<&str>, path: &str) -> Result<String, String> {
    let Some(root) = block_id.and_then(|b| root_of(state, b)) else {
        return Ok(path.to_string());
    };
    let asked = path.to_string();
    tokio::task::spawn_blocking(move || confine(&root, &asked))
        .await
        .map_err(|e| format!("filesystem task failed: {e}"))?
        .map(|real| real.to_string_lossy().into_owned())
}

/// Whether `block_id` is a pane confined to a folder.
pub(crate) fn is_confined(state: &AppState, block_id: Option<&str>) -> bool {
    block_id.is_some_and(|b| root_of(state, b).is_some())
}

/// The refusal for a change asked of a confined pane.
pub(crate) const READ_ONLY: &str = "This folder is shown read-only from an agent's workspace: nothing in it can be changed from this pane.";

/// The refusal for handing a file to another program from a confined pane.
pub(crate) const NO_PROGRAMS: &str = "This folder is shown read-only from an agent's workspace: nothing in it can be opened with another program from this pane.";

/// Refuses a change from the pane `block_id` when it is confined.
pub(crate) fn refuse_change(state: &AppState, block_id: Option<&str>) -> Result<(), String> {
    match block_id {
        Some(block) if root_of(state, block).is_some() => Err(READ_ONLY.to_string()),
        _ => Ok(()),
    }
}

/// `pane.open` from a confined pane: only a file inside its folder, opened in an Editor. Its path is
/// replaced by the resolved one.
pub(crate) async fn bind_pane_open(state: &AppState, cmd: &mut CommandPaneOpenData) -> Result<(), String> {
    let from = [cmd.split_reference_block_id.as_deref(), cmd.stack_onto_block_id.as_deref()]
        .into_iter()
        .flatten()
        .find(|b| root_of(state, b).is_some())
        .map(str::to_string);
    let Some(from) = from else { return Ok(()) };
    let remote = cmd.connection.as_deref().map(str::trim).is_some_and(|c| !c.is_empty() && c != "local");
    let Some(file) = cmd.file.clone().filter(|_| cmd.view == "editor" && cmd.meta.is_none() && !remote) else {
        return Err("From this folder, shown read-only from an agent's workspace, only a file inside it can be opened, in an Editor.".to_string());
    };
    cmd.file = Some(bound_path(state, Some(&from), &file).await?);
    Ok(())
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

    /// A state holding a Files pane confined to `root`, and one that is not.
    fn state_with_panes(root: &Path) -> AppState {
        let state = crate::server::tests::test_state();
        let mut meta = crate::backend::obj::MetaMapType::default();
        meta.insert(FILES_ROOT_META_KEY.into(), root.to_string_lossy().into_owned().into());
        let mut confined = crate::backend::obj::Block { oid: "confined".into(), meta, ..Default::default() };
        state.mstore.insert(&mut confined).unwrap();
        let mut open = crate::backend::obj::Block { oid: "open".into(), ..Default::default() };
        state.mstore.insert(&mut open).unwrap();
        state
    }

    #[tokio::test]
    async fn a_confined_pane_gets_only_paths_inside_its_root() {
        let (top, root, outside) = folders();
        let state = state_with_panes(&root);
        let inside = root.join("src").to_string_lossy().into_owned();
        let secret = outside.join("secret.txt").to_string_lossy().into_owned();
        assert!(bound_path(&state, Some("confined"), &inside).await.is_ok());
        assert!(bound_path(&state, Some("confined"), &secret).await.unwrap_err().contains("outside it"));
        // A pane that is not confined, and a request naming no pane, are as before.
        assert_eq!(bound_path(&state, Some("open"), &secret).await.unwrap(), secret);
        assert_eq!(bound_path(&state, None, &secret).await.unwrap(), secret);
        assert!(is_confined(&state, Some("confined")));
        assert!(!is_confined(&state, Some("open")) && !is_confined(&state, None));
        let _ = std::fs::remove_dir_all(top);
    }

    #[tokio::test]
    async fn a_confined_pane_hands_nothing_to_another_program() {
        let (top, root, _outside) = folders();
        let file = root.join("src").join("a.sh");
        std::fs::write(&file, "x").unwrap();
        let state = state_with_panes(&root);
        for reveal in [false, true] {
            let cmd = crate::backend::rpc_types::FsPathReq {
                path: file.to_string_lossy().into_owned(),
                block_id: Some("confined".into()),
            };
            let err = crate::server::fs_handlers::open_or_reveal_bound(&state, cmd, reveal).await.unwrap_err();
            assert_eq!(err, NO_PROGRAMS);
        }
        let _ = std::fs::remove_dir_all(top);
    }

    #[tokio::test]
    async fn pane_open_from_a_confined_pane_is_one_file_inside_it_in_an_editor() {
        let (top, root, outside) = folders();
        std::fs::write(root.join("src").join("a.txt"), "x").unwrap();
        let state = state_with_panes(&root);
        let open = |view: &str, file: &std::path::Path| CommandPaneOpenData {
            view: view.into(),
            file: Some(file.to_string_lossy().into_owned()),
            split_reference_block_id: Some("confined".into()),
            ..Default::default()
        };
        let mut ok = open("editor", &root.join("src").join("a.txt"));
        bind_pane_open(&state, &mut ok).await.unwrap();
        assert!(ok.file.unwrap().ends_with("a.txt"));
        let mut elsewhere = open("editor", &outside.join("secret.txt"));
        assert!(bind_pane_open(&state, &mut elsewhere).await.unwrap_err().contains("outside it"));
        let mut terminal = open("term", &root);
        assert!(bind_pane_open(&state, &mut terminal).await.is_err());
        // From a pane that is not confined, nothing changes.
        let mut free = open("editor", &outside.join("secret.txt"));
        free.split_reference_block_id = Some("open".into());
        bind_pane_open(&state, &mut free).await.unwrap();
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
