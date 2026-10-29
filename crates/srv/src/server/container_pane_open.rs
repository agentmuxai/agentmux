// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `pane.open` from a container agent: panes that show the agent's own workspace, and nothing that
//! reaches the host beyond it.
//!
//! A native agent opens an editor, a media viewer or a file browser on a path of its machine. A
//! container agent's machine is its container, whose `/workspace` is a folder of the host, and the
//! pane is drawn by the window on the host. So a container agent may open the views that show files,
//! on paths inside that folder (`/workspace/...`, or relative to it), and no more: not a terminal (it
//! runs on the host), not a connection to another machine, not a browser (the Browser tool opens one
//! in the agent's own Incognito jar), and not a path outside the workspace, which would show the
//! user's `~/.ssh` in a pane the agent asked for.

use std::path::{Path, PathBuf};

use crate::backend::rpc_types::CommandPaneOpenData;
use crate::server::AppState;

/// The views a container agent may open: those that show files or app information.
const VIEWS: &[&str] = &["editor", "media", "files", "sysinfo", "help", "tower"];

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
    let joined = match raw.strip_prefix("/workspace") {
        Some("") => workspace.to_path_buf(),
        Some(rest) if rest.starts_with('/') => workspace.join(rest.trim_start_matches('/')),
        _ if Path::new(raw).is_absolute() => PathBuf::from(raw),
        _ => workspace.join(raw),
    };
    let root = crate::server::browser_uploads::canonical(workspace).map_err(|e| format!("the agent's workspace can't be read: {e}"))?;
    let real = crate::server::browser_uploads::canonical(&joined).map_err(|_| format!("no such file or folder: {raw}"))?;
    if real.starts_with(&root) {
        Ok(real.to_string_lossy().into_owned())
    } else {
        Err(format!("{raw} is outside this agent's workspace. A container agent can open only files in /workspace: copy the file there first."))
    }
}

/// Check and rewrite a container agent's `pane.open` against its `workspace`: only the views above, only
/// paths inside the workspace (rewritten to the host's), and nothing the caller sets beyond that.
pub(crate) fn confine(req: &mut CommandPaneOpenData, workspace: &Path) -> Result<(), String> {
    match req.view.as_str() {
        "term" => return Err("a terminal pane runs on the host, so a container agent can't open one. Use your own shell in the container, or ask a native agent or your leader.".into()),
        "browser" => return Err("use the Browser tool to open a page: it opens a browser pane in the agent's own Incognito jar.".into()),
        v if VIEWS.contains(&v) => {}
        other => return Err(format!("INVALID_VIEW: a container agent can open {} panes, not '{other}'", VIEWS.join(", "))),
    }
    if req.connection.as_deref().map(str::trim).is_some_and(|c| !c.is_empty() && c != "local") {
        return Err("a pane on another machine reaches it as the user, so a container agent can't open one.".into());
    }
    // Everything else the caller could set on the pane is theirs only through the fields below.
    req.meta = None;
    req.auth = None;
    req.url = None;
    req.connection = None;
    for path in [&mut req.file, &mut req.cwd] {
        if let Some(p) = path.as_deref().filter(|p| !p.trim().is_empty()) {
            *path = Some(inside(workspace, p)?);
        }
    }
    // The files view shows a folder; with none named it shows the workspace, not the user's home.
    if req.view == "files" && req.file.is_none() && req.cwd.is_none() {
        req.file = Some(inside(workspace, ".")?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(view: &str, file: Option<&str>) -> CommandPaneOpenData {
        CommandPaneOpenData { view: view.into(), file: file.map(str::to_string), ..Default::default() }
    }

    fn workspace() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("agentmux-cpo-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
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
            let mut req = open("editor", Some(given));
            confine(&mut req, &ws).unwrap_or_else(|e| panic!("{given}: {e}"));
            assert_eq!(req.file.as_deref(), Some(want.as_str()), "{given}");
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
            let mut req = open("editor", Some(&given));
            let err = confine(&mut req, &ws).expect_err(&given);
            assert!(err.contains("outside this agent's workspace") || err.contains("no such file"), "{given}: {err}");
        }
        // A symlink inside the workspace that points out is outside.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, ws.join("link.txt")).unwrap();
            let mut req = open("editor", Some("/workspace/link.txt"));
            assert!(confine(&mut req, &ws).unwrap_err().contains("outside this agent's workspace"));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_views_that_show_files_are_open_and_a_terminal_says_why() {
        let (root, ws) = workspace();
        for view in ["editor", "media", "files", "sysinfo", "help", "tower"] {
            let mut req = open(view, Some("/workspace/src/main.rs"));
            assert!(confine(&mut req, &ws).is_ok(), "{view}");
        }
        let term = confine(&mut open("term", None), &ws).unwrap_err();
        assert!(term.contains("runs on the host") && term.contains("own shell"), "{term}");
        let browser = confine(&mut open("browser", None), &ws).unwrap_err();
        assert!(browser.contains("Browser tool") && browser.contains("Incognito"), "{browser}");
        assert!(confine(&mut open("launcher", None), &ws).unwrap_err().contains("INVALID_VIEW"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_but_the_checked_paths_survives_from_the_caller() {
        let (root, ws) = workspace();
        let mut req = open("files", None);
        req.connection = Some("local".into());
        req.url = Some("http://x".into());
        req.meta = Some(Default::default());
        confine(&mut req, &ws).unwrap();
        assert_eq!((req.meta.is_none(), req.url.is_none(), req.connection.is_none(), req.auth.is_none()), (true, true, true, true));
        assert_eq!(req.file.as_deref(), Some(crate::server::browser_uploads::canonical(&ws).unwrap().to_string_lossy().as_ref()), "the files view with no folder shows the workspace");
        let mut ssh = open("editor", Some("src/main.rs"));
        ssh.connection = Some("user@host".into());
        assert!(confine(&mut ssh, &ws).unwrap_err().contains("another machine"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// The route itself: a container token reaches the handler, which confines it; a native caller is not.
    #[tokio::test]
    async fn the_handler_confines_a_container_token_and_leaves_the_instance_key_alone() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;
        let state = crate::server::tests::test_state();
        let app = crate::server::build_router(state);
        let call = |key: String| {
            let app = app.clone();
            async move {
                let req = Request::builder()
                    .method("POST")
                    .uri("/api/v1/pane/open")
                    .header("X-AuthKey", key)
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"view":"term"}"#))
                    .unwrap();
                let resp = app.oneshot(req).await.unwrap();
                let status = resp.status();
                let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                (status, String::from_utf8_lossy(&bytes).into_owned())
            }
        };
        let token = crate::backend::container_credential::token_for_block("cpo-block", Some("agent-tok"));
        let (status, body) = call(token).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.contains("no workspace folder"), "{body}");
        let (status, body) = call("test-secret-key".to_string()).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "the instance key is not confined: {body}");
    }
}
