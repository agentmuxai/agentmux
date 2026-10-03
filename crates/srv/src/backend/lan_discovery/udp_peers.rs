// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Desktop-to-desktop discovery that does not depend on mDNS
//! (`docs/specs/SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` §4.7 item 5,
//! `docs/specs/SPEC_LAN_UDP_PEER_DISCOVERY_2026_10_02.md`).
//!
//! mDNS can fail for reasons nothing on our side can fix: another program holds
//! UDP 5353 (Area54, 2026-10-01: Chrome, then the DNS Client service), a guest
//! network filters multicast, an interface never gets a socket. The UDP probe
//! responder on 47891 only ever served mobile clients, so two desktops had one
//! route to each other and no second.
//!
//! Each instance binds one UDP port ([`DESKTOP_DISCOVERY_PORT`]) while LAN is on
//! and, from that same socket:
//!
//! * sends the existing probe to the limited broadcast address and to each
//!   interface's directed broadcast every [`PROBE_INTERVAL`];
//! * answers a probe from a private-range source with the identity payload the
//!   47891 responder sends (`probe_response_json`), without its `siblings`;
//! * reads identity payloads that come back and records the sender as a peer.
//!
//! Probing and answering share one socket on purpose: a peer's reply goes to the
//! probe's source port, so it lands on a port the firewall rule names rather than
//! on an ephemeral one.
//!
//! **Trust is the same as mDNS.** A reply is network-claimed and unauthenticated,
//! exactly like a TXT record; what it gains an attacker is a place in the peer
//! table, not an identity. Jekt signing and LAN key pinning are untouched. The
//! limits below (private-range source, field sizes, port floor, a peer cap) are
//! the same defence in depth the mDNS path has.
//!
//! Like the 47891 responder this socket does not set `SO_REUSEADDR`, so only the
//! first instance on a host holds it. A second instance (a `task dev` build next
//! to an installed one) quietly does without; hosts with one install are the
//! case this exists for.

use super::*;

/// UDP port for desktop-to-desktop discovery. Inside the fixed block
/// (`lan_ports::LAN_PORT_RANGE`, below every OS's ephemeral range) so one
/// firewall rule can name it. UDP and TCP port numbers are separate, so sharing
/// a number with a TCP listener in the block is harmless.
pub(crate) const DESKTOP_DISCOVERY_PORT: u16 = 29700;

/// How often each instance probes.
pub(super) const PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
///
/// How long a UDP-learned peer stays listed after its last reply. This is the
/// staleness floor every peer gets (`peer_staleness_window_secs` clamps a
/// smaller `other_ttl_secs` up to it), so it is the real lifetime, not a hint:
/// ten missed probes. A departed peer therefore lingers five minutes, which is
/// still far shorter than an mDNS peer's TTL.
const UDP_PEER_TTL_SECS: u32 = LAN_PEER_STALE_TIMEOUT_FLOOR_SECS as u32;

/// Key prefix of a peer learned over UDP in `LanDiscovery::instances` (mDNS
/// peers are keyed by their service name).
const UDP_KEY_PREFIX: &str = "udp:";

/// Peers learned over UDP at once. An unauthenticated broadcast must not be able
/// to grow the table without bound; a real LAN has a handful of machines.
const MAX_UDP_PEERS: usize = 64;

const MAX_ID_LEN: usize = 128;
const MAX_HOSTNAME_LEN: usize = 253;
const MAX_VERSION_LEN: usize = 64;
const MAX_CHANNEL_LEN: usize = 64;
const MAX_AUTH_KEY_LEN: usize = 256;
/// Below this a port is privileged or a well-known service, never one of ours.
const MIN_PEER_PORT: u16 = 1024;

/// The identity a peer's reply claims, after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UdpPeer {
    pub instance_id: String,
    pub hostname: String,
    pub version: String,
    /// Empty when the peer predates advertising it.
    pub channel: String,
    pub address: IpAddr,
    pub port: u16,
    pub auth_key: String,
}

/// Validate a datagram as a peer's identity reply from `src`. `None` for noise,
/// a probe, another protocol version, a source outside the private ranges, or
/// any field that is missing, empty or implausibly large.
pub(super) fn parse_peer_reply(bytes: &[u8], src: &std::net::SocketAddr) -> Option<UdpPeer> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some(UDP_RESPONSE_TYPE)
        || !is_probe_version_match(&value)
    {
        return None;
    }
    if !is_lan_source(src) {
        return None;
    }
    let text = |key: &str, max: usize| -> Option<String> {
        let s = value.get(key)?.as_str()?;
        (s.len() <= max).then(|| s.to_string())
    };
    let instance_id = text("instance_id", MAX_ID_LEN).filter(|s| !s.is_empty())?;
    // Without a key the peer cannot be queried at all (`query_peers_concurrently`
    // skips it), so there is nothing to record.
    let auth_key = text("auth_key", MAX_AUTH_KEY_LEN).filter(|s| !s.is_empty())?;
    let hostname = text("hostname", MAX_HOSTNAME_LEN)?;
    let version = text("version", MAX_VERSION_LEN)?;
    // Optional: a peer from before it was advertised sends none, and an
    // oversize one is dropped rather than refusing the whole reply.
    let channel = text("channel", MAX_CHANNEL_LEN).unwrap_or_default();
    let port = u16::try_from(value.get("port")?.as_u64()?)
        .ok()
        .filter(|p| *p >= MIN_PEER_PORT)?;
    Some(UdpPeer {
        instance_id,
        hostname,
        version,
        channel,
        address: src.ip(),
        port,
        auth_key,
    })
}

/// What [`merge_udp_peer`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Merge {
    /// A new peer: the table changed, so clients should be told.
    Inserted,
    /// Already known (by mDNS or an earlier reply); only its liveness moved.
    Refreshed,
    /// Not recorded (table full).
    Dropped,
}

/// A UDP peer's key: the address its reply came from and the port it serves.
///
/// Not the instance id. That is `v<version>` (the launcher's `AGENTMUX_INSTANCE_ID`),
/// so every machine on the same release shares it, and keying on it merged two
/// such machines into one entry that flapped between them (Codex P1 on #4241).
fn udp_key(address: &str, port: u16) -> String {
    format!("{UDP_KEY_PREFIX}{address}:{port}")
}

/// The same peer as `peer` already in the table, by the one identity that is
/// unique on a LAN: the address it answers on and the port it serves.
fn find_existing<'a>(
    instances: &'a mut HashMap<String, LanInstance>,
    peer: &UdpPeer,
) -> Option<(&'a String, &'a mut LanInstance)> {
    let address = peer.address.to_string();
    instances
        .iter_mut()
        .find(|(_, i)| i.address == address && i.port == peer.port)
}

/// Record `peer` in the table without ever duplicating a peer mDNS already
/// knows: `find_agent` queries every entry, so a double would double every
/// lookup. An mDNS entry is authoritative and only has its liveness refreshed.
pub(super) fn merge_udp_peer(
    instances: &mut HashMap<String, LanInstance>,
    peer: &UdpPeer,
    now: u64,
) -> Merge {
    if let Some((key, existing)) = find_existing(instances, peer) {
        existing.last_seen = now;
        if key.starts_with(UDP_KEY_PREFIX) {
            // Ours: the reply is the freshest word on who this is. A peer that
            // restarted on a new release keeps its address and port but reports a
            // new version and instance id; the key does not depend on either.
            existing.instance_id = peer.instance_id.clone();
            existing.hostname = peer.hostname.clone();
            existing.version = peer.version.clone();
            // An older build, or an oversize field, sends none: keep the channel
            // already known rather than fall back to the port label (Codex P2 on
            // #4241), as the mDNS path does.
            if !peer.channel.is_empty() {
                existing.channel = peer.channel.clone();
            }
            existing.auth_key = peer.auth_key.clone();
        } else {
            if existing.auth_key.is_empty() {
                existing.auth_key = peer.auth_key.clone();
            }
            if existing.channel.is_empty() {
                existing.channel = peer.channel.clone();
            }
        }
        return Merge::Refreshed;
    }
    if instances
        .keys()
        .filter(|k| k.starts_with(UDP_KEY_PREFIX))
        .count()
        >= MAX_UDP_PEERS
    {
        return Merge::Dropped;
    }
    let address = peer.address.to_string();
    instances.insert(
        udp_key(&address, peer.port),
        LanInstance {
            instance_id: peer.instance_id.clone(),
            hostname: peer.hostname.clone(),
            version: peer.version.clone(),
            channel: peer.channel.clone(),
            address,
            port: peer.port,
            auth_key: peer.auth_key.clone(),
            agents: Vec::new(),
            first_seen: now,
            last_seen: now,
            last_polled_ok: 0,
            other_ttl_secs: UDP_PEER_TTL_SECS,
        },
    );
    Merge::Inserted
}

/// Remove UDP-learned entries that an mDNS entry now covers (the same address
/// and port). Called after an mDNS resolution so the order the two routes find a
/// peer in does not decide whether it is listed twice. Instance ids are not
/// compared: they are per release, not per machine (see [`udp_key`]).
pub(super) fn drop_udp_duplicates(instances: &mut HashMap<String, LanInstance>) {
    let mdns: Vec<(String, u16)> = instances
        .iter()
        .filter(|(k, _)| !k.starts_with(UDP_KEY_PREFIX))
        .map(|(_, i)| (i.address.clone(), i.port))
        .collect();
    instances.retain(|key, i| {
        !key.starts_with(UDP_KEY_PREFIX)
            || !mdns
                .iter()
                .any(|(addr, port)| *addr == i.address && *port == i.port)
    });
}

/// Where to send a probe: the limited broadcast address, plus each non-loopback
/// IPv4 interface's directed broadcast. The limited broadcast alone leaves by
/// one interface only on Windows, which is the wrong one on a host with a VPN or
/// a virtual adapter (narko has four).
pub(super) fn probe_targets(
    broadcasts: impl IntoIterator<Item = Ipv4Addr>,
    port: u16,
) -> Vec<std::net::SocketAddr> {
    let mut all: BTreeSet<Ipv4Addr> = broadcasts
        .into_iter()
        .filter(|b| !b.is_loopback())
        .collect();
    all.insert(Ipv4Addr::BROADCAST);
    all.into_iter()
        .map(|b| std::net::SocketAddr::from((b, port)))
        .collect()
}

/// The directed broadcast address of every usable local IPv4 interface.
pub(super) fn local_broadcasts() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => v4.broadcast,
            _ => None,
        })
        .collect()
}

/// The probe datagram. The same one a mobile client sends, so every responder in
/// the field already understands it.
pub(super) fn probe_datagram() -> Vec<u8> {
    serde_json::to_vec(&json!({ "type": UDP_PROBE_TYPE, "v": UDP_PROTOCOL_VERSION }))
        .unwrap_or_default()
}

impl LanDiscovery {
    /// Bind the discovery socket and run [`Self::udp_peer_loop_on`] over it. A
    /// failed bind (most likely another local instance holds the port) is
    /// logged at debug and ends the task; mDNS is unaffected.
    pub(super) async fn udp_peer_loop(self: Arc<Self>, cancel: oneshot::Receiver<()>) {
        let socket = match UdpSocket::bind(("0.0.0.0", DESKTOP_DISCOVERY_PORT)).await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    port = DESKTOP_DISCOVERY_PORT,
                    "UDP peer discovery not started (bind failed, likely another local instance \
                     holds this port): {e}"
                );
                return;
            }
        };
        if let Err(e) = socket.set_broadcast(true) {
            tracing::warn!(error = %e, "UDP peer discovery: cannot enable broadcast, not starting");
            return;
        }
        tracing::debug!(
            port = DESKTOP_DISCOVERY_PORT,
            "UDP peer discovery listening"
        );
        let targets = || probe_targets(local_broadcasts(), DESKTOP_DISCOVERY_PORT);
        self.udp_peer_loop_on(socket, targets, PROBE_INTERVAL, cancel)
            .await;
    }

    /// The loop proper, over a socket and target list the caller supplies so a
    /// test can drive two instances on loopback without the production port.
    pub(super) async fn udp_peer_loop_on(
        self: Arc<Self>,
        socket: UdpSocket,
        targets: impl Fn() -> Vec<std::net::SocketAddr>,
        every: std::time::Duration,
        mut cancel: oneshot::Receiver<()>,
    ) {
        let own_port = socket.local_addr().map(|a| a.port()).unwrap_or(0);
        let probe = probe_datagram();
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut buf = [0u8; 1024];
        loop {
            tokio::select! {
                _ = &mut cancel => {
                    tracing::debug!("UDP peer discovery stopping");
                    return;
                }
                _ = ticker.tick() => {
                    for target in targets() {
                        // An interface that is down, or a host with no route to
                        // a broadcast address, fails here; the others still go.
                        if let Err(e) = socket.send_to(&probe, target).await {
                            tracing::debug!(%target, error = %e, "UDP peer probe send failed");
                        }
                    }
                }
                result = socket.recv_from(&mut buf) => match result {
                    Ok((len, src)) => self.handle_datagram(&socket, own_port, &buf[..len], src).await,
                    Err(e) => {
                        // A persistent error (Windows reports an ICMP
                        // port-unreachable from our own earlier send as
                        // WSAECONNRESET) must not spin this loop.
                        tracing::debug!("UDP peer discovery recv_from error: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                },
            }
        }
    }

    async fn handle_datagram(
        &self,
        socket: &UdpSocket,
        own_port: u16,
        bytes: &[u8],
        src: std::net::SocketAddr,
    ) {
        // Our own broadcast comes back to our own socket on most stacks.
        let own_addrs = crate::backend::lan_listeners::cached_local_addresses();
        if src.port() == own_port && own_addrs.contains(&src.ip()) {
            return;
        }
        if is_valid_probe(bytes) {
            // Same trust boundary as the 47891 responder: the reply carries
            // `auth_key`, so only a private-range source gets one.
            if !is_lan_source(&src) {
                return;
            }
            // Identity only, without the 47891 reply's `siblings`: a desktop
            // reads replies into a 1024-byte buffer, which siblings could
            // overflow, and finds a host's other channels over mDNS anyway.
            if let Ok(payload) = serde_json::to_vec(&self.build_identity_response()) {
                if let Err(e) = socket.send_to(&payload, src).await {
                    tracing::debug!(%src, error = %e, "UDP peer reply send failed");
                }
            }
            return;
        }
        let Some(peer) = parse_peer_reply(bytes, &src) else {
            return;
        };
        // Not `peer.instance_id == self.instance_id`: that id is the release
        // (`v<version>`), so every other machine on our release would have been
        // ignored. "Us" is our own address with our own port, checked below.
        let own_addr_list = [peer.address];
        if is_self_resolution(peer.port, &own_addr_list, self.port, &own_addrs) {
            return;
        }
        let now = agentmux_common::time::now_secs_u64();
        let outcome = merge_udp_peer(&mut self.instances.write(), &peer, now);
        match outcome {
            Merge::Inserted => {
                tracing::info!(
                    peer_id = %peer.instance_id,
                    address = %peer.address,
                    port = peer.port,
                    "LAN peer discovered (UDP broadcast)"
                );
                self.broadcast_instances();
            }
            Merge::Refreshed => {}
            Merge::Dropped => {
                tracing::debug!(%src, "UDP peer table full, reply ignored");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(ip: &str) -> std::net::SocketAddr {
        format!("{ip}:{DESKTOP_DISCOVERY_PORT}").parse().unwrap()
    }

    fn reply(id: &str, port: u64) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type": UDP_RESPONSE_TYPE, "v": 1, "instance_id": id, "hostname": "area54",
            "version": "0.59.4", "port": port, "auth_key": "k-secret",
        }))
        .unwrap()
    }

    fn peer(id: &str, addr: &str, port: u16) -> UdpPeer {
        UdpPeer {
            instance_id: id.into(),
            hostname: "h".into(),
            version: "0.59.4".into(),
            channel: "stable".into(),
            address: addr.parse().unwrap(),
            port,
            auth_key: "k".into(),
        }
    }

    fn mdns_entry(id: &str, addr: &str, port: u16) -> LanInstance {
        LanInstance {
            instance_id: id.into(),
            hostname: "mdns-name".into(),
            version: "0.59.4".into(),
            channel: String::new(),
            address: addr.into(),
            port,
            auth_key: "mdns-key".into(),
            agents: vec!["korp".into()],
            first_seen: 1,
            last_seen: 1,
            last_polled_ok: 0,
            other_ttl_secs: 4500,
        }
    }

    #[test]
    fn a_well_formed_reply_from_a_lan_address_is_a_peer() {
        let p = parse_peer_reply(&reply("abc", 29700), &src("192.168.1.26")).unwrap();
        assert_eq!(p.instance_id, "abc");
        assert_eq!(p.address, "192.168.1.26".parse::<IpAddr>().unwrap());
        assert_eq!(p.port, 29700);
        assert_eq!(p.auth_key, "k-secret");
    }

    #[test]
    fn the_channel_is_read_when_present_and_optional_when_not() {
        let mut v: serde_json::Value = serde_json::from_slice(&reply("abc", 29700)).unwrap();
        assert_eq!(
            parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &src("192.168.1.26"))
                .unwrap()
                .channel,
            ""
        );
        v["channel"] = json!("dev-fix-lan");
        assert_eq!(
            parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &src("192.168.1.26"))
                .unwrap()
                .channel,
            "dev-fix-lan"
        );
        // Oversize: dropped, the reply still counts.
        v["channel"] = json!("c".repeat(MAX_CHANNEL_LEN + 1));
        assert_eq!(
            parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &src("192.168.1.26"))
                .unwrap()
                .channel,
            ""
        );
    }

    #[test]
    fn the_address_is_the_datagrams_source_never_a_claimed_one() {
        let mut v: serde_json::Value = serde_json::from_slice(&reply("abc", 29700)).unwrap();
        v["address"] = json!("10.9.9.9");
        let p = parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &src("192.168.1.26")).unwrap();
        assert_eq!(p.address.to_string(), "192.168.1.26");
    }

    #[test]
    fn replies_that_are_not_trustworthy_enough_to_record_are_refused() {
        let ok = src("192.168.1.26");
        // Not from the private ranges: a public host must not seed the table.
        assert!(parse_peer_reply(&reply("abc", 29700), &src("8.8.8.8")).is_none());
        // A probe, noise, another version.
        assert!(parse_peer_reply(&probe_datagram(), &ok).is_none());
        assert!(parse_peer_reply(b"not json", &ok).is_none());
        let mut v: serde_json::Value = serde_json::from_slice(&reply("abc", 29700)).unwrap();
        v["v"] = json!(2);
        assert!(parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &ok).is_none());
        // Privileged or out-of-range ports.
        assert!(parse_peer_reply(&reply("abc", 80), &ok).is_none());
        assert!(parse_peer_reply(&reply("abc", 70000), &ok).is_none());
        // Missing or empty identity or key, and oversize fields.
        assert!(parse_peer_reply(&reply("", 29700), &ok).is_none());
        for field in ["instance_id", "auth_key", "hostname", "version", "port"] {
            let mut v: serde_json::Value = serde_json::from_slice(&reply("abc", 29700)).unwrap();
            v.as_object_mut().unwrap().remove(field);
            assert!(
                parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &ok).is_none(),
                "{field}"
            );
        }
        let mut v: serde_json::Value = serde_json::from_slice(&reply("abc", 29700)).unwrap();
        v["auth_key"] = json!("k".repeat(MAX_AUTH_KEY_LEN + 1));
        assert!(parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &ok).is_none());
        v["auth_key"] = json!("");
        assert!(parse_peer_reply(&serde_json::to_vec(&v).unwrap(), &ok).is_none());
    }

    #[test]
    fn a_new_peer_is_recorded_under_a_udp_key_with_a_short_ttl() {
        let mut t = HashMap::new();
        assert_eq!(
            merge_udp_peer(&mut t, &peer("abc", "192.168.1.26", 29700), 100),
            Merge::Inserted
        );
        let e = &t["udp:192.168.1.26:29700"];
        assert_eq!(
            (e.last_seen, e.first_seen, e.other_ttl_secs),
            (100, 100, UDP_PEER_TTL_SECS)
        );
        assert!(e.agents.is_empty());
    }

    #[test]
    fn a_peer_mdns_already_knows_is_not_listed_twice() {
        // By address and port, whatever the TXT record says or has not said yet.
        for known in [
            mdns_entry("abc", "192.168.1.26", 29700),
            mdns_entry("", "192.168.1.26", 29700),
        ] {
            let mut t = HashMap::new();
            t.insert("agentmux-x._agentmux._tcp.local.".to_string(), known);
            assert_eq!(
                merge_udp_peer(&mut t, &peer("abc", "192.168.1.26", 29700), 500),
                Merge::Refreshed
            );
            assert_eq!(t.len(), 1, "no second entry");
            let e = &t["agentmux-x._agentmux._tcp.local."];
            assert_eq!(e.last_seen, 500, "liveness moves");
            assert_eq!(e.hostname, "mdns-name", "mDNS data is authoritative");
            assert_eq!(e.auth_key, "mdns-key");
        }
    }

    #[test]
    fn a_reply_without_a_channel_keeps_the_known_one() {
        // Codex P2 on #4241: an older build (or an oversize field) sends none.
        let mut t = HashMap::new();
        merge_udp_peer(&mut t, &peer("abc", "192.168.1.26", 29700), 100);
        assert_eq!(t["udp:192.168.1.26:29700"].channel, "stable");
        let mut no_channel = peer("abc", "192.168.1.26", 29700);
        no_channel.channel = String::new();
        merge_udp_peer(&mut t, &no_channel, 200);
        assert_eq!(t["udp:192.168.1.26:29700"].channel, "stable");
    }

    #[test]
    fn a_later_reply_updates_a_udp_entry_in_place() {
        let mut t = HashMap::new();
        merge_udp_peer(&mut t, &peer("abc", "192.168.1.26", 29700), 100);
        let mut renamed = peer("abc", "192.168.1.26", 29700);
        renamed.hostname = "area54-renamed".into();
        assert_eq!(merge_udp_peer(&mut t, &renamed, 200), Merge::Refreshed);
        assert_eq!(t.len(), 1);
        let e = &t["udp:192.168.1.26:29700"];
        assert_eq!((e.hostname.as_str(), e.last_seen), ("area54-renamed", 200));
    }

    /// Codex P1 on #4241: the instance id is the release (`v<version>`), so two
    /// machines on the same release send the same one. They are two peers.
    #[test]
    fn two_machines_on_the_same_release_are_two_peers() {
        let mut t = HashMap::new();
        assert_eq!(
            merge_udp_peer(&mut t, &peer("v0.59.5", "192.168.1.26", 29700), 1),
            Merge::Inserted
        );
        assert_eq!(
            merge_udp_peer(&mut t, &peer("v0.59.5", "192.168.1.195", 29700), 1),
            Merge::Inserted
        );
        assert_eq!(t.len(), 2);
        // And an mDNS entry for one of them does not swallow the other.
        t.insert(
            "mdns-a".into(),
            mdns_entry("v0.59.5", "192.168.1.26", 29700),
        );
        drop_udp_duplicates(&mut t);
        assert!(t.contains_key("udp:192.168.1.195:29700"));
        assert!(!t.contains_key("udp:192.168.1.26:29700"));
    }

    #[test]
    fn the_udp_peer_table_is_bounded() {
        let mut t = HashMap::new();
        for n in 0..MAX_UDP_PEERS {
            assert_eq!(
                merge_udp_peer(
                    &mut t,
                    &peer(
                        &format!("id{n}"),
                        &format!("10.0.{}.{}", n / 200, n % 200 + 1),
                        29700
                    ),
                    1
                ),
                Merge::Inserted
            );
        }
        assert_eq!(
            merge_udp_peer(&mut t, &peer("one-too-many", "10.1.0.1", 29700), 1),
            Merge::Dropped
        );
        // A peer already in the table still refreshes at the cap.
        assert_eq!(
            merge_udp_peer(&mut t, &peer("id0", "10.0.0.1", 29700), 2),
            Merge::Refreshed
        );
        // mDNS entries are not counted against the UDP cap.
        t.insert("mdns".into(), mdns_entry("m", "10.2.0.1", 29700));
        assert_eq!(
            t.keys().filter(|k| k.starts_with("udp:")).count(),
            MAX_UDP_PEERS
        );
    }

    #[test]
    fn an_mdns_resolution_after_the_udp_one_replaces_it() {
        let mut t = HashMap::new();
        merge_udp_peer(&mut t, &peer("abc", "192.168.1.26", 29700), 100);
        merge_udp_peer(&mut t, &peer("other", "192.168.1.40", 29700), 100);
        t.insert(
            "agentmux-x._agentmux._tcp.local.".to_string(),
            mdns_entry("abc", "192.168.1.26", 29700),
        );
        drop_udp_duplicates(&mut t);
        assert!(!t.contains_key("udp:192.168.1.26:29700"));
        assert!(
            t.contains_key("udp:192.168.1.40:29700"),
            "an unrelated UDP peer stays"
        );
        assert!(t.contains_key("agentmux-x._agentmux._tcp.local."));
    }

    #[test]
    fn probes_go_to_the_limited_and_every_directed_broadcast_once() {
        let t = probe_targets(
            [
                "192.168.1.255".parse().unwrap(),
                "172.17.31.255".parse().unwrap(),
                "192.168.1.255".parse().unwrap(),
                "127.255.255.255".parse().unwrap(),
            ],
            DESKTOP_DISCOVERY_PORT,
        );
        let t: Vec<String> = t.iter().map(|a| a.to_string()).collect();
        assert_eq!(
            t,
            [
                "172.17.31.255:29700",
                "192.168.1.255:29700",
                "255.255.255.255:29700"
            ]
        );
        // No interface at all still probes the limited broadcast.
        assert_eq!(probe_targets([], 29700).len(), 1);
    }

    #[test]
    fn the_probe_is_one_every_responder_already_accepts() {
        assert!(is_valid_probe(&probe_datagram()));
    }

    #[test]
    fn the_desktop_port_is_in_the_fixed_block() {
        assert!(crate::backend::lan_ports::LAN_PORT_RANGE.contains(&DESKTOP_DISCOVERY_PORT));
        assert!(
            DESKTOP_DISCOVERY_PORT < 32768,
            "below every OS's ephemeral range"
        );
    }

    /// A `LanDiscovery` with a real (idle) mDNS daemon but none of `start()`'s
    /// registration or tasks, as `handle_event_tests` builds one.
    fn instance(id: &str, host: &str, port: u16, key: &str) -> Arc<LanDiscovery> {
        Arc::new(LanDiscovery {
            daemon: ServiceDaemon::new().expect("daemon construction (no register/browse)"),
            instances: Arc::new(RwLock::new(HashMap::new())),
            instance_id: id.to_string(),
            event_bus: Arc::new(crate::backend::eventbus::EventBus::new()),
            service_fullname: String::new(),
            auth_key: key.to_string(),
            hostname: host.to_string(),
            version: "0.59.4".to_string(),
            port,
            udp_cancel: Mutex::new(None),
            udp_peer_cancel: Mutex::new(None),
            agent_names_cancel: Mutex::new(None),
            instance_record_cancel: Mutex::new(None),
            instances_dir: None,
            instance_record_live: Mutex::new(false),
            announced_v4: Arc::new(Mutex::new(BTreeSet::new())),
            monitored: true,
            started_at: std::time::Instant::now(),
        })
    }

    /// The whole route, with mDNS out of the picture: two instances probe each
    /// other over loopback sockets and each ends up with exactly one entry for
    /// the other, carrying what it advertised.
    #[tokio::test]
    async fn two_instances_find_each_other_by_probe_alone() {
        let a = instance("id-a", "host-a", 55001, "key-a");
        let b = instance("id-b", "host-b", 55002, "key-b");
        let sa = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sb = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (addr_a, addr_b) = (sa.local_addr().unwrap(), sb.local_addr().unwrap());
        let (cancel_a, rx_a) = oneshot::channel();
        let (cancel_b, rx_b) = oneshot::channel();
        let every = std::time::Duration::from_millis(50);
        // Each also probes its own address, as a real broadcast comes back to the
        // sender's own socket.
        tokio::spawn(
            a.clone()
                .udp_peer_loop_on(sa, move || vec![addr_b, addr_a], every, rx_a),
        );
        tokio::spawn(
            b.clone()
                .udp_peer_loop_on(sb, move || vec![addr_a, addr_b], every, rx_b),
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (seen_by_a, seen_by_b) = loop {
            let (x, y) = (a.get_instances(), b.get_instances());
            if (!x.is_empty() && !y.is_empty()) || std::time::Instant::now() > deadline {
                break (x, y);
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        };
        let _ = (cancel_a.send(()), cancel_b.send(()));

        assert_eq!(seen_by_a.len(), 1, "a sees b once: {seen_by_a:?}");
        assert_eq!(seen_by_b.len(), 1, "b sees a once: {seen_by_b:?}");
        let b_as_seen = &seen_by_a[0];
        assert_eq!(
            (
                b_as_seen.instance_id.as_str(),
                b_as_seen.hostname.as_str(),
                b_as_seen.port,
                b_as_seen.auth_key.as_str()
            ),
            ("id-b", "host-b", 55002, "key-b")
        );
        assert_eq!(
            b_as_seen.address, "127.0.0.1",
            "the address is the datagram's source"
        );
        assert_eq!(seen_by_b[0].instance_id, "id-a");
        // Neither listed itself, which its own broadcast would have caused.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            a.get_instances().iter().all(|i| i.instance_id != "id-a"),
            "a listed itself"
        );
        assert!(
            b.get_instances().iter().all(|i| i.instance_id != "id-b"),
            "b listed itself"
        );
    }

    /// "Us" is our own address serving our own port, not our instance id: that
    /// id is the release, shared by every machine on it (Codex P1 on #4241).
    #[tokio::test]
    async fn a_machine_on_our_release_is_a_peer_and_our_own_address_is_us() {
        let a = instance("v0.59.5", "host-a", 55001, "key-a");
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let own_port = socket.local_addr().unwrap().port();
        // Same instance id as ours (the release), from another machine: a peer.
        let from: std::net::SocketAddr = "192.168.1.77:29700".parse().unwrap();
        a.handle_datagram(&socket, own_port, &reply("v0.59.5", 29799), from)
            .await;
        assert_eq!(
            a.get_instances().len(),
            1,
            "another machine on our release is not us"
        );
        // Our own address serving our own port: us, never a peer.
        if let Some(own) = crate::backend::lan_listeners::cached_local_addresses()
            .iter()
            .find(|ip| ip.is_ipv4() && !ip.is_loopback())
        {
            let from_self = std::net::SocketAddr::new(*own, 29700);
            a.handle_datagram(&socket, own_port, &reply("v0.59.5", 55001), from_self)
                .await;
            assert_eq!(a.get_instances().len(), 1, "our own address and port is us");
        }
    }

    /// A UDP peer is listed for the staleness floor after its last reply and not
    /// a second longer, whatever `other_ttl_secs` says (ReAgent P2 on #4230: the
    /// constant used to claim 120 s while the floor made it 300).
    #[tokio::test]
    async fn a_udp_peer_is_listed_for_exactly_the_staleness_floor() {
        let a = instance("id-a", "host-a", 55001, "key-a");
        let now = agentmux_common::time::now_secs_u64();
        let floor = LAN_PEER_STALE_TIMEOUT_FLOOR_SECS;
        assert_eq!(
            u64::from(UDP_PEER_TTL_SECS),
            floor,
            "the constant states the real lifetime"
        );
        for (age, listed) in [(0, true), (floor - 5, true), (floor + 5, false)] {
            merge_udp_peer(
                &mut a.instances.write(),
                &peer("abc", "192.168.1.26", 29700),
                now - age,
            );
            assert_eq!(a.get_instances().len(), usize::from(listed), "age {age}s");
            a.instances.write().clear();
        }
    }

    /// A peer that restarts keeps its address and port and gets a new instance
    /// id (ReAgent P2 on #4230): the entry follows it, re-keyed, so id-based
    /// dedup compares against the live id and not a stale one.
    #[test]
    fn a_restarted_peer_keeps_one_entry_under_its_new_id() {
        let mut t = HashMap::new();
        merge_udp_peer(&mut t, &peer("v0.59.4", "192.168.1.26", 29700), 100);
        assert_eq!(
            merge_udp_peer(&mut t, &peer("v0.59.5", "192.168.1.26", 29700), 200),
            Merge::Refreshed
        );
        assert_eq!(t.len(), 1, "{:?}", t.keys().collect::<Vec<_>>());
        let e = &t["udp:192.168.1.26:29700"];
        assert_eq!(
            (e.instance_id.as_str(), e.first_seen, e.last_seen),
            ("v0.59.5", 100, 200)
        );
    }
}
