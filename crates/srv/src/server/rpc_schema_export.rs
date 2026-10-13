// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Writes every typed command's request and response, as TypeScript, to
//! `frontend/types/rpc/rpc-schema.json`: the input of the generated RPC stubs
//! (`scripts/gen-rpc-stubs.mjs` -> `frontend/app/store/rpc-api/generated.ts`).
//!
//! The test is named `export_bindings_*` so it runs with the ts-rs exports
//! (`cargo test -p agentmux-srv export_bindings`), and it writes into the
//! generated bindings directory so `scripts/check-rpc-bindings.sh` fails when
//! the file is stale, the same as for any generated type.

use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;

#[tokio::test]
async fn export_bindings_rpc_schema() {
    let state = test_state();
    let (engine, _rx) = WshRpcEngine::new();
    register_handlers(&engine, state, "rpc-schema".to_string());
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/types/rpc/rpc-schema.json");
    let mut json = serde_json::to_string_pretty(&engine.schema_json()).unwrap();
    json.push('\n');
    std::fs::write(&out, json).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
}
