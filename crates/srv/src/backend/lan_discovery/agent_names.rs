// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading an instance's `/agentmux/reactive/agent-names` answer: the names a
//! LAN peer's poll (and the Swarm's ask of this machine's other channels) list,
//! and each listed agent's state and summary line, for the Swarm's remote rows
//! (SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md §3.4 Phase 4).

use super::*;
use std::collections::HashSet;

/// This instance's `/agentmux/reactive/agent-names` answer. Readable with the
/// LAN key, so public to the local network: each field needed the owner's
/// decision.
/// - `agents`, enumerating names (2026-09-08): a `lan_key` holder could
///   already confirm a specific name via `/agentmux/reactive/agent?id=…`.
/// - `agent_kinds` (`{name: "host" | "container"}`, 2026-10-06, agentmux-mobile's
///   SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06 §5.2, §11).
/// - `agent_status` (`{name: {state, since_ms}}`) and `now_ms`, decision D1 of
///   2026-10-07 (agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07
///   §3.4, §11), also on the fleet feed this route is the fallback for.
/// - `agent_lines` (`{name: line}`, 2026-10-13): content, each agent's Swarm
///   summary line (`backend::agent_line`); absent with `swarm:sharesummaries` off.
/// - `agent_providers` (`{name: provider id}`, decision D1 of 2026-10-17,
///   SPEC_SWARM_REFINEMENTS_2026_10_17 §3, §7): the Swarm's provider icon on
///   remote rows. Only an id this build knows (`providers::resolve_provider_alias`).
///
/// An agent with no kind, state, line or provider is left out of that map.
pub(crate) fn agent_names_body(
    mstore: &crate::backend::storage::store::Store,
    settings: &crate::backend::wconfig::SettingsType,
    agents: &[crate::backend::reactive::types::AgentRegistration],
) -> serde_json::Value {
    let pairs = || agents.iter().map(|a| (a.agent_id.as_str(), a.block_id.as_str()));
    let kinds = crate::backend::fleet_source::kinds_by_name(pairs(), |block_id| {
        crate::backend::operator_config_seed::agent_kind_of_block(mstore, block_id)
    });
    let status: BTreeMap<&str, _> = pairs()
        .filter_map(|(name, block_id)| crate::backend::agent_state::agent_status_of_block(mstore, block_id).map(|s| (name, s)))
        .collect();
    let providers: BTreeMap<&str, &str> = pairs()
        .filter_map(|(name, block_id)| {
            let block = mstore.get::<crate::backend::obj::Block>(block_id).ok()??;
            let raw = crate::backend::obj::meta_get_string(&block.meta, "agentProvider", "");
            let id = crate::backend::providers::resolve_provider_alias(&raw);
            (!id.is_empty()).then_some((name, id))
        })
        .collect();
    let mut body = json!({
        "agents": agents.iter().map(|a| a.agent_id.as_str()).collect::<Vec<_>>(),
        "agent_kinds": kinds,
        "now_ms": agentmux_common::time::now_ms_u64(),
        "agent_status": status,
        "agent_providers": providers,
    });
    if let Some(lines) = crate::backend::agent_line::lines_by_name(mstore, settings, pairs()) {
        body["agent_lines"] = json!(lines);
    }
    body
}

/// What an instance's `/agentmux/reactive/agent-names` answer says about its
/// agents.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PeerAgents {
    pub names: Vec<String>,
    /// Lower-cased name → state, for listed names only.
    pub status: BTreeMap<String, AgentState>,
    /// Lower-cased name → summary line (`backend::agent_line`), for listed
    /// names only, each cleaned again here.
    pub lines: BTreeMap<String, String>,
    /// Lower-cased name → provider id, for listed names only, and only an id
    /// this build knows (`providers::resolve_provider_alias`), so a peer can't
    /// put free text in a row (SPEC_SWARM_REFINEMENTS_2026_10_17.md §3).
    pub providers: BTreeMap<String, String>,
}

/// Reads an `agent-names` body: the names, capped as above, and each listed
/// agent's state from `agent_status` (`{name: {state, since_ms}}`). A state for
/// a name the body doesn't list, or one this build doesn't know, is dropped,
/// so the map is never larger than the name list. The same goes for
/// `agent_lines` (`{name: line}`). A body from a build before `agent_status`
/// gives names only.
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
    let lines = value
        .get("agent_lines")
        .and_then(|s| s.as_object())
        .map(|map| {
            map.iter()
                .filter(|(name, _)| listed.contains(&name.to_lowercase()))
                .filter_map(|(name, line)| {
                    let line = crate::backend::agent_line::clean_line(line.as_str()?)?;
                    Some((name.to_lowercase(), line))
                })
                .collect()
        })
        .unwrap_or_default();
    let providers = value
        .get("agent_providers")
        .and_then(|s| s.as_object())
        .map(|map| {
            map.iter()
                .filter(|(name, _)| listed.contains(&name.to_lowercase()))
                .filter_map(|(name, provider)| {
                    let id = crate::backend::providers::resolve_provider_alias(provider.as_str()?);
                    (!id.is_empty()).then(|| (name.to_lowercase(), id.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    PeerAgents { names, status, lines, providers }
}

/// Applies one poll of a peer's `agent-names` route to its entry, and says
/// whether what the frontend shows changed. A failed poll keeps the last-known
/// names, since a flapping list is worse than a slightly old one, but clears
/// the states and lines: mDNS can keep the peer looking fresh long after it
/// stopped answering, and its last "working" would otherwise show indefinitely
/// (#4703).
pub(super) fn apply_agent_names_poll(
    entry: &mut LanInstance,
    peer: Option<PeerAgents>,
    now_secs: u64,
) -> bool {
    let Some(peer) = peer else {
        let changed = !entry.agent_status.is_empty() || !entry.agent_lines.is_empty();
        entry.agent_status.clear();
        entry.agent_lines.clear();
        return changed;
    };
    entry.last_polled_ok = now_secs;
    let changed = entry.agents != peer.names
        || entry.agent_status != peer.status
        || entry.agent_lines != peer.lines
        || entry.agent_providers != peer.providers;
    entry.agents = peer.names;
    entry.agent_status = peer.status;
    entry.agent_lines = peer.lines;
    entry.agent_providers = peer.providers;
    changed
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
    fn agent_lines_are_kept_for_listed_agents_as_one_bounded_line() {
        let body = serde_json::json!({
            "agents": ["Manoz", "Opaz", "Loap"],
            "agent_lines": {
                "Manoz": "Fixing the\nlogin race",
                "opaz": "x".repeat(500),
                "Loap": 7,
                "Ghost": "Not listed",
            },
        });
        let peer = parse_agent_names_body(&body);
        assert_eq!(peer.lines.get("manoz").map(String::as_str), Some("Fixing the login race"));
        assert_eq!(
            peer.lines.get("opaz").map(|l| l.chars().count()),
            Some(crate::backend::agent_line::MAX_LINE_CHARS)
        );
        assert_eq!(peer.lines.len(), 2, "{:?}", peer.lines);
    }

    /// The Swarm's provider icon on remote rows (SPEC_SWARM_REFINEMENTS_2026_10_17.md §3).
    #[test]
    fn agent_providers_keep_only_known_ids_for_listed_agents() {
        let body = serde_json::json!({
            "agents": ["Manoz", "Opaz", "Loap", "Korp"],
            "agent_providers": {
                "Manoz": "claude",
                "opaz": "codex-cli",
                "Loap": "<script>",
                "Korp": 7,
                "Ghost": "gemini",
            },
        });
        let peer = parse_agent_names_body(&body);
        let got: Vec<(&str, &str)> = peer.providers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        // A legacy alias resolves to its id; free text, a non-string and a name
        // the body doesn't list are dropped.
        assert_eq!(got, [("manoz", "claude"), ("opaz", "codex")]);
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
        let mut inst = instance();
        inst.agent_status.insert("manoz".into(), AgentState::Idle);
        let json = serde_json::to_value(&inst).unwrap();
        assert_eq!(json["agent_status"], serde_json::json!({"manoz": "idle"}));
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("agent_status");
        let back: LanInstance = serde_json::from_value(old).unwrap();
        assert!(back.agent_status.is_empty());
    }

    /// The same JSON goes to the frontend, `/api/lan-instances` and
    /// `/agentmux/discovery` (which container agents may call); a peer's
    /// `lan_key` is in none of them.
    #[test]
    fn lan_instance_json_never_carries_the_peer_key() {
        let json = serde_json::to_value(instance()).unwrap();
        assert!(json.get("auth_key").is_none(), "{json}");
        let back: LanInstance = serde_json::from_value(json).unwrap();
        assert!(back.auth_key.is_empty());
    }

    #[test]
    fn a_poll_that_answers_replaces_names_and_states() {
        let mut inst = instance();
        let peer = parse_agent_names_body(&serde_json::json!({
            "agents": ["Manoz", "Opaz"],
            "agent_status": {"Manoz": {"state": "working", "since_ms": 1}},
        }));
        assert!(apply_agent_names_poll(&mut inst, Some(peer.clone()), 77));
        assert_eq!(inst.agents, ["Manoz", "Opaz"]);
        assert_eq!(inst.agent_status.get("manoz"), Some(&AgentState::Working));
        assert_eq!(inst.last_polled_ok, 77);
        // The same answer again changes nothing the frontend shows.
        assert!(!apply_agent_names_poll(&mut inst, Some(peer), 78));
        // A new line alone is a change too.
        let peer = parse_agent_names_body(&serde_json::json!({
            "agents": ["Manoz", "Opaz"],
            "agent_status": {"Manoz": {"state": "working", "since_ms": 1}},
            "agent_lines": {"Manoz": "Fixing the login race"},
        }));
        assert!(apply_agent_names_poll(&mut inst, Some(peer), 79));
        assert_eq!(inst.agent_lines.get("manoz").map(String::as_str), Some("Fixing the login race"));
    }

    /// A peer still advertising over mDNS but no longer answering polls must
    /// not keep showing its last state (#4703).
    #[test]
    fn a_failed_poll_keeps_the_names_and_clears_the_states() {
        let mut inst = instance();
        inst.agent_status
            .insert("manoz".into(), AgentState::Working);
        inst.agent_lines.insert("manoz".into(), "Fixing the login race".into());
        inst.last_polled_ok = 5;
        assert!(apply_agent_names_poll(&mut inst, None, 99));
        assert_eq!(inst.agents, ["Manoz"]);
        assert!(inst.agent_status.is_empty());
        assert!(inst.agent_lines.is_empty());
        assert_eq!(
            inst.last_polled_ok, 5,
            "a failed poll is not a successful one"
        );
        // Failing again, with nothing left to clear, changes nothing.
        assert!(!apply_agent_names_poll(&mut inst, None, 100));
    }

    fn instance() -> LanInstance {
        LanInstance {
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
            agent_lines: Default::default(),
            agent_providers: Default::default(),
            first_seen: 0,
            last_seen: 0,
            last_polled_ok: 0,
            other_ttl_secs: 4500,
        }
    }
}
