#![allow(dead_code)]
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pub/sub event publishing for StoreObj updates.

use crate::backend::oref::ORef;
use crate::backend::storage::store::Store;
use crate::backend::obj::*;
use crate::backend::mps::Broker;

/// Publish a StoreObj update event through the broker.
pub fn send_mux_obj_update(broker: &Broker, store: &Store, oref: &ORef) {
    let obj_json = match oref.otype.as_str() {
        OTYPE_CLIENT => store
            .get::<Client>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_WINDOW => store
            .get::<Window>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_WORKSPACE => store
            .get::<Workspace>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_TAB => store
            .get::<Tab>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_LAYOUT => store
            .get::<LayoutState>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_BLOCK => store
            .get::<Block>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        _ => None,
    };

    if let Some(obj) = obj_json {
        let update = crate::backend::obj::MuxObjUpdate {
            updatetype: UPDATE_TYPE_UPDATE.to_string(),
            otype: oref.otype.to_string(),
            oid: oref.oid.to_string(),
            obj: Some(obj),
        };
        broker.publish_event::<crate::backend::mps_events::MuxObjUpdated>(&update, vec![oref.to_string()], 0);
    }
}
