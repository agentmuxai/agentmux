// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `memory.global.put`, `memory.global.remove` and `memory.global.list`: one Global Memory for
//! the fleet (task #12439, section 4.6). The rules for which write wins and what is never touched
//! are in `backend::fleet_memory`; these are the thin handlers the transport calls.
//!
//! A write is attributed to the remote agent as `slug@machine`, never to the operator, and with
//! no uid: the caller's uid is signed but not tied to its key, so it describes and never proves.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::backend::fleet_memory::{self, Actor, FleetEntry, FleetError, FleetRemoval, Outcome};
use crate::backend::storage::store::Store;

fn parse<T: DeserializeOwned>(op: &str, params: Value) -> Result<T, RemoteRefusal> {
    serde_json::from_value(params).map_err(|e| RemoteRefusal::new("bad_params", format!("{op}: {e}")))
}

/// Run a store write off the async threads and tell the Memory pane when it changed something.
async fn write(
    state: &AppState,
    caller: &RemoteCaller,
    run: impl FnOnce(&Store, Actor<'_>) -> Result<Outcome, FleetError> + Send + 'static,
) -> RemoteResult {
    let store = state.id_store.clone();
    let label = caller.actor();
    let outcome = tokio::task::spawn_blocking(move || run(&store, Actor { label: &label, uid: "" }))
        .await
        .map_err(|e| RemoteRefusal::failed(format!("the write did not finish: {e}")))?
        .map_err(|e| match e {
            FleetError::Invalid(message) => RemoteRefusal::new("bad_params", message),
            FleetError::Store(e) => RemoteRefusal::failed(e.to_string()),
        })?;
    if matches!(outcome, Outcome::Applied(_)) {
        state.broker.publish_event::<crate::backend::mps_events::MemoriesChanged>(&(), vec![], 0);
        tracing::info!(actor = %caller.actor(), outcome = %outcome.to_json(), "fleet memory: applied a write from another machine");
    }
    Ok(outcome.to_json())
}

pub(super) async fn put(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let entry: FleetEntry = parse("memory.global.put", params)?;
    write(state, caller, move |store, actor| fleet_memory::apply_put(store, &entry, actor)).await
}

pub(super) async fn remove(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let removal: FleetRemoval = parse("memory.global.remove", params)?;
    write(state, caller, move |store, actor| fleet_memory::apply_remove(store, &removal, actor)).await
}

/// What this machine holds for the fleet, without any text: the ids, when each was written, a hash
/// of its content and whether it was removed, so the caller can tell what to send.
pub(super) async fn list(state: &AppState, _caller: &RemoteCaller, _params: Value) -> RemoteResult {
    let store = state.id_store.clone();
    let entries = tokio::task::spawn_blocking(move || fleet_memory::list_meta(&store))
        .await
        .map_err(|e| RemoteRefusal::failed(format!("the listing did not finish: {e}")))?
        .map_err(|e| RemoteRefusal::failed(e.to_string()))?;
    Ok(json!({ "entries": entries }))
}

#[cfg(test)]
mod tests;
