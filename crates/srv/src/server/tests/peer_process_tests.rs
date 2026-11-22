// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn addr(ip: [u8; 4], port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(ip), port)
}

/// A `MIB_TCPTABLE_OWNER_PID` buffer: the count, then a row of six little-endian words each.
fn table(rows: &[(SocketAddrV4, SocketAddrV4, u32)]) -> Vec<u8> {
    let mut out = (rows.len() as u32).to_le_bytes().to_vec();
    for (local, remote, pid) in rows {
        let port = |p: u16| u32::from(p.to_be());
        for word in [5, u32::from_le_bytes(local.ip().octets()), port(local.port()), u32::from_le_bytes(remote.ip().octets()), port(remote.port()), *pid] {
            out.extend(word.to_le_bytes());
        }
    }
    out
}

#[test]
fn the_windows_table_names_the_process_at_the_clients_end() {
    let (peer, server) = (addr([127, 0, 0, 1], 50123), addr([127, 0, 0, 1], 7380));
    let rows = parse_tcp_table(&table(&[(server, peer, 111), (peer, server, 222), (addr([127, 0, 0, 1], 50124), server, 333)]));
    assert_eq!(rows.len(), 3);
    assert_eq!(owner_in_rows(&rows, peer, server), Some(222), "the client's row, not the server's accepted end");
    assert_eq!(owner_in_rows(&rows, addr([127, 0, 0, 1], 50999), server), None, "a peer the table does not hold");
}

#[test]
fn a_cut_short_windows_table_is_read_as_far_as_it_goes() {
    let (peer, server) = (addr([127, 0, 0, 1], 50123), addr([127, 0, 0, 1], 7380));
    let mut bytes = table(&[(peer, server, 222), (server, peer, 111)]);
    bytes.truncate(bytes.len() - 10);
    assert_eq!(parse_tcp_table(&bytes).len(), 1);
    assert!(parse_tcp_table(&[]).is_empty());
}

#[test]
fn proc_net_tcp_gives_the_inode_of_the_clients_socket() {
    let hex = |a: SocketAddrV4| format!("{:08X}:{:04X}", u32::from_ne_bytes(a.ip().octets()), a.port());
    let (peer, server) = (addr([127, 0, 0, 1], 50123), addr([127, 0, 0, 1], 7380));
    let line = |local: SocketAddrV4, remote: SocketAddrV4, inode: u64| {
        format!("   0: {} {} 01 00000000:00000000 00:00000000 00000000  1000        0 {inode} 1 0000000000000000 100 0 0 10 0", hex(local), hex(remote))
    };
    let text = format!(
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n{}\n{}\n",
        line(server, peer, 1111),
        line(peer, server, 2222)
    );
    assert_eq!(inode_in_proc_net_tcp(&text, peer, server), Some(2222));
    assert_eq!(inode_in_proc_net_tcp(&text, addr([127, 0, 0, 1], 1), server), None);
}

#[test]
fn the_windows_table_names_the_processes_that_listen_on_an_address() {
    let (server, client) = (addr([127, 0, 0, 1], 7380), addr([127, 0, 0, 1], 50123));
    let nothing = addr([0, 0, 0, 0], 0);
    let rows = parse_tcp_table(&table(&[
        (server, nothing, 111),
        (server, client, 222),
        (client, server, 333),
        (addr([0, 0, 0, 0], 7381), nothing, 444),
        (addr([127, 0, 0, 1], 7382), nothing, 555),
    ]));
    assert_eq!(listeners_in_rows(&rows, server), vec![111], "the listening row, not a connection's either end");
    assert_eq!(listeners_in_rows(&rows, addr([127, 0, 0, 1], 7381)), vec![444], "a listener on every address answers on loopback");
    assert!(listeners_in_rows(&rows, addr([127, 0, 0, 1], 7399)).is_empty(), "nothing listens there");
}

#[test]
fn proc_net_tcp_gives_the_inodes_that_listen_on_an_address() {
    let hex = |a: SocketAddrV4| format!("{:08X}:{:04X}", u32::from_ne_bytes(a.ip().octets()), a.port());
    let (server, client, any) = (addr([127, 0, 0, 1], 7380), addr([127, 0, 0, 1], 50123), addr([0, 0, 0, 0], 0));
    let line = |local: SocketAddrV4, remote: SocketAddrV4, state: &str, inode: u64| {
        format!("   0: {} {} {state} 00000000:00000000 00:00000000 00000000  1000        0 {inode} 1 0000000000000000 100 0 0 10 0", hex(local), hex(remote))
    };
    let text = format!(
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n{}\n{}\n{}\n",
        line(server, any, "0A", 1111),
        line(server, client, "01", 2222),
        line(addr([0, 0, 0, 0], 7381), any, "0A", 3333)
    );
    assert_eq!(listening_inodes_in_proc_net_tcp(&text, server), vec![1111], "the listening socket, not an accepted connection");
    assert_eq!(listening_inodes_in_proc_net_tcp(&text, addr([127, 0, 0, 1], 7381)), vec![3333]);
    assert!(listening_inodes_in_proc_net_tcp(&text, addr([127, 0, 0, 1], 9)).is_empty());
}

#[test]
fn only_a_loopback_ipv4_listener_is_looked_up() {
    assert_eq!(listeners_at("[::1]:7380".parse().unwrap()), None);
    assert_eq!(listeners_at("203.0.113.9:7380".parse().unwrap()), None);
}

/// On this system the lookup finds the process that listens on a real loopback port: this one.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_real_loopback_listener_is_found_to_be_this_process() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    assert_eq!(listeners_at(listener.local_addr().unwrap()), Some(vec![std::process::id()]));
}

#[test]
fn lsof_names_every_process_it_lists() {
    assert_eq!(pids_in_lsof("p123\nf5\np456\n"), vec![123, 456]);
    assert!(pids_in_lsof("").is_empty());
}

#[test]
fn only_a_loopback_ipv4_peer_is_looked_up() {
    let server: SocketAddr = "127.0.0.1:7380".parse().unwrap();
    assert_eq!(owner_of("[::1]:50123".parse().unwrap(), server), None, "an IPv6 peer is not looked up");
    assert_eq!(owner_of("203.0.113.9:50123".parse().unwrap(), server), None, "a peer that is not loopback is not looked up");
}

/// On this system the lookup finds the process that holds a real loopback connection: this one.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_real_loopback_connection_is_found_to_be_this_process() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap();
    let client = std::net::TcpStream::connect(server).unwrap();
    let (_accepted, _) = listener.accept().unwrap();
    assert_eq!(owner_of(client.local_addr().unwrap(), server), Some(std::process::id()));
}
