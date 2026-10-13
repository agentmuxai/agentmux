// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Broker events with a typed payload (docs/specs/SPEC_TYPED_EVENT_PAYLOADS_2026_10_13.md).
//!
//! Each event is a marker type naming its wire name and its payload type, declared once in the
//! `events!` list below. That list is also the schema: `schema()` (exported to
//! `frontend/types/rpc/event-schema.json`) feeds the generated `MuxEventPayloads` map, so the
//! frontend's `onMuxEvent(WpsEvent.X, (data) => …)` gets `data` as the Rust type. Publish one with
//! [`Broker::publish_event`](super::mps::Broker::publish_event). An event not listed here is still
//! published as a plain `MuxEvent` with untyped data.

use serde::Serialize;

use crate::backend::rpc::schema::{ts_type, TsType};

/// A broker event with a typed payload.
pub trait Event {
    /// The wire name (`MuxEvent::event`).
    const NAME: &'static str;
    type Payload: Serialize + ts_rs::TS + 'static;
}

macro_rules! events {
    ($($(#[$doc:meta])* $marker:ident: $name:expr => $payload:ty,)*) => {
        $(
            $(#[$doc])*
            pub struct $marker;
            impl Event for $marker {
                const NAME: &'static str = $name;
                type Payload = $payload;
            }
        )*
        /// Every typed event: its wire name and its payload as TypeScript.
        pub fn schema() -> Vec<(&'static str, TsType)> {
            vec![$(($name, ts_type::<$payload>())),*]
        }
    };
}

events! {
    /// A block controller's runtime status (`publish_controller_status`).
    ControllerStatus: super::mps::EVENT_CONTROLLER_STATUS => crate::backend::blockcontroller::BlockControllerRuntimeStatus,
    /// One sysinfo sample for a connection (`sysinfo::run_sysinfo_loop`).
    SysInfo: super::mps::EVENT_SYS_INFO => crate::backend::rpc_types::TimeSeriesData,
}

/// `schema()` as the JSON the event-schema export writes: one row per event, sorted by name.
pub fn schema_json() -> serde_json::Value {
    let mut rows = schema();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    serde_json::Value::Array(
        rows.into_iter()
            .map(|(name, ts)| serde_json::json!({ "event": name, "payloadTs": { "name": ts.name, "imports": ts.imports } }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_event_is_listed_once() {
        let names: Vec<&str> = schema().into_iter().map(|(n, _)| n).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "an event listed twice: {names:?}");
    }

    #[test]
    fn payloads_name_their_generated_types() {
        let rows = schema();
        let sys = rows.iter().find(|(n, _)| *n == "sysinfo").expect("sysinfo");
        assert_eq!(sys.1.name, "TimeSeriesData");
        assert_eq!(sys.1.imports, vec!["TimeSeriesData".to_string()]);
    }
}
