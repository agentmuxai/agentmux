// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Presets, identity accounts and self.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `GET /api/v1/agent/preset/list` — list all presets (shared catalog, summary
/// fields only). Backs the `PresetList` MCP tool.
pub(super) async fn handle_agent_preset_list(State(state): State<AppState>) -> impl IntoResponse {
    app_api_response(app_api::bundle_list_impl(&state).await)
}

#[derive(serde::Deserialize)]
pub(super) struct AgentPresetGetQuery {
    #[serde(default)]
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: String,
}

/// `GET /api/v1/agent/preset/get?agent_id=<slug>&id=&name=` — fetch a preset by
/// id or name; with neither, return the agent's own bound preset (self).
/// Backs the `PresetGet` MCP tool.
pub(super) async fn handle_agent_preset_get(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Query(q): Query<AgentPresetGetQuery>,
) -> impl IntoResponse {
    if q.id.is_empty() && q.name.is_empty() {
        // Self mode: `agent_id` is the actor (owner = actor). By id or name
        // it is only an optional hint, so it is not checked there.
        actor::check_actor(
            &state,
            caller.as_deref(),
            actor::ActorSite::PresetGet,
            Some(&q.agent_id),
        );
        if q.agent_id.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "preset.get: provide id, name, or agent_id (for self)" })),
            )
                .into_response();
        }
        return app_api_response(
            app_api::bundle_self_get_impl(
                &state,
                app_api::SelfOwner::of(caller.as_deref(), &q.agent_id, "m4c.preset_owner_by_name"),
            )
            .await,
        );
    }
    app_api_response(app_api::bundle_get_impl(&state, &q.id, &q.name).await)
}

#[derive(serde::Deserialize)]
pub(super) struct AgentIdentityAccountsQuery {
    pub(super) agent_id: String,
}

/// `GET /api/v1/agent/identity/accounts?agent_id=<slug>` — list the agent's own
/// linked credential accounts (masked tails only, never secrets). Backs the
/// `IdentityAccounts` MCP tool.
pub(super) async fn handle_agent_identity_accounts(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Query(q): Query<AgentIdentityAccountsQuery>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::IdentityAccounts,
        Some(&q.agent_id),
    );
    app_api_response(
        app_api::identity_self_accounts_impl(
            &state,
            app_api::SelfOwner::of(caller.as_deref(), &q.agent_id, "m4c.identity_owner_by_name"),
        )
        .await,
    )
}

#[derive(serde::Deserialize)]
pub(super) struct AgentIdentityValidateRequest {
    pub(super) agent_id: String,
    pub(super) account_id: String,
}

/// `POST /api/v1/agent/identity/validate` — live-probe one of the agent's own
/// linked accounts using its stored keychain secret (the agent never supplies a
/// secret). Backs the `IdentityValidate` MCP tool.
pub(super) async fn handle_agent_identity_validate(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentIdentityValidateRequest>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::IdentityValidate,
        Some(&req.agent_id),
    );
    app_api_response(
        app_api::identity_account_validate_stored_impl(
            &state,
            app_api::SelfOwner::of(caller.as_deref(), &req.agent_id, "m4c.identity_owner_by_name"),
            &req.account_id,
        )
        .await,
    )
}

#[derive(serde::Deserialize)]
pub(super) struct SelfQuery {
    /// Block UUID of the calling agent pane (its `AGENTMUX_BLOCKID`).
    pub(super) block_id: Option<String>,
}

/// `GET /api/v1/self?block_id=<id>` — resolve the calling agent's place in the
/// object tree (block → tab → window → workspace, with their names). The
/// sidecar serves many agents, so the caller identifies itself by its block id
/// (the MCP `WhoAmI` tool passes `AGENTMUX_BLOCKID`). Naming verbs reuse the
/// same resolver to default their target to the agent's own context.
pub(super) async fn handle_self(
    State(state): State<AppState>,
    Query(q): Query<SelfQuery>,
) -> impl IntoResponse {
    let block_id = q.block_id.unwrap_or_default();
    if block_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing block_id" }))).into_response();
    }
    match service::resolve_agent_context(&state.mstore, &block_id) {
        Ok(ctx) => Json(serde_json::to_value(&ctx).unwrap_or_default()).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
    }
}
