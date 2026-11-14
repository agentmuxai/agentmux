// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/file/rename`: an agent renames a file or folder in its folder (`Pane` action
//! `rename` with target `file`). It is the Files pane's and the Editor tab's `fs.rename`, so the
//! name rules, the refusals and the `fs:renamed` event are the same, and every Editor tab open on
//! the file follows (docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md §8).
//!
//! A container agent names a path in its `/workspace`: the entry is renamed there and nowhere
//! else, and the answer names the new path as the container sees it. An SSH host is reached as
//! the user, so it takes the agent's signed identity and the user's consent for that host, as
//! `pane.open` does; a container agent has no such identity to give, and is told so.

use std::path::{Path, PathBuf};

use agentmux_common::api_types::FileRenameRequest;

use super::*;
use crate::backend::remote::ConnTarget;
use crate::backend::rpc_types::FsRenameReq;

/// A container agent asked for a file on an SSH host.
const NO_REMOTE_FOR_CONTAINERS: &str = "A container agent can't rename a file on another machine: an SSH host is reached as the user. \
                                        Rename it from a shell you have on that machine, or ask a native agent or your leader.";
/// A WSL distribution was named as the connection.
const NO_WSL: &str = "Pane rename takes an SSH host as its connection. For a file in a WSL distribution, \
                      give its Windows path (\\\\wsl.localhost\\<distribution>\\...) and no connection.";
/// The container agent's pane has no folder recorded.
const NO_WORKSPACE: &str = "This agent's workspace folder isn't known to AgentMux, so nothing in it can be renamed here. \
                            Rename the file in your own shell.";

/// A refusal: its status, and the sentence the caller is told.
type Refusal = (StatusCode, String);

fn forbidden(why: String) -> Refusal {
    (StatusCode::FORBIDDEN, why)
}

/// `fs.rename` said no: a name that exists, a name the platform refuses, no permission, a protected place.
fn not_done(why: String) -> Refusal {
    (StatusCode::UNPROCESSABLE_ENTITY, why)
}

/// `POST /api/v1/file/rename`. agentmux-mcp's `Pane` (`rename`, target `file`) calls it.
pub(super) async fn handle_file_rename(
    State(state): State<AppState>,
    grant: Option<axum::Extension<crate::backend::container::credential::ContainerGrant>>,
    Json(req): Json<FileRenameRequest>,
) -> impl IntoResponse {
    let container = grant.map(|g| g.0.block_id);
    match rename_file(&state, container.as_deref(), req).await {
        Ok(new_path) => (StatusCode::OK, Json(json!({ "success": true, "new_path": new_path }))).into_response(),
        Err((status, why)) => (status, Json(json!({ "error": why }))).into_response(),
    }
}

/// Renames `req.path` to `req.name` in its folder, for a container agent (`container` is its
/// pane) or for a caller on the host. The new path, as the caller names paths.
pub(crate) async fn rename_file(state: &AppState, container: Option<&str>, req: FileRenameRequest) -> Result<String, Refusal> {
    let path = req.path.trim().to_string();
    if path.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path must not be empty".to_string()));
    }
    if req.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name must not be empty".to_string()));
    }
    let ssh = ssh_host(req.connection.as_deref()).map_err(|why| (StatusCode::BAD_REQUEST, why))?;
    if let Some(block) = container {
        if ssh.is_some() {
            return Err(forbidden(NO_REMOTE_FOR_CONTAINERS.to_string()));
        }
        let workspace = super::container_workspace::workspace_of(state, block).ok_or_else(|| forbidden(NO_WORKSPACE.to_string()))?;
        return rename_in_workspace(state, workspace, path, req.name).await;
    }
    let mut block_id = None;
    if let Some(conn) = &ssh {
        let block = req.block_id.clone().unwrap_or_default();
        let agent = app_api::connections::verified_agent(state, &block, req.auth.as_ref()).map_err(forbidden)?;
        let what = format!("rename {path} to {}", req.name);
        app_api::connections::consent_for_ssh(state, &block, &agent, conn, &what).await.map_err(forbidden)?;
        block_id = Some(block);
    }
    let cmd = FsRenameReq { path, new_name: req.name, connection: ssh, block_id };
    super::fs_handlers::fs_rename(state, cmd).await.map(|done| done.new_path).map_err(not_done)
}

/// A container agent's rename: `raw` is renamed where it lies in `workspace` (its `/workspace` on
/// the host), and the answer is the container's own name for the new path.
async fn rename_in_workspace(state: &AppState, workspace: PathBuf, raw: String, name: String) -> Result<String, Refusal> {
    // Resolves folders on disk: off the async workers.
    let found = tokio::task::spawn_blocking(move || super::container_workspace::entry_inside(&workspace, &raw))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("the path couldn't be checked: {e}")))?;
    let (entry, folder) = found.map_err(forbidden)?;
    let cmd = FsRenameReq { path: entry.to_string_lossy().into_owned(), new_name: name.clone(), connection: None, block_id: None };
    let done = super::fs_handlers::fs_rename(state, cmd).await.map_err(not_done)?;
    let new_name = Path::new(&done.new_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
    Ok(format!("{}/{new_name}", folder.trim_end_matches('/')))
}

/// The SSH host `connection` names, in its canonical form; `None` for this computer. A WSL
/// distribution is refused: its files are renamed by their Windows path.
fn ssh_host(connection: Option<&str>) -> Result<Option<String>, String> {
    let Some(name) = connection.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    match ConnTarget::parse(name) {
        Ok(ConnTarget::Local) => Ok(None),
        Ok(target @ ConnTarget::Ssh(_)) => Ok(Some(target.name())),
        Ok(ConnTarget::Wsl(_)) => Err(NO_WSL.to_string()),
        Err(why) => Err(format!("connection {name:?} is not an SSH host: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(path: impl Into<String>, name: &str) -> FileRenameRequest {
        FileRenameRequest { path: path.into(), name: name.to_string(), connection: None, block_id: None, auth: None }
    }

    /// The `fs:renamed` events the state's broker saw, as their data.
    fn renames_seen(state: &AppState) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        state.broker.add_observer(std::sync::Arc::new(move |e: &crate::backend::mps::MuxEvent| {
            if e.event == "fs:renamed" {
                sink.lock().unwrap().push(e.data.clone().unwrap_or_default());
            }
        }));
        seen
    }

    /// A caller on the host: the file is renamed, the answer is its new path, and the windows are told once.
    #[tokio::test]
    async fn a_rename_on_the_host_answers_the_new_path_and_tells_the_windows_once() {
        let state = crate::server::tests::test_state();
        let seen = renames_seen(&state);
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("a.txt");
        std::fs::write(&old, "a").unwrap();

        let new_path = rename_file(&state, None, req(old.to_string_lossy(), "b.txt")).await.unwrap();
        assert!(dir.path().join("b.txt").exists() && !old.exists());
        assert!(new_path.ends_with("b.txt"), "{new_path}");
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["new_path"], new_path);
    }

    /// Each refusal is a sentence with its status, nothing is renamed and no window is told.
    #[tokio::test]
    async fn a_refusal_is_a_sentence_and_renames_nothing() {
        let state = crate::server::tests::test_state();
        let seen = renames_seen(&state);
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("a.txt");
        std::fs::write(&old, "a").unwrap();
        std::fs::write(dir.path().join("taken.txt"), "t").unwrap();
        let path = old.to_string_lossy().into_owned();

        let (status, why) = rename_file(&state, None, req(&path, "taken.txt")).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(why.contains("already exists"), "{why}");
        // A name can't carry a folder.
        let (status, _) = rename_file(&state, None, req(&path, "sub/b.txt")).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let gone = dir.path().join("gone.txt").to_string_lossy().into_owned();
        let (status, _) = rename_file(&state, None, req(gone, "b.txt")).await.unwrap_err();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        assert_eq!(rename_file(&state, None, req("  ", "b.txt")).await.unwrap_err(), (StatusCode::BAD_REQUEST, "path must not be empty".to_string()));
        assert_eq!(rename_file(&state, None, req(&path, " ")).await.unwrap_err(), (StatusCode::BAD_REQUEST, "name must not be empty".to_string()));
        let mut wsl = req(&path, "b.txt");
        wsl.connection = Some("wsl://Ubuntu".to_string());
        assert_eq!(rename_file(&state, None, wsl).await.unwrap_err(), (StatusCode::BAD_REQUEST, NO_WSL.to_string()));

        assert!(old.exists(), "nothing was renamed");
        assert!(seen.lock().unwrap().is_empty());
    }

    /// An SSH host needs the agent's signed identity: a request without it goes nowhere.
    #[tokio::test]
    async fn an_ssh_host_without_the_agents_identity_is_refused() {
        let state = crate::server::tests::test_state();
        let mut remote = req("~/a.txt", "b.txt");
        remote.connection = Some("devbox".to_string());
        remote.block_id = Some("blk".to_string());
        let (status, why) = rename_file(&state, None, remote).await.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(why.contains("signed identity"), "{why}");
    }

    #[test]
    fn a_connection_is_an_ssh_host_or_this_computer() {
        assert_eq!(ssh_host(None), Ok(None));
        assert_eq!(ssh_host(Some(" ")), Ok(None));
        assert_eq!(ssh_host(Some("local")), Ok(None));
        assert_eq!(ssh_host(Some(" me@devbox:2222 ")), Ok(Some("me@devbox:2222".to_string())));
        assert_eq!(ssh_host(Some("wsl://Ubuntu")), Err(NO_WSL.to_string()));
        assert!(ssh_host(Some("dev box")).unwrap_err().contains("is not an SSH host"));
    }

    /// A container agent: a path in its workspace is renamed there and answered as the container names it.
    #[tokio::test]
    async fn a_container_agent_renames_inside_its_workspace_and_is_answered_in_its_own_names() {
        let state = crate::server::tests::test_state();
        let seen = renames_seen(&state);
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/a.rs"), "a").unwrap();
        std::fs::write(ws.join("top.txt"), "t").unwrap();

        let new_path = rename_in_workspace(&state, ws.clone(), "/workspace/src/a.rs".to_string(), "b.rs".to_string()).await.unwrap();
        assert_eq!(new_path, "/workspace/src/b.rs");
        assert!(ws.join("src/b.rs").exists() && !ws.join("src/a.rs").exists());
        // A relative path is in the workspace too.
        let new_path = rename_in_workspace(&state, ws.clone(), "top.txt".to_string(), "first.txt".to_string()).await.unwrap();
        assert_eq!(new_path, "/workspace/first.txt");
        assert!(ws.join("first.txt").exists());
        assert_eq!(seen.lock().unwrap().len(), 2, "the windows hear of each, by its host path");
    }

    /// A container agent: a path outside its workspace is refused with what to do, and nothing is renamed.
    #[tokio::test]
    async fn a_container_agent_is_refused_outside_its_workspace() {
        let state = crate::server::tests::test_state();
        let seen = renames_seen(&state);
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let outside = root.path().join("secret.txt");
        std::fs::write(&outside, "s").unwrap();

        for given in ["/workspace/../secret.txt".to_string(), "../secret.txt".to_string(), outside.to_string_lossy().into_owned()] {
            let (status, why) = rename_in_workspace(&state, ws.clone(), given.clone(), "mine.txt".to_string()).await.unwrap_err();
            assert_eq!(status, StatusCode::FORBIDDEN, "{given}");
            assert!(why.contains("outside this agent's workspace") && why.contains("/workspace"), "{given}: {why}");
        }
        // The workspace folder itself is not the agent's to rename.
        let (status, _) = rename_in_workspace(&state, ws.clone(), "/workspace".to_string(), "other".to_string()).await.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(outside.exists() && ws.exists());
        assert!(seen.lock().unwrap().is_empty());

        // An SSH host is reached as the user, and a pane with no folder has nothing to rename in.
        let mut remote = req("/workspace/a.txt", "b.txt");
        remote.connection = Some("devbox".to_string());
        assert_eq!(rename_file(&state, Some("blk"), remote).await.unwrap_err(), (StatusCode::FORBIDDEN, NO_REMOTE_FOR_CONTAINERS.to_string()));
        assert_eq!(
            rename_file(&state, Some("no-such-pane"), req("/workspace/a.txt", "b.txt")).await.unwrap_err(),
            (StatusCode::FORBIDDEN, NO_WORKSPACE.to_string())
        );
    }

    /// A link in the workspace is renamed itself; what it points at, outside, is left alone.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_container_agents_link_is_renamed_itself_not_its_target() {
        let state = crate::server::tests::test_state();
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let outside = root.path().join("secret.txt");
        std::fs::write(&outside, "s").unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(root.path(), ws.join("out")).unwrap();

        let new_path = rename_in_workspace(&state, ws.clone(), "/workspace/link.txt".to_string(), "moved.txt".to_string()).await.unwrap();
        assert_eq!(new_path, "/workspace/moved.txt");
        assert!(ws.join("moved.txt").symlink_metadata().unwrap().file_type().is_symlink());
        assert!(outside.exists(), "the target keeps its name");
        // Through a linked folder the entry is outside.
        let (status, why) = rename_in_workspace(&state, ws.clone(), "/workspace/out/secret.txt".to_string(), "x".to_string()).await.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(why.contains("outside this agent's workspace"), "{why}");
    }
}
