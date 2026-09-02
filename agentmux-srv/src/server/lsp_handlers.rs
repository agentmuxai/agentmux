// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::base::expand_home_dir_safe;

use super::AppState;

pub fn register_lsp_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ── LSP RPCs ───────────────────────────────────────────────────
    // Three handlers backing the editor pane's LSP integration:
    //   * lspstart — spawn (or attach to) the server for a file
    //   * lspsend  — forward an LSP JSON-RPC message to the server's stdin
    //   * lspstop  — refcount-decrement; server exits when count hits 0
    // Server-pushed notifications (publishDiagnostics, $/progress, …)
    // arrive via WS event `lsp:message` from the supervisor's reader task.
    // Spec: docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md

    let lsp_supervisor_start = state.lsp_supervisor.clone();
    engine.register_handler(
        "lspstart",
        Box::new(move |data, _ctx| {
            let supervisor = lsp_supervisor_start.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd {
                    language: String,
                    file_path: String,
                }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("lspstart: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.file_path);
                let workspace_root =
                    crate::backend::lsp::workspace::detect_workspace_root(expanded.as_path());
                let result = supervisor
                    .start(crate::backend::lsp::StartArgs {
                        language: cmd.language,
                        workspace_root,
                    })
                    .await
                    .map_err(|e| e.to_wire_string())?;
                Ok(Some(serde_json::json!({
                    "server_id": result.server_id,
                    "workspace_root": result.workspace_root,
                })))
            })
        }),
    );

    let lsp_supervisor_send = state.lsp_supervisor.clone();
    engine.register_handler(
        "lspsend",
        Box::new(move |data, _ctx| {
            let supervisor = lsp_supervisor_send.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd {
                    server_id: String,
                    message: serde_json::Value,
                }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("lspsend: {e}"))?;
                let message_json = serde_json::to_string(&cmd.message)
                    .map_err(|e| format!("lspsend serialize: {e}"))?;
                supervisor
                    .send(&cmd.server_id, &message_json)
                    .await
                    .map_err(|e| e.to_wire_string())?;
                Ok(None)
            })
        }),
    );

    let lsp_supervisor_stop = state.lsp_supervisor.clone();
    engine.register_handler(
        "lspstop",
        Box::new(move |data, _ctx| {
            let supervisor = lsp_supervisor_stop.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd {
                    server_id: String,
                }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("lspstop: {e}"))?;
                supervisor
                    .stop(&cmd.server_id)
                    .await
                    .map_err(|e| e.to_wire_string())?;
                Ok(None)
            })
        }),
    );
}
