// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One entry per running instance in the peer list.
//!
//! An instance can be found more than once: mDNS may resolve the same machine under two service
//! names (an IPv4 record and an IPv6 link-local one). The two entries then share a host name, a
//! channel, a port and the peer's `lan_key`, and differ in the address (and in whether this
//! machine has managed to poll it). Listed twice, a host named in a remote operation fits "more
//! than one instance" and the Swarm shows the machine twice.
//!
//! The entries stay in the store, so when the preferred address goes stale the other one is what
//! is left and takes over: this only chooses which one to show.

use std::net::IpAddr;

use super::{peer_staleness_window_secs, LanDiscovery, LanDiscoveryController, LanInstance};

impl LanDiscovery {
    /// Every live entry, an instance found at two addresses once for each: what to match an
    /// incoming connection's source address against, which may be either. Anything that lists or
    /// targets peers reads `get_instances` (each instance once) instead.
    pub fn get_all_instances(&self) -> Vec<LanInstance> {
        let now = agentmux_common::time::now_secs_u64();
        self.instances
            .read()
            .values()
            .filter(|inst| now.saturating_sub(inst.last_seen) <= peer_staleness_window_secs(inst.other_ttl_secs))
            .cloned()
            .collect()
    }
}

impl LanDiscoveryController {
    /// [`LanDiscovery::get_all_instances`], empty when discovery is not running.
    pub fn get_all_instances(&self) -> Vec<LanInstance> {
        self.slot.read().as_ref().map(|d| d.get_all_instances()).unwrap_or_default()
    }
}

/// How good an address is to reach a peer on: a routable IPv4 address first, then a routable IPv6
/// one, then a link-local one (it needs the right interface to be reachable at all), then an
/// address that does not parse.
fn address_rank(address: &str) -> u8 {
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) if a.is_link_local() => 2,
        Ok(IpAddr::V4(_)) => 0,
        Ok(IpAddr::V6(a)) if (a.segments()[0] & 0xffc0) == 0xfe80 => 2,
        Ok(IpAddr::V6(_)) => 1,
        Err(_) => 3,
    }
}

/// Whether two entries are the same running instance: the same host name, channel and port, and
/// the same `lan_key`. An entry with no key says nothing about who it is, so it is never merged.
fn same_instance(a: &LanInstance, b: &LanInstance) -> bool {
    !a.auth_key.is_empty()
        && a.auth_key == b.auth_key
        && a.port == b.port
        && a.hostname.trim().eq_ignore_ascii_case(b.hostname.trim())
        && a.channel.eq_ignore_ascii_case(&b.channel)
}

/// Whether `a` is a better entry to show than `b` for the same instance: the better address, then
/// the one this machine has polled most recently, then the one heard from most recently.
fn better(a: &LanInstance, b: &LanInstance) -> bool {
    (address_rank(&a.address), std::cmp::Reverse(a.last_polled_ok), std::cmp::Reverse(a.last_seen))
        < (address_rank(&b.address), std::cmp::Reverse(b.last_polled_ok), std::cmp::Reverse(b.last_seen))
}

/// `peers` with each running instance once, in the order its first entry was listed.
pub(crate) fn one_per_instance(peers: Vec<LanInstance>) -> Vec<LanInstance> {
    let mut out: Vec<LanInstance> = Vec::with_capacity(peers.len());
    for peer in peers {
        match out.iter().position(|kept| same_instance(kept, &peer)) {
            Some(i) if better(&peer, &out[i]) => out[i] = peer,
            Some(_) => {}
            None => out.push(peer),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::LanDiscovery;
    use super::*;

    fn peer(address: &str, polled: u64, seen: u64) -> LanInstance {
        LanInstance {
            instance_id: "v0.60.9".into(),
            hostname: "charlie".into(),
            version: "0.60.9".into(),
            channel: "local-main-aaaaaa-00000000".into(),
            os: "linux".into(),
            address: address.into(),
            port: 29700,
            auth_key: "charlie-key".into(),
            agents: Vec::new(),
            agent_status: Default::default(),
            agent_lines: Default::default(),
            agent_providers: Default::default(),
            agent_accounts: Default::default(),
            agent_seats: Default::default(),
            first_seen: 0,
            last_seen: seen,
            last_polled_ok: polled,
            other_ttl_secs: 4500,
        }
    }

    /// A `LanDiscovery` with an idle mDNS daemon and no tasks, holding the given peers.
    fn discovery_with(peers: Vec<(&str, LanInstance)>) -> LanDiscovery {
        let discovery = LanDiscovery {
            daemon: mdns_sd::ServiceDaemon::new().expect("daemon construction (no register/browse)"),
            instances: std::sync::Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
            instance_id: "self-id".to_string(),
            event_bus: std::sync::Arc::new(crate::backend::eventbus::EventBus::new()),
            service_fullname: String::new(),
            auth_key: String::new(),
            hostname: String::new(),
            version: String::new(),
            port: 56024,
            udp_cancel: parking_lot::Mutex::new(None),
            udp_peer_cancel: parking_lot::Mutex::new(None),
            agent_names_cancel: parking_lot::Mutex::new(None),
            instance_record_cancel: parking_lot::Mutex::new(None),
            instances_dir: None,
            instance_record_live: parking_lot::Mutex::new(false),
            announced_v4: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::BTreeSet::new())),
            monitored: true,
            started_at: std::time::Instant::now(),
        };
        for (name, peer) in peers {
            discovery.instances.write().insert(name.to_string(), peer);
        }
        discovery
    }

    #[test]
    fn the_peer_list_has_each_machine_once_and_the_other_address_takes_over_when_the_first_goes_stale() {
        let now = agentmux_common::time::now_secs_u64();
        let discovery = discovery_with(vec![
            ("host-v6._agentmux._tcp.local.", peer(LINK_LOCAL_V6, 0, now)),
            ("host-v4._agentmux._tcp.local.", peer(V4, now, now)),
        ]);
        let listed = discovery.get_instances();
        assert_eq!(listed.len(), 1, "one machine, one entry: {listed:?}");
        assert_eq!(listed[0].address, V4);
        // Matching a connection's source address still sees both.
        let mut all: Vec<String> = discovery.get_all_instances().into_iter().map(|p| p.address).collect();
        all.sort();
        assert_eq!(all, [V4, LINK_LOCAL_V6]);

        // The routable entry goes quiet past its TTL: the link-local one is what is left.
        discovery.instances.write().get_mut("host-v4._agentmux._tcp.local.").unwrap().last_seen = 0;
        let left = discovery.get_instances();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].address, LINK_LOCAL_V6);
    }

    const V4: &str = "198.51.100.25";
    const LINK_LOCAL_V6: &str = "fe80::1";

    #[test]
    fn one_machine_found_at_an_ipv4_and_a_link_local_address_is_listed_once_at_the_ipv4_one() {
        // As one host listed another: the same channel, port and key; the link-local entry
        // never polled, and listed first.
        let listed = one_per_instance(vec![peer(LINK_LOCAL_V6, 0, 1793535714), peer(V4, 1793535744, 1793535744)]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].address, V4);

        let other_order = one_per_instance(vec![peer(V4, 1793535744, 1793535744), peer(LINK_LOCAL_V6, 0, 1793535714)]);
        assert_eq!(other_order.len(), 1);
        assert_eq!(other_order[0].address, V4);
    }

    #[test]
    fn the_other_address_is_what_is_left_when_the_preferred_one_is_gone() {
        let listed = one_per_instance(vec![peer(LINK_LOCAL_V6, 0, 1793535714)]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].address, LINK_LOCAL_V6);
    }

    #[test]
    fn a_routable_address_beats_a_link_local_one_whatever_was_polled() {
        let listed = one_per_instance(vec![peer(V4, 0, 5), peer(LINK_LOCAL_V6, 99, 99)]);
        assert_eq!(listed[0].address, V4);
        let global_v6 = one_per_instance(vec![peer(LINK_LOCAL_V6, 99, 99), peer("2001:db8::1", 0, 5)]);
        assert_eq!(global_v6[0].address, "2001:db8::1", "a routable IPv6 address beats a link-local one");
        let v4_over_v6 = one_per_instance(vec![peer("2001:db8::1", 99, 99), peer(V4, 0, 5)]);
        assert_eq!(v4_over_v6[0].address, V4, "IPv4 beats IPv6");
    }

    #[test]
    fn with_equal_addresses_the_one_polled_or_heard_from_last_wins() {
        let listed = one_per_instance(vec![peer("198.51.100.10", 10, 10), peer("198.51.100.11", 20, 5)]);
        assert_eq!(listed[0].address, "198.51.100.11", "the more recently polled");
        let heard = one_per_instance(vec![peer("198.51.100.10", 10, 10), peer("198.51.100.11", 10, 30)]);
        assert_eq!(heard[0].address, "198.51.100.11", "the more recently heard from");
    }

    #[test]
    fn different_channels_ports_keys_or_hosts_are_different_instances() {
        let mut other_channel = peer("198.51.100.25", 1, 1);
        other_channel.channel = "stable".into();
        let mut other_port = peer("198.51.100.25", 1, 1);
        other_port.port = 29701;
        let mut other_key = peer("198.51.100.25", 1, 1);
        other_key.auth_key = "another-key".into();
        let mut other_host = peer("198.51.100.25", 1, 1);
        other_host.hostname = "starpower".into();
        let listed = one_per_instance(vec![peer(V4, 1, 1), other_channel, other_port, other_key, other_host]);
        assert_eq!(listed.len(), 5);
    }

    #[test]
    fn an_entry_with_no_key_is_never_merged() {
        let mut a = peer(V4, 1, 1);
        a.auth_key.clear();
        let mut b = peer(LINK_LOCAL_V6, 1, 1);
        b.auth_key.clear();
        assert_eq!(one_per_instance(vec![a, b]).len(), 2);
    }

    #[test]
    fn host_names_match_without_regard_to_case_or_padding() {
        let mut padded = peer(LINK_LOCAL_V6, 0, 1);
        padded.hostname = " Charlie ".into();
        assert_eq!(one_per_instance(vec![peer(V4, 1, 1), padded]).len(), 1);
    }
}
