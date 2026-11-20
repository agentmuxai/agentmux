// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A file of a container agent's folder, opened by the person: the read-only copy, never the path.
//!
//! A container agent can change its folder while a window reads it, so an Editor given a path in it reads
//! whatever the agent puts there later, as the person. Two opens the person makes lead into such a folder: a
//! click on a file in a Files pane confined to it (`files_root.rs`), and a click on a link in a message
//! (`from_link`). Both get what the agent's own `pane.open` gets (`container_pane_snapshot.rs`): a file opens
//! as a read-only copy that goes with its pane, and a linked folder opens as a Files pane confined to it. A
//! link that leads out of the agent's folder, or one whose folder can't be told from an agent's, is refused
//! with the reason.

use std::path::{Path, PathBuf};

use crate::backend::fs_ops::container_folders::{self, Place};
use crate::backend::rpc_types::CommandPaneOpenData;
use crate::server::container_pane_snapshot::{self, Snapshot};
use crate::server::AppState;

/// The refusal for a link whose folder can't be told from a container agent's.
const NOT_KNOWN: &str = "AgentMux can't tell right now whether this link leads into a container agent's folder, so it isn't opened. Try again in a moment, or open it from the Files pane.";

/// The refusal for a link that is written inside a container agent's folder and resolves outside it.
const LEADS_OUT: &str = "This link is in a container agent's folder and leads out of it: a link the agent made points elsewhere. Nothing outside the agent's folder is opened from it.";

type Turned = (CommandPaneOpenData, Option<Snapshot>);

/// `pane.open` where it leads into a container agent's folder: `cmd` is turned to open the read-only copy (or
/// the confined Files pane). The copy taken is returned, for the caller to note to the pane that opens.
pub(crate) async fn bind(state: &AppState, cmd: &mut CommandPaneOpenData) -> Result<Option<Snapshot>, String> {
    let from_pane = crate::server::files_root::bind_pane_open(state, cmd).await?;
    if from_pane.is_none() && cmd.from_link != Some(true) {
        return Ok(None);
    }
    let asked = cmd.clone();
    // Reads and copies a file: off the async workers.
    let turned = tokio::task::spawn_blocking(move || {
        let snapshots = container_pane_snapshot::root();
        turn(asked, from_pane, snapshots.as_deref(), &container_folders::place)
    })
    .await
    .map_err(|e| format!("filesystem task failed: {e}"))?;
    let (turned, snapshot) = turned?;
    *cmd = turned;
    Ok(snapshot)
}

/// What [`bind`] does with the disk. `from_pane` is the folder of the confined pane the open came from, whose
/// check left the file's resolved path in `cmd`; `snapshots` is where the copies are kept.
fn turn(mut cmd: CommandPaneOpenData, from_pane: Option<PathBuf>, snapshots: Option<&Path>, place: &dyn Fn(&str) -> Place) -> Result<Turned, String> {
    let Some(root) = from_pane else { return linked(cmd, snapshots, place) };
    let within = crate::server::browser_uploads::canonical(&root).map_err(|_| "The folder this pane shows can't be read any more.".to_string())?;
    let file = PathBuf::from(cmd.file.clone().unwrap_or_default());
    let of = called(&within, &file, place);
    let snapshot = container_pane_snapshot::for_person(&mut cmd, snapshots, &within, &file, of)?;
    Ok((cmd, Some(snapshot)))
}

/// What a file's pane calls it: the container's own name for it where that is known, else its path from `within`.
fn called(within: &Path, file: &Path, place: &dyn Fn(&str) -> Place) -> String {
    if let Place::Inside(inside) = place(&file.to_string_lossy()) {
        return inside.dir;
    }
    let rest = file.strip_prefix(within).unwrap_or(file);
    rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// A link in a message. A path no container agent can write opens as asked, and so does anything that is not
/// a file or folder of this computer.
fn linked(mut cmd: CommandPaneOpenData, snapshots: Option<&Path>, place: &dyn Fn(&str) -> Place) -> Result<Turned, String> {
    let local = cmd.connection.as_deref().map(str::trim).is_none_or(|c| c.is_empty() || c == "local");
    let path = cmd.file.as_deref().map(str::trim).filter(|f| !f.is_empty()).and_then(|f| crate::backend::fs_ops::resolve_request_path(f).ok());
    let (true, None, "editor" | "files", Some(path)) = (local, &cmd.meta, cmd.view.as_str(), path) else {
        return Ok((cmd, None));
    };
    let inside = match place(&path.to_string_lossy()) {
        Place::Host => return Ok((cmd, None)),
        Place::Unknown => return Err(NOT_KNOWN.to_string()),
        Place::Inside(inside) => inside,
    };
    let canonical = crate::server::browser_uploads::canonical;
    let within = canonical(Path::new(inside.bound())).map_err(|_| "The agent's folder this link leads into can't be read.".to_string())?;
    let real = canonical(&path).map_err(|_| "What this link leads to doesn't exist any more.".to_string())?;
    if !real.starts_with(&within) {
        return Err(LEADS_OUT.to_string());
    }
    if real.is_dir() {
        // The meta is used as it is: a Files pane that lists and navigates inside the folder and changes nothing.
        cmd.view = "files".into();
        cmd.file = None;
        cmd.meta = Some(crate::server::files_root::confined_meta(&real, &inside.dir));
        return Ok((cmd, None));
    }
    cmd.view = "editor".into();
    let snapshot = container_pane_snapshot::for_person(&mut cmd, snapshots, &within, &real, inside.dir)?;
    Ok((cmd, Some(snapshot)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent's folder with a file in it, the person's own folder beside it, and where the copies go.
    fn folders() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let (ws, private, snaps) = (tmp.path().join("ws"), tmp.path().join("private"), tmp.path().join("snaps"));
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(ws.join("src").join("a.txt"), "the agent's file").unwrap();
        std::fs::write(private.join("a.txt"), "the person's own file").unwrap();
        (tmp, ws, private, snaps)
    }

    /// Puts a link to the folder `to` where the folder `at` was; false where the platform refuses one.
    fn swap_for_a_link(at: &Path, to: &Path) -> bool {
        std::fs::remove_dir_all(at).unwrap();
        #[cfg(unix)]
        return std::os::unix::fs::symlink(to, at).is_ok();
        #[cfg(windows)]
        return std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &at.to_string_lossy(), &to.to_string_lossy()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    }

    fn link(view: &str, path: &Path) -> CommandPaneOpenData {
        CommandPaneOpenData { view: view.into(), file: Some(path.to_string_lossy().into_owned()), from_link: Some(true), ..Default::default() }
    }

    /// `ws` is bound into a container at `/workspace`.
    fn agents(ws: &Path) -> impl Fn(&str) -> Place {
        let ws = ws.to_path_buf();
        move |asked| container_folders::inside_one(Path::new(asked), &ws, "agentmux-aria", "/workspace").map_or(Place::Host, Place::Inside)
    }

    #[test]
    fn a_file_clicked_in_a_confined_pane_opens_as_a_copy_that_a_later_swap_does_not_change() {
        let (_tmp, ws, private, snaps) = folders();
        let within = crate::server::browser_uploads::canonical(&ws).unwrap();
        let cmd = CommandPaneOpenData { view: "editor".into(), file: Some(within.join("src").join("a.txt").to_string_lossy().into_owned()), ..Default::default() };
        let (open, snapshot) = turn(cmd, Some(ws.clone()), Some(&snaps), &|_| Place::Host).unwrap();
        let copy = PathBuf::from(open.file.unwrap());
        assert!(copy.starts_with(&snaps) && !copy.starts_with(&within), "the pane is given the copy, never the folder's path");
        assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
        assert!(open.title.unwrap().starts_with("src/a.txt (read-only snapshot, "));
        if swap_for_a_link(&ws.join("src"), &private) {
            assert_eq!(std::fs::read_to_string(&copy).unwrap(), "the agent's file", "what is put in the file's place later is not read");
        }
        snapshot.expect("a copy").discard();
        assert!(!copy.exists(), "a pane that did not open leaves no copy");
    }

    #[test]
    fn a_file_swapped_for_a_link_before_the_copy_is_not_read() {
        let (_tmp, ws, private, snaps) = folders();
        let within = crate::server::browser_uploads::canonical(&ws).unwrap();
        let cmd = CommandPaneOpenData { view: "editor".into(), file: Some(within.join("src").join("a.txt").to_string_lossy().into_owned()), ..Default::default() };
        if swap_for_a_link(&ws.join("src"), &private) {
            assert!(turn(cmd, Some(ws.clone()), Some(&snaps), &|_| Place::Host).is_err());
            assert_eq!(std::fs::read_dir(&snaps).map(|d| d.count()).unwrap_or(0), 0, "a refused file leaves no copy");
        }
    }

    #[test]
    fn a_linked_file_in_a_container_agents_folder_opens_as_a_copy_named_as_the_container_names_it() {
        let (_tmp, ws, _private, snaps) = folders();
        let place = agents(&ws);
        let (open, snapshot) = turn(link("editor", &ws.join("src").join("a.txt")), None, Some(&snaps), &place).unwrap();
        let copy = PathBuf::from(open.file.unwrap());
        assert!(copy.starts_with(&snaps), "{}", copy.display());
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "the agent's file");
        assert!(open.title.unwrap().starts_with("/workspace/src/a.txt (read-only snapshot, "));
        assert_eq!(snapshot.expect("a copy").of, "/workspace/src/a.txt");
    }

    #[test]
    fn a_linked_folder_in_a_container_agents_folder_opens_as_a_files_pane_confined_to_it() {
        let (_tmp, ws, _private, snaps) = folders();
        let place = agents(&ws);
        // The window takes a path for a file when it can't list it: what is on the disk decides.
        for view in ["files", "editor"] {
            let (open, snapshot) = turn(link(view, &ws.join("src")), None, Some(&snaps), &place).unwrap();
            assert!(snapshot.is_none() && open.file.is_none(), "a folder is not copied");
            let want = crate::server::browser_uploads::canonical(&ws.join("src")).unwrap().to_string_lossy().into_owned();
            let meta = open.meta.expect("the pane's own meta");
            assert_eq!((open.view.as_str(), meta["view"].as_str()), ("files", Some("files")));
            assert_eq!(meta[crate::server::files_root::FILES_ROOT_META_KEY], want.as_str());
            assert_eq!(meta["files:path"], want.as_str());
            assert!(meta["frame:title"].as_str().unwrap().starts_with("/workspace/src (read-only"));
        }
    }

    #[test]
    fn a_link_out_of_a_container_agents_folder_is_refused_with_the_reason() {
        let (_tmp, ws, private, snaps) = folders();
        let place = agents(&ws);
        if swap_for_a_link(&ws.join("src"), &private) {
            for (view, path) in [("editor", ws.join("src").join("a.txt")), ("files", ws.join("src"))] {
                let err = turn(link(view, &path), None, Some(&snaps), &place).unwrap_err();
                assert_eq!(err, LEADS_OUT);
            }
            assert_eq!(std::fs::read_dir(&snaps).map(|d| d.count()).unwrap_or(0), 0);
        }
    }

    #[test]
    fn a_link_is_refused_while_its_folders_owner_cannot_be_told() {
        let (_tmp, ws, _private, snaps) = folders();
        let err = turn(link("editor", &ws.join("src").join("a.txt")), None, Some(&snaps), &|_| Place::Unknown).unwrap_err();
        assert_eq!(err, NOT_KNOWN);
    }

    #[test]
    fn a_linked_path_no_container_agent_can_write_opens_as_asked() {
        let (_tmp, ws, private, snaps) = folders();
        let place = agents(&ws);
        let asked = link("editor", &private.join("a.txt"));
        let (open, snapshot) = turn(asked.clone(), None, Some(&snaps), &place).unwrap();
        assert!(snapshot.is_none());
        assert_eq!((open.file, open.view, open.meta.is_none()), (asked.file, asked.view, true));
        // A path on another machine, and an open that brings its own meta, are not this computer's files.
        let mut remote = link("editor", &ws.join("src").join("a.txt"));
        remote.connection = Some("user@host".into());
        assert!(turn(remote, None, Some(&snaps), &place).unwrap().1.is_none());
    }

    #[tokio::test]
    async fn an_open_without_a_link_or_a_confined_pane_is_left_as_it_is() {
        let (_tmp, ws, _private, _snaps) = folders();
        let state = crate::server::tests::test_state();
        let asked = CommandPaneOpenData { view: "editor".into(), file: Some(ws.join("src").join("a.txt").to_string_lossy().into_owned()), ..Default::default() };
        let mut cmd = asked.clone();
        assert!(bind(&state, &mut cmd).await.unwrap().is_none());
        assert_eq!(cmd.file, asked.file, "an open the person makes any other way is not changed");
    }
}
