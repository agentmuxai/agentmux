// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The fixed port range srv's listeners live in, so a firewall rule can name
//! them (`docs/specs/SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` §4.1).
//!
//! srv used to let the OS choose its web and ws ports on every start. A
//! firewall rule that must survive updates, new portable builds and reinstalls
//! cannot name a port that changes, and a program rule cannot either: the
//! sidecar's filename embeds its version, so a per-exe rule stops matching on
//! every update and re-registering it needs another elevation. A port range
//! does neither.
//!
//! The loopback and LAN listeners share a port (see `lan_listeners.rs`), so
//! choosing the *startup* ports from the range is enough: every LAN listener
//! added later lands in the range too, and mDNS advertises the real port.
//!
//! Several instances on one host (channels, portable builds, `task dev`) each
//! take the next free ports. If fewer than two are free srv does not fail: it
//! falls back to OS-chosen ports exactly as before, and
//! [`StartupListeners::in_range`] is `false` so the LAN indicator can say the
//! rule does not cover this instance.

use std::net::Ipv4Addr;
use std::ops::RangeInclusive;

use tokio::net::TcpListener;

/// TCP ports srv's web and ws listeners are taken from. Sits just above the
/// UDP broadcast fallback (`lan_discovery::UDP_DISCOVERY_PORT`, 47891), inside
/// the IANA registered range and below the Windows/Linux dynamic ranges
/// (49152+, 32768+ on older Linux), so the OS does not hand these out for
/// ephemeral connections. One hundred ports hold about fifty instances.
pub const LAN_PORT_RANGE: RangeInclusive<u16> = 47892..=47991;

/// The two startup listeners, plus where they came from.
pub struct StartupListeners {
    pub web: TcpListener,
    pub ws: TcpListener,
    pub in_range: bool,
}

/// Bind the web and ws listeners on loopback, from [`LAN_PORT_RANGE`] when two
/// ports are free there, else OS-chosen. Panics only if even an OS-chosen
/// loopback bind fails, as the code this replaces did.
pub async fn bind_startup_listeners() -> StartupListeners {
    bind_startup_listeners_from(LAN_PORT_RANGE).await
}

/// [`bind_startup_listeners`] over an explicit candidate list. Separate so the
/// tests can drive it with ports they control instead of racing real
/// instances for the production range.
pub(crate) async fn bind_startup_listeners_from(
    candidates: impl IntoIterator<Item = u16>,
) -> StartupListeners {
    if let Some((web, ws)) = first_two_free(candidates).await {
        return StartupListeners {
            web,
            ws,
            in_range: true,
        };
    }
    let any = (Ipv4Addr::LOCALHOST, 0);
    let web = TcpListener::bind(any)
        .await
        .unwrap_or_else(|e| panic!("failed to bind web listener on 127.0.0.1:0: {e}"));
    let ws = TcpListener::bind(any)
        .await
        .unwrap_or_else(|e| panic!("failed to bind ws listener on 127.0.0.1:0: {e}"));
    StartupListeners {
        web,
        ws,
        in_range: false,
    }
}

/// The first two loopback ports among `candidates` that bind, in order. Not
/// necessarily adjacent: an instance that started earlier may hold one in the
/// middle. `None` when fewer than two are free; nothing is left bound then.
async fn first_two_free(
    candidates: impl IntoIterator<Item = u16>,
) -> Option<(TcpListener, TcpListener)> {
    let mut bound = Vec::with_capacity(2);
    for port in candidates {
        // A port another instance (or anything else) holds simply fails: skip it.
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
            bound.push(listener);
            if bound.len() == 2 {
                let ws = bound.pop()?;
                let web = bound.pop()?;
                return Some((web, ws));
            }
        }
    }
    // Fewer than two: `bound` drops here, so a lone free port is not left held.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A port that is free right now (bound, noted, released).
    fn free_port() -> u16 {
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Two distinct free ports.
    fn two_free_ports() -> (u16, u16) {
        let a = free_port();
        loop {
            let b = free_port();
            if b != a {
                return (a, b);
            }
        }
    }

    #[tokio::test]
    async fn takes_the_first_two_free_ports_in_order() {
        let (a, b) = two_free_ports();
        let got = bind_startup_listeners_from([a, b]).await;
        assert!(got.in_range);
        assert_eq!(got.web.local_addr().unwrap().port(), a);
        assert_eq!(got.ws.local_addr().unwrap().port(), b);
    }

    #[tokio::test]
    async fn skips_a_port_another_instance_holds() {
        // `taken` stands for an instance that started earlier and is still running.
        let taken = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let (b, c) = two_free_ports();
        let got = bind_startup_listeners_from([taken_port, b, c]).await;
        assert!(
            got.in_range,
            "two free ports remain, so it is still in range"
        );
        assert_eq!(got.web.local_addr().unwrap().port(), b);
        assert_eq!(got.ws.local_addr().unwrap().port(), c);
        drop(taken);
    }

    #[tokio::test]
    async fn a_taken_port_between_two_free_ones_is_skipped_not_paired() {
        let (a, c) = two_free_ports();
        let taken = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let got = bind_startup_listeners_from([a, taken_port, c]).await;
        assert!(got.in_range);
        assert_eq!(got.web.local_addr().unwrap().port(), a);
        assert_eq!(
            got.ws.local_addr().unwrap().port(),
            c,
            "not adjacent, and that is fine"
        );
        drop(taken);
    }

    #[tokio::test]
    async fn fewer_than_two_free_falls_back_to_os_chosen_ports() {
        let taken = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let only_one_free = free_port();
        let got = bind_startup_listeners_from([taken_port, only_one_free]).await;
        assert!(!got.in_range, "one free port is not enough for the pair");
        let (web, ws) = (got.web.local_addr().unwrap(), got.ws.local_addr().unwrap());
        assert!(web.ip().is_loopback() && ws.ip().is_loopback());
        assert_ne!(web.port(), ws.port());
        assert_ne!(web.port(), taken_port);
        // The lone free port was released again, not left half-bound.
        assert!(std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, only_one_free)).is_ok());
        drop(taken);
    }

    #[tokio::test]
    async fn an_empty_candidate_list_falls_back() {
        let got = bind_startup_listeners_from(std::iter::empty()).await;
        assert!(!got.in_range);
        assert!(got.web.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn the_range_is_one_a_rule_can_rely_on() {
        let (start, end) = (*LAN_PORT_RANGE.start(), *LAN_PORT_RANGE.end());
        assert!(
            start > crate::backend::lan_discovery::UDP_DISCOVERY_PORT,
            "must not overlap the UDP broadcast fallback port"
        );
        assert!(
            end < 49152,
            "must stay below the dynamic port range, or the OS hands these out"
        );
        assert!(
            usize::from(end - start) + 1 >= 100,
            "room for about fifty instances (two ports each)"
        );
    }
}
