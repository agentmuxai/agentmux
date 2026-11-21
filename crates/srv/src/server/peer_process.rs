// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which process holds the other end of a loopback connection: the one that connected from `peer` to `server`. It is read
//! from the operating system's own connection table (Windows: the extended TCP table; Linux: `/proc/net/tcp` and the
//! descriptors under `/proc`; macOS: `lsof`). `None` when it cannot be said: the lookup is not supported, the table could
//! not be read, the peer is not an IPv4 loopback address, or no process was found (a proxy or a tunnel holds the connection).
//!
//! The tables are read by the pure functions below, so their tests need no connection.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

/// The process that connected from `peer` to `server`, if the system says.
pub(crate) fn owner_of(peer: SocketAddr, server: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(peer), SocketAddr::V4(server)) = (peer, server) else { return None };
    if !peer.ip().is_loopback() || !server.ip().is_loopback() {
        return None;
    }
    os::owner_of(peer, server)
}

/// One row of the Windows extended TCP table, in the fields the lookup reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) struct TcpRow {
    pub local: SocketAddrV4,
    pub remote: SocketAddrV4,
    pub pid: u32,
}

/// The rows of a `MIB_TCPTABLE_OWNER_PID` buffer (a count, then 24 bytes a row: state, local address, local port, remote
/// address, remote port, owning process; ports in network order in the low 16 bits).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn parse_tcp_table(bytes: &[u8]) -> Vec<TcpRow> {
    let word = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let port = |v: u32| u16::from_be((v & 0xffff) as u16);
    let Some(count) = word(0) else { return Vec::new() };
    (0..count as usize)
        .map_while(|i| {
            let at = 4 + i * 24;
            Some(TcpRow {
                local: SocketAddrV4::new(Ipv4Addr::from(word(at + 4)?.to_le_bytes()), port(word(at + 8)?)),
                remote: SocketAddrV4::new(Ipv4Addr::from(word(at + 12)?.to_le_bytes()), port(word(at + 16)?)),
                pid: word(at + 20)?,
            })
        })
        .collect()
}

/// The process of the row that is the client's end: its local end is `peer` and its remote end is `server`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn owner_in_rows(rows: &[TcpRow], peer: SocketAddrV4, server: SocketAddrV4) -> Option<u32> {
    rows.iter().find(|r| r.local == peer && r.remote == server).map(|r| r.pid)
}

/// The socket inode of the connection from `peer` to `server` in the text of `/proc/net/tcp`.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn inode_in_proc_net_tcp(text: &str, peer: SocketAddrV4, server: SocketAddrV4) -> Option<u64> {
    let end = |s: &str| -> Option<SocketAddrV4> {
        let (ip, port) = s.split_once(':')?;
        let ip = u32::from_str_radix(ip, 16).ok()?;
        Some(SocketAddrV4::new(Ipv4Addr::from(ip.to_ne_bytes()), u16::from_str_radix(port, 16).ok()?))
    };
    text.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        (f.len() > 9 && end(f[1])? == peer && end(f[2])? == server).then(|| f[9].parse().ok()).flatten()
    })
}

/// The process ids `lsof -Fp` named, one `p<pid>` line each.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn pids_in_lsof(out: &str) -> Vec<u32> {
    out.lines().filter_map(|l| l.strip_prefix('p')?.trim().parse().ok()).collect()
}

#[cfg(windows)]
mod os {
    use super::*;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_ALL};
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    pub(super) fn owner_of(peer: SocketAddrV4, server: SocketAddrV4) -> Option<u32> {
        let mut size: u32 = 0;
        // SAFETY: a null table with a zero size only asks how large the table is.
        unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, u32::from(AF_INET), TCP_TABLE_OWNER_PID_ALL, 0) };
        for _ in 0..4 {
            let mut buf = vec![0u8; size as usize + 4096];
            size = buf.len() as u32;
            // SAFETY: `buf` is `size` bytes long and lives through the call.
            let rc = unsafe { GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, u32::from(AF_INET), TCP_TABLE_OWNER_PID_ALL, 0) };
            match rc {
                0 => return owner_in_rows(&parse_tcp_table(&buf), peer, server),
                ERROR_INSUFFICIENT_BUFFER => continue,
                _ => return None,
            }
        }
        None
    }
}

#[cfg(target_os = "linux")]
mod os {
    use super::*;

    pub(super) fn owner_of(peer: SocketAddrV4, server: SocketAddrV4) -> Option<u32> {
        let table = std::fs::read_to_string("/proc/net/tcp").ok()?;
        let inode = inode_in_proc_net_tcp(&table, peer, server)?;
        let want = format!("socket:[{inode}]");
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
            if fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|t| t.to_str() == Some(want.as_str()))) {
                return Some(pid);
            }
        }
        None
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::*;

    pub(super) fn owner_of(peer: SocketAddrV4, _server: SocketAddrV4) -> Option<u32> {
        let arg = format!("TCP@127.0.0.1:{}", peer.port());
        let mut lsof = std::process::Command::new("lsof");
        crate::backend::pane_env::sanitize_external_std_command(&mut lsof);
        let out = lsof.args(["-nP", "-a", "-i", &arg, "-sTCP:ESTABLISHED", "-Fp"]).output().ok()?;
        let own = std::process::id();
        pids_in_lsof(&String::from_utf8_lossy(&out.stdout)).into_iter().find(|p| *p != own)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod os {
    use super::*;

    pub(super) fn owner_of(_peer: SocketAddrV4, _server: SocketAddrV4) -> Option<u32> {
        None
    }
}

#[cfg(test)]
#[path = "tests/peer_process_tests.rs"]
mod tests;
