// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agents of other AgentMux instances, for the Swarm's sections below this
//! instance's own tree (`docs/specs/SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md`).
//!
//! Phase 2 adds LAN hosts, from the LAN discovery peer list (mDNS and the UDP
//! route): one host per hostname, one channel per advertising instance, names
//! from each peer's `agents` list.
//!
//! Phase 1: other channels on this machine, read from the host-global shared
//! registry (`reactive::registry::list_all_shared`), the same source the
//! `/agentmux/discovery` endpoint's `host.cross_channel` uses. Names only: the
//! registry carries no status, provider or summary. LAN hosts and cloud installs
//! are later phases and arrive as more `RemoteHost`s with their own `tier`.
//!
//! Nothing here carries a credential to the frontend: the registry's `auth_key`
//! and `local_url` are dropped.

use serde::Serialize;

use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;

use crate::backend::lan_discovery::LanInstance;
use crate::backend::reactive::registry::AgentEntry;

/// A registry entry not rewritten for this long is shown as stale. The registry
/// heartbeat rewrites every live agent's entry every 20 s, so this is three
/// missed beats.
pub const STALE_AFTER_MS: u64 = 60_000;

/// Past this an entry is not shown at all. The registry's own startup sweep only
/// drops entries older than 4 h, and a crashed channel should not look present
/// for that long.
pub const HIDE_AFTER_MS: u64 = 10 * 60_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteAgent {
    pub name: String,
    /// The agent's block in its own instance. Not usable from this one; kept so a
    /// later phase can address it (the cross-channel fleet forwarder does).
    pub block_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteChannel {
    pub channel: String,
    /// Newest `updated_at` among the channel's entries, Unix ms.
    pub seen_at_ms: u64,
    /// No entry of this channel rewritten within [`STALE_AFTER_MS`].
    pub stale: bool,
    /// Sorted by name, case-insensitively.
    pub agents: Vec<RemoteAgent>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteHost {
    /// Stable id for UI state (collapse): `host:<hostname>` for this machine.
    pub host_id: String,
    pub display_name: String,
    /// How it was found: `host` (this machine), later `lan` or `cloud`.
    pub tier: &'static str,
    /// Sorted by channel name.
    pub channels: Vec<RemoteChannel>,
}

/// Response of `swarm.other-instances`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SwarmOtherInstancesResult {
    /// This machine's name and this instance's channel, for the naming rule
    /// ("name the channel only when a host has more than one").
    pub hostname: String,
    pub channel: String,
    pub hosts: Vec<RemoteHost>,
}

/// Group the shared registry's entries into this machine's other channels.
///
/// Skips this instance's own channel and URL (a stale self-entry after a crash),
/// entries past [`HIDE_AFTER_MS`], and entries with no channel (per-channel
/// registry rows, which never belong in the shared directory). An agent listed
/// twice in one channel (two files racing) appears once.
pub fn other_channels(
    entries: &[AgentEntry],
    own_channel: &str,
    own_url: &str,
    now_ms: u64,
) -> Vec<RemoteChannel> {
    use std::collections::BTreeMap;
    let mut by_channel: BTreeMap<String, (u64, BTreeMap<String, RemoteAgent>)> = BTreeMap::new();
    for e in entries {
        if e.channel.is_empty()
            || e.channel == own_channel
            || (!own_url.is_empty() && e.local_url == own_url)
        {
            continue;
        }
        if now_ms.saturating_sub(e.updated_at) > HIDE_AFTER_MS {
            continue;
        }
        let slot = by_channel.entry(e.channel.clone()).or_default();
        slot.0 = slot.0.max(e.updated_at);
        slot.1
            .entry(e.agent_id.to_lowercase())
            .or_insert_with(|| RemoteAgent {
                name: e.agent_id.clone(),
                block_id: e.block_id.clone(),
            });
    }
    by_channel
        .into_iter()
        .map(|(channel, (seen_at_ms, agents))| RemoteChannel {
            channel,
            seen_at_ms,
            stale: now_ms.saturating_sub(seen_at_ms) > STALE_AFTER_MS,
            agents: agents.into_values().collect(),
        })
        .collect()
}

fn fnv1a32(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// LAN hosts from the discovery peer list. A peer at one of this machine's own
/// addresses is another channel on this host, which the registry already lists
/// (with more detail), so it is left out here. A peer that did not advertise its
/// channel (an older build) is labelled by its port, so two of them on one host
/// stay apart.
pub fn lan_hosts(
    peers: &[LanInstance],
    own_addrs: &HashSet<IpAddr>,
    now_ms: u64,
) -> Vec<RemoteHost> {
    let mut hosts: BTreeMap<String, (String, BTreeMap<String, RemoteChannel>)> = BTreeMap::new();
    for p in peers {
        if p.address
            .parse::<IpAddr>()
            .is_ok_and(|a| own_addrs.contains(&a))
        {
            continue;
        }
        // The address is the grouping key only, never a label: the answer
        // carries no addresses (Codex P2 on #4241).
        let display = if p.hostname.trim().is_empty() {
            "unnamed host".to_string()
        } else {
            p.hostname.trim().to_string()
        };
        let channel = if p.channel.is_empty() {
            format!(":{}", p.port)
        } else {
            p.channel.clone()
        };
        // Heard from by mDNS or answered a poll, whichever is later.
        let seen_at_ms = p.last_seen.max(p.last_polled_ok).saturating_mul(1000);
        let mut agents: Vec<RemoteAgent> = p
            .agents
            .iter()
            .map(|name| RemoteAgent {
                name: name.clone(),
                block_id: String::new(),
            })
            .collect();
        agents.sort_by_key(|a| a.name.to_lowercase());
        agents.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
        // A machine is its hostname AND its address: two machines that share a
        // name (cloned VMs both called `ubuntu`) must not merge, or one would hide
        // the other's agents (Codex P2 on #4241). Two channels on one machine
        // share the address, so they still group.
        let host = hosts
            .entry(format!("{}@{}", display.to_lowercase(), p.address))
            .or_insert_with(|| (display.clone(), BTreeMap::new()));
        // Two records for the same instance (mDNS and UDP before they merge)
        // keep the fresher one.
        let keep = host
            .1
            .get(&channel)
            .is_none_or(|c| c.seen_at_ms < seen_at_ms);
        if keep {
            host.1.insert(
                channel.clone(),
                RemoteChannel {
                    channel,
                    seen_at_ms,
                    stale: now_ms.saturating_sub(seen_at_ms) > STALE_AFTER_MS,
                    agents,
                },
            );
        }
    }
    // A name shared by several machines gets a number per machine, in key
    // (address) order, so the sections can be told apart without showing an
    // address; a unique name stays bare.
    let mut name_count: BTreeMap<String, usize> = BTreeMap::new();
    for (display, _) in hosts.values() {
        *name_count.entry(display.to_lowercase()).or_default() += 1;
    }
    let mut name_seen: BTreeMap<String, usize> = BTreeMap::new();
    hosts
        .into_iter()
        .map(|(key, (display, channels))| {
            // Stable for the UI's collapse state, without putting the address
            // in the answer: a short hash of name and address.
            let host_id = format!("lan:{}#{:08x}", display.to_lowercase(), fnv1a32(&key));
            let lower = display.to_lowercase();
            let display_name = if name_count[&lower] > 1 {
                let n = name_seen.entry(lower).or_default();
                *n += 1;
                format!("{display} ({n})")
            } else {
                display
            };
            RemoteHost {
                host_id,
                display_name,
                tier: "lan",
                channels: channels.into_values().collect(),
            }
        })
        .collect()
}

/// The full answer: this machine's other channels as one host (absent when there
/// are none), then the LAN hosts.
pub fn snapshot(
    entries: &[AgentEntry],
    lan: &[LanInstance],
    own_addrs: &HashSet<IpAddr>,
    hostname: &str,
    own_channel: &str,
    own_url: &str,
    now_ms: u64,
) -> SwarmOtherInstancesResult {
    let channels = other_channels(entries, own_channel, own_url, now_ms);
    let mut hosts = Vec::new();
    if !channels.is_empty() {
        hosts.push(RemoteHost {
            host_id: format!("host:{hostname}"),
            display_name: hostname.to_string(),
            tier: "host",
            channels,
        });
    }
    hosts.extend(lan_hosts(lan, own_addrs, now_ms));
    SwarmOtherInstancesResult {
        hostname: hostname.to_string(),
        channel: own_channel.to_string(),
        hosts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    fn entry(name: &str, channel: &str, age_ms: u64) -> AgentEntry {
        AgentEntry {
            agent_id: name.to_string(),
            local_url: format!("http://127.0.0.1:{}", 29700 + channel.len()),
            block_id: format!("blk-{name}"),
            pid: 1,
            updated_at: NOW - age_ms,
            auth_key: "secret".to_string(),
            channel: channel.to_string(),
            registration_nonce: 0,
            jekt_public_key: String::new(),
        }
    }

    #[test]
    fn groups_other_channels_and_leaves_this_one_out() {
        let entries = [
            entry("Korp", "dev-fix-lan", 5_000),
            entry("Loap", "dev-fix-lan", 1_000),
            entry("AgentX", "stable", 0),
        ];
        let chans = other_channels(&entries, "stable", "", NOW);
        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].channel, "dev-fix-lan");
        assert_eq!(
            chans[0]
                .agents
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["Korp", "Loap"]
        );
        assert_eq!(chans[0].seen_at_ms, NOW - 1_000, "newest entry");
        assert!(!chans[0].stale);
    }

    #[test]
    fn a_stale_self_entry_by_url_is_left_out() {
        let mut me = entry("AgentX", "old-name", 0);
        me.local_url = "http://127.0.0.1:29706".to_string();
        assert!(other_channels(&[me], "stable", "http://127.0.0.1:29706", NOW).is_empty());
    }

    #[test]
    fn freshness_marks_stale_then_hides() {
        let stale = other_channels(
            &[entry("Korp", "dev", STALE_AFTER_MS + 1)],
            "stable",
            "",
            NOW,
        );
        assert!(stale[0].stale);
        assert!(other_channels(
            &[entry("Korp", "dev", HIDE_AFTER_MS + 1)],
            "stable",
            "",
            NOW
        )
        .is_empty());
    }

    #[test]
    fn an_agent_listed_twice_in_a_channel_appears_once() {
        let chans = other_channels(
            &[entry("Korp", "dev", 0), entry("korp", "dev", 10)],
            "stable",
            "",
            NOW,
        );
        assert_eq!(chans[0].agents.len(), 1);
    }

    #[test]
    fn entries_without_a_channel_are_not_another_channel() {
        assert!(other_channels(&[entry("Korp", "", 0)], "stable", "", NOW).is_empty());
    }

    #[test]
    fn the_snapshot_names_this_machine_and_carries_no_credentials() {
        let snap = snapshot(
            &[entry("Korp", "dev", 0)],
            &[],
            &HashSet::new(),
            "narko",
            "stable",
            "",
            NOW,
        );
        assert_eq!(
            (snap.hostname.as_str(), snap.channel.as_str()),
            ("narko", "stable")
        );
        assert_eq!(snap.hosts.len(), 1);
        assert_eq!(
            (snap.hosts[0].display_name.as_str(), snap.hosts[0].tier),
            ("narko", "host")
        );
        let json = serde_json::to_string(&snap).unwrap();
        assert!(
            !json.contains("secret") && !json.contains("local_url") && !json.contains("127.0.0.1"),
            "{json}"
        );
        // No other channel: no host section at all.
        assert!(
            snapshot(&[], &[], &HashSet::new(), "narko", "stable", "", NOW)
                .hosts
                .is_empty()
        );
    }

    fn peer(
        host: &str,
        channel: &str,
        addr: &str,
        port: u16,
        agents: &[&str],
        age_secs: u64,
    ) -> LanInstance {
        LanInstance {
            instance_id: format!("{host}-{port}"),
            hostname: host.to_string(),
            version: "0.59.5".to_string(),
            channel: channel.to_string(),
            address: addr.to_string(),
            port,
            auth_key: "lan-secret".to_string(),
            agents: agents.iter().map(|a| a.to_string()).collect(),
            first_seen: 0,
            last_seen: NOW / 1000 - age_secs,
            last_polled_ok: 0,
            other_ttl_secs: 4500,
        }
    }

    #[test]
    fn lan_peers_group_into_hosts_with_a_channel_each() {
        let peers = [
            peer("Area54", "stable", "192.168.1.26", 29700, &["Manoz"], 5),
            peer(
                "starpower",
                "stable",
                "192.168.1.195",
                29700,
                &["Opaz", "korp"],
                5,
            ),
            peer("starpower", "dev", "192.168.1.195", 29702, &["Loap"], 5),
        ];
        let hosts = lan_hosts(&peers, &HashSet::new(), NOW);
        let shape: Vec<(&str, &str, usize)> = hosts
            .iter()
            .map(|h| (h.display_name.as_str(), h.tier, h.channels.len()))
            .collect();
        assert_eq!(shape, [("Area54", "lan", 1), ("starpower", "lan", 2)]);
        let chans: Vec<&str> = hosts[1]
            .channels
            .iter()
            .map(|c| c.channel.as_str())
            .collect();
        assert_eq!(chans, ["dev", "stable"]);
        let names: Vec<&str> = hosts[1].channels[1]
            .agents
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["korp", "Opaz"]);
    }

    /// Codex P2 on #4241: cloned VMs that share a hostname are two machines.
    #[test]
    fn two_machines_with_the_same_hostname_stay_apart() {
        let peers = [
            peer("ubuntu", "stable", "192.168.1.40", 29700, &["A"], 5),
            peer("ubuntu", "stable", "192.168.1.41", 29700, &["B"], 5),
        ];
        let hosts = lan_hosts(&peers, &HashSet::new(), NOW);
        assert_eq!(hosts.len(), 2, "neither hides the other");
        let names: Vec<&str> = hosts.iter().map(|h| h.display_name.as_str()).collect();
        assert_eq!(names, ["ubuntu (1)", "ubuntu (2)"]);
        // And still no address anywhere in the answer (Codex P2 on #4241).
        let json = serde_json::to_string(&hosts).unwrap();
        assert!(!json.contains("192.168.1.4"), "{json}");
        assert_ne!(hosts[0].host_id, hosts[1].host_id);
        let agents: Vec<&str> = hosts
            .iter()
            .map(|h| h.channels[0].agents[0].name.as_str())
            .collect();
        assert_eq!(agents, ["A", "B"]);
    }

    #[test]
    fn a_lan_peer_at_one_of_our_own_addresses_is_left_to_the_registry() {
        let own: HashSet<IpAddr> = ["192.168.1.230".parse().unwrap()].into_iter().collect();
        let peers = [peer("narko", "dev", "192.168.1.230", 29702, &["Loap"], 5)];
        assert!(lan_hosts(&peers, &own, NOW).is_empty());
    }

    #[test]
    fn a_peer_without_a_channel_is_labelled_by_its_port() {
        let peers = [
            peer("old", "", "192.168.1.9", 29700, &["A"], 5),
            peer("old", "", "192.168.1.9", 29701, &["B"], 5),
        ];
        let hosts = lan_hosts(&peers, &HashSet::new(), NOW);
        let chans: Vec<&str> = hosts[0]
            .channels
            .iter()
            .map(|c| c.channel.as_str())
            .collect();
        assert_eq!(chans, [":29700", ":29701"]);
    }

    #[test]
    fn a_quiet_lan_peer_is_stale_and_no_lan_key_or_address_leaves() {
        let quiet = [peer(
            "Area54",
            "stable",
            "192.168.1.26",
            29700,
            &["Manoz"],
            90,
        )];
        assert!(lan_hosts(&quiet, &HashSet::new(), NOW)[0].channels[0].stale);
        let fresh = [peer(
            "Area54",
            "stable",
            "192.168.1.26",
            29700,
            &["Manoz"],
            5,
        )];
        let snap = snapshot(&[], &fresh, &HashSet::new(), "narko", "stable", "", NOW);
        let json = serde_json::to_string(&snap).unwrap();
        assert!(
            !json.contains("lan-secret") && !json.contains("192.168.1.26"),
            "{json}"
        );
    }

    /// Codex P1 on #4241: an mDNS-only peer is re-resolved tens of minutes apart,
    /// so a peer that answers the agent-name poll is live, whatever `last_seen` says.
    #[test]
    fn a_peer_that_answers_polls_is_not_stale() {
        let mut p = peer("Area54", "stable", "192.168.1.26", 29700, &["Manoz"], 1800);
        assert!(lan_hosts(std::slice::from_ref(&p), &HashSet::new(), NOW)[0].channels[0].stale);
        p.last_polled_ok = NOW / 1000 - 10;
        assert!(!lan_hosts(&[p], &HashSet::new(), NOW)[0].channels[0].stale);
    }

    #[test]
    fn a_host_with_no_name_is_not_labelled_by_its_address() {
        let hosts = lan_hosts(
            &[peer("", "stable", "192.168.1.77", 29700, &["A"], 5)],
            &HashSet::new(),
            NOW,
        );
        assert_eq!(hosts[0].display_name, "unnamed host");
    }
}
