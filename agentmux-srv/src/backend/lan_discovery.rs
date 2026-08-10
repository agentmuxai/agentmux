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

    fn handle_event(&self, event: ServiceEvent) {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let peer_id = info
                    .get_property_val_str("instance_id")
                    .unwrap_or_default()
                    .to_string();

                // Skip self
                if peer_id == self.instance_id {
                    return;
                }

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

                let fullname = info.get_fullname().to_string();
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
                entry.hostname = hostname;
                entry.version = version;
                entry.address = address;
                entry.port = info.get_port();
                entry.auth_key = auth_key;
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
/// Spec: specs/lan-discovery-toggle.md
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
}

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
        }
    }

    /// Query LAN peers for which one hosts `agent_id`. Returns `(peer_url,
    /// auth_key)` for the first peer that responds 2xx to the agent-lookup
    /// endpoint. Results — both positive and negative — are cached for
    /// `LAN_AGENT_CACHE_TTL_SECS` seconds to avoid a blocking peer fan-out on
    /// every inject for cloud-only agents.
    ///
    /// Security: `auth_key` is broadcast in the mDNS TXT record. This is
    /// intentional and matches the same trust assumption as tier-2 loopback
    /// forwarding — LAN traffic is trusted (private network). Anyone on the LAN
    /// who can already intercept mDNS multicast can intercept the HTTP traffic
    /// too, so the key adds no exposure beyond what already exists.
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

        // Slow path: query each peer. Use reqwest's .query() for safe
        // percent-encoding of the agent_id (handles spaces, &, =, #, etc.).
        let peers = self.get_instances();
        for peer in &peers {
            if peer.address.is_empty() || peer.auth_key.is_empty() {
                continue;
            }
            let peer_url = format!("http://{}:{}", peer.address, peer.port);
            let result = http
                .get(format!("{}/agentmux/reactive/agent", peer_url))
                .query(&[("id", agent_id)])
                .header("X-AuthKey", &peer.auth_key)
                .timeout(std::time::Duration::from_secs(LAN_PEER_QUERY_TIMEOUT_SECS))
                .send()
                .await;
            if matches!(result, Ok(ref r) if r.status().is_success()) {
                tracing::debug!(agent_id, peer_url = %peer_url, "LAN agent found on peer");
                if let Ok(mut cache) = self.agent_cache.write() {
                    cache.insert(
                        agent_id.to_string(),
                        LanCacheEntry {
                            peer_url: Some(peer_url.clone()),
                            auth_key: peer.auth_key.clone(),
                            expires: std::time::Instant::now()
                                + std::time::Duration::from_secs(LAN_AGENT_CACHE_TTL_SECS),
                        },
                    );
                }
                return Some((peer_url, peer.auth_key.clone()));
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
    /// network stack (multicast on 5353); it's slow (~3s) and, like any mDNS
    /// test, has a small flakiness ceiling on a hostile CI network — but it's
    /// the only way to catch a registration bug that a browse-only or
    /// probe-only unit test can't reach.
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
    #[tokio::test]
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
