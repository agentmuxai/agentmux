// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The last-session snapshot kept in the client row's meta (`session:last_topology`): saved when a window closes
//! or the OS shuts down, read back by the restore (`server/service/session_restore.rs`, which re-exports these
//! under its old paths). Only the store is involved, so the upgrade's carry-forward test reaches it from the core.

use serde_json::Value;

use crate::backend::obj::Client;
use crate::backend::storage::store::Store;

pub(crate) const SNAPSHOT_META_KEY: &str = "session:last_topology";

/// Persist a snapshot as the durable "last session" record, overwriting
/// whatever was there before. Best-effort: a write failure is logged, never
/// propagated — this must never block the close it rides alongside.
///
/// Reads-then-writes the singleton `Client` row inside a single
/// `with_tx` (reagentx P1 on PR #2560): the two used to be separate
/// `store.get_all`/`store.update` calls, unlike `persist_subscriber`'s
/// window handlers which already wrap this exact shape in a transaction.
/// A concurrent write to `Client` between the read and the write here
/// (e.g. another in-flight `CloseWindow` pruning `windowids`, or a second
/// concurrent snapshot save/clear) would silently lose whichever change
/// didn't win the race — a classic lost-update. `with_tx` holds the
/// store's connection lock for the whole read+write, so no other caller
/// can observe or write an intermediate state.
/// Returns whether the write actually landed. Callers on the clean-close path
/// (`window_close.rs`) intentionally discard this — that path is documented
/// best-effort, and must never let a snapshot failure block the close it is
/// riding along with. The OS-shutdown path (`handle_save_session_snapshot`
/// below) is different: it has nothing else to do after this call, so it
/// uses the result to report whether the flush actually succeeded rather
/// than reporting success unconditionally.
pub(crate) fn save_last_session_snapshot(store: &Store, snapshot: Value) -> bool {
    let result = store.with_tx(|tx| {
        let clients = tx.get_all::<Client>()?;
        let Some(mut client) = clients.into_iter().next() else {
            // No client row to attach the snapshot to — nothing was written.
            // Not a store error, but still not a successful save: surface it
            // as one via the Ok(false) below rather than Ok(()) reading as
            // unconditional success regardless of what actually happened.
            return Ok(false);
        };
        client.meta.insert(SNAPSHOT_META_KEY.to_string(), snapshot);
        tx.update(&mut client)?;
        Ok(true)
    });
    match result {
        Ok(saved) => saved,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "session_restore: failed to persist last-session snapshot"
            );
            false
        }
    }
}

pub(crate) fn load_last_session_snapshot(store: &Store) -> Option<Value> {
    let clients = store.get_all::<Client>().ok()?;
    let client = clients.into_iter().next()?;
    client.meta.get(SNAPSHOT_META_KEY).cloned()
}
