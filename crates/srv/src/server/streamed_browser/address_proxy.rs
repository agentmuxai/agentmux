// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one way out for the server's Chromium: a SOCKS5 proxy on the loopback address that connects only where
//! `address_policy` allows. Chromium is told to use it for every connection (page loads, scripts, WebSocket, and
//! the browser's own requests), to ask it for name resolution too, and to refuse a local lookup, so a page cannot
//! reach the cloud metadata address, a private range, or this server's own ports, by an address or by a name.
//!
//! The proxy resolves the name itself, judges every address the name resolved to (one refused address refuses the
//! name), and connects to an address it judged: it never resolves a second time, so a name that answers with a
//! public address first and a private one later gets nothing from the second answer.

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use super::address_policy;

/// How long a client has to say what it wants, and how long a connection may take to open.
const HANDSHAKE: Duration = Duration::from_secs(10);
const CONNECT: Duration = Duration::from_secs(15);
/// The most connections open at once: a page that opens thousands is held to this.
const MAX_OPEN: usize = 512;

/// SOCKS5 reply codes (RFC 1928 section 6).
const OK: u8 = 0;
const NOT_ALLOWED: u8 = 2;
const HOST_UNREACHABLE: u8 = 4;
const REFUSED: u8 = 5;
const BAD_COMMAND: u8 = 7;
const BAD_ADDRESS_TYPE: u8 = 8;

/// What a client asked to connect to.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Ip(IpAddr),
    Name(String),
}

/// Turns a name into addresses. The system's resolver in use; a scripted one in the tests.
pub trait Resolve: Send + Sync + 'static {
    fn resolve(&self, name: &str, port: u16) -> impl Future<Output = io::Result<Vec<IpAddr>>> + Send;
}

pub struct SystemResolver;

impl Resolve for SystemResolver {
    async fn resolve(&self, name: &str, port: u16) -> io::Result<Vec<IpAddr>> {
        Ok(tokio::net::lookup_host((name, port)).await?.map(|a| a.ip()).collect())
    }
}

/// The addresses a connection to `target` on `port` may go to, in the order to try them, or the SOCKS5 reply that
/// refuses it. Every address the name resolved to has to pass the policy, and only those are tried (a name that
/// answers `::1` and `127.0.0.1` reaches a development server on either).
pub async fn plan<R: Resolve>(resolver: &R, target: &Target, port: u16, own_ports: &[u16]) -> Result<Vec<SocketAddr>, u8> {
    let addrs = match target {
        Target::Ip(ip) => vec![*ip],
        Target::Name(name) => match resolver.resolve(name, port).await {
            Ok(a) if !a.is_empty() => a,
            _ => return Err(HOST_UNREACHABLE),
        },
    };
    for ip in &addrs {
        if let Err(why) = address_policy::check(*ip, port, own_ports) {
            tracing::info!(target: "streamed-browser", port, why, "address proxy: refused a connection");
            return Err(NOT_ALLOWED);
        }
    }
    Ok(addrs.into_iter().map(|ip| SocketAddr::new(ip, port)).collect())
}

/// Connect to the first of `addrs` that answers, each within the connect limit. The reply for the last failure.
async fn connect_first(addrs: &[SocketAddr]) -> Result<TcpStream, u8> {
    let mut code = HOST_UNREACHABLE;
    for addr in addrs {
        match tokio::time::timeout(CONNECT, TcpStream::connect(addr)).await {
            Ok(Ok(s)) => return Ok(s),
            Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionRefused => code = REFUSED,
            _ => code = HOST_UNREACHABLE,
        }
    }
    Err(code)
}

/// The client's request: the address it wants and the port.
async fn read_request(s: &mut TcpStream) -> Result<(Target, u16), u8> {
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.map_err(|_| REFUSED)?;
    if head[0] != 5 {
        return Err(REFUSED);
    }
    // The whole request is read before any refusal, so a client that sent more than was read is not reset (a reset
    // can overtake the reply) and gets the reply.
    let target = match head[3] {
        1 => {
            let mut b = [0u8; 4];
            s.read_exact(&mut b).await.map_err(|_| REFUSED)?;
            Target::Ip(IpAddr::V4(Ipv4Addr::from(b)))
        }
        4 => {
            let mut b = [0u8; 16];
            s.read_exact(&mut b).await.map_err(|_| REFUSED)?;
            Target::Ip(IpAddr::from(b))
        }
        3 => {
            let len = s.read_u8().await.map_err(|_| REFUSED)? as usize;
            let mut b = vec![0u8; len];
            s.read_exact(&mut b).await.map_err(|_| REFUSED)?;
            Target::Name(String::from_utf8(b).map_err(|_| BAD_ADDRESS_TYPE)?)
        }
        _ => return Err(BAD_ADDRESS_TYPE),
    };
    let port = s.read_u16().await.map_err(|_| REFUSED)?;
    if head[1] != 1 {
        return Err(BAD_COMMAND);
    }
    Ok((target, port))
}

async fn reply(s: &mut TcpStream, code: u8) {
    // Version, reply, reserved, IPv4 address type, the (unused) bound address and port.
    let _ = s.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await;
}

/// One client: greeting, request, a checked connection, then bytes both ways.
async fn serve_one<R: Resolve>(mut client: TcpStream, resolver: Arc<R>) {
    let handshake = async {
        let mut greeting = [0u8; 2];
        client.read_exact(&mut greeting).await.ok()?;
        let mut methods = vec![0u8; greeting[1] as usize];
        client.read_exact(&mut methods).await.ok()?;
        // Only "no authentication": the proxy listens on the loopback address of a machine the page's own code can
        // reach, but it only connects where the policy allows, so there is nothing for a password to protect.
        if greeting[0] != 5 || !methods.contains(&0) {
            let _ = client.write_all(&[5, 0xff]).await;
            return None;
        }
        client.write_all(&[5, 0]).await.ok()?;
        Some(read_request(&mut client).await)
    };
    let Ok(Some(request)) = tokio::time::timeout(HANDSHAKE, handshake).await else { return };
    let (target, port) = match request {
        Ok(r) => r,
        Err(code) => return reply(&mut client, code).await,
    };
    let addrs = match plan(&*resolver, &target, port, &own_ports_with_proxy()).await {
        Ok(a) => a,
        Err(code) => return reply(&mut client, code).await,
    };
    let mut upstream = match connect_first(&addrs).await {
        Ok(s) => s,
        Err(code) => return reply(&mut client, code).await,
    };
    reply(&mut client, OK).await;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

/// This server's own ports, this proxy's, and the development-server router's.
fn own_ports_with_proxy() -> Vec<u16> {
    let mut ports = address_policy::own_ports();
    ports.extend(PROXY_PORT.get());
    ports.push(crate::backend::dev_proxy::port());
    ports
}

static PROXY_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

/// Accept clients on `listener` until the process ends.
async fn accept_loop<R: Resolve>(listener: TcpListener, resolver: Arc<R>) {
    let open = Arc::new(Semaphore::new(MAX_OPEN));
    loop {
        let Ok((client, _)) = listener.accept().await else {
            // A persistent failure (out of descriptors) must not spin.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            continue;
        };
        let Ok(permit) = open.clone().try_acquire_owned() else { continue };
        let resolver = resolver.clone();
        tokio::spawn(async move {
            serve_one(client, resolver).await;
            drop(permit);
        });
    }
}

/// Start the proxy on a loopback port of the system's choosing, once; the port is the same for every later call.
pub async fn start() -> io::Result<u16> {
    static STARTED: tokio::sync::OnceCell<u16> = tokio::sync::OnceCell::const_new();
    STARTED
        .get_or_try_init(|| async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
            let port = listener.local_addr()?.port();
            let _ = PROXY_PORT.set(port);
            address_policy::note_ports_from_env();
            tokio::spawn(accept_loop(listener, Arc::new(SystemResolver)));
            Ok(port)
        })
        .await
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// A resolver that answers from a script, one answer per call, so a name can change its answer.
    struct Scripted(Mutex<VecDeque<Vec<IpAddr>>>);

    impl Scripted {
        fn new(answers: &[&[&str]]) -> Self {
            Self(Mutex::new(answers.iter().map(|a| a.iter().map(|s| s.parse().unwrap()).collect()).collect()))
        }
    }

    impl Resolve for Scripted {
        async fn resolve(&self, _name: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
            Ok(self.0.lock().unwrap().pop_front().unwrap_or_default())
        }
    }

    fn name(n: &str) -> Target {
        Target::Name(n.to_string())
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_a_private_address_is_refused() {
        let lan = [std::net::Ipv4Addr::new(10, 0, 0, 5), std::net::Ipv4Addr::new(192, 168, 1, 1)].map(|a| a.to_string());
        for answer in [lan[0].as_str(), "169.254.170.2", lan[1].as_str(), "fd00:ec2::254", "127.0.0.1"] {
            let r = Scripted::new(&[&[answer]]);
            // 127.0.0.1 is refused here because the port is one of the server's own.
            let got = plan(&r, &name("evil.example"), 8190, &[8190]).await;
            assert_eq!(got, Err(NOT_ALLOWED), "{answer}");
        }
    }

    #[tokio::test]
    async fn a_name_with_one_private_address_among_public_ones_is_refused() {
        let lan = std::net::Ipv4Addr::new(10, 0, 0, 5).to_string();
        let r = Scripted::new(&[&["93.184.216.34", lan.as_str()]]);
        assert_eq!(plan(&r, &name("mixed.example"), 443, &[]).await, Err(NOT_ALLOWED));
    }

    #[tokio::test]
    async fn a_name_that_answers_public_first_and_private_later_gets_nothing_from_the_later_answer() {
        // The rebinding trick: the first lookup is public, the next is the metadata address.
        let r = Scripted::new(&[&["93.184.216.34"], &["169.254.170.2"]]);
        // The first connection is planned for the public address the policy saw...
        assert_eq!(plan(&r, &name("rebind.example"), 80, &[]).await, Ok(vec![SocketAddr::new("93.184.216.34".parse().unwrap(), 80)]));
        // ...and a new connection asks again and is refused. A connection already planned never resolves again,
        // because `serve_one` connects to the planned address itself.
        assert_eq!(plan(&r, &name("rebind.example"), 80, &[]).await, Err(NOT_ALLOWED));
    }

    #[tokio::test]
    async fn every_judged_address_of_a_name_is_offered_in_order() {
        // `localhost` answers `::1` first; a development server listening on IPv4 only is reached on the second.
        let r = Scripted::new(&[&["::1", "127.0.0.1"]]);
        let got = plan(&r, &name("localhost"), 3000, &[]).await.unwrap();
        assert_eq!(got, vec![SocketAddr::new("::1".parse().unwrap(), 3000), SocketAddr::new("127.0.0.1".parse().unwrap(), 3000)]);
    }

    #[tokio::test]
    async fn the_connection_falls_through_to_the_next_address_when_the_first_is_refused() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let live = listener.local_addr().unwrap();
        // Port 1 on the loopback address nothing listens on: refused at once.
        let dead = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1);
        assert!(connect_first(&[dead, live]).await.is_ok());
        assert_eq!(connect_first(&[dead]).await.err(), Some(REFUSED));
    }

    #[test]
    fn the_development_server_router_is_one_of_the_servers_own_ports() {
        assert!(own_ports_with_proxy().contains(&crate::backend::dev_proxy::port()));
    }

    #[tokio::test]
    async fn an_address_given_directly_is_judged_without_a_lookup() {
        let r = Scripted::new(&[]);
        assert_eq!(plan(&r, &Target::Ip("169.254.170.2".parse().unwrap()), 80, &[]).await, Err(NOT_ALLOWED));
        assert_eq!(plan(&r, &Target::Ip("::ffff:a00:1".parse().unwrap()), 80, &[]).await, Err(NOT_ALLOWED));
        assert!(plan(&r, &Target::Ip("8.8.8.8".parse().unwrap()), 443, &[]).await.is_ok());
    }

    #[tokio::test]
    async fn a_name_that_does_not_resolve_is_unreachable() {
        let r = Scripted::new(&[&[]]);
        assert_eq!(plan(&r, &name("nothing.example"), 443, &[]).await, Err(HOST_UNREACHABLE));
    }

    /// A SOCKS5 client: connect to the proxy, ask for `request`, and return the reply code and the open socket.
    async fn ask(proxy: u16, request: &[u8]) -> (u8, TcpStream) {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, proxy)).await.unwrap();
        s.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        s.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        s.write_all(request).await.unwrap();
        let mut rep = [0u8; 10];
        s.read_exact(&mut rep).await.unwrap();
        (rep[1], s)
    }

    fn connect_ip(ip: [u8; 4], port: u16) -> Vec<u8> {
        let mut r = vec![5, 1, 0, 1];
        r.extend(ip);
        r.extend(port.to_be_bytes());
        r
    }

    fn connect_name(name: &str, port: u16) -> Vec<u8> {
        let mut r = vec![5, 1, 0, 3, name.len() as u8];
        r.extend(name.as_bytes());
        r.extend(port.to_be_bytes());
        r
    }

    /// A proxy on a test port with a scripted resolver, and an echo server on the loopback address.
    async fn proxy_and_echo(resolver: Scripted) -> (u16, u16) {
        let echo = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = echo.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 64];
                    while let Ok(n) = c.read(&mut buf).await {
                        if n == 0 || c.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        tokio::spawn(accept_loop(listener, Arc::new(resolver)));
        (proxy_port, echo_port)
    }

    #[tokio::test]
    async fn the_proxy_carries_a_connection_to_a_loopback_development_server() {
        let (proxy, echo) = proxy_and_echo(Scripted::new(&[])).await;
        let (code, mut s) = ask(proxy, &connect_ip([127, 0, 0, 1], echo)).await;
        assert_eq!(code, OK);
        s.write_all(b"hello").await.unwrap();
        let mut back = [0u8; 5];
        s.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"hello");
    }

    #[tokio::test]
    async fn a_page_cannot_reach_a_private_range_or_the_metadata_address_through_the_proxy() {
        let (proxy, _) = proxy_and_echo(Scripted::new(&[])).await;
        for ip in [[169, 254, 170, 2], [169, 254, 169, 254], [10, 1, 2, 3], [192, 168, 0, 1], [172, 16, 0, 9]] {
            let (code, _) = ask(proxy, &connect_ip(ip, 80)).await;
            assert_eq!(code, NOT_ALLOWED, "{ip:?}");
        }
    }

    #[tokio::test]
    async fn a_page_cannot_reach_this_servers_own_ports_through_the_proxy() {
        let (proxy, echo) = proxy_and_echo(Scripted::new(&[])).await;
        // The echo server stands for one of the server's own listeners: once its port is noted, it is refused.
        address_policy::note_own_port(echo);
        let (code, _) = ask(proxy, &connect_ip([127, 0, 0, 1], echo)).await;
        assert_eq!(code, NOT_ALLOWED);
        let (code, _) = ask(proxy, &connect_name("localhost", echo)).await;
        // `localhost` goes to the scripted resolver, which has no answer: unreachable, never a connection.
        assert_ne!(code, OK);
    }

    #[tokio::test]
    async fn a_name_that_turns_private_on_its_second_lookup_is_carried_once_and_then_refused() {
        // First lookup: the loopback address of a development server (allowed). Second: a private address.
        let lan = std::net::Ipv4Addr::new(10, 0, 0, 1).to_string();
        let (proxy, echo) = proxy_and_echo(Scripted::new(&[&["127.0.0.1"], &[lan.as_str()]])).await;
        let (code, _) = ask(proxy, &connect_name("rebind.example", echo)).await;
        assert_eq!(code, OK);
        let (code, _) = ask(proxy, &connect_name("rebind.example", echo)).await;
        assert_eq!(code, NOT_ALLOWED);
    }

    #[tokio::test]
    async fn only_connect_is_served() {
        let (proxy, _) = proxy_and_echo(Scripted::new(&[])).await;
        // BIND (2) and UDP ASSOCIATE (3) are refused: the proxy opens outgoing TCP connections and nothing else.
        for command in [2u8, 3] {
            let (code, _) = ask(proxy, &[5, command, 0, 1, 127, 0, 0, 1, 0, 80]).await;
            assert_eq!(code, BAD_COMMAND);
        }
    }
}
