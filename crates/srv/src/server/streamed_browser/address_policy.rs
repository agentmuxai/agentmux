// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which addresses a page in the server's Chromium may connect to. The browser opens pages from the web on a
//! machine that holds the user's workspace, so everything it connects to goes through `address_proxy`, which asks
//! this module about each address it is about to connect to, after it has resolved the name itself. Plain data and
//! functions, so every rule is tested without a network.
//!
//! Refused: this server's own listening ports on the loopback address, every private, link-local (the cloud
//! metadata addresses among them), shared, documentation, multicast and reserved range, IPv6 forms that embed an
//! IPv4 address (translated, 6to4, Teredo), and port 0. Allowed: the public internet, and other ports on the
//! loopback address (a development server the user runs in the same workspace).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;

/// The ports this server listens on, noted as each listener binds. A page may never connect to one of them,
/// whatever address it names for this machine.
static OWN_PORTS: Mutex<Vec<u16>> = Mutex::new(Vec::new());

/// Note a port this server listens on.
pub fn note_own_port(port: u16) {
    let mut ports = OWN_PORTS.lock().unwrap_or_else(|e| e.into_inner());
    if !ports.contains(&port) {
        ports.push(port);
    }
}

/// Other loopback ports the person who runs this server wants kept from pages (a gateway or another service in the
/// same network namespace), as a comma-separated list. Read by the server's own process: a page, or an agent, cannot
/// set it.
pub const DENY_PORTS_ENV: &str = "AGENTMUX_STREAMED_BROWSER_DENY_PORTS";

/// The ports in a comma-separated list; anything that is not a port number is skipped.
pub fn parse_ports(list: &str) -> Vec<u16> {
    list.split(',').filter_map(|p| p.trim().parse::<u16>().ok()).filter(|p| *p != 0).collect()
}

/// Note the ports `DENY_PORTS_ENV` names.
pub fn note_ports_from_env() {
    if let Ok(list) = std::env::var(DENY_PORTS_ENV) {
        parse_ports(&list).into_iter().for_each(note_own_port);
    }
}

/// The ports noted so far.
pub fn own_ports() -> Vec<u16> {
    OWN_PORTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn v4_refused(ip: Ipv4Addr) -> Option<&'static str> {
    let o = ip.octets();
    Some(match (o[0], o[1], o[2]) {
        (0, _, _) => "this-network address",
        (10, _, _) => "private address",
        (100, 64..=127, _) => "shared (carrier-grade NAT) address",
        (169, 254, _) => "link-local address (cloud metadata is here)",
        (172, 16..=31, _) => "private address",
        (192, 0, 0) => "reserved address",
        (192, 0, 2) => "documentation address",
        (192, 88, 99) => "reserved address",
        (192, 168, _) => "private address",
        (198, 18..=19, _) => "benchmarking address",
        (198, 51, 100) => "documentation address",
        (203, 0, 113) => "documentation address",
        (224..=255, _, _) => "multicast or reserved address",
        _ => return None,
    })
}

fn v6_refused(ip: Ipv6Addr) -> Option<&'static str> {
    let s = ip.segments();
    if ip.is_unspecified() {
        return Some("unspecified address");
    }
    // fc00::/7 unique local (the cloud's IPv6 metadata address is in it), fe80::/10 link-local, fec0::/10 site-local.
    if s[0] & 0xfe00 == 0xfc00 {
        return Some("private address");
    }
    if s[0] & 0xffc0 == 0xfe80 || s[0] & 0xffc0 == 0xfec0 {
        return Some("link-local address");
    }
    if s[0] & 0xff00 == 0xff00 {
        return Some("multicast address");
    }
    // Forms that carry an IPv4 address inside: a name server could hide a private one in them.
    if s[0] == 0x0064 && s[1] == 0xff9b {
        return Some("translated (NAT64) address");
    }
    if s[0] == 0x2002 {
        return Some("6to4 address");
    }
    if s[0] == 0x2001 && s[1] == 0 {
        return Some("Teredo address");
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return Some("documentation address");
    }
    None
}

/// The address with an IPv4 address written as IPv6 (`::ffff:a00:1`) read as the IPv4 one it is.
fn plain(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// Whether a page may connect to `ip` on `port`; the reason in words when it may not. `own_ports` are this
/// server's own listening ports.
pub fn check(ip: IpAddr, port: u16, own_ports: &[u16]) -> Result<(), &'static str> {
    if port == 0 {
        return Err("port 0");
    }
    let ip = plain(ip);
    if ip.is_loopback() {
        return if own_ports.contains(&port) { Err("this server's own port") } else { Ok(()) };
    }
    let refused = match ip {
        IpAddr::V4(v4) => v4_refused(v4),
        IpAddr::V6(v6) => v6_refused(v6),
    };
    refused.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_page_cannot_reach_the_cloud_metadata_address() {
        // The container task metadata and credentials endpoint, the instance metadata address, and the IPv6 one.
        for a in ["169.254.170.2", "169.254.169.254", "fd00:ec2::254"] {
            assert!(check(ip(a), 80, &[]).is_err(), "{a}");
        }
        // The whole link-local range, not just the two known addresses.
        assert!(check(ip("169.254.1.1"), 80, &[]).is_err());
        assert!(check(ip("fe80::1"), 80, &[]).is_err());
    }

    #[test]
    fn a_page_cannot_reach_a_private_range() {
        let private_v4: Vec<String> = [[10, 0, 0, 1], [10, 255, 255, 255], [172, 16, 0, 1], [172, 31, 255, 255], [192, 168, 1, 1]]
            .map(|o| std::net::Ipv4Addr::from(o).to_string())
            .into();
        for a in private_v4.iter().map(String::as_str).chain([
            "100.64.0.1", "100.127.255.255",
            "0.0.0.0", "192.0.0.1", "198.18.0.1", "224.0.0.1", "239.255.255.250", "240.0.0.1", "255.255.255.255",
            "fc00::1", "fdff::1", "fec0::1", "ff02::1", "::",
        ]) {
            assert!(check(ip(a), 443, &[]).is_err(), "{a}");
        }
    }

    #[test]
    fn the_edges_of_a_private_range_are_not_private() {
        for a in ["172.15.255.255", "172.32.0.1", "100.63.255.255", "100.128.0.1", "11.0.0.1", "169.253.0.1", "169.255.0.1"] {
            assert!(check(ip(a), 443, &[]).is_ok(), "{a}");
        }
    }

    #[test]
    fn an_ipv4_address_written_as_ipv6_is_judged_as_ipv4() {
        for a in ["::ffff:a00:1", "::ffff:169.254.170.2", "::ffff:c0a8:1"] {
            assert!(check(ip(a), 80, &[]).is_err(), "{a}");
        }
        assert!(check(ip("::ffff:8.8.8.8"), 443, &[]).is_ok());
        // Loopback written that way is loopback, with a loopback's rules.
        assert!(check(ip("::ffff:127.0.0.1"), 3000, &[]).is_ok());
        assert!(check(ip("::ffff:127.0.0.1"), 8190, &[8190]).is_err());
    }

    #[test]
    fn forms_that_hide_an_ipv4_address_are_refused() {
        for a in ["64:ff9b::a00:1", "2002:a00:1::1", "2001:0:4136:e378:8000:63bf:3fff:fdd2", "2001:db8::1"] {
            assert!(check(ip(a), 443, &[]).is_err(), "{a}");
        }
    }

    #[test]
    fn a_page_cannot_reach_this_servers_own_ports_on_any_loopback_form() {
        let own = [8190, 8191];
        for a in ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1"] {
            for p in own {
                assert!(check(ip(a), p, &own).is_err(), "{a}:{p}");
            }
            // The user's own development server on any other port is fine.
            assert!(check(ip(a), 3000, &own).is_ok(), "{a}:3000");
        }
    }

    #[test]
    fn the_public_internet_is_allowed_and_port_zero_is_not() {
        for a in ["8.8.8.8", "93.184.216.34", "1.1.1.1", "2606:4700:4700::1111", "2a00:1450:4001:81b::200e"] {
            assert!(check(ip(a), 443, &[8190]).is_ok(), "{a}");
        }
        assert!(check(ip("8.8.8.8"), 0, &[]).is_err());
    }

    #[test]
    fn the_list_of_extra_denied_ports_reads_numbers_and_skips_the_rest() {
        assert_eq!(parse_ports("8080, 9000,x,0,70000, 3001"), vec![8080, 9000, 3001]);
        assert!(parse_ports("").is_empty());
        // A port in the list is refused on the loopback address like srv's own.
        assert!(check(ip("127.0.0.1"), 8080, &parse_ports("8080")).is_err());
        assert!(check(ip("127.0.0.1"), 3000, &parse_ports("8080")).is_ok());
    }

    #[test]
    fn own_ports_are_kept_once() {
        note_own_port(40111);
        note_own_port(40111);
        assert_eq!(own_ports().iter().filter(|p| **p == 40111).count(), 1);
    }
}
