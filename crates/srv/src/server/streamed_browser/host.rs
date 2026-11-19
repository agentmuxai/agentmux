// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-srv --streamed-browser-host`: the streamed browser as a server of its own. It runs in a task made for one
//! person's browser and serves nothing else: the pane's WebSocket and `/health`. No database, no agents, no auth key,
//! none of srv's other routes.
//!
//! A connection must carry a lease token (`x-agentmux-lease`) signed for this lease and workspace, checked against the
//! public keys the host was given (`lease_token`). A token opens one connection; a second connection while one is open is
//! refused. The page, the stream and the input are the same code as the headless pane's.
//!
//! Settings: `--browser-host-listen <address:port>` (default `0.0.0.0:8291`), and the environment
//! `AGENTMUX_BROWSER_HOST_PUBLIC_KEYS` (JSON, key id to base64 SPKI), `AGENTMUX_BROWSER_HOST_LEASE` and
//! `AGENTMUX_BROWSER_HOST_WORKSPACE`. The profile is a fresh folder under the temp directory.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Query, State, WebSocketUpgrade};
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use super::lease_token::{parse_public_keys, LeaseGate};
use super::{start_from, Params, ROUTE};

/// The flag that starts srv in this mode.
pub const FLAG: &str = "--streamed-browser-host";
const LISTEN_FLAG: &str = "--browser-host-listen";
const DEFAULT_LISTEN: &str = "0.0.0.0:8291";
const KEYS_ENV: &str = "AGENTMUX_BROWSER_HOST_PUBLIC_KEYS";
const LEASE_ENV: &str = "AGENTMUX_BROWSER_HOST_LEASE";
const WORKSPACE_ENV: &str = "AGENTMUX_BROWSER_HOST_WORKSPACE";
/// The header the lease token arrives in.
pub const TOKEN_HEADER: &str = "x-agentmux-lease";

/// What the server holds: the gate, and whether a connection is open.
struct Host {
    gate: LeaseGate,
    open: AtomicBool,
}

/// Was srv started as the browser host?
pub(crate) fn requested() -> bool {
    std::env::args_os().any(|a| a == FLAG)
}

/// Runs the browser host and exits when srv was started as one; returns when it was not.
pub(crate) async fn exit_if_requested() {
    if !requested() {
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    match run(&args).await {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("agentmux-srv {FLAG}: {e}");
            std::process::exit(1);
        }
    }
}

/// The address to listen on: the flag's value, else the default.
fn listen_from(args: &[String]) -> Result<SocketAddr, String> {
    let value = args.iter().position(|a| a == LISTEN_FLAG).map(|i| args.get(i + 1).ok_or(format!("{LISTEN_FLAG} needs an address")));
    let text = match value {
        Some(v) => v?.as_str(),
        None => DEFAULT_LISTEN,
    };
    text.parse().map_err(|e| format!("{LISTEN_FLAG} {text}: {e}"))
}

fn host_from(env: impl Fn(&str) -> Option<String>) -> Result<Host, String> {
    let need = |name: &str| env(name).filter(|v| !v.trim().is_empty()).ok_or(format!("{name} is not set"));
    let keys = parse_public_keys(&need(KEYS_ENV)?)?;
    Ok(Host { gate: LeaseGate::new(keys, need(LEASE_ENV)?, need(WORKSPACE_ENV)?), open: AtomicBool::new(false) })
}

fn router(host: Arc<Host>) -> Router {
    Router::new().route("/health", get(|| async { "ok" })).route(ROUTE, get(connect)).with_state(host)
}

async fn run(args: &[String]) -> Result<(), String> {
    let listen = listen_from(args)?;
    let host = Arc::new(host_from(|n| std::env::var(n).ok())?);
    let profile = std::env::temp_dir().join(format!("agentmux-browser-{}", std::process::id()));
    std::fs::create_dir_all(&profile).map_err(|e| format!("cannot make the browser's profile folder: {e}"))?;
    super::chromium::use_profile_root(profile);
    super::address_policy::note_ports_from_env();
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("cannot listen on {listen}: {e}"))?;
    if let Ok(addr) = listener.local_addr() {
        // A page in the browser may never connect to this server's own port.
        super::address_policy::note_own_port(addr.port());
    }
    eprintln!("agentmux-srv {FLAG}: listening on {listen}");
    axum::serve(listener, router(host)).with_graceful_shutdown(shutdown()).await.map_err(|e| e.to_string())
}

/// SIGTERM (a task being stopped) and SIGINT end the server; as a container's first process it has no default for them.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            return std::future::pending().await;
        };
        tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Releases the one open connection when the pane ends, however it ends.
struct Open(Arc<Host>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.open.store(false, Ordering::SeqCst);
    }
}

/// `GET /agentmux/streamed-browser?url=&w=&h=` with a lease token, upgraded to a WebSocket.
async fn connect(
    State(host): State<Arc<Host>>,
    headers: HeaderMap,
    Query(params): Query<Params>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some(token) = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()) else {
        return (StatusCode::UNAUTHORIZED, "a lease token is needed").into_response();
    };
    let Ok(ws) = ws else {
        return (StatusCode::UPGRADE_REQUIRED, "this route is a WebSocket").into_response();
    };
    let start = match start_from(&params) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    // Checked last of the refusals that cost nothing, so a refused connection never uses up a token.
    if host.open.swap(true, Ordering::SeqCst) {
        return (StatusCode::CONFLICT, "this browser is already open").into_response();
    }
    let open = Open(host.clone());
    if let Err(why) = host.gate.accept(token, now()) {
        tracing::warn!(?why, "browser host: a lease token was refused");
        return (StatusCode::FORBIDDEN, "the lease token was refused").into_response();
    }
    super::upgrade(ws, start, async move {
        let _open = open;
        std::future::pending::<()>().await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_listens_where_it_is_told_and_otherwise_on_the_default() {
        assert_eq!(listen_from(&args(&["srv", FLAG])).unwrap().port(), 8291);
        assert_eq!(listen_from(&args(&["srv", FLAG, LISTEN_FLAG, "10.0.0.5:9000"])).unwrap().to_string(), "10.0.0.5:9000");
        assert!(listen_from(&args(&["srv", FLAG, LISTEN_FLAG])).is_err());
        assert!(listen_from(&args(&["srv", FLAG, LISTEN_FLAG, "nonsense"])).is_err());
    }

    #[tokio::test]
    async fn it_serves_health_and_the_pane_route_and_nothing_else() {
        let keys = r#"{"k1":"MCowBQYDK2VwAyEA6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw="}"#;
        let host = Arc::new(host_from(|n| Some(if n == KEYS_ENV { keys } else { "x1234567890a" }.to_string())).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(host)).await.unwrap() });
        let get = |path: &str, token: Option<&str>| {
            let mut req = reqwest::Client::new().get(format!("{base}{path}"));
            if let Some(t) = token {
                req = req.header(TOKEN_HEADER, t);
            }
            async move { req.send().await.unwrap().status() }
        };
        assert_eq!(get("/health", None).await, StatusCode::OK);
        assert_eq!(get(ROUTE, None).await, StatusCode::UNAUTHORIZED, "no token, no pane");
        assert_eq!(get(ROUTE, Some("lease1")).await, StatusCode::UPGRADE_REQUIRED, "a plain request is told to upgrade");
        for other in ["/", "/ws", "/agentmux/streamed-browser/x", "/api/anything"] {
            assert_eq!(get(other, None).await, StatusCode::NOT_FOUND, "{other}");
        }
    }

    #[test]
    fn it_does_not_start_without_its_keys_lease_and_workspace() {
        let keys = r#"{"k1":"MCowBQYDK2VwAyEA6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw="}"#;
        let env = |missing: &'static str| {
            move |n: &str| match n {
                _ if n == missing => None,
                KEYS_ENV => Some(keys.to_string()),
                LEASE_ENV => Some("lse-abc123".to_string()),
                WORKSPACE_ENV => Some("abcdefghijkl".to_string()),
                _ => None,
            }
        };
        assert!(host_from(env("")).is_ok());
        for missing in [KEYS_ENV, LEASE_ENV, WORKSPACE_ENV] {
            let why = host_from(env(missing)).err().unwrap();
            assert!(why.contains(missing), "{why}");
        }
    }
}
