// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which machine a remote pane is connected to, by name and address, for its
//! stats badge tooltip (`blockstats.detail`).
//!
//! - SSH: `ssh -G` says which host a destination really names (the user's
//!   ssh config applies: an alias's `HostName`, `Include`, `Match`), and a
//!   DNS lookup gives its address. Through a `ProxyJump` the address is
//!   what this computer resolves the name to, which may be none.
//! - WSL: the distribution's address on the interface its default route
//!   uses, from `/proc/net/route` and `/proc/net/fib_trie`, which every
//!   distribution has (BusyBox's `hostname` has no `-I`).
//!
//! Answers are kept for [`TTL`], failures too, so a tooltip that asks every
//! few seconds runs `ssh` or `wsl.exe` at most once in that time.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::ConnTarget;

/// How long an answer is kept.
const TTL: Duration = Duration::from_secs(300);

/// How long `ssh -G`, `wsl.exe` or a DNS lookup may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(3);

/// A remote pane's machine. Either part is `None` when it couldn't be found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteAddress {
    /// SSH: the host name `ssh` connects to. WSL: the distribution.
    pub host: Option<String>,
    /// Its IP address.
    pub address: Option<String>,
}

/// [`RemoteAddress`] for a remote connection, from the cache when it is
/// fresh; `None` for a local one.
pub async fn of(name: &str) -> Option<RemoteAddress> {
    let target = ConnTarget::parse(name).ok()?;
    if target == ConnTarget::Local {
        return None;
    }
    // One slot per connection, its lock held across the lookup: a second
    // tooltip asking for the same connection meanwhile waits for this answer
    // instead of starting another, and one slow host holds up no other.
    static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    let slot = SLOTS.get_or_init(Default::default).lock().unwrap().entry(name.to_string()).or_default().clone();
    let mut cached = slot.lock().await;
    if let Some((at, found)) = cached.as_ref() {
        if at.elapsed() < TTL {
            return Some(found.clone());
        }
    }
    let found = match &target {
        ConnTarget::Ssh(dest) => ssh(dest).await,
        ConnTarget::Wsl(distro) => RemoteAddress {
            host: Some(distro.clone()),
            address: wsl_address(distro).await,
        },
        ConnTarget::Local => unreachable!(),
    };
    *cached = Some((Instant::now(), found.clone()));
    Some(found)
}

type Slot = Arc<tokio::sync::Mutex<Option<(Instant, RemoteAddress)>>>;

async fn ssh(dest: &super::conn::SshDest) -> RemoteAddress {
    let Some(binary) = super::ssh::binary_for_user() else {
        return RemoteAddress::default();
    };
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("-G");
    if let Some(port) = dest.port {
        cmd.args(["-p", &port.to_string()]);
    }
    // `--` so a destination can never be read as an option.
    cmd.args(["--", &dest.destination]).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        cmd.no_window();
    }
    let (host, port) = match tokio::time::timeout(STEP_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => parse_ssh_g(&String::from_utf8_lossy(&out.stdout)),
        _ => (None, None),
    };
    let Some(host) = host else {
        return RemoteAddress::default();
    };
    let address = resolve(&host, port.or(dest.port).unwrap_or(22)).await;
    RemoteAddress { host: Some(host), address }
}

/// `hostname` and `port` from `ssh -G`'s output.
fn parse_ssh_g(out: &str) -> (Option<String>, Option<u16>) {
    let mut host = None;
    let mut port = None;
    for line in out.lines() {
        match line.split_once(' ') {
            Some(("hostname", v)) if !v.trim().is_empty() => host = Some(v.trim().to_string()),
            Some(("port", v)) => port = v.trim().parse().ok(),
            _ => {}
        }
    }
    (host, port)
}

/// `host`'s address: itself when it is one, else the first a lookup gives,
/// IPv4 first.
async fn resolve(host: &str, port: u16) -> Option<String> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    let addrs: Vec<IpAddr> = tokio::time::timeout(STEP_TIMEOUT, tokio::net::lookup_host((host, port)))
        .await
        .ok()?
        .ok()?
        .map(|a| a.ip())
        .collect();
    addrs.iter().find(|ip| ip.is_ipv4()).or(addrs.first()).map(IpAddr::to_string)
}

async fn wsl_address(distro: &str) -> Option<String> {
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        let mut cmd = tokio::process::Command::new("wsl.exe");
        cmd.args(["-d", distro, "--exec", "cat", "/proc/net/route", "/proc/net/fib_trie"])
            .stdin(std::process::Stdio::null())
            .no_window();
        match tokio::time::timeout(STEP_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => linux_address(&String::from_utf8_lossy(&out.stdout)),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = distro;
        None
    }
}

/// A Linux machine's own IPv4 address, from `/proc/net/route` followed by
/// `/proc/net/fib_trie`: of its local addresses, the one in a network on the
/// interface the default route uses, else the first that isn't loopback.
/// WSL also puts its DNS proxy's address on `lo`, which this skips.
fn linux_address(out: &str) -> Option<String> {
    use std::net::Ipv4Addr;
    // /proc/net/route: tab-separated, addresses in hex, in memory order.
    let hex = |s: &str| u32::from_str_radix(s, 16).ok().map(|v| Ipv4Addr::from(v.to_le_bytes()));
    let routes: Vec<(&str, Ipv4Addr, Ipv4Addr)> = out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            (f.len() >= 8 && f[0] != "Iface").then(|| Some((f[0], hex(f[1])?, hex(f[7])?))).flatten()
        })
        .collect();
    let default_iface = routes.iter().find(|r| r.1.is_unspecified()).map(|r| r.0);
    let nets: Vec<(u32, u32)> = routes
        .iter()
        .filter(|r| Some(r.0) == default_iface && !r.1.is_unspecified())
        .map(|r| (u32::from(r.1), u32::from(r.2)))
        .collect();
    // /proc/net/fib_trie: "|-- <address>" then "/32 host LOCAL" for each of
    // the machine's own addresses.
    let lines: Vec<&str> = out.lines().collect();
    let mut local: Vec<Ipv4Addr> = Vec::new();
    for w in lines.windows(2) {
        if let (Some(addr), true) = (w[0].trim().strip_prefix("|-- "), w[1].contains("/32 host LOCAL")) {
            if let Ok(ip) = addr.trim().parse::<Ipv4Addr>() {
                if !ip.is_loopback() && !local.contains(&ip) {
                    local.push(ip);
                }
            }
        }
    }
    local
        .iter()
        .find(|ip| nets.iter().any(|&(net, mask)| u32::from(**ip) & mask == net))
        .or(local.first())
        .map(Ipv4Addr::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_g_gives_the_real_host_and_port() {
        let out = "user me\nhostname build.example.net\nport 2222\nhostkeyalias none\n";
        assert_eq!(parse_ssh_g(out), (Some("build.example.net".to_string()), Some(2222)));
        assert_eq!(parse_ssh_g("user me\n"), (None, None));
        // Windows' ssh ends lines with CRLF.
        assert_eq!(parse_ssh_g("hostname box\r\nport 22\r\n"), (Some("box".to_string()), Some(22)));
    }

    /// What a WSL distribution's `cat /proc/net/route /proc/net/fib_trie`
    /// prints (docker-desktop's, abridged, with documentation addresses).
    const WSL_NET: &str = concat!(
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n",
        "eth0\t00000000\t017100CB\t0003\t0\t0\t0\t00000000\t0\t0\t0\n",
        "eth0\t007100CB\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n",
        "Main:\n",
        "  +-- 0.0.0.0/0 3 0 5\n",
        "     |-- 0.0.0.0\n",
        "        /0 universe UNICAST\n",
        "     +-- 198.51.100.254/32 host LOCAL\n",
        "     |-- 198.51.100.254\n",
        "        /32 host LOCAL\n",
        "     +-- 127.0.0.0/8 2 0 2\n",
        "        |-- 127.0.0.1\n",
        "           /32 host LOCAL\n",
        "     +-- 203.0.113.0/24 2 0 2\n",
        "        |-- 203.0.113.121\n",
        "           /32 host LOCAL\n",
    );

    #[test]
    fn a_wsl_distro_is_its_address_on_the_default_route() {
        assert_eq!(linux_address(WSL_NET), Some("203.0.113.121".to_string()));
    }

    #[test]
    fn without_routes_the_first_non_loopback_address() {
        assert_eq!(linux_address("Main:\n  |-- 127.0.0.1\n     /32 host LOCAL\n  |-- 192.0.2.20\n     /32 host LOCAL\n"), Some("192.0.2.20".to_string()));
        assert_eq!(linux_address("Main:\n  |-- 127.0.0.1\n     /32 host LOCAL\n"), None);
        assert_eq!(linux_address(""), None);
    }

    #[tokio::test]
    async fn an_address_resolves_to_itself_without_a_lookup() {
        assert_eq!(resolve("192.0.2.7", 22).await, Some("192.0.2.7".to_string()));
        assert_eq!(resolve("[2001:db8::1]", 22).await, Some("2001:db8::1".to_string()));
    }

    #[tokio::test]
    async fn localhost_resolves() {
        let ip = resolve("localhost", 22).await.expect("localhost resolves");
        assert!(ip == "127.0.0.1" || ip == "::1", "{ip}");
    }

    #[tokio::test]
    async fn a_local_pane_has_no_remote_address() {
        assert_eq!(of("").await, None);
        assert_eq!(of("local").await, None);
    }
}
