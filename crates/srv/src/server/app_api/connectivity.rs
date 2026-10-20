// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `connectivity.status`: each service's reachability (`connectivity`), for a
//! window that opens after the last `connectivity:changed` event.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{NoArgsReq, COMMAND_CONNECTIVITY_STATUS};

pub(super) fn register(engine: &Arc<WshRpcEngine>) {
    engine.register_typed(COMMAND_CONNECTIVITY_STATUS, |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::connectivity::status())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::RpcMessage;

    #[tokio::test]
    async fn answers_with_the_current_status() {
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine);
        engine.handle_message(RpcMessage {
            command: COMMAND_CONNECTIVITY_STATUS.to_string(),
            reqid: "req-1".to_string(),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert!(resp.error.is_empty(), "{}", resp.error);
        let v = resp.data.expect("result data");
        assert!(v["revision"].is_number());
        assert_eq!(v["services"][0]["service"], "internet");
    }
}
