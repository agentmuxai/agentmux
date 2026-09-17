// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bookmarks.list` / `bookmarks.set` — the browser pane's saved-URL list.
//! Deliberately a thin wrapper over `backend::bookmarks_store`: no `Store`/
//! sqlite involvement, no per-agent scoping — see
//! `docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md` for why
//! this is a dedicated `shared_dir` file instead of the ABF/`db_bundles`
//! pattern the rest of this directory mostly follows.

use super::*;
use crate::backend::bookmarks_store::{self, BrowserBookmark};

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    register_bookmarks_list(engine);
    register_bookmarks_set(engine);
}

/// `None` from `bookmarks_file_path()` means `DataPaths` couldn't resolve
/// (unusual/CI env, same fallback `providers_handlers.rs` already accepts
/// for `provider_auth_dir()`) — best-effort empty list rather than an RPC
/// error, so a missing data-dir environment doesn't break the whole nav bar.
fn bookmarks_list_impl() -> Result<Vec<BrowserBookmark>, String> {
    let Some(path) = bookmarks_store::bookmarks_file_path() else {
        return Ok(Vec::new());
    };
    bookmarks_store::read_bookmarks(&path)
}

fn register_bookmarks_list(engine: &Arc<WshRpcEngine>) {
    // Typed registration (SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.1) — the
    // request/response shapes are named types carrying `#[derive(ts_rs::TS)]`,
    // so the frontend consumes generated bindings and drift fails the build.
    // `()` is a legal Req for a command that takes no arguments.
    engine.register_typed(COMMAND_BOOKMARKS_LIST, move |_req: (), _ctx| async move {
        Ok(BookmarksResult { bookmarks: bookmarks_list_impl()? })
    });
}

fn register_bookmarks_set(engine: &Arc<WshRpcEngine>) {
    engine.register_typed(
        COMMAND_BOOKMARKS_SET,
        move |req: CommandBookmarksSetData, _ctx| async move {
            // Unlike bookmarks.list's best-effort empty fallback, a SET
            // with nowhere to durably land must fail loudly — silently
            // no-oping here would tell the caller a bookmark was saved
            // when it wasn't (see the spec's "shared_dir can't be
            // resolved" unhappy-path entry).
            let path = bookmarks_store::bookmarks_file_path()
                .ok_or_else(|| "bookmarks.set: could not resolve the shared data directory".to_string())?;
            bookmarks_store::write_bookmarks(&path, &req.bookmarks)?;
            Ok(BookmarksResult { bookmarks: req.bookmarks })
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_req_deserializes_a_bookmarks_array() {
        let data = json!({
            "bookmarks": [
                {"id": "b1", "title": "T", "url": "https://example.com"}
            ]
        });
        let req: CommandBookmarksSetData = serde_json::from_value(data).unwrap();
        assert_eq!(
            req.bookmarks,
            vec![BrowserBookmark {
                id: "b1".to_string(),
                title: "T".to_string(),
                url: "https://example.com".to_string(),
                favicon_url: String::new(),
                created_at: 0,
            }]
        );
    }

    #[test]
    fn set_req_rejects_missing_bookmarks_field() {
        let data = json!({});
        let result: Result<CommandBookmarksSetData, _> = serde_json::from_value(data);
        assert!(result.is_err());
    }
}
