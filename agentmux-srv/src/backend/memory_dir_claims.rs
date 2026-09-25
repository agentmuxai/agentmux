// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Is a memory directory this agent's alone?
//! (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, phase M3b.)
//!
//! A Claude memory directory is keyed by account + working directory, not by
//! agent, so two agents can share one — the same account and working
//! directory, or two unlinked agents with the same working directory. The
//! memory record may project into, capture from, or delete in a directory
//! only when it is proven exclusive; a shared one is left exactly as it is.
//!
//! A directory is **shared unless proven exclusive**:
//! 1. **Known shared locations** — a blank working directory (the
//!    `~/.agentmux/agents/<slug>` default, shared across same-name tabs),
//!    `$HOME`, or an ancestor of another agent's working directory. And any
//!    folder the CLI keys by a whole repository rather than this working
//!    directory alone: a subdirectory or linked worktree of a repository, or
//!    a main checkout with linked worktrees
//!    ([`crate::backend::claude_layout::memory_project_root`]).
//! 2. **Vetoes** — any other agent whose directory resolves, by any route,
//!    to the same canonical directory. Guesses count here (a veto only ever
//!    removes a directory; the resolver never uses one to *find* memory).
//! 3. **A machine-wide claim** — zone `memory-dir:<hash>` in the global
//!    store. Every spawn that uses the folder records one, a shared one
//!    too: channels share `projects/` folders, and an agent in another
//!    channel is visible here only through its claim. A claim by another
//!    UID makes the folder shared. A claim is released only by the channel
//!    store that wrote it, once that store no longer has the agent and no
//!    active shared definition does — another channel can't tell a live
//!    agent of its neighbour's from a deleted one.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::backend::claude_layout as layout;
use crate::backend::memory_record::sha256_hex;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

const CLAIMS_FILE: &str = "claims.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Exclusivity {
    Exclusive,
    Shared(SharedReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharedReason {
    /// A blank working directory, `$HOME`, or an ancestor of another
    /// agent's working directory.
    KnownSharedLocation,
    /// The CLI keys this folder by more than the agent's own working
    /// directory: it is a subdirectory or linked worktree of a repository
    /// (memory is keyed by the main checkout), or the main checkout of one
    /// with linked worktrees. Whoever works in the rest of that repository —
    /// a person, an agent in another channel — shares it unseen.
    RepositoryWide,
    /// Another agent resolves to the same directory.
    Vetoed { other_uid: String },
    /// Another live agent has claimed the directory.
    Claimed { other_uid: String },
    /// The checks didn't finish before the deadline; unproven means shared.
    Unproven,
}

/// A directory's canonical form: symlinks resolved as far as the path
/// exists (the per-channel `projects/` links into the shared store), the
/// rest kept as given. Two routes to one folder get one id.
pub(crate) fn dir_id(path: &Path) -> String {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(c) = existing.canonicalize() {
            let mut out = c;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out.to_string_lossy().into_owned();
        }
        match (existing.file_name().map(|n| n.to_os_string()), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return path.to_string_lossy().into_owned(),
        }
    }
}

fn claims_zone(dir_id: &str) -> String {
    format!("memory-dir:{}", &sha256_hex(dir_id.as_bytes())[..32])
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Claims {
    #[serde(default)]
    uids: BTreeMap<String, Claim>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Claim {
    #[serde(default)]
    at_ms: i64,
    /// The channel store the claiming agent lives in ([`Store::origin_id`]).
    #[serde(default)]
    origin: String,
}

/// Whether a claim can be shown to be stale: only the store that wrote it
/// can say its agent is gone.
fn claim_is_stale(mstore: &Store, origin: &str, defs: Option<&crate::registry::DefinitionStore>, uid: &str, c: &Claim) -> bool {
    c.origin == origin && !matches!(mstore.agent_def_get(uid), Ok(Some(_))) && !defs.is_some_and(|d| d.exists(uid))
}

/// Decide whether `dir` — the memory directory `uid`'s spawn uses, from
/// working directory `cwd` — is exclusive to `uid`, recording `uid`'s claim.
/// Checks that can't finish by `deadline` leave the directory `Unproven`,
/// i.e. shared.
pub(crate) fn check_and_claim(
    fs: &FileStore,
    mstore: &Store,
    uid: &str,
    dir: &Path,
    cwd: &str,
    deadline: std::time::Instant,
) -> Result<Exclusivity, StoreError> {
    let id = dir_id(dir);
    // Claimed first, whatever the verdict: a spawn using a folder it may
    // not write still has to be visible to the agents that might.
    let defs = crate::registry::resolve_shared_definitions_dir()
        .and_then(|dir| crate::registry::DefinitionStore::open(dir).ok());
    let origin = mstore.origin_id();
    let other = fs.zone_txn(&claims_zone(&id), |z| {
        let mut claims: Claims = match z.read(CLAIMS_FILE)? {
            None => Claims::default(),
            Some(b) => serde_json::from_slice(&b)
                .map_err(|e| StoreError::Other(format!("memory dir claims unreadable: {e}")))?,
        };
        claims.uids.retain(|claimant, c| claimant == uid || !claim_is_stale(mstore, &origin, defs.as_ref(), claimant, c));
        let mine = claims.uids.entry(uid.to_string()).or_insert_with(|| Claim { at_ms: agentmux_common::time::now_ms(), origin: String::new() });
        mine.origin = origin.clone();
        let other = claims.uids.keys().find(|c| c.as_str() != uid).cloned();
        z.put(CLAIMS_FILE, &serde_json::to_vec(&claims).map_err(|e| StoreError::Other(e.to_string()))?)?;
        Ok(other)
    })?;
    if let Some(reason) = known_shared_or_vetoed(mstore, uid, &id, cwd, deadline) {
        return Ok(Exclusivity::Shared(reason));
    }
    Ok(match other {
        Some(other_uid) => Exclusivity::Shared(SharedReason::Claimed { other_uid }),
        None => Exclusivity::Exclusive,
    })
}

/// Whether `uid` still holds the only claim on `dir` — the re-check before
/// every capture while the agent runs (spec §2.1.3). Read-only: the full
/// check ran at spawn; this sees any agent that has claimed it since.
pub(crate) fn held_exclusively(fs: &FileStore, uid: &str, dir: &Path) -> Result<bool, StoreError> {
    let zone = claims_zone(&dir_id(dir));
    let Some(bytes) = fs.read_files_consistent(&zone, &[CLAIMS_FILE])?.pop().flatten() else {
        return Ok(false);
    };
    let claims: Claims =
        serde_json::from_slice(&bytes).map_err(|e| StoreError::Other(format!("memory dir claims unreadable: {e}")))?;
    Ok(claims.uids.contains_key(uid) && claims.uids.len() == 1)
}

/// Rules 1 and 2: no store writes.
fn known_shared_or_vetoed(
    mstore: &Store,
    uid: &str,
    dir_id: &str,
    cwd: &str,
    deadline: std::time::Instant,
) -> Option<SharedReason> {
    let own = mstore.agent_def_get(uid).ok().flatten();
    if own.as_ref().is_none_or(|a| a.working_directory.trim().is_empty()) {
        return Some(SharedReason::KnownSharedLocation);
    }
    let cwd = crate::backend::base::expand_home_dir_safe(cwd);
    let root = layout::memory_project_root(&cwd);
    if root != layout::physical_dir(&cwd) || layout::has_linked_worktrees(&root) {
        return Some(SharedReason::RepositoryWide);
    }
    if dirs::home_dir().is_some_and(|h| h == cwd || layout::physical_dir(&h) == root) {
        return Some(SharedReason::KnownSharedLocation);
    }
    let agents = mstore.agent_def_list().unwrap_or_default();
    for other in agents.iter().filter(|a| a.id != uid) {
        if std::time::Instant::now() > deadline {
            return Some(SharedReason::Unproven);
        }
        if !other.working_directory.trim().is_empty() {
            let other_cwd = crate::backend::base::expand_home_dir_safe(&other.working_directory);
            if other_cwd != cwd && other_cwd.starts_with(&cwd) {
                return Some(SharedReason::KnownSharedLocation);
            }
            let other_physical = layout::physical_dir(&other_cwd);
            if other_physical != root && other_physical.starts_with(&root) {
                return Some(SharedReason::KnownSharedLocation);
            }
        }
        let resolved = crate::server::native_memory_handlers::resolve_memory_dir_by_id(mstore, other);
        if resolved.is_some_and(|r| self::dir_id(&r.path) == dir_id) {
            return Some(SharedReason::Vetoed { other_uid: other.id.clone() });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::AgentDefinition;
    use std::path::PathBuf;

    fn agent(store: &Store, id: &str, wd: &str) -> AgentDefinition {
        let mut def = crate::backend::storage::agents::test_agent_def(id, id, "claude", "agent", 1, "");
        def.working_directory = wd.to_string();
        store.agent_def_insert(&mut def).unwrap();
        def
    }

    fn check_and_claim_now(fs: &FileStore, store: &Store, uid: &str, dir: &Path, cwd: &str) -> Result<Exclusivity, StoreError> {
        check_and_claim(fs, store, uid, dir, cwd, std::time::Instant::now() + std::time::Duration::from_secs(30))
    }

    #[test]
    fn checks_past_the_deadline_leave_the_directory_unproven() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        agent(&store, "agent-a", "/work/a");
        agent(&store, "agent-b", "/work/b");
        let dir = tempfile::tempdir().unwrap();
        let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
        assert_eq!(
            check_and_claim(&fs, &store, "agent-a", dir.path(), "/work/a", past).unwrap(),
            Exclusivity::Shared(SharedReason::Unproven)
        );
    }

    fn memory_dir(store: &Store, def: &AgentDefinition) -> PathBuf {
        crate::server::native_memory_handlers::memory_dir_for_agent_by_id(store, def).unwrap()
    }

    #[test]
    fn an_agent_alone_in_its_directory_is_exclusive_and_claims_it() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        let a = agent(&store, "agent-a", "/work/a");
        agent(&store, "agent-b", "/work/b");
        let dir = memory_dir(&store, &a);
        assert_eq!(check_and_claim_now(&fs, &store, "agent-a", &dir, "/work/a").unwrap(), Exclusivity::Exclusive);
        assert_eq!(check_and_claim_now(&fs, &store, "agent-a", &dir, "/work/a").unwrap(), Exclusivity::Exclusive, "re-claiming is idempotent");
    }

    #[test]
    fn another_agent_resolving_to_the_same_directory_vetoes_it() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        let a = agent(&store, "agent-a", "/work/shared");
        agent(&store, "agent-b", "/work/shared");
        let dir = memory_dir(&store, &a);
        assert_eq!(
            check_and_claim_now(&fs, &store, "agent-a", &dir, "/work/shared").unwrap(),
            Exclusivity::Shared(SharedReason::Vetoed { other_uid: "agent-b".into() })
        );
    }

    #[test]
    fn blank_home_and_ancestor_directories_are_known_shared() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        agent(&store, "blank", "");
        agent(&store, "parent", "/work/repo");
        agent(&store, "child", "/work/repo/sub");
        let tmp = tempfile::tempdir().unwrap();
        let shared = Exclusivity::Shared(SharedReason::KnownSharedLocation);
        assert_eq!(check_and_claim_now(&fs, &store, "blank", tmp.path(), "~/.agentmux/agents/blank").unwrap(), shared);
        assert_eq!(check_and_claim_now(&fs, &store, "parent", tmp.path(), "/work/repo").unwrap(), shared);
        let home = dirs::home_dir().unwrap();
        agent(&store, "homey", &home.to_string_lossy());
        assert_eq!(check_and_claim_now(&fs, &store, "homey", tmp.path(), &home.to_string_lossy()).unwrap(), shared);
    }

    /// Two agents claiming one directory — neither resolvable to it by the
    /// other's rows, e.g. a race across channels — both see it shared.
    #[test]
    fn a_second_live_claim_makes_it_shared_for_both() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        agent(&store, "agent-a", "/work/a");
        agent(&store, "agent-b", "/work/b");
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check_and_claim_now(&fs, &store, "agent-a", dir.path(), "/work/a").unwrap(), Exclusivity::Exclusive);
        assert_eq!(
            check_and_claim_now(&fs, &store, "agent-b", dir.path(), "/work/b").unwrap(),
            Exclusivity::Shared(SharedReason::Claimed { other_uid: "agent-a".into() })
        );
        assert_eq!(
            check_and_claim_now(&fs, &store, "agent-a", dir.path(), "/work/a").unwrap(),
            Exclusivity::Shared(SharedReason::Claimed { other_uid: "agent-b".into() })
        );
    }

    /// A claim by an agent that no longer exists anywhere is released, so a
    /// re-created agent reusing the directory isn't stranded.
    #[test]
    fn a_deleted_agents_claim_is_released() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let shared = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", shared.path());
        let fs = FileStore::open_in_memory().unwrap();
        let store = Store::open_in_memory().unwrap();
        agent(&store, "old", "/work/old");
        agent(&store, "new", "/work/new");
        let dir = tempfile::tempdir().unwrap();
        check_and_claim_now(&fs, &store, "old", dir.path(), "/work/old").unwrap();
        store.agent_def_delete("old").unwrap();
        let got = check_and_claim_now(&fs, &store, "new", dir.path(), "/work/new").unwrap();
        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
        assert_eq!(got, Exclusivity::Exclusive);
    }

    /// A claim from another channel's store is never released here, though
    /// this store has no row for its agent: only its own store can tell.
    /// And a shared verdict still records the claim.
    #[test]
    fn another_channels_claim_is_kept_and_a_shared_spawn_still_claims() {
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
        let shared = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", shared.path());
        let fs = FileStore::open_in_memory().unwrap();
        let (ch1, ch2) = (Store::open_in_memory().unwrap(), Store::open_in_memory().unwrap());
        agent(&ch1, "agent-a", "/work/a");
        agent(&ch2, "agent-b", "");
        let dir = tempfile::tempdir().unwrap();
        let b = check_and_claim_now(&fs, &ch2, "agent-b", dir.path(), "~/.agentmux/agents/agent-b").unwrap();
        let a = check_and_claim_now(&fs, &ch1, "agent-a", dir.path(), "/work/a").unwrap();
        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
        assert_eq!(b, Exclusivity::Shared(SharedReason::KnownSharedLocation));
        assert_eq!(a, Exclusivity::Shared(SharedReason::Claimed { other_uid: "agent-b".into() }));
    }

    #[test]
    fn dir_id_resolves_symlinks_even_for_a_folder_not_created_yet() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("shared-projects");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("channel-projects");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(unix)]
        assert_eq!(dir_id(&link.join("x").join("memory")), dir_id(&real.join("x").join("memory")));
    }
}
