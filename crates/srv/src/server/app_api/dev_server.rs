// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/dev_server/register` — backs the `RegisterDevServer`
//! MCP tool. See docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
//!
//! Identity is verified the same way `ClosePane`/`UIClick`/`UIQuery` do
//! (`ui_handlers::verified_block_id`) — the calling agent proves it IS
//! `auth.agent_id` via its own `AGENTMUX_JEKT_KEY` signature; there is no
//! client-supplied agent id to spoof. The container's internal dev-proxy
//! network address is then resolved server-side from THAT agent's own
//! container (`ContainerManager::agent_network_ip`) — never a
//! client-supplied address, so there is nothing left to spoof there either.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use agentmux_common::api_types::{RegisterDevServerRequest, RegisterDevServerResponse};

use crate::backend::dev_proxy::DEV_PROXY_PORT;

use super::AppState;

pub(crate) async fn handle_register_dev_server(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<RegisterDevServerRequest>,
) -> impl IntoResponse {
    // Verifies `req.auth`'s signature against the claimed agent_id's own key on file; the pane it
    // proves is the one whose container this registers. See this module's doc comment.
    let block_id = match crate::server::ui_handlers::verified_block_id(&state, caller.as_deref(), &req.auth) {
        Ok(b) => b,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))).into_response(),
    };
    let agent_id = req.auth.agent_id.clone();

    let project = req.project.trim().to_lowercase();
    if project.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "project must not be empty" })),
        )
            .into_response();
    }
    // `project` becomes a DNS label in `<project>-<agent>.localhost` (see
    // `DevProxyRegistry::routing_key`) — validate it as one instead of only
    // checking non-empty. An unvalidated value containing e.g. '.' or
    // whitespace would previously register successfully and hand the caller
    // back a URL that can never actually route, discovered only when they
    // tried to open it. reagent P2, PR #3439.
    let is_hostname_label = !project.is_empty()
        && project.len() <= 63
        && !project.starts_with('-')
        && !project.ends_with('-')
        && project
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !is_hostname_label {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "project must be a valid hostname label: lowercase \
                          letters, digits, and hyphens only, not starting or \
                          ending with a hyphen, 63 characters or fewer"
            })),
        )
            .into_response();
    }
    if req.port == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "port must be nonzero" })),
        )
            .into_response();
    }

    let Some(cm) = state.container_manager.get().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "Docker is not available — RegisterDevServer only works from a \
                          container-type agent's own container"
            })),
        )
            .into_response();
    };

    // Named from the pane's `agentId` (the definition's uuid), as the launch path names it; the
    // slug in `auth.agent_id` names no container.
    let Some(container_name) = container_of_pane(&state, &block_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "RegisterDevServer only works from inside a container-type agent's own container"
            })),
        )
            .into_response();
    };
    let Some(ip) = cm.agent_network_ip(&container_name).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!(
                    "no running container found for agent {agent_id:?} on the dev-proxy \
                     network — RegisterDevServer only works from inside a container-type \
                     agent's own container"
                )
            })),
        )
            .into_response();
    };

    // Where the proxy reaches it. On Linux the container's own address; where the daemon runs in a VM
    // (Docker Desktop) that address is not routable from here, so the dev relay joins the two.
    let target: crate::backend::dev_proxy::Target = if crate::backend::dev_relay::needed(&cm).await {
        let image = cm
            .docker()
            .inspect_container(&container_name, None)
            .await
            .ok()
            .and_then(|c| c.config?.image)
            .unwrap_or_default();
        match crate::backend::dev_relay::ensure(&cm, &image).await {
            Ok(via) => crate::backend::dev_proxy::Target::Relay { via, container: container_name, port: req.port },
            Err(e) => {
                return (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("the dev proxy's relay is not available: {e}") }))).into_response();
            }
        }
    } else {
        match format!("{ip}:{}", req.port).parse::<std::net::SocketAddr>() {
            Ok(a) => a.into(),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("invalid address {ip}:{}: {e}", req.port) })),
                )
                    .into_response();
            }
        }
    };

    state.dev_proxy.register(&project, &agent_id, target).await;

    // Lowercased to match the actual routing key (`DevProxyRegistry::routing_key`
    // lowercases both halves) — this is what a browser should actually be
    // pointed at, not necessarily `agent_id`'s on-disk casing.
    let url = format!(
        "http://{project}-{}.localhost:{DEV_PROXY_PORT}",
        agent_id.to_lowercase()
    );
    (StatusCode::OK, Json(RegisterDevServerResponse { url })).into_response()
}

/// The container the verified pane `block_id` runs in.
fn container_of_pane(state: &AppState, block_id: &str) -> Option<String> {
    let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten()?;
    crate::server::agent_resources::container_of(&block)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(state: &AppState, meta: &[(&str, &str)]) -> String {
        let meta = meta.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        let mut block = crate::backend::obj::Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
        state.mstore.insert(&mut block).unwrap();
        block.oid
    }

    #[tokio::test]
    async fn the_container_is_named_from_the_panes_definition_id_not_the_slug() {
        let state = crate::server::tests::test_state();
        let block = pane(&state, &[("agentMode", "container"), ("agentId", "5d2c7e1a-0000-4000-8000-000000000001")]);
        assert_eq!(container_of_pane(&state, &block).as_deref(), Some("agentmux-5d2c7e1a-0000-4000-8000-000000000001"));
    }

    #[tokio::test]
    async fn a_host_agent_or_an_unknown_pane_has_no_container() {
        let state = crate::server::tests::test_state();
        let host = pane(&state, &[("agentMode", "host"), ("agentId", "lark")]);
        assert_eq!(container_of_pane(&state, &host), None);
        assert_eq!(container_of_pane(&state, "no-such-pane"), None);
    }
}
