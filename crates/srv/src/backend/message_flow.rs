// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "A sent B a message at T": the `message-flow` event and the short list of recent ones, for the message pulses
//! of the Atlas agent map.
//!
//! It carries ids and times only: the message id, the two agent ids, the host of each end when this srv knows
//! it, how the message travelled and what happened to it here. There is no field for the text, the subject, a
//! file name or a size, so none can leak through it. Each srv reports what it handled: a send to another
//! computer shows on both srvs (the sender's as `lan` or `queued`, the receiver's as `delivered`), and a reader
//! joins the two by `msg_id`.
//!
//! The recent list lives in memory only (`RECENT_MAX` entries, newest last) and is read with
//! `GET /api/v1/message-flow`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use super::mps::Broker;
use super::mps_events::MessageFlow;

/// How many recent messages are kept.
pub const RECENT_MAX: usize = 200;

/// One end of a message: the agent's id (its slug) and the host it runs on, when this srv knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MessageFlowEnd {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
}

/// How the message travelled to or from this srv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum MessageFlowVia {
    /// Between two agents of this srv.
    Host,
    /// To or from another srv on this computer.
    Channel,
    /// To or from another computer on the LAN.
    Lan,
    /// Through the cloud relay.
    Wan,
}

/// What became of the message here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum MessageFlowOutcome {
    /// The receiving agent took it, here or on the peer this srv handed it to.
    Delivered,
    /// Kept for an agent that cannot take it now; it is delivered later.
    Held,
    /// Given to the cloud relay for the receiver's srv to collect.
    Queued,
    /// Not taken: the sender is told why.
    Refused,
}

/// The `message-flow` event's payload (`mps_events::MessageFlow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MessageFlowEvent {
    pub msg_id: String,
    /// When this srv handled it, in milliseconds since 1970.
    #[ts(type = "number")]
    pub at_ms: u64,
    pub from: MessageFlowEnd,
    pub to: MessageFlowEnd,
    pub via: MessageFlowVia,
    pub outcome: MessageFlowOutcome,
}

/// What a call site knows of a message: ids and the two outcomes, never its content.
pub struct Seen<'a> {
    pub msg_id: &'a str,
    pub from: &'a str,
    pub from_host: Option<&'a str>,
    pub to: &'a str,
    pub to_host: Option<&'a str>,
    pub via: MessageFlowVia,
    pub outcome: MessageFlowOutcome,
}

struct Flow {
    broker: Arc<Broker>,
    recent: Mutex<VecDeque<MessageFlowEvent>>,
}

static FLOW: OnceLock<Flow> = OnceLock::new();

/// Start publishing on `broker` for the rest of the process. The first call wins.
pub fn install(broker: Arc<Broker>) {
    let _ = FLOW.set(Flow { broker, recent: Mutex::new(VecDeque::with_capacity(RECENT_MAX)) });
}

/// The event for `seen`, as handled at `at_ms`. A message with no sender (a cron job, a bridge) or no id has no
/// line to draw, so there is none.
fn event_of(seen: &Seen<'_>, at_ms: u64) -> Option<MessageFlowEvent> {
    let (msg_id, from, to) = (seen.msg_id.trim(), seen.from.trim(), seen.to.trim());
    if msg_id.is_empty() || from.is_empty() || to.is_empty() {
        return None;
    }
    let end = |agent: &str, host: Option<&str>| MessageFlowEnd {
        agent: agent.to_string(),
        host: host.map(str::trim).filter(|h| !h.is_empty()).map(str::to_string),
    };
    Some(MessageFlowEvent {
        msg_id: msg_id.to_string(),
        at_ms,
        from: end(from, seen.from_host),
        to: end(to, seen.to_host),
        via: seen.via,
        outcome: seen.outcome,
    })
}

/// Adds `event` to `recent`, dropping the oldest once there are `RECENT_MAX`.
fn remember(recent: &mut VecDeque<MessageFlowEvent>, event: MessageFlowEvent) {
    if recent.len() >= RECENT_MAX {
        recent.pop_front();
    }
    recent.push_back(event);
}

/// Records that `seen` happened: published as the `message-flow` event, and kept in the recent list. Does
/// nothing before [`install`].
pub fn note(seen: &Seen<'_>) {
    if let Some(flow) = FLOW.get() {
        flow.record(seen, agentmux_common::time::now_ms() as u64);
    }
}

impl Flow {
    /// Keeps and publishes the event for `seen`, handled at `at_ms`.
    fn record(&self, seen: &Seen<'_>, at_ms: u64) {
        let Some(event) = event_of(seen, at_ms) else { return };
        remember(&mut self.recent.lock().unwrap_or_else(|e| e.into_inner()), event.clone());
        self.broker.publish_event::<MessageFlow>(&event, vec![], 0);
    }
}

/// The recent messages, oldest first.
pub fn recent() -> Vec<MessageFlowEvent> {
    FLOW.get().map(|f| f.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen<'a>(msg_id: &'a str, from: &'a str, to: &'a str) -> Seen<'a> {
        Seen { msg_id, from, from_host: Some("charlie"), to, to_host: None, via: MessageFlowVia::Lan, outcome: MessageFlowOutcome::Delivered }
    }

    #[test]
    fn an_event_has_the_ids_the_hosts_and_the_outcome_and_nothing_else() {
        let e = event_of(&seen("m-1", "maricon-2", "lazo"), 7).unwrap();
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({
                "msg_id": "m-1", "at_ms": 7,
                "from": { "agent": "maricon-2", "host": "charlie" },
                "to": { "agent": "lazo" },
                "via": "lan", "outcome": "delivered",
            })
        );
    }

    #[test]
    fn a_message_with_no_id_or_no_sender_has_no_event() {
        assert!(event_of(&seen("", "a", "b"), 1).is_none());
        assert!(event_of(&seen("m", "  ", "b"), 1).is_none());
        assert!(event_of(&seen("m", "a", ""), 1).is_none());
    }

    #[test]
    fn a_blank_host_is_no_host() {
        let mut s = seen("m", "a", "b");
        s.from_host = Some(" ");
        assert_eq!(event_of(&s, 1).unwrap().from.host, None);
    }

    #[test]
    fn every_outcome_and_route_has_its_own_name() {
        let names = |s: &Seen<'_>| {
            let v = serde_json::to_value(event_of(s, 1).unwrap()).unwrap();
            (v["via"].as_str().unwrap().to_string(), v["outcome"].as_str().unwrap().to_string())
        };
        for (via, outcome, want) in [
            (MessageFlowVia::Host, MessageFlowOutcome::Held, ("host", "held")),
            (MessageFlowVia::Channel, MessageFlowOutcome::Refused, ("channel", "refused")),
            (MessageFlowVia::Wan, MessageFlowOutcome::Queued, ("wan", "queued")),
        ] {
            let mut s = seen("m", "a", "b");
            (s.via, s.outcome) = (via, outcome);
            assert_eq!(names(&s), (want.0.to_string(), want.1.to_string()));
        }
    }

    #[test]
    fn a_message_is_published_as_the_event_and_kept_in_the_recent_list() {
        let broker = Arc::new(Broker::new());
        let published = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&published);
        broker.add_observer(Arc::new(move |e: &crate::backend::mps::MuxEvent| sink.lock().unwrap().push((e.event.clone(), e.data.clone()))));
        let flow = Flow { broker, recent: Mutex::new(VecDeque::new()) };
        flow.record(&seen("m-1", "maricon-2", "lazo"), 9);
        flow.record(&seen("", "maricon-2", "lazo"), 10);
        let published = published.lock().unwrap();
        assert_eq!(published.len(), 1, "a message with no id says nothing");
        assert_eq!(published[0].0, "message-flow");
        assert_eq!(published[0].1.as_ref().unwrap()["msg_id"], "m-1");
        assert_eq!(flow.recent.lock().unwrap().iter().map(|e| e.at_ms).collect::<Vec<_>>(), vec![9]);
    }

    #[test]
    fn the_recent_list_keeps_the_newest_and_is_bounded() {
        let mut recent = VecDeque::new();
        for n in 0..RECENT_MAX + 5 {
            remember(&mut recent, event_of(&seen(&format!("m-{n}"), "a", "b"), n as u64).unwrap());
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent.front().unwrap().msg_id, "m-5");
        assert_eq!(recent.back().unwrap().msg_id, format!("m-{}", RECENT_MAX + 4));
    }
}
