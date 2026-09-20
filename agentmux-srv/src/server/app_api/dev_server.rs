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

use crate::backend::container::container_name_for_slug;
use crate::backend::dev_proxy::DEV_PROXY_PORT;

use super::AppState;

pub(crate) async fn handle_register_dev_server(
    State(state): State<AppState>,
    Json(req): Json<RegisterDevServerRequest>,
) -> impl IntoResponse {
    // Verifies `req.auth`'s signature against the claimed agent_id's own
    // key on file. We don't need the returned block_id — the signature
    // check succeeding is itself the proof that `req.auth.agent_id` is who
    // it claims to be. See this module's doc comment.
    if let Err(e) = crate::server::ui_handlers::verified_block_id(&state, &req.auth) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))).into_response();
    }
    let agent_id = req.auth.agent_id.clone();

    let project = req.project.trim().to_lowercase();
    if project.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "project must not be empty" })),
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

    let container_name = container_name_for_slug(&agent_id);
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

    let addr: std::net::SocketAddr = match format!("{ip}:{}", req.port).parse() {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid address {ip}:{}: {e}", req.port) })),
            )
                .into_response();
        }
    };

    state.dev_proxy.register(&project, &agent_id, addr).await;

    // Lowercased to match the actual routing key (`DevProxyRegistry::routing_key`
    // lowercases both halves) — this is what a browser should actually be
    // pointed at, not necessarily `agent_id`'s on-disk casing.
    let url = format!(
        "http://{project}-{}.localhost:{DEV_PROXY_PORT}",
        agent_id.to_lowercase()
    );
    (StatusCode::OK, Json(RegisterDevServerResponse { url })).into_response()
}
