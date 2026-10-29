// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `pane.open` from a container agent: a read-only snapshot of a file in its workspace.
//!
//! The pane is drawn by a window on the host, and a window reads whatever host path its pane is given, as the user. A
//! container agent's `/workspace` is a folder of the host the agent can change while a window reads it, so handing the
//! window a path in it would let a link swapped in later show the user's own files in a pane the agent asked for.
//! So the window is never given a path in the workspace: srv reads the file out of it without following a link,
//! keeps a read-only copy in a folder of its own, and opens the pane on the copy. The pane's title says what it is
//! (the container path and the time), the answer tells the agent to open it again to refresh, and the copy goes
//! when its pane closes (and any left over, when srv starts).
//!
//! The views that show no path (sysinfo, help, tower) open as they are. A terminal runs on the host, another machine
//! is reached as the user, a browser has its own tool, and a folder can't be a snapshot; each is refused with the way on.

use std::path::{Path, PathBuf};

use crate::backend::rpc_types::CommandPaneOpenData;

/// The editor reads no more than this itself.
const EDITOR_LIMIT: u64 = 10_000_000;
/// A picture or recording to look at.
const MEDIA_LIMIT: u64 = 50_000_000;
/// Snapshots on disk at once.
const MAX_SNAPSHOTS: usize = 40;
/// Names the pane a snapshot's folder belongs to.
const MARKER: &str = ".block";

/// A snapshot taken for a pane that is about to open.
#[derive(Debug)]
pub(crate) struct Snapshot {
    dir: PathBuf,
    /// What it is a snapshot of: the file's path in the container.
    pub of: String,
    /// When it was taken (local time, `HH:MM`).
    pub taken: String,
}

impl Snapshot {
    /// What the agent is told.
    pub fn note(&self) -> String {
        format!(
            "This is a read-only snapshot of {} taken at {}. It does not change when the file does: open it again to refresh it.",
            self.of, self.taken
        )
    }

    /// The pane opened: note which one it is, so closing it takes the copy with it.
    pub fn opened(&self, block_id: &str) {
        if let Err(e) = std::fs::write(self.dir.join(MARKER), block_id) {
            tracing::warn!(error = %e, "[container-snapshot] couldn't note the pane; the copy goes when srv next starts");
        }
    }

    /// The pane didn't open: the copy goes.
    pub fn discard(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Where the copies are kept.
fn root() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|p| p.data_dir.join("container-snapshots"))
}

/// Check a container agent's `pane.open` and, for a file, take its snapshot under `root`. Returns the request to open, with
/// nothing the caller set beyond the checked fields, and the snapshot if there is one.
pub(crate) fn prepare(workspace: &Path, root: &Path, req: CommandPaneOpenData) -> Result<(CommandPaneOpenData, Option<Snapshot>), String> {
    let limit = match req.view.as_str() {
        "editor" => EDITOR_LIMIT,
        "media" => MEDIA_LIMIT,
        "sysinfo" | "help" | "tower" => 0,
        "term" => return Err("a terminal pane runs on the host, so a container agent can't open one. Use your own shell in the container, or ask a native agent or your leader.".into()),
        "browser" => return Err("use the Browser tool to open a page: it opens a browser pane in the agent's own Incognito jar.".into()),
        v if v.starts_with("ext:") => return Err("a widget opens from the user's widget bar once they approve it; the Widget tool's open isn't available to a container agent yet.".into()),
        "files" => return Err("a folder can't be shown from a container yet: open a file in it (a read-only snapshot of the file is shown), or ask a native agent to open the folder.".into()),
        other => return Err(format!("INVALID_VIEW: a container agent can open editor, media, sysinfo, help and tower panes, not '{other}'")),
    };
    if req.connection.as_deref().map(str::trim).is_some_and(|c| !c.is_empty() && c != "local") {
        return Err("a pane on another machine reaches it as the user, so a container agent can't open one.".into());
    }
    // Only these fields survive: where the pane goes, how it looks, and the file, which becomes the copy.
    let mut open = CommandPaneOpenData {
        view: req.view.clone(),
        title: req.title.clone(),
        split_direction: req.split_direction.clone(),
        split_reference_block_id: req.split_reference_block_id.clone(),
        focus: req.focus,
        line: req.line,
        tree_expanded: req.tree_expanded,
        follow_folder: (req.view == "media").then_some(true),
        ..Default::default()
    };
    if limit == 0 {
        return Ok((open, None));
    }
    let raw = req.file.as_deref().map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| format!("MISSING_ARG: view={} requires 'file'", req.view))?;
    let real = super::container_workspace::inside(workspace, raw)?;
    let real = PathBuf::from(real);
    let workspace_real = crate::server::browser_uploads::canonical(workspace).map_err(|e| format!("the agent's workspace can't be read: {e}"))?;
    let of = container_path(&workspace_real, &real).ok_or_else(|| format!("{raw} is outside this agent's workspace"))?;
    if std::fs::read_dir(root).map(|d| d.count()).unwrap_or(0) >= MAX_SNAPSHOTS {
        return Err(format!("there are {MAX_SNAPSHOTS} snapshots open already: close some of those panes first"));
    }
    let dir = root.join(uuid::Uuid::new_v4().to_string());
    let copy = crate::backend::workspace_copy::copy_file(&workspace_real, &real, &dir, limit).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&dir);
    })?;
    let taken = chrono::Local::now().format("%H:%M").to_string();
    open.file = Some(copy.to_string_lossy().into_owned());
    open.title = Some(format!("{of} (read-only snapshot, {taken})"));
    Ok((open, Some(Snapshot { dir, of, taken })))
}

/// `real` as the container names it: `/workspace/...`.
fn container_path(workspace: &Path, real: &Path) -> Option<String> {
    let rel = real.strip_prefix(workspace).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    Some(if parts.is_empty() { "/workspace".to_string() } else { format!("/workspace/{}", parts.join("/")) })
}

/// For a container agent's `pane.open`: [`prepare`] under srv's snapshot folder.
pub(crate) fn open_for(
    state: &crate::server::AppState,
    block_id: &str,
    req: CommandPaneOpenData,
) -> Result<(CommandPaneOpenData, Option<Snapshot>), String> {
    let workspace = super::container_workspace::workspace_of(state, block_id).ok_or("this container agent has no workspace folder to open files from")?;
    let root = root().ok_or("srv has no folder to keep snapshots in")?;
    prepare(&workspace, &root, req)
}

/// The snapshot kept for `block_id`'s pane goes with it. One not yet noted to a pane is a pane still opening: left alone.
pub(crate) fn forget(block_id: &str) {
    if let Some(root) = root() {
        remove_where(&root, |noted| noted == Some(block_id));
    }
}

/// Removes every snapshot under `root` whose pane `gone` says is gone, given the pane it was noted to (none: not noted).
fn remove_where(root: &Path, gone: impl Fn(Option<&str>) -> bool) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let noted = std::fs::read_to_string(entry.path().join(MARKER)).ok().map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        if gone(noted.as_deref()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// When srv starts, nothing is opening: copies whose pane is no longer there, or was never noted, are removed.
pub(crate) fn sweep(pane_exists: impl Fn(&str) -> bool) {
    if let Some(root) = root() {
        remove_where(&root, |noted| noted.is_none_or(|b| !pane_exists(b)));
    }
}

/// Removes a snapshot when its pane (or the tab or workspace holding it) is deleted.
pub(crate) fn spawn_sweeper(mut events: tokio::sync::broadcast::Receiver<agentmux_common::ipc::Event>) {
    use agentmux_common::ipc::Event;
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(Event::BlockDeleted { block_id, .. }) => forget(&block_id),
                Ok(Event::TabDeleted { block_ids, .. } | Event::WorkspaceDeleted { block_ids, .. }) => block_ids.iter().for_each(|b| forget(b)),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(ws.join("docs")).unwrap();
        std::fs::write(ws.join("docs/notes.md"), "# notes").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "outside").unwrap();
        let root = tmp.path().join("snaps");
        (tmp, ws, root)
    }

    fn open(view: &str, file: Option<&str>) -> CommandPaneOpenData {
        CommandPaneOpenData { view: view.into(), file: file.map(str::to_string), ..Default::default() }
    }

    #[test]
    fn a_file_opens_as_a_read_only_copy_that_says_what_it_is() {
        let (_tmp, ws, root) = setup();
        for given in ["/workspace/docs/notes.md", "docs/notes.md"] {
            let (open, snap) = prepare(&ws, &root, CommandPaneOpenData { follow_folder: Some(false), ..self::open("editor", Some(given)) }).unwrap();
            let snap = snap.expect("a snapshot");
            let copy = PathBuf::from(open.file.clone().unwrap());
            assert!(copy.starts_with(&root) && !copy.starts_with(&ws), "the pane is given the copy, never the workspace");
            assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# notes");
            assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
            assert_eq!(snap.of, "/workspace/docs/notes.md");
            let title = open.title.unwrap();
            assert!(title.starts_with("/workspace/docs/notes.md (read-only snapshot, ") && !title.contains(root.to_str().unwrap()), "{title}");
            assert!(snap.note().contains("open it again to refresh"));
        }
    }

    #[test]
    fn only_the_checked_fields_survive_and_a_path_outside_is_refused() {
        let (tmp, ws, root) = setup();
        let mut req = open("media", Some("docs/notes.md"));
        req.url = Some("http://x".into());
        req.cwd = Some("/".into());
        req.meta = Some(Default::default());
        req.connection = Some("local".into());
        req.viewer = Some("text".into());
        let (out, _) = prepare(&ws, &root, req).unwrap();
        assert!(out.url.is_none() && out.cwd.is_none() && out.meta.is_none() && out.connection.is_none() && out.viewer.is_none());
        let outside = tmp.path().join("secret.txt");
        for given in ["../secret.txt".to_string(), "/etc/passwd".to_string(), outside.to_string_lossy().into_owned()] {
            assert!(prepare(&ws, &root, open("editor", Some(&given))).is_err(), "{given}");
        }
        let mut ssh = open("editor", Some("docs/notes.md"));
        ssh.connection = Some("user@host".into());
        assert!(prepare(&ws, &root, ssh).unwrap_err().contains("another machine"));
    }

    #[test]
    fn the_views_that_cannot_be_snapshots_say_why() {
        let (_tmp, ws, root) = setup();
        for view in ["sysinfo", "help", "tower"] {
            let (_, snap) = prepare(&ws, &root, open(view, None)).unwrap();
            assert!(snap.is_none(), "{view} shows no file");
        }
        assert!(prepare(&ws, &root, open("term", None)).unwrap_err().contains("runs on the host"));
        assert!(prepare(&ws, &root, open("browser", None)).unwrap_err().contains("Browser tool"));
        assert!(prepare(&ws, &root, open("files", Some("docs"))).unwrap_err().contains("folder"));
        assert!(prepare(&ws, &root, open("launcher", None)).unwrap_err().contains("INVALID_VIEW"));
        assert!(prepare(&ws, &root, open("editor", None)).unwrap_err().contains("MISSING_ARG"));
    }

    #[test]
    fn a_big_file_and_too_many_snapshots_are_refused_and_leave_nothing_behind() {
        let (_tmp, ws, root) = setup();
        std::fs::write(ws.join("big.md"), vec![b'x'; (EDITOR_LIMIT + 1) as usize]).unwrap();
        assert!(prepare(&ws, &root, open("editor", Some("big.md"))).unwrap_err().contains("a snapshot can be"));
        assert_eq!(std::fs::read_dir(&root).map(|d| d.count()).unwrap_or(0), 0, "a refused file leaves no folder");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..MAX_SNAPSHOTS {
            std::fs::create_dir_all(root.join(format!("s{i}"))).unwrap();
        }
        assert!(prepare(&ws, &root, open("editor", Some("docs/notes.md"))).unwrap_err().contains("close some of those panes"));
    }

    #[test]
    fn a_copy_goes_with_its_pane_and_the_unclaimed_go_at_start() {
        let (_tmp, ws, root) = setup();
        let take = |note: Option<&str>| {
            let (open, snap) = prepare(&ws, &root, open("editor", Some("docs/notes.md"))).unwrap();
            let snap = snap.unwrap();
            if let Some(block) = note {
                snap.opened(block);
            }
            PathBuf::from(open.file.unwrap()).parent().unwrap().to_path_buf()
        };
        let (kept, closed, opening) = (take(Some("pane-a")), take(Some("pane-b")), take(None));
        remove_where(&root, |noted| noted == Some("pane-b"));
        assert!(kept.exists() && !closed.exists(), "closing a pane takes only its copy");
        assert!(opening.exists(), "a copy whose pane is still opening is left alone");
        // At start nothing is opening: a copy no pane claimed goes, and so does one whose pane is gone.
        let (live, dead) = (take(Some("live")), take(Some("dead")));
        remove_where(&root, |noted| noted.is_none_or(|b| b != "live"));
        assert!(live.exists() && !dead.exists() && !kept.exists() && !opening.exists());
    }
}
