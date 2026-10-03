// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The generic `fs.*` RPC family behind the Files pane ("Hangar").
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
    FsCreateReq, FsDeleteReq, FsEmptyResult, FsListReq, FsOpCancelReq, FsOpResolveReq, FsOpResult,
    FsOpResults, FsOpStartReq, FsPathReq, FsPlace, FsPlaceKind, FsPlacesReq, FsPlacesResult,
    FsRenameReq, FsRestoreReq, FsTrashReq, FsUnwatchReq, FsWatchReq, FsWatchResult,
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

pub fn register_fs_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // fs.list → one page of a folder. The first page opens it; later pages
    // continue from `cursor`. Spec §6.3.
    engine.register_typed("fs.list", |cmd: FsListReq, _ctx| async move {
        fs_ops::ensure_cursor_sweeper();
        blocking(move || fs_ops::list_page(&cmd)).await
    });

    // fs.places → home, known folders, drives and WSL distros. Known folders
    // are named, never read (spec §9.1.5).
    engine.register_typed(
        "fs.places",
        // `Option<_>` -- see `FsPlacesReq`: both encodings of "no argument".
        |_req: Option<FsPlacesReq>, _ctx| async move {
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
        },
    );

    // fs.watch / fs.unwatch → live updates for a listed folder, published as
    // `files:changed` scoped to the block. Spec §6.3.
    {
        let watcher = state.files_watcher.clone();
        engine.register_typed("fs.watch", move |cmd: FsWatchReq, _ctx| {
            let watcher = watcher.clone();
            async move {
                let path = cmd.path.clone();
                let dir = blocking(move || -> Result<std::path::PathBuf, String> {
                    let canonical = fs_ops::resolve_request_path(&path)?
                        .canonicalize()
                        .map_err(|e| fs_ops::classify_folder_error(&e).1)?;
                    if !canonical.is_dir() {
                        return Err("This isn't a folder.".to_string());
                    }
                    Ok(canonical)
                })
                .await??;
                Ok(FsWatchResult { watch_id: watcher.watch(&dir, &cmd.block_id) })
            }
        });
    }
    {
        let watcher = state.files_watcher.clone();
        engine.register_typed("fs.unwatch", move |cmd: FsUnwatchReq, _ctx| {
            let watcher = watcher.clone();
            async move {
                watcher.unwatch(&cmd.watch_id);
                Ok(FsEmptyResult {})
            }
        });
    }

    // ── Mutations. Spec §7, §9 ──────────────────────────────────────────

    engine.register_typed("fs.rename", |cmd: FsRenameReq, _ctx| async move {
        blocking(move || fs_ops::rename(&cmd)).await?
    });

    engine.register_typed("fs.create", |cmd: FsCreateReq, _ctx| async move {
        blocking(move || fs_ops::create(&cmd)).await?
    });

    // fs.trash → the OS Trash, on the single trash thread (spec §7.3).
    engine.register_typed("fs.trash", |cmd: FsTrashReq, _ctx| async move {
        let paths = cmd.paths;
        let for_worker = paths.clone();
        let outcomes = fs_ops::trash_worker::run(move || for_worker.iter().map(|p| fs_ops::trash(p)).collect()).await?;
        Ok(op_results(paths, outcomes))
    });

    // fs.restore → undo of fs.trash; the paths are the original locations.
    engine.register_typed("fs.restore", |cmd: FsRestoreReq, _ctx| async move {
        let paths = cmd.paths;
        let for_worker = paths.clone();
        let outcomes = fs_ops::trash_worker::run(move || fs_ops::restore(&for_worker)).await?;
        Ok(op_results(paths, outcomes))
    });

    // fs.delete → PERMANENT. The UI confirms first (spec §7.3).
    engine.register_typed("fs.delete", |cmd: FsDeleteReq, _ctx| async move {
        let paths = cmd.paths;
        let for_worker = paths.clone();
        let outcomes = blocking(move || for_worker.iter().map(|p| fs_ops::delete_permanently(p)).collect()).await?;
        Ok(op_results(paths, outcomes))
    });

    // fs.open / fs.reveal → hand a path to the OS default application / file
    // manager. Not home-scoped: they change nothing.
    engine.register_typed("fs.open", |cmd: FsPathReq, _ctx| async move {
        blocking(move || fs_ops::open_or_reveal(&cmd.path, false)).await??;
        Ok(FsEmptyResult {})
    });
    engine.register_typed("fs.reveal", |cmd: FsPathReq, _ctx| async move {
        blocking(move || fs_ops::open_or_reveal(&cmd.path, true)).await??;
        Ok(FsEmptyResult {})
    });

    // ── Copy/move jobs. Spec §7.1, §7.2 ─────────────────────────────────

    // fs.op.start → validate, answer with an op id, then work on the op's
    // own thread, publishing `files:op` to the block.
    {
        let broker = state.broker.clone();
        engine.register_typed("fs.op.start", move |cmd: FsOpStartReq, _ctx| {
            let broker = broker.clone();
            async move {
                if cmd.block_id.is_empty() {
                    return Err("No pane was named to report the operation's progress to.".to_string());
                }
                let emit = fs_ops::jobs::broker_emitter(broker, cmd.block_id.clone());
                blocking(move || fs_ops::jobs::JOBS.start(&cmd, emit)).await?
            }
        });
    }

    // fs.op.resolve / fs.op.cancel → touch only the op's in-memory controls,
    // so they answer at once, on the async thread.
    engine.register_typed("fs.op.resolve", |cmd: FsOpResolveReq, _ctx| async move {
        fs_ops::jobs::JOBS.resolve(&cmd.op_id, cmd.choice, cmd.apply_to_all)?;
        Ok(FsEmptyResult {})
    });
    engine.register_typed("fs.op.cancel", |cmd: FsOpCancelReq, _ctx| async move {
        fs_ops::jobs::JOBS.cancel(&cmd.op_id);
        Ok(FsEmptyResult {})
    });

    // fs.git_status → the markers for a listed folder's entries. Never an
    // error: outside a repository, or if git fails, the pane shows none.
    engine.register_typed("fs.git_status", |cmd: FsPathReq, _ctx| async move {
        Ok::<_, String>(fs_ops::git::git_status(&cmd.path).await)
    });
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
}
