// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a message in an agent's conversation came from, as srv knows it when
//! it writes the message down (docs/specs/SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md
//! §3).
//!
//! Every producer of an agent message (a pane, a jekt, a Swarm broadcast, a
//! schedule, the Supervisor, a paired phone, AgentMux itself) builds one
//! [`MessageOrigin`] from the typed values it already holds, and passes it down
//! the delivery path with the message. The record writers store it beside the
//! message in the record's `amx` object:
//!
//! ```json
//! {"type":"user","message":{"role":"user","content":"…"},
//!  "amx":{"v":1,"origin":{"kind":"agent","from":"agent1","trust":"host-verified"},
//!         "notes":["[AgentMux] AgentMux was upgraded …"]}}
//! ```
//!
//! Nothing here reads a message body: the origin is never inferred from text.
//! A producer that sets none is a compile error, because the parameter is not
//! optional anywhere on the path.

use serde::{Deserialize, Serialize};

use crate::backend::blockcontroller::health::TriggerKind;

/// The shape version of `amx.origin`; a reader that doesn't know a version
/// treats the record as untyped.
pub const ORIGIN_VERSION: u32 = 1;

/// How far a jekt's sender is trusted, as the jekt marker's `TRUST=` says
/// (`reactive::sanitize::jekt_trust`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "kebab-case")]
pub enum OriginTrust {
    HostVerified,
    ChannelVerified,
    LanVerified,
    WanVerified,
    Unverified,
    SelfDeclared,
    NetworkClaimed,
}

impl OriginTrust {
    /// The label the jekt marker carries (`TRUST=<label>`).
    pub fn label(self) -> &'static str {
        match self {
            OriginTrust::HostVerified => "host-verified",
            OriginTrust::ChannelVerified => "channel-verified",
            OriginTrust::LanVerified => "lan-verified",
            OriginTrust::WanVerified => "wan-verified",
            OriginTrust::Unverified => "unverified",
            OriginTrust::SelfDeclared => "self-declared",
            OriginTrust::NetworkClaimed => "network-claimed",
        }
    }
}

/// A jekt's sensitivity tier, as in the marker's `TIER=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum OriginTier {
    Info,
    Coord,
    Sensitive,
}

/// How a jekt reached this srv, as in the marker's `DELIVERY=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum OriginDelivery {
    Host,
    Channel,
    Lan,
    Wan,
}

/// Whether the sender's signature checked out (the marker's `SIG=`); absent
/// when the message carried none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum OriginSig {
    Verified,
    Invalid,
}

/// Who or what a message is from, set by srv from typed values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MessageOrigin {
    /// The display kind; the same enum the working row's turn trigger uses.
    pub kind: TriggerKind,
    /// The sender: the agent, `cron`, `github-consumer`; absent for the human.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub trust: Option<OriginTrust>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tier: Option<OriginTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub delivery: Option<OriginDelivery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sig: Option<OriginSig>,
    /// The jekt's `MSGID`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub msg_id: Option<String>,
    /// When the sender sent it (unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub sent_ms: Option<i64>,
    /// When srv delivered it (unix ms); far from `sent_ms` means a late delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub delivered_ms: Option<i64>,
    /// Seconds a held message waited before it was replayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub held_for_s: Option<u64>,
}

impl MessageOrigin {
    /// A message of `kind` and nothing else known about it.
    pub fn of(kind: TriggerKind) -> Self {
        MessageOrigin {
            kind,
            from: None,
            trust: None,
            tier: None,
            delivery: None,
            sig: None,
            msg_id: None,
            sent_ms: None,
            delivered_ms: None,
            held_for_s: None,
        }
    }

    /// The human, in a pane or on a paired device.
    pub fn user() -> Self {
        Self::of(TriggerKind::User)
    }

    /// The human's Swarm broadcast.
    pub fn broadcast() -> Self {
        Self::of(TriggerKind::Broadcast)
    }

    /// AgentMux on its own behalf (reinjection, a side question).
    pub fn system() -> Self {
        Self::of(TriggerKind::System)
    }

    /// The Swarm Supervisor's continuation.
    pub fn supervisor() -> Self {
        Self::of(TriggerKind::Supervisor)
    }

    /// A message `from` sent, with the kind its name implies: `cron` and
    /// `loop` are schedules, `*-consumer` services, anything else an agent.
    /// The name is srv's own record of the sender (the request's
    /// `source_agent`), not text read from a body.
    pub fn from_sender(from: &str) -> Self {
        let kind = match from {
            "cron" | "loop" => TriggerKind::Schedule,
            f if f.ends_with("-consumer") => TriggerKind::Service,
            _ => TriggerKind::Agent,
        };
        MessageOrigin { from: Some(from.to_string()), ..Self::of(kind) }
    }

    /// The security class of the turn this message starts: the human (and
    /// their Swarm broadcast) is `User`, AgentMux's own input is `System`,
    /// everything else is `Automated`. Derived here so the two cannot drift.
    pub fn turn_origin(&self) -> crate::backend::blockcontroller::health::TurnOrigin {
        use crate::backend::blockcontroller::health::TurnOrigin;
        match self.kind {
            TriggerKind::User | TriggerKind::Broadcast => TurnOrigin::User,
            TriggerKind::System => TurnOrigin::System,
            _ => TurnOrigin::Automated,
        }
    }

    /// The origin of a composer message: `origin` is what the route proved
    /// about the caller (`composer_origin`), never the request's word.
    pub fn from_composer(origin: crate::backend::blockcontroller::health::TurnOrigin) -> Self {
        use crate::backend::blockcontroller::health::TurnOrigin;
        match origin {
            TurnOrigin::User => Self::user(),
            TurnOrigin::System => Self::system(),
            // An unproven caller is never the human; `composer_origin` refuses an agent before this.
            TurnOrigin::Automated => Self::of(TriggerKind::Agent),
        }
    }

    /// The record `amx.origin` for this message.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// What a record writer stores beside a message: its origin and the notes srv
/// attached to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordOrigin {
    pub origin: MessageOrigin,
    /// The `[AgentMux]` notes srv appended to the message, in order
    /// (`held_notes::attach`). A viewer compares the exact suffix
    /// `"\n\n" + notes.join("\n")` of the content (spec §3.2).
    pub notes: Vec<String>,
}

impl RecordOrigin {
    pub fn new(origin: MessageOrigin) -> Self {
        RecordOrigin { origin, notes: Vec::new() }
    }

    /// Adds `origin` and `notes` to `record["amx"]`, keeping what is already
    /// there (`via`, `device`, `message_id`).
    pub fn stamp(&self, record: &mut serde_json::Value) {
        if !record.get("amx").is_some_and(serde_json::Value::is_object) {
            record["amx"] = serde_json::json!({});
        }
        let amx = &mut record["amx"];
        amx["v"] = serde_json::json!(ORIGIN_VERSION);
        amx["origin"] = self.origin.to_json();
        if !self.notes.is_empty() {
            amx["notes"] = serde_json::json!(self.notes);
        }
    }
}

/// `line` (one NDJSON user record, no trailing newline) with `record` added to
/// its `amx`. A line that is not a JSON object is returned as it is: the
/// record then has no origin, and a reader takes it as untyped.
pub fn stamp_line(line: &str, record: &RecordOrigin) -> String {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(mut value) if value.is_object() => {
            record.stamp(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| line.to_string())
        }
        _ => line.to_string(),
    }
}

/// [`stamp_line`] for a line with its trailing newline, which is kept.
pub fn stamp_bytes(line: &[u8], record: &RecordOrigin) -> Vec<u8> {
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let Ok(text) = std::str::from_utf8(body) else { return line.to_vec() };
    let mut out = stamp_line(text, record).into_bytes();
    if line.ends_with(b"\n") {
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_human_and_their_broadcast_start_a_user_turn() {
        use crate::backend::blockcontroller::health::TurnOrigin;
        assert_eq!(MessageOrigin::user().turn_origin(), TurnOrigin::User);
        assert_eq!(MessageOrigin::broadcast().turn_origin(), TurnOrigin::User);
        assert_eq!(MessageOrigin::system().turn_origin(), TurnOrigin::System);
        for o in [MessageOrigin::from_sender("agent1"), MessageOrigin::from_sender("cron"), MessageOrigin::supervisor()] {
            assert_eq!(o.turn_origin(), TurnOrigin::Automated, "{o:?}");
        }
        assert_eq!(MessageOrigin::from_composer(TurnOrigin::Automated).turn_origin(), TurnOrigin::Automated);
    }

    #[test]
    fn a_sender_name_decides_the_kind_srv_gave_it() {
        assert_eq!(MessageOrigin::from_sender("cron").kind, TriggerKind::Schedule);
        assert_eq!(MessageOrigin::from_sender("loop").kind, TriggerKind::Schedule);
        assert_eq!(MessageOrigin::from_sender("github-consumer").kind, TriggerKind::Service);
        assert_eq!(MessageOrigin::from_sender("agent1").kind, TriggerKind::Agent);
        assert_eq!(MessageOrigin::from_sender("agent1").from.as_deref(), Some("agent1"));
    }

    #[test]
    fn stamping_keeps_the_device_fields_and_adds_the_origin() {
        let mut record = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": "hi" },
            "amx": { "via": "viewer", "device": "Pixel 9", "message_id": "m1" },
        });
        RecordOrigin::new(MessageOrigin::user()).stamp(&mut record);
        assert_eq!(record["amx"]["via"], "viewer");
        assert_eq!(record["amx"]["device"], "Pixel 9");
        assert_eq!(record["amx"]["v"], 1);
        assert_eq!(record["amx"]["origin"], serde_json::json!({ "kind": "user" }));
        assert!(record["amx"].get("notes").is_none(), "no notes, no key");
        assert_eq!(record["message"]["content"], "hi", "the message itself is untouched");
    }

    #[test]
    fn a_line_that_is_not_a_record_is_stored_as_it_is() {
        let ro = RecordOrigin::new(MessageOrigin::user());
        assert_eq!(stamp_line("not json", &ro), "not json");
        assert_eq!(stamp_line("[1,2]", &ro), "[1,2]");
        let stamped: serde_json::Value = serde_json::from_str(&stamp_line(r#"{"type":"user"}"#, &ro)).unwrap();
        assert_eq!(stamped["amx"]["origin"]["kind"], "user");
    }

    #[test]
    fn notes_are_stored_in_order() {
        let mut record = serde_json::json!({ "type": "user", "message": { "role": "user", "content": "x" } });
        let ro = RecordOrigin {
            origin: MessageOrigin::from_sender("agent1"),
            notes: vec!["[AgentMux] one".into(), "[AgentMux] two".into()],
        };
        ro.stamp(&mut record);
        assert_eq!(record["amx"]["notes"], serde_json::json!(["[AgentMux] one", "[AgentMux] two"]));
        assert_eq!(record["amx"]["origin"]["from"], "agent1");
        assert_eq!(record["amx"]["origin"]["kind"], "agent");
    }
}
