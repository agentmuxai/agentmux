// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Merge a meta update into a stored object, in one transaction.
//!
//! Moved here from `server/service/object_helpers.rs` (task #1350, architecture
//! cleanup stage 2): the blockcontroller and session code in `backend/` calls
//! it, and it uses nothing above the store. `server::service` re-exports it.

use crate::backend::obj::*;
use crate::backend::storage::store::Store;

/// Update object meta by oref string. Merges meta into existing object.
///
/// The whole read-merge-write round trip runs inside ONE `Store::with_tx`
/// critical section (a single acquisition of the store's connection Mutex,
/// wrapping a real SQLite transaction) instead of two separate
/// `must_get`/`update` calls. This closes a lost-update race: two
/// call sites can legitimately fire on the SAME block within milliseconds
/// of each other — e.g. `persist_session_id` (clearing a stale `--resume`
/// id) and `clear_active_pid` both run from independent async tasks on a
/// persistent controller's process exit. Each used to read a pre-write
/// snapshot of the block's *entire* meta map, merge in its own single key,
/// and write the *entire* merged map back — so whichever of the two won
/// the race to write LAST silently reverted the other's change (every key
/// it didn't itself touch reverts to whatever it read, which may already
/// be stale). `with_tx` holds the one connection lock across the full
/// read+merge+write, so no other `Store::*` call for ANY object can
/// interleave inside it — the two writers now strictly serialize instead
/// of racing, and each one's merge is always built from the other's
/// already-committed result. See
/// REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md.
pub fn update_object_meta(
    store: &Store,
    oref_str: &str,
    meta_update: &MetaMapType,
) -> Result<(), String> {
    let oref = crate::backend::ORef::parse(oref_str).map_err(|e| e.to_string())?;

    // Validate the otype up front — StoreTx's methods return StoreError, so
    // the "unsupported otype" case (a plain String) has to be rejected
    // before entering with_tx rather than from inside its closure.
    match oref.otype.as_str() {
        OTYPE_CLIENT | OTYPE_WINDOW | OTYPE_WORKSPACE | OTYPE_TAB | OTYPE_BLOCK => {}
        _ => return Err(format!("cannot update meta for otype: {}", oref.otype)),
    }

    store
        .with_tx(|tx| {
            match oref.otype.as_str() {
                OTYPE_CLIENT => {
                    let mut obj = tx.must_get::<Client>(&oref.oid)?;
                    obj.meta = merge_meta(&obj.meta, meta_update, true);
                    tx.update(&mut obj)?;
                }
                OTYPE_WINDOW => {
                    let mut obj = tx.must_get::<Window>(&oref.oid)?;
                    obj.meta = merge_meta(&obj.meta, meta_update, true);
                    tx.update(&mut obj)?;
                }
                OTYPE_WORKSPACE => {
                    let mut obj = tx.must_get::<Workspace>(&oref.oid)?;
                    obj.meta = merge_meta(&obj.meta, meta_update, true);
                    tx.update(&mut obj)?;
                }
                OTYPE_TAB => {
                    let mut obj = tx.must_get::<Tab>(&oref.oid)?;
                    obj.meta = merge_meta(&obj.meta, meta_update, true);
                    tx.update(&mut obj)?;
                }
                OTYPE_BLOCK => {
                    let mut obj = tx.must_get::<Block>(&oref.oid)?;
                    obj.meta = merge_meta(&obj.meta, meta_update, true);
                    tx.update(&mut obj)?;
                }
                // Unreachable — validated above, before with_tx was entered.
                _ => unreachable!("otype validated before with_tx"),
            }
            Ok(())
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn insert_block(store: &Store, block_id: &str) {
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: MetaMapType::new(),
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
    }

    /// Pins the fix for a real, reproduced lost-update race: two concurrent
    /// `update_object_meta` calls on the SAME block, each merging in a
    /// DIFFERENT single key, must both survive — neither may silently
    /// revert the other's key by writing back a merged snapshot it read
    /// before the other's write landed. This is exactly the shape of the
    /// `persist_session_id` / `clear_active_pid` race
    /// (REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md): both
    /// fire from independent async tasks on a persistent controller's
    /// process exit, each touching one key of the same block's meta.
    ///
    /// A `Barrier` forces both threads to call `update_object_meta` as
    /// close to simultaneously as real concurrent tasks would, maximizing
    /// the odds this test actually exercises the race window rather than
    /// happening to run the two calls back-to-back. Before the `with_tx`
    /// fix (two separate `must_get`+`update` lock acquisitions instead of
    /// one spanning both), this test was flaky — failing whenever the two
    /// threads' read-then-write windows overlapped.
    #[test]
    fn update_object_meta_concurrent_writers_both_keys_survive() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        // ORef::parse requires a UUID-shaped oid (see oref.rs) — a plain
        // slug like "block-race" fails to parse, not the race itself.
        let block_id = uuid::Uuid::new_v4().to_string();
        insert_block(&store, &block_id);
        let oref_str = format!("block:{block_id}");

        let barrier = Arc::new(Barrier::new(2));

        let store_a = Arc::clone(&store);
        let oref_a = oref_str.clone();
        let barrier_a = Arc::clone(&barrier);
        let handle_a = thread::spawn(move || {
            let mut meta = MetaMapType::new();
            meta.insert("agent:sessionid".to_string(), serde_json::Value::String(String::new()));
            barrier_a.wait();
            update_object_meta(&store_a, &oref_a, &meta).unwrap();
        });

        let store_b = Arc::clone(&store);
        let oref_b = oref_str.clone();
        let barrier_b = Arc::clone(&barrier);
        let handle_b = thread::spawn(move || {
            let mut meta = MetaMapType::new();
            meta.insert("session:active_pid".to_string(), serde_json::Value::from(4242));
            barrier_b.wait();
            update_object_meta(&store_b, &oref_b, &meta).unwrap();
        });

        handle_a.join().unwrap();
        handle_b.join().unwrap();

        let final_block = store.must_get::<Block>(&block_id).unwrap();
        // Both threads' writes must have stuck — a lost-update race would
        // have whichever writer finished LAST silently revert the other's
        // key back to absent, since it wrote back a meta snapshot it read
        // before the other's write landed.
        assert_eq!(
            final_block.meta.get("agent:sessionid").and_then(|v| v.as_str()),
            Some(""),
            "agent:sessionid should be the empty string thread A wrote, not missing/reverted",
        );
        assert_eq!(
            final_block.meta.get("session:active_pid").and_then(|v| v.as_i64()),
            Some(4242),
            "session:active_pid should be the value thread B wrote, not missing/reverted",
        );
        // Version must reflect exactly two applied updates (1 insert + 2
        // updates = version 3) — a lost update would also show as a
        // version that skipped one of the two writers' increments in a
        // way that leaves one key's write invisible (already covered by
        // the assert above); this is a secondary, cheap corroboration.
        assert_eq!(final_block.version, 3, "both concurrent updates must have applied, not one clobbering the other");
    }
}
