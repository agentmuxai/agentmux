// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browser_start_page.set` — the browser pane's configured start page.
//! Write-only: the read side is `GetFullConfig`'s `browserstartpage` field
//! (`agentmux-srv/src/backend/wconfig/types.rs`), not a matching `.get`
//! here — see `docs/specs/SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md` §3.1
//! for why. Deliberately its own file rather than folded into
//! `bookmarks.rs`: unlike bookmarks, this handler needs `AppState`'s
//! `config_watcher`/`event_bus` to update the live config and broadcast
//! immediately, the same shape `COMMAND_SET_CONFIG` already has
//! (`server/websocket.rs`).

use super::*;
use crate::backend::browser_start_page;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_browser_start_page_set(engine, state);
}

#[derive(serde::Deserialize)]
struct StartPageSetReq {
    url: String,
}

fn register_browser_start_page_set(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let config_watcher = state.config_watcher.clone();
    let event_bus = state.event_bus.clone();
    engine.register_handler(
        COMMAND_BROWSER_START_PAGE_SET,
        Box::new(move |data, _ctx| {
            let config_watcher = config_watcher.clone();
            let event_bus = event_bus.clone();
            Box::pin(async move {
                let req: StartPageSetReq = serde_json::from_value(data)
                    .map_err(|e| format!("browser_start_page.set: {e}"))?;
                if req.url.trim().is_empty() {
                    return Err("browser_start_page.set: url must not be empty".to_string());
                }

                // 1. Write to disk. Unlike bookmarks.set, a failure to
                //    resolve the shared dir must fail loudly here too — the
                //    same "don't tell the caller it saved when it didn't"
                //    posture, not silently no-oping.
                let path = browser_start_page::start_page_file_path()
                    .ok_or_else(|| "browser_start_page.set: could not resolve the shared data directory".to_string())?;
                browser_start_page::write_start_page(&path, &req.url)?;

                // 2. Update in-memory config immediately — same
                //    write-then-update-then-broadcast-now shape as
                //    COMMAND_SET_CONFIG (server/websocket.rs), so every
                //    window converges on the new value without waiting on
                //    the fs watcher's 300ms debounce (spawn_start_page_watcher
                //    still exists as a backstop for an external file edit).
                config_watcher.update_browser_start_page(Some(req.url.clone()));

                // 3. Broadcast now.
                crate::backend::config_watcher_fs::broadcast_full_config(&config_watcher, &event_bus);

                Ok(Some(json!({ "url": req.url })))
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_req_deserializes_a_url() {
        let data = json!({ "url": "https://example.com" });
        let req: StartPageSetReq = serde_json::from_value(data).unwrap();
        assert_eq!(req.url, "https://example.com");
    }
}
