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
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::eventbus::{EventBus, WSEventType};

const SERVICE_TYPE: &str = "_agentmux._tcp.local.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanInstance {
    pub instance_id: String,
    pub hostname: String,
    pub version: String,
    pub address: String,
    pub port: u16,
    pub agents: Vec<String>,
    pub first_seen: u64,
    pub last_seen: u64,
}

pub struct LanDiscovery {
    daemon: ServiceDaemon,
    instances: Arc<RwLock<HashMap<String, LanInstance>>>,
    instance_id: String,
    event_bus: Arc<EventBus>,
    service_fullname: String,
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
        ];
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &service_name,
            &host_name_mdns,
            "",  // empty = auto-detect IP
            port,
            &properties[..],
        )
        .map_err(|e| format!("ServiceInfo creation failed: {e}"))?;

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
        });

        // Spawn event receiver on a blocking thread to avoid starving the tokio runtime
        let disc = discovery.clone();
        tokio::task::spawn_blocking(move || {
            disc.event_loop(browse_receiver);
        });

        tracing::info!(
            instance_id = %instance_id,
            port = port,
            "LAN discovery started (mDNS)"
        );

        Ok(discovery)
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

                let fullname = info.get_fullname().to_string();
                let mut instances = self.instances.write();
                let entry = instances.entry(fullname).or_insert_with(|| LanInstance {
                    instance_id: peer_id.clone(),
                    hostname: hostname.clone(),
                    version: version.clone(),
                    address: address.clone(),
                    port: info.get_port(),
                    agents: Vec::new(),
                    first_seen: now,
                    last_seen: now,
                });
                entry.last_seen = now;
                entry.hostname = hostname;
                entry.version = version;
                entry.address = address;
                entry.port = info.get_port();
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

    /// Stop the mDNS daemon — synchronously closes the daemon socket
    /// (UDP:5353), causing the `browse_receiver` to return Err and the
    /// event-loop thread spawned by `start()` to exit.
    ///
    /// Required for live-disable to actually stop discovery: the event-loop
    /// thread holds its own `Arc<Self>` clone, so simply dropping the
    /// controller's Arc never reaches refcount zero and `Drop` does not
    /// run. Callers must invoke `shutdown()` before clearing their Arc.
    /// Idempotent — safe to call from both the explicit path and `Drop`.
    pub fn shutdown(&self) {
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
    event_bus: Arc<EventBus>,
}

impl LanDiscoveryController {
    pub fn new(
        instance_id: String,
        hostname: String,
        version: String,
        port: u16,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            slot: Arc::new(RwLock::new(None)),
            instance_id,
            hostname,
            version,
            port,
            event_bus,
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
    use super::mdns_hostname;

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
}
