// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A file a container agent's message shows or opens: read from the agent's own folders, never from a host path.
//!
//! A message of a container agent can name any path: `![](C:/Users/me/photo.png)`, `![](/etc/x.png)`, a link the
//! agent made inside its workspace that points elsewhere. The window that draws the message reads a path as the
//! person, so reading the path as written would show the person's own files in a pane the agent can screenshot. So
//! a request that names the agent's pane (`block_id`) is answered here: the path is taken as the agent wrote it, a
//! path in its container (`/workspace/a.png`) or a path of the folder it shares (a relative path resolved against
//! the working directory), found in one of the folders the pane binds writable into its container, and opened by
//! walking down from that folder without following a link (`workspace_copy::open_beneath`). A path anywhere else
//! (another folder of the host, a drive or network path, the agent's home inside its container) is refused with the
//! reason, and so is one that leaves the folder through a link.
//!
//! The pane's own folders only: another container agent's workspace is not this agent's. A folder bound read-only
//! is not listed (the agent can't change it, but nothing in this check needs it).

use std::path::{Path, PathBuf};

use crate::backend::container::mounts::writable::{writable_binds, WritableBind};
use crate::backend::obj::{Block, MetaMapType};
use crate::backend::work_facts::paths::{normalize_path, relative_to};
use crate::server::AppState;

/// The refusal for a path that is not in the agent's own folders.
pub(crate) const OUTSIDE: &str = "This file isn't in a folder the agent shares with its container, so it isn't shown from the agent's message. Only the agent's own folders can be shown there.";

/// The refusal for a path that is in the agent's folder and leads out of it.
pub(crate) const LEADS_OUT: &str = "This file is in the agent's folder and leads out of it: a link the agent made points elsewhere. Nothing outside the agent's folder is shown from its message.";

/// The refusal for an agent with no folder to show.
const NO_FOLDER: &str = "This agent shares no folder with its container, so there is nothing to show from its message.";

/// A file found in one of an agent's folders.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// The bound folder, resolved.
    pub root: PathBuf,
    /// The file, resolved, inside `root`.
    pub file: PathBuf,
    /// The file's path in the container: what its copy is called.
    pub of: String,
}

/// The settings of the pane `block_id` when it is a container agent's pane: the one an agent's message is shown
/// in, whose folders its paths may lead into. `None` for any other pane (a native agent's messages are unchanged)
/// and for a pane that is not there.
pub(crate) fn pane_meta(state: &AppState, block_id: &str) -> Option<MetaMapType> {
    let block = state.mstore.get::<Block>(block_id).ok().flatten()?;
    crate::backend::runtime_adapter::RuntimeKind::of_block_meta(&block.meta).is_container().then_some(block.meta)
}

/// `path` with `.` and `..` taken out, as one text with `/` separators; `None` when a `..` would leave its root.
fn collapse(path: &str) -> Option<String> {
    // Not `normalize_path`: on Windows it reads `/c/x` as the drive `C:`, and a container's `/c/x` is its own folder.
    let text = path.trim().replace('\\', "/");
    let (root, rest) = match text.strip_prefix('/') {
        Some(rest) => ("/", rest),
        // A drive: `C:/...`.
        None => match text.split_once('/') {
            Some((drive, rest)) if drive.ends_with(':') => (&text[..drive.len() + 1], rest),
            _ => return None,
        },
    };
    let mut kept: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                kept.pop()?;
            }
            other => kept.push(other),
        }
    }
    Some(format!("{root}{}", kept.join("/")))
}

/// What `lexical` (collapsed, absolute) is inside `bind`, if it is: as the container names it (`/workspace/a.png`;
/// a container's paths are case-sensitive) or as the host does (the folder's own path, which a relative path of the
/// message is resolved against). Returns how long the match is, so the deepest folder wins, and the rest of the path.
fn within(bind: &WritableBind, lexical: &str) -> Option<(usize, String)> {
    let target = bind.target.trim_end_matches('/');
    if let Some(rest) = lexical.strip_prefix(target).filter(|rest| rest.is_empty() || rest.starts_with('/')) {
        return Some((target.len(), rest.trim_start_matches('/').to_string()));
    }
    let host = normalize_path(&bind.host.to_string_lossy());
    relative_to(lexical, &host).map(|rest| (host.len(), rest))
}

/// Whether `path` is a name or a path from the working folder: not rooted, and not led by a drive.
fn is_relative(path: &str) -> bool {
    let text = path.replace('\\', "/");
    !text.starts_with('/') && !text.split_once('/').is_some_and(|(drive, _)| drive.ends_with(':'))
}

/// `asked`, as a message of the container agent of the pane `meta` wrote it, found in the agent's own folders.
pub(crate) fn resolve(meta: &MetaMapType, asked: &str) -> Result<Found, String> {
    let binds = writable_binds(meta, None);
    if binds.is_empty() {
        return Err(NO_FOLDER.to_string());
    }
    let asked = asked.trim();
    // `~` is the agent's home inside its container, never the person's; a network path is another computer.
    if asked.is_empty() || asked.starts_with('~') || asked.starts_with("\\\\") || asked.starts_with("//") {
        return Err(OUTSIDE.to_string());
    }
    // A name with no folder is read from where the agent's shell starts, its workspace.
    let from_workspace = format!("{}/{asked}", crate::backend::container::CONTAINER_WORKSPACE_DIR.trim_end_matches('/'));
    let asked = if is_relative(asked) { from_workspace.as_str() } else { asked };
    let lexical = collapse(asked).ok_or_else(|| OUTSIDE.to_string())?;
    let (bind, rest) = binds
        .iter()
        .filter_map(|bind| within(bind, &lexical).map(|(len, rest)| (len, bind, rest)))
        .max_by_key(|(len, ..)| *len)
        .map(|(_, bind, rest)| (bind, rest))
        .ok_or_else(|| OUTSIDE.to_string())?;
    let canonical = crate::server::browser_uploads::canonical;
    let root = canonical(&bind.host).map_err(|_| "The agent's folder can't be read right now.".to_string())?;
    let spelled = if rest.is_empty() { bind.host.clone() } else { bind.host.join(&rest) };
    let Ok(file) = canonical(&spelled) else {
        // A link on the way that leads out of the folder with nothing at its end is refused as one with a file is.
        if crate::server::link_file_check::dangling_link_leaves(&spelled, &[&bind.host, &root]) {
            return Err(LEADS_OUT.to_string());
        }
        return Err("That file doesn't exist.".to_string());
    };
    if !file.starts_with(&root) {
        return Err(LEADS_OUT.to_string());
    }
    let target = bind.target.trim_end_matches('/');
    let of = if rest.is_empty() { bind.target.clone() } else { format!("{target}/{rest}") };
    Ok(Found { root, file, of })
}

/// The file `asked` names, opened from the agent's own folders (see [`resolve`]) without following a link.
pub(crate) fn open(meta: &MetaMapType, asked: &str) -> Result<(std::fs::File, Found), String> {
    let found = resolve(meta, asked)?;
    let file = crate::backend::workspace_copy::open_beneath(&found.root, &found.file)?;
    Ok((file, found))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent's workspace with a picture in it, the person's own folder beside it, and a pane that binds the
    /// workspace at `/workspace`.
    struct Fixture {
        _tmp: tempfile::TempDir,
        ws: PathBuf,
        private: PathBuf,
        meta: MetaMapType,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let private = tmp.path().join("private");
        std::fs::create_dir_all(ws.join("img")).unwrap();
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(ws.join("img").join("a.png"), "the agent's picture").unwrap();
        std::fs::write(private.join("photo.png"), "the person's photo").unwrap();
        let mut meta = MetaMapType::new();
        meta.insert("agentMode".to_string(), serde_json::json!("container"));
        meta.insert(crate::backend::blockcontroller::META_KEY_CMD_CWD.to_string(), serde_json::json!(ws.to_string_lossy()));
        Fixture { _tmp: tmp, ws, private, meta }
    }

    fn read(opened: Result<(std::fs::File, Found), String>) -> String {
        use std::io::Read as _;
        let mut text = String::new();
        opened.unwrap().0.read_to_string(&mut text).unwrap();
        text
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

    #[test]
    fn a_path_in_the_container_is_read_from_the_folder_the_container_shares() {
        let f = fixture();
        assert_eq!(read(open(&f.meta, "/workspace/img/a.png")), "the agent's picture");
        let found = resolve(&f.meta, "/workspace/img/a.png").unwrap();
        assert_eq!(found.of, "/workspace/img/a.png");
    }

    #[test]
    fn a_path_of_the_shared_folder_on_the_host_is_read_too_which_is_how_a_relative_path_arrives() {
        let f = fixture();
        // The pane resolves a relative path against the working directory, which is the host's folder.
        let relative = format!("{}/img/a.png", f.ws.to_string_lossy());
        assert_eq!(read(open(&f.meta, &relative)), "the agent's picture");
        assert_eq!(read(open(&f.meta, &format!("{}/img/../img/a.png", f.ws.to_string_lossy()))), "the agent's picture");
    }

    #[test]
    fn a_path_that_exists_on_the_host_but_is_not_in_the_agents_folders_is_refused() {
        let f = fixture();
        let photo = f.private.join("photo.png");
        assert_eq!(resolve(&f.meta, &photo.to_string_lossy()).unwrap_err(), OUTSIDE);
        assert_eq!(resolve(&f.meta, "/workspace/../private/photo.png").unwrap_err(), OUTSIDE);
        // A container path that is not the shared folder is the container's own: not on the host to read.
        assert_eq!(resolve(&f.meta, "/etc/hostname").unwrap_err(), OUTSIDE);
        assert_eq!(resolve(&f.meta, "/workspaces/img/a.png").unwrap_err(), OUTSIDE, "a folder that only starts with the same letters is not inside it");
        assert_eq!(resolve(&f.meta, "/Workspace/img/a.png").unwrap_err(), OUTSIDE, "a container's paths are case-sensitive");
    }

    #[test]
    fn home_unc_drive_and_dot_dot_paths_are_refused() {
        let f = fixture();
        for asked in ["~/img/a.png", "~", "//server/share/a.png", "\\\\server\\share\\a.png", "C:/Users/me/photo.png", "C:\\Users\\me\\photo.png", "/../x.png", "", "  "] {
            assert_eq!(resolve(&f.meta, asked).unwrap_err(), OUTSIDE, "{asked:?}");
        }
    }

    #[test]
    fn a_link_in_the_folder_that_leads_out_is_refused_whether_it_is_a_file_or_a_folder_on_the_way() {
        let f = fixture();
        if swap_for_a_link(&f.ws.join("img"), &f.private) {
            // A folder of the workspace that now points at the person's folder.
            assert_eq!(resolve(&f.meta, "/workspace/img/photo.png").unwrap_err(), LEADS_OUT);
            let host = format!("{}/img/photo.png", f.ws.to_string_lossy());
            assert_eq!(resolve(&f.meta, &host).unwrap_err(), LEADS_OUT);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_link_out_of_the_folder_is_refused_and_an_inside_link_is_read() {
        let f = fixture();
        std::os::unix::fs::symlink(f.private.join("photo.png"), f.ws.join("out.png")).unwrap();
        std::os::unix::fs::symlink(f.ws.join("img").join("a.png"), f.ws.join("in.png")).unwrap();
        assert_eq!(resolve(&f.meta, "/workspace/out.png").unwrap_err(), LEADS_OUT);
        assert_eq!(read(open(&f.meta, "/workspace/in.png")), "the agent's picture");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_folder_is_refused_alike_whether_or_not_its_target_is_there() {
        let f = fixture();
        std::os::unix::fs::symlink(f.private.join("not-there.png"), f.ws.join("gone.png")).unwrap();
        std::os::unix::fs::symlink(f.private.join("not-there-dir"), f.ws.join("gone-dir")).unwrap();
        std::os::unix::fs::symlink(f.ws.join("img").join("later.png"), f.ws.join("soon.png")).unwrap();
        assert_eq!(resolve(&f.meta, "/workspace/gone.png").unwrap_err(), LEADS_OUT);
        assert_eq!(resolve(&f.meta, "/workspace/gone-dir/a.png").unwrap_err(), LEADS_OUT);
        // A link to a folder that is there, with nothing at the end of the path, is refused as one with a file is.
        std::os::unix::fs::symlink(&f.private, f.ws.join("up")).unwrap();
        assert_eq!(resolve(&f.meta, "/workspace/up/nothing.png").unwrap_err(), LEADS_OUT);
        assert_eq!(resolve(&f.meta, "/workspace/up/photo.png").unwrap_err(), LEADS_OUT);
        // A link inside the folder whose own target runs through that link answers alike, with or without a file at its end.
        std::os::unix::fs::symlink(f.ws.join("up").join("nothing.png"), f.ws.join("via-up.png")).unwrap();
        std::os::unix::fs::symlink(f.ws.join("up").join("photo.png"), f.ws.join("via-up-file.png")).unwrap();
        std::fs::create_dir_all(f.ws.join("x").join("y")).unwrap();
        std::os::unix::fs::symlink(&f.ws, f.ws.join("x").join("y").join("dirlink")).unwrap();
        std::os::unix::fs::symlink("../probe.png", f.ws.join("d.png")).unwrap();
        assert_eq!(resolve(&f.meta, "/workspace/x/y/dirlink/d.png").unwrap_err(), LEADS_OUT);
        std::os::unix::fs::symlink("up/../probe.png", f.ws.join("dotdot.png")).unwrap();
        assert_eq!(resolve(&f.meta, "/workspace/dotdot.png").unwrap_err(), LEADS_OUT);
        assert_eq!(resolve(&f.meta, "/workspace/via-up.png").unwrap_err(), LEADS_OUT);
        assert_eq!(resolve(&f.meta, "/workspace/via-up-file.png").unwrap_err(), LEADS_OUT);
        let host = format!("{}/gone.png", f.ws.to_string_lossy());
        assert_eq!(resolve(&f.meta, &host).unwrap_err(), LEADS_OUT);
        // A link inside the folder whose target is not there yet is only a file that is not there.
        assert_eq!(resolve(&f.meta, "/workspace/soon.png").unwrap_err(), "That file doesn't exist.");
    }

    #[test]
    fn a_name_with_no_folder_is_read_from_the_workspace() {
        let f = fixture();
        assert_eq!(resolve(&f.meta, "img/a.png").unwrap().of, "/workspace/img/a.png");
        assert_eq!(read(open(&f.meta, "img/a.png")), "the agent's picture");
        assert_eq!(resolve(&f.meta, "img/none.png").unwrap_err(), "That file doesn't exist.");
        assert_eq!(resolve(&f.meta, "../a.png").unwrap_err(), OUTSIDE);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_link_above_it_says_a_missing_file_is_not_there() {
        let mut f = fixture();
        let door = f._tmp.path().join("door");
        std::os::unix::fs::symlink(f._tmp.path(), &door).unwrap();
        f.meta.insert(crate::backend::blockcontroller::META_KEY_CMD_CWD.to_string(), serde_json::json!(door.join("ws").to_string_lossy()));
        assert_eq!(resolve(&f.meta, "/workspace/img/none.png").unwrap_err(), "That file doesn't exist.");
        assert_eq!(read(open(&f.meta, "/workspace/img/a.png")), "the agent's picture");
        std::os::unix::fs::symlink(&f.private, f.ws.join("up")).unwrap();
        assert_eq!(resolve(&f.meta, "/workspace/up/nothing.png").unwrap_err(), LEADS_OUT);
    }

    #[test]
    fn a_link_swapped_in_after_the_path_was_found_is_not_followed() {
        let f = fixture();
        let found = resolve(&f.meta, "/workspace/img/a.png").unwrap();
        // The person's folder holds a file of the same name, which is what a followed link would read.
        std::fs::write(f.private.join("a.png"), "the person's file").unwrap();
        if swap_for_a_link(&f.ws.join("img"), &f.private) {
            // What was resolved a moment ago is opened again, now under a link: the open does not follow it.
            assert!(crate::backend::workspace_copy::open_beneath(&found.root, &found.file).is_err());
        }
    }

    #[test]
    fn an_agent_with_no_shared_folder_has_nothing_to_show() {
        let mut meta = MetaMapType::new();
        meta.insert("agentMode".to_string(), serde_json::json!("container"));
        assert_eq!(resolve(&meta, "/workspace/a.png").unwrap_err(), NO_FOLDER);
        // A native agent's pane binds nothing either: it is not asked here at all (`pane_meta`).
        assert_eq!(resolve(&MetaMapType::new(), "/workspace/a.png").unwrap_err(), NO_FOLDER);
    }

    #[test]
    fn only_the_workspace_and_the_volumes_the_person_declared_can_be_shown_never_the_folders_the_app_binds_itself() {
        let f = fixture();
        let shared = f.private.to_string_lossy().replace('\\', "/");
        let volumes = format!("[\"{shared}:/data\",\"{shared}:/docs:ro\",\"cache:/cache\"]");
        let mut meta = f.meta.clone();
        meta.insert("agent:container_volumes".to_string(), serde_json::json!(volumes));
        let targets: Vec<String> = writable_binds(&meta, None).into_iter().map(|b| b.target).collect();
        assert_eq!(targets, ["/workspace", "/data"], "a read-only volume and a named volume are not folders of the host to show");
        // The agent's Personal Memory, its credentials, its Git hooks and its AWS folder are bound by the app under the
        // container's home, outside these folders.
        for app_owned in ["/root/.claude/projects/-workspace/memory/m.md", "/root/.claude/.credentials.json", "/root/.aws/credentials", "/docs/photo.png", "/cache/x"] {
            assert_eq!(resolve(&meta, app_owned).unwrap_err(), OUTSIDE, "{app_owned}");
        }
    }

    #[test]
    fn a_file_that_is_not_there_says_so() {
        let f = fixture();
        assert_eq!(resolve(&f.meta, "/workspace/img/missing.png").unwrap_err(), "That file doesn't exist.");
    }

    #[test]
    fn collapse_takes_dots_out_and_refuses_to_leave_the_root() {
        assert_eq!(collapse("/a/./b//c/../d").as_deref(), Some("/a/b/d"));
        assert_eq!(collapse("C:\\a\\b\\..\\c").as_deref(), Some("C:/a/c"));
        assert_eq!(collapse("/c/x/../y").as_deref(), Some("/c/y"));
        assert_eq!(collapse("/a/../..").as_deref(), None);
        assert_eq!(collapse("relative/path").as_deref(), None);
    }

    #[test]
    fn the_deepest_of_two_folders_holds_a_path_that_both_do() {
        let f = fixture();
        let mut meta = f.meta.clone();
        let inner = f.ws.join("img");
        meta.insert("agent:container_volumes".to_string(), serde_json::json!(format!("[\"{}:/workspace/pictures\"]", inner.to_string_lossy().replace('\\', "/"))));
        let found = resolve(&meta, "/workspace/pictures/a.png").unwrap();
        assert_eq!(found.of, "/workspace/pictures/a.png");
        assert_eq!(read(open(&meta, "/workspace/pictures/a.png")), "the agent's picture");
    }
}
