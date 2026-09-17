// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc_types::{LspSendReq, LspStartReq, LspStartResult, LspStopReq};

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
    engine.register_typed(
        "lspstart",
        move |cmd: LspStartReq, _ctx| {
            let supervisor = lsp_supervisor_start.clone();
            async move {
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
                Ok(LspStartResult {
                    server_id: result.server_id,
                    workspace_root: result.workspace_root,
                })
            }
        },
    );

    let lsp_supervisor_send = state.lsp_supervisor.clone();
    engine.register_typed(
        "lspsend",
        move |cmd: LspSendReq, _ctx| {
            let supervisor = lsp_supervisor_send.clone();
            async move {
                let message_json = serde_json::to_string(&cmd.message)
                    .map_err(|e| format!("lspsend serialize: {e}"))?;
                supervisor
                    .send(&cmd.server_id, &message_json)
                    .await
                    .map_err(|e| e.to_wire_string())?;
                Ok(())
            }
        },
    );

    let lsp_supervisor_stop = state.lsp_supervisor.clone();
    engine.register_typed(
        "lspstop",
        move |cmd: LspStopReq, _ctx| {
            let supervisor = lsp_supervisor_stop.clone();
            async move {
                supervisor
                    .stop(&cmd.server_id)
                    .await
                    .map_err(|e| e.to_wire_string())?;
                Ok(())
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::RpcMessage;

    /// All three LSP commands record their request and response types.
    ///
    /// Before this, each handler deserialized into a `Cmd` struct declared
    /// inside its own closure. Those cannot be named, so they could not be
    /// generated from and the frontend restated every one of them by hand --
    /// three shapes with nothing connecting them to the Rust they describe.
    #[tokio::test]
    async fn register_typed_records_the_three_lsp_commands() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register_lsp_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };

        let start = find("lspstart");
        assert_eq!(start["requestName"], "LspStartReq");
        assert_eq!(start["responseName"], "LspStartResult");

        // `lspsend` and `lspstop` answer nothing, which is a real response
        // type rather than an absent one -- the stubs say `Promise<void>` and
        // the registry should agree rather than recording `Value`.
        for cmd in ["lspsend", "lspstop"] {
            let row = find(cmd);
            assert_eq!(row["responseName"], "()", "{cmd} should answer nothing");
        }
        assert_eq!(find("lspsend")["requestName"], "LspSendReq");
        assert_eq!(find("lspstop")["requestName"], "LspStopReq");
    }

    /// `lspsend.message` is an opaque passthrough: the backend re-serializes
    /// it and writes it to the server's stdin without inspecting it. Anything
    /// that is valid JSON has to survive, including the shapes a narrower
    /// Rust type would have rejected -- which is why it stays a `Value` and
    /// generates as `unknown` rather than as a named message type.
    #[test]
    fn lspsend_accepts_any_json_message() {
        for message in [
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
            serde_json::json!([1, 2, 3]),
            serde_json::json!("a bare string"),
            serde_json::Value::Null,
        ] {
            let req: LspSendReq = serde_json::from_value(serde_json::json!({
                "server_id": "srv-1",
                "message": message.clone(),
            }))
            .unwrap_or_else(|e| panic!("{message} should deserialize, got {e}"));
            assert_eq!(req.message, message, "the proxy must not reshape the payload");
        }
    }

    /// A missing `server_id` is an error rather than a silent no-op. The
    /// untyped handlers answered `lspstop: missing field ...`; `register_typed`
    /// rejects it before the handler body runs, so the request never reaches
    /// the supervisor either way.
    #[tokio::test]
    async fn a_malformed_request_is_rejected_rather_than_run() {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register_lsp_handlers(&engine, &state);

        engine.handle_message(RpcMessage {
            command: "lspstop".to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({})),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            resp.error.contains("server_id"),
            "expected a missing-field error naming the field, got {:?}",
            resp.error,
        );
    }
}
