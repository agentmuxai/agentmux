// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Is our mDNS service really announced on an IPv4 interface?
//!
//! `mdns-sd` builds one socket per interface when its daemon is created. For
//! IPv4 that is a bind to `0.0.0.0:5353`, a multicast-group join and a test
//! packet; if any step fails the crate logs at `debug!` and **skips that
//! interface** (mdns-sd's service daemon, "bind a socket to {}: {}. Skipped."). The
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

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

/// How long after start the daemon gets to announce before "nothing announced"
/// counts as a verdict. Probing plus the first announcement takes well under a
/// second on a healthy host.
pub const GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Consecutive undiscoverable verdicts, one per watchdog tick, required before
/// the daemon is rebuilt. One miss can be a slow start, not a fault.
pub const STRIKES_BEFORE_REBUILD: u32 = 2;

/// Ticks to wait after rebuild attempt k (1-based) before attempt k+1; the last
/// value repeats for ever. At the 15 s tick that is 30 s, 1 min, 2 min, 5 min, then
/// every 10 min. There is NO cap on the number of attempts.
///
/// A cap of three, 30 s apart, was the first version and it failed on Area54
/// (0.59.4, 2026-10-02): the holder of UDP 5353 was a long-lived program (Chrome had
/// started 14 s before srv), all three attempts landed inside the same 90 s of
/// contention, the watchdog gave up, and by 06:32 the port was free again with
/// nothing left to retry. A rebuild costs milliseconds, so retrying slowly for ever
/// is the right trade; each attempt names the cause (`lan_mdns_diag`).
pub const RETRY_AFTER_TICKS: &[u32] = &[2, 4, 8, 20, 40];

/// Ticks the watchdog will keep trying to bring the daemon back when LAN is
/// wanted but no daemon is running (a rebuild whose new daemon failed to start).
/// Without this a transient failure inside a rebuild would leave LAN dead until
/// the user toggled it (ReAgent P2 on #4148).
pub const MAX_DOWN_RETRIES: u32 = 6;

/// Whether this verdict should be sent to the windows on this tick.
///
/// The frontend learns the verdict only from `laninstances:health` events, and
/// a window opened (or a WebSocket reconnected) after the event has no way to
/// ask. So a standing problem is **re-sent every tick**; sending it only on
/// change left a late window showing `peers` for as long as the fault lasted
/// (ReAgent P1 on #4148). `Healthy` is sent only on change: no news is the
/// frontend's default for a window with nothing recorded. `Pending` is a gap
/// between verdicts, never news.
pub fn should_publish(health: &MdnsHealth, changed: bool) -> bool {
    match health {
        MdnsHealth::Pending => false,
        MdnsHealth::Healthy => changed,
        MdnsHealth::Degraded { .. } | MdnsHealth::Undiscoverable { .. } => true,
    }
}

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

/// The IPv4 addresses a LAN peer could reach us on, **grouped by interface**:
/// not loopback, unspecified, link-local (169.254/16), multicast or broadcast.
///
/// Grouped because `mdns-sd` announces on ONE address per interface and IP
/// version (`send_unsolicited_response`), so an interface with two IPv4
/// addresses never announces its second. Counting each address would leave such
/// a host `Degraded` for good. An interface is covered when any one of its
/// addresses was announced.
pub fn expected_by_interface(
    local: impl IntoIterator<Item = (String, IpAddr)>,
) -> Vec<BTreeSet<Ipv4Addr>> {
    let mut by_name: BTreeMap<String, BTreeSet<Ipv4Addr>> = BTreeMap::new();
    for (name, addr) in local {
        let IpAddr::V4(v4) = addr else { continue };
        if v4.is_loopback()
            || v4.is_unspecified()
            || v4.is_link_local()
            || v4.is_multicast()
            || v4.is_broadcast()
        {
            continue;
        }
        by_name.entry(name).or_default().insert(v4);
    }
    by_name.into_values().collect()
}

/// The verdict. Only addresses we expected count: an announcement on an address
/// we do not listen on proves nothing about the ones we do. `missing` names one
/// address (the lowest) per uncovered interface.
pub fn evaluate(expected: &[BTreeSet<Ipv4Addr>], announced: &BTreeSet<Ipv4Addr>) -> MdnsHealth {
    if expected.is_empty() {
        return MdnsHealth::Healthy;
    }
    let missing: Vec<Ipv4Addr> = expected
        .iter()
        .filter(|iface| iface.is_disjoint(announced))
        .filter_map(|iface| iface.iter().next().copied())
        .collect();
    if missing.is_empty() {
        MdnsHealth::Healthy
    } else if missing.len() == expected.len() {
        MdnsHealth::Undiscoverable { missing }
    } else {
        MdnsHealth::Degraded { missing }
    }
}

/// The verdict for a daemon of a given age.
///
/// `Pending` unless we are actually watching the daemon (`monitored`) and it has
/// had its start-up grace. Without a monitor nothing ever fills `announced`, so
/// evaluating would report a false `Undiscoverable` on any host with an IPv4
/// address, rebuild three times for nothing and flag the indicator (ReAgent P1
/// on #4148). "We cannot tell" is `Pending`, never a fault.
pub fn verdict(
    monitored: bool,
    since_start: std::time::Duration,
    expected: &[BTreeSet<Ipv4Addr>],
    announced: &BTreeSet<Ipv4Addr>,
) -> MdnsHealth {
    if !monitored || since_start < GRACE {
        return MdnsHealth::Pending;
    }
    evaluate(expected, announced)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Rebuild the daemon (turn LAN off and on).
    Rebuild,
}

/// Decides, tick by tick, when an undiscoverable verdict justifies a rebuild.
#[derive(Debug, Default)]
pub struct Watchdog {
    strikes: u32,
    rebuilds: u32,
    /// Ticks left before the next rebuild attempt is allowed.
    cooldown: u32,
    down_retries: u32,
}

impl Watchdog {
    pub fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    /// Ticks until the next attempt, right after a `Rebuild` (for the log line).
    pub fn retry_in_ticks(&self) -> u32 {
        self.cooldown
    }

    /// Forget everything, for when LAN is switched off.
    pub fn reset(&mut self) {
        *self = Watchdog::default();
    }

    /// LAN is wanted but no daemon is running. Returns whether to try starting
    /// one again this tick. Deliberately does not touch the rebuild budget: a
    /// failed rebuild must not wipe the count of rebuilds already spent.
    pub fn observe_down(&mut self) -> bool {
        if self.down_retries < MAX_DOWN_RETRIES {
            self.down_retries += 1;
            true
        } else {
            false
        }
    }

    pub fn observe(&mut self, health: &MdnsHealth) -> Action {
        // A daemon exists again, so the "down" retries start over.
        self.down_retries = 0;
        // Time passes on every tick, whatever the verdict.
        self.cooldown = self.cooldown.saturating_sub(1);
        match health {
            // No verdict yet: neither a strike nor a recovery.
            MdnsHealth::Pending => Action::Nothing,
            MdnsHealth::Healthy => {
                *self = Watchdog { down_retries: 0, ..Watchdog::default() };
                Action::Nothing
            }
            MdnsHealth::Degraded { .. } => {
                self.strikes = 0;
                Action::Nothing
            }
            MdnsHealth::Undiscoverable { .. } => {
                if self.rebuilds == 0 {
                    // One miss can be a slow start; the first rebuild needs two in a row.
                    self.strikes += 1;
                    if self.strikes < STRIKES_BEFORE_REBUILD {
                        return Action::Nothing;
                    }
                } else if self.cooldown > 0 {
                    // Backing off between attempts.
                    return Action::Nothing;
                }
                self.strikes = 0;
                self.rebuilds += 1;
                let step = (self.rebuilds as usize - 1).min(RETRY_AFTER_TICKS.len() - 1);
                self.cooldown = RETRY_AFTER_TICKS[step];
                Action::Rebuild
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
    /// One entry per interface, one address each.
    fn ifaces(v: &[&str]) -> Vec<BTreeSet<Ipv4Addr>> {
        v.iter().map(|s| set(&[s])).collect()
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
            ("lo", "127.0.0.1"),
            ("any", "0.0.0.0"),
            ("apipa", "169.254.21.81"),
            ("mc", "224.0.0.251"),
            ("bc", "255.255.255.255"),
            ("Ethernet", "192.168.1.26"),
            ("vEthernet", "172.17.16.1"),
            ("v6only", "fe80::1"),
        ]
        .map(|(n, a)| (n.to_string(), a.parse::<IpAddr>().unwrap()));
        assert_eq!(expected_by_interface(local), vec![set(&["192.168.1.26"]), set(&["172.17.16.1"])]);
    }

    #[test]
    fn two_addresses_on_one_interface_are_one_group() {
        let local = [("Ethernet", "192.168.1.26"), ("Ethernet", "192.168.1.27"), ("Wi-Fi", "10.0.0.5")]
            .map(|(n, a)| (n.to_string(), a.parse::<IpAddr>().unwrap()));
        assert_eq!(
            expected_by_interface(local),
            vec![set(&["192.168.1.26", "192.168.1.27"]), set(&["10.0.0.5"])]
        );
    }

    #[test]
    fn nothing_to_announce_on_is_not_a_fault() {
        assert_eq!(evaluate(&[], &set(&[])), MdnsHealth::Healthy);
    }

    #[test]
    fn announced_on_all_expected_is_healthy() {
        let e = ifaces(&["192.168.1.26", "172.17.16.1"]);
        assert_eq!(evaluate(&e, &set(&["192.168.1.26", "172.17.16.1"])), MdnsHealth::Healthy);
    }

    // ReAgent P2 on #4148: mdns-sd announces one address per interface, so the
    // second address of a NIC is never announced and must not count as missing.
    #[test]
    fn an_interface_is_covered_by_any_one_of_its_addresses() {
        let e = vec![set(&["192.168.1.26", "192.168.1.27"])];
        assert_eq!(evaluate(&e, &set(&["192.168.1.26"])), MdnsHealth::Healthy);
        assert_eq!(evaluate(&e, &set(&["192.168.1.27"])), MdnsHealth::Healthy);
    }

    #[test]
    fn announcements_outside_the_expected_set_prove_nothing() {
        let e = ifaces(&["192.168.1.26"]);
        assert!(evaluate(&e, &set(&["10.9.9.9"])).is_undiscoverable());
    }

    #[test]
    fn the_area54_case_is_undiscoverable() {
        // Both IPv4 interfaces expected; the daemon announced on neither.
        let e = ifaces(&["192.168.1.26", "172.17.16.1"]);
        let h = evaluate(&e, &set(&[]));
        assert_eq!(h.wire_state(), "undiscoverable");
        assert_eq!(h.missing(), &[ip("192.168.1.26"), ip("172.17.16.1")]);
    }

    #[test]
    fn a_missing_virtual_adapter_alone_is_only_degraded() {
        let e = ifaces(&["192.168.1.26", "172.17.16.1"]);
        let h = evaluate(&e, &set(&["192.168.1.26"]));
        assert_eq!(h, MdnsHealth::Degraded { missing: vec![ip("172.17.16.1")] });
        assert!(!h.is_undiscoverable());
    }

    #[test]
    fn a_standing_problem_is_resent_every_tick_so_a_late_window_learns_of_it() {
        // ReAgent P1 on #4148.
        let bad = undiscoverable();
        for _ in 0..3 {
            assert!(should_publish(&bad, false), "unchanged but still broken: resend");
        }
        let degraded = MdnsHealth::Degraded { missing: vec![ip("172.17.16.1")] };
        assert!(should_publish(&degraded, false));
    }

    #[test]
    fn healthy_is_only_sent_on_change_and_pending_never() {
        assert!(should_publish(&MdnsHealth::Healthy, true), "recovery must clear a warning");
        assert!(!should_publish(&MdnsHealth::Healthy, false));
        assert!(!should_publish(&MdnsHealth::Pending, true));
        assert!(!should_publish(&MdnsHealth::Pending, false));
    }

    #[test]
    fn a_down_daemon_is_retried_a_bounded_number_of_times() {
        let mut w = Watchdog::default();
        let tries = (0..20).filter(|_| w.observe_down()).count() as u32;
        assert_eq!(tries, MAX_DOWN_RETRIES);
    }

    #[test]
    fn a_failed_rebuild_does_not_wipe_the_rebuild_budget() {
        // ReAgent P2 on #4148: rebuild, the new daemon fails to start, the watchdog
        // sees "down" and retries. The spent rebuild must still be counted.
        let mut w = Watchdog::default();
        w.observe(&undiscoverable());
        assert_eq!(w.observe(&undiscoverable()), Action::Rebuild);
        assert_eq!(w.rebuilds(), 1);
        assert!(w.observe_down());
        assert!(w.observe_down());
        assert_eq!(w.rebuilds(), 1, "still counted while the daemon is down");
    }

    #[test]
    fn a_daemon_coming_back_restores_the_retries() {
        let mut w = Watchdog::default();
        while w.observe_down() {}
        assert!(!w.observe_down(), "exhausted");
        w.observe(&MdnsHealth::Pending); // a daemon exists again
        assert!(w.observe_down(), "retries start over");
    }

    #[test]
    fn no_monitor_means_no_verdict_however_long_it_has_been() {
        // ReAgent P1 on #4148: nothing would ever fill `announced`.
        let e = ifaces(&["192.168.1.26"]);
        let long_ago = GRACE * 100;
        assert_eq!(verdict(false, long_ago, &e, &set(&[])), MdnsHealth::Pending);
        assert_eq!(verdict(false, long_ago, &e, &set(&["192.168.1.26"])), MdnsHealth::Pending);
    }

    #[test]
    fn a_monitored_daemon_gets_its_start_up_grace_then_a_verdict() {
        let e = ifaces(&["192.168.1.26"]);
        assert_eq!(verdict(true, GRACE / 2, &e, &set(&[])), MdnsHealth::Pending);
        assert!(verdict(true, GRACE, &e, &set(&[])).is_undiscoverable());
        assert_eq!(verdict(true, GRACE, &e, &set(&["192.168.1.26"])), MdnsHealth::Healthy);
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

    /// Never gives up, and backs off: the gaps between attempts are 2, 4, 8, 20 then 40
    /// ticks, for ever. (The first version stopped after three, 30 s apart, and failed on
    /// Area54: the port was free again minutes later and nothing was left to retry.)
    #[test]
    fn rebuilds_back_off_and_never_stop() {
        let mut w = Watchdog::default();
        let mut at = Vec::new();
        for tick in 1..=400u32 {
            if w.observe(&undiscoverable()) == Action::Rebuild {
                at.push(tick);
            }
        }
        let gaps: Vec<u32> = at.windows(2).map(|p| p[1] - p[0]).collect();
        assert_eq!(at[0], 2, "the first rebuild needs two misses in a row");
        assert_eq!(&gaps[..5], &[2, 4, 8, 20, 40], "{at:?}");
        assert!(gaps[5..].iter().all(|g| *g == 40), "settles at one attempt per 40 ticks: {gaps:?}");
        assert!(400 - at.last().unwrap() < 40, "still retrying at the end: {at:?}");
        assert!(at.len() > 10, "far more than the old cap of 3: {}", at.len());
    }

    /// The Area54 timeline: a long-lived program holds the port for minutes, then lets
    /// go. Every attempt while it holds fails; the first one after it lets go succeeds.
    /// Under the old cap this never recovered.
    #[test]
    fn a_holder_that_lets_go_minutes_later_is_still_recovered() {
        const HOLDER_LEAVES_AT_TICK: u32 = 28; // 7 minutes at the 15 s tick
        let mut w = Watchdog::default();
        let mut recovered_ok = false;
        let mut recovered_at = None;
        for tick in 1..=200u32 {
            let health = if recovered_ok { MdnsHealth::Healthy } else { undiscoverable() };
            if w.observe(&health) == Action::Rebuild {
                // A rebuilt daemon announces only if the port is free by then.
                recovered_ok = tick >= HOLDER_LEAVES_AT_TICK;
            }
            if recovered_ok && recovered_at.is_none() {
                recovered_at = Some(tick);
            }
        }
        let at = recovered_at.expect("it must recover once the holder lets go");
        assert!((HOLDER_LEAVES_AT_TICK..HOLDER_LEAVES_AT_TICK + 40).contains(&at), "recovered at tick {at}");
        assert_eq!(w.rebuilds(), 0, "full health restores the budget");
    }

    #[test]
    fn the_next_attempt_time_is_exposed_for_the_log_line() {
        let mut w = Watchdog::default();
        w.observe(&undiscoverable());
        assert_eq!(w.observe(&undiscoverable()), Action::Rebuild);
        assert_eq!(w.retry_in_ticks(), RETRY_AFTER_TICKS[0]);
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
