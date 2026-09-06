// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LAN-facing listener management.
//!
//! Background: `bootstrap.rs` resolves the srv bind address **once at
//! startup** — `0.0.0.0` when `network:lan_discovery` is on, `127.0.0.1`
//! otherwise. Toggling the setting at runtime therefore starts/stops mDNS
//! (live, via `LanDiscoveryController::apply`) but leaves the listeners where
//! they were, producing the worst possible state: peers discover this instance
//! and then cannot reach it. See
//! `docs/reports/REPORT_LAN_TOGGLE_WITHOUT_RESTART_2026_09_06.md` and
//! `docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md`.
//!
//! The fix does **not** require rebinding an active axum server (the reason
//! the in-code comment gave for deferring). `0.0.0.0:PORT` conflicts with
//! `127.0.0.1:PORT`, but a *specific* non-loopback address plus the same port
//! does not — so LAN reachability can be added by binding ADDITIONAL
//! listeners alongside the untouched loopback one, and removed by dropping
//! them.
//!
//! That claim is load-bearing for the whole design, so it is asserted by a
//! test here (`loopback_and_specific_ip_can_share_a_port`) rather than
//! assumed. It runs on every CI platform, so a platform where it does not
//! hold fails the build instead of shipping a silently broken toggle.

/// Best-effort discovery of this host's primary non-loopback IPv4 address.
///
/// Uses the standard "connect a UDP socket and read back its local address"
/// trick: `connect` on UDP sends no packets, it only fixes the socket's
/// peer so the kernel picks the outbound interface — which is exactly the
/// address a LAN peer would reach us on. Chosen over an interface-enumeration
/// crate deliberately: no new dependency, and it answers the question we
/// actually care about (which local address routes outward) rather than
/// handing back every virtual/container adapter to be filtered.
///
/// Returns `None` when there is no usable route (offline, or a CI container
/// with loopback only) — callers must treat that as "no LAN address today",
/// not as an error.
// Currently exercised only by this module's tests: it is the verified
// primitive the `LanListenerSupervisor` will use to pick bind addresses, and
// it lands with the spike so the assumption and the helper that depends on it
// are reviewed together. Remove this allow when the supervisor calls it.
#[allow(dead_code)]
pub fn primary_lan_ipv4() -> Option<std::net::IpAddr> {
    // 203.0.113.0/24 is TEST-NET-3 (RFC 5737) — reserved for documentation and
    // guaranteed never routable, so this cannot accidentally reach a real
    // host. No packet is sent either way; only the routing decision matters.
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("203.0.113.1:80").ok()?;
    let addr = sock.local_addr().ok()?.ip();
    if addr.is_loopback() || addr.is_unspecified() {
        return None;
    }
    Some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// **The one assumption the LAN-listener design rests on:** a specific
    /// non-loopback address can be bound on a port already held by loopback.
    ///
    /// That is what lets us become LAN-reachable by ADDING a listener, rather
    /// than rebinding the live server (which would drop the frontend's own
    /// loopback WebSocket) or changing the port (which would invalidate the
    /// mDNS TXT record and `authkey.dev`).
    ///
    /// Skips (rather than fails) when the host has no non-loopback address —
    /// an offline machine or loopback-only CI container can't exercise it, and
    /// failing there would be noise, not signal.
    #[test]
    fn loopback_and_specific_ip_can_share_a_port() {
        let Some(lan_ip) = primary_lan_ipv4() else {
            eprintln!("skipping: no non-loopback IPv4 on this host");
            return;
        };

        // Hold loopback on an ephemeral port, exactly as srv does today when
        // LAN discovery is off.
        let loopback = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = loopback.local_addr().unwrap().port();

        let lan = TcpListener::bind((lan_ip, port));
        assert!(
            lan.is_ok(),
            "binding {lan_ip}:{port} alongside 127.0.0.1:{port} must succeed \
             — the add-a-listener design depends on it; got {:?}",
            lan.err()
        );
    }

    /// Records a **platform difference**, deliberately without asserting a
    /// universal rule — measured, 2026-09-06.
    ///
    /// The design docs originally claimed `0.0.0.0:PORT` universally conflicts
    /// with an existing `127.0.0.1:PORT`. On Linux/macOS it does. **On Windows
    /// it does not** — absent `SO_EXCLUSIVEADDRUSE`, the wildcard bind
    /// succeeds alongside the loopback one. This test was written asserting
    /// the conflict and failed on Windows, which is how the claim was caught
    /// before it reached an implementation.
    ///
    /// Why it matters: it rules out "just bind the wildcard on toggle" as a
    /// portable shortcut. On Windows two sockets would hold the same port with
    /// ambiguous accept behaviour — precisely the port-hijack hazard
    /// `SO_EXCLUSIVEADDRUSE` exists to prevent. Binding SPECIFIC addresses (the
    /// chosen design) has no such ambiguity on any platform.
    ///
    /// Asserts nothing about which way it goes; it exists to keep the observed
    /// behaviour visible and to fail loudly if it ever becomes uniform.
    #[test]
    fn wildcard_vs_loopback_conflict_is_platform_dependent() {
        let loopback = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = loopback.local_addr().unwrap().port();
        let wildcard = TcpListener::bind(("0.0.0.0".parse::<std::net::IpAddr>().unwrap(), port));

        let conflicts = wildcard.is_err();
        eprintln!(
            "0.0.0.0:{port} vs existing 127.0.0.1:{port} — conflicts: {conflicts} \
             (expected: true on Linux/macOS, false on Windows)"
        );

        #[cfg(windows)]
        assert!(
            !conflicts,
            "Windows was measured as ALLOWING the wildcard bind alongside \
             loopback; if it now conflicts, the platform note in \
             REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md \
             is stale"
        );
        #[cfg(not(windows))]
        assert!(
            conflicts,
            "expected the wildcard bind to conflict with loopback on this \
             platform; if it no longer does, the same report's platform note \
             is stale"
        );
    }

    /// `primary_lan_ipv4` must never hand back something unusable as a bind
    /// target — loopback or unspecified would silently produce a listener
    /// that adds no LAN reachability at all.
    #[test]
    fn primary_lan_ipv4_is_never_loopback_or_unspecified() {
        if let Some(ip) = primary_lan_ipv4() {
            assert!(!ip.is_loopback(), "returned loopback: {ip}");
            assert!(!ip.is_unspecified(), "returned unspecified: {ip}");
        }
    }
}
