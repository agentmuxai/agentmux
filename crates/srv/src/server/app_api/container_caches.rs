// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container agent's cache volumes (a developer agent's cargo registry and build output): how much they hold, and
//! clearing them. `Agents container_caches` (`POST /api/v1/agent/container/caches`), and the window's
//! `getcontainercaches` / `clearcontainercaches`, which share [`view`] and [`clear`].
//! `docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md` section 5.
//!
//! Clearing removes the agent's container along with the volumes (a volume in use can't go), so it waits for a turn
//! to finish like an image update does; the next message starts a new container, which fills them again. A container
//! agent can read its caches but not clear them: that is the host's to do, like its limits.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::container::{container_name_for_slug, ContainerManager};
use crate::backend::rpc_types::{AgentContainerCachesResult, ContainerCacheVolume};
use crate::server::app_api::container_limits::{container_agent, Refusal};
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// Above this much in an agent's caches its pane says so (20 GiB).
pub(crate) const WARN_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// The containers whose pane has been told about large caches in this run of srv: one notice each.
static WARNED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Whether this is the first time `container_name` is to be told about it (and records that it was).
fn first_warning(container_name: &str) -> bool {
    WARNED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(container_name.to_string())
}

/// Lets `container_name` be warned again, after its caches were cleared.
fn forget_warning(container_name: &str) {
    if let Some(set) = WARNED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        set.remove(container_name);
    }
}

/// Writes one row in the pane, once per agent per run of srv, when the agent's cache volumes hold [`WARN_BYTES`] or
/// more. Docker's disk-usage report can take a while with many volumes, so a start-up runs this in the background.
pub(crate) async fn note_if_large(
    cm: &ContainerManager,
    container_name: &str,
    block_id: &str,
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
) {
    if WARNED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|set| set.contains(container_name)) {
        return;
    }
    let Ok(volumes) = cm.cache_volumes(container_name).await else { return };
    let total: u64 = volumes.iter().map(|v| v.bytes).sum();
    if total < WARN_BYTES || !first_warning(container_name) {
        return;
    }
    let meta = |key: &str, default: &str| match mstore.get::<crate::backend::obj::Block>(block_id) {
        Ok(Some(block)) => crate::backend::obj::meta_get_string(&block.meta, key, default),
        _ => default.to_string(),
    };
    let (provider, agent_id, agent_name) = (meta("agentProvider", "claude"), meta("agentId", ""), meta("agentName", ""));
    tracing::warn!(container = container_name, bytes = total, "a container agent's cache volumes are large");
    let frame = crate::backend::cli_notice::container_caches_frame(&provider, block_id, (&agent_id, &agent_name), total);
    crate::backend::cli_notice::append_to_pane(broker, filestore, mstore, block_id, &frame);
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    (status, message.into())
}

fn result_of(volumes: Vec<ContainerCacheVolume>, freed: Option<u64>) -> AgentContainerCachesResult {
    let total_bytes = volumes.iter().map(|v| v.bytes).sum();
    AgentContainerCachesResult {
        volumes,
        total_bytes,
        warn_bytes: WARN_BYTES,
        freed_bytes: freed,
    }
}

/// What the agent with uid `uid` holds in cache volumes now.
pub(crate) async fn view(state: &AppState, uid: &str) -> Result<AgentContainerCachesResult, Refusal> {
    let Some(cm) = state.container_manager.get().await else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "Docker is not available on this machine"));
    };
    let volumes = cm.cache_volumes(&container_name_for_slug(uid)).await.map_err(|e| refuse(StatusCode::BAD_GATEWAY, e))?;
    Ok(result_of(volumes, None))
}

/// Removes the agent's cache volumes, and its container with them; answers with what is left (nothing) and what was freed.
pub(crate) async fn clear(state: &AppState, uid: &str) -> Result<AgentContainerCachesResult, Refusal> {
    let Some(cm) = state.container_manager.get().await else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "Docker is not available on this machine"));
    };
    let name = container_name_for_slug(uid);
    // Every turn of this agent holds the gate shared: clearing waits for the turn rather than pulling the container out from under it.
    let gate = crate::backend::container::drift::update_gate(&name);
    let _clearing = gate.try_write_owned().map_err(|_| refuse(StatusCode::CONFLICT, "a turn is running; clear the caches when it has finished"))?;
    let removed = cm.clear_caches(&name).await.map_err(|e| refuse(StatusCode::BAD_GATEWAY, e))?;
    let freed = removed.iter().map(|v| v.bytes).sum();
    forget_warning(&name);
    tracing::info!(container = %name, volumes = removed.len(), freed_bytes = freed, "cleared a container agent's cache volumes");
    Ok(result_of(Vec::new(), Some(freed)))
}

/// `POST /api/v1/agent/container/caches`: `Agents container_caches`. Without `clear` it only reads.
#[derive(Debug, Deserialize)]
pub(crate) struct CachesRequest {
    agent: String,
    #[serde(default)]
    clear: bool,
}

async fn run(state: &AppState, caller: Option<&Caller>, req: CachesRequest) -> Result<Value, Refusal> {
    let (uid, def) = container_agent(state, &req.agent)?;
    if let Some(caller_uid) = caller.and_then(Caller::uid) {
        let caller_def = state
            .mstore
            .agent_def_get(caller_uid)
            .ok()
            .flatten()
            .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
        if req.clear && caller_def.agent_type == "container" {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                "CONTAINER_AGENT: a container agent cannot clear a container's caches, its own included: it is the container \
                 being removed. Ask a host agent, your steward or the operator to clear them",
            ));
        }
    }
    let caches = if req.clear { clear(state, &uid).await? } else { view(state, &uid).await? };
    Ok(json!({
        "agent": def.name,
        "caches": caches,
        "note": "bytes are what Docker reports for each volume; total_bytes above warn_bytes is a lot. With clear, the container is \
                 removed with the volumes and starts again at the agent's next message, filling them again (the first build is cold)",
    }))
}

pub(crate) async fn handle_container_caches(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<CachesRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err((status, message)) => (status, Json(json!({ "error": message }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_totals_the_volumes_and_carries_the_warning_level() {
        let r = result_of(vec![ContainerCacheVolume { name: "a".into(), bytes: 3 }, ContainerCacheVolume { name: "b".into(), bytes: 4 }], None);
        assert_eq!((r.total_bytes, r.warn_bytes, r.freed_bytes), (7, WARN_BYTES, None));
        assert_eq!(r.volumes.len(), 2);
        let cleared = result_of(Vec::new(), Some(7));
        assert_eq!((cleared.total_bytes, cleared.freed_bytes), (0, Some(7)));
    }

    #[test]
    fn an_agent_is_told_once_until_its_caches_are_cleared() {
        assert!(first_warning("agentmux-test-warned"));
        assert!(!first_warning("agentmux-test-warned"), "the second start says nothing");
        forget_warning("agentmux-test-warned");
        assert!(first_warning("agentmux-test-warned"), "after a clear it may say so again");
        forget_warning("agentmux-test-warned");
    }
}
