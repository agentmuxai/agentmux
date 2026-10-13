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

#[cfg(test)]
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
        /// Every typed event: its wire name and its payload as TypeScript (the export test reads it).
        #[cfg(test)]
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
    /// An agent's turn ledger changed (`blockcontroller::health::ledger_publisher`).
    AgentTurn: super::mps::EVENT_AGENT_TURN => crate::backend::blockcontroller::health::AgentTurnEvent,
    /// An agent failed, classified (`agents::failure`).
    AgentFailed: super::mps::EVENT_AGENT_FAILURE => crate::agents::failure::AgentFailure,
    /// What an agent process runs with, and whether a restart is pending.
    AgentRuntime: super::mps::EVENT_AGENT_RUNTIME => crate::backend::agent_runtime::AgentRuntimeEvent,
    /// Where an agent's CLI auto-compacts.
    AgentContextUsageChanged: super::mps::EVENT_AGENT_CONTEXT_USAGE => crate::backend::agent_context_usage::AgentContextUsageEvent,
    /// A queued message reached the agent.
    AgentMessageAccepted: super::mps::EVENT_AGENT_MESSAGE_ACCEPTED => crate::backend::blockcontroller::MessageAcceptedEvent,
    /// A stale `--resume` is being retried, or the retry resolved.
    AgentResumeRetry: super::mps::EVENT_AGENT_RESUME_RETRY => ResumeRetryEvent,
    /// One line of a closing agent pane's shutdown log (`sagas::close_pane::publish_shutdown`).
    AgentShutdown: crate::sagas::close_pane::EVENT_AGENT_SHUTDOWN => crate::sagas::close_pane::ShutdownLineEvent,
    /// A call to action srv holds open started, or ended (`backend::user_attention`).
    UserAttention: super::mps::EVENT_USER_ATTENTION => crate::backend::user_attention::UserAttentionEvent,
}

/// The `agent-resume-retry` event's payload (`mps_events::AgentResumeRetry`).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ResumeRetryEvent {
    #[ts(type = r#""retrying" | "resolved""#)]
    pub status: String,
    /// When the retry began; only on `retrying`.
    #[serde(rename = "startedAt", skip_serializing_if = "Option::is_none")]
    #[ts(rename = "startedAt", optional)]
    pub started_at: Option<String>,
}

/// `schema()` as the JSON the event-schema export writes: one row per event, sorted by name.
#[cfg(test)]
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
