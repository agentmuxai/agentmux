// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An install's presence record: one AgentMux install (a channel on a
//! machine) saying it is up, on which platform, and which agents it runs, so
//! the account's relay can list it to a signed-in phone that is not on the
//! same LAN. Wire contract: agentmux-mobile's
//! SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06 §6.1; version 2 adds each
//! agent's `state` (agentmux-mobile's
//! SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §13.1). The desktop
//! publishes v2; v1 is still built and checked, for the relay's sake.
//! Version 3 is v2 with an optional top-level `gone`. The desktop sends it
//! only as the goodbye when it quits, signs out or stops publishing:
//! `gone: true` and no agent states, which the relay keeps as "offline
//! since" for a short while instead of a live entry.
//!
//! Signed with the install's WAN instance key (the W3-S identity in
//! [`crate::jekt_sign`]), so the record names the key that signed it and
//! `instance_id` is checkable against it. `agentmux-srv`'s
//! `muxbus::wan_presence` builds and publishes it; the relay re-implements
//! the check in TypeScript, and the fixed vector in the tests below is the
//! contract between the two.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};

use crate::jekt_sign::{decode_public_key_b64, ed25519_sign_b64, ed25519_verify_b64, wan_instance_id, FIELD_SEP};

/// Domain separator for the presence signature, the same in v1 and v2 (`v`
/// is signed right after it).
const PRESENCE_DOMAIN: &str = "amx-install-presence-v1";
/// Between an agent's name, kind and (v2) state in the signed list.
const AGENT_FIELD_SEP: char = '\u{2}';
/// Between agents in the signed list.
const AGENT_SEP: char = '\u{3}';

/// The version the desktop publishes.
pub const PRESENCE_VERSION: u32 = 2;
/// The first version: no agent `state`.
pub const PRESENCE_VERSION_V1: u32 = 1;
/// The goodbye: `gone: true`, no agent `state`.
pub const PRESENCE_VERSION_GOODBYE: u32 = 3;
/// The values an agent's `state` may take (the LAN feed's `agent_status`).
pub const PRESENCE_STATES: [&str; 5] = ["working", "waiting", "idle", "stopped", "error"];
/// Agents in one record; the rest are left out.
pub const MAX_PRESENCE_AGENTS: usize = 200;
/// Longest `hostname`, `channel` and agent `name`, in characters.
pub const MAX_PRESENCE_NAME_CHARS: usize = 128;
pub const MAX_PRESENCE_VERSION_CHARS: usize = 64;
pub const MAX_CHANNELS_RUNNING: u32 = 99;
const MAX_OS_LEN: usize = 16;

/// One agent of the install. `kind` is `host` or `container` (the agent's
/// block `agentMode`); `state` (v2 only) is one of [`PRESENCE_STATES`], or
/// absent when unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceAgent {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// The record, in the contract's field order. Public keys and the signature
/// are standard base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPresence {
    pub v: u32,
    pub instance_id: String,
    pub instance_public_key: String,
    pub hostname: String,
    pub channel: String,
    pub os: String,
    pub version: String,
    pub channels_running: u32,
    pub agents: Vec<PresenceAgent>,
    pub published_at_ms: u64,
    /// v3 only: the install has gone offline. The desktop sends v3 only as
    /// a goodbye, so always with this set; the relay also takes a v3
    /// record without it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub gone: bool,
    pub sig: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn has_control_char(s: &str) -> bool {
    s.chars().any(|c| u32::from(c) < 0x20)
}

fn text_in_bounds(s: &str, max_chars: usize) -> bool {
    (1..=max_chars).contains(&s.chars().count()) && !has_control_char(s)
}

/// Empty, or a plain lowercase token (`host_os`'s rule).
fn os_in_bounds(os: &str) -> bool {
    os.is_empty()
        || (os.len() <= MAX_OS_LEN
            && os.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'))
}

fn agent_in_bounds(agent: &PresenceAgent) -> bool {
    text_in_bounds(&agent.name, MAX_PRESENCE_NAME_CHARS)
        && (agent.kind == "host" || agent.kind == "container")
        && agent.state.as_deref().is_none_or(|s| PRESENCE_STATES.contains(&s))
}

/// The agents as a record carries them: sorted by lower-cased name, ties by
/// name; an agent whose name, kind or state is out of bounds (a control
/// character, too long, an unknown kind or state) left out; each name once;
/// at most [`MAX_PRESENCE_AGENTS`].
pub fn canonical_agents(agents: impl IntoIterator<Item = PresenceAgent>) -> Vec<PresenceAgent> {
    let mut keyed: Vec<(String, PresenceAgent)> = agents
        .into_iter()
        .filter(agent_in_bounds)
        .map(|a| (a.name.to_lowercase(), a))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    keyed.dedup_by(|later, earlier| later.1.name == earlier.1.name);
    keyed.truncate(MAX_PRESENCE_AGENTS);
    keyed.into_iter().map(|(_, a)| a).collect()
}

impl InstallPresence {
    /// Build and sign a [`PRESENCE_VERSION`] record with the instance key.
    /// `agents` may come in any order and is passed through
    /// [`canonical_agents`]; `channels_running` is clamped to
    /// 1..=[`MAX_CHANNELS_RUNNING`]. `None` for a malformed key or a
    /// top-level field out of bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        instance_private_key: &[u8],
        hostname: &str,
        channel: &str,
        os: &str,
        version: &str,
        channels_running: u32,
        agents: impl IntoIterator<Item = PresenceAgent>,
        published_at_ms: u64,
    ) -> Option<Self> {
        Self::sign_version(
            PRESENCE_VERSION,
            instance_private_key,
            hostname,
            channel,
            os,
            version,
            channels_running,
            agents,
            published_at_ms,
        )
    }

    /// [`Self::sign`] for record version `v` (1, 2 or 3). A v1 record
    /// carries no states: any given are dropped. A v3 record is the goodbye:
    /// `gone` is set and the states are dropped too.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_version(
        v: u32,
        instance_private_key: &[u8],
        hostname: &str,
        channel: &str,
        os: &str,
        version: &str,
        channels_running: u32,
        agents: impl IntoIterator<Item = PresenceAgent>,
        published_at_ms: u64,
    ) -> Option<Self> {
        let seed: [u8; 32] = instance_private_key.try_into().ok()?;
        let public_key = ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        let gone = v == PRESENCE_VERSION_GOODBYE;
        let agents = agents.into_iter().map(|mut a| {
            if v == PRESENCE_VERSION_V1 || gone {
                a.state = None;
            }
            a
        });
        let mut record = Self {
            v,
            instance_id: wan_instance_id(&public_key),
            instance_public_key: BASE64.encode(public_key),
            hostname: hostname.to_string(),
            channel: channel.to_string(),
            os: os.to_string(),
            version: version.to_string(),
            channels_running: channels_running.clamp(1, MAX_CHANNELS_RUNNING),
            agents: canonical_agents(agents),
            published_at_ms,
            gone,
            sig: String::new(),
        };
        if !record.in_bounds() {
            return None;
        }
        record.sig = ed25519_sign_b64(&seed, &record.signed_material())?;
        Some(record)
    }

    /// The UTF-8 string the signature covers: the fields joined by U+0001,
    /// numbers in decimal, the agents joined by U+0003 (empty when there are
    /// none), in the record's order. In v1 an agent is `name` U+0002 `kind`;
    /// in v2 and v3 `name` U+0002 `kind` U+0002 `state`, `state` empty when
    /// absent. v3 adds one more field at the end: U+0001 then `1` when
    /// `gone`, nothing otherwise.
    pub fn signed_material(&self) -> String {
        let agents = self
            .agents
            .iter()
            .map(|a| {
                if self.v == PRESENCE_VERSION_V1 {
                    format!("{}{AGENT_FIELD_SEP}{}", a.name, a.kind)
                } else {
                    let state = a.state.as_deref().unwrap_or("");
                    format!("{}{AGENT_FIELD_SEP}{}{AGENT_FIELD_SEP}{state}", a.name, a.kind)
                }
            })
            .collect::<Vec<_>>()
            .join(&AGENT_SEP.to_string());
        let gone = match (self.v, self.gone) {
            (PRESENCE_VERSION_GOODBYE, true) => format!("{FIELD_SEP}1"),
            (PRESENCE_VERSION_GOODBYE, false) => FIELD_SEP.to_string(),
            _ => String::new(),
        };
        format!(
            "{PRESENCE_DOMAIN}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}{}{FIELD_SEP}\
             {}{FIELD_SEP}{}{FIELD_SEP}{agents}{FIELD_SEP}{}{gone}",
            self.v,
            self.instance_id,
            self.hostname,
            self.channel,
            self.os,
            self.version,
            self.channels_running,
            self.published_at_ms
        )
    }

    /// The contract's bounds, on every field the signature covers. The
    /// agents must already be in canonical order: the order is signed. A v1
    /// record may not carry a state, which its signature would not cover.
    /// Only v3 may be `gone`, and a `gone` record carries no state.
    pub fn in_bounds(&self) -> bool {
        let stateless = || self.agents.iter().all(|a| a.state.is_none());
        let version_ok = match self.v {
            PRESENCE_VERSION_V1 => !self.gone && stateless(),
            PRESENCE_VERSION => !self.gone,
            PRESENCE_VERSION_GOODBYE => !self.gone || stateless(),
            _ => false,
        };
        version_ok
            && text_in_bounds(&self.hostname, MAX_PRESENCE_NAME_CHARS)
            && text_in_bounds(&self.channel, MAX_PRESENCE_NAME_CHARS)
            && text_in_bounds(&self.version, MAX_PRESENCE_VERSION_CHARS)
            && os_in_bounds(&self.os)
            && (1..=MAX_CHANNELS_RUNNING).contains(&self.channels_running)
            && self.agents.len() <= MAX_PRESENCE_AGENTS
            && self.agents == canonical_agents(self.agents.iter().cloned())
    }

    /// In bounds, `instance_id` is the hash of `instance_public_key`, and
    /// that key signed the record. Never panics on malformed input.
    pub fn verify(&self) -> bool {
        let Some(public_key) = decode_public_key_b64(&self.instance_public_key) else { return false };
        self.in_bounds()
            && wan_instance_id(&public_key) == self.instance_id
            && ed25519_verify_b64(&public_key, &self.signed_material(), &self.sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str, kind: &str) -> PresenceAgent {
        PresenceAgent { name: name.to_string(), kind: kind.to_string(), state: None }
    }

    fn agent_in(name: &str, kind: &str, state: &str) -> PresenceAgent {
        PresenceAgent { state: Some(state.to_string()), ..agent(name, kind) }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    // The shared vector: the relay's TypeScript suite asserts the same seed,
    // record, material and signature.
    const SEED_HEX: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    const MATERIAL_HEX: &str = "\
        616d782d696e7374616c6c2d70726573656e63652d76310131016d7733616d3436773577656578346134667172633361766e\
        7561016e61726b6f01737461626c650177696e646f777301302e35392e31310133016167656e747802636f6e7461696e6572\
        034167656e745902686f73740343616d70657202686f73740131373931333532343933333838";
    const SIG: &str = "7FxOua+TWFMp3fwOP/mGr+YXjgLfKESklzZ3odiOu5WH5QrstJlk4JSV5WrkMo9rnywbYWdFhRZ7KySij/GNAg==";

    fn seed() -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&SEED_HEX[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    fn vector_record() -> InstallPresence {
        InstallPresence::sign_version(
            PRESENCE_VERSION_V1,
            &seed(),
            "narko",
            "stable",
            "windows",
            "0.59.11",
            3,
            // Any input order: the record sorts.
            [agent("Camper", "host"), agent("agentx", "container"), agent("AgentY", "host")],
            1_791_352_493_388,
        )
        .expect("the vector is in bounds")
    }

    #[test]
    fn the_shared_vector_signs_byte_for_byte() {
        let record = vector_record();
        assert_eq!(hex(record.signed_material().as_bytes()), MATERIAL_HEX);
        assert_eq!(record.sig, SIG);
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "v": 1,
                "instance_id": "mw3am46w5weex4a4fqrc3avnua",
                "instance_public_key": "ebVWLo/mVPlAeLES6KmLp5AfhTrmlb7X4OORC60ElmQ=",
                "hostname": "narko",
                "channel": "stable",
                "os": "windows",
                "version": "0.59.11",
                "channels_running": 3,
                "agents": [
                    {"name": "agentx", "kind": "container"},
                    {"name": "AgentY", "kind": "host"},
                    {"name": "Camper", "kind": "host"},
                ],
                "published_at_ms": 1_791_352_493_388u64,
                "sig": SIG,
            })
        );
        assert!(record.verify());
        // The JSON parses back to the same record, which still verifies.
        let back: InstallPresence = serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert!(back.verify());
    }

    // The v2 vector: the same install, two agents with a state and one
    // without. Shared with the relay's suite like the v1 vector.
    const MATERIAL_V2_HEX: &str = "\
        616d782d696e7374616c6c2d70726573656e63652d76310132016d7733616d3436773577656578346134667172633361766e\
        7561016e61726b6f01737461626c650177696e646f777301302e35392e31310133016167656e747802636f6e7461696e6572\
        02776f726b696e67034167656e745902686f7374020343616d70657202686f73740269646c65013137393133353234393333\
        3838";
    const SIG_V2: &str = "ELDWTLT+BEoxMn/o/AsYYVbGLgIAbJjDPuQ6/AbAZ8fk+jqphiEtEn/c/5ZKjHJZdQF+hxIKzmnq6gLZzmWDDw==";

    fn vector_v2_record() -> InstallPresence {
        InstallPresence::sign(
            &seed(),
            "narko",
            "stable",
            "windows",
            "0.59.11",
            3,
            [agent_in("Camper", "host", "idle"), agent_in("agentx", "container", "working"), agent("AgentY", "host")],
            1_791_352_493_388,
        )
        .expect("the vector is in bounds")
    }

    #[test]
    fn the_v2_shared_vector_signs_byte_for_byte() {
        let record = vector_v2_record();
        assert_eq!(record.v, 2);
        assert_eq!(hex(record.signed_material().as_bytes()), MATERIAL_V2_HEX);
        assert_eq!(record.sig, SIG_V2);
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "v": 2,
                "instance_id": "mw3am46w5weex4a4fqrc3avnua",
                "instance_public_key": "ebVWLo/mVPlAeLES6KmLp5AfhTrmlb7X4OORC60ElmQ=",
                "hostname": "narko",
                "channel": "stable",
                "os": "windows",
                "version": "0.59.11",
                "channels_running": 3,
                "agents": [
                    {"name": "agentx", "kind": "container", "state": "working"},
                    {"name": "AgentY", "kind": "host"},
                    {"name": "Camper", "kind": "host", "state": "idle"},
                ],
                "published_at_ms": 1_791_352_493_388u64,
                "sig": SIG_V2,
            })
        );
        assert!(record.verify());
        let back: InstallPresence = serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(back, record);
        assert!(back.verify());
    }

    // The v3 (goodbye) vector: the v2 vector's install and agents, as a
    // goodbye. Shared with the relay's suite like the others.
    const MATERIAL_V3_HEX: &str = "\
        616d782d696e7374616c6c2d70726573656e63652d76310133016d7733616d3436773577656578346134667172633361766e\
        7561016e61726b6f01737461626c650177696e646f777301302e35392e31310133016167656e747802636f6e7461696e6572\
        02034167656e745902686f7374020343616d70657202686f73740201313739313335323439333338380131";
    const SIG_V3: &str = "CihvXkUTZu79ykAvxqgN2WOkOw5t+FsYJ5YRF3I94MiHZCsi301grrVNLErajob9g+AEd2J2e2Kt1ulOR5KpCQ==";

    fn vector_v3_record() -> InstallPresence {
        InstallPresence::sign_version(
            PRESENCE_VERSION_GOODBYE,
            &seed(),
            "narko",
            "stable",
            "windows",
            "0.59.11",
            3,
            [agent_in("Camper", "host", "idle"), agent_in("agentx", "container", "working"), agent("AgentY", "host")],
            1_791_352_493_388,
        )
        .expect("the vector is in bounds")
    }

    #[test]
    fn the_v3_goodbye_vector_signs_byte_for_byte() {
        let record = vector_v3_record();
        assert_eq!(hex(record.signed_material().as_bytes()), MATERIAL_V3_HEX);
        assert_eq!(record.sig, SIG_V3);
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "v": 3,
                "instance_id": "mw3am46w5weex4a4fqrc3avnua",
                "instance_public_key": "ebVWLo/mVPlAeLES6KmLp5AfhTrmlb7X4OORC60ElmQ=",
                "hostname": "narko",
                "channel": "stable",
                "os": "windows",
                "version": "0.59.11",
                "channels_running": 3,
                "agents": [
                    {"name": "agentx", "kind": "container"},
                    {"name": "AgentY", "kind": "host"},
                    {"name": "Camper", "kind": "host"},
                ],
                "published_at_ms": 1_791_352_493_388u64,
                "gone": true,
                "sig": SIG_V3,
            }),
            "every state absent, gone set"
        );
        assert!(record.verify());
        let back: InstallPresence = serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(back, record);
        assert!(back.verify());
        // v2's layout, states empty, then the flag: 11 fields.
        let material = record.signed_material();
        assert!(material.starts_with(&format!("{PRESENCE_DOMAIN}{FIELD_SEP}3{FIELD_SEP}")), "{material:?}");
        assert!(material.ends_with(&format!("1791352493388{FIELD_SEP}1")), "{material:?}");
        assert_eq!(material.split(FIELD_SEP).count(), 11);
    }

    // The same v3 record without `gone`: the last field is empty. Shared
    // with the relay's suite; the desktop never sends one.
    const SIG_V3_NOT_GONE: &str =
        "/CV2c+4Cc7FJvrbhbzh3fhUnSS6388P/mY2fy8m3FDCrgcauiLrssFCkZh6Y5LemxpbP+UDdRi04Pu4iwEUiCw==";

    #[test]
    fn a_v3_record_without_gone_signs_an_empty_last_field() {
        let mut record = vector_v3_record();
        record.gone = false;
        let material = record.signed_material();
        assert!(material.ends_with(&format!("1791352493388{FIELD_SEP}")), "{material:?}");
        assert_eq!(material.split(FIELD_SEP).count(), 11);
        assert!(!record.verify(), "the goodbye's signature covers gone");
        record.sig = ed25519_sign_b64(&seed(), &material).unwrap();
        assert_eq!(record.sig, SIG_V3_NOT_GONE);
        assert!(record.verify());
        let wire = serde_json::to_value(&record).unwrap();
        assert!(wire.get("gone").is_none(), "{wire}");
        let back: InstallPresence = serde_json::from_value(wire).unwrap();
        assert!(back.verify());
        // Without gone, a v3 record may carry states, like v2.
        let mut stateful = record.clone();
        stateful.agents[0].state = Some("working".into());
        assert!(stateful.in_bounds());
    }

    #[test]
    fn v1_and_v2_keep_ten_fields() {
        assert_eq!(vector_record().signed_material().split(FIELD_SEP).count(), 10);
        assert_eq!(vector_v2_record().signed_material().split(FIELD_SEP).count(), 10);
    }

    #[test]
    fn gone_is_signed_and_only_a_v3_record_may_say_it() {
        let goodbye = vector_v3_record();
        let tampered: Vec<(&str, Box<dyn Fn(&mut InstallPresence)>)> = vec![
            ("gone cleared", Box::new(|r| r.gone = false)),
            ("a state added", Box::new(|r| r.agents[0].state = Some("working".into()))),
            ("version lowered to 2", Box::new(|r| r.v = 2)),
        ];
        for (what, change) in tampered {
            let mut r = goodbye.clone();
            change(&mut r);
            assert!(!r.verify(), "{what} but the goodbye still verifies");
        }
        // A live record that claims to be gone is out of bounds.
        for mut live in [vector_record(), vector_v2_record()] {
            live.gone = true;
            assert!(!live.in_bounds() && !live.verify(), "v{} with gone", live.v);
        }
        // `gone` is on the wire only in v3; a v1 or v2 record reads it as false.
        let v2 = serde_json::to_value(vector_v2_record()).unwrap();
        assert!(v2.get("gone").is_none(), "{v2}");
        let back: InstallPresence = serde_json::from_value(v2).unwrap();
        assert!(!back.gone && back.verify());
    }

    #[test]
    fn a_changed_added_or_removed_state_fails_verification() {
        let good = vector_v2_record();
        let tampered: Vec<(&str, Box<dyn Fn(&mut InstallPresence)>)> = vec![
            ("state changed", Box::new(|r| r.agents[0].state = Some("idle".into()))),
            ("state added", Box::new(|r| r.agents[1].state = Some("working".into()))),
            ("state removed", Box::new(|r| r.agents[2].state = None)),
            ("version lowered", Box::new(|r| r.v = 1)),
            ("unknown version", Box::new(|r| r.v = 3)),
        ];
        for (what, change) in tampered {
            let mut r = good.clone();
            change(&mut r);
            assert!(!r.verify(), "{what} but the record still verifies");
        }
    }

    #[test]
    fn a_v1_record_carries_no_state() {
        let r = InstallPresence::sign_version(
            PRESENCE_VERSION_V1,
            &seed(),
            "narko",
            "stable",
            "windows",
            "1",
            1,
            [agent_in("AgentY", "host", "working")],
            1,
        )
        .unwrap();
        assert_eq!(r.agents, vec![agent("AgentY", "host")], "the state is dropped, not signed");
        assert!(r.verify());
        let mut smuggled = r.clone();
        smuggled.agents[0].state = Some("working".into());
        assert!(!smuggled.verify(), "a v1 signature does not cover a state");
    }

    #[test]
    fn an_unknown_state_leaves_the_agent_out() {
        assert_eq!(
            canonical_agents([agent_in("a", "host", "busy"), agent_in("b", "host", "idle")]),
            vec![agent_in("b", "host", "idle")]
        );
    }

    #[test]
    fn a_change_to_any_signed_field_fails_verification() {
        let good = vector_record();
        let tampered: Vec<(&str, Box<dyn Fn(&mut InstallPresence)>)> = vec![
            ("hostname", Box::new(|r| r.hostname = "area54".into())),
            ("channel", Box::new(|r| r.channel = "dev".into())),
            ("os", Box::new(|r| r.os = "linux".into())),
            ("version", Box::new(|r| r.version = "0.59.12".into())),
            ("channels_running", Box::new(|r| r.channels_running = 4)),
            ("published_at_ms", Box::new(|r| r.published_at_ms += 1)),
            ("agent kind", Box::new(|r| r.agents[0].kind = "host".into())),
            ("agent dropped", Box::new(|r| {
                r.agents.pop();
            })),
            ("instance_id", Box::new(|r| r.instance_id = "a".repeat(26))),
            ("sig", Box::new(|r| r.sig = BASE64.encode([0u8; 64]))),
        ];
        for (what, change) in tampered {
            let mut r = good.clone();
            change(&mut r);
            assert!(!r.verify(), "{what} changed but the record still verifies");
        }
    }

    #[test]
    fn another_key_cannot_claim_the_instance() {
        let mut r = vector_record();
        let other = InstallPresence::sign(&[9u8; 32], "narko", "stable", "windows", "0.59.11", 3, [], 1).unwrap();
        r.instance_public_key = other.instance_public_key;
        assert!(!r.verify(), "the id no longer names the key");
    }

    #[test]
    fn no_agents_signs_an_empty_list_field() {
        let r = InstallPresence::sign(&seed(), "narko", "stable", "", "0.59.11", 1, [], 5).unwrap();
        assert!(r.agents.is_empty());
        assert!(r.signed_material().contains(&format!("{FIELD_SEP}{FIELD_SEP}5")));
        assert!(r.verify());
    }

    #[test]
    fn agents_out_of_bounds_are_left_out_and_the_list_is_capped() {
        let agents = canonical_agents([
            agent("ok", "host"),
            agent("bad\u{1}name", "host"),
            agent("tab\tname", "host"),
            agent("", "host"),
            agent(&"x".repeat(MAX_PRESENCE_NAME_CHARS + 1), "host"),
            agent("weird", "vm"),
            agent("ok", "container"),
        ]);
        assert_eq!(agents, vec![agent("ok", "host")], "each name once, the first kept");

        let many: Vec<_> = (0..MAX_PRESENCE_AGENTS + 10).map(|i| agent(&format!("a{i:03}"), "host")).collect();
        let r = InstallPresence::sign(&seed(), "narko", "stable", "windows", "1", 1, many, 1).unwrap();
        assert_eq!(r.agents.len(), MAX_PRESENCE_AGENTS);
        assert!(r.verify());
    }

    #[test]
    fn a_top_level_field_out_of_bounds_is_not_signed() {
        let sign = |hostname: &str, channel: &str, os: &str, version: &str| {
            InstallPresence::sign(&seed(), hostname, channel, os, version, 1, [], 1)
        };
        assert!(sign("", "stable", "windows", "1").is_none());
        assert!(sign("nar\nko", "stable", "windows", "1").is_none());
        assert!(sign(&"h".repeat(MAX_PRESENCE_NAME_CHARS + 1), "stable", "windows", "1").is_none());
        assert!(sign("narko", "", "windows", "1").is_none());
        assert!(sign("narko", "stable", "Windows", "1").is_none());
        assert!(sign("narko", "stable", "windows", "").is_none());
        assert!(sign("narko", "stable", "windows", &"9".repeat(MAX_PRESENCE_VERSION_CHARS + 1)).is_none());
        assert!(InstallPresence::sign(&[1u8; 31], "narko", "stable", "windows", "1", 1, [], 1).is_none());
    }

    #[test]
    fn channels_running_is_clamped() {
        let at = |n| InstallPresence::sign(&seed(), "narko", "stable", "windows", "1", n, [], 1).unwrap().channels_running;
        assert_eq!(at(0), 1);
        assert_eq!(at(3), 3);
        assert_eq!(at(500), MAX_CHANNELS_RUNNING);
    }

    #[test]
    fn agents_out_of_canonical_order_do_not_verify() {
        let mut r = vector_record();
        r.agents.reverse();
        assert!(!r.verify());
    }
}
