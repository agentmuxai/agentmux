// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The page's file chooser, answered from the agent's workspace
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §6).
//!
//! A pane an agent drives never shows the OS file dialog: the host takes the
//! chooser the page opens and asks here what to do with it. On a page a grant
//! with the uploads power covers, the host keeps the chooser for 30 seconds,
//! and `set_files` with `chooser: true` puts files in it. Everywhere else it is
//! cancelled at once, as before, and the agent is told to use `set_files` on
//! the control's own reference.
//!
//! The files come through the same door as any upload (`browser_uploads`): only
//! files in the agent's own workspace, links and `..` resolved, never a
//! directory, 25 MB in all. The grant lifts no part of that.

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use super::browser_actions::{note, Kind};
use super::browser_powers::Power;
use super::ui_handlers::{banner_for, proxy_value};
use super::AppState;

/// How long the host keeps a chooser. Said to the agent; the host holds the clock.
pub(crate) const HELD_SECONDS: u64 = 30;

/// Should the host keep the chooser `agent`'s pane `target` just showed on `page`? Only when the
/// agent holds the uploads power on that site and the pane isn't paused. A folder chooser never.
pub(crate) fn holds(state: &AppState, target: &str, agent: &str, page: &str, ask: &str) -> bool {
    ask != "choose a folder"
        && !agent.is_empty()
        && super::browser_actions::not_paused(target).is_ok()
        && super::browser_press::grant(state, &banner_for(target).0, agent, page, Power::Uploads).is_some()
}

/// What the agent reads about a chooser the host is keeping.
pub(crate) fn held_reason() -> String {
    format!(
        "AgentMux is keeping the page's file chooser for {HELD_SECONDS} seconds: call Browser set_files with chooser: true and paths (files in your workspace) to fill it. A page shows one chooser at a time, so a control you click that opens another one now shows it only after this is filled or has run out, with its own 30 seconds"
    )
}

/// `set_files` with `chooser: true`: the files go into the chooser the page last opened in `block_id`.
/// `own` is the calling agent's own block, whose workspace the files come from.
pub(crate) async fn fill(state: &AppState, block_id: &str, own: &str, agent: &str, paths: Vec<String>) -> Response {
    let log = |names: &str| note(block_id, agent, Kind::Upload).target("the page's file chooser").detail(names);
    let page = match super::browser_bypass_routes::page_url(state, block_id).await {
        Ok(p) => p,
        Err(e) => return log("").fail(state, StatusCode::BAD_GATEWAY, e),
    };
    let Some(g) = super::browser_press::grant(state, &banner_for(block_id).0, agent, &page, Power::Uploads) else {
        return log("").fail(
            state,
            StatusCode::FORBIDDEN,
            "no file chooser was filled: that needs the uploads power on this site, and no grant gives it to you here. Ask for it with request_auto_approve (powers [\"uploads\"]), or use set_files with the ref of the control that opens the chooser".to_string(),
        );
    };
    if let Err((code, e)) = super::browser_actions::not_paused(block_id) {
        return log("").fail(state, code, e);
    }
    let files = match super::browser_uploads::files_for_agent(own, paths).await {
        Ok(f) => f,
        Err((code, e)) => return log("").fail(state, code, e),
    };
    let names: Vec<&str> = files.iter().filter_map(|f| f["name"].as_str()).collect();
    tracing::info!(agent_id = %agent, block_id = %block_id, files = ?names, "[ui-automation] browser upload into the page's file chooser");
    let sent = json!({ "block_id": block_id, "files": files, "allowed_origins": g.origins });
    let done: Result<Value, _> = proxy_value(state, "chooser_files", sent, std::time::Duration::from_secs(60)).await;
    log(&names.join(", ")).outcome(state, done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_is_told_how_long_the_chooser_is_kept_and_how_to_fill_it() {
        let r = held_reason();
        assert!(r.contains("30 seconds") && r.contains("chooser: true") && r.contains("your workspace"), "{r}");
    }
}
