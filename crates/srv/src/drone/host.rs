// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server a running drone's blocks reach AgentMux through.
//!
//! A block is `run(node, &scope)` and has no server handle in its signature
//! (the engine's rule: Drone phase 2, the Drone real-world jobs spec §4.2).
//! The blocks that act on AgentMux itself (App query, Message, Task, Spawn, Notify) read it here.
//! Set once at startup, the way `server::job_actions::install` and `server::jekt_cold::install` are.

use std::sync::{Arc, OnceLock};

use crate::server::AppState;

static HOST: OnceLock<AppState> = OnceLock::new();

/// Give the drone blocks the server. Later calls change nothing.
pub(crate) fn install(state: &AppState) {
    let _ = HOST.set(state.clone());
}

/// Where a Drone block finds the drone it calls.
pub(crate) trait DroneLookup: Send + Sync {
    /// The graph of drone `id` at saved `version` (as it is saved now when none),
    /// with the number of the version it is (0 for a graph that is no saved version).
    fn graph(&self, id: &str, version: Option<i64>) -> Result<(crate::drone::types::DroneGraph, i64), String>;
}

/// The drones this AgentMux has saved.
struct Saved;

impl DroneLookup for Saved {
    fn graph(&self, id: &str, version: Option<i64>) -> Result<(crate::drone::types::DroneGraph, i64), String> {
        use crate::backend::storage::drone_store::DroneStore;
        use crate::drone::versions::{graph_hash, DroneVersionStore, Pin};
        let state = state()?;
        if let Some(n) = version {
            return match state.id_store.version_get(id, n) {
                Ok(Some(v)) => Ok((v.graph, v.version)),
                Ok(None) => Err(format!("drone {id} has no saved version {n}")),
                Err(e) => Err(format!("drone {id} cannot be read: {e}")),
            };
        }
        match state.id_store.drone_get(id) {
            Ok(Some(d)) => {
                let number = match state.id_store.version_resolve(id, Pin::Latest) {
                    Ok(Some(v)) if v.hash == graph_hash(&d.graph) => v.version,
                    _ => 0,
                };
                Ok((d.graph, number))
            }
            Ok(None) => Err(format!("there is no drone {id}")),
            Err(e) => Err(format!("drone {id} cannot be read: {e}")),
        }
    }
}

/// The lookup a run gives its Drone blocks: the saved drones, once the server is set.
pub(crate) fn lookup() -> Option<Arc<dyn DroneLookup>> {
    HOST.get().map(|_| Arc::new(Saved) as Arc<dyn DroneLookup>)
}

/// The server, or why a block can't act yet.
pub(crate) fn state() -> Result<&'static AppState, String> {
    HOST.get().ok_or_else(|| "AgentMux isn't ready for this block yet".to_string())
}
