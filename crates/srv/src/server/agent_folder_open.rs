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
    // A file or a link clicked in a container agent's message: found in the agent's own folders, whatever else the open names.
    // For a pane that is not a container agent's the open goes on as asked.
    if let Some(pane) = cmd.message_of.take() {
        if let Some(meta) = crate::server::agent_media_files::pane_meta(state, &pane) {
            let asked = cmd.clone();
            let turned = tokio::task::spawn_blocking(move || {
                let snapshots = container_pane_snapshot::root();
                from_message(asked, &meta, snapshots.as_deref())
            })
            .await
            .map_err(|e| format!("filesystem task failed: {e}"))?;
            let (turned, snapshot) = turned?;
            *cmd = turned;
            return Ok(snapshot);
        }
    }
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

/// A file or a link clicked in a message of the container agent whose pane is `meta`: the file the agent's own folders
/// hold under that path, as a read-only copy named as the container names it, or a folder of them, as a Files pane
/// confined to it. A path anywhere else, and one that leads out of the folder through a link, is refused with the
/// reason; so is an open on another computer.
fn from_message(mut cmd: CommandPaneOpenData, meta: &crate::backend::obj::MetaMapType, snapshots: Option<&Path>) -> Result<Turned, String> {
    if cmd.connection.as_deref().map(str::trim).is_some_and(|c| !c.is_empty() && c != "local") {
        return Err("A file of an agent's message is on this computer; it isn't opened from another one.".to_string());
    }
    let asked = cmd.file.clone().unwrap_or_default();
    let found = crate::server::agent_media_files::resolve(meta, &asked)?;
    // The copy does not change when the file does, so there is no folder to follow; and the open is no link.
    cmd.follow_folder = None;
    cmd.from_link = None;
    if found.file.is_dir() {
        // A Files pane that lists and navigates inside the folder and changes nothing.
        cmd.view = "files".into();
        cmd.file = None;
        cmd.meta = Some(crate::server::files_root::confined_meta(&found.file, &found.of));
        return Ok((cmd, None));
    }
    cmd.view = "editor".into();
    cmd.meta = None;
    let snapshot = container_pane_snapshot::for_person(&mut cmd, snapshots, &found.root, &found.file, found.of)?;
    Ok((cmd, Some(snapshot)))
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

    /// The pane of a container agent whose workspace is `ws`.
    fn agent_pane(ws: &Path) -> crate::backend::obj::MetaMapType {
        let mut meta = crate::backend::obj::MetaMapType::default();
        meta.insert("agentMode".into(), "container".into());
        meta.insert(crate::backend::blockcontroller::META_KEY_CMD_CWD.into(), ws.to_string_lossy().into_owned().into());
        meta
    }

    fn clicked(path: &str) -> CommandPaneOpenData {
        CommandPaneOpenData { view: "editor".into(), file: Some(path.into()), follow_folder: Some(true), message_of: Some("pane".into()), ..Default::default() }
    }

    #[test]
    fn a_picture_clicked_in_a_container_agents_message_opens_as_a_read_only_copy_named_as_the_container_names_it() {
        let (_tmp, ws, _private, snaps) = folders();
        std::fs::write(ws.join("pic.png"), "the agent's picture").unwrap();
        for asked in ["/workspace/pic.png".to_string(), format!("{}/pic.png", ws.to_string_lossy())] {
            let (open, snapshot) = from_message(clicked(&asked), &agent_pane(&ws), Some(&snaps)).unwrap();
            let snapshot = snapshot.unwrap();
            let copy = PathBuf::from(open.file.clone().unwrap());
            assert!(copy.starts_with(&snaps), "{asked}: the pane is given the copy, never the folder's path");
            assert_eq!(std::fs::read_to_string(&copy).unwrap(), "the agent's picture");
            assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
            assert!(open.title.clone().unwrap().starts_with("/workspace/pic.png (read-only snapshot, "));
            assert_eq!((open.view.as_str(), open.follow_folder, open.from_link, open.meta.is_none()), ("editor", None, None, true));
            assert_eq!(snapshot.of, "/workspace/pic.png");
        }
    }

    #[test]
    fn a_folder_linked_in_a_container_agents_message_opens_in_a_files_pane_confined_to_it() {
        let (_tmp, ws, private, snaps) = folders();
        let pane = agent_pane(&ws);
        for asked in ["/workspace/src".to_string(), format!("{}/src/", ws.to_string_lossy()), "/workspace".to_string()] {
            let (open, snapshot) = from_message(clicked(&asked), &pane, Some(&snaps)).unwrap();
            assert!(snapshot.is_none(), "{asked}: a folder is not copied");
            assert_eq!((open.view.as_str(), open.file.is_none(), open.from_link), ("files", true, None));
            assert!(open.meta.is_some(), "{asked}: the pane is confined to the folder");
        }
        // The person's own folder is not the agent's, however it is spelled.
        for asked in [private.to_string_lossy().into_owned(), "/workspace/../private".into(), "~".into()] {
            assert_eq!(from_message(clicked(&asked), &pane, Some(&snaps)).unwrap_err(), crate::server::agent_media_files::OUTSIDE, "{asked}");
        }
    }

    #[test]
    fn a_file_clicked_in_a_message_that_is_not_in_the_agents_folders_is_refused_and_leaves_no_copy() {
        let (_tmp, ws, private, snaps) = folders();
        let pane = agent_pane(&ws);
        for asked in [
            private.join("a.txt").to_string_lossy().into_owned(),
            "/workspace/../private/a.txt".into(),
            "~/a.txt".into(),
            "//host/share/a.png".into(),
            "C:/Users/me/a.png".into(),
            "/etc/hostname".into(),
        ] {
            let err = from_message(clicked(&asked), &pane, Some(&snaps)).unwrap_err();
            assert_eq!(err, crate::server::agent_media_files::OUTSIDE, "{asked}");
        }
        let mut elsewhere = clicked("/workspace/src/a.txt");
        elsewhere.connection = Some("user@host".into());
        assert!(from_message(elsewhere, &pane, Some(&snaps)).is_err(), "a file of another computer is not opened from a message");
        assert_eq!(std::fs::read_dir(&snaps).map(|d| d.count()).unwrap_or(0), 0);
    }

    #[test]
    fn a_link_that_leads_out_of_the_agents_folder_is_refused_when_clicked_in_a_message() {
        let (_tmp, ws, private, snaps) = folders();
        if swap_for_a_link(&ws.join("src"), &private) {
            let err = from_message(clicked("/workspace/src/a.txt"), &agent_pane(&ws), Some(&snaps)).unwrap_err();
            assert_eq!(err, crate::server::agent_media_files::LEADS_OUT);
            assert_eq!(std::fs::read_dir(&snaps).map(|d| d.count()).unwrap_or(0), 0);
        }
    }

    #[tokio::test]
    async fn a_click_in_a_pane_that_is_not_a_container_agents_opens_as_asked_and_the_field_is_not_kept() {
        let (_tmp, ws, private, _snaps) = folders();
        let state = crate::server::tests::test_state();
        let mut meta = agent_pane(&ws);
        meta.insert("agentMode".into(), "host".into());
        let mut block = crate::backend::obj::Block { oid: "pane".into(), meta, ..Default::default() };
        state.mstore.insert(&mut block).unwrap();
        let asked = private.join("a.txt").to_string_lossy().into_owned();
        let mut cmd = clicked(&asked);
        assert!(bind(&state, &mut cmd).await.unwrap().is_none());
        assert_eq!((cmd.file.as_deref(), cmd.message_of), (Some(asked.as_str()), None));
        // A pane that is not there is not a container agent's either.
        let mut gone = clicked(&asked);
        gone.message_of = Some("no-such-pane".into());
        assert!(bind(&state, &mut gone).await.unwrap().is_none());
        assert_eq!(gone.file.as_deref(), Some(asked.as_str()));
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
