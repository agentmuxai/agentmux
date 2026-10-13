// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `shell.list`: the shells installed here, and which one a new local terminal
//! starts with, for Settings' default-shell picker (`backend::shells`).

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let config = state.config_watcher.clone();
    engine.register_typed(COMMAND_SHELL_LIST, move |_req: Option<NoArgsReq>, _ctx| {
        let config = config.clone();
        async move {
            let setting = config.get_settings().term_local_shell_path;
            // Finding versions runs each shell once: off the async runtime.
            tokio::task::spawn_blocking(move || crate::backend::shells::list(&setting))
                .await
                .map_err(|e| format!("shell.list: {e}"))
        }
    });
}
