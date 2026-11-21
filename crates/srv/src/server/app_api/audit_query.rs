// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `audit.query` (`backend::audit_query`): a page of the Audit view's list. The window's, not an agent API: the
//! Audit is the operator's view of every agent, so a connection registered as an agent is refused.
//!
//! On the `machine` scope's first page it also asks the other channels on this computer for the rows they keep in
//! their own file (`handle_audit_own`, the route they answer on), so a channel that stays out of the shared folder is
//! still listed.

use super::*;
use crate::backend::audit_query::{self, Ctx, Sources};
use crate::backend::rpc_types::{AuditChannelRows, AuditQueryReq, AuditQueryResult};
use std::path::Path;

/// A page from this srv's stores. `own_only` reads only what this srv keeps in its own files, and leaves out the
/// shared audit file, the shared activity folder and the shared segment ledger: what another channel cannot read.
pub(in crate::server) fn query_here(state: &AppState, req: &AuditQueryReq, own_only: bool) -> Result<AuditQueryResult, String> {
    let slugs = state.mstore.shared_def_registry().and_then(|reg| reg.slug_index().ok());
    let slug_of_uid = |uid: &str| slugs.as_ref().and_then(|s| s.get(uid)).map(|e| e.slug.clone());
    let uid_of_slug = |slug: &str| {
        let all = slugs.as_ref()?;
        let by_slug = all.iter().find(|(_, e)| e.slug.eq_ignore_ascii_case(slug));
        by_slug.or_else(|| all.iter().find(|(_, e)| e.former.iter().any(|f| f.eq_ignore_ascii_case(slug)))).map(|(uid, _)| uid.clone())
    };
    let pane_of_agent = |slug: &str| match state.reactive_handler.lookup_by_name(slug) {
        crate::backend::reactive::handler::LookupOutcome::One(reg) => Some(reg.block_id).filter(|b| !b.is_empty()),
        _ => None,
    };
    let block_exists = |id: &str| state.mstore.get::<crate::backend::obj::Block>(id).ok().flatten().is_some();
    let file = crate::backend::reactive::audit_store::shared_path_for_reading().filter(|_| !own_only);
    let own = crate::backend::reactive::audit_store::own_path();
    let channel = std::env::var("AGENTMUX_INSTANCE_CHANNEL").unwrap_or_default();
    let src = Sources {
        channel: &channel,
        jekt_file: file.as_deref(),
        own_file: own.as_deref(),
        activity_dir: file.as_deref().and_then(Path::parent),
        own_activity_dir: own.as_deref().and_then(Path::parent),
        segments: state.global_transcript_store.as_deref().filter(|_| !own_only),
        ctx: Ctx { slug_of_uid: &slug_of_uid, uid_of_slug: &uid_of_slug, pane_of_agent: &pane_of_agent, block_exists: &block_exists },
    };
    let mut result = audit_query::query(&src, req)?;
    // The task each row's agent held at the time, and its PR: from the Kanban's history.
    let held = audit_query::tasks::from_store(&state.work_store);
    audit_query::tasks::attach(&mut result.entries, &held);
    Ok(result)
}

/// Whether a request is the first page of a scope that lists more than this channel, the only page that asks the
/// other channels (and, for `lan`, the paired computers).
fn asks_beyond_this_channel(req: &AuditQueryReq) -> bool {
    req.cursor.is_none() && matches!(req.scope.as_deref().map(str::trim), None | Some("") | Some("machine") | Some("lan"))
}

/// A page of this computer's Audit as the window lists it: this channel's stores, and on the first page of a scope
/// that lists more, the rows the other channels on this computer keep in their own files.
pub(in crate::server) async fn machine_page(state: &AppState, req: &AuditQueryReq) -> Result<AuditQueryResult, String> {
    let here = {
        let (state, req) = (state.clone(), req.clone());
        // Reads files, the whole segment ledger and the registry: off the async workers.
        tokio::task::spawn_blocking(move || query_here(&state, &req, false)).await.map_err(|e| format!("audit.query: {e}"))?
    };
    let mut result = here?;
    if asks_beyond_this_channel(req) {
        result.other_channels = Some(other_channels(state, req).await).filter(|rows| !rows.is_empty());
    }
    Ok(result)
}

/// The other channels that are running, asked for their own rows.
async fn other_channels(state: &AppState, req: &AuditQueryReq) -> Vec<AuditChannelRows> {
    let entries = tokio::task::spawn_blocking(|| {
        crate::registry::resolve_shared_reactive_dir().map(|dir| crate::backend::reactive::registry::list_all_shared(&dir)).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let own_channel = crate::backend::reactive::registry::local_channel_id();
    let now_ms = agentmux_common::time::now_ms_u64();
    let targets = crate::backend::swarm_remote::channel_status_targets(&entries, &own_channel, &state.local_web_url, now_ms);
    audit_query::other_channels::ask(&state.http_client, &targets, req).await
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_AUDIT_QUERY, move |req: AuditQueryReq, ctx| {
        let state = state.clone();
        async move {
            ctx.operator_only("the Audit is the AgentMux window's, not an agent API")?;
            let mut result = machine_page(&state, &req).await?;
            // The paired computers are asked on the `lan` scope's first page, and shown and forgotten.
            if req.cursor.is_none() && req.scope.as_deref().map(str::trim) == Some("lan") {
                result.peers = Some(crate::server::remote_ops::audit_ops::ask_peers(&state, &req).await).filter(|rows| !rows.is_empty());
            }
            Ok(result)
        }
    });
}

/// `POST /agentmux/audit/own`: what this channel keeps in its own file, as an `audit.query` page, for another channel
/// on this computer that is listing every channel. Full key only, never LAN; an agent's token is refused, as the
/// Audit is not an agent API.
pub(in crate::server) async fn handle_audit_own(
    axum::extract::State(state): axum::extract::State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    axum::Json(req): axum::Json<AuditQueryReq>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if caller.is_some_and(|c| c.0.uid().is_some()) {
        return (axum::http::StatusCode::FORBIDDEN, "the Audit is the AgentMux window's, not an agent API").into_response();
    }
    match tokio::task::spawn_blocking(move || query_here(&state, &req, true)).await {
        Ok(Ok(result)) => axum::Json(result).into_response(),
        Ok(Err(e)) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("audit own: {e}")).into_response(),
    }
}
