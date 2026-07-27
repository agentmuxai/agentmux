// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wave-object read/update/meta primitives used by the `object` service
//! handler, plus the per-agent zoom mirror. `update_object_meta` and
//! `schedule_agent_zoom_mirror` are re-exported crate-wide.

use crate::backend::obj::*;
use crate::backend::storage::store::Store;

/// Phase E.4 (Option A) — reverse lookup: given a `LayoutState.oid`,
/// find the `Tab.oid` that owns it (i.e., the tab whose `layoutstate`
/// field matches). Returns `None` when the layout is unowned (legacy
/// or partially-migrated row) or the wstore read fails — caller treats
/// either as "skip the reducer dispatch and fall through to the wcore
/// write." Linear scan over all tabs; acceptable here because the
/// layout-update path is low-frequency relative to drag-resize and
/// the reducer mutex itself is held for sub-millisecond intervals.
pub(super) fn find_tab_for_layout(store: &Store, layout_oid: &str) -> Option<String> {
    let tabs = store.get_all::<Tab>().ok()?;
    tabs.into_iter()
        .find(|t| t.layoutstate == layout_oid)
        .map(|t| t.oid)
}

/// Resolve an "otype:oid" string to the corresponding wave object JSON.
pub(super) fn get_object_by_oref(store: &Store, oref_str: &str) -> Result<serde_json::Value, String> {
    let oref = crate::backend::ORef::parse(oref_str).map_err(|e| e.to_string())?;

    // Validate otype is known
    match oref.otype.as_str() {
        OTYPE_CLIENT | OTYPE_WINDOW | OTYPE_WORKSPACE | OTYPE_TAB | OTYPE_LAYOUT | OTYPE_BLOCK => {}
        _ => return Err(format!("unknown otype: {}", oref.otype)),
    }

    // Use raw JSON read to avoid strict struct deserialization issues
    // (e.g. layout leaforder with embedded BlockDef objects).
    // This matches Go's generic map-based GetObject behavior.
    store
        .get_raw(&oref.otype, &oref.oid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("not found: {}", oref_str))
}

/// Update a wave object by replacing it wholesale in the store.
/// The incoming value must have `otype` and `oid` fields.
/// Matches Go's ObjectService.UpdateObject behavior.
/// Returns (otype, oid, updated_value_with_new_version) on success.
pub(super) fn update_object(
    store: &Store,
    mut value: serde_json::Value,
) -> Result<(String, String, serde_json::Value), String> {
    let otype = value
        .get("otype")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "UpdateObject: missing otype field".to_string())?
        .to_string();
    let oid = value
        .get("oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "UpdateObject: missing oid field".to_string())?
        .to_string();

    // Validate the otype is known
    match otype.as_str() {
        OTYPE_CLIENT | OTYPE_WINDOW | OTYPE_WORKSPACE | OTYPE_TAB | OTYPE_LAYOUT | OTYPE_BLOCK => {}
        _ => return Err(format!("UpdateObject: unsupported otype: {}", otype)),
    }

    // Use raw JSON storage (matching Go's generic map-based UpdateObject).
    // The frontend sends the full replacement object; strict Rust struct deserialization
    // can fail on dynamic fields (e.g. layout rootnode with embedded BlockDefs).
    let new_version = store
        .update_raw(&otype, &oid, &value)
        .map_err(|e| format!("UpdateObject: {}", e))?;

    // Update version in the value for the returned update event
    if let Some(obj) = value.as_object_mut() {
        obj.insert("version".to_string(), serde_json::json!(new_version));
    }

    Ok((otype, oid, value))
}

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
pub(crate) fn update_object_meta(
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

/// Mirror an agent block's `term:zoom` into the per-agent `ui:zoom` content
/// blob immediately (no debounce) so the zoom survives pane close even if the
/// user closes the pane right after zooming. `zoom = Some(z)` upserts;
/// `zoom = None` (term:zoom reset to null / 1.0) deletes the row so a
/// default agent stores nothing. Writes are off-thread so the WebSocket
/// handler stays non-blocking; SQLite serializes concurrent writes via the
/// store mutex. See SPEC_AGENT_ZOOM_PERSISTENCE §4.3.
pub(crate) fn schedule_agent_zoom_mirror(
    store: std::sync::Arc<crate::backend::storage::store::Store>,
    agent_id: String,
    zoom: Option<f64>,
) {
    tokio::spawn(async move {
        let now = chrono::Utc::now().timestamp_millis();
        let result = match zoom {
            Some(z) => store.agent_content_set(
                &crate::backend::storage::store::AgentContent {
                    agent_id: agent_id.clone(),
                    content_type: "ui:zoom".to_string(),
                    content: format!("{}", z),
                    updated_at: now,
                },
            ),
            None => store.agent_content_delete(&agent_id, "ui:zoom").map(|_| ()),
        };
        if let Err(e) = result {
            tracing::warn!(agent_id = %agent_id, error = %e, "[zoom] agent zoom mirror write failed");
        }
    });
}

#[cfg(test)]
mod agent_zoom_mirror_tests {
    use super::schedule_agent_zoom_mirror;
    use crate::backend::storage::store::{AgentContent, AgentDefinition, Store};
    use std::sync::Arc;

    /// `db_agent_content.agent_id` has a FK to the agent-definitions table, so a
    /// real mirror only ever fires for an existing agent (the def was loaded at
    /// agent.open). Seed a minimal def so the test mirrors that invariant.
    fn seed_agent_def(store: &Store, id: &str) {
        let mut def: AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": id,
            "name": id,
            "icon": "sparkles",
            "provider": "claude",
            "description": "",
            "created_at": 1,
        }))
        .expect("build minimal AgentDefinition");
        store.agent_def_insert(&mut def).expect("insert agent def");
    }

    /// Each `term:zoom` change is written immediately; last write wins.
    #[tokio::test]
    async fn mirror_writes_each_zoom_immediately() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let agent = "agent-zoom-burst";
        seed_agent_def(&store, agent);

        schedule_agent_zoom_mirror(store.clone(), agent.into(), Some(1.1));
        schedule_agent_zoom_mirror(store.clone(), agent.into(), Some(1.4));

        // Give the spawned tasks a moment to complete.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let saved = store
            .agent_content_get(agent, "ui:zoom")
            .unwrap()
            .expect("zoom persisted immediately");
        // Last write wins.
        assert_eq!(saved.content, "1.4", "last zoom value persists");
    }

    /// `term:zoom` reset to null/1.0 → `None` → delete the saved row so a
    /// default agent stores nothing.
    #[tokio::test]
    async fn reset_to_default_deletes_saved_zoom() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let agent = "agent-zoom-reset";
        seed_agent_def(&store, agent);
        store
            .agent_content_set(&AgentContent {
                agent_id: agent.into(),
                content_type: "ui:zoom".into(),
                content: "1.5".into(),
                updated_at: 1,
            })
            .unwrap();

        schedule_agent_zoom_mirror(store.clone(), agent.into(), None);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        assert!(
            store.agent_content_get(agent, "ui:zoom").unwrap().is_none(),
            "reset-to-default removes the persisted zoom"
        );
    }
}
