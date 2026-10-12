// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading an instance's `/agentmux/reactive/agent-names` answer: the names a
//! LAN peer's poll (and the Swarm's ask of this machine's other channels) list,
//! and each listed agent's state, for the Swarm's status chip on remote rows
//! (SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md §3.4 Phase 4).

use super::*;
use std::collections::HashSet;

/// What an instance's `/agentmux/reactive/agent-names` answer says about its
/// agents.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PeerAgents {
    pub names: Vec<String>,
    /// Lower-cased name → state, for listed names only.
    pub status: BTreeMap<String, AgentState>,
}

/// Reads an `agent-names` body: the names, capped as above, and each listed
/// agent's state from `agent_status` (`{name: {state, since_ms}}`). A state for
/// a name the body doesn't list, or one this build doesn't know, is dropped,
/// so the map is never larger than the name list. A body from a build before
/// `agent_status` gives names only.
pub(crate) fn parse_agent_names_body(value: &serde_json::Value) -> PeerAgents {
    let names: Vec<String> = value
        .get("agents")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|n| n.as_str())
                .filter(|n| n.len() <= LAN_AGENT_NAMES_MAX_NAME_LEN)
                .take(LAN_AGENT_NAMES_MAX_COUNT)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let listed: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let status = value
        .get("agent_status")
        .and_then(|s| s.as_object())
        .map(|map| {
            map.iter()
                .filter(|(name, _)| listed.contains(&name.to_lowercase()))
                .filter_map(|(name, s)| {
                    let state =
                        serde_json::from_value::<AgentState>(s.get("state")?.clone()).ok()?;
                    Some((name.to_lowercase(), state))
                })
                .collect()
        })
        .unwrap_or_default();
    PeerAgents { names, status }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Swarm's status chip on remote rows
    /// (SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md §3.4 Phase 4).
    #[test]
    fn agent_status_is_kept_for_listed_agents_with_a_known_state() {
        let body = serde_json::json!({
            "agents": ["Manoz", "Opaz", "Loap", "Korp"],
            "now_ms": 5_000,
            "agent_status": {
                "Manoz": {"state": "working", "since_ms": 1_000},
                "opaz": {"state": "waiting", "since_ms": 2_000},
                "Loap": {"state": "dreaming", "since_ms": 2_000},
                "Korp": {"since_ms": 2_000},
                "Ghost": {"state": "idle", "since_ms": 2_000},
            },
        });
        let peer = parse_agent_names_body(&body);
        assert_eq!(peer.names, ["Manoz", "Opaz", "Loap", "Korp"]);
        let status: Vec<(&str, AgentState)> = peer
            .status
            .iter()
            .map(|(k, v): (&String, &AgentState)| (k.as_str(), *v))
            .collect();
        // Keyed by lower-cased name; an unknown state, a missing state and a
        // name the body doesn't list are all dropped.
        assert_eq!(
            status,
            [
                ("manoz", AgentState::Working),
                ("opaz", AgentState::Waiting)
            ]
        );
    }

    #[test]
    fn a_body_from_before_agent_status_gives_names_only() {
        let peer = parse_agent_names_body(&serde_json::json!({"agents": ["Manoz"]}));
        assert_eq!(peer.names, ["Manoz"]);
        assert!(peer.status.is_empty());
        let peer = parse_agent_names_body(
            &serde_json::json!({"agents": ["Manoz"], "agent_status": "bogus"}),
        );
        assert!(peer.status.is_empty());
    }

    /// The `laninstances` event carries the state to the frontend, and a record
    /// from before the field reads as none.
    #[test]
    fn lan_instance_agent_status_round_trips_and_defaults_to_empty() {
        let mut inst = LanInstance {
            instance_id: "area54-29700".into(),
            hostname: "Area54".into(),
            version: "0".into(),
            channel: "stable".into(),
            os: String::new(),
            address: "10.0.0.26".into(),
            port: 29700,
            auth_key: "k".into(),
            agents: vec!["Manoz".into()],
            agent_status: Default::default(),
            first_seen: 0,
            last_seen: 0,
            last_polled_ok: 0,
            other_ttl_secs: 4500,
        };
        inst.agent_status.insert("manoz".into(), AgentState::Idle);
        let json = serde_json::to_value(&inst).unwrap();
        assert_eq!(json["agent_status"], serde_json::json!({"manoz": "idle"}));
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("agent_status");
        let back: LanInstance = serde_json::from_value(old).unwrap();
        assert!(back.agent_status.is_empty());
    }
}
