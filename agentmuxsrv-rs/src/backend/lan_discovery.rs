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

        // Register this instance
        let service_name = format!("agentmux-{}", &instance_id);
        let properties = [
            ("version", version.as_str()),
            ("hostname", hostname.as_str()),
            ("instance_id", instance_id.as_str()),
        ];
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &service_name,
            &hostname,
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
    pub fn peer_count(&self) -> usize {
        self.instances.read().len()
    }
}

impl Drop for LanDiscovery {
    fn drop(&mut self) {
        // Gracefully unregister from mDNS
        if let Err(e) = self.daemon.unregister(&self.service_fullname) {
            tracing::warn!("mDNS unregister failed: {e}");
        }
        if let Err(e) = self.daemon.shutdown() {
            tracing::warn!("mDNS daemon shutdown failed: {e}");
        }
    }
}
