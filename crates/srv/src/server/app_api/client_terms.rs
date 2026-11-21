// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Accepting the terms (`client.agreetos`). It is the person's own agreement, so only the AgentMux window
//! makes it: an agent's connection, or one that has not proven it is the window, is refused. The generic
//! service route refuses it too (`service::client`), since a request on it cannot prove who made it.

use super::*;

/// What a caller is told when it is not the window.
pub(crate) const TERMS_ARE_THE_PERSONS: &str = "only the person accepts the terms: ask the person to accept them in the AgentMux window";

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_CLIENT_AGREE_TOS, move |_req: Option<NoArgsReq>, ctx| {
        let state = state.clone();
        async move {
            ctx.operator_only(TERMS_ARE_THE_PERSONS)?;
            let mut client = crate::backend::wcore::get_client(&state.mstore).map_err(|e| e.to_string())?;
            client.tosagreed = chrono::Utc::now().timestamp_millis();
            state.mstore.update(&mut client).map_err(|e| e.to_string())?;
            Ok(ClientAgreeTosResult { tosagreed: client.tosagreed })
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::service::{WebCallType, WebReturnType};

    fn tosagreed(state: &AppState) -> i64 {
        crate::backend::wcore::get_client(&state.mstore).unwrap().tosagreed
    }

    async fn agree(state: &AppState, ctx: RpcContext) -> (serde_json::Value, String) {
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, state);
        engine.set_rpc_context(ctx);
        engine.handle_message(RpcMessage {
            command: COMMAND_CLIENT_AGREE_TOS.to_string(),
            reqid: "req-1".to_string(),
            data: Some(json!({})),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        (resp.data.unwrap_or_default(), resp.error)
    }

    #[tokio::test]
    async fn an_agent_connection_even_one_carrying_the_proof_cannot_accept_the_terms() {
        let state = crate::server::tests::test_state();
        let before = tosagreed(&state);
        let agent = RpcContext { agent_id: "korp".into(), operator_proven: true, ..Default::default() };
        let (_, error) = agree(&state, agent).await;
        assert!(error.starts_with("FORBIDDEN: only the person accepts the terms"), "{error}");
        assert!(error.contains("ask the person to accept them in the AgentMux window"), "{error}");
        assert_eq!(tosagreed(&state), before);
    }

    #[tokio::test]
    async fn a_connection_that_has_not_proved_it_is_the_window_cannot_accept_the_terms() {
        let state = crate::server::tests::test_state();
        let before = tosagreed(&state);
        let (_, error) = agree(&state, RpcContext::default()).await;
        assert!(error.contains("isn't the AgentMux window"), "{error}");
        assert_eq!(tosagreed(&state), before);
    }

    #[tokio::test]
    async fn the_window_accepts_the_terms() {
        let state = crate::server::tests::test_state();
        let (data, error) = agree(&state, RpcContext::operator_for_tests()).await;
        assert_eq!(error, "");
        let stored = tosagreed(&state);
        assert!(stored > 0);
        assert_eq!(data["tosagreed"], json!(stored));
    }

    #[tokio::test]
    async fn the_service_route_refuses_the_terms_and_says_where_the_person_accepts_them() {
        let state = crate::server::tests::test_state();
        let before = tosagreed(&state);
        let call = WebCallType { service: "client".into(), method: "AgreeTos".into(), uicontext: None, args: vec![] };
        let reply: WebReturnType = crate::server::service::run_service_call(&state, &call).await;
        assert!(!reply.success);
        assert_eq!(reply.error.as_deref(), Some(TERMS_ARE_THE_PERSONS));
        assert_eq!(tosagreed(&state), before);
    }
}
