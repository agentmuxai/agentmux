// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The human "release this folder" action
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, phase M3c).
//!
//! An agent's claim on a memory folder makes that folder shared for every
//! other agent, so nothing else's memory is written or captured there. A
//! claim is released on its own when the agent is deleted — but not when
//! it merely moves on (a new account, working directory or channel), and a
//! stale claim then keeps the folder shared for good. A human can release
//! it.
//!
//! Releasing can make the folder exclusive to another agent, whose next
//! spawn then writes and deletes in it, so it is confirmed the same way as
//! adoption: [`list`] issues the agent's claims under a `list_id`; the
//! release itself ([`release`]) is reached only through the host's
//! confirmation window (`memoryadopt.Release`, host secret), naming
//! `(list_id, index)`. If the agent still uses the folder, its next spawn
//! there simply claims it again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::backend::memory_dir_claims as claims;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;

const LIST_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NativeMemoryClaimedFolder")]
pub(crate) struct ClaimedFolderView {
    pub index: usize,
    /// The folder, when the claim recorded it.
    pub dir: Option<String>,
    #[ts(type = "number")]
    pub claimed_at_ms: i64,
    /// Other agents claiming it too (it is shared while they do).
    pub others: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/", rename = "NativeMemoryClaimList")]
pub(crate) struct ClaimList {
    pub list_id: String,
    pub agent_uid: String,
    pub folders: Vec<ClaimedFolderView>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum ReleaseError {
    UnknownList,
    BadChoice,
    Store(String),
}

impl From<StoreError> for ReleaseError {
    fn from(e: StoreError) -> Self {
        ReleaseError::Store(e.to_string())
    }
}

struct Issued {
    agent_uid: String,
    zones: Vec<String>,
    at: Instant,
}

fn issued() -> &'static Mutex<HashMap<String, Issued>> {
    static ISSUED: std::sync::OnceLock<Mutex<HashMap<String, Issued>>> = std::sync::OnceLock::new();
    ISSUED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The folders `agent_uid` has claimed, under a new `list_id`.
pub(crate) fn list(fs: &FileStore, agent_uid: &str) -> Result<ClaimList, StoreError> {
    let found = claims::claims_of(fs, agent_uid)?;
    let folders = found
        .iter()
        .enumerate()
        .map(|(index, c)| ClaimedFolderView { index, dir: c.dir.clone(), claimed_at_ms: c.claimed_at_ms, others: c.others })
        .collect();
    let list_id = uuid::Uuid::new_v4().simple().to_string();
    let mut map = issued().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, i| i.at.elapsed() < LIST_TTL);
    map.insert(
        list_id.clone(),
        Issued { agent_uid: agent_uid.to_string(), zones: found.into_iter().map(|c| c.zone).collect(), at: Instant::now() },
    );
    Ok(ClaimList { list_id, agent_uid: agent_uid.to_string(), folders })
}

/// Release `agent_uid`'s claim on folder `index` of list `list_id`. Returns
/// whether a claim was removed (it may already be gone).
pub(crate) fn release(fs: &FileStore, agent_uid: &str, list_id: &str, index: usize) -> Result<bool, ReleaseError> {
    let zone = {
        let map = issued().lock().unwrap_or_else(|e| e.into_inner());
        let list = map
            .get(list_id)
            .filter(|l| l.agent_uid == agent_uid && l.at.elapsed() < LIST_TTL)
            .ok_or(ReleaseError::UnknownList)?;
        list.zones.get(index).cloned().ok_or(ReleaseError::BadChoice)?
    };
    Ok(claims::release(fs, agent_uid, &zone)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    /// Agents need rows: a claim by an agent this store has no row for is
    /// released as stale by the next claim.
    fn claim(fs: &FileStore, store: &Store, uid: &str, dir: &std::path::Path) {
        if store.agent_def_get(uid).ok().flatten().is_none() {
            let mut def = crate::backend::storage::agents::test_agent_def(uid, uid, "claude", "agent", 1, "");
            def.working_directory = format!("/work/{uid}");
            store.agent_def_insert(&mut def).unwrap();
        }
        claims::check_and_claim(fs, store, uid, dir, "/work/x", Instant::now() + Duration::from_secs(10)).unwrap();
    }

    #[test]
    fn an_agents_claims_are_listed_with_their_folders_and_released_by_list_index() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        claim(&fs, &store, "agent-rel", a.path());
        claim(&fs, &store, "agent-rel", b.path());
        claim(&fs, &store, "agent-other", b.path());
        let list = list(&fs, "agent-rel").unwrap();
        assert_eq!(list.folders.len(), 2);
        let b_dir = claims::dir_id(b.path());
        let b_entry = list.folders.iter().find(|f| f.dir.as_deref() == Some(b_dir.as_str())).unwrap();
        assert_eq!(b_entry.others, 1);

        assert_eq!(release(&fs, "agent-rel", &list.list_id, b_entry.index), Ok(true));
        let after = claims::claims_of(&fs, "agent-rel").unwrap();
        assert_eq!(after.len(), 1);
        assert_ne!(after[0].dir.as_deref(), Some(b_dir.as_str()));
        // The other agent's claim on it is untouched: now it alone holds it.
        assert!(claims::held_exclusively(&fs, "agent-other", b.path()).unwrap());
    }

    #[test]
    fn only_the_agents_own_list_releases_its_claims() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        let a = tempfile::tempdir().unwrap();
        claim(&fs, &store, "agent-rel", a.path());
        let list = list(&fs, "agent-rel").unwrap();
        assert_eq!(release(&fs, "agent-else", &list.list_id, 0), Err(ReleaseError::UnknownList));
        assert_eq!(release(&fs, "agent-rel", "no-such-list", 0), Err(ReleaseError::UnknownList));
        assert_eq!(release(&fs, "agent-rel", &list.list_id, 9), Err(ReleaseError::BadChoice));
        assert_eq!(claims::claims_of(&fs, "agent-rel").unwrap().len(), 1);
    }
}
