// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WaveObjUpdate broadcast bridge.
//!
//! Subscribes to `srv_events_tx` (the internal sidecar event bus that the
//! reducer publishes mutations to) and translates each event into one or
//! more `WaveObjUpdate` records, broadcast to all connected WS clients via
//! the existing `event_bus.broadcast_event(...)` plumbing — the same path
//! that `service.rs:39-52`'s response-broadcast loop uses.
//!
//! Why this exists: per-RPC handlers were responsible for attaching
//! `WaveObjUpdate`s to their responses (`success_with_updates(...)`).
//! Forgetting that call left the frontend WOS cache stale (e.g. workspace
//! renames not propagating to the OS title or the InstancePanel — see
//! `docs/specs/SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14.md`).
//!
//! With this bridge in place, any reducer event automatically reaches the
//! frontend, so the per-handler convention becomes belt-and-suspenders
//! instead of load-bearing.
//!
//! Spec: `docs/specs/SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`.
//!
//! Phase 1 scope (this implementation): workspace events only —
//! immediately fixes the user-reported bug. Phase 2 expands to tabs /
//! blocks / windows / layouts; Phase 3 retires the per-handler
//! `success_with_updates(...)` calls now that the bridge covers them.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use tokio::sync::broadcast;

use crate::backend::eventbus::{EventBus, WSEventType};
use crate::backend::obj::{
    wave_obj_to_value, Client, Tab, StoreObj, OTYPE_BLOCK, OTYPE_CLIENT, OTYPE_LAYOUT, OTYPE_TAB,
    OTYPE_WINDOW, OTYPE_WORKSPACE,
};
use crate::backend::storage::store::Store;

/// JSON shape that gets broadcast as the `data` payload of a
/// `waveobj:update` WS event. Matches the shape of `WaveObjUpdate` in
/// `agentmux-srv/src/backend/obj.rs:465-474` so the frontend's existing
/// `updateWaveObject` handler accepts it without changes.
fn build_update_payload(
    updatetype: &str,
    otype: &str,
    oid: &str,
    obj: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(4);
    map.insert("updatetype".into(), serde_json::Value::String(updatetype.into()));
    map.insert("otype".into(), serde_json::Value::String(otype.into()));
    map.insert("oid".into(), serde_json::Value::String(oid.into()));
    if let Some(o) = obj {
        map.insert("obj".into(), o);
    }
    serde_json::Value::Object(map)
}

/// Push one `WaveObjUpdate` payload to all connected WS clients via the
/// shared event_bus. Mirrors the response-broadcast loop in
/// `service.rs:39-52`.
fn emit(event_bus: &EventBus, otype: &str, oid: &str, payload: serde_json::Value) {
    let oref = format!("{otype}:{oid}");
    event_bus.broadcast_event(&WSEventType {
        eventtype: "waveobj:update".to_string(),
        oref,
        data: Some(payload),
    });
}

/// Fetch one StoreObj by oid and broadcast it as a `waveobj:update`. The
/// SQLite read is offloaded to the blocking thread pool per ReAgent P1
/// on PR #852 (Store is `std::sync::Mutex<Connection>`; brief in
/// steady state but a long reducer transaction would block the tokio
/// worker thread). Silently logs + skips on missing/error to satisfy
/// the §8.15 idempotency contract — duplicate or stale events fold to
/// no-op.
async fn emit_fetched<T: StoreObj + Send + 'static>(
    wstore: &Arc<Store>,
    event_bus: &Arc<EventBus>,
    otype: &'static str,
    oid: String,
    context: &'static str,
) {
    let id = oid.clone();
    let store = Arc::clone(wstore);
    let result = tokio::task::spawn_blocking(move || store.get::<T>(&id)).await;
    match result {
        Ok(Ok(Some(obj))) => {
            let payload = build_update_payload(
                "update",
                otype,
                &oid,
                Some(wave_obj_to_value(&obj)),
            );
            emit(event_bus, otype, &oid, payload);
        }
        Ok(Ok(None)) => {
            tracing::warn!(
                target: "wave-obj-bridge",
                oid = %oid, otype = otype, ctx = context,
                "object not found in wstore; skipping broadcast"
            );
        }
        Ok(Err(e)) => {
            tracing::error!(
                target: "wave-obj-bridge",
                oid = %oid, otype = otype, ctx = context, error = %e,
                "wstore.get failed; skipping broadcast"
            );
        }
        Err(join_err) => {
            tracing::error!(
                target: "wave-obj-bridge",
                oid = %oid, otype = otype, ctx = context, error = %join_err,
                "spawn_blocking join failed (likely panicked); skipping broadcast"
            );
        }
    }
}

/// Broadcast a "delete" `waveobj:update` for the given oid. No fetch
/// needed — the frontend's `updateWaveObject` (`wos.ts:263-265`) handles
/// the delete arm with just the oid.
fn emit_delete(event_bus: &EventBus, otype: &'static str, oid: &str) {
    let payload = build_update_payload("delete", otype, oid, None);
    emit(event_bus, otype, oid, payload);
}

/// Broadcast the singleton `Client` StoreObj. SrvWindowOpened /
/// SrvWindowClosed mutate `Client.windowids` (per
/// `apply_srv_window_opened` in persist_subscriber.rs:518) so renderers
/// holding a pinned Client need to see the new windowids list — without
/// this broadcast they'd render stale window membership until reload.
/// Codex P2 on PR #861.
///
/// Client is a singleton — the first `get_all::<Client>()` row is THE
/// client. Same lookup pattern persist_subscriber uses.
async fn emit_client_singleton(
    wstore: &Arc<Store>,
    event_bus: &Arc<EventBus>,
    context: &'static str,
) {
    let store = Arc::clone(wstore);
    let result = tokio::task::spawn_blocking(move || store.get_all::<Client>()).await;
    match result {
        Ok(Ok(clients)) => {
            if let Some(client) = clients.into_iter().next() {
                let oid = client.oid.clone();
                let payload = build_update_payload(
                    "update",
                    OTYPE_CLIENT,
                    &oid,
                    Some(wave_obj_to_value(&client)),
                );
                emit(event_bus, OTYPE_CLIENT, &oid, payload);
            } else {
                tracing::warn!(
                    target: "wave-obj-bridge",
                    ctx = context,
                    "no Client row in wstore; skipping Client broadcast"
                );
            }
        }
        Ok(Err(e)) => {
            tracing::error!(
                target: "wave-obj-bridge",
                ctx = context, error = %e,
                "wstore.get_all::<Client> failed; skipping broadcast"
            );
        }
        Err(je) => {
            tracing::error!(
                target: "wave-obj-bridge",
                ctx = context, error = %je,
                "spawn_blocking join failed during Client lookup"
            );
        }
    }
}

/// Layout events all reference a `tab_id`; the affected StoreObj is the
/// `LayoutState` referenced by the tab's `layoutstate` field. Two
/// SQLite reads chained inside one `spawn_blocking` to keep the lock
/// hold short.
async fn emit_layout_for_tab(
    wstore: &Arc<Store>,
    event_bus: &Arc<EventBus>,
    tab_id: String,
    context: &'static str,
) {
    use crate::backend::obj::LayoutState;
    let id_for_log = tab_id.clone();
    let store = Arc::clone(wstore);
    let result = tokio::task::spawn_blocking(move || -> Result<Option<LayoutState>, _> {
        match store.get::<Tab>(&tab_id) {
            Ok(Some(tab)) => {
                if tab.layoutstate.is_empty() {
                    Ok(None)
                } else {
                    store.get::<LayoutState>(&tab.layoutstate)
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .await;
    match result {
        Ok(Ok(Some(layout))) => {
            let layout_id = layout.oid.clone();
            let payload = build_update_payload(
                "update",
                OTYPE_LAYOUT,
                &layout_id,
                Some(wave_obj_to_value(&layout)),
            );
            emit(event_bus, OTYPE_LAYOUT, &layout_id, payload);
        }
        Ok(Ok(None)) => {
            tracing::warn!(
                target: "wave-obj-bridge",
                tab_id = %id_for_log, ctx = context,
                "layout event but tab/layoutstate not found; skipping broadcast"
            );
        }
        Ok(Err(e)) => {
            tracing::error!(
                target: "wave-obj-bridge",
                tab_id = %id_for_log, ctx = context, error = %e,
                "wstore.get failed during layout resolution; skipping broadcast"
            );
        }
        Err(join_err) => {
            tracing::error!(
                target: "wave-obj-bridge",
                tab_id = %id_for_log, ctx = context, error = %join_err,
                "spawn_blocking join failed during layout resolution"
            );
        }
    }
}

/// Translate one reducer event into zero or more `waveobj:update` broadcasts.
///
/// **Read source — post-event state guarantee:**
/// For events emitted via the HTTP `service.rs` RPC handlers,
/// `apply_event_to_wstore` is called synchronously (`service.rs:1297-1304`
/// for workspace; equivalent path for tab/block/window/layout commands)
/// before `publish_events` (`service.rs:1305`). So when the bridge
/// receives such an event, SQLite is already up-to-date.
///
/// **IPC-path caveat:** the launcher → IPC path in `srv_ipc/server.rs:295`
/// dispatches reducer events directly without first calling
/// `apply_event_to_wstore`; the persist subscriber and bridge then race.
/// At time of writing none of the events the bridge handles are emitted
/// via that path (verified for `Command::UpdateWindowMeta` and the
/// workspace family). When that changes, options are: (a) make the IPC
/// path apply synchronously like HTTP does, or (b) read from the
/// in-memory `srv_state` reducer rather than SQLite. Tracked in
/// `SPEC_OBJ_UPDATE_BRIDGE §11.1`.
///
/// **Lock discipline (per ReAgent P1 on PR #852):** every `wstore.get<T>()`
/// is wrapped in `tokio::task::spawn_blocking` via the helpers above so
/// the async runtime stays responsive even under reducer-transaction
/// contention.
///
/// **Coverage:** Phase 1 + 2 covers workspace, window, tab, block, layout
/// events. Saga events, OS facts, launcher-domain events all fall through
/// to the catch-all `_ => {}` arm.
async fn dispatch_event(event: Event, wstore: Arc<Store>, event_bus: Arc<EventBus>) {
    use crate::backend::obj::{Block, Window, Workspace};

    match event {
        // ----- Workspace -----
        Event::WorkspaceRenamed { workspace_id, .. }
        | Event::WorkspaceMetaUpdated { workspace_id, .. }
        | Event::WorkspaceCreated { workspace_id, .. } => {
            emit_fetched::<Workspace>(
                &wstore, &event_bus, OTYPE_WORKSPACE, workspace_id, "Workspace*",
            )
            .await;
        }
        Event::WorkspaceDeleted { workspace_id, .. } => {
            emit_delete(&event_bus, OTYPE_WORKSPACE, &workspace_id);
        }

        // ----- Window (Phase 2 + #855) -----
        Event::WindowMetaUpdated { window_id, .. } => {
            emit_fetched::<Window>(
                &wstore, &event_bus, OTYPE_WINDOW, window_id, "WindowMetaUpdated",
            )
            .await;
        }
        Event::SrvWindowWorkspaceChanged { window_id, .. } => {
            emit_fetched::<Window>(
                &wstore, &event_bus, OTYPE_WINDOW, window_id, "SrvWindowWorkspaceChanged",
            )
            .await;
        }
        // SrvWindowOpened/Closed: the persist path
        // (apply_srv_window_opened / apply_srv_window_closed) ALSO
        // mutates Client.windowids inside the same transaction, so
        // renderers with a pinned Client need to see the updated
        // singleton too — otherwise their window list lags behind
        // until reload. (Codex P2 on PR #861.)
        Event::SrvWindowOpened { window_id, .. } => {
            emit_fetched::<Window>(
                &wstore, &event_bus, OTYPE_WINDOW, window_id, "SrvWindowOpened",
            )
            .await;
            emit_client_singleton(&wstore, &event_bus, "SrvWindowOpened").await;
        }
        // Parent (Client singleton) update first, window delete second —
        // same delete-ordering rationale as the TabDeleted arm below.
        Event::SrvWindowClosed { window_id, .. } => {
            emit_client_singleton(&wstore, &event_bus, "SrvWindowClosed").await;
            emit_delete(&event_bus, OTYPE_WINDOW, &window_id);
        }

        // ----- Tab (Phase 2) -----
        // TabCreated also touches the parent workspace's tab_ids field
        // (reducer mutates both in one dispatch). Broadcast both so the
        // frontend WOS sees the new Tab AND the updated parent ordering.
        Event::TabCreated {
            workspace_id,
            tab_id,
            ..
        } => {
            emit_fetched::<Tab>(&wstore, &event_bus, OTYPE_TAB, tab_id, "TabCreated").await;
            emit_fetched::<Workspace>(
                &wstore, &event_bus, OTYPE_WORKSPACE, workspace_id, "TabCreated parent",
            )
            .await;
        }
        // Delete ordering: PARENT UPDATE FIRST, child delete second — the
        // reverse of the create arms above. These are two separate WS
        // frames the renderer applies (and paints) independently; if the
        // tab delete lands first, the still-mounted <Tab>'s own signal
        // goes null and it blanks in place for a paint before the
        // workspace update finally unmounts it (the §6 flash of
        // SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md, riding in
        // through the WS push path — §7). Parent-first, the component
        // unmounts with its data still intact and the late child delete
        // touches an unsubscribed signal — nothing paints.
        Event::TabDeleted {
            workspace_id,
            tab_id,
            ..
        } => {
            emit_fetched::<Workspace>(
                &wstore, &event_bus, OTYPE_WORKSPACE, workspace_id, "TabDeleted parent",
            )
            .await;
            emit_delete(&event_bus, OTYPE_TAB, &tab_id);
        }
        Event::TabRenamed { tab_id, .. } | Event::TabMetaUpdated { tab_id, .. } => {
            emit_fetched::<Tab>(&wstore, &event_bus, OTYPE_TAB, tab_id, "Tab*").await;
        }
        Event::ActiveTabChanged { workspace_id, .. }
        | Event::TabReordered { workspace_id, .. }
        | Event::TabsReorderedBulk { workspace_id, .. } => {
            emit_fetched::<Workspace>(
                &wstore,
                &event_bus,
                OTYPE_WORKSPACE,
                workspace_id,
                "ActiveTab/Reorder",
            )
            .await;
        }
        Event::TabMoved {
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            ..
        } => {
            emit_fetched::<Tab>(&wstore, &event_bus, OTYPE_TAB, tab_id, "TabMoved").await;
            emit_fetched::<Workspace>(
                &wstore,
                &event_bus,
                OTYPE_WORKSPACE,
                src_workspace_id,
                "TabMoved src",
            )
            .await;
            emit_fetched::<Workspace>(
                &wstore,
                &event_bus,
                OTYPE_WORKSPACE,
                dst_workspace_id,
                "TabMoved dst",
            )
            .await;
        }

        // ----- Block (Phase 2) -----
        // BlockCreated/BlockDeleted touch the parent tab's blockids field.
        Event::BlockCreated {
            tab_id, block_id, ..
        } => {
            emit_fetched::<Block>(
                &wstore, &event_bus, OTYPE_BLOCK, block_id, "BlockCreated",
            )
            .await;
            emit_fetched::<Tab>(
                &wstore, &event_bus, OTYPE_TAB, tab_id, "BlockCreated parent",
            )
            .await;
        }
        // Parent update first, block delete second — same delete-ordering
        // rationale as the TabDeleted arm above.
        Event::BlockDeleted {
            tab_id, block_id, ..
        } => {
            emit_fetched::<Tab>(
                &wstore, &event_bus, OTYPE_TAB, tab_id, "BlockDeleted parent",
            )
            .await;
            emit_delete(&event_bus, OTYPE_BLOCK, &block_id);
        }
        Event::BlockMetaUpdated { block_id, .. } => {
            emit_fetched::<Block>(
                &wstore,
                &event_bus,
                OTYPE_BLOCK,
                block_id,
                "BlockMetaUpdated",
            )
            .await;
        }
        Event::BlockMoved {
            block_id,
            src_tab_id,
            dst_tab_id,
            ..
        } => {
            emit_fetched::<Block>(&wstore, &event_bus, OTYPE_BLOCK, block_id, "BlockMoved").await;
            emit_fetched::<Tab>(&wstore, &event_bus, OTYPE_TAB, src_tab_id, "BlockMoved src")
                .await;
            emit_fetched::<Tab>(&wstore, &event_bus, OTYPE_TAB, dst_tab_id, "BlockMoved dst")
                .await;
        }

        // ----- Layout (Phase 2 — partial) -----
        // Focused/Magnified + Cleared/TreeReplaced are now persisted by
        // `apply_event_to_wstore` (persist_subscriber.rs), so the bridge
        // can safely re-read LayoutState and broadcast it: the HTTP RPC
        // path applies SQLite synchronously before publishing the event
        // (see the post-event-state guarantee above), so the read sees
        // post-event tree state.
        //
        // The remaining tree events ARE persisted by the subscriber now
        // (SPEC_864 phases 2-5, #1970-#1981 — wcore-direct layout writes
        // are retired; this comment predated that migration). They are
        // still deliberately NOT bridged, but for a different reason than
        // the original wcore-direct race: see the srv-IPC-path caveat on
        // the granular-events note below. They remain covered by their
        // handlers' existing `success_with_updates(...)` response
        // broadcasts (Codex P2 on PR #861).
        Event::FocusedNodeChanged { tab_id, .. }
        | Event::MagnifiedNodeChanged { tab_id, .. } => {
            emit_layout_for_tab(&wstore, &event_bus, tab_id, "Focused/MagnifiedNodeChanged").await;
        }
        Event::LayoutCleared { tab_id, .. } | Event::LayoutTreeReplaced { tab_id, .. } => {
            emit_layout_for_tab(&wstore, &event_bus, tab_id, "LayoutCleared/TreeReplaced").await;
        }
        // The 7 granular structural events (LayoutNodeMoved/…/InsertedAtIndex)
        // are persisted by the subscriber (they carry `new_tree`), but are
        // deliberately NOT bridged here. `emit_layout_for_tab` re-reads
        // LayoutState, which is only guaranteed post-event on the HTTP-RPC
        // path (synchronous apply before publish). On the srv-IPC path,
        // `srv_ipc/server.rs` publishes to `events_tx` without a synchronous
        // apply, so a bridge broadcast could race ahead of the persist
        // subscriber and emit pre-event state (codex P2 on #1883 — the IPC-path
        // caveat documented above). These events have no production dispatcher
        // yet; their bridge projection lands in the Phase-4 frontend
        // intent-flip, where the path is HTTP-RPC (synchronous, race-free).

        // Saga lifecycle, launcher-domain events, OS facts, etc. — not
        // StoreObj changes. The catch-all keeps the bridge future-proof
        // for new event variants the reducer may add.
        _ => {}
    }
}

/// Spawn the bridge task. Returns the `JoinHandle` so callers can keep it
/// alive (typically forever — the task lives for the lifetime of the srv
/// process). Per ReAgent P1 on PR #852: the loop is panic-resilient — a
/// panic inside `dispatch_event` is caught and logged, and the loop
/// continues processing subsequent events. Without this, a single
/// malformed event could silently kill the entire bridge task and
/// frontend WOS would stop seeing updates.
///
/// Subscribe ordering: per `SPEC §11.1` the bridge can subscribe in any
/// order relative to the persist subscriber. For Phase 1's workspace
/// events the HTTP RPC handler applies SQLite synchronously before
/// publishing the event, so the bridge always sees post-event state.
pub fn spawn_wave_obj_bridge(
    events_rx: broadcast::Receiver<Event>,
    wstore: Arc<Store>,
    event_bus: Arc<EventBus>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_wave_obj_bridge(events_rx, wstore, event_bus))
}

async fn run_wave_obj_bridge(
    mut events_rx: broadcast::Receiver<Event>,
    wstore: Arc<Store>,
    event_bus: Arc<EventBus>,
) {
    tracing::info!(target: "wave-obj-bridge", "[wave-obj-bridge] started (Phase 1: workspace events)");
    loop {
        match events_rx.recv().await {
            Ok(event) => {
                // Per-event panic isolation (ReAgent P1 on PR #852): use
                // FuturesUnordered with a catch_unwind future would be the
                // textbook fix, but for a single event-at-a-time loop the
                // simpler pattern is to spawn the dispatch as its own task
                // and observe the JoinError if it panics. We `await` it
                // immediately so events still process serially (matching
                // the broadcast channel's send order), but a panic in one
                // event can't kill the bridge.
                let store = Arc::clone(&wstore);
                let bus = Arc::clone(&event_bus);
                let event_dbg = format!("{:?}", &event);
                let join = tokio::spawn(dispatch_event(event, store, bus)).await;
                if let Err(join_err) = join {
                    if join_err.is_panic() {
                        tracing::error!(
                            target: "wave-obj-bridge",
                            event = %event_dbg,
                            "dispatch_event panicked; bridge continues with next event. Panic: {}",
                            join_err,
                        );
                    } else {
                        tracing::error!(
                            target: "wave-obj-bridge",
                            event = %event_dbg,
                            error = %join_err,
                            "dispatch_event task aborted unexpectedly"
                        );
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // The broadcast channel has 1024 capacity (main.rs:624).
                // If we lag, frontend WOS state diverges silently — log it
                // loudly so operators can correlate with user-visible drift
                // (e.g. the InstancePanel/title showing stale names).
                // No automatic recovery; the next event resyncs the affected
                // object and frontend reads everything else from its cache.
                tracing::error!(
                    target: "wave-obj-bridge",
                    skipped = n,
                    "broadcast channel lagged; some waveobj:update events were dropped — frontend WOS may show stale state until the affected object is mutated again"
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!(target: "wave-obj-bridge", "events channel closed; bridge exiting");
                return;
            }
        }
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{MetaMapType, Workspace};

    fn test_workspace(oid: &str, tabids: Vec<String>) -> Workspace {
        Workspace {
            oid: oid.to_string(),
            version: 1,
            name: "test-ws".to_string(),
            tabids,
            pinnedtabids: vec![],
            activetabid: String::new(),
            meta: MetaMapType::new(),
        }
    }

    fn test_tab(oid: &str) -> Tab {
        Tab {
            oid: oid.to_string(),
            version: 1,
            name: "test-tab".to_string(),
            layoutstate: String::new(),
            blockids: vec![],
            meta: MetaMapType::new(),
        }
    }

    /// SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §7: the bridge's
    /// TabDeleted arm must broadcast the PARENT workspace update BEFORE the
    /// tab delete. These are two separate WS frames the renderer applies
    /// (and paints) independently — delete-first blanks the still-mounted
    /// <Tab> in place for a paint before the workspace update unmounts it.
    #[tokio::test]
    async fn test_tab_deleted_broadcasts_parent_update_before_tab_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let wstore = Arc::new(Store::open(&tmp.path().join("objects.db")).unwrap());
        let event_bus = Arc::new(EventBus::new());

        let ws_id = "11111111-1111-1111-1111-111111111111";
        let deleted_tab_id = "22222222-2222-2222-2222-222222222222";
        let surviving_tab_id = "33333333-3333-3333-3333-333333333333";
        // Post-delete state: the workspace no longer lists the deleted tab.
        let mut ws = test_workspace(ws_id, vec![surviving_tab_id.to_string()]);
        wstore.insert(&mut ws).unwrap();

        let mut receivers = event_bus.register_ws("test-conn", "test-tab");

        dispatch_event(
            Event::TabDeleted {
                workspace_id: ws_id.to_string(),
                tab_id: deleted_tab_id.to_string(),
                block_ids: vec![],
                version: 1,
            },
            Arc::clone(&wstore),
            Arc::clone(&event_bus),
        )
        .await;

        let first = tokio::time::timeout(std::time::Duration::from_secs(2), receivers.priority.recv())
            .await
            .expect("timed out waiting for first broadcast")
            .expect("priority channel closed");
        assert_eq!(first.get("eventtype").and_then(|v| v.as_str()), Some("waveobj:update"));
        assert_eq!(
            first.get("oref").and_then(|v| v.as_str()),
            Some(format!("workspace:{ws_id}").as_str()),
            "first frame must be the parent workspace update, got: {first}"
        );
        assert_eq!(
            first.get("data").and_then(|d| d.get("updatetype")).and_then(|v| v.as_str()),
            Some("update"),
        );

        let second = tokio::time::timeout(std::time::Duration::from_secs(2), receivers.priority.recv())
            .await
            .expect("timed out waiting for second broadcast")
            .expect("priority channel closed");
        assert_eq!(
            second.get("oref").and_then(|v| v.as_str()),
            Some(format!("tab:{deleted_tab_id}").as_str()),
            "second frame must be the tab delete, got: {second}"
        );
        assert_eq!(
            second.get("data").and_then(|d| d.get("updatetype")).and_then(|v| v.as_str()),
            Some("delete"),
        );
    }

    /// Same delete-ordering contract for BlockDeleted: parent tab update
    /// first, block delete second.
    #[tokio::test]
    async fn test_block_deleted_broadcasts_parent_update_before_block_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let wstore = Arc::new(Store::open(&tmp.path().join("objects.db")).unwrap());
        let event_bus = Arc::new(EventBus::new());

        let tab_id = "44444444-4444-4444-4444-444444444444";
        let deleted_block_id = "55555555-5555-5555-5555-555555555555";
        let mut tab = test_tab(tab_id);
        wstore.insert(&mut tab).unwrap();

        let mut receivers = event_bus.register_ws("test-conn", "test-tab");

        dispatch_event(
            Event::BlockDeleted {
                tab_id: tab_id.to_string(),
                block_id: deleted_block_id.to_string(),
                version: 1,
            },
            Arc::clone(&wstore),
            Arc::clone(&event_bus),
        )
        .await;

        let first = tokio::time::timeout(std::time::Duration::from_secs(2), receivers.priority.recv())
            .await
            .expect("timed out waiting for first broadcast")
            .expect("priority channel closed");
        assert_eq!(
            first.get("oref").and_then(|v| v.as_str()),
            Some(format!("tab:{tab_id}").as_str()),
            "first frame must be the parent tab update, got: {first}"
        );

        let second = tokio::time::timeout(std::time::Duration::from_secs(2), receivers.priority.recv())
            .await
            .expect("timed out waiting for second broadcast")
            .expect("priority channel closed");
        assert_eq!(
            second.get("oref").and_then(|v| v.as_str()),
            Some(format!("block:{deleted_block_id}").as_str()),
            "second frame must be the block delete, got: {second}"
        );
        assert_eq!(
            second.get("data").and_then(|d| d.get("updatetype")).and_then(|v| v.as_str()),
            Some("delete"),
        );
    }
}
