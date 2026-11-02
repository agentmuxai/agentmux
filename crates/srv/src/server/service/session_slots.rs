// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The window slots of the open workspace: the order its windows are listed in, open and closed alike, in
//! the version pill's instance panel. Every window has a slot for good: closing one keeps it, reopening
//! it returns to it, and a window that is opened for the first time takes the next one at the end. Only a
//! window that is deleted (an open one removed from the workspace, a closed one removed from the list)
//! gives its slot up, and the ones after it move up.
//!
//! The order is the client meta `workspace:window_slots`, the workspace ids in slot order. It is saved
//! with the workspace's record (`window_slots`), so it survives a restart, an upgrade (the new version
//! copies the record) and a switch of workspace (the record of the one opened brings its own). The panel
//! asks [`handle_place_window_slots`] with the windows it shows; srv appends the ones it has no slot for
//! and answers with the whole order, which every window's panel then follows.

use serde_json::{json, Value};

use crate::backend::obj::Client;
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::server::AppState;

/// The client meta key holding the order.
pub(crate) const SLOTS_META_KEY: &str = "workspace:window_slots";
/// Its key in a workspace's record.
const RECORD_KEY: &str = "window_slots";
/// Far more windows than a workspace holds; only here so a long-lived workspace can't grow the list
/// without end. The oldest slots go first.
const MAX_SLOTS: usize = 200;

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().filter(|s| !s.is_empty()).map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The slots, in order.
pub(crate) fn slots(store: &Store) -> Vec<String> {
    store
        .get_all::<Client>()
        .ok()
        .and_then(|cs| cs.into_iter().next())
        .map(|c| strings(c.meta.get(SLOTS_META_KEY)))
        .unwrap_or_default()
}

/// Change the slots in one transaction (a place and a forget can run together) and return them. A change
/// counts as one to the workspace's windows, so the saver saves it.
fn update(store: &Store, f: impl FnOnce(&mut Vec<String>)) -> Vec<String> {
    let result = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else {
            return Ok((Vec::new(), false));
        };
        let before = strings(client.meta.get(SLOTS_META_KEY));
        let mut list = before.clone();
        f(&mut list);
        let changed = list != before;
        if changed {
            if list.is_empty() {
                client.meta.remove(SLOTS_META_KEY);
            } else {
                client.meta.insert(SLOTS_META_KEY.to_string(), json!(list));
            }
            tx.update(&mut client)?;
        }
        Ok((list, changed))
    });
    match result {
        Ok((list, changed)) => {
            if changed {
                store.layout_writes().wrote("window");
            }
            list
        }
        Err(e) => {
            tracing::warn!(error = %e, "session_slots: couldn't update the window slots");
            slots(store)
        }
    }
}

/// Give each of `keys` that has no slot the next one at the end, in the order given, and return every slot.
pub(crate) fn place(store: &Store, keys: &[String]) -> Vec<String> {
    update(store, |list| {
        for key in keys.iter().filter(|k| !k.is_empty()) {
            if !list.contains(key) {
                list.push(key.clone());
            }
        }
        if list.len() > MAX_SLOTS {
            let over = list.len() - MAX_SLOTS;
            list.drain(..over);
        }
    })
}

/// A window's contents were deleted: its slot goes and the ones after it move up.
pub(crate) fn forget(store: &Store, ws_id: &str) {
    if slots(store).iter().any(|k| k == ws_id) {
        update(store, |list| list.retain(|k| k != ws_id));
    }
}

/// A window's contents were rebuilt under a new id: it keeps its slot.
pub(crate) fn renamed(store: &Store, old: &str, new: &str) {
    if old == new || !slots(store).iter().any(|k| k == old) {
        return;
    }
    update(store, |list| {
        if list.iter().any(|k| k == new) {
            list.retain(|k| k != old);
        } else if let Some(at) = list.iter().position(|k| k == old) {
            list[at] = new.to_string();
        }
    });
}

/// Add the order to a workspace's record, so it is saved with it.
pub(crate) fn into_record(store: &Store, record: &mut Value) {
    let list = slots(store);
    if list.is_empty() {
        return;
    }
    if let Some(obj) = record.as_object_mut() {
        obj.insert(RECORD_KEY.to_string(), json!(list));
    }
}

/// A record was loaded (a start, a new version's first start, a switch of workspace): its order is the
/// open workspace's. A record without one (an older version's) leaves the order as it is.
pub(crate) fn adopt(store: &Store, record: &Value) {
    let Some(saved) = record.get(RECORD_KEY).filter(|v| v.is_array()) else { return };
    let saved = strings(Some(saved));
    update(store, |list| *list = saved);
}

/// `window.PlaceWindowSlots(keys)`: the panel shows the windows `keys` (their workspace ids); the ones
/// with no slot take the next ones. Answers with the whole order.
pub(crate) fn handle_place_window_slots(state: &AppState, call: &WebCallType) -> WebReturnType {
    let keys: Vec<String> = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    WebReturnType::success(json!(place(&state.mstore, &keys)))
}
