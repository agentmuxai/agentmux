# SPEC: Desktops find each other without mDNS — UDP broadcast peer discovery

**Date:** 2026-10-02
**Status:** implemented — PR #4230; not yet verified on two real machines (section 5).
**Author:** AgentX (narko), at the owner's request
**Affects:** `crates/srv/src/backend/lan_discovery.rs`, `crates/srv/src/backend/lan_discovery/udp_peers.rs` (new), `crates/srv/src/backend/lan_firewall.rs` (the ports LAN needs)
**Builds on:** `docs/specs/SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` (§4.7 item 5 asks for this; §9 names it PR E)

## 1. Why

Two desktops had exactly one way to find each other: mDNS on UDP 5353. On Area54 (0.59.4, 2026-10-02) that route was held by another program for minutes at a time (Chrome's network service, then the Windows DNS Client service), so Area54 was reachable by TCP on 29700 and silent on mDNS, and narko never saw it. The recovery work in #4148 and #4196 detects that and keeps retrying, which fixes the cases where the port frees up. It cannot help when the port never frees up, or on a network that filters multicast (guest Wi-Fi), where mDNS fails for reasons outside the process.

The UDP responder on 47891 already answers a broadcast probe with the instance's identity. It was written for mobile clients, so nothing on a desktop ever sends that probe.

## 2. Design

One more discovery route, running whenever LAN discovery is on, that uses the probe and the reply that already exist.

1. **A desktop discovery socket**, UDP `29700` (`DESKTOP_DISCOVERY_PORT`), bound on `0.0.0.0` with broadcast enabled. 29700 is in the fixed block of §4.1, below every OS's ephemeral range, so one firewall rule names it. UDP and TCP numbers are separate namespaces, so it can share a number with a TCP listener in the block. The 47891 responder is unchanged, because the mobile client hardcodes it.
2. **Probing.** Every 30 seconds the socket sends the existing probe (`{"type":"agentmux_discover","v":1}`) to the limited broadcast address and to the directed broadcast of every non-loopback IPv4 interface. The limited broadcast alone leaves by one interface on Windows, the wrong one on a host with a VPN or virtual adapters (narko has four).
3. **Answering.** A probe from a private-range source gets the same identity payload the 47891 responder sends (`probe_response_json`). The two paths share the trust check (`is_lan_source`).
4. **Learning.** A reply becomes a peer: address is the datagram's source (never a claimed address), port, instance id, hostname, version and key come from the payload. Probing and answering share one socket on purpose, so a peer's reply lands on a port the firewall rule names and not on an ephemeral one.
5. **One entry per peer.** `instances` is keyed by mDNS service name, and `find_agent` queries every entry, so a peer listed twice would double every lookup. A UDP peer is keyed `udp:<instance id>`. If an entry with the same instance id, or the same address and port, exists (mDNS data may not have its TXT yet), the reply only refreshes `last_seen`: the mDNS entry stays authoritative. When mDNS resolves a peer that UDP found first, the UDP entry is dropped.
6. **Lifetime.** A UDP peer is kept 300 seconds after its last reply (ten probe cycles): the staleness floor every peer gets, below which `peer_staleness_window_secs` will not go. A test pins it. It is cancelled with the rest of LAN discovery on shutdown.

### 2.1 What it does not do

- **It adds no trust.** A reply is unauthenticated and network-claimed, exactly like an mDNS TXT record. Jekt signing and LAN key pinning are untouched, and a spoofed reply gets a place in the peer table, which is all a spoofed mDNS record gets.
- **One instance per host.** Like the 47891 responder, the socket does not set `SO_REUSEADDR` (on Windows that lets a second process take a port from the first, and the reply carries a key). The first instance on a host holds it; a later one (a `task dev` build beside an installed one) quietly does without. A host with one install is the case this is for.
- **No amplification control beyond the existing posture.** A probe is about 40 bytes and a reply about 200, to a private-range source only, the same as the 47891 responder. Spoofing a source inside the LAN needs a host on the LAN.

### 2.2 Limits on what a reply may claim

Private-range source (`is_lan_source`); `type`, version and the five fields present; `instance_id` and `auth_key` non-empty (a peer without a key cannot be queried, so there is nothing to record); `port` at least 1024; field lengths at most 128, 253, 64 and 256; a peer's own instance id or our own address and port are skipped; at most 64 UDP-learned peers at once.

## 3. Firewall

`lan_firewall::lan_needs` gains UDP `29700`, so the reader reports a firewall that blocks it. The Windows port rule of §4.1 becomes UDP `5353,29700,47891`. A machine whose rule is the program rule the OS prompt creates already covers it.

## 4. Tests

`udp_peers.rs`: reply validation (accepted, and each refusal), source-address-not-claimed, no duplicate for a peer mDNS knows (by id and by address and port), in-place update, the peer cap, the mDNS-after-UDP replacement, probe targets, and that the probe is one every responder accepts. A two-instance exchange on loopback drives the real loop end to end. The firewall tests include the new port.

## 5. Verification still to do

On two real machines where one cannot use 5353: narko with a program holding UDP 5353 (a Chrome window reproduces it), and Area54 or starpower as the peer. Expected: the peer appears in the LAN list within one probe interval, and the indicator moves off `undiscoverable` to `peers`.
