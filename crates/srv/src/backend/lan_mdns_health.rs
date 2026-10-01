// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Is our mDNS service really announced on an IPv4 interface?
//!
//! `mdns-sd` builds one socket per interface when its daemon is created. For
//! IPv4 that is a bind to `0.0.0.0:5353`, a multicast-group join and a test
//! packet; if any step fails the crate logs at `debug!` and **skips that
//! interface** (`service_daemon.rs`, "bind a socket to {}: {}. Skipped."). The
//! daemon still starts and `LAN discovery started (mDNS)` is still logged, so a
//! host can hear its peers (over IPv6) and never be heard. That is exactly what
//! happened on Area54 on 2026-10-01 (SPEC_LAN_FIREWALL_SETUP_2026_10_01.md 8.1).
//!
//! The daemon already says which interfaces it announced on: its monitor
//! channel emits `DaemonEvent::Announce(fullname, addresses)` only for
//! interfaces that have a working socket. This module turns that into a health
//! verdict. It opens **no socket of its own**: a second receiver on UDP 5353
//! would compete with the daemon it is checking (Codex P1 on #4133), and
//! `mdns-sd` always answers by multicast, so a legacy-unicast probe from an
//! ephemeral port would never get an answer.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

/// How long after start the daemon gets to announce before "nothing announced"
/// counts as a verdict. Probing plus the first announcement takes well under a
/// second on a healthy host.
pub const GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Consecutive undiscoverable verdicts, one per watchdog tick, required before
/// the daemon is rebuilt. One miss can be a slow start, not a fault.
pub const STRIKES_BEFORE_REBUILD: u32 = 2;

/// Rebuilds attempted before the watchdog stops and leaves the indicator
/// flagged. Each rebuild is the same path as switching LAN off and on, which
/// cleared the fault on Area54.
pub const MAX_REBUILDS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MdnsHealth {
    /// Too soon after start to say.
    #[default]
    Pending,
    /// Announced on every IPv4 address we expect to be reachable on, or there
    /// is no usable IPv4 address to announce on.
    Healthy,
    /// Announced on some expected IPv4 addresses but not all (usually a virtual
    /// adapter). Worth a warning, not a flagged indicator.
    Degraded { missing: Vec<Ipv4Addr> },
    /// Announced on none of them: other machines cannot find this one.
    Undiscoverable { missing: Vec<Ipv4Addr> },
}

impl MdnsHealth {
    pub fn is_undiscoverable(&self) -> bool {
        matches!(self, MdnsHealth::Undiscoverable { .. })
    }

    /// The value sent to the frontend as `state`.
    pub fn wire_state(&self) -> &'static str {
        match self {
            MdnsHealth::Pending => "pending",
            MdnsHealth::Healthy => "healthy",
            MdnsHealth::Degraded { .. } => "degraded",
            MdnsHealth::Undiscoverable { .. } => "undiscoverable",
        }
    }

    pub fn missing(&self) -> &[Ipv4Addr] {
        match self {
            MdnsHealth::Degraded { missing } | MdnsHealth::Undiscoverable { missing } => missing,
            _ => &[],
        }
    }
}

/// Every IPv4 address written in `payload`. `mdns-sd` 0.12 formats the address
/// part of `DaemonEvent::Announce` two ways (a debug-formatted list at
/// registration, `host:addr` on later announcements), so this scans for
/// dotted quads and does not depend on either shape. IPv6 is ignored: only the
/// IPv4 socket was ever missing.
pub fn ipv4_in(payload: &str) -> Vec<Ipv4Addr> {
    payload
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|t| !t.is_empty())
        .filter_map(|t| Ipv4Addr::from_str(t).ok())
        .collect()
}

/// The IPv4 addresses a LAN peer could reach us on: not loopback, not
/// unspecified, not link-local (169.254/16), not multicast or broadcast.
pub fn expected_ipv4(local: impl IntoIterator<Item = IpAddr>) -> BTreeSet<Ipv4Addr> {
    local
        .into_iter()
        .filter_map(|a| match a {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .filter(|a| {
            !(a.is_loopback()
                || a.is_unspecified()
                || a.is_link_local()
                || a.is_multicast()
                || a.is_broadcast())
        })
        .collect()
}

/// The verdict. Only addresses we expected count: an announcement on an address
/// we do not listen on proves nothing about the ones we do.
pub fn evaluate(expected: &BTreeSet<Ipv4Addr>, announced: &BTreeSet<Ipv4Addr>) -> MdnsHealth {
    if expected.is_empty() {
        return MdnsHealth::Healthy;
    }
    let missing: Vec<Ipv4Addr> = expected.difference(announced).copied().collect();
    if missing.is_empty() {
        MdnsHealth::Healthy
    } else if missing.len() == expected.len() {
        MdnsHealth::Undiscoverable { missing }
    } else {
        MdnsHealth::Degraded { missing }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Rebuild the daemon (turn LAN off and on).
    Rebuild,
    /// Rebuilds are used up; stay flagged and say so once.
    GiveUp,
}

/// Decides, tick by tick, when an undiscoverable verdict justifies a rebuild.
#[derive(Debug, Default)]
pub struct Watchdog {
    strikes: u32,
    rebuilds: u32,
    gave_up: bool,
}

impl Watchdog {
    pub fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    /// Forget everything, for when LAN is switched off.
    pub fn reset(&mut self) {
        *self = Watchdog::default();
    }

    pub fn observe(&mut self, health: &MdnsHealth) -> Action {
        match health {
            // No verdict yet: neither a strike nor a recovery.
            MdnsHealth::Pending => Action::Nothing,
            MdnsHealth::Healthy | MdnsHealth::Degraded { .. } => {
                self.strikes = 0;
                if matches!(health, MdnsHealth::Healthy) {
                    self.rebuilds = 0;
                    self.gave_up = false;
                }
                Action::Nothing
            }
            MdnsHealth::Undiscoverable { .. } => {
                self.strikes += 1;
                if self.strikes < STRIKES_BEFORE_REBUILD {
                    return Action::Nothing;
                }
                if self.rebuilds < MAX_REBUILDS {
                    self.strikes = 0;
                    self.rebuilds += 1;
                    Action::Rebuild
                } else if !self.gave_up {
                    self.gave_up = true;
                    Action::GiveUp
                } else {
                    Action::Nothing
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }
    fn set(v: &[&str]) -> BTreeSet<Ipv4Addr> {
        v.iter().map(|s| ip(s)).collect()
    }
    fn undiscoverable() -> MdnsHealth {
        MdnsHealth::Undiscoverable { missing: vec![ip("192.168.1.26")] }
    }

    #[test]
    fn parses_the_registration_payload_shape() {
        // `format!("{:?}", Vec<IpAddr>)`
        let got = ipv4_in("[192.168.1.26, 172.17.16.1, fe80::1]");
        assert_eq!(got, vec![ip("192.168.1.26"), ip("172.17.16.1")]);
    }

    #[test]
    fn parses_the_host_colon_addr_payload_shape() {
        assert_eq!(ipv4_in("Area54.local.:192.168.1.26"), vec![ip("192.168.1.26")]);
        // A host label with digits must not be mistaken for an address.
        assert!(ipv4_in("Area54.local.").is_empty());
        assert!(ipv4_in("").is_empty());
    }

    #[test]
    fn expected_drops_everything_a_peer_could_not_use() {
        let local = [
            "127.0.0.1",
            "0.0.0.0",
            "169.254.21.81",
            "224.0.0.251",
            "255.255.255.255",
            "192.168.1.26",
            "172.17.16.1",
            "fe80::1",
        ]
        .map(|s| s.parse::<IpAddr>().unwrap());
        assert_eq!(expected_ipv4(local), set(&["192.168.1.26", "172.17.16.1"]));
    }

    #[test]
    fn nothing_to_announce_on_is_not_a_fault() {
        assert_eq!(evaluate(&set(&[]), &set(&[])), MdnsHealth::Healthy);
    }

    #[test]
    fn announced_on_all_expected_is_healthy() {
        let e = set(&["192.168.1.26", "172.17.16.1"]);
        assert_eq!(evaluate(&e, &e), MdnsHealth::Healthy);
    }

    #[test]
    fn announcements_outside_the_expected_set_prove_nothing() {
        let e = set(&["192.168.1.26"]);
        let a = set(&["10.9.9.9"]);
        assert!(evaluate(&e, &a).is_undiscoverable());
    }

    #[test]
    fn the_area54_case_is_undiscoverable() {
        // Both IPv4 interfaces expected; the daemon announced on neither.
        let e = set(&["192.168.1.26", "172.17.16.1"]);
        let h = evaluate(&e, &set(&[]));
        assert_eq!(h.wire_state(), "undiscoverable");
        assert_eq!(h.missing(), &[ip("172.17.16.1"), ip("192.168.1.26")]);
    }

    #[test]
    fn a_missing_virtual_adapter_alone_is_only_degraded() {
        let e = set(&["192.168.1.26", "172.17.16.1"]);
        let h = evaluate(&e, &set(&["192.168.1.26"]));
        assert_eq!(h, MdnsHealth::Degraded { missing: vec![ip("172.17.16.1")] });
        assert!(!h.is_undiscoverable());
    }

    #[test]
    fn one_miss_is_not_enough_to_rebuild() {
        let mut w = Watchdog::default();
        assert_eq!(w.observe(&undiscoverable()), Action::Nothing);
        assert_eq!(w.rebuilds(), 0);
    }

    #[test]
    fn two_consecutive_misses_rebuild_and_a_pending_verdict_does_not_count() {
        let mut w = Watchdog::default();
        assert_eq!(w.observe(&undiscoverable()), Action::Nothing);
        // The rebuilt daemon reports Pending: not a strike, not a recovery.
        assert_eq!(w.observe(&MdnsHealth::Pending), Action::Nothing);
        assert_eq!(w.observe(&undiscoverable()), Action::Rebuild);
        assert_eq!(w.rebuilds(), 1);
    }

    #[test]
    fn a_healthy_tick_resets_the_strikes() {
        let mut w = Watchdog::default();
        w.observe(&undiscoverable());
        w.observe(&MdnsHealth::Healthy);
        assert_eq!(w.observe(&undiscoverable()), Action::Nothing, "strikes restarted");
    }

    #[test]
    fn rebuilds_are_capped_then_it_gives_up_once() {
        let mut w = Watchdog::default();
        let mut rebuilds = 0;
        let mut gave_up = 0;
        for _ in 0..40 {
            match w.observe(&undiscoverable()) {
                Action::Rebuild => rebuilds += 1,
                Action::GiveUp => gave_up += 1,
                Action::Nothing => {}
            }
        }
        assert_eq!(rebuilds, MAX_REBUILDS);
        assert_eq!(gave_up, 1, "says so once, then stays quiet");
    }

    #[test]
    fn recovery_restores_the_rebuild_budget() {
        let mut w = Watchdog::default();
        for _ in 0..40 {
            w.observe(&undiscoverable());
        }
        w.observe(&MdnsHealth::Healthy);
        assert_eq!(w.rebuilds(), 0);
        w.observe(&undiscoverable());
        assert_eq!(w.observe(&undiscoverable()), Action::Rebuild);
    }

    #[test]
    fn degraded_does_not_clear_the_rebuild_count_but_does_clear_strikes() {
        let mut w = Watchdog::default();
        w.observe(&undiscoverable());
        w.observe(&undiscoverable()); // rebuild #1
        assert_eq!(w.rebuilds(), 1);
        w.observe(&MdnsHealth::Degraded { missing: vec![ip("172.17.16.1")] });
        assert_eq!(w.rebuilds(), 1, "only full health restores the budget");
    }
}
