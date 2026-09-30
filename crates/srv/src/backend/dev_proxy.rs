// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Native reverse proxy for container-agent dev servers.
//!
//! Routes `http://<project>-<agent_id>.localhost:{DEV_PROXY_PORT}/...` to
//! whatever internal `ip:port` that agent's dev server registered via the
//! `RegisterDevServer` MCP tool (the HTTP endpoint backing that tool lives
//! in `agentmux-srv/src/server/app_api/dev_server.rs`; this module owns the
//! routing table and the proxy server itself). See
//! `docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md`.
//!
//! Deliberately NOT a general-purpose reverse proxy (spec §4): Host-header
//! → backend address is the entire feature. No path routing, no TLS, no
//! auth, no rate limiting, no websocket upgrade handling.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use futures_util::TryStreamExt;
use tokio::sync::RwLock;

/// Fixed local port the proxy listens on. Chosen for this PR — nothing
/// else in `agentmux-srv` binds it (checked: no other `TcpListener::bind`
/// or config default references 8090 anywhere in this crate). See spec §5.
pub const DEV_PROXY_PORT: u16 = 8090;

/// One registered dev server: where to actually send the request, and
/// which agent owns the registration (needed so `unregister_agent` can
/// remove exactly this agent's entries — see its doc comment for why a
/// plain string-suffix match on the routing key isn't safe).
#[derive(Debug, Clone)]
struct Route {
    addr: SocketAddr,
    agent_id: String,
}

/// `"<project>-<agent_id>"` → the container's internal `ip:port` its dev
/// server is actually listening on. Cleared per-agent when that agent's
/// container stops (`ContainerManager::stop`/`remove` in `container.rs`
/// call `unregister_agent` — see that module's `cleanup_dev_proxy_routes`).
///
/// Cheap to clone (one `Arc` inside) — held directly by `AppState` and
/// passed to the proxy's own axum router as state.
#[derive(Clone, Default)]
pub struct DevProxyRegistry {
    routes: Arc<RwLock<HashMap<String, Route>>>,
}

impl DevProxyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The exact routing key both `register` and the Host-header lookup
    /// use — kept as one function so the two can never drift apart.
    /// Lowercased: hostnames are case-insensitive, and a browser may send
    /// whatever case the URL bar has, so registration and lookup must
    /// agree regardless of how either side capitalized `project`/`agent_id`.
    pub fn routing_key(project: &str, agent_id: &str) -> String {
        format!("{}-{}", project.to_lowercase(), agent_id.to_lowercase())
    }

    pub async fn register(&self, project: &str, agent_id: &str, addr: SocketAddr) {
        let key = Self::routing_key(project, agent_id);
        self.routes.write().await.insert(
            key,
            Route {
                addr,
                agent_id: agent_id.to_lowercase(),
            },
        );
    }

    /// Removes every registration belonging to `agent_id`, regardless of
    /// project name — called when that agent's container stops so a dead
    /// container never keeps serving (stale) proxy traffic.
    ///
    /// Matches on the stored `agent_id` field, NOT a `key.ends_with(...)`
    /// string check — agent ids can themselves contain hyphens (e.g. a
    /// named identity like `korp-asaf`), so a suffix match on the combined
    /// `"<project>-<agent_id>"` key could wrongly match a different
    /// agent's registration whose id happens to end in the same substring.
    pub async fn unregister_agent(&self, agent_id: &str) {
        let agent_id = agent_id.to_lowercase();
        self.routes
            .write()
            .await
            .retain(|_, route| route.agent_id != agent_id);
    }

    pub async fn lookup(&self, key: &str) -> Option<SocketAddr> {
        self.routes.read().await.get(key).map(|r| r.addr)
    }

    #[cfg(test)]
    async fn len(&self) -> usize {
        self.routes.read().await.len()
    }
}

/// Host header → routing key. `"pulse-korp.localhost:8090"` and
/// `"pulse-korp.localhost"` both become `"pulse-korp"`. A Host that isn't
/// `.localhost` is still accepted (minus port, lowercased) rather than
/// rejected outright — an empty/missing Host is the only thing that
/// produces `None`. Whether the key actually resolves to anything is the
/// registry lookup's job, not this function's; either way the caller gets
/// a clear "no dev server registered for `<key>`" 404, never a confusing
/// hostname-format error.
pub fn routing_key_from_host(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    // Lowercase BEFORE stripping the `.localhost` suffix, not after — a
    // literal `.strip_suffix(".localhost")` is case-sensitive, so an
    // upper/mixed-case Host (e.g. from a client that title-cases URLs)
    // would otherwise fail to match and fall through with the suffix
    // still attached.
    let without_port = host.split(':').next().unwrap_or(host).to_lowercase();
    let key = without_port.strip_suffix(".localhost").unwrap_or(&without_port);
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

#[derive(Clone)]
struct ProxyState {
    registry: DevProxyRegistry,
    client: reqwest::Client,
}

/// Builds the proxy's axum router. Split out from [`serve`] so tests can
/// drive it directly (via a real bound listener) without needing the fixed
/// [`DEV_PROXY_PORT`] to be free.
pub fn router(registry: DevProxyRegistry) -> Router {
    let client = reqwest::Client::builder()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let state = ProxyState { registry, client };
    // `.fallback` (not a path route): routing here is entirely by Host
    // header, so every path/method for every host is the same handler.
    Router::new()
        .fallback(any(proxy_handler))
        .with_state(state)
}

/// Binds [`DEV_PROXY_PORT`] on loopback and serves the proxy until the
/// process exits. Spawned once at srv startup (`main.rs`). A bind failure
/// (e.g. the port is already in use on this machine) is logged and the
/// proxy is simply unavailable for this run — never fatal to srv startup;
/// dev-server routing is an additive convenience, not core functionality.
pub async fn serve(registry: DevProxyRegistry) {
    let addr = SocketAddr::from(([127, 0, 0, 1], DEV_PROXY_PORT));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(
                port = DEV_PROXY_PORT,
                error = %e,
                "dev-proxy: failed to bind — dev-server routing unavailable this run"
            );
            return;
        }
    };
    tracing::info!(port = DEV_PROXY_PORT, "dev-proxy: listening");
    if let Err(e) = axum::serve(listener, router(registry)).await {
        tracing::error!(error = %e, "dev-proxy: server error");
    }
}

fn not_found_response(key: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        format!(
            "AgentMux dev proxy: no dev server registered for \"{key}\". \
             Call RegisterDevServer from the owning agent once its dev server is listening."
        ),
    )
        .into_response()
}

/// The one handler this proxy has. Reads `Host`, looks up the backend,
/// forwards the request (method/headers/body, streamed both ways so a
/// large response body — or a long-lived one — isn't buffered whole in
/// memory), and streams the response back unchanged.
async fn proxy_handler(State(state): State<ProxyState>, req: Request) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let Some(key) = routing_key_from_host(&host) else {
        return not_found_response(&host);
    };

    let Some(addr) = state.registry.lookup(&key).await else {
        return not_found_response(&key);
    };

    let (parts, body) = req.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target_url = format!("http://{addr}{path_and_query}");

    let req_stream = body.into_data_stream().map_err(std::io::Error::other);
    let outbound_body = reqwest::Body::wrap_stream(req_stream);

    let mut builder = state.client.request(parts.method.clone(), &target_url);
    for (name, value) in parts.headers.iter() {
        // Same hop-by-hop exclusion as the response path below, applied to
        // the request side: `Host` describes a connection to THIS proxy, not
        // the backend, and `Transfer-Encoding`/`Connection` describe the
        // client's connection to us. Forwarding `Transfer-Encoding: chunked`
        // verbatim alongside the independently re-streamed
        // `reqwest::Body::wrap_stream` body would assert a chunked encoding
        // reqwest is not actually producing — a conflicting signal that can
        // break the forwarded request. reagent P1, PR #3439.
        if name == header::HOST
            || name == header::TRANSFER_ENCODING
            || name == header::CONNECTION
        {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }
    let builder = builder.body(outbound_body);

    match builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let resp_stream = resp.bytes_stream().map_err(std::io::Error::other);
            let mut out = Response::builder().status(status);
            for (name, value) in headers.iter() {
                // Hop-by-hop headers describing reqwest's own connection to
                // the backend — passing them through would describe a
                // connection that doesn't exist between the real client
                // and this proxy (e.g. a stale Content-Length next to a
                // re-chunked stream).
                if name == header::TRANSFER_ENCODING || name == header::CONNECTION {
                    continue;
                }
                out = out.header(name.clone(), value.clone());
            }
            out.body(Body::from_stream(resp_stream))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => {
            tracing::warn!(
                key = %key,
                target = %target_url,
                error = %e,
                "dev-proxy: upstream request failed"
            );
            (
                StatusCode::BAD_GATEWAY,
                format!("AgentMux dev proxy: could not reach dev server for \"{key}\": {e}"),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn routing_key_is_project_dash_agent_lowercased() {
        assert_eq!(DevProxyRegistry::routing_key("Pulse", "Korp"), "pulse-korp");
        assert_eq!(DevProxyRegistry::routing_key("api", "agent1"), "api-agent1");
    }

    #[test]
    fn host_parsing_strips_port_and_localhost_suffix() {
        assert_eq!(
            routing_key_from_host("pulse-korp.localhost:8090"),
            Some("pulse-korp".to_string())
        );
        assert_eq!(
            routing_key_from_host("pulse-korp.localhost"),
            Some("pulse-korp".to_string())
        );
        assert_eq!(
            routing_key_from_host("Pulse-Korp.LOCALHOST:8090"),
            Some("pulse-korp".to_string())
        );
    }

    #[test]
    fn host_parsing_passes_through_a_non_localhost_host_minus_port() {
        // Not rejected outright — the registry lookup (empty for anything
        // never registered) is what actually 404s an unrecognized host,
        // not this parsing step.
        assert_eq!(
            routing_key_from_host("example.com:8090"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn host_parsing_rejects_only_empty_host() {
        assert_eq!(routing_key_from_host(""), None);
        assert_eq!(routing_key_from_host("   "), None);
    }

    #[tokio::test]
    async fn register_then_lookup_finds_the_backend() {
        let registry = DevProxyRegistry::new();
        registry.register("pulse", "korp", addr(4000)).await;
        assert_eq!(
            registry
                .lookup(&DevProxyRegistry::routing_key("pulse", "korp"))
                .await,
            Some(addr(4000))
        );
    }

    #[tokio::test]
    async fn lookup_of_an_unregistered_key_is_none() {
        let registry = DevProxyRegistry::new();
        assert_eq!(registry.lookup("nope-nobody").await, None);
    }

    #[tokio::test]
    async fn unregister_agent_removes_only_that_agents_routes() {
        let registry = DevProxyRegistry::new();
        registry.register("pulse", "korp", addr(4000)).await;
        registry.register("stratum", "korp", addr(4001)).await;
        registry.register("app", "narko", addr(4002)).await;
        assert_eq!(registry.len().await, 3);

        registry.unregister_agent("korp").await;

        assert_eq!(registry.len().await, 1);
        assert_eq!(
            registry
                .lookup(&DevProxyRegistry::routing_key("pulse", "korp"))
                .await,
            None
        );
        assert_eq!(
            registry
                .lookup(&DevProxyRegistry::routing_key("app", "narko"))
                .await,
            Some(addr(4002))
        );
    }

    /// The exact scenario the doc comment on `unregister_agent` warns
    /// about: an agent id that itself contains a hyphen and is a suffix of
    /// a DIFFERENT agent's id must not be swept up by a naive
    /// `key.ends_with("-<agent_id>")` check.
    #[tokio::test]
    async fn unregister_agent_is_exact_not_a_suffix_match() {
        let registry = DevProxyRegistry::new();
        registry.register("proj", "x-lib", addr(4000)).await; // key: "proj-x-lib"
        registry.register("proj", "lib", addr(4001)).await; // key: "proj-lib"

        registry.unregister_agent("lib").await;

        assert_eq!(
            registry
                .lookup(&DevProxyRegistry::routing_key("proj", "x-lib"))
                .await,
            Some(addr(4000)),
            "a different agent (\"x-lib\") whose id merely ends with \"lib\" must survive"
        );
        assert_eq!(
            registry
                .lookup(&DevProxyRegistry::routing_key("proj", "lib"))
                .await,
            None
        );
    }

    #[tokio::test]
    async fn re_registering_the_same_key_overwrites_the_address() {
        let registry = DevProxyRegistry::new();
        registry.register("pulse", "korp", addr(4000)).await;
        registry.register("pulse", "korp", addr(5000)).await;
        assert_eq!(
            registry
                .lookup(&DevProxyRegistry::routing_key("pulse", "korp"))
                .await,
            Some(addr(5000))
        );
    }

    /// End-to-end through real HTTP: a tiny backend server, the proxy
    /// router in front of it, and a real client request carrying a Host
    /// header — not an in-process function call. Confirms the whole
    /// request path (Host parsing → lookup → forward → stream response
    /// back) actually works, and that an unregistered Host 404s cleanly
    /// instead of hanging or producing a confusing proxy error.
    #[tokio::test]
    async fn proxies_a_real_request_and_404s_an_unregistered_host() {
        // Fake dev server: echoes back a fixed, recognizable body.
        let backend_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake backend");
        let backend_addr = backend_listener.local_addr().unwrap();
        let backend_router = Router::new().fallback(any(|| async {
            (StatusCode::OK, "hello from the fake dev server")
        }));
        tokio::spawn(async move {
            let _ = axum::serve(backend_listener, backend_router).await;
        });

        let registry = DevProxyRegistry::new();
        registry.register("pulse", "korp", backend_addr).await;

        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(proxy_listener, router(registry)).await;
        });

        let client = reqwest::Client::new();

        // Correct Host — should reach the fake backend.
        let ok_resp = client
            .get(format!("http://{proxy_addr}/anything"))
            .header(header::HOST, "pulse-korp.localhost:8090")
            .send()
            .await
            .expect("request through proxy");
        assert_eq!(ok_resp.status(), StatusCode::OK);
        let body = ok_resp.text().await.expect("body");
        assert_eq!(body, "hello from the fake dev server");

        // Wrong/unregistered Host — clean 404, not a hang or a raw
        // connection-refused error.
        let missing_resp = client
            .get(format!("http://{proxy_addr}/anything"))
            .header(header::HOST, "nobody-here.localhost:8090")
            .send()
            .await
            .expect("request through proxy");
        assert_eq!(missing_resp.status(), StatusCode::NOT_FOUND);
        let missing_body = missing_resp.text().await.expect("body");
        assert!(missing_body.contains("no dev server registered"));
    }

    /// reagent P1, PR #3439: hop-by-hop headers describing the CLIENT's
    /// connection to THIS proxy (`Transfer-Encoding`, `Connection`) must not
    /// be forwarded to the backend — the outbound body is independently
    /// re-streamed via `reqwest::Body::wrap_stream`, so a forwarded
    /// `Transfer-Encoding: chunked` would assert an encoding reqwest isn't
    /// actually producing. Confirmed by having the fake backend report back
    /// exactly which headers it received, not by inspecting the proxy's
    /// outbound request in isolation — the same "real HTTP, not an
    /// in-process function call" standard as the test above.
    #[tokio::test]
    async fn hop_by_hop_request_headers_are_not_forwarded_to_the_backend() {
        let backend_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake backend");
        let backend_addr = backend_listener.local_addr().unwrap();
        let backend_router = Router::new().fallback(any(
            |req: Request| async move {
                let saw_transfer_encoding =
                    req.headers().contains_key(header::TRANSFER_ENCODING);
                let saw_connection = req.headers().contains_key(header::CONNECTION);
                (
                    StatusCode::OK,
                    format!("te={saw_transfer_encoding} conn={saw_connection}"),
                )
            },
        ));
        tokio::spawn(async move {
            let _ = axum::serve(backend_listener, backend_router).await;
        });

        let registry = DevProxyRegistry::new();
        registry.register("pulse", "korp", backend_addr).await;

        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(proxy_listener, router(registry)).await;
        });

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{proxy_addr}/anything"))
            .header(header::HOST, "pulse-korp.localhost:8090")
            .header(header::TRANSFER_ENCODING, "chunked")
            .header(header::CONNECTION, "keep-alive")
            .send()
            .await
            .expect("request through proxy");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.text().await.expect("body");
        assert_eq!(body, "te=false conn=false");
    }
}
