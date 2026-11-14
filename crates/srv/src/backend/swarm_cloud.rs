// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Swarm's cloud tier: the account's other installs, as hosts of tier `cloud`
//! (`docs/specs/SPEC_SWARM_CLOUD_TIER_2026_11_14.md`). One record is one install, so one host with one channel.
//!
//! Display only. A cloud channel has no route, so a broadcast and a stop skip it, and its rows carry what a signed
//! presence record carries (a state, a line, where it runs) and no provider, seat or account. The answer holds no
//! address, key, instance id or signature.

use super::agent_state::AgentState;
use super::lan::discovery::LanInstance;
use super::runtime_adapter::RuntimeKind;
use super::swarm_remote::{fnv1a32, RemoteAgent, RemoteChannel, RemoteHost, SwarmOtherInstancesResult};
use crate::muxbus::cloud_installs::{CloudInstall, CloudView};

/// A record not heard from for this long is shown as stale: three missed beats of the one-minute publisher.
pub const STALE_AFTER_MS: u64 = 3 * 60_000;
/// Past this a record is not shown at all.
pub const HIDE_AFTER_MS: u64 = 10 * 60_000;

/// Adds the cloud hosts to `snapshot`, and says how the read went in its `cloud` field. `view` is `None` where
/// this build has no sign-in to read with.
pub fn add_cloud(snapshot: &mut SwarmOtherInstancesResult, view: Option<&CloudView>, own_wan_id: Option<&str>, lan: &[LanInstance], now_ms: u64) {
    let Some(view) = view else {
        snapshot.cloud = Some("off");
        return;
    };
    snapshot.cloud = Some(view.state.label());
    // This machine's other channels, as the registry listed them: the same machines, in more detail.
    let mut here: Vec<String> = vec![snapshot.channel.to_lowercase()];
    for host in snapshot.hosts.iter().filter(|h| h.tier == "host") {
        here.extend(host.channels.iter().map(|c| c.channel.to_lowercase()));
    }
    let hosts = cloud_hosts(&view.installs, own_wan_id, &snapshot.hostname, &here, lan, now_ms);
    snapshot.hosts.extend(hosts);
}

/// The hosts for `installs`, leaving out what is listed elsewhere: this install's own record, another channel of
/// this machine (`channels_here`), and a LAN peer of the same host name and channel. A peer from before it
/// advertised its channel is not matched.
pub fn cloud_hosts(
    installs: &[CloudInstall],
    own_wan_id: Option<&str>,
    hostname: &str,
    channels_here: &[String],
    lan: &[LanInstance],
    now_ms: u64,
) -> Vec<RemoteHost> {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let mut hosts: Vec<RemoteHost> = installs
        .iter()
        .filter(|c| own_wan_id != Some(c.record.instance_id.as_str()))
        .filter(|c| now_ms.saturating_sub(c.seen_at_ms) <= HIDE_AFTER_MS)
        .filter(|c| !(same(&c.record.hostname, hostname) && channels_here.iter().any(|ch| same(ch, &c.record.channel))))
        .filter(|c| !lan.iter().any(|p| !p.channel.is_empty() && same(&p.hostname, &c.record.hostname) && same(&p.channel, &c.record.channel)))
        .map(|c| host_of(c, now_ms))
        .collect();
    hosts.sort_by(|a, b| (a.display_name.to_lowercase(), &a.channels[0].channel).cmp(&(b.display_name.to_lowercase(), &b.channels[0].channel)));
    hosts
}

fn host_of(install: &CloudInstall, now_ms: u64) -> RemoteHost {
    let record = &install.record;
    let agents = record
        .agents
        .iter()
        .map(|a| RemoteAgent {
            name: a.name.clone(),
            block_id: String::new(),
            status: a.state.as_deref().and_then(|s| serde_json::from_value::<AgentState>(serde_json::Value::String(s.to_string())).ok()),
            line: a.line.as_deref().and_then(super::agent_line::clean_line),
            provider: None,
            seat: None,
            account: None,
            account_label: None,
            runtime: RuntimeKind::parse(&a.kind),
        })
        .collect();
    RemoteHost {
        // A short hash of the install's id keeps two installs of one name apart without putting the id in the answer.
        host_id: format!("cloud:{}#{:08x}", record.hostname.to_lowercase(), fnv1a32(&record.instance_id)),
        display_name: record.hostname.clone(),
        tier: "cloud",
        channels: vec![RemoteChannel {
            channel: record.channel.clone(),
            seen_at_ms: install.seen_at_ms,
            stale: now_ms.saturating_sub(install.seen_at_ms) > STALE_AFTER_MS,
            os: record.os.clone(),
            agents,
            route: None,
        }],
    }
}

#[cfg(test)]
mod tests;
