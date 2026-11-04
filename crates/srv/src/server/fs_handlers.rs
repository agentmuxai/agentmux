// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The generic `fs.*` RPC family behind the Files pane.
//!
//! Thin handlers over `backend::fs_ops`: each runs its filesystem work on a
//! blocking thread (`spawn_blocking`, or the trash thread for `fs.trash` /
//! `fs.restore`), never on an async worker, because on macOS the first
//! access to a protected folder blocks the calling thread while a privacy
//! prompt is up (spec §9.1.1 fact 10).
//!
//! Reads serve any path the user can read, as the editor's do. Mutations go
//! through `fs_ops::ProtectedPaths` instead of the editor's home-only rule
//! (spec §9, "Security").
//!
//! Copy and move (`fs.op.*`) are jobs rather than calls: `fs.op.start`
//! answers once the request is validated, and the work reports through
//! `files:op` events (`fs_ops::jobs`).
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3, §7, §9.

use std::sync::Arc;

use crate::backend::fs_ops;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    FsCreateReq, FsDeleteReq, FsEmptyResult, FsGitStatus, FsListReq, FsListResult, FsOpCancelReq, FsOpResolveReq,
    FsOpResult, FsOpResults, FsOpStartReq, FsOpStartResult, FsPathReq, FsPlace, FsPlaceKind, FsPlacesReq, FsPlacesResult,
    FsCreateResult, FsRenameReq, FsRenameResult, FsRestoreReq, FsTrashReq, FsUnwatchReq, FsWatchReq,
    FsWatchResult,
};

use super::AppState;

/// Run blocking filesystem work off the async runtime.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("filesystem task failed: {e}"))
}

/// Pair each requested path with its outcome, in request order.
fn op_results(paths: Vec<String>, outcomes: Vec<Result<(), String>>) -> FsOpResults {
    FsOpResults {
        results: paths
            .into_iter()
            .zip(outcomes)
            .map(|(path, outcome)| FsOpResult { path, ok: outcome.is_ok(), error: outcome.err() })
            .collect(),
    }
}

use crate::backend::fs_ops::remote as fs_remote;

fn remote_of<'a>(conn: &'a str, block_id: Option<&'a str>, auth_key: &'a str) -> fs_remote::Remote<'a> {
    fs_remote::Remote {
        connection: conn,
        block_id,
        auth_key,
    }
}

/// `fs.open` / `fs.reveal`: a confined pane hands nothing to another program.
pub(super) async fn open_or_reveal_bound(state: &AppState, cmd: FsPathReq, reveal: bool) -> Result<FsEmptyResult, String> {
    if super::files_root::is_confined(state, cmd.block_id.as_deref()) {
        return Err(super::files_root::NO_PROGRAMS.to_string());
    }
    let path = cmd.path;
    blocking(move || fs_ops::open_or_reveal(&path, reveal)).await??;
    Ok(FsEmptyResult {})
}

/// `fs.list` → one page of a folder. The first page opens it; later pages
/// continue from `cursor`. Spec §6.3.
/// A request naming an SSH connection runs on that host, through its
/// helper (`fs_ops::remote`, spec §6.3 of the remote terminals spec).
pub(crate) async fn fs_list(state: &AppState, mut cmd: FsListReq) -> Result<FsListResult, String> {
    // A pane confined to a folder (a container agent's workspace) lists only inside it, resolved now.
    if let Some(root) = cmd.block_id.as_deref().and_then(|b| super::files_root::root_of(state, b)) {
        let asked = cmd.path.clone();
        match blocking(move || super::files_root::confine(&root, &asked)).await? {
            Ok(real) => cmd.path = real.to_string_lossy().into_owned(),
            Err(message) => return Ok(super::files_root::outside_listing(&cmd.path, message)),
        }
    }
    if let Some(conn) = fs_remote::ssh_connection(cmd.connection.as_deref()) {
        let r = remote_of(conn, cmd.block_id.as_deref(), &state.auth_key);
        return fs_remote::list(r, &cmd.path).await;
    }
    fs_ops::ensure_cursor_sweeper();
    blocking(move || fs_ops::list_page(&cmd)).await
}

/// `fs.places` → home, known folders, drives and WSL distros. Known folders
/// are named, never read (spec §9.1.5).
pub(crate) async fn fs_places() -> Result<FsPlacesResult, String> {
    let distros = crate::backend::remote::wsl::list_cached().await;
    blocking(move || {
        let (home, mut places) = fs_ops::home_and_known_places();
        places.extend(super::editor_handlers::list_drives().into_iter().map(|d| FsPlace {
            id: format!("drive:{}", d.path),
            label: d.name,
            path: d.path,
            kind: FsPlaceKind::Drive,
        }));
        places.extend(distros.into_iter().map(|d| FsPlace {
            id: format!("wsl:{d}"),
            path: crate::backend::remote::wsl_fs::share_path(&d, "/"),
            label: d,
            kind: FsPlaceKind::Wsl,
        }));
        FsPlacesResult { home, sep: std::path::MAIN_SEPARATOR.to_string(), places }
    })
    .await
}

/// `fs.watch` → live updates for a listed folder, published as
/// `files:changed` scoped to the block. Spec §6.3.
pub(crate) async fn fs_watch(state: &AppState, cmd: FsWatchReq) -> Result<FsWatchResult, String> {
    let path = cmd.path.clone();
    let root = super::files_root::root_of(state, &cmd.block_id);
    let dir = blocking(move || -> Result<std::path::PathBuf, String> {
        if let Some(root) = &root {
            return super::files_root::confine(root, &path);
        }
        let canonical = fs_ops::resolve_request_path(&path)?
            .canonicalize()
            .map_err(|e| fs_ops::classify_folder_error(&e).1)?;
        if !canonical.is_dir() {
            return Err("This isn't a folder.".to_string());
        }
        Ok(canonical)
    })
    .await??;
    Ok(FsWatchResult { watch_id: state.files_watcher.watch(&dir, &cmd.block_id) })
}

/// `fs.unwatch`.
pub(crate) fn fs_unwatch(state: &AppState, cmd: FsUnwatchReq) -> Result<FsEmptyResult, String> {
    state.files_watcher.unwatch(&cmd.watch_id);
    Ok(FsEmptyResult {})
}

/// `fs.git_status` → the markers for a listed folder's entries. Never an
/// error: outside a repository, or if git fails, the pane shows none.
pub(crate) async fn fs_git_status(state: &AppState, cmd: FsPathReq) -> Result<FsGitStatus, String> {
    // A confined pane shows no markers.
    if super::files_root::is_confined(state, cmd.block_id.as_deref()) {
        return Ok(Default::default());
    }
    Ok(fs_ops::git::git_status(&cmd.path).await)
}

// ── Mutations. Spec §7, §9 ──────────────────────────────────────────

/// `fs.rename`.
pub(crate) async fn fs_rename(state: &AppState, cmd: FsRenameReq) -> Result<FsRenameResult, String> {
    super::files_root::refuse_change(state, cmd.block_id.as_deref())?;
    if let Some(conn) = fs_remote::ssh_connection(cmd.connection.as_deref()) {
        let r = remote_of(conn, cmd.block_id.as_deref(), &state.auth_key);
        let new_path = fs_remote::rename(r, &cmd.path, &cmd.new_name).await?;
        return Ok(FsRenameResult { new_path });
    }
    blocking(move || fs_ops::rename(&cmd)).await?
}

/// `fs.create`.
pub(crate) async fn fs_create(state: &AppState, cmd: FsCreateReq) -> Result<FsCreateResult, String> {
    super::files_root::refuse_change(state, cmd.block_id.as_deref())?;
    if let Some(conn) = fs_remote::ssh_connection(cmd.connection.as_deref()) {
        let r = remote_of(conn, cmd.block_id.as_deref(), &state.auth_key);
        let path = fs_remote::create(r, &cmd.parent, &cmd.name, cmd.kind).await?;
        return Ok(FsCreateResult { path });
    }
    blocking(move || fs_ops::create(&cmd)).await?
}

/// `fs.trash` → the OS Trash, on the single trash thread (spec §7.3).
pub(crate) async fn fs_trash(state: &AppState, cmd: FsTrashReq) -> Result<FsOpResults, String> {
    super::files_root::refuse_change(state, cmd.block_id.as_deref())?;
    if let Some(conn) = fs_remote::ssh_connection(cmd.connection.as_deref()) {
        return Err(fs_remote::no_trash(conn));
    }
    let paths = cmd.paths;
    let for_worker = paths.clone();
    let outcomes = fs_ops::trash_worker::run(move || for_worker.iter().map(|p| fs_ops::trash(p)).collect()).await?;
    Ok(op_results(paths, outcomes))
}

/// `fs.restore` → undo of fs.trash; the paths are the original locations.
pub(crate) async fn fs_restore(state: &AppState, cmd: FsRestoreReq) -> Result<FsOpResults, String> {
    super::files_root::refuse_change(state, cmd.block_id.as_deref())?;
    if let Some(conn) = fs_remote::ssh_connection(cmd.connection.as_deref()) {
        return Err(fs_remote::no_trash(conn));
    }
    let paths = cmd.paths;
    let for_worker = paths.clone();
    let outcomes = fs_ops::trash_worker::run(move || fs_ops::restore(&for_worker)).await?;
    Ok(op_results(paths, outcomes))
}

/// `fs.delete` → PERMANENT. The UI confirms first (spec §7.3).
pub(crate) async fn fs_delete(state: &AppState, cmd: FsDeleteReq) -> Result<FsOpResults, String> {
    super::files_root::refuse_change(state, cmd.block_id.as_deref())?;
    if let Some(conn) = fs_remote::ssh_connection(cmd.connection.as_deref()) {
        let r = remote_of(conn, cmd.block_id.as_deref(), &state.auth_key);
        return Ok(FsOpResults {
            results: fs_remote::delete(r, &cmd.paths).await,
        });
    }
    let paths = cmd.paths;
    let for_worker = paths.clone();
    let outcomes =
        blocking(move || for_worker.iter().map(|p| fs_ops::delete_permanently(p)).collect()).await?;
    Ok(op_results(paths, outcomes))
}

// ── Copy/move jobs. Spec §7.1, §7.2 ─────────────────────────────────

/// `fs.op.start` → validate, answer with an op id, then work on the op's
/// own thread, publishing `files:op` to the block.
pub(crate) async fn fs_op_start(state: &AppState, cmd: FsOpStartReq) -> Result<FsOpStartResult, String> {
    if cmd.block_id.is_empty() {
        return Err("No pane was named to report the operation's progress to.".to_string());
    }
    super::files_root::refuse_change(state, Some(&cmd.block_id))?;
    let emit = fs_ops::jobs::broker_emitter(state.broker.clone(), cmd.block_id.clone());
    // Either side on an SSH host: the host transfer (same events,
    // cancel and questions), its ssh prompts in this pane's window.
    let from = fs_remote::ssh_connection(cmd.source_connection.as_deref());
    let to = fs_remote::ssh_connection(cmd.connection.as_deref());
    if from.is_some() || to.is_some() {
        let ask = crate::backend::remote::sessions::AskIn {
            block_id: &cmd.block_id,
            auth_key: &state.auth_key,
        };
        // Both at once: each may first ask the user something over
        // ssh, and two in a row could outlast the request.
        let connect = |conn: Option<&str>| {
            let conn = conn.map(str::to_string);
            async move {
                match conn {
                    None => Ok(fs_ops::host_jobs::End::Local),
                    Some(c) => crate::backend::remote::files::connect(&c, Some(ask))
                        .await
                        .map(fs_ops::host_jobs::End::host)
                        .map_err(|e| e.message),
                }
            }
        };
        let (src, dest) = tokio::join!(connect(from), connect(to));
        let (src, dest) = (src?, dest?);
        return fs_ops::host_jobs::start(cmd.kind, src, cmd.sources, dest, cmd.dest_dir, emit).await;
    }
    blocking(move || fs_ops::jobs::JOBS.start(&cmd, emit)).await?
}

/// `fs.op.resolve` → touches only the op's in-memory controls, so it answers
/// at once, on the async thread.
pub(crate) fn fs_op_resolve(cmd: FsOpResolveReq) -> Result<FsEmptyResult, String> {
    fs_ops::jobs::JOBS.resolve(&cmd.op_id, cmd.choice, cmd.apply_to_all)?;
    Ok(FsEmptyResult {})
}

/// `fs.op.cancel`.
pub(crate) fn fs_op_cancel(cmd: FsOpCancelReq) -> Result<FsEmptyResult, String> {
    fs_ops::jobs::JOBS.cancel(&cmd.op_id);
    Ok(FsEmptyResult {})
}

fn register_fs_browsing(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed("fs.list", move |cmd: FsListReq, _ctx| {
        let s = s.clone();
        async move { fs_list(&s, cmd).await }
    });

    engine.register_typed(
        "fs.places",
        // `Option<_>` -- see `FsPlacesReq`: both encodings of "no argument".
        |_req: Option<FsPlacesReq>, _ctx| async move { fs_places().await },
    );

    let s = state.clone();
    engine.register_typed("fs.watch", move |cmd: FsWatchReq, _ctx| {
        let s = s.clone();
        async move { fs_watch(&s, cmd).await }
    });
    let s = state.clone();
    engine.register_typed("fs.unwatch", move |cmd: FsUnwatchReq, _ctx| {
        let s = s.clone();
        async move { fs_unwatch(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed("fs.git_status", move |cmd: FsPathReq, _ctx| {
        let s = s.clone();
        async move { fs_git_status(&s, cmd).await }
    });
}

fn register_fs_mutations(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed("fs.rename", move |cmd: FsRenameReq, _ctx| {
        let s = s.clone();
        async move { fs_rename(&s, cmd).await }
    });
    let s = state.clone();
    engine.register_typed("fs.create", move |cmd: FsCreateReq, _ctx| {
        let s = s.clone();
        async move { fs_create(&s, cmd).await }
    });
    let s = state.clone();
    engine.register_typed("fs.trash", move |cmd: FsTrashReq, _ctx| {
        let s = s.clone();
        async move { fs_trash(&s, cmd).await }
    });
    let s = state.clone();
    engine.register_typed("fs.restore", move |cmd: FsRestoreReq, _ctx| {
        let s = s.clone();
        async move { fs_restore(&s, cmd).await }
    });
    let s = state.clone();
    engine.register_typed("fs.delete", move |cmd: FsDeleteReq, _ctx| {
        let s = s.clone();
        async move { fs_delete(&s, cmd).await }
    });

    // fs.open / fs.reveal → hand a path to the OS default application / file
    // manager. Not home-scoped: they change nothing.
    let s = state.clone();
    engine.register_typed("fs.open", move |cmd: FsPathReq, _ctx| {
        let s = s.clone();
        async move { open_or_reveal_bound(&s, cmd, false).await }
    });
    let s = state.clone();
    engine.register_typed("fs.reveal", move |cmd: FsPathReq, _ctx| {
        let s = s.clone();
        async move { open_or_reveal_bound(&s, cmd, true).await }
    });
}

fn register_fs_jobs(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed("fs.op.start", move |cmd: FsOpStartReq, _ctx| {
        let s = s.clone();
        async move { fs_op_start(&s, cmd).await }
    });
    engine.register_typed("fs.op.resolve", |cmd: FsOpResolveReq, _ctx| async move { fs_op_resolve(cmd) });
    engine.register_typed("fs.op.cancel", |cmd: FsOpCancelReq, _ctx| async move { fs_op_cancel(cmd) });
}

pub fn register_fs_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // fileprobe → what a file is, before the Editor reads it (one stat, the
    // first 8 KB): SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2.
    crate::backend::editor_probe::register(engine);
    register_fs_browsing(engine, state);
    register_fs_mutations(engine, state);
    register_fs_jobs(engine, state);
}

#[cfg(test)]
mod tests {
    use crate::backend::rpc_types::RpcMessage;

    /// Every `fs.*` command records its request and response types, so the
    /// generated bindings describe the real wire format.
    #[tokio::test]
    async fn every_fs_command_records_its_types() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_fs_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };
        for (cmd, req, resp) in [
            ("fs.list", "FsListReq", "FsListResult"),
            ("fs.watch", "FsWatchReq", "FsWatchResult"),
            ("fs.unwatch", "FsUnwatchReq", "FsEmptyResult"),
            ("fs.rename", "FsRenameReq", "FsRenameResult"),
            ("fs.create", "FsCreateReq", "FsCreateResult"),
            ("fs.trash", "FsTrashReq", "FsOpResults"),
            ("fs.restore", "FsRestoreReq", "FsOpResults"),
            ("fs.delete", "FsDeleteReq", "FsOpResults"),
            ("fs.open", "FsPathReq", "FsEmptyResult"),
            ("fs.reveal", "FsPathReq", "FsEmptyResult"),
            ("fs.op.start", "FsOpStartReq", "FsOpStartResult"),
            ("fs.op.resolve", "FsOpResolveReq", "FsEmptyResult"),
            ("fs.op.cancel", "FsOpCancelReq", "FsEmptyResult"),
            ("fs.git_status", "FsPathReq", "FsGitStatus"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert_eq!(row["responseName"], resp, "{cmd} response");
        }
        assert_eq!(find("fs.places")["responseName"], "FsPlacesResult");
    }

    /// `fs.places` takes no argument; the stub sends `{}`, a client that
    /// omits `data` sends `null`. Both must work.
    #[tokio::test]
    async fn places_accepts_both_encodings_of_no_argument() {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_fs_handlers(&engine, &state);
        for (i, payload) in [serde_json::json!({}), serde_json::Value::Null].into_iter().enumerate() {
            engine.handle_message(RpcMessage {
                command: "fs.places".to_string(),
                reqid: format!("places-{i}"),
                data: Some(payload.clone()),
                ..Default::default()
            });
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
            assert!(resp.error.is_empty(), "fs.places should accept {payload}, got: {}", resp.error);
            let data = resp.data.unwrap();
            assert_eq!(data["sep"], std::path::MAIN_SEPARATOR.to_string());
            assert!(data["places"].as_array().unwrap().iter().any(|p| p["kind"] == "home"));
        }
    }

    /// The batch operations answer per path and never fail as a whole for
    /// one bad path.
    #[tokio::test]
    async fn a_batch_answers_per_path() {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_fs_handlers(&engine, &state);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doomed.txt");
        std::fs::write(&file, "x").unwrap();
        let missing = dir.path().join("missing.txt");

        engine.handle_message(RpcMessage {
            command: "fs.delete".to_string(),
            reqid: "del".to_string(),
            data: Some(serde_json::json!({ "paths": [file.to_string_lossy(), missing.to_string_lossy()] })),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert!(resp.error.is_empty(), "{}", resp.error);
        let results = resp.data.unwrap()["results"].as_array().unwrap().clone();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["ok"], true);
        assert_eq!(results[1]["ok"], false);
        assert!(results[1]["error"].is_string());
        assert!(!file.exists());
    }

    fn req<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    /// Without the engine: create a file, rename it, and list the folder.
    #[tokio::test]
    async fn create_rename_and_list_agree() {
        let state = crate::server::tests::test_state();
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().to_string_lossy().into_owned();

        let made = super::fs_create(&state, req(serde_json::json!({ "parent": parent, "name": "a.txt", "kind": "file" }))).await.unwrap();
        assert!(std::path::Path::new(&made.path).exists());

        let renamed = super::fs_rename(&state, req(serde_json::json!({ "path": made.path, "new_name": "b.txt" }))).await.unwrap();
        assert!(renamed.new_path.ends_with("b.txt"));
        assert!(!dir.path().join("a.txt").exists() && dir.path().join("b.txt").exists());

        let page = super::fs_list(&state, req(serde_json::json!({ "path": parent }))).await.unwrap();
        let names: Vec<String> = serde_json::to_value(&page).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["b.txt".to_string()]);
    }

    /// `fs.delete` answers per path, as the engine test checks, here through the handler.
    #[tokio::test]
    async fn fs_delete_reports_each_path_in_request_order() {
        let state = crate::server::tests::test_state();
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.txt");
        std::fs::write(&gone, "x").unwrap();
        let missing = dir.path().join("missing.txt");
        let out = super::fs_delete(&state, req(serde_json::json!({ "paths": [gone.to_string_lossy(), missing.to_string_lossy()] }))).await.unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(out.results[0].ok && !out.results[1].ok);
        assert_eq!(out.results[0].path, gone.to_string_lossy());
    }

    /// A watch on a folder answers with an id, `fs.unwatch` accepts it, and a
    /// file is not a folder to watch.
    #[tokio::test]
    async fn fs_watch_answers_with_an_id_and_refuses_a_file() {
        let state = crate::server::tests::test_state();
        let dir = tempfile::tempdir().unwrap();
        let watched = super::fs_watch(&state, req(serde_json::json!({ "path": dir.path().to_string_lossy(), "block_id": "watch-block" }))).await.unwrap();
        assert!(!watched.watch_id.is_empty());
        super::fs_unwatch(&state, req(serde_json::json!({ "watch_id": watched.watch_id }))).unwrap();

        let file = dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let err = super::fs_watch(&state, req(serde_json::json!({ "path": file.to_string_lossy(), "block_id": "watch-block" }))).await.unwrap_err();
        assert_eq!(err, "This isn't a folder.");
    }

    /// A copy or move must name the pane its progress goes to.
    #[tokio::test]
    async fn fs_op_start_wants_a_pane_to_report_to() {
        let state = crate::server::tests::test_state();
        let err = super::fs_op_start(&state, req(serde_json::json!({ "kind": "copy", "sources": [], "dest_dir": "x", "block_id": "" }))).await.unwrap_err();
        assert_eq!(err, "No pane was named to report the operation's progress to.");
    }

    /// Outside a repository the markers are empty, never an error.
    #[tokio::test]
    async fn fs_git_status_outside_a_repository_is_empty_not_an_error() {
        let state = crate::server::tests::test_state();
        let dir = tempfile::tempdir().unwrap();
        let status = super::fs_git_status(&state, req(serde_json::json!({ "path": dir.path().to_string_lossy() }))).await.unwrap();
        assert!(!status.in_repo, "a temp folder is not a repository");
        assert!(status.entries.is_empty());
    }
}
