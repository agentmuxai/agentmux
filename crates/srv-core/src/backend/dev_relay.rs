// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A relay into the agents network, for a dev server srv cannot reach by the container's IP.
//!
//! The dev proxy (`dev_proxy`) forwards to the address a container agent's dev server listens on.
//! On Linux that is the container's address on the agents network. On Docker Desktop (Windows and
//! macOS) the daemon runs in a VM and container addresses are not routable from the host, so the
//! same connection times out. There, one small container per host sits on the agents network with a
//! single port published on `127.0.0.1`; srv opens a connection to that port, names the container
//! and port it wants, and the relay joins the two. One relay serves every agent and every project,
//! it needs nothing published per dev server, and a WebSocket is just another connection.
//!
//! The relay runs the agent image's own `node` (nothing to pull), and takes a request line
//! `<token> <container> <port>`, where the token is generated here and only the loopback caller
//! that knows it is served. It only reaches containers named `agentmux-*`. srv adopts a running relay
//! after a restart (the token is in its environment) and replaces one that is gone.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
use bollard::models::{HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::backend::container::{ContainerManager, AGENTMUX_DOCKER_NETWORK};

/// The relay container's name. Not under `agentmux-`: that is the namespace of agent containers
/// (`container_name_for_slug`), and an agent called `dev-relay` must not be the relay's neighbour.
pub const RELAY_NAME: &str = "amx-dev-relay";
/// Marks the relay container, so nothing else is taken for it.
const LABEL: &str = "ai.agentmux.dev-relay";
/// The port the relay listens on inside its container.
const RELAY_PORT: u16 = 9100;
/// How long a connection through the relay may take to be set up.
const SETUP: Duration = Duration::from_secs(8);

/// What the relay does, as one `node -e` program. A request line, then bytes both ways.
const PROGRAM: &str = r#"const net=require('net');const T=process.env.AMX_RELAY_TOKEN;
net.createServer(c=>{let b=Buffer.alloc(0);c.on('error',()=>{});
const on=d=>{b=Buffer.concat([b,d]);const i=b.indexOf(10);if(i<0){if(b.length>256)c.destroy();return}
c.off('data',on);c.pause();const [t,h,p]=b.subarray(0,i).toString().trim().split(' ');
if(!T||t!==T||!/^agentmux-[A-Za-z0-9_.-]+$/.test(h)||!(+p>0&&+p<65536)){c.end('ERR refused\n');return}
const s=net.connect(+p,h);s.on('error',e=>{c.end('ERR '+(e.code||'failed')+'\n')});
s.on('connect',()=>{c.write('OK\n');const r=b.subarray(i+1);if(r.length)s.write(r);c.pipe(s);s.pipe(c);c.resume()});
s.on('close',()=>c.destroy());c.on('close',()=>s.destroy())};
c.on('data',on)}).listen(9100,'0.0.0.0')"#;

/// Where the relay can be reached and the token that opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEndpoint {
    pub addr: SocketAddr,
    pub token: String,
}

/// Whether this Docker keeps container addresses to itself: Docker Desktop, and any macOS or Windows
/// host. `AGENTMUX_DEV_PROXY_RELAY=1` (or `0`) overrides it.
pub async fn needed(manager: &ContainerManager) -> bool {
    if let Ok(v) = std::env::var("AGENTMUX_DEV_PROXY_RELAY") {
        return v == "1";
    }
    if cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    manager
        .docker()
        .info()
        .await
        .ok()
        .and_then(|i| i.operating_system)
        .is_some_and(|os| os.contains("Docker Desktop"))
}

fn token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn published_port(details: &bollard::models::ContainerInspectResponse) -> Option<u16> {
    let ports = details.network_settings.as_ref()?.ports.as_ref()?;
    let binding = ports.get(&format!("{RELAY_PORT}/tcp"))?.as_ref()?.first()?;
    binding.host_port.as_deref()?.parse().ok()
}

fn env_token(details: &bollard::models::ContainerInspectResponse) -> Option<String> {
    let env = details.config.as_ref()?.env.as_ref()?;
    env.iter().find_map(|e| e.strip_prefix("AMX_RELAY_TOKEN=").map(str::to_string))
}

/// The running relay, created if there is none. `image` is an image already on this machine (the
/// one an agent container runs from).
pub async fn ensure(manager: &ContainerManager, image: &str) -> Result<RelayEndpoint, String> {
    ensure_named(manager, image, RELAY_NAME).await
}

async fn ensure_named(manager: &ContainerManager, image: &str, name: &str) -> Result<RelayEndpoint, String> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _one_at_a_time = LOCK.get_or_init(Default::default).lock().await;
    let docker = manager.docker();

    if let Ok(details) = docker.inspect_container(name, None).await {
        let ours = details.config.as_ref().and_then(|c| c.labels.as_ref()).is_some_and(|l| l.contains_key(LABEL));
        if ours {
            let running = details.state.as_ref().and_then(|s| s.running).unwrap_or(false);
            if !running {
                let _ = docker.start_container(name, None::<StartContainerOptions<String>>).await;
            }
            let details = docker.inspect_container(name, None).await.map_err(|e| e.to_string())?;
            if let (Some(port), Some(token)) = (published_port(&details), env_token(&details)) {
                if details.state.as_ref().and_then(|s| s.running).unwrap_or(false) {
                    let addr = SocketAddr::from(([127, 0, 0, 1], port));
                    wait_until_listening(addr).await?;
                    return Ok(RelayEndpoint { addr, token });
                }
            }
        }
        if !ours {
            return Err(format!("a container called {name} exists and is not the dev relay; not touching it"));
        }
        // Ours but broken: replace it.
        let _ = docker
            .remove_container(name, Some(bollard::container::RemoveContainerOptions { force: true, ..Default::default() }))
            .await;
    }

    let token = token();
    let mut ports = HashMap::new();
    ports.insert(format!("{RELAY_PORT}/tcp"), Some(vec![PortBinding { host_ip: Some("127.0.0.1".into()), host_port: Some(String::new()) }]));
    let config = Config {
        image: Some(image.to_string()),
        entrypoint: Some(vec!["node".into(), "-e".into(), PROGRAM.into()]),
        cmd: Some(vec![]),
        env: Some(vec![format!("AMX_RELAY_TOKEN={token}")]),
        labels: Some(HashMap::from([(LABEL.to_string(), "1".to_string())])),
        exposed_ports: Some(HashMap::from([(format!("{RELAY_PORT}/tcp"), HashMap::new())])),
        host_config: Some(HostConfig {
            network_mode: Some(AGENTMUX_DOCKER_NETWORK.to_string()),
            port_bindings: Some(ports),
            init: Some(true),
            restart_policy: Some(RestartPolicy { name: Some(RestartPolicyNameEnum::UNLESS_STOPPED), maximum_retry_count: None }),
            ..Default::default()
        }),
        ..Default::default()
    };
    docker
        .create_container(Some(CreateContainerOptions { name, platform: None }), config)
        .await
        .map_err(|e| format!("couldn't create the dev relay: {e}"))?;
    docker
        .start_container(name, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| format!("couldn't start the dev relay: {e}"))?;
    let details = docker.inspect_container(name, None).await.map_err(|e| e.to_string())?;
    let port = published_port(&details).ok_or("the dev relay has no published port")?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    wait_until_listening(addr).await?;
    Ok(RelayEndpoint { addr, token })
}

/// Waits until the relay answers: Docker publishes the port the moment the container starts, before
/// node is listening in it, and a connection in that gap is reset.
async fn wait_until_listening(addr: SocketAddr) -> Result<(), String> {
    for _ in 0..50 {
        if let Ok(mut s) = TcpStream::connect(addr).await {
            // A request the relay refuses by design; any answer at all means it is up.
            if s.write_all(b"ready\n").await.is_ok() {
                let mut buf = [0u8; 16];
                if matches!(s.read(&mut buf).await, Ok(n) if n > 0) {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err("the dev relay did not start listening".to_string())
}

/// A connection to `container:port` through the relay at `via`.
pub async fn connect(via: &RelayEndpoint, container: &str, port: u16) -> std::io::Result<TcpStream> {
    let attempt = async {
        let mut stream = TcpStream::connect(via.addr).await?;
        stream.write_all(format!("{} {container} {port}\n", via.token).as_bytes()).await?;
        // The answer is one short line; read it a byte at a time so nothing of the stream is consumed.
        let mut line = Vec::with_capacity(32);
        loop {
            let mut b = [0u8; 1];
            if stream.read(&mut b).await? == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the dev relay closed the connection"));
            }
            if b[0] == b'\n' {
                break;
            }
            line.push(b[0]);
            if line.len() > 64 {
                return Err(std::io::Error::other("the dev relay answered with something else"));
            }
        }
        match line.as_slice() {
            b"OK" => Ok(stream),
            other => Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, String::from_utf8_lossy(other).into_owned())),
        }
    };
    tokio::time::timeout(SETUP, attempt).await.unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A stand-in relay speaking the same line protocol: answers `OK` to the right token and then echoes.
    async fn fake_relay(token: &'static str) -> RelayEndpoint {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut line = Vec::new();
                    loop {
                        let mut b = [0u8; 1];
                        if s.read(&mut b).await.unwrap_or(0) == 0 {
                            return;
                        }
                        if b[0] == b'\n' {
                            break;
                        }
                        line.push(b[0]);
                    }
                    let line = String::from_utf8_lossy(&line).into_owned();
                    if line.split(' ').next() != Some(token) {
                        let _ = s.write_all(b"ERR refused\n").await;
                        return;
                    }
                    let _ = s.write_all(b"OK\n").await;
                    let mut buf = [0u8; 256];
                    while let Ok(n) = s.read(&mut buf).await {
                        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        RelayEndpoint { addr, token: token.to_string() }
    }

    #[tokio::test]
    async fn a_connection_through_the_relay_carries_bytes_after_the_ok() {
        let relay = fake_relay("secret").await;
        let mut s = connect(&relay, "agentmux-korp", 7000).await.expect("connected");
        s.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[tokio::test]
    async fn the_wrong_token_is_refused_with_the_relays_reason() {
        let relay = RelayEndpoint { token: "wrong".into(), ..fake_relay("secret").await };
        let err = connect(&relay, "agentmux-korp", 7000).await.expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::ConnectionRefused);
        assert!(err.to_string().contains("refused"), "{err}");
    }

    #[tokio::test]
    async fn a_relay_that_is_not_there_is_an_error_not_a_hang() {
        let gone = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let relay = RelayEndpoint { addr: gone, token: "t".into() };
        assert!(connect(&relay, "agentmux-korp", 7000).await.is_err());
    }

    #[test]
    fn the_relay_program_only_reaches_agent_containers_and_needs_the_token() {
        assert!(PROGRAM.contains("agentmux-[A-Za-z0-9_.-]+"));
        assert!(PROGRAM.contains("t!==T"));
        assert!(PROGRAM.contains(&RELAY_PORT.to_string()));
    }

    /// Against a real daemon: the relay is created, reached, adopted again and replaced.
    ///   cargo test -p agentmux-srv --bin agentmux-srv -- --ignored --nocapture itest_dev_relay
    #[tokio::test]
    #[ignore]
    async fn itest_dev_relay_reaches_a_container_by_name_and_is_adopted() {
        use bollard::network::CreateNetworkOptions;
        use bollard::container::RemoveContainerOptions;
        let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
        cm.check_available().await.expect("docker daemon must be reachable");
        let docker = cm.docker();
        let image = "ghcr.io/agentmuxai/agent-base:0.60.3";
        let relay_name = "amx-itest-dev-relay";
        let target = "agentmux-itest-relay-target";
        async fn rm(docker: &bollard::Docker, name: &str) {
            let _ = docker.remove_container(name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        }
        rm(docker, relay_name).await;
        rm(docker, target).await;
        let _ = docker.create_network(CreateNetworkOptions { name: AGENTMUX_DOCKER_NETWORK, driver: "bridge", ..Default::default() }).await;

        let server = "require('http').createServer((q,r)=>r.end('hello from '+q.headers.host)).listen(7000,'0.0.0.0')";
        docker
            .create_container(
                Some(CreateContainerOptions { name: target, platform: None }),
                Config {
                    image: Some(image.to_string()),
                    cmd: Some(vec!["node".into(), "-e".into(), server.into()]),
                    host_config: Some(HostConfig { network_mode: Some(AGENTMUX_DOCKER_NETWORK.to_string()), ..Default::default() }),
                    ..Default::default()
                },
            )
            .await
            .expect("create target");
        docker.start_container(target, None::<StartContainerOptions<String>>).await.expect("start target");
        tokio::time::sleep(Duration::from_secs(2)).await;

        // A container that is not the relay, under the relay's name, is left alone and reported.
        let foreign = "amx-itest-foreign";
        rm(docker, foreign).await;
        docker
            .create_container(Some(CreateContainerOptions { name: foreign, platform: None }), Config { image: Some(image.to_string()), cmd: Some(vec!["sleep".into(), "60".into()]), ..Default::default() })
            .await
            .expect("create foreign");
        let refused = ensure_named(&cm, image, foreign).await.expect_err("not the relay's to replace");
        let still_there = docker.inspect_container(foreign, None).await.is_ok();
        rm(docker, foreign).await;
        assert!(refused.contains("not touching it") && still_there, "{refused}");

        let first = ensure_named(&cm, image, relay_name).await.expect("relay");
        let again = ensure_named(&cm, image, relay_name).await.expect("adopted");
        let mut s = connect(&first, target, 7000).await.expect("through the relay to the target");
        s.write_all(b"GET / HTTP/1.1\r\nHost: localhost:7000\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).await.unwrap();
        let refused_name = connect(&first, "not-an-agent", 7000).await.expect_err("only agent containers");
        let refused_token = connect(&RelayEndpoint { token: "wrong".into(), ..first.clone() }, target, 7000).await.expect_err("token");
        let no_such = connect(&first, "agentmux-nobody-here", 7000).await.expect_err("no such container");

        rm(docker, relay_name).await;
        rm(docker, target).await;

        assert_eq!(first, again, "the running relay is adopted, not replaced");
        assert!(first.addr.ip().is_loopback(), "published on loopback only: {first:?}");
        assert!(answer.contains("200 OK") && answer.contains("hello from localhost:7000"), "{answer}");
        assert!(refused_name.to_string().contains("refused"), "{refused_name}");
        assert!(refused_token.to_string().contains("refused"), "{refused_token}");
        assert!(no_such.to_string().to_uppercase().contains("ERR"), "{no_such}");
    }
}
