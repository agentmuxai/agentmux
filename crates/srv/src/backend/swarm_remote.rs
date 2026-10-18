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
//! Phase 4 adds each agent's state (working, waiting, idle, ...) where its
//! instance reports one: a LAN peer's from the `agent-names` poll the discovery
//! loop already makes, another channel's from the same route asked over
//! loopback ([`channel_statuses`]). With it, by the owner's decision of
//! 2026-10-13, each agent's summary line where its instance shares one
//! (`backend::agent_line`).
//!
//! Nothing here carries a credential to the frontend: the registry's `auth_key`
//! and `local_url` are dropped.

use serde::Serialize;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;

use crate::backend::agent_state::AgentState;
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
    /// What its instance last said it is doing; absent when that instance
    /// gave no state (a build from before it did, or a terminal pane).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentState>,
    /// Its summary line as its instance last shared it; absent when that
    /// instance shared none (an older build, or `swarm:sharesummaries` off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// Its provider id (`claude`, `codex`, ...) as its instance reported it,
    /// a known one only; absent from an older build. Display only: the Swarm's
    /// provider icon (SPEC_SWARM_REFINEMENTS_2026_10_17.md §3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// What one channel of this machine said about its agents, by lower-cased name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelReport {
    pub status: BTreeMap<String, AgentState>,
    pub lines: BTreeMap<String, String>,
    pub providers: BTreeMap<String, String>,
}

/// Each channel's report, by channel.
pub type ChannelStatuses = HashMap<String, ChannelReport>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteChannel {
    pub channel: String,
    /// Newest `updated_at` among the channel's entries, Unix ms.
    pub seen_at_ms: u64,
    /// No entry of this channel rewritten within [`STALE_AFTER_MS`].
    pub stale: bool,
    /// The instance's operating system (`windows`, `macos`, `linux`), for the
    /// platform tag. Empty when unknown: a peer from before it was advertised.
    /// Self-reported and display only (`host_os`).
    pub os: String,
    /// Sorted by name, case-insensitively.
    pub agents: Vec<RemoteAgent>,
    /// How to reach a LAN peer's instance, for a Swarm broadcast to one of its
    /// agents (SPEC_SWARM_REFINEMENTS_2026_10_17.md §4.2). Never serialized:
    /// the answer to the frontend carries no address or credential.
    #[serde(skip)]
    pub route: Option<PeerRoute>,
}

/// A LAN peer's address and the broadcast key it advertised.
#[derive(Clone, PartialEq, Eq)]
pub struct PeerRoute {
    pub url: String,
    pub auth_key: String,
}

// By hand, so the key never reaches a log line.
impl std::fmt::Debug for PeerRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerRoute").field("url", &self.url).field("auth_key", &"<redacted>").finish()
    }
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
    statuses: &ChannelStatuses,
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
            .or_insert_with(|| {
                let report = statuses.get(&e.channel);
                let key = e.agent_id.to_lowercase();
                RemoteAgent {
                    name: e.agent_id.clone(),
                    block_id: e.block_id.clone(),
                    status: report.and_then(|r| r.status.get(&key)).copied(),
                    line: report.and_then(|r| r.lines.get(&key)).cloned(),
                    provider: report.and_then(|r| r.providers.get(&key)).cloned(),
                }
            });
    }
    by_channel
        .into_iter()
        .map(|(channel, (seen_at_ms, agents))| RemoteChannel {
            route: None,
            channel,
            seen_at_ms,
            stale: now_ms.saturating_sub(seen_at_ms) > STALE_AFTER_MS,
            // Every channel on this machine runs on its operating system.
            os: crate::backend::host_os::local_os(),
            agents: agents.into_values().collect(),
        })
        .collect()
}

pub(crate) fn fnv1a32(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// A host's instances as channels, with a channel name that appears more than
/// once suffixed by its port ("stable :29702"), so the sections stay apart.
fn label_repeated_channels(by_instance: BTreeMap<String, RemoteChannel>) -> Vec<RemoteChannel> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for c in by_instance.values() {
        *count.entry(c.channel.clone()).or_default() += 1;
    }
    by_instance
        .into_iter()
        .map(|(key, mut c)| {
            if count[&c.channel] > 1 {
                if let Some((_, port)) = key.rsplit_once('@') {
                    c.channel = format!("{} :{port}", c.channel);
                }
            }
            c
        })
        .collect()
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
                status: p.agent_status.get(&name.to_lowercase()).copied(),
                line: p.agent_lines.get(&name.to_lowercase()).cloned(),
                provider: p.agent_providers.get(&name.to_lowercase()).cloned(),
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
        // One entry per instance, which on a machine is its port: two records
        // for the same instance (mDNS and UDP before they merge) keep the fresher
        // one, while two instances on one channel (an installed and a portable
        // build, say) both stay (Codex P2 on #4241).
        let instance_key = format!("{channel}@{}", p.port);
        let keep = host
            .1
            .get(&instance_key)
            .is_none_or(|c| c.seen_at_ms < seen_at_ms);
        if keep {
            host.1.insert(
                instance_key,
                RemoteChannel {
                    route: Some(PeerRoute { url: format!("http://{}:{}", p.address, p.port), auth_key: p.auth_key.clone() }),
                    channel,
                    seen_at_ms,
                    stale: now_ms.saturating_sub(seen_at_ms) > STALE_AFTER_MS,
                    os: p.os.clone(),
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
                channels: label_repeated_channels(channels),
            }
        })
        .collect()
}

/// How long another channel on this machine gets to say what its agents are
/// doing. It is loopback, so one that takes longer is hung, and the Swarm's poll
/// shouldn't wait on it: its rows show names only until it answers.
const CHANNEL_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// One instance to ask for its agents' states: `(channel, local_url, auth_key)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelStatusTarget {
    pub channel: String,
    pub local_url: String,
    pub auth_key: String,
}

fn is_loopback_url(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

/// The instances whose agents [`other_channels`] lists and that are worth
/// asking: one per `local_url`, loopback only (as the cross-channel fleet
/// forwarder requires), never this instance, and not a stale one, whose rows
/// are dimmed and show no state.
///
/// Each instance is asked with its freshest entry's key: an instance that
/// restarted on the same URL has a new per-launch key, while its older entries
/// keep the old one until they are rewritten or age out (#4703).
pub fn channel_status_targets(
    entries: &[AgentEntry],
    own_channel: &str,
    own_url: &str,
    now_ms: u64,
) -> Vec<ChannelStatusTarget> {
    let mut by_url: BTreeMap<&str, &AgentEntry> = BTreeMap::new();
    for e in entries {
        if e.channel.is_empty()
            || e.channel == own_channel
            || e.local_url == own_url
            || !is_loopback_url(&e.local_url)
            || now_ms.saturating_sub(e.updated_at) > STALE_AFTER_MS
        {
            continue;
        }
        let slot = by_url.entry(&e.local_url).or_insert(e);
        if e.updated_at > slot.updated_at {
            *slot = e;
        }
    }
    by_url
        .into_values()
        .map(|e| ChannelStatusTarget {
            channel: e.channel.clone(),
            local_url: e.local_url.clone(),
            auth_key: e.auth_key.clone(),
        })
        .collect()
}

/// Asks each target's `agent-names` route, concurrently, for its agents'
/// states and lines. A target that fails, times out or answers badly
/// contributes nothing.
pub async fn channel_statuses(
    http: &reqwest::Client,
    targets: &[ChannelStatusTarget],
) -> ChannelStatuses {
    let answers = futures_util::future::join_all(targets.iter().map(|t| async move {
        let resp = http
            .get(format!("{}/agentmux/reactive/agent-names", t.local_url))
            .header(agentmux_common::AUTH_KEY_HEADER, &t.auth_key)
            .timeout(CHANNEL_STATUS_TIMEOUT)
            .send()
            .await
            .ok()
            .filter(|r| r.status().is_success())?;
        let body = crate::backend::lan_discovery::LanDiscovery::read_body_capped(
            resp,
            crate::backend::lan_discovery::LAN_AGENT_NAMES_MAX_RESPONSE_BYTES,
        )
        .await
        .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
        Some((
            t.channel.clone(),
            crate::backend::lan_discovery::parse_agent_names_body(&value),
        ))
    }))
    .await;
    let mut out = ChannelStatuses::new();
    for (channel, peer) in answers.into_iter().flatten() {
        let report = out.entry(channel).or_default();
        report.status.extend(peer.status);
        report.lines.extend(peer.lines);
        report.providers.extend(peer.providers);
    }
    out
}

/// The full answer: this machine's other channels as one host (absent when there
/// are none), then the LAN hosts.
pub fn snapshot(
    entries: &[AgentEntry],
    statuses: &ChannelStatuses,
    lan: &[LanInstance],
    own_addrs: &HashSet<IpAddr>,
    hostname: &str,
    own_channel: &str,
    own_url: &str,
    now_ms: u64,
) -> SwarmOtherInstancesResult {
    let channels = other_channels(entries, statuses, own_channel, own_url, now_ms);
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

/// Each agent's provider id as this machine's agent definitions record it, by
/// the `uid` its registry entry carries; a known id only.
pub type DefinedProviders = HashMap<String, String>;

/// The definitions' providers for `entries`, read from the host-global
/// definition store. Blocking: it reads one file per agent.
pub fn defined_providers(entries: &[AgentEntry]) -> DefinedProviders {
    let Some(store) = crate::registry::resolve_shared_definitions_dir()
        .and_then(|dir| crate::registry::DefinitionStore::open(dir).ok())
    else {
        return DefinedProviders::new();
    };
    defined_providers_from(entries, |uid| store.get(uid).ok().flatten().map(|r| r.data.provider))
}

fn defined_providers_from(entries: &[AgentEntry], lookup: impl Fn(&str) -> Option<String>) -> DefinedProviders {
    let mut out = DefinedProviders::new();
    for e in entries {
        if e.uid.is_empty() || out.contains_key(&e.uid) {
            continue;
        }
        let Some(raw) = lookup(&e.uid) else { continue };
        let id = crate::backend::providers::resolve_provider_alias(raw.trim());
        if !id.is_empty() {
            out.insert(e.uid.clone(), id.to_string());
        }
    }
    out
}

/// Give this machine's other channels' agents the provider their definition
/// records, where their instance reported none: an instance from before it
/// reported providers (0.59.30 and older) still shows its agents' icons
/// (REPORT_SWARM_REMOTE_PROVIDER_ICON_RCA_2026_10_18.md §6.1). A provider the
/// instance reported wins: it describes the live pane. LAN hosts are left as
/// they are: their definitions aren't on this machine.
pub fn fill_defined_providers(result: &mut SwarmOtherInstancesResult, entries: &[AgentEntry], defined: &DefinedProviders) {
    let uid_of: HashMap<(&str, String), &str> = entries
        .iter()
        .filter(|e| !e.uid.is_empty())
        .map(|e| ((e.channel.as_str(), e.agent_id.to_lowercase()), e.uid.as_str()))
        .collect();
    for host in result.hosts.iter_mut().filter(|h| h.tier == "host") {
        for ch in &mut host.channels {
            let channel = ch.channel.clone();
            for a in ch.agents.iter_mut().filter(|a| a.provider.is_none()) {
                a.provider = uid_of
                    .get(&(channel.as_str(), a.name.to_lowercase()))
                    .and_then(|uid| defined.get(*uid))
                    .cloned();
            }
        }
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
            uid: String::new(),
            uid_public_key: String::new(),
            jekt_features: Vec::new(),
        }
    }

    #[test]
    fn groups_other_channels_and_leaves_this_one_out() {
        let entries = [
            entry("Korp", "dev-fix-lan", 5_000),
            entry("Loap", "dev-fix-lan", 1_000),
            entry("AgentX", "stable", 0),
        ];
        let chans = other_channels(&entries, &ChannelStatuses::new(), "stable", "", NOW);
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
        assert!(other_channels(
            &[me],
            &ChannelStatuses::new(),
            "stable",
            "http://127.0.0.1:29706",
            NOW
        )
        .is_empty());
    }

    #[test]
    fn freshness_marks_stale_then_hides() {
        let stale = other_channels(
            &[entry("Korp", "dev", STALE_AFTER_MS + 1)],
            &ChannelStatuses::new(),
            "stable",
            "",
            NOW,
        );
        assert!(stale[0].stale);
        assert!(other_channels(
            &[entry("Korp", "dev", HIDE_AFTER_MS + 1)],
            &ChannelStatuses::new(),
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
            &ChannelStatuses::new(),
            "stable",
            "",
            NOW,
        );
        assert_eq!(chans[0].agents.len(), 1);
    }

    #[test]
    fn entries_without_a_channel_are_not_another_channel() {
        assert!(other_channels(
            &[entry("Korp", "", 0)],
            &ChannelStatuses::new(),
            "stable",
            "",
            NOW
        )
        .is_empty());
    }

    #[test]
    fn the_snapshot_names_this_machine_and_carries_no_credentials() {
        let snap = snapshot(
            &[entry("Korp", "dev", 0)],
            &ChannelStatuses::new(),
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
        assert!(snapshot(
            &[],
            &ChannelStatuses::new(),
            &[],
            &HashSet::new(),
            "narko",
            "stable",
            "",
            NOW
        )
        .hosts
        .is_empty());
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
            os: String::new(),
            address: addr.to_string(),
            port,
            auth_key: "lan-secret".to_string(),
            agents: agents.iter().map(|a| a.to_string()).collect(),
            agent_status: Default::default(),
            agent_lines: Default::default(),
            agent_providers: Default::default(),
            first_seen: 0,
            last_seen: NOW / 1000 - age_secs,
            last_polled_ok: 0,
            other_ttl_secs: 4500,
        }
    }

    #[test]
    fn lan_peers_group_into_hosts_with_a_channel_each() {
        let peers = [
            peer("Area54", "stable", "10.0.0.26", 29700, &["Manoz"], 5),
            peer(
                "starpower",
                "stable",
                "10.0.0.195",
                29700,
                &["Opaz", "korp"],
                5,
            ),
            peer("starpower", "dev", "10.0.0.195", 29702, &["Loap"], 5),
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
            peer("ubuntu", "stable", "10.0.0.40", 29700, &["A"], 5),
            peer("ubuntu", "stable", "10.0.0.41", 29700, &["B"], 5),
        ];
        let hosts = lan_hosts(&peers, &HashSet::new(), NOW);
        assert_eq!(hosts.len(), 2, "neither hides the other");
        let names: Vec<&str> = hosts.iter().map(|h| h.display_name.as_str()).collect();
        assert_eq!(names, ["ubuntu (1)", "ubuntu (2)"]);
        // And still no address anywhere in the answer (Codex P2 on #4241).
        let json = serde_json::to_string(&hosts).unwrap();
        assert!(!json.contains("10.0.0.4"), "{json}");
        assert_ne!(hosts[0].host_id, hosts[1].host_id);
        let agents: Vec<&str> = hosts
            .iter()
            .map(|h| h.channels[0].agents[0].name.as_str())
            .collect();
        assert_eq!(agents, ["A", "B"]);
    }

    #[test]
    fn a_lan_peer_at_one_of_our_own_addresses_is_left_to_the_registry() {
        let own: HashSet<IpAddr> = ["10.0.0.230".parse().unwrap()].into_iter().collect();
        let peers = [peer("narko", "dev", "10.0.0.230", 29702, &["Loap"], 5)];
        assert!(lan_hosts(&peers, &own, NOW).is_empty());
    }

    #[test]
    fn a_peer_without_a_channel_is_labelled_by_its_port() {
        let peers = [
            peer("old", "", "10.0.0.9", 29700, &["A"], 5),
            peer("old", "", "10.0.0.9", 29701, &["B"], 5),
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
            "10.0.0.26",
            29700,
            &["Manoz"],
            90,
        )];
        assert!(lan_hosts(&quiet, &HashSet::new(), NOW)[0].channels[0].stale);
        let fresh = [peer(
            "Area54",
            "stable",
            "10.0.0.26",
            29700,
            &["Manoz"],
            5,
        )];
        let snap = snapshot(
            &[],
            &ChannelStatuses::new(),
            &fresh,
            &HashSet::new(),
            "narko",
            "stable",
            "",
            NOW,
        );
        let json = serde_json::to_string(&snap).unwrap();
        assert!(
            !json.contains("lan-secret") && !json.contains("10.0.0.26"),
            "{json}"
        );
    }

    /// Codex P1 on #4241: an mDNS-only peer is re-resolved tens of minutes apart,
    /// so a peer that answers the agent-name poll is live, whatever `last_seen` says.
    #[test]
    fn a_peer_that_answers_polls_is_not_stale() {
        let mut p = peer("Area54", "stable", "10.0.0.26", 29700, &["Manoz"], 1800);
        assert!(lan_hosts(std::slice::from_ref(&p), &HashSet::new(), NOW)[0].channels[0].stale);
        p.last_polled_ok = NOW / 1000 - 10;
        assert!(!lan_hosts(&[p], &HashSet::new(), NOW)[0].channels[0].stale);
    }

    #[test]
    fn a_host_with_no_name_is_not_labelled_by_its_address() {
        let hosts = lan_hosts(
            &[peer("", "stable", "10.0.0.77", 29700, &["A"], 5)],
            &HashSet::new(),
            NOW,
        );
        assert_eq!(hosts[0].display_name, "unnamed host");
    }

    /// Codex P2 on #4241: two instances on one machine and channel (an installed
    /// and a portable build) host different agents; neither may replace the other.
    #[test]
    fn two_instances_on_one_channel_both_stay() {
        let peers = [
            peer("narko2", "stable", "10.0.0.50", 29700, &["A"], 5),
            peer("narko2", "stable", "10.0.0.50", 29702, &["B"], 5),
        ];
        let hosts = lan_hosts(&peers, &HashSet::new(), NOW);
        assert_eq!(hosts.len(), 1);
        let chans: Vec<&str> = hosts[0]
            .channels
            .iter()
            .map(|c| c.channel.as_str())
            .collect();
        assert_eq!(chans, ["stable :29700", "stable :29702"]);
    }

    #[test]
    fn the_same_instance_seen_twice_is_listed_once() {
        let older = peer("Area54", "stable", "10.0.0.26", 29700, &["Old"], 50);
        let newer = peer("Area54", "stable", "10.0.0.26", 29700, &["New"], 5);
        let hosts = lan_hosts(&[older, newer], &HashSet::new(), NOW);
        assert_eq!(hosts[0].channels.len(), 1);
        assert_eq!(hosts[0].channels[0].channel, "stable");
        assert_eq!(hosts[0].channels[0].agents[0].name, "New");
    }

    // The platform tag (SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §3).
    #[test]
    fn this_machines_other_channels_carry_this_machines_platform() {
        let chans = other_channels(
            &[entry("Loap", "dev-fix", 0)],
            &ChannelStatuses::new(),
            "stable",
            "",
            NOW,
        );
        assert_eq!(chans[0].os, crate::backend::host_os::local_os());
    }

    #[test]
    fn a_lan_peer_carries_the_platform_it_advertised_and_none_when_it_did_not() {
        let mut mac = peer("Area54", "stable", "10.0.0.26", 29700, &["Manoz"], 5);
        mac.os = "macos".to_string();
        let old = peer("starpower", "stable", "10.0.0.27", 29700, &["Opaz"], 5);
        let hosts = lan_hosts(&[mac, old], &HashSet::new(), NOW);
        let os_of = |name: &str| {
            hosts
                .iter()
                .find(|h| h.display_name == name)
                .unwrap()
                .channels[0]
                .os
                .clone()
        };
        assert_eq!(os_of("Area54"), "macos");
        assert_eq!(os_of("starpower"), "");
    }

    #[test]
    fn two_instances_on_one_host_keep_their_own_platform() {
        // A Windows machine that also runs an instance inside WSL.
        let mut win = peer("narko2", "stable", "10.0.0.30", 29700, &["A"], 5);
        win.os = "windows".to_string();
        let mut wsl = peer("narko2", "stable", "10.0.0.30", 29702, &["B"], 5);
        wsl.os = "linux".to_string();
        let hosts = lan_hosts(&[win, wsl], &HashSet::new(), NOW);
        let mut oses: Vec<String> = hosts[0].channels.iter().map(|c| c.os.clone()).collect();
        oses.sort();
        assert_eq!(oses, vec!["linux".to_string(), "windows".to_string()]);
    }

    // Phase 4: each agent's state (SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md §3.4).
    #[test]
    fn another_channels_agents_carry_the_state_that_channel_reported() {
        let report = ChannelReport {
            status: [("korp".to_string(), AgentState::Waiting)].into_iter().collect(),
            lines: [("korp".to_string(), "Fixing the login race".to_string())].into_iter().collect(),
            providers: [("korp".to_string(), "codex".to_string())].into_iter().collect(),
        };
        let statuses: ChannelStatuses = [("dev".to_string(), report)].into_iter().collect();
        let chans = other_channels(
            &[entry("Korp", "dev", 0), entry("Loap", "dev", 0)],
            &statuses,
            "stable",
            "",
            NOW,
        );
        let got: Vec<(&str, Option<AgentState>)> = chans[0]
            .agents
            .iter()
            .map(|a| (a.name.as_str(), a.status))
            .collect();
        assert_eq!(got, [("Korp", Some(AgentState::Waiting)), ("Loap", None)]);
        // An agent with no state has no `status` key at all.
        let json = serde_json::to_value(&chans[0].agents).unwrap();
        assert_eq!(json[0]["status"], "waiting");
        assert!(json[1].get("status").is_none(), "{json}");
        // The same for the line, and the provider.
        assert_eq!(json[0]["line"], "Fixing the login race");
        assert!(json[1].get("line").is_none(), "{json}");
        assert_eq!(json[0]["provider"], "codex");
        assert!(json[1].get("provider").is_none(), "{json}");
    }

    #[test]
    fn a_lan_peers_agents_carry_the_state_it_reported() {
        let mut p = peer(
            "Area54",
            "stable",
            "10.0.0.26",
            29700,
            &["Manoz", "Opaz"],
            5,
        );
        p.agent_status.insert("manoz".into(), AgentState::Working);
        p.agent_lines.insert("manoz".into(), "Fixing the login race".into());
        p.agent_providers.insert("manoz".into(), "claude".into());
        let hosts = lan_hosts(&[p], &HashSet::new(), NOW);
        let got: Vec<(Option<AgentState>, Option<&str>, Option<&str>)> = hosts[0].channels[0]
            .agents
            .iter()
            .map(|a| (a.status, a.line.as_deref(), a.provider.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                (Some(AgentState::Working), Some("Fixing the login race"), Some("claude")),
                (None, None, None)
            ]
        );
    }

    #[test]
    fn status_targets_are_live_loopback_instances_other_than_this_one() {
        let at = |name: &str, channel: &str, url: &str, age_ms: u64| {
            let mut e = entry(name, channel, age_ms);
            e.local_url = url.to_string();
            e
        };
        let entries = [
            at("Korp", "dev", "http://127.0.0.1:29702", 0),
            at("Loap", "dev", "http://127.0.0.1:29702", 0),
            at("Old", "gone", "http://127.0.0.1:29703", STALE_AFTER_MS + 1),
            at("Far", "lan", "http://10.0.0.9:29700", 0),
            at("Me", "renamed", "http://127.0.0.1:29700", 0),
            at("Self", "stable", "http://127.0.0.1:29701", 0),
            at("Nochan", "", "http://127.0.0.1:29704", 0),
            at("Six", "v6", "http://[::1]:29705", 0),
        ];
        let targets = channel_status_targets(&entries, "stable", "http://127.0.0.1:29700", NOW);
        let urls: Vec<(&str, &str)> = targets
            .iter()
            .map(|t| (t.channel.as_str(), t.local_url.as_str()))
            .collect();
        assert_eq!(
            urls,
            [
                ("dev", "http://127.0.0.1:29702"),
                ("v6", "http://[::1]:29705")
            ],
            "one per instance; not stale, not off-machine, not this instance"
        );
        assert_eq!(targets[0].auth_key, "secret");
    }

    /// An instance that restarted on the same URL has a new key, and its older
    /// entries keep the old one for a while. Whatever order the registry lists
    /// them in, the freshest entry's key is used (#4703).
    #[test]
    fn a_restarted_instance_is_asked_with_its_newest_key() {
        let at = |name: &str, age_ms: u64, key: &str| {
            let mut e = entry(name, "dev", age_ms);
            e.local_url = "http://127.0.0.1:29702".to_string();
            e.auth_key = key.to_string();
            e
        };
        let entries = [
            at("Korp", 30_000, "old-key"),
            at("Loap", 1_000, "new-key"),
            at("Opaz", 20_000, "old-key"),
        ];
        for order in [entries.to_vec(), entries.iter().rev().cloned().collect()] {
            let targets = channel_status_targets(&order, "stable", "", NOW);
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].auth_key, "new-key");
        }
    }

    /// One-shot HTTP server answering any request with `body`, and the request it
    /// received.
    async fn serve_once(body: &'static str) -> (String, tokio::sync::oneshot::Receiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        (url, rx)
    }

    #[tokio::test]
    async fn channel_statuses_asks_each_instance_and_skips_one_that_fails() {
        let (url, request) = serve_once(
            r#"{"agents":["Korp"],"now_ms":1,"agent_status":{"Korp":{"state":"working","since_ms":1}},"agent_lines":{"Korp":"Building"}}"#,
        )
        .await;
        let targets = [
            ChannelStatusTarget {
                channel: "dev".into(),
                local_url: url,
                auth_key: "chan-key".into(),
            },
            // Nothing listens on the discard port.
            ChannelStatusTarget {
                channel: "dead".into(),
                local_url: "http://127.0.0.1:9".into(),
                auth_key: "k".into(),
            },
        ];
        let statuses = channel_statuses(&reqwest::Client::new(), &targets).await;
        assert_eq!(statuses.len(), 1, "{statuses:?}");
        assert_eq!(statuses["dev"].status.get("korp"), Some(&AgentState::Working));
        assert_eq!(statuses["dev"].lines.get("korp").map(String::as_str), Some("Building"));
        let request = request.await.unwrap().to_lowercase();
        assert!(
            request.starts_with("get /agentmux/reactive/agent-names "),
            "{request}"
        );
        assert!(
            request.contains(&format!(
                "{}: chan-key",
                agentmux_common::AUTH_KEY_HEADER.to_lowercase()
            )),
            "{request}"
        );
    }

    fn with_uid(name: &str, channel: &str, uid: &str) -> AgentEntry {
        AgentEntry { uid: uid.to_string(), ..entry(name, channel, 0) }
    }

    /// REPORT_SWARM_REMOTE_PROVIDER_ICON_RCA_2026_10_18.md: an instance from
    /// before providers were reported still shows its agents' providers, from
    /// this machine's definitions.
    #[test]
    fn an_older_channels_agents_get_their_defined_provider() {
        let entries = [with_uid("Camper", "dev", "uid-camper"), with_uid("Clamk", "dev", "uid-clamk"), entry("Loap", "dev", 0)];
        let defined: DefinedProviders =
            [("uid-camper".to_string(), "claude".to_string()), ("uid-clamk".to_string(), "codex".to_string())].into();
        let mut snap = snapshot(&entries, &ChannelStatuses::new(), &[], &HashSet::new(), "narko", "stable", "", NOW);
        fill_defined_providers(&mut snap, &entries, &defined);
        let got: Vec<(&str, Option<&str>)> =
            snap.hosts[0].channels[0].agents.iter().map(|a| (a.name.as_str(), a.provider.as_deref())).collect();
        // Loap's entry has no uid, so nothing is known about it.
        assert_eq!(got, vec![("Camper", Some("claude")), ("Clamk", Some("codex")), ("Loap", None)]);
    }

    #[test]
    fn the_instances_own_report_wins_over_the_definition() {
        let entries = [with_uid("Camper", "dev", "uid-camper")];
        let mut report = ChannelReport::default();
        report.providers.insert("camper".into(), "codex".into());
        let statuses: ChannelStatuses = [("dev".to_string(), report)].into();
        let defined: DefinedProviders = [("uid-camper".to_string(), "claude".to_string())].into();
        let mut snap = snapshot(&entries, &statuses, &[], &HashSet::new(), "narko", "stable", "", NOW);
        fill_defined_providers(&mut snap, &entries, &defined);
        assert_eq!(snap.hosts[0].channels[0].agents[0].provider.as_deref(), Some("codex"));
    }

    #[test]
    fn lan_hosts_are_left_as_they_are() {
        let entries = [with_uid("Camper", "dev", "uid-camper")];
        let defined: DefinedProviders = [("uid-camper".to_string(), "claude".to_string())].into();
        let agent = RemoteAgent { name: "Camper".into(), block_id: String::new(), status: None, line: None, provider: None };
        let channel = RemoteChannel { channel: "dev".into(), seen_at_ms: NOW, stale: false, os: String::new(), agents: vec![agent], route: None };
        let mut snap = SwarmOtherInstancesResult {
            hostname: "narko".into(),
            channel: "stable".into(),
            hosts: vec![RemoteHost { host_id: "lan:x".into(), display_name: "x".into(), tier: "lan", channels: vec![channel] }],
        };
        fill_defined_providers(&mut snap, &entries, &defined);
        assert_eq!(snap.hosts[0].channels[0].agents[0].provider, None, "another machine's agent isn't this machine's definition");
    }

    #[test]
    fn only_known_providers_are_read_from_the_definitions() {
        let entries = [with_uid("A", "dev", "u1"), with_uid("B", "dev", "u2"), with_uid("C", "dev", "u3"), entry("D", "dev", 0)];
        let defined = defined_providers_from(&entries, |uid| match uid {
            "u1" => Some(" claude ".to_string()),
            "u2" => Some("<b>made-up</b>".to_string()),
            _ => None,
        });
        assert_eq!(defined, [("u1".to_string(), "claude".to_string())].into());
    }
}
