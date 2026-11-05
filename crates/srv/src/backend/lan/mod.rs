// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Everything the local network does: finding other AgentMux instances (mDNS and UDP
//! discovery), the listeners and ports srv opens for them, the firewall check, and the
//! swarm broadcast to the peers found.

pub mod discovery;
pub mod firewall;
#[cfg(windows)]
pub mod firewall_windows;
pub mod listeners;
pub mod mdns_diag;
pub mod mdns_health;
pub mod peer_words;
pub mod ports;
pub mod swarm_broadcast;
