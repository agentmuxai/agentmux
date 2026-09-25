// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `memoryadopt` service — the paired CEF host calls this, host→srv, once a
//! human has confirmed in the host's window which of an agent's earlier
//! accounts' memory folders to adopt
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.4, phase M3c).
//!
//! **Host-only**, like `credential`: every method requires `args[1]` to be
//! this srv instance's `AGENTMUX_HOST_REG_SECRET`. The shared `X-AuthKey` is
//! in every agent's environment, so an RPC an agent can reach can't stand
//! for a human's confirmation (see `credential.rs`'s module doc for the
//! incident that established this). The choices themselves are only
//! `(list_id, index, dir_hash)` from a list the server issued
//! (`agent:memory:adoption_list`) — never a path.
//!
//! Until GHSA-6726-q276-g6f6 is fixed this protects against MCP tools, not a
//! same-user process; the worst such a process could do is adopt memory
//! from a folder the server listed for that agent.

use serde::Deserialize;

use crate::backend::memory_adopt::{self, AdoptError};
use crate::backend::service::{get_arg, WebCallType, WebReturnType};

use super::super::AppState;
use super::credential::host_caller_allowed;

pub(super) async fn handle_memory_adopt_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let supplied = call.args.get(1).and_then(|v| v.as_str()).unwrap_or("");
    if !host_caller_allowed(state.host_reg_secret.as_deref(), supplied) {
        tracing::error!(
            "[memoryadopt] REJECTED a {} call — args[1] is not this srv instance's host-registration \
             secret. Only the paired host, after a human confirmed in its window, may adopt memory.",
            call.method
        );
        return WebReturnType::error("memoryadopt: host-only");
    }
    match call.method.as_str() {
        "Adopt" => handle_adopt(state, call).await,
        other => WebReturnType::error(format!("memoryadopt: unknown method {other}")),
    }
}

#[derive(Deserialize)]
struct Choice {
    index: usize,
    dir_hash: String,
}

#[derive(Deserialize)]
struct AdoptArgs {
    agent_id: String,
    list_id: String,
    choices: Vec<Choice>,
}

async fn handle_adopt(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args: AdoptArgs = match get_arg(&call.args, 0) {
        Ok(a) => a,
        Err(e) => return WebReturnType::error(format!("memoryadopt.Adopt: {e}")),
    };
    let Some(fs) = crate::backend::agent_session::global_transcript_store() else {
        return WebReturnType::error("memoryadopt.Adopt: memory record store unavailable");
    };
    let mstore = state.mstore.clone();
    let choices: Vec<(usize, String)> = args.choices.into_iter().map(|c| (c.index, c.dir_hash)).collect();
    let result = tokio::task::spawn_blocking(move || {
        memory_adopt::adopt(fs, &mstore, &args.agent_id, &args.list_id, &choices)
    })
    .await;
    match result {
        Ok(Ok(report)) => match serde_json::to_value(&report) {
            Ok(v) => WebReturnType::success(v),
            Err(e) => WebReturnType::error(format!("memoryadopt.Adopt: {e}")),
        },
        // A stable code the host can show: "changed" means list again.
        Ok(Err(AdoptError::Changed { index })) => {
            WebReturnType::error(format!("changed: folder {index} changed since the list was issued; list again"))
        }
        Ok(Err(AdoptError::UnknownList)) => WebReturnType::error("unknown-list: the list expired or is not this agent's; list again"),
        Ok(Err(AdoptError::BadChoice)) => WebReturnType::error("bad-choice"),
        Ok(Err(AdoptError::Busy)) => WebReturnType::error("busy: the agent's memory is being synced; try again"),
        Ok(Err(AdoptError::Store(e))) => {
            tracing::warn!("[memoryadopt] Adopt failed: {e}");
            WebReturnType::error("store error")
        }
        Err(e) => WebReturnType::error(format!("memoryadopt.Adopt: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(secret: Option<&str>) -> WebCallType {
        let mut args = vec![serde_json::json!({ "agent_id": "a", "list_id": "l", "choices": [] })];
        if let Some(s) = secret {
            args.push(serde_json::json!(s));
        }
        WebCallType { service: "memoryadopt".into(), method: "Adopt".into(), uicontext: None, args }
    }

    /// The attack this closes: an agent holds `X-AuthKey` (every agent's
    /// environment does) and calls Adopt itself, skipping the human's
    /// confirmation in the host window. It can't produce the host secret.
    #[tokio::test]
    async fn an_agent_without_the_host_secret_cannot_adopt() {
        let state = crate::server::tests::test_state();
        for secret in [None, Some(""), Some("not-the-secret")] {
            let r = handle_memory_adopt_service(&state, &call(secret)).await;
            assert_eq!(r.error.as_deref(), Some("memoryadopt: host-only"), "{secret:?}");
        }
    }

    #[tokio::test]
    async fn the_host_reaches_adopt() {
        let state = crate::server::tests::test_state();
        let r = handle_memory_adopt_service(&state, &call(Some("test-host-reg-secret"))).await;
        assert_ne!(r.error.as_deref(), Some("memoryadopt: host-only"));
    }
}
