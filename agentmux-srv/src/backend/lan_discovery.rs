// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LAN instance discovery via mDNS/DNS-SD.
//!
//! Each AgentMux backend advertises itself as `_agentmux._tcp.local.` and
//! continuously browses for peers. Discovered instances are tracked in memory
//! and broadcast to frontend clients via EventBus.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

use super::eventbus::{EventBus, WSEventType};

const SERVICE_TYPE: &str = "_agentmux._tcp.local.";
const LAN_AGENT_CACHE_TTL_SECS: u64 = 60;
const LAN_PEER_QUERY_TIMEOUT_SECS: u64 = 2;

/// UDP broadcast discovery fallback (Layer 2), for LANs where mDNS multicast
/// is filtered (common on corporate/guest WiFi). Mobile clients broadcast a
/// small JSON probe to this port; any listening desktop instance unicasts a
/// response straight back to the sender's source address.
///
/// Port 47891 is picked from the "ephemeral-safe-but-memorable" range: above
/// both the well-known (0-1023) and IANA-registered (1024-49151) ranges, so
/// it never collides with a registered service, while still being a fixed,
/// easy-to-grep value in logs and firewall rules (unlike a random ephemeral
/// port, which a fixed-port broadcast probe cannot target).
const UDP_DISCOVERY_PORT: u16 = 47891;

/// Wire-protocol `type` value a probe datagram must carry.
const UDP_PROBE_TYPE: &str = "agentmux_discover";
/// Wire-protocol `type` value this responder replies with.
const UDP_RESPONSE_TYPE: &str = "agentmux_discover_response";
/// Wire-protocol version. Bump alongside the mobile client if the schema
/// changes; `is_valid_probe` rejects anything else.
const UDP_PROTOCOL_VERSION: u64 = 1;

/// Build the JSON response payload for a valid probe. Pure/free function
/// (no `&self`) so it is trivially testable without spinning up a full
/// `LanDiscovery` (which owns a real mDNS `ServiceDaemon`). `LanDiscovery`'s
/// production loop calls this via `build_probe_response`.
fn probe_response_json(
    instance_id: &str,
    hostname: &str,
    version: &str,
    port: u16,
    auth_key: &str,
) -> serde_json::Value {
    json!({
        "type": UDP_RESPONSE_TYPE,
        "v": UDP_PROTOCOL_VERSION,
        "instance_id": instance_id,
        "hostname": hostname,
        "version": version,
        "port": port,
        "auth_key": auth_key,
    })
}

/// Validate a received UDP datagram as an in-protocol discovery probe.
/// Anything that fails to parse as JSON, or does not carry the exact
/// `type`/`v` fields, is treated as noise (unrelated LAN traffic or a
/// future/older protocol version) and silently ignored by the caller.
fn is_valid_probe(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    value.get("type").and_then(|t| t.as_str()) == Some(UDP_PROBE_TYPE) && is_probe_version_match(&value)
}

/// Match the probe's `v` field against `UDP_PROTOCOL_VERSION`, accepting
/// either JSON integer or float encoding. Some JSON serializers (e.g. a
/// client whose numeric type is inferred as `double`) emit a whole number
/// like `1` as `1.0`; `Value::as_u64()` alone returns `None` for that
/// representation, which would silently drop an otherwise-conformant probe.
fn is_probe_version_match(value: &serde_json::Value) -> bool {
    let Some(v) = value.get("v") else {
        return false;
    };
    v.as_u64() == Some(UDP_PROTOCOL_VERSION) || v.as_f64() == Some(UDP_PROTOCOL_VERSION as f64)
}

/// Trust-boundary check for the UDP responder: is `addr` reachable only from
/// a private/link-local network, i.e. plausibly on this LAN? Unlike mDNS
/// (link-local multicast, structurally non-routable off-subnet), this socket
/// binds `0.0.0.0` and would otherwise answer *any* routable unicast probe
/// with `auth_key` — an internet-facing disclosure oracle / UDP reflection
/// vector if the port is ever reachable externally (port forward, hairpin
/// NAT). Mirrors the private/loopback/link-local classification used for the
/// opposite purpose (SSRF egress checks) in
/// `drone/executor/blocks/api.rs::is_reserved_v4`/`is_reserved_v6`, minus the
/// broadcast/unspecified/multicast cases that don't apply to a source addr.
fn is_lan_source(addr: &std::net::SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if v6.is_loopback()
                // unique-local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
            {
                return true;
            }
            // IPv4-mapped/-compatible v6 literals route to the embedded v4
            // address on most kernels — delegate so a private v4 source
            // wearing a v6 wrapper isn't misclassified as public.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local();
            }
            #[allow(deprecated)]
            if let Some(v4) = v6.to_ipv4() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local();
            }
            false
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanInstance {
    pub instance_id: String,
    pub hostname: String,
    pub version: String,
    pub address: String,
    pub port: u16,
    pub auth_key: String,
    pub agents: Vec<String>,
    pub first_seen: u64,
    pub last_seen: u64,
}

struct LanCacheEntry {
    /// `None` = negative cache entry: agent is not on any LAN peer.
    peer_url: Option<String>,
    auth_key: String,
    expires: std::time::Instant,
}

/// One in-flight `GET /agentmux/reactive/agent?id=…` against a single peer.
type PeerQueryOutcome = (String, String, Result<reqwest::Response, reqwest::Error>);

/// Query EVERY eligible LAN peer concurrently for `agent_id`, yielding results
/// as they arrive.
///
/// Replaces the sequential `for peer in &peers { … .await }` both lookups used
/// to run. That loop cost up to `LAN_PEER_QUERY_TIMEOUT_SECS × peer_count`
/// before a message could be delivered — 10s across five peers — and the
/// pathological ordering (a dead or slow peer ahead of the right one) is the
/// common case on a laptop network where stale peers linger until their mDNS
/// TTL expires. Fanning out makes the worst case ~one timeout regardless of
/// peer count.
///
/// Returns a stream so each caller keeps its OWN acceptance rule: `find_agent`
/// accepts any 2xx, while `find_agent_lan_pubkey` must keep trying after a 2xx
/// whose body carries no `lan_public_key` (that peer hosts the agent but hasn't
/// minted a key). A shared "first success wins" helper could not express both.
///
/// **Dropping the returned stream cancels every still-in-flight request** —
/// so a caller that `break`s on the first acceptable answer stops paying for
/// the rest, rather than merely ignoring them.
fn query_peers_concurrently<'a>(
    peers: &'a [LanInstance],
    agent_id: &'a str,
    http: &'a reqwest::Client,
) -> futures_util::stream::FuturesUnordered<impl std::future::Future<Output = PeerQueryOutcome> + 'a> {
    let futures = futures_util::stream::FuturesUnordered::new();
    for peer in peers {
        // Same eligibility rule the sequential loops used: a peer with no
        // address or no scoped lan_key can't be queried at all.
        if peer.address.is_empty() || peer.auth_key.is_empty() {
            continue;
        }
        let peer_url = format!("http://{}:{}", peer.address, peer.port);
        let auth_key = peer.auth_key.clone();
        futures.push(async move {
            let result = http
                .get(format!("{peer_url}/agentmux/reactive/agent"))
                .query(&[("id", agent_id)])
                .header("X-AuthKey", &auth_key)
                .timeout(std::time::Duration::from_secs(LAN_PEER_QUERY_TIMEOUT_SECS))
                .send()
                .await;
            (peer_url, auth_key, result)
        });
    }
    futures
}

/// Pure predicate behind `LanDiscovery::resolves_to_this_instance`, split out
/// so it can be tested without standing up an mDNS `ServiceDaemon`.
///
/// `any` rather than `all` on the address match: a resolution for our own
/// service can legitimately carry an address that interface enumeration missed
/// (they race), and requiring every address to be ours would let that one
/// stray entry resurrect the phantom. `any` cannot produce a false positive —
/// combined with the port equality, a genuine remote peer would have to be
/// listening on our port *at one of our own interface addresses*, which it
/// cannot be. Another AgentMux instance on this same host does share our
/// addresses, but never our port.
fn is_self_resolution(
    resolved_port: u16,
    resolved_addrs: &[IpAddr],
    own_port: u16,
    own_addrs: &std::collections::HashSet<IpAddr>,
) -> bool {
    if resolved_port != own_port || own_addrs.is_empty() {
        return false;
    }
    resolved_addrs.iter().any(|a| own_addrs.contains(a))
}

pub struct LanDiscovery {
    daemon: ServiceDaemon,
    instances: Arc<RwLock<HashMap<String, LanInstance>>>,
    instance_id: String,
    event_bus: Arc<EventBus>,
    service_fullname: String,
    auth_key: String,
    hostname: String,
    version: String,
    port: u16,
    /// Cancellation half for the UDP responder task spawned in `start()`.
    /// `None` once `shutdown()` has fired (or if the UDP socket never bound
    /// — see `spawn_udp_responder`). Guarded by a sync mutex since
    /// `shutdown()` is `&self` and called from both an explicit live-toggle
    /// path and `Drop`.
    udp_cancel: Mutex<Option<oneshot::Sender<()>>>,
}

/// Normalize an OS hostname into a valid mDNS host name by appending the
/// `.local.` suffix that mdns-sd's `ServiceInfo::new` requires. Idempotent
/// — already-normalized inputs pass through unchanged. We also strip any
/// trailing dot first so `"foo.local"` doesn't end up as `"foo.local..local."`.
fn mdns_hostname(os_hostname: &str) -> String {
    let trimmed = os_hostname.trim_end_matches('.');
    if trimmed.ends_with(".local") {
        format!("{trimmed}.")
    } else {
        format!("{trimmed}.local.")
    }
}

impl LanDiscovery {
    /// Start LAN discovery: register this instance and browse for peers.
    pub fn start(
        instance_id: String,
        hostname: String,
        version: String,
        port: u16,
        auth_key: String,
        event_bus: Arc<EventBus>,
    ) -> Result<Arc<Self>, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS daemon failed: {e}"))?;

        // Register this instance. mdns-sd requires the host name passed to
        // `ServiceInfo::new` to end with `.local.` — we always normalize so
        // a raw OS hostname like "claudius" becomes "claudius.local.".
        let service_name = format!("agentmux-{}", &instance_id);
        let host_name_mdns = mdns_hostname(&hostname);
        let properties = [
            ("version", version.as_str()),
            ("hostname", hostname.as_str()),
            ("instance_id", instance_id.as_str()),
            ("auth_key", auth_key.as_str()),
        ];
        // `""` alone does NOT mean "auto-detect" despite how that reads —
        // `AsIpAddrs for &str`'s own doc comment: "If the string is empty,
        // will return an empty set." Auto-detection is a SEPARATE opt-in:
        // `.enable_addr_auto()` tells the daemon to fill in (and keep
        // updated) this service's addresses from the host's own interfaces.
        // Without it, a service registered via `ServiceInfo::new(..., "",
        // ...)` has zero addresses and `addr_auto: false` (the struct's
        // hardcoded default) — `register()` still returns `Ok(())` (it just
        // enqueues a `Command::Register`, no synchronous validation), so
        // this silently produced a service that was never actually
        // discoverable by ANY peer, not even another mDNS client on the same
        // host. Caught by `a_registered_instance_is_discoverable_by_an_
        // independent_mdns_client` below.
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &service_name,
            &host_name_mdns,
            "",
            port,
            &properties[..],
        )
        .map_err(|e| format!("ServiceInfo creation failed: {e}"))?
        .enable_addr_auto();

        let service_fullname = service_info.get_fullname().to_string();

        daemon
            .register(service_info)
            .map_err(|e| format!("mDNS register failed: {e}"))?;

        // Browse for peers — keep the receiver for the event loop
        let browse_receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| format!("mDNS browse failed: {e}"))?;

        let instances = Arc::new(RwLock::new(HashMap::new()));

        let discovery = Arc::new(Self {
            daemon,
            instances: instances.clone(),
            instance_id: instance_id.clone(),
            event_bus: event_bus.clone(),
            service_fullname,
            auth_key,
            hostname,
            version,
            port,
            udp_cancel: Mutex::new(None),
        });

        // Spawn event receiver on a blocking thread to avoid starving the tokio runtime
        let disc = discovery.clone();
        tokio::task::spawn_blocking(move || {
            disc.event_loop(browse_receiver);
        });

        // Spawn the UDP broadcast-probe responder (Layer 2 fallback for
        // filtered mDNS). `UdpSocket::recv_from` is natively async, so this
        // uses `tokio::spawn` (not `spawn_blocking`, unlike the mdns-sd event
        // loop above whose receiver is a sync channel).
        let (cancel_tx, cancel_rx) = oneshot::channel();
        *discovery.udp_cancel.lock() = Some(cancel_tx);
        let disc_udp = discovery.clone();
        tokio::spawn(async move {
            disc_udp.udp_responder_loop(cancel_rx).await;
        });

        tracing::info!(
            instance_id = %instance_id,
            port = port,
            "LAN discovery started (mDNS)"
        );

        Ok(discovery)
    }

    /// Build the JSON response payload for this instance's identity — see
    /// `probe_response_json` for the pure field-assembly logic shared with
    /// tests.
    fn build_probe_response(&self) -> serde_json::Value {
        probe_response_json(
            &self.instance_id,
            &self.hostname,
            &self.version,
            self.port,
            &self.auth_key,
        )
    }

    /// UDP broadcast-probe responder loop (Layer 2 discovery fallback).
    ///
    /// Binds `0.0.0.0:UDP_DISCOVERY_PORT` and answers valid probes from
    /// private/link-local source addresses (see `is_lan_source`) with a
    /// unicast response back to the sender's `recv_from` source address.
    /// Deliberately does NOT set `SO_REUSEADDR`: this codebase supports
    /// running multiple AgentMux instances on one host simultaneously, and
    /// on Windows `SO_REUSEADDR` lets a second process silently steal a UDP
    /// port already owned by another — an inappropriate risk for a socket
    /// that hands out `auth_key`. If the bind fails (most likely because
    /// another local instance already holds the port), this task logs and
    /// exits quietly — mDNS discovery (already running via `event_loop`)
    /// is unaffected, matching the "never fail `start()` over this" contract.
    ///
    /// We do not call `set_broadcast(true)`: that flag is only required to
    /// *send* to a broadcast address, and this socket only receives (probes
    /// arrive as broadcast/subnet-broadcast datagrams addressed to us, which
    /// requires no special socket option on the receiving end) and replies
    /// with a plain unicast send back to the probe's source address.
    async fn udp_responder_loop(self: Arc<Self>, mut cancel_rx: oneshot::Receiver<()>) {
        let socket = match UdpSocket::bind(("0.0.0.0", UDP_DISCOVERY_PORT)).await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    port = UDP_DISCOVERY_PORT,
                    "UDP discovery responder not started (bind failed, likely a second \
                     local instance already holds this port): {e}"
                );
                return;
            }
        };

        tracing::debug!(port = UDP_DISCOVERY_PORT, "UDP discovery responder listening");

        let mut buf = [0u8; 1024];
        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    tracing::debug!("UDP discovery responder stopping");
                    break;
                }
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, src)) => {
                            // Noise (unrelated LAN UDP traffic, malformed
                            // packets) is expected on a live network — do
                            // not log per-packet above debug.
                            if !is_valid_probe(&buf[..len]) {
                                continue;
                            }
                            // Trust boundary: never hand out auth_key to a
                            // source address outside the private/link-local
                            // ranges, even if the packet is a well-formed
                            // probe (see is_lan_source doc comment).
                            if !is_lan_source(&src) {
                                tracing::debug!(
                                    src = %src,
                                    "UDP discovery probe from non-LAN source, ignoring"
                                );
                                continue;
                            }
                            tracing::debug!(src = %src, "UDP discovery probe received");
                            let response = self.build_probe_response();
                            match serde_json::to_vec(&response) {
                                Ok(payload) => {
                                    if let Err(e) = socket.send_to(&payload, src).await {
                                        tracing::debug!("UDP discovery response send failed: {e}");
                                    } else {
                                        tracing::debug!(src = %src, "UDP discovery probe answered");
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!("UDP discovery response serialize failed: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            // A persistent OS-level error (e.g. Windows
                            // WSAECONNRESET/10054 from an ICMP
                            // port-unreachable triggered by our own prior
                            // send_to) would otherwise spin this loop at
                            // full CPU with no delay between retries.
                            tracing::debug!("UDP discovery recv_from error: {e}");
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        }
                    }
                }
            }
        }
    }

    fn event_loop(&self, receiver: mdns_sd::Receiver<ServiceEvent>) {
        loop {
            match receiver.recv() {
                Ok(event) => self.handle_event(event),
                Err(_) => {
                    tracing::warn!("mDNS event receiver closed");
                    break;
                }
            }
        }
    }

    /// True when a resolved service is really this instance, reached over one
    /// of this host's own addresses.
    ///
    /// Both conditions are required. The port alone is far too weak (two hosts
    /// can land on the same ephemeral port), and an address alone is too weak
    /// while several AgentMux instances share a machine — but no remote peer
    /// can be listening on OUR port at an address that is OURS.
    fn resolves_to_this_instance(&self, info: &ServiceInfo) -> bool {
        // Port first, and BEFORE touching the address set. A resolution on a
        // different port cannot be us, and that is the overwhelmingly common
        // case (every genuine remote peer), so it must not pay for an address
        // lookup. Passing the set as an argument would defeat this — arguments
        // are evaluated eagerly, so the cheap check inside `is_self_resolution`
        // would come too late [reagent #3025 P2].
        if info.get_port() != self.port {
            return false;
        }
        let addrs: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
        is_self_resolution(
            info.get_port(),
            &addrs,
            self.port,
            &crate::backend::lan_listeners::cached_local_addresses(),
        )
    }

    fn handle_event(&self, event: ServiceEvent) {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let fullname = info.get_fullname().to_string();

                // Skip self. Compared by `fullname` (deterministic from
                // `service_name`, set once at registration — see `start()`)
                // rather than the TXT-record `instance_id` property: on this
                // machine's own virtual/link-local interfaces (WSL/Hyper-V/
                // VPN adapters), `enable_addr_auto()` re-fires
                // `ServiceResolved` far more often than fresh TXT data
                // arrives — most of those events resolve with an empty TXT
                // record (confirmed via `LAN peer discovered` log: ~96% of
                // events for this instance's own addresses logged
                // `peer_id=""`). Comparing on `instance_id` alone meant a
                // blank-TXT self-resolution was never recognized as self and
                // was inserted as a phantom peer that could never self-heal
                // (see the instance_id-preservation fix below for why).
                if fullname == self.service_fullname {
                    return;
                }

                // Belt-and-braces on the fullname check above, which was
                // measured NOT to be sufficient on 2026-09-06: this instance
                // was still inserting itself as a peer. The srv log showed
                // four `LAN peer discovered` events, all `peer_id=""`, all on
                // port 55019 — this instance's own port — at 192.168.1.230,
                // 172.23.176.1 and fe80::5c1e:c2bb:b5bc:9655, every one of
                // them an address of this very host. They collapse into a
                // single phantom map entry (the map is keyed by fullname), so
                // `DiscoverAgents` reported one LAN "peer" that was really us,
                // with empty agents/hostname/instance_id.
                //
                // Why the fullname check misses them is not established —
                // plausibly mdns-sd conflict-renaming when several AgentMux
                // instances share this host, or a re-registration under
                // `enable_addr_auto()`. Rather than guess at that, this checks
                // the thing we can state with certainty: a service on OUR port
                // at an address that belongs to THIS machine is us. A genuine
                // remote peer cannot satisfy both — it would have to be
                // reachable at one of our own interface addresses.
                //
                // Cost of the phantom, and why it is worth a second guard:
                // `find_agent` now fans out concurrently to every known peer,
                // so each LAN lookup spent a request asking ourselves a
                // question we had already answered locally.
                if self.resolves_to_this_instance(&info) {
                    tracing::debug!(
                        address = %info.get_addresses().iter().next().map(|a| a.to_string()).unwrap_or_default(),
                        port = info.get_port(),
                        "skipping mDNS resolution of this instance's own service"
                    );
                    return;
                }

                let peer_id = info
                    .get_property_val_str("instance_id")
                    .unwrap_or_default()
                    .to_string();

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let address = info
                    .get_addresses()
                    .iter()
                    .find(|a| matches!(a, IpAddr::V4(_)))
                    .or_else(|| info.get_addresses().iter().next())
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let hostname = info
                    .get_property_val_str("hostname")
                    .unwrap_or_default()
                    .to_string();
                let version = info
                    .get_property_val_str("version")
                    .unwrap_or_default()
                    .to_string();
                let auth_key = info
                    .get_property_val_str("auth_key")
                    .unwrap_or_default()
                    .to_string();

                let mut instances = self.instances.write();
                let entry = instances.entry(fullname).or_insert_with(|| LanInstance {
                    instance_id: peer_id.clone(),
                    hostname: hostname.clone(),
                    version: version.clone(),
                    address: address.clone(),
                    port: info.get_port(),
                    auth_key: auth_key.clone(),
                    agents: Vec::new(),
                    first_seen: now,
                    last_seen: now,
                });
                entry.last_seen = now;
                entry.address = address;
                entry.port = info.get_port();
                // TXT-derived fields only: `enable_addr_auto()` re-fires
                // `ServiceResolved` on every interface/address change, and
                // most of those re-fires resolve with an empty TXT record
                // (see the self-skip comment above) — apply a value only
                // when this event actually carried one, so a stale re-fire
                // never erases previously-known-good peer identity. Address/
                // port come from the SRV/A record, not TXT, and are always
                // populated, so they're safe to overwrite unconditionally.
                if !peer_id.is_empty() {
                    entry.instance_id = peer_id.clone();
                }
                if !hostname.is_empty() {
                    entry.hostname = hostname;
                }
                if !version.is_empty() {
                    entry.version = version;
                }
                if !auth_key.is_empty() {
                    entry.auth_key = auth_key;
                }
                drop(instances);

                tracing::info!(
                    peer_id = %peer_id,
                    address = %info.get_addresses().iter().next().map(|a| a.to_string()).unwrap_or_default(),
                    port = info.get_port(),
                    "LAN peer discovered"
                );

                self.broadcast_instances();
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                let removed = {
                    let mut instances = self.instances.write();
                    instances.remove(&fullname).is_some()
                };
                if removed {
                    tracing::info!(fullname = %fullname, "LAN peer removed");
                    self.broadcast_instances();
                }
            }
            _ => {}
        }
    }

    fn broadcast_instances(&self) {
        let instances: Vec<LanInstance> = self.instances.read().values().cloned().collect();
        self.event_bus.broadcast_event(&WSEventType {
            eventtype: "laninstances".to_string(),
            oref: String::new(),
            data: Some(json!(instances)),
        });
    }

    /// Get current list of discovered LAN peers (excludes self).
    pub fn get_instances(&self) -> Vec<LanInstance> {
        self.instances.read().values().cloned().collect()
    }

    /// Get peer count (excludes self).
    #[allow(dead_code)]
    pub fn peer_count(&self) -> usize {
        self.instances.read().len()
    }

    /// Stop the mDNS daemon and the UDP broadcast-probe responder.
    ///
    /// The mDNS side synchronously closes the daemon socket (UDP:5353),
    /// causing the `browse_receiver` to return Err and the event-loop thread
    /// spawned by `start()` to exit. The UDP responder is stopped by firing
    /// its cancellation channel, which the `tokio::select!` in
    /// `udp_responder_loop` is racing against `recv_from` — this is what
    /// makes a live setting toggle actually stop responding to probes, not
    /// just stop the mDNS advertisement.
    ///
    /// Required for live-disable to actually stop discovery: both background
    /// tasks hold their own `Arc<Self>` clone, so simply dropping the
    /// controller's Arc never reaches refcount zero and `Drop` does not
    /// run. Callers must invoke `shutdown()` before clearing their Arc.
    /// Idempotent — safe to call from both the explicit path and `Drop`.
    pub fn shutdown(&self) {
        if let Some(tx) = self.udp_cancel.lock().take() {
            // Ignore send errors: an `Err` here just means the responder
            // task already exited on its own (e.g. the bind failed), which
            // is a no-op we're happy with.
            let _ = tx.send(());
        }
        if let Err(e) = self.daemon.unregister(&self.service_fullname) {
            // Likely already unregistered; do not warn loudly.
            tracing::debug!("mDNS unregister returned: {e}");
        }
        if let Err(e) = self.daemon.shutdown() {
            tracing::debug!("mDNS daemon shutdown returned: {e}");
        }
    }
}

impl Drop for LanDiscovery {
    fn drop(&mut self) {
        // Fallback path. Under normal live-toggle flow the controller calls
        // `shutdown()` explicitly; this only fires for process exit, when
        // the event-loop thread has already terminated and the final Arc
        // is being released.
        self.shutdown();
    }
}

/// Controller for live start/stop of `LanDiscovery` in response to setting changes.
///
/// Owns the daemon slot plus the start arguments, so toggling
/// `network:lan_discovery` from the UI (or from an external edit of
/// `settings.json`) can start or stop the daemon without restarting the
/// process.
///
/// Spec: docs/specs/lan-discovery-toggle.md
pub struct LanDiscoveryController {
    slot: Arc<RwLock<Option<Arc<LanDiscovery>>>>,
    instance_id: String,
    hostname: String,
    version: String,
    port: u16,
    auth_key: String,
    event_bus: Arc<EventBus>,
    /// Short-lived cache mapping agent_id → (peer_url, auth_key). Entries expire
    /// after LAN_AGENT_CACHE_TTL_SECS to handle agent migration between peers.
    agent_cache: std::sync::RwLock<HashMap<String, LanCacheEntry>>,
    /// Separate cache mapping agent_id → LAN Ed25519 public key bytes.
    /// SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2 — deliberately its own
    /// map rather than folded into `agent_cache`: the two are keyed by the
    /// same agent_id but answer different questions ("which peer currently
    /// hosts this agent" churns on process restarts/migration; "what is
    /// this agent's public key" is effectively static for the agent's
    /// lifetime), and a lookup for one is not always paired with a lookup
    /// for the other (a message TO agent X needs `agent_cache`'s entry for
    /// X; verifying a message FROM agent Y needs `pubkey_cache`'s entry for
    /// Y — different agent_ids on the same inbound request).
    pubkey_cache: std::sync::RwLock<HashMap<String, LanPubkeyCacheEntry>>,
    /// reagentx P1 on the LAN signing PR: `find_agent_lan_pubkey`'s negative
    /// cache is keyed by agent_id, so a caller holding only the `lan_key`
    /// can force a fresh multi-peer fan-out on every single request just by
    /// sending a novel random `source_agent` each time — the cache never
    /// hits, and this expensive walk runs (in `verify_lan_signature`)
    /// BEFORE `Handler::inject_message`'s own rate limiter is ever reached.
    /// A simple global token bucket on the fan-out ITSELF (not per-agent_id
    /// — the abuse pattern is specifically about varying the id to dodge a
    /// per-id cache) bounds the damage regardless of how many distinct
    /// identities are probed.
    pubkey_lookup_limiter: std::sync::Mutex<LookupRateLimiter>,
}

struct LanPubkeyCacheEntry {
    /// `None` = negative cache entry: no peer has a LAN public key on file
    /// for this agent_id (never minted one, or genuinely unknown).
    public_key: Option<Vec<u8>>,
    expires: std::time::Instant,
}

/// Outcome of `find_agent_lan_pubkey`. Three states, not two — reagentx P0
/// follow-up on the LAN signing PR: collapsing "the lookup was skipped
/// (rate-limited)" into the same `None`/"not found" outcome as "genuinely
/// no peer has this key" let an attacker exhaust
/// `LAN_PUBKEY_LOOKUP_RATE_LIMIT` with junk lookups, then slip a forged
/// signature for a REAL agent's identity through as unverified (benign)
/// instead of a verification failure (forced sensitive). By the time this
/// function is ever called, `verify_lan_signature` has already confirmed a
/// `lan_sig` was actually presented — so "we didn't check" must be
/// distinguishable from "there was nothing to check."
pub(crate) enum LanPubkeyLookup {
    /// A peer has a public key on file for this agent_id.
    Found(Vec<u8>),
    /// No peer (that answered) has ever minted a LAN key for this agent_id
    /// — genuinely unknown sender, not a red flag on its own (same
    /// "unverified, not escalated" treatment self-declared senders already
    /// get elsewhere in this system).
    NotFound,
    /// The lookup was skipped by the rate limiter — unknown, NOT the same
    /// as `NotFound`. Callers verifying an actual signature attempt must
    /// treat this conservatively (as a failure), never as "nothing to
    /// check."
    RateLimited,
}

/// Minimal token bucket, refilled once per second — same shape as
/// `backend::reactive::handler::RateLimiter`, duplicated locally rather
/// than widening that one's visibility across modules for a single caller.
struct LookupRateLimiter {
    tokens: u32,
    max_tokens: u32,
    last_refill: std::time::Instant,
}

impl LookupRateLimiter {
    fn new(max_tokens: u32) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            last_refill: std::time::Instant::now(),
        }
    }

    fn check(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_refill) >= std::time::Duration::from_secs(1) {
            self.tokens = self.max_tokens;
            self.last_refill = now;
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

/// Fan-outs per second `find_agent_lan_pubkey` will perform before failing
/// closed (returning `None` without querying any peer). Generous for
/// legitimate traffic — LAN jekts to previously-unseen senders are rare
/// once agents have been talking a while, and the 60s positive/negative
/// cache absorbs repeat lookups for the SAME agent_id — while still
/// bounding the worst case to a small, fixed number of outbound requests
/// per second regardless of how many distinct agent_ids are probed.
const LAN_PUBKEY_LOOKUP_RATE_LIMIT: u32 = 10;

impl LanDiscoveryController {
    pub fn new(
        instance_id: String,
        hostname: String,
        version: String,
        port: u16,
        event_bus: Arc<EventBus>,
        auth_key: String,
    ) -> Self {
        Self {
            slot: Arc::new(RwLock::new(None)),
            instance_id,
            hostname,
            version,
            port,
            auth_key,
            event_bus,
            agent_cache: std::sync::RwLock::new(HashMap::new()),
            pubkey_cache: std::sync::RwLock::new(HashMap::new()),
            pubkey_lookup_limiter: std::sync::Mutex::new(LookupRateLimiter::new(LAN_PUBKEY_LOOKUP_RATE_LIMIT)),
        }
    }

    /// Query LAN peers for which one hosts `agent_id`. Returns `(peer_url,
    /// lan_key)` for the first peer that responds 2xx to the agent-lookup
    /// endpoint. Results — both positive and negative — are cached for
    /// `LAN_AGENT_CACHE_TTL_SECS` seconds to avoid a blocking peer fan-out on
    /// every inject for cloud-only agents.
    ///
    /// Security (SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md LAN P0-1):
    /// the value broadcast in the mDNS TXT record / UDP probe response
    /// (`self.auth_key` field name kept for wire back-compat with older
    /// peer versions — see `Config::lan_key`'s doc comment) is now a
    /// separate, narrowly-scoped credential, NOT the instance's full-access
    /// `auth_key`. A passive LAN listener who captures it gets standing
    /// access to only the two LAN-forwarding routes
    /// (`lan_or_full_auth_middleware` in `server/mod.rs`) — not the full
    /// `/agentmux/service` surface this used to expose. Broadcasting
    /// *something* in cleartext to the LAN is still an accepted trade-off
    /// of this opt-in feature (mDNS/UDP have no confidentiality of their
    /// own); this change is about shrinking what a captured value is worth,
    /// not about hiding it in transit — see the spec's LAN P1-1 for the
    /// separate, not-yet-implemented "encrypt/authenticate the transport
    /// too" hardening.
    pub async fn find_agent(
        &self,
        agent_id: &str,
        http: &reqwest::Client,
    ) -> Option<(String, String)> {
        // Fast path: valid cache hit (positive or negative)
        if let Ok(cache) = self.agent_cache.read() {
            if let Some(e) = cache.get(agent_id) {
                if e.expires > std::time::Instant::now() {
                    return e.peer_url.as_ref().map(|url| (url.clone(), e.auth_key.clone()));
                }
            }
        }

        // Slow path: fan out to every peer CONCURRENTLY and take the first
        // 2xx. Use reqwest's .query() for safe percent-encoding of the
        // agent_id (handles spaces, &, =, #, etc.) — see
        // `query_peers_concurrently`, which also explains why this is a stream
        // rather than a first-success-wins helper.
        let peers = self.get_instances();
        {
            use futures_util::StreamExt as _;
            let mut inflight = query_peers_concurrently(&peers, agent_id, http);
            while let Some((peer_url, auth_key, result)) = inflight.next().await {
                if matches!(result, Ok(ref r) if r.status().is_success()) {
                    tracing::debug!(agent_id, peer_url = %peer_url, "LAN agent found on peer");
                    if let Ok(mut cache) = self.agent_cache.write() {
                        cache.insert(
                            agent_id.to_string(),
                            LanCacheEntry {
                                peer_url: Some(peer_url.clone()),
                                auth_key: auth_key.clone(),
                                expires: std::time::Instant::now()
                                    + std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS),
                            },
                        );
                    }
                    // Dropping `inflight` here cancels the remaining requests.
                    return Some((peer_url, auth_key));
                }
            }
        }

        // No peer has this agent — write a negative cache entry so future
        // injects for cloud-only agents skip the full peer fan-out.
        if let Ok(mut cache) = self.agent_cache.write() {
            cache.insert(
                agent_id.to_string(),
                LanCacheEntry {
                    peer_url: None,
                    auth_key: String::new(),
                    expires: std::time::Instant::now()
                        + std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS),
                },
            );
        }
        None
    }

    /// Look up a claimed LAN sender's Ed25519 public key by fanning out to
    /// every discovered LAN peer, same query the target-agent lookup above
    /// makes (`GET /agentmux/reactive/agent?id=<agent_id>`, now extended —
    /// see `handle_reactive_agent` — to embed `lan_public_key` when the
    /// responding peer has minted one). A SEPARATE lookup from `find_agent`
    /// even though it hits the same endpoint: verifying an inbound message
    /// FROM agent Y needs Y's key, which is almost never the same agent_id
    /// as whatever THIS request's own target-agent lookup (if any) was for.
    ///
    /// Returns the raw decoded public key bytes, or `None` if no peer has
    /// one on file for this agent_id (cached negatively, same as
    /// `find_agent`'s "not found" case) — callers (`verify_lan_signature`)
    /// treat that identically to "no signature attempted": nothing to check
    /// against, not a red flag on its own.
    pub async fn find_agent_lan_pubkey(&self, agent_id: &str, http: &reqwest::Client) -> LanPubkeyLookup {
        if let Ok(cache) = self.pubkey_cache.read() {
            if let Some(e) = cache.get(agent_id) {
                if e.expires > std::time::Instant::now() {
                    return match &e.public_key {
                        Some(k) => LanPubkeyLookup::Found(k.clone()),
                        None => LanPubkeyLookup::NotFound,
                    };
                }
            }
        }

        // reagentx P1: fail closed on the FAN-OUT (no outbound peer
        // queries) rather than let an attacker force unbounded network
        // traffic by varying agent_id on every request — see
        // LAN_PUBKEY_LOOKUP_RATE_LIMIT's doc comment. Returning
        // `RateLimited` here (reagentx P0 follow-up, NOT the same as
        // `NotFound`) is the point: by the time this function is called at
        // all, `verify_lan_signature` has already confirmed a `lan_sig` WAS
        // presented, so "we didn't check" must never collapse into the same
        // outcome as "genuinely nothing to check" — a caller could
        // otherwise exhaust this limiter with junk lookups, then slip a
        // forged signature for a REAL agent's identity through unverified
        // instead of forced-sensitive.
        let allowed = self
            .pubkey_lookup_limiter
            .lock()
            .map(|mut limiter| limiter.check())
            .unwrap_or(true);
        if !allowed {
            tracing::debug!(agent_id, "LAN pubkey lookup rate-limited — skipping peer fan-out");
            return LanPubkeyLookup::RateLimited;
        }

        // Concurrent fan-out, same as `find_agent` — but note the acceptance
        // rule differs: a 2xx whose body has no `lan_public_key` means that
        // peer hosts the agent but hasn't minted a key, so we must keep
        // consuming the stream rather than stopping at the first 2xx.
        let peers = self.get_instances();
        {
            use futures_util::StreamExt as _;
            let mut inflight = query_peers_concurrently(&peers, agent_id, http);
            while let Some((peer_url, _auth_key, result)) = inflight.next().await {
                let Ok(resp) = result else { continue };
                if !resp.status().is_success() {
                    continue;
                }
                let Ok(body) = resp.json::<serde_json::Value>().await else { continue };
                let Some(pubkey_b64) = body.get("lan_public_key").and_then(|v| v.as_str()) else {
                    continue; // this peer has the agent but no LAN key minted for it yet
                };
                use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
                let Ok(pubkey_bytes) = BASE64.decode(pubkey_b64) else { continue };
                tracing::debug!(agent_id, peer_url = %peer_url, "LAN agent pubkey found on peer");
                if let Ok(mut cache) = self.pubkey_cache.write() {
                    cache.insert(
                        agent_id.to_string(),
                        LanPubkeyCacheEntry {
                            public_key: Some(pubkey_bytes.clone()),
                            expires: std::time::Instant::now()
                                + std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS),
                        },
                    );
                }
                // Dropping `inflight` here cancels the remaining requests.
                return LanPubkeyLookup::Found(pubkey_bytes);
            }
        }

        if let Ok(mut cache) = self.pubkey_cache.write() {
            cache.insert(
                agent_id.to_string(),
                LanPubkeyCacheEntry {
                    public_key: None,
                    expires: std::time::Instant::now()
                        + std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS),
                },
            );
        }
        LanPubkeyLookup::NotFound
    }

    /// Evict a stale cache entry (e.g. after a forward to that peer failed).
    pub fn evict_agent(&self, agent_id: &str) {
        if let Ok(mut cache) = self.agent_cache.write() {
            cache.remove(agent_id);
        }
    }

    /// Idempotent: starts the daemon when `enabled` and not running, stops it
    /// when `!enabled` and running. Re-entrant safe.
    ///
    /// Holds the slot's write lock for the entire check-and-modify transaction
    /// to avoid a TOCTOU race between the `is_running` read and the slot
    /// mutation. `apply()` is called from toggle clicks and setting writes —
    /// low frequency — so briefly blocking concurrent peer-list reads is
    /// acceptable. `LanDiscovery::start()` and `Drop` are both fast (mDNS
    /// daemon construction + service register/unregister are local socket ops).
    pub fn apply(&self, enabled: bool) {
        let mut slot = self.slot.write();
        let is_running = slot.is_some();
        match (enabled, is_running) {
            (true, false) => {
                match LanDiscovery::start(
                    self.instance_id.clone(),
                    self.hostname.clone(),
                    self.version.clone(),
                    self.port,
                    self.auth_key.clone(),
                    self.event_bus.clone(),
                ) {
                    Ok(d) => {
                        *slot = Some(d);
                        tracing::info!("LAN discovery enabled via setting");
                    }
                    Err(e) => {
                        tracing::warn!("LAN discovery start failed: {e}");
                        // Surface to the UI so the user sees why the toggle
                        // didn't take effect (e.g. Windows Firewall blocked).
                        // `e` is already a String, but `.to_string()` is the
                        // documented contract for the wire payload (frontend
                        // reads `event.data.error` as a string).
                        self.event_bus.broadcast_event(&WSEventType {
                            eventtype: "laninstances:error".to_string(),
                            oref: String::new(),
                            data: Some(json!({ "error": e.to_string() })),
                        });
                    }
                }
            }
            (false, true) => {
                // Explicitly shut down before dropping our Arc. The
                // spawn_blocking event-loop thread holds an `Arc<LanDiscovery>`
                // clone (see `start()` line ~94), so dropping the slot's Arc
                // alone does not reach refcount zero — `Drop` would never run
                // and the daemon would keep advertising/browsing. `shutdown()`
                // closes the mDNS socket synchronously, the receiver returns
                // Err, the event-loop exits, and the spawned thread releases
                // its Arc. `Drop`'s subsequent call to `shutdown()` is a
                // no-op (idempotent).
                if let Some(d) = slot.as_ref() {
                    d.shutdown();
                }
                *slot = None;
                tracing::info!("LAN discovery disabled via setting");
                self.event_bus.broadcast_event(&WSEventType {
                    eventtype: "laninstances".to_string(),
                    oref: String::new(),
                    data: Some(json!([])),
                });
            }
            _ => {}
        }
    }

    /// Read the current peer list. Returns empty when the daemon is not
    /// running (discovery disabled or start failed).
    pub fn get_instances(&self) -> Vec<LanInstance> {
        self.slot
            .read()
            .as_ref()
            .map(|d| d.get_instances())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_lan_source, is_valid_probe, mdns_hostname, probe_response_json, LanDiscovery,
        SERVICE_TYPE, UDP_PROBE_TYPE, UDP_PROTOCOL_VERSION, UDP_RESPONSE_TYPE,
    };
    use mdns_sd::ServiceEvent;
    use serde_json::json;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::net::UdpSocket;

    #[test]
    fn is_lan_source_accepts_private_v4_ranges() {
        for addr in [
            "127.0.0.1:1234",
            "10.0.0.5:1234",
            "172.16.4.4:1234",
            "192.168.1.50:1234",
            "169.254.1.1:1234", // link-local / APIPA
        ] {
            let addr: SocketAddr = addr.parse().unwrap();
            assert!(is_lan_source(&addr), "{addr} should be treated as LAN");
        }
    }

    #[test]
    fn is_lan_source_rejects_public_v4() {
        for addr in ["8.8.8.8:1234", "1.1.1.1:1234", "203.0.113.7:1234"] {
            let addr: SocketAddr = addr.parse().unwrap();
            assert!(!is_lan_source(&addr), "{addr} should NOT be treated as LAN");
        }
    }

    #[test]
    fn is_lan_source_accepts_v6_loopback_and_local_ranges() {
        for addr in ["[::1]:1234", "[fe80::1]:1234", "[fc00::1]:1234"] {
            let addr: SocketAddr = addr.parse().unwrap();
            assert!(is_lan_source(&addr), "{addr} should be treated as LAN");
        }
    }

    #[test]
    fn is_lan_source_rejects_public_v6() {
        let addr: SocketAddr = "[2001:4860:4860::8888]:1234".parse().unwrap();
        assert!(!is_lan_source(&addr));
    }

    #[test]
    fn is_lan_source_unwraps_v4_mapped_v6() {
        // A private v4 address wearing a v6 mapped-address wrapper must
        // still be classified by its embedded v4 range, not treated as an
        // opaque public v6 address.
        let addr: SocketAddr = "[::ffff:10.0.0.5]:1234".parse().unwrap();
        assert!(is_lan_source(&addr));
        let addr: SocketAddr = "[::ffff:8.8.8.8]:1234".parse().unwrap();
        assert!(!is_lan_source(&addr));
    }

    #[test]
    fn appends_local_dot_to_bare_hostname() {
        assert_eq!(mdns_hostname("claudius"), "claudius.local.");
    }

    #[test]
    fn preserves_already_fully_qualified_name() {
        assert_eq!(mdns_hostname("claudius.local."), "claudius.local.");
    }

    #[test]
    fn appends_trailing_dot_to_local_suffix() {
        // mdns-sd needs the trailing dot; we add it without doubling .local.
        assert_eq!(mdns_hostname("claudius.local"), "claudius.local.");
    }

    #[test]
    fn handles_trailing_dot_on_bare_hostname() {
        assert_eq!(mdns_hostname("claudius."), "claudius.local.");
    }

    #[test]
    fn does_not_double_suffix() {
        // Two passes through the normalizer produce the same result.
        let once = mdns_hostname("claudius");
        let twice = mdns_hostname(&once);
        assert_eq!(twice, once);
    }

    // -- UDP broadcast-probe wire protocol (Layer 2 discovery fallback) --

    #[test]
    fn is_valid_probe_accepts_well_formed_probe() {
        let bytes = serde_json::to_vec(&json!({"type": UDP_PROBE_TYPE, "v": 1})).unwrap();
        assert!(is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_rejects_non_json() {
        assert!(!is_valid_probe(b"not json at all"));
    }

    #[test]
    fn is_valid_probe_rejects_wrong_type() {
        let bytes = serde_json::to_vec(&json!({"type": "something_else", "v": 1})).unwrap();
        assert!(!is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_rejects_wrong_version() {
        let bytes = serde_json::to_vec(&json!({"type": UDP_PROBE_TYPE, "v": 2})).unwrap();
        assert!(!is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_accepts_version_encoded_as_float() {
        // A client whose JSON serializer infers a `double` type for the
        // version field emits `1.0` rather than `1` — both must validate.
        let bytes = serde_json::to_vec(&json!({"type": UDP_PROBE_TYPE, "v": 1.0})).unwrap();
        assert!(is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_rejects_version_encoded_as_non_integral_float() {
        let bytes = serde_json::to_vec(&json!({"type": UDP_PROBE_TYPE, "v": 1.5})).unwrap();
        assert!(!is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_rejects_unrelated_json() {
        // Simulates non-AgentMux JSON noise landing on the port.
        let bytes = serde_json::to_vec(&json!({"hello": "world"})).unwrap();
        assert!(!is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_rejects_missing_version_field() {
        let bytes = serde_json::to_vec(&json!({"type": UDP_PROBE_TYPE})).unwrap();
        assert!(!is_valid_probe(&bytes));
    }

    #[test]
    fn is_valid_probe_rejects_missing_type_field() {
        let bytes = serde_json::to_vec(&json!({"v": UDP_PROTOCOL_VERSION})).unwrap();
        assert!(!is_valid_probe(&bytes));
    }

    #[test]
    fn probe_response_json_populates_all_fields() {
        let response = probe_response_json("inst-1", "myhost", "1.2.3", 9999, "secret-key");
        assert_eq!(response["type"], UDP_RESPONSE_TYPE);
        assert_eq!(response["v"], UDP_PROTOCOL_VERSION);
        assert_eq!(response["instance_id"], "inst-1");
        assert_eq!(response["hostname"], "myhost");
        assert_eq!(response["version"], "1.2.3");
        assert_eq!(response["port"], 9999);
        assert_eq!(response["auth_key"], "secret-key");
    }

    // -- Real mDNS registration round-trip (reagent-style investigation:
    //    LAN peer discovery was silently broken end-to-end — see below) --

    /// Registers a real LanDiscovery instance (exactly as `start()` does in
    /// production) and browses for it with a SEPARATE, independent
    /// `mdns_sd::ServiceDaemon` — mirroring what a real peer's browse-side
    /// would see. This is the one test in this module that touches the real
    /// network stack (multicast on 5353) — it's slow (~3s) but it's the only
    /// way to catch a registration bug that a browse-only or probe-only unit
    /// test can't reach.
    ///
    /// This test is the reason the `""` (empty string) IP passed to
    /// `ServiceInfo::new` in `start()` was found to be broken: mdns-sd's own
    /// doc comment on `AsIpAddrs for &str` says plainly "If the string is
    /// empty, will return an empty set" — NOT "auto-detect". Combined with
    /// `ServiceInfo::new` hardcoding `addr_auto: false` and `start()` never
    /// calling the builder's `.enable_addr_auto()`, every registered service
    /// had zero addresses and no instruction to ever fill them in — it was
    /// never actually discoverable, not by a real peer, not even by another
    /// mDNS client on the SAME machine. `register()` itself has no
    /// synchronous validation for this (it just enqueues a `Command::
    /// Register` and returns `Ok`), so `start()` logged "LAN discovery
    /// started" successfully every time despite never actually working.
    ///
    /// IGNORED by default (opt in with `cargo test -- --ignored`) —
    /// confirmed environment-fragile in two independent, unrelated ways, not
    /// just "flaky CI networking":
    ///
    /// 1. GitHub Actions' `macos-latest` runner doesn't support UDP
    ///    multicast at all: `mDNSResponder` is disabled there
    ///    (actions/runner-images#9628) and macOS 15+ runners separately lack
    ///    the "Local Network" permission sandboxed processes need for
    ///    multicast (actions/runner-images discussion #170669). Confirmed
    ///    deterministic (3/3 nightly runs), not intermittent.
    /// 2. Fails on real macOS hardware too, outside any CI — reproduced with
    ///    a minimal from-scratch `mdns-sd` program (no AgentMux code at all)
    ///    on a dev machine with several active VPN/tunnel interfaces: two
    ///    `ServiceDaemon`s in the *same process*, both sharing port 5353
    ///    with the OS's own `mDNSResponder`, and the browse side received
    ///    zero packets — not even `ServiceFound`. The OS-level `dns-sd`
    ///    CLI (two separate *processes*) discovers the identical
    ///    registration instantly on the same machine, so this isn't a
    ///    broken network or firewall; it's specific to same-process
    ///    multi-`ServiceDaemon` port-5353 sharing in this crate.
    ///
    /// Both failure modes are outside this codebase's control (GitHub's
    /// runner sandboxing; mdns-sd's own same-process daemon behavior), so
    /// this stays a real, valuable, manually-run integration check —
    /// exactly how the bug above was actually caught — rather than
    /// something CI can gate on. See
    /// docs/retro/retro-macos-ci-mdns-multicast-unsupported-2026-08-12.md.
    #[tokio::test]
    #[ignore = "environment-fragile: real multicast, not gateable in GH Actions macOS CI \
                (no multicast support there) or reliably on multi-interface dev machines \
                (same-process ServiceDaemon port-5353 sharing) — see retro doc"]
    async fn a_registered_instance_is_discoverable_by_an_independent_mdns_client() {
        let event_bus = Arc::new(crate::backend::eventbus::EventBus::new());
        let instance_id = format!("test-{}", std::process::id());
        let discovery = LanDiscovery::start(
            instance_id.clone(),
            "test-host".to_string(),
            "0.0.0-test".to_string(),
            54321,
            "test-auth-key".to_string(),
            event_bus,
        )
        .expect("LanDiscovery::start should succeed");

        // Independent daemon — a stand-in for a real peer's browse side.
        // Deliberately NOT the same ServiceDaemon `discovery` uses; a
        // same-process shortcut (e.g. reading `discovery`'s own in-memory
        // `instances` map) wouldn't exercise the actual wire announcement.
        let browser_daemon = mdns_sd::ServiceDaemon::new().expect("browser daemon");
        let receiver = browser_daemon.browse(SERVICE_TYPE).expect("browse");

        let mut found = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(event) = receiver.recv_timeout(std::time::Duration::from_millis(200)) {
                if let ServiceEvent::ServiceResolved(info) = event {
                    if info.get_property_val_str("instance_id") == Some(instance_id.as_str()) {
                        found = true;
                        break;
                    }
                }
            }
        }

        let _ = browser_daemon.shutdown();
        discovery.shutdown();

        assert!(
            found,
            "an independent mDNS client must be able to discover a freshly-registered LanDiscovery instance within 5s"
        );
    }

    /// End-to-end probe/response round trip over real loopback sockets
    /// (ephemeral ports, NOT the fixed UDP_DISCOVERY_PORT — avoids CI port
    /// collisions and avoids needing a full `LanDiscovery` + real mDNS
    /// `ServiceDaemon` just to exercise the wire format). Exercises the same
    /// `is_valid_probe` / `probe_response_json` functions the production
    /// `udp_responder_loop` calls.
    #[tokio::test]
    async fn udp_probe_round_trip_returns_valid_response() {
        let responder = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let responder_addr = responder.local_addr().unwrap();
        let prober = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let probe = json!({"type": UDP_PROBE_TYPE, "v": UDP_PROTOCOL_VERSION});
        prober
            .send_to(&serde_json::to_vec(&probe).unwrap(), responder_addr)
            .await
            .unwrap();

        let mut buf = [0u8; 1024];
        let (len, src) = responder.recv_from(&mut buf).await.unwrap();
        assert!(is_valid_probe(&buf[..len]));

        let response = probe_response_json("inst-1", "myhost", "1.2.3", 9999, "secret-key");
        responder
            .send_to(&serde_json::to_vec(&response).unwrap(), src)
            .await
            .unwrap();

        let (len, _) = prober.recv_from(&mut buf).await.unwrap();
        let received: serde_json::Value = serde_json::from_slice(&buf[..len]).unwrap();
        assert_eq!(received["type"], UDP_RESPONSE_TYPE);
        assert_eq!(received["v"], UDP_PROTOCOL_VERSION);
        assert_eq!(received["instance_id"], "inst-1");
        assert_eq!(received["hostname"], "myhost");
        assert_eq!(received["version"], "1.2.3");
        assert_eq!(received["port"], 9999);
        assert_eq!(received["auth_key"], "secret-key");
    }
}

// -- `handle_event` self-skip + TXT-clobber regression tests --
//
// See docs/specs/SPEC_LAN_DISCOVERY_TXT_CLOBBER_FIX_2026_08_16.md. Root
// cause: `enable_addr_auto()` makes mdns-sd re-fire `ServiceResolved` for
// the SAME service far more often than fresh TXT data actually arrives —
// live logs showed ~96% of resolution events for this instance's own
// virtual/link-local addresses carrying an empty TXT record while
// address/port were always populated. These tests exercise
// `LanDiscovery::handle_event` directly against hand-built `ServiceInfo`
// values (no real daemon register/browse — construction of a
// `ServiceDaemon` is required only to satisfy the struct field, exactly
// as the existing ignored round-trip test above already relies on being
// safe) so they run fast and are not subject to that test's documented
// multicast flakiness.
#[cfg(test)]
mod handle_event_tests {
    use super::{LanDiscovery, LanInstance, SERVICE_TYPE};
    use mdns_sd::{ServiceEvent, ServiceInfo};
    use parking_lot::{Mutex, RwLock};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Build a `ServiceInfo` for `instance_id`, mirroring the exact
    /// `service_name`/host-name shape `LanDiscovery::start()` registers
    /// with, so `get_fullname()` matches what production code would
    /// compute for the same `instance_id`.
    fn test_service_info(instance_id: &str, port: u16, properties: &[(&str, &str)]) -> ServiceInfo {
        let service_name = format!("agentmux-{instance_id}");
        ServiceInfo::new(
            SERVICE_TYPE,
            &service_name,
            "test-host.local.",
            "127.0.0.1",
            port,
            properties,
        )
        .expect("ServiceInfo::new should succeed for a well-formed test fixture")
    }

    /// A `LanDiscovery` whose `service_fullname` is derived the same way
    /// production's `start()` derives it (from a real `ServiceInfo` for
    /// `self_instance_id`), so the self-skip comparison in `handle_event`
    /// is exercised exactly as it runs in production — not asserted
    /// against a hand-typed guess at mdns-sd's fullname format.
    fn test_discovery(self_instance_id: &str) -> LanDiscovery {
        let self_info = test_service_info(self_instance_id, 0, &[]);
        LanDiscovery {
            daemon: mdns_sd::ServiceDaemon::new().expect("daemon construction (no register/browse)"),
            instances: Arc::new(RwLock::new(HashMap::new())),
            instance_id: self_instance_id.to_string(),
            event_bus: Arc::new(crate::backend::eventbus::EventBus::new()),
            service_fullname: self_info.get_fullname().to_string(),
            auth_key: String::new(),
            hostname: String::new(),
            version: String::new(),
            port: 0,
            udp_cancel: Mutex::new(None),
        }
    }

    fn get(instances: &[LanInstance], instance_id: &str) -> Option<LanInstance> {
        instances.iter().find(|i| i.instance_id == instance_id).cloned()
    }

    #[test]
    fn self_resolution_with_blank_txt_is_never_inserted_as_a_peer() {
        // This is the exact failure mode from the live logs: mdns-sd
        // re-resolves this instance's own service on a virtual/link-local
        // interface and the TXT record (instance_id/hostname/version/
        // auth_key) comes back empty. Skipping by `fullname` (SRV-record
        // derived, always present) rather than the TXT `instance_id`
        // property must catch this even though the property itself is
        // blank on this event.
        let discovery = test_discovery("self-id");
        let blank_self_event = test_service_info("self-id", 56023, &[]);

        discovery.handle_event(ServiceEvent::ServiceResolved(blank_self_event));

        assert!(
            discovery.get_instances().is_empty(),
            "a blank-TXT resolution of this instance's own service must never appear as a peer"
        );
    }

    #[test]
    fn self_resolution_with_full_txt_is_also_never_inserted_as_a_peer() {
        // Sanity check that the fullname-based skip doesn't regress the
        // straightforward case (this was already correct before the fix).
        let discovery = test_discovery("self-id");
        let full_self_event =
            test_service_info("self-id", 56023, &[("instance_id", "self-id"), ("hostname", "myhost")]);

        discovery.handle_event(ServiceEvent::ServiceResolved(full_self_event));

        assert!(discovery.get_instances().is_empty());
    }

    #[test]
    fn blank_txt_refire_does_not_clobber_previously_known_good_peer_fields() {
        let discovery = test_discovery("self-id");

        let full_event = test_service_info(
            "peer-1",
            9999,
            &[
                ("instance_id", "peer-1"),
                ("hostname", "realhost"),
                ("version", "1.2.3"),
                ("auth_key", "secret"),
            ],
        );
        discovery.handle_event(ServiceEvent::ServiceResolved(full_event));

        let after_full = get(&discovery.get_instances(), "peer-1").expect("peer-1 should be discovered");
        assert_eq!(after_full.hostname, "realhost");
        assert_eq!(after_full.version, "1.2.3");
        assert_eq!(after_full.auth_key, "secret");

        // Same service (same fullname), a re-resolution with an empty TXT
        // record — exactly what mdns-sd's `enable_addr_auto()` produces on
        // most re-fires per the live-log evidence.
        let blank_refire = test_service_info("peer-1", 9999, &[]);
        discovery.handle_event(ServiceEvent::ServiceResolved(blank_refire));

        let instances = discovery.get_instances();
        assert_eq!(instances.len(), 1, "the blank re-fire must update the existing entry, not create a duplicate");
        let after_blank = get(&instances, "peer-1").expect("peer-1 must still be present");
        assert_eq!(after_blank.hostname, "realhost", "hostname must survive a blank-TXT re-fire");
        assert_eq!(after_blank.version, "1.2.3", "version must survive a blank-TXT re-fire");
        assert_eq!(after_blank.auth_key, "secret", "auth_key must survive a blank-TXT re-fire");
        assert_eq!(after_blank.instance_id, "peer-1", "instance_id must survive a blank-TXT re-fire");
    }

    #[test]
    fn address_and_port_still_update_unconditionally_on_a_blank_txt_refire() {
        // address/port come from the SRV/A record, not TXT, and are always
        // populated by mdns-sd — a genuine interface/address change must
        // still be reflected even when the TXT-derived fields are absent
        // on that same event.
        let discovery = test_discovery("self-id");

        let first = test_service_info("peer-1", 9999, &[("instance_id", "peer-1"), ("hostname", "realhost")]);
        discovery.handle_event(ServiceEvent::ServiceResolved(first));
        assert_eq!(get(&discovery.get_instances(), "peer-1").unwrap().port, 9999);

        let moved = test_service_info("peer-1", 8888, &[]);
        discovery.handle_event(ServiceEvent::ServiceResolved(moved));

        let instances = discovery.get_instances();
        let entry = get(&instances, "peer-1").expect("peer-1 must still be present");
        assert_eq!(entry.port, 8888, "port must update even on a blank-TXT event");
        assert_eq!(entry.hostname, "realhost", "hostname must still survive despite the address/port change");
    }

    #[test]
    fn a_new_peer_first_seen_via_blank_txt_has_no_data_to_preserve() {
        // Not a regression this fix is responsible for solving — merely
        // documents the expected (acceptable) behavior for a genuinely
        // first-seen blank-TXT event: nothing was ever known, so nothing
        // can be preserved. hostname stays empty until a TXT-bearing event
        // eventually arrives.
        let discovery = test_discovery("self-id");
        let blank_first = test_service_info("peer-2", 7777, &[]);

        discovery.handle_event(ServiceEvent::ServiceResolved(blank_first));

        let instances = discovery.get_instances();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].hostname, "");
        assert_eq!(instances[0].instance_id, "");
    }
}

#[cfg(test)]
mod lookup_rate_limiter_tests {
    use super::LookupRateLimiter;

    #[test]
    fn allows_up_to_max_tokens_per_window() {
        let mut limiter = LookupRateLimiter::new(3);
        assert!(limiter.check());
        assert!(limiter.check());
        assert!(limiter.check());
        assert!(!limiter.check(), "a 4th check within the same second must be denied");
    }

    #[test]
    fn refills_after_the_window_elapses() {
        let mut limiter = LookupRateLimiter::new(1);
        assert!(limiter.check());
        assert!(!limiter.check());
        // Simulate the refill window having passed rather than sleeping in
        // a test — same approach as the existing RATE_LIMIT_MAX tests use
        // conceptually, just directly on the struct field since this type
        // has no injectable clock.
        limiter.last_refill -= std::time::Duration::from_secs(2);
        assert!(limiter.check(), "must refill once a full second has elapsed");
    }
}

#[cfg(test)]
mod peer_fanout_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Spawn a minimal HTTP peer that waits `delay` before answering `status`.
    /// Returns its `127.0.0.1:port`. Same raw-TCP shape as the mock servers in
    /// `muxbus::pkce`'s tests — no extra dev-dependency needed.
    async fn mock_peer(delay: std::time::Duration, status: &'static str, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    tokio::time::sleep(delay).await;
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("127.0.0.1:{}", addr.port())
    }

    fn peer_at(addr: &str) -> LanInstance {
        let (host, port) = addr.rsplit_once(':').unwrap();
        LanInstance {
            instance_id: format!("peer-{port}"),
            hostname: "test".into(),
            version: "0".into(),
            address: host.to_string(),
            port: port.parse().unwrap(),
            auth_key: "k".into(),
            agents: vec![],
            first_seen: 0,
            last_seen: 0,
        }
    }

    /// The regression this fan-out exists to prevent: a slow peer listed BEFORE
    /// the peer that actually has the agent must not delay the answer.
    ///
    /// Sequentially this took `LAN_PEER_QUERY_TIMEOUT_SECS` (2s) to time the
    /// slow peer out before even trying the second one. Concurrently the fast
    /// peer answers immediately. The 1500ms bound sits well clear of both the
    /// ~0ms concurrent path and the ≥2000ms sequential one, so it discriminates
    /// without being timing-fragile.
    #[tokio::test]
    async fn a_slow_peer_does_not_delay_a_fast_one() {
        let slow = mock_peer(
            std::time::Duration::from_secs(LAN_PEER_QUERY_TIMEOUT_SECS + 5),
            "200 OK",
            "{}",
        )
        .await;
        let fast = mock_peer(std::time::Duration::from_millis(0), "200 OK", "{}").await;
        // Slow first — the ordering that defeated the sequential loop.
        let peers = vec![peer_at(&slow), peer_at(&fast)];
        let http = reqwest::Client::new();

        let started = std::time::Instant::now();
        let mut inflight = query_peers_concurrently(&peers, "agent-x", &http);
        let mut winner = None;
        {
            use futures_util::StreamExt as _;
            while let Some((peer_url, _key, result)) = inflight.next().await {
                if matches!(result, Ok(ref r) if r.status().is_success()) {
                    winner = Some(peer_url);
                    break;
                }
            }
        }
        let elapsed = started.elapsed();

        assert_eq!(
            winner.as_deref(),
            Some(format!("http://{fast}").as_str()),
            "the fast peer should win even though the slow peer was queried first"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(1500),
            "fan-out should not wait on the slow peer; took {elapsed:?}"
        );
    }

    /// Peers with no address or no scoped lan_key are not queryable and must be
    /// skipped — same eligibility rule the sequential loops applied.
    #[tokio::test]
    async fn ineligible_peers_are_skipped() {
        let mut no_addr = peer_at("127.0.0.1:1");
        no_addr.address = String::new();
        let mut no_key = peer_at("127.0.0.1:2");
        no_key.auth_key = String::new();
        let peers = vec![no_addr, no_key];
        let http = reqwest::Client::new();
        let inflight = query_peers_concurrently(&peers, "agent-x", &http);
        assert_eq!(inflight.len(), 0, "neither peer is eligible to be queried");
    }
}

/// `is_self_resolution` — the guard that stops this instance inserting itself
/// as a LAN peer.
///
/// The scenario these encode was measured on 2026-09-06, not imagined: four
/// `LAN peer discovered` events, all `peer_id=""`, all on this instance's own
/// port 55019, at 192.168.1.230 / 172.23.176.1 / fe80::5c1e:c2bb:b5bc:9655 —
/// every one an address of this host. The pre-existing fullname-based self-skip
/// did not catch them.
#[cfg(test)]
mod self_resolution_tests {
    use super::is_self_resolution;
    use std::collections::HashSet;
    use std::net::IpAddr;

    fn own() -> HashSet<IpAddr> {
        ["192.168.1.230", "172.23.176.1", "fe80::5c1e:c2bb:b5bc:9655", "127.0.0.1"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// The exact observed case, one per address the log showed.
    #[test]
    fn our_own_addresses_on_our_own_port_are_self() {
        for addr in ["192.168.1.230", "172.23.176.1", "fe80::5c1e:c2bb:b5bc:9655"] {
            assert!(
                is_self_resolution(55019, &[ip(addr)], 55019, &own()),
                "{addr}:55019 is this host on this instance's port — must be skipped"
            );
        }
    }

    /// A real peer: different machine, so its addresses are not ours. Sharing
    /// our ephemeral port by coincidence must NOT be enough to discard it —
    /// that would silently drop a genuine peer, which is far worse than the
    /// phantom this guard exists to remove.
    #[test]
    fn a_remote_peer_on_a_coincidentally_equal_port_is_not_self() {
        assert!(!is_self_resolution(55019, &[ip("192.168.1.77")], 55019, &own()));
    }

    /// Another AgentMux instance on THIS host does share our addresses — it is
    /// the port that distinguishes it. Two instances never hold the same port.
    #[test]
    fn a_sibling_instance_on_this_host_is_not_self() {
        assert!(
            !is_self_resolution(51095, &[ip("192.168.1.230")], 55019, &own()),
            "same host, different port — a real sibling instance, not us"
        );
    }

    /// `any`, not `all`: interface enumeration and mDNS resolution race, so a
    /// resolution of our own service can carry one address we didn't enumerate.
    /// Requiring every address to match would let that stray entry resurrect
    /// the phantom.
    #[test]
    fn one_matching_address_is_enough_even_beside_an_unknown_one() {
        assert!(is_self_resolution(
            55019,
            &[ip("10.99.99.99"), ip("172.23.176.1")],
            55019,
            &own()
        ));
    }

    /// Enumeration failing means "we cannot prove any address is ours". Degrade
    /// to the old behaviour (a phantom peer) rather than to something worse —
    /// an empty set must never make everything look like self.
    #[test]
    fn an_empty_own_set_never_claims_self() {
        assert!(!is_self_resolution(55019, &[ip("192.168.1.230")], 55019, &HashSet::new()));
    }

    #[test]
    fn a_resolution_with_no_addresses_is_not_self() {
        assert!(!is_self_resolution(55019, &[], 55019, &own()));
    }
}
