// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pub/sub event publishing for WaveObj updates.

use crate::backend::oref::ORef;
use crate::backend::storage::wstore::WaveStore;
use crate::backend::obj::*;
use crate::backend::wps::{self, Broker, WaveEvent};

/// Publish a WaveObj update event through the broker.
pub fn send_wave_obj_update(broker: &Broker, store: &WaveStore, oref: &ORef) {
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
        broker.publish(WaveEvent {
            event: wps::EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec![oref.to_string()],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({
                "updatetype": UPDATE_TYPE_UPDATE,
                "otype": oref.otype,
                "oid": oref.oid,
                "obj": obj,
            })),
        });
    }
}
