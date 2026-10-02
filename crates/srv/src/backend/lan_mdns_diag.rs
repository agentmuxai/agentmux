// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Why can't this host announce itself on mDNS? One line for the log.
//!
//! `mdns-sd` skips an interface whose socket cannot be set up and says so only at
//! `debug!`, so when `lan_mdns_health` finds the host undiscoverable the log has no
//! reason. On Area54 (2026-10-02, 0.59.4) the watchdog rebuilt the daemon three
//! times and gave up while the cause, Chrome and then the Windows DNS client taking
//! turns holding UDP 5353, had to be found by hand over several cloud round trips.
//! This replays what `mdns-sd` does on one interface, and names who holds the port,
//! so that the next one is a single `WARN` line.
//!
//! It runs only AFTER the daemon has already failed to announce, once per recovery
//! attempt, and each probe socket lives for a few milliseconds. It is a diagnosis,
//! not a health check: a health check on UDP 5353 would compete with the daemon it
//! is checking (Codex P1 on #4133), and this never decides anything.
//! See docs/specs/SPEC_LAN_FIREWALL_SETUP_2026_10_01.md sections 4.7 and 8.1.

use std::net::{Ipv4Addr, SocketAddrV4};

use socket2::{Domain, Protocol, Socket, Type};

const MDNS_PORT: u16 = 5353;
const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// The step of `mdns-sd`'s per-interface set-up that failed, and the OS's error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindFailure {
    pub step: &'static str,
    /// `raw_os_error`, for example 10048 (`WSAEADDRINUSE`) or 10013 (`WSAEACCES`).
    pub os_error: Option<i32>,
    pub message: String,
}

fn fail(step: &'static str, e: std::io::Error) -> BindFailure {
    BindFailure { step, os_error: e.raw_os_error(), message: e.to_string() }
}

/// Replay `mdns-sd`'s `new_socket_bind` for the IPv4 interface `ip`: a UDP socket
/// with address reuse, bound to `0.0.0.0:5353`, joined to the mDNS group on `ip`,
/// with `ip` as the multicast interface, then one empty packet to the group.
/// `Ok` means every step worked, so a failure to announce was not a bind failure.
pub fn probe_bind(ip: Ipv4Addr) -> Result<(), BindFailure> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| fail("create", e))?;
    sock.set_reuse_address(true).map_err(|e| fail("reuse-address", e))?;
    #[cfg(unix)]
    sock.set_reuse_port(true).map_err(|e| fail("reuse-port", e))?;
    let any = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT);
    sock.bind(&any.into()).map_err(|e| fail("bind", e))?;
    sock.join_multicast_v4(&MDNS_GROUP, &ip).map_err(|e| fail("join-group", e))?;
    sock.set_multicast_if_v4(&ip).map_err(|e| fail("set-multicast-if", e))?;
    let group = SocketAddrV4::new(MDNS_GROUP, MDNS_PORT);
    sock.send_to(&[0u8; 12], &group.into()).map_err(|e| fail("send", e))?;
    Ok(())
}

/// A process holding a socket on UDP 5353.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortHolder {
    pub pid: u32,
    pub process: String,
    /// The local address it is bound to: `0.0.0.0`, `::`, or a specific one.
    pub address: String,
}

/// Who holds UDP 5353 right now. Windows only (`GetExtendedUdpTable`); elsewhere it
/// says so rather than guessing.
#[cfg(windows)]
pub fn udp_5353_holders() -> Result<Vec<PortHolder>, String> {
    windows_impl::holders()
}

#[cfg(not(windows))]
pub fn udp_5353_holders() -> Result<Vec<PortHolder>, String> {
    Err("holder lookup is only implemented on Windows".to_string())
}

#[cfg(windows)]
mod windows_impl {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedUdpTable, MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID, UDP_TABLE_OWNER_PID,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    use super::{PortHolder, MDNS_PORT};

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    /// The owner-PID UDP table for one address family, as raw bytes.
    fn table(family: u32) -> Result<Vec<u8>, String> {
        let mut buf: Vec<u8> = Vec::new();
        let mut size: u32 = 0;
        // The first call reports the size it needs; the table can grow between calls,
        // so a few tries.
        for _ in 0..5 {
            // SAFETY: `buf` is at least `size` bytes whenever it is passed (empty with
            // size 0 on the first call), and both outlive the call.
            let rc = unsafe {
                GetExtendedUdpTable(buf.as_mut_ptr().cast(), &mut size, 0, family, UDP_TABLE_OWNER_PID, 0)
            };
            match rc {
                0 => {
                    buf.truncate(size as usize);
                    return Ok(buf);
                }
                ERROR_INSUFFICIENT_BUFFER => buf.resize(size as usize, 0),
                other => return Err(format!("GetExtendedUdpTable failed with {other}")),
            }
        }
        Err("the UDP table kept growing".to_string())
    }

    /// `(local address, local port, pid)` rows of the IPv4 table.
    fn rows_v4(buf: &[u8]) -> Vec<(String, u16, u32)> {
        let count = buf.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0) as usize;
        let row = std::mem::size_of::<MIB_UDPROW_OWNER_PID>();
        (0..count)
            .filter_map(|i| {
                let at = 4 + i * row;
                if at + row > buf.len() {
                    return None;
                }
                // SAFETY: bounds checked above; read_unaligned because the buffer is bytes.
                let r = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(at).cast::<MIB_UDPROW_OWNER_PID>()) };
                let addr = Ipv4Addr::from(r.dwLocalAddr.to_ne_bytes()).to_string();
                // The port is in network byte order in the low 16 bits.
                Some((addr, u16::from_be((r.dwLocalPort & 0xFFFF) as u16), r.dwOwningPid))
            })
            .collect()
    }

    fn rows_v6(buf: &[u8]) -> Vec<(String, u16, u32)> {
        let count = buf.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0) as usize;
        let row = std::mem::size_of::<MIB_UDP6ROW_OWNER_PID>();
        (0..count)
            .filter_map(|i| {
                let at = 4 + i * row;
                if at + row > buf.len() {
                    return None;
                }
                // SAFETY: bounds checked above; read_unaligned because the buffer is bytes.
                let r = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(at).cast::<MIB_UDP6ROW_OWNER_PID>()) };
                let addr = Ipv6Addr::from(r.ucLocalAddr).to_string();
                Some((addr, u16::from_be((r.dwLocalPort & 0xFFFF) as u16), r.dwOwningPid))
            })
            .collect()
    }

    pub fn holders() -> Result<Vec<PortHolder>, String> {
        let mut found: Vec<(String, u32)> = Vec::new();
        for (family, v6) in [(AF_INET as u32, false), (AF_INET6 as u32, true)] {
            let buf = table(family)?;
            let rows = if v6 { rows_v6(&buf) } else { rows_v4(&buf) };
            found.extend(rows.into_iter().filter(|(_, port, _)| *port == MDNS_PORT).map(|(a, _, pid)| (a, pid)));
        }
        let pids: Vec<sysinfo::Pid> = found.iter().map(|(_, pid)| sysinfo::Pid::from_u32(*pid)).collect();
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&pids), false);
        Ok(found
            .into_iter()
            .map(|(address, pid)| PortHolder {
                pid,
                process: sys
                    .process(sysinfo::Pid::from_u32(pid))
                    .map(|p| p.name().to_string_lossy().to_string())
                    .unwrap_or_else(|| "(exited or not visible)".to_string()),
                address,
            })
            .collect())
    }
}

/// At most this many distinct holders are named; the rest are counted.
const MAX_HOLDERS_NAMED: usize = 8;

/// One entry per (process, pid, address) with a count, because a program holds one socket
/// per interface and per family: on a developer machine Chrome alone is dozens of
/// sockets, and a line that repeats it is unreadable. Sorted by pid so the line is stable.
fn group_holders(holders: &[PortHolder], own_pid: u32) -> String {
    let mut groups: Vec<((u32, &str, &str), usize)> = Vec::new();
    for h in holders {
        let key = (h.pid, h.process.as_str(), h.address.as_str());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => groups.push((key, 1)),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let shown: Vec<String> = groups
        .iter()
        .take(MAX_HOLDERS_NAMED)
        .map(|((pid, process, address), n)| {
            let me = if *pid == own_pid { ", this process" } else { "" };
            let times = if *n > 1 { format!(" x{n}") } else { String::new() };
            format!("{process} (pid {pid}{me}) on {address}{times}")
        })
        .collect();
    let more = groups.len().saturating_sub(MAX_HOLDERS_NAMED);
    let tail = if more > 0 { format!(", and {more} more" ) } else { String::new() };
    format!("{}{tail}", shown.join(", "))
}

/// The one log line. Pure, so every shape is unit-tested without a network.
pub fn format_diagnosis(
    probes: &[(Ipv4Addr, Result<(), BindFailure>)],
    holders: &Result<Vec<PortHolder>, String>,
    own_pid: u32,
) -> String {
    let probes_text = if probes.is_empty() {
        "no interface to probe".to_string()
    } else {
        probes
            .iter()
            .map(|(ip, r)| match r {
                Ok(()) => format!("{ip}: set-up works (so this was not a bind failure)"),
                Err(f) => match f.os_error {
                    Some(code) => format!("{ip}: FAILED at {} (os error {code}: {})", f.step, f.message),
                    None => format!("{ip}: FAILED at {} ({})", f.step, f.message),
                },
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let holders_text = match holders {
        Err(why) => format!("holder lookup unavailable ({why})"),
        Ok(h) if h.is_empty() => "nobody holds UDP 5353".to_string(),
        Ok(h) => format!("UDP 5353 is held by {}", group_holders(h, own_pid)),
    };
    format!("{probes_text}. {holders_text}")
}

/// Probe each uncovered address and look up the holders. Blocking; call it from a
/// blocking context. Never fails: a problem becomes part of the text.
pub fn diagnose(missing: &[Ipv4Addr]) -> String {
    let probes: Vec<(Ipv4Addr, Result<(), BindFailure>)> = missing.iter().map(|ip| (*ip, probe_bind(*ip))).collect();
    format_diagnosis(&probes, &udp_5353_holders(), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn holder(pid: u32, process: &str, address: &str) -> PortHolder {
        PortHolder { pid, process: process.to_string(), address: address.to_string() }
    }

    /// The Area54 situation of 2026-10-02: Chrome and the DNS client holding the port,
    /// and a bind that fails, in one readable line.
    #[test]
    fn the_area54_case_reads_as_one_line_naming_the_holders() {
        let probes = vec![(
            ip("192.168.1.26"),
            Err(BindFailure { step: "bind", os_error: Some(10048), message: "Only one usage of each socket address".into() }),
        )];
        let holders = Ok(vec![
            holder(23800, "chrome.exe", "::"),
            holder(5648, "chrome.exe", "0.0.0.0"),
            holder(2964, "svchost.exe", "0.0.0.0"),
        ]);
        let text = format_diagnosis(&probes, &holders, 10708);
        assert!(text.contains("192.168.1.26: FAILED at bind (os error 10048"), "{text}");
        assert!(text.contains("chrome.exe (pid 23800) on ::"), "{text}");
        assert!(text.contains("svchost.exe (pid 2964) on 0.0.0.0"), "{text}");
        assert!(!text.contains("this process"), "{text}");
    }

    #[test]
    fn a_working_set_up_says_so_because_it_rules_out_the_bind() {
        let text = format_diagnosis(&[(ip("192.168.1.26"), Ok(()))], &Ok(vec![holder(10708, "agentmux-srv.exe", "0.0.0.0")]), 10708);
        assert!(text.contains("set-up works (so this was not a bind failure)"), "{text}");
        assert!(text.contains("agentmux-srv.exe (pid 10708, this process) on 0.0.0.0"), "{text}");
    }

    /// Read off narko on 2026-10-02: one program holds a socket per interface and family, so
    /// the raw list repeated Chrome and node dozens of times.
    #[test]
    fn a_program_holding_many_sockets_is_one_entry_with_a_count_and_the_list_is_capped() {
        let mut holders = Vec::new();
        for _ in 0..9 {
            holders.push(holder(29204, "chrome.exe", "0.0.0.0"));
        }
        holders.push(holder(11936, "node.exe", "0.0.0.0"));
        holders.push(holder(11936, "node.exe", "0.0.0.0"));
        let text = format_diagnosis(&[], &Ok(holders), 1);
        assert!(text.contains("chrome.exe (pid 29204) on 0.0.0.0 x9"), "{text}");
        assert!(text.contains("node.exe (pid 11936) on 0.0.0.0 x2"), "{text}");
        assert_eq!(text.matches("chrome.exe").count(), 1, "{text}");

        let many: Vec<PortHolder> = (1..=12).map(|pid| holder(pid, "svc.exe", "0.0.0.0")).collect();
        let capped = format_diagnosis(&[], &Ok(many), 1);
        assert!(capped.contains(", and 4 more"), "{capped}");
        assert_eq!(capped.matches("svc.exe").count(), MAX_HOLDERS_NAMED, "{capped}");
    }

    #[test]
    fn every_other_shape_still_reads_sensibly() {
        assert!(format_diagnosis(&[], &Ok(vec![]), 1).contains("no interface to probe"));
        assert!(format_diagnosis(&[], &Ok(vec![]), 1).contains("nobody holds UDP 5353"));
        let unavailable = format_diagnosis(&[], &Err("only on Windows".into()), 1);
        assert!(unavailable.contains("holder lookup unavailable (only on Windows)"), "{unavailable}");
        let no_code = format_diagnosis(
            &[(ip("10.0.0.5"), Err(BindFailure { step: "join-group", os_error: None, message: "weird".into() }))],
            &Ok(vec![]),
            1,
        );
        assert!(no_code.contains("FAILED at join-group (weird)"), "{no_code}");
    }

    /// Deterministic on every platform: a socket that holds the port WITHOUT address
    /// reuse makes the reuse-bind fail, and the probe says it failed at the bind step
    /// with the OS's own error code. This is the failure the diagnosis exists to name.
    #[test]
    fn an_exclusive_holder_makes_the_probe_fail_at_the_bind_step() {
        let holder = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        // Windows: an exclusive bind. Elsewhere: a plain bind with no reuse flags.
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawSocket;
            const SOL_SOCKET: i32 = 0xffff;
            const SO_EXCLUSIVEADDRUSE: i32 = !4;
            let on: i32 = 1;
            // SAFETY: a valid socket and a 4-byte option value.
            let rc = unsafe {
                windows_sys::Win32::Networking::WinSock::setsockopt(
                    holder.as_raw_socket() as usize,
                    SOL_SOCKET,
                    SO_EXCLUSIVEADDRUSE,
                    (&on as *const i32).cast(),
                    4,
                )
            };
            assert_eq!(rc, 0, "set SO_EXCLUSIVEADDRUSE");
        }
        let any = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT);
        if holder.bind(&any.into()).is_err() {
            // Something on this machine already holds UDP 5353 in a way that blocks
            // even this; the case under test cannot be set up here, which is itself
            // the situation the module diagnoses. Nothing to assert.
            return;
        }
        let failure = probe_bind(Ipv4Addr::LOCALHOST).expect_err("an exclusive holder must make the bind fail");
        assert_eq!(failure.step, "bind", "{failure:?}");
        assert!(failure.os_error.is_some(), "the OS's own error code is kept: {failure:?}");
        drop(holder);
    }

    /// Manual: prints the diagnosis line this machine would log right now, so the format
    /// can be read on real data. `cargo test -p agentmux-srv -- --ignored --nocapture
    /// prints_the_diagnosis_for_this_machine`
    #[test]
    #[ignore = "reads this machine's sockets; run by hand"]
    fn prints_the_diagnosis_for_this_machine() {
        let ips: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| match i.ip() {
                std::net::IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(v4),
                _ => None,
            })
            .collect();
        println!("DIAGNOSIS: {}", diagnose(&ips));
    }

    /// The lookup must see this very process: bind the port here, then ask. This is the
    /// check that the table parsing (byte order, row size, process names) is right.
    #[cfg(windows)]
    #[test]
    fn the_holder_lookup_finds_this_process_on_the_port() {
        let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        sock.set_reuse_address(true).unwrap();
        let any = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT);
        if sock.bind(&any.into()).is_err() {
            return; // another program holds the port exclusively; nothing to check
        }
        let holders = udp_5353_holders().expect("the lookup works");
        let me = holders.iter().find(|h| h.pid == std::process::id());
        let me = me.unwrap_or_else(|| panic!("this process (pid {}) must be listed: {holders:?}", std::process::id()));
        assert_eq!(me.address, "0.0.0.0");
        assert!(!me.process.is_empty() && me.process != "(exited or not visible)", "{me:?}");
        drop(sock);
    }
}
