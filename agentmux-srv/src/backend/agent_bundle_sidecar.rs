// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which bundle is an agent's own ABF memory bundle, across channels
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.6, phase M4).
//!
//! `db_agents.default_memory_id` lives in each channel's object store, so
//! migration m0021 minted a **new** bundle for an agent in every channel it
//! ran in — "Name — ABF", "Name — ABF (2)", … in the one shared store. The
//! id belongs with the agent, but not in `DefinitionRecordV1`: that would
//! need a schema bump older builds reject. So it goes in a sidecar zone in
//! the global store, `agent-uid:<uid>:bundle`, holding `{memory_id}`: the
//! first channel to provision the agent records it, and m0021 reads it
//! before minting.

use serde::{Deserialize, Serialize};

use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;

const FILE: &str = "bundle.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Sidecar {
    #[serde(default)]
    memory_id: String,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

fn zone(uid: &str) -> Option<String> {
    crate::backend::agent_session::is_valid_definition_id(uid).then(|| format!("agent-uid:{uid}:bundle"))
}

/// The agent's recorded bundle id, if any.
pub(crate) fn get(fs: &FileStore, uid: &str) -> Result<Option<String>, StoreError> {
    let Some(zone) = zone(uid) else { return Ok(None) };
    let Some(bytes) = fs.read_files_consistent(&zone, &[FILE])?.pop().flatten() else { return Ok(None) };
    let s: Sidecar = serde_json::from_slice(&bytes).map_err(|e| StoreError::Other(format!("bundle sidecar unreadable: {e}")))?;
    Ok(Some(s.memory_id).filter(|m| !m.is_empty()))
}

/// Record `memory_id` as the agent's bundle unless one is recorded already.
/// Returns the id now recorded — the caller's, or the one another channel
/// recorded first.
pub(crate) fn claim(fs: &FileStore, uid: &str, memory_id: &str) -> Result<String, StoreError> {
    let Some(zone) = zone(uid) else { return Ok(memory_id.to_string()) };
    fs.zone_txn(&zone, |z| {
        let mut s: Sidecar = match z.read(FILE)? {
            Some(b) => serde_json::from_slice(&b).map_err(|e| StoreError::Other(format!("bundle sidecar unreadable: {e}")))?,
            None => Sidecar::default(),
        };
        if !s.memory_id.is_empty() {
            return Ok(s.memory_id);
        }
        s.memory_id = memory_id.to_string();
        z.put(FILE, &serde_json::to_vec(&s).map_err(|e| StoreError::Other(e.to_string()))?)?;
        Ok(s.memory_id)
    })
}

/// Record a freshly provisioned bundle at runtime, best-effort (the global
/// store may be off).
pub(crate) fn record(uid: &str, memory_id: &str) {
    let Some(fs) = crate::backend::agent_session::global_transcript_store() else { return };
    if let Err(e) = claim(fs, uid, memory_id) {
        tracing::warn!(uid, error = %e, "agent bundle sidecar not recorded");
    }
}

/// The global store, opened directly — for migrations, which run before
/// srv attaches it. `None` when it can't be resolved or opened.
pub(crate) fn open_global() -> Option<FileStore> {
    let dir = crate::registry::resolve_shared_transcripts_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    FileStore::open(&dir.join("filestore.db")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_recorded_bundle_wins() {
        let fs = FileStore::open_in_memory().unwrap();
        assert_eq!(get(&fs, "agent-a").unwrap(), None);
        assert_eq!(claim(&fs, "agent-a", "b1").unwrap(), "b1");
        assert_eq!(claim(&fs, "agent-a", "b2").unwrap(), "b1", "another channel's later bundle doesn't replace it");
        assert_eq!(get(&fs, "agent-a").unwrap().as_deref(), Some("b1"));
        assert_eq!(get(&fs, "agent-b").unwrap(), None);
    }
}
