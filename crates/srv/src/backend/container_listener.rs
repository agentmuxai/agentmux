// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How a container agent's `agentmux-mcp` and hooks reach srv on Linux (P9 of the container-parity
//! work).
//!
//! srv listens on loopback only, and on Linux a container cannot reach the host's loopback:
//! `host.docker.internal` resolves to the bridge gateway, where nothing listened. So srv also
//! listens on the gateway address of the `agentmux-agents` network, with these limits
//! (AgentY, security lane):
//!
//! 1. **Only that address**, never `0.0.0.0`, and only on Linux. A gateway that is not a private
//!    address of this host (Docker Desktop's and rootless Docker's live in a VM or another
//!    network namespace, where the bind fails) is not used and the old route stays.
//! 2. **Only container tokens.** The router is the full router behind [`layer`], which admits a
//!    request only with a live container token (`amxc_…`): the instance key, the LAN key and no
//!    key at all are refused, and so is every route on the denylist
//!    ([`container_routes`](crate::backend::container_routes)): the operator RPCs, `/ws` and what runs
//!    or writes on the host.
//! 3. **Only from the agents subnet.** The peer address must be inside the network's subnet.
//!
//! The container's `AGENTMUX_LOCAL_URL` becomes `http://<gateway>:<port>`
//! ([`url`], applied in `container_credential::container_exec_env`).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Mutex, OnceLock};

use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde_json::json;

use crate::backend::container_credential::grant_for;

/// An IPv4 network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    net: u32,
    mask: u32,
}

impl Subnet {
    /// A CIDR block such as `a.b.0.0/16`.
    pub fn parse(cidr: &str) -> Option<Self> {
        let (ip, bits) = cidr.trim().split_once('/')?;
        let ip: Ipv4Addr = ip.parse().ok()?;
        let bits: u32 = bits.parse().ok().filter(|b| (1..=32).contains(b))?;
        let mask = u32::MAX << (32 - bits);
        Some(Self { net: u32::from(ip) & mask, mask })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => u32::from(v4) & self.mask == self.net,
            // A mapped address (`::ffff:a.b.c.d`) is the IPv4 one.
            IpAddr::V6(v6) => v6.to_ipv4_mapped().is_some_and(|v4| self.contains(IpAddr::V4(v4))),
        }
    }
}

/// Why a request on the container listener is refused, or `Ok`. The pure part of [`layer`].
pub fn check(peer: IpAddr, subnet: Option<&Subnet>, token: Option<&str>, path: &str) -> Result<(), (StatusCode, String)> {
    let Some(subnet) = subnet else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "the container listener is not set up".to_string()));
    };
    if !subnet.contains(peer) {
        return Err((StatusCode::FORBIDDEN, "not from the agents network".to_string()));
    }
    // Only a live container token: the instance key and the LAN key are not tokens, and an
    // unknown string has no grant.
    if token.map_or(true, |t| grant_for(t).is_none()) {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".to_string()));
    }
    if let Some(why) = crate::backend::container_routes::refusal_message(path) {
        return Err((StatusCode::FORBIDDEN, why));
    }
    Ok(())
}

/// `router` as the container listener serves it: every request is [`check`]ed first, against the
/// subnet `subnet()` gives at that moment.
pub fn layer(router: Router, subnet: impl Fn() -> Option<Subnet> + Clone + Send + Sync + 'static) -> Router {
    router.layer(axum::middleware::from_fn(move |req: Request, next: Next| {
        let subnet = subnet.clone();
        async move {
            let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
            let token = req.headers().get(agentmux_common::AUTH_KEY_HEADER).and_then(|v| v.to_str().ok());
            let verdict = match peer {
                Some(peer) => check(peer, subnet().as_ref(), token, req.uri().path()),
                None => Err((StatusCode::FORBIDDEN, "unknown peer".to_string())),
            };
            match verdict {
                Ok(()) => next.run(req).await,
                Err((status, why)) => {
                    tracing::warn!(path = %req.uri().path(), peer = ?peer, %why, "container listener refused a request");
                    (status, Json(json!({ "error": why }))).into_response()
                }
            }
        }
    }))
}

/// What the listener is bound to.
struct Binding {
    gateway: Ipv4Addr,
    subnet: Subnet,
    port: u16,
    stop: tokio_util::sync::CancellationToken,
}

static ROUTER: OnceLock<Router> = OnceLock::new();
static BOUND: Mutex<Option<Binding>> = Mutex::new(None);
#[cfg(target_os = "linux")]
static BINDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Hands over the container router (`SrvRouters::container`). Called once at boot.
pub fn set_router(router: Router) {
    let _ = ROUTER.set(router);
}

/// The agents subnet while the listener is up; what [`layer`] checks the peer against.
pub fn current_subnet() -> Option<Subnet> {
    BOUND.lock().unwrap().as_ref().map(|b| b.subnet)
}

/// `http://<gateway>:<port>` while the listener is up: what a container's `AGENTMUX_LOCAL_URL`
/// should be.
pub fn url() -> Option<String> {
    BOUND.lock().unwrap().as_ref().map(|b| format!("http://{}:{}", b.gateway, b.port))
}

/// Points a container turn's `AGENTMUX_LOCAL_URL` at the listener, if it is up. Called when the
/// environment is built and again once the container has started, which is what binds the
/// listener on the first turn after srv restarts.
pub fn point_env_at_listener(env: &mut [(String, String)]) {
    if let Some(url) = url() {
        if let Some(slot) = env.iter_mut().find(|(k, _)| k == "AGENTMUX_LOCAL_URL") {
            slot.1 = url;
        }
    }
}

/// Whether `gateway` is an address this listener may bind: a private IPv4 address that is not
/// loopback or unspecified, and inside `subnet`.
fn bindable(gateway: Ipv4Addr, subnet: &Subnet) -> bool {
    gateway.is_private() && !gateway.is_loopback() && !gateway.is_unspecified() && subnet.contains(IpAddr::V4(gateway))
}

/// The gateway and subnet of the `agentmux-agents` network, from its IPAM config.
pub fn gateway_and_subnet(ipam: &[(Option<String>, Option<String>)]) -> Option<(Ipv4Addr, Subnet)> {
    ipam.iter().find_map(|(subnet, gateway)| {
        let subnet = Subnet::parse(subnet.as_deref()?)?;
        let gateway: Ipv4Addr = gateway.as_deref()?.trim().parse().ok()?;
        Some((gateway, subnet))
    })
}

/// Stops serving and forgets the address, so containers fall back to `host.docker.internal`
/// rather than being pointed at a gateway that is gone.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn unbind() {
    if let Some(old) = BOUND.lock().unwrap().take() {
        old.stop.cancel();
    }
}

/// Binds the listener to the agents network's gateway if the network exists and the address can
/// be bound, and returns its URL. Idempotent: the same gateway keeps its listener. Linux only;
/// anywhere else, or when the bind fails, `None` and the old route (`host.docker.internal`) stays.
pub async fn ensure_bound(docker: &bollard::Docker) -> Option<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = docker;
        None
    }
    #[cfg(target_os = "linux")]
    {
        let network = docker
            .inspect_network::<String>(crate::backend::container::AGENTMUX_DOCKER_NETWORK, None)
            .await
            .ok()?;
        let ipam: Vec<(Option<String>, Option<String>)> = network
            .ipam
            .and_then(|i| i.config)
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.subnet, c.gateway))
            .collect();
        let (gateway, subnet) = gateway_and_subnet(&ipam)?;
        if !bindable(gateway, &subnet) {
            tracing::debug!(%gateway, "container listener: the gateway is not a private address of this host; not binding");
            unbind();
            return None;
        }
        // One binder at a time, so two containers starting together bind once.
        let _binding = BINDING.lock().await;
        let unchanged = BOUND.lock().unwrap().as_ref().is_some_and(|b| b.gateway == gateway && b.subnet == subnet);
        if unchanged {
            return url();
        }
        let router = ROUTER.get()?.clone();
        let listener = match tokio::net::TcpListener::bind((gateway, 0)).await {
            Ok(l) => l,
            Err(e) => {
                // Docker Desktop and rootless Docker keep the gateway out of this host's
                // network namespace, so the address cannot be bound here.
                tracing::info!(%gateway, error = %e, "container listener: could not bind the agents network's gateway; containers keep host.docker.internal");
                unbind();
                return None;
            }
        };
        let port = listener.local_addr().ok()?.port();
        let stop = tokio_util::sync::CancellationToken::new();
        let served = stop.clone();
        tokio::spawn(async move {
            let result = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(async move { served.cancelled().await })
                .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "container listener stopped");
            }
        });
        let old = BOUND.lock().unwrap().replace(Binding { gateway, subnet, port, stop });
        if let Some(old) = old {
            old.stop.cancel();
        }
        tracing::info!(%gateway, port, "container listener: containers reach srv at the agents network's gateway");
        url()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    fn subnet() -> Subnet {
        Subnet::parse(&d("172_30_0_0/16")).unwrap()
    }

    /// Test addresses are written with `_` so no dotted private address sits in the source.
    fn d(s: &str) -> String {
        s.replace('_', ".")
    }

    fn ip(s: impl AsRef<str>) -> IpAddr {
        s.as_ref().parse().unwrap()
    }

    #[test]
    fn a_subnet_contains_its_own_addresses_only() {
        let s = subnet();
        assert!(s.contains(ip(d("172_30_0_1"))) && s.contains(ip(d("172_30_255_254"))));
        assert!(!s.contains(ip(d("172_31_0_1"))) && !s.contains(ip("127.0.0.1")) && !s.contains(ip(d("10_0_0_1"))));
        assert!(s.contains(ip(format!("::ffff:{}", d("172_30_4_5")))), "a mapped address is the IPv4 one");
        assert!(!s.contains(ip("::1")));
        assert_eq!(Subnet::parse(&d("172_30_0_0/0")), None, "a /0 would admit everyone");
        assert_eq!(Subnet::parse(&d("172_30_0_0")), None);
        assert_eq!(Subnet::parse("nonsense/16"), None);
        assert!(Subnet::parse(&d("172_30_9_9/24")).unwrap().contains(ip(d("172_30_9_200"))), "host bits are ignored");
    }

    #[test]
    fn only_a_private_gateway_inside_its_subnet_is_bound() {
        let s = subnet();
        assert!(bindable(d("172_30_0_1").parse().unwrap(), &s));
        let public = Subnet::parse(&d("8_8_0_0/16")).unwrap();
        assert!(!bindable(d("8_8_0_1").parse().unwrap(), &public), "a public address");
        let all = Subnet::parse("0.0.0.0/1").unwrap();
        assert!(!bindable("0.0.0.0".parse().unwrap(), &all), "the wildcard");
        let lo = Subnet::parse("127.0.0.0/8").unwrap();
        assert!(!bindable("127.0.0.1".parse().unwrap(), &lo), "loopback");
        assert!(!bindable(d("172_31_0_1").parse().unwrap(), &s), "outside the subnet");
    }

    #[test]
    fn the_gateway_comes_from_the_first_ipv4_ipam_entry() {
        let ipam = vec![
            (Some("fd00::/64".to_string()), Some("fd00::1".to_string())),
            (Some(d("172_30_0_0/16").to_string()), Some(d("172_30_0_1").to_string())),
        ];
        assert_eq!(gateway_and_subnet(&ipam), Some((d("172_30_0_1").parse().unwrap(), subnet())));
        assert_eq!(gateway_and_subnet(&[]), None);
        assert_eq!(gateway_and_subnet(&[(Some(d("172_30_0_0/16").into()), None)]), None);
    }

    #[test]
    fn a_request_needs_the_subnet_a_live_container_token_and_an_allowed_route() {
        let token = crate::backend::container_credential::token_for_block("cl-block-check", Some("agent-tok"));
        let s = subnet();
        let ok = |peer: &str, token: Option<&str>, path: &str| check(ip(peer), Some(&s), token, path);
        assert!(ok(&d("172_30_0_7"), Some(&token), "/agentmux/discovery").is_ok());
        assert_eq!(ok(&d("172_30_0_7"), Some(&token), "/agentmux/service").unwrap_err().0, StatusCode::FORBIDDEN);
        assert_eq!(ok(&d("172_30_0_7"), Some(&token), "/ws").unwrap_err().0, StatusCode::FORBIDDEN);
        assert_eq!(ok(&d("172_30_0_7"), None, "/agentmux/discovery").unwrap_err().0, StatusCode::UNAUTHORIZED);
        assert_eq!(ok(&d("172_30_0_7"), Some("some-instance-key"), "/agentmux/discovery").unwrap_err().0, StatusCode::UNAUTHORIZED);
        assert_eq!(ok(&d("172_30_0_7"), Some("amxc_not-minted"), "/agentmux/discovery").unwrap_err().0, StatusCode::UNAUTHORIZED);
        // The right token from the wrong place.
        assert_eq!(ok("127.0.0.1", Some(&token), "/agentmux/discovery").unwrap_err().0, StatusCode::FORBIDDEN);
        assert_eq!(ok(&d("192_168_1_9"), Some(&token), "/agentmux/discovery").unwrap_err().0, StatusCode::FORBIDDEN);
        // No listener set up: nothing is served.
        assert_eq!(
            check(ip(d("172_30_0_7")), None, Some(&token), "/agentmux/discovery").unwrap_err().0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    async fn call(router: &Router, peer: &str, key: Option<&str>, path: &str) -> StatusCode {
        let mut req = Request::builder().uri(path);
        if let Some(key) = key {
            req = req.header(agentmux_common::AUTH_KEY_HEADER, key);
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::new(ip(peer), 40000)));
        router.clone().oneshot(req).await.unwrap().status()
    }

    /// On the real router: the loopback-only operator RPCs and the instance key are refused on the
    /// container listener, whatever the source.
    #[tokio::test]
    async fn the_operator_surface_is_refused_on_the_container_listener() {
        let state = crate::server::tests::test_state();
        let instance_key = state.auth_key.clone();
        let router = layer(crate::server::build_routers(state).full, || Some(subnet()));
        let token = crate::backend::container_credential::token_for_block("cl-block-real", Some("agent-tok"));

        // The instance key is not a container token, from the right network too.
        assert_eq!(call(&router, &d("172_30_0_7"), Some(&instance_key), "/agentmux/discovery").await, StatusCode::UNAUTHORIZED);
        assert_eq!(call(&router, &d("172_30_0_7"), Some(&instance_key), "/agentmux/service").await, StatusCode::UNAUTHORIZED);
        // A container token reaches every route but the host's: commands, host files, the app's plumbing.
        for operator in ["/agentmux/service", "/ws", "/agentmux/file", "/api/v1/shell/create", "/api/v1/ptyshell/input", "/agentmux/stream-local-file", "/api/v1/host/vault", "/api/v1/agent/desktop/authorize"] {
            let got = call(&router, &d("172_30_0_7"), Some(&token), operator).await;
            assert_eq!(got, StatusCode::FORBIDDEN, "{operator}");
        }
        // An allowed route is passed to the app (whatever it then answers is not the guard's).
        let got = call(&router, &d("172_30_0_7"), Some(&token), "/agentmux/discovery").await;
        assert!(got != StatusCode::UNAUTHORIZED && got != StatusCode::FORBIDDEN, "discovery reached the app: {got}");
        // Not from loopback, not from another network.
        assert_eq!(call(&router, "127.0.0.1", Some(&token), "/agentmux/discovery").await, StatusCode::FORBIDDEN);
        assert_eq!(call(&router, &d("10_1_2_3"), Some(&token), "/agentmux/discovery").await, StatusCode::FORBIDDEN);
    }

    /// What the real HTTP parser and router make of a request line: the status the listener
    /// answers with, over a TCP connection.
    async fn raw_status(addr: SocketAddr, target: &str, key: &str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut conn = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = format!("GET {target} HTTP/1.1\r\nHost: x\r\nX-AuthKey: {key}\r\nConnection: close\r\n\r\n");
        conn.write_all(request.as_bytes()).await.unwrap();
        let mut answer = String::new();
        let _ = conn.read_to_string(&mut answer).await;
        answer.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("none").to_string()
    }

    /// Paths that look like an allowed route, or climb out of one, never reach a refused route:
    /// the denylist matches the raw path exactly as the router does, so a trick that names a
    /// refused route is refused and one that names no route gets the router's 404.
    #[tokio::test]
    async fn path_tricks_do_not_reach_an_operator_route() {
        let state = crate::server::tests::test_state();
        let router = layer(crate::server::build_routers(state).full, || Subnet::parse("127.0.0.0/8"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
        });
        let token = crate::backend::container_credential::token_for_block("cl-block-paths", Some("agent-tok"));

        let tricks = [
            "//agentmux/service",
            "//agentmux/discovery",
            "/agentmux/service/",
            "/agentmux/service.",
            "/agentmux/service/.",
            "/agentmux/service?/agentmux/discovery",
            "/agentmux/discovery/",
            "/agentmux/discovery/.",
            "/agentmux/discovery/../service",
            "/agentmux/discovery/%2e%2e/service",
            "/agentmux/discovery%2F..%2Fservice",
            "/agentmux%2Fservice",
            "/agentmux/%73ervice",
            "/AGENTMUX/service",
            "/api/v1/agent/../../agentmux/service",
            "/api/v1/agent/%2e%2e/%2e%2e/agentmux/service",
            "/api/v1/agent/%2E%2E/%2E%2E/agentmux/service",
            "/api/v1/agent/memory/list/../../../../agentmux/service",
            "/api/v1/agent/memory/list/%2e%2e/%2e%2e/%2e%2e/%2e%2e/agentmux/service",
            "/api/v1/agent/shutdown/..%2F..%2F..%2Fagentmux/service",
            "/api/v1/agent/shutdown/x/../../../../ws",
            "/agentmux/work/x/../../service",
            "/agentmux/work/%2e%2e/service",
            "/agentmux/work/a%2Fb",
            "/agentmux/work/x/heartbeat/../../../service",
            "/agentmux/work/x\\..\\service",
            "/%2e%2e/agentmux/service",
            "/ws/",
            "/ws/.",
        ];
        for target in tricks {
            // With a denylist a trick that names no route is a 404 from the router; what must never happen is
            // an answer from a route the container is refused.
            let got = raw_status(addr, target, &token).await;
            assert!(got == "403" || got == "400" || got == "404", "{target} answered {got}");
        }
        // The guard is not simply refusing everything.
        let got = raw_status(addr, "/agentmux/discovery", &token).await;
        assert!(got != "401" && got != "403" && got != "400", "discovery answered {got}");
    }

    #[test]
    fn an_environment_without_a_listener_is_left_alone() {
        let mut env = vec![("AGENTMUX_LOCAL_URL".to_string(), "http://host.docker.internal:1".to_string())];
        let before = env.clone();
        if url().is_none() {
            point_env_at_listener(&mut env);
            assert_eq!(env, before);
        }
        let mut other = vec![("OTHER".to_string(), "x".to_string())];
        point_env_at_listener(&mut other);
        assert_eq!(other, vec![("OTHER".to_string(), "x".to_string())], "no URL slot is added");
    }

    #[tokio::test]
    async fn the_layer_reads_the_subnet_at_each_request() {
        let inner = Router::new().route("/agentmux/discovery", get(|| async { "ok" }));
        let token = crate::backend::container_credential::token_for_block("cl-block-live", None);
        let up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = up.clone();
        let router = layer(inner, move || flag.load(std::sync::atomic::Ordering::SeqCst).then(subnet));
        assert_eq!(call(&router, &d("172_30_0_7"), Some(&token), "/agentmux/discovery").await, StatusCode::SERVICE_UNAVAILABLE);
        up.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(call(&router, &d("172_30_0_7"), Some(&token), "/agentmux/discovery").await, StatusCode::OK);
    }
}
