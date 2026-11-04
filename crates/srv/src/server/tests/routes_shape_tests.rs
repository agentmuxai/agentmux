//! What the assembled routers serve, checked without a key: which routes need one and which paths the
//! LAN router reaches. A route moved between the groups `build_routers_with` builds keeps its answer.

use axum::body::Body;
use axum::http::{Method, Request};
use tower::ServiceExt;

use crate::server::route_scan::registered_routes;
use crate::server::routes::{build_routers_with, SrvRouters};
use crate::server::tests::test_state;

fn sample(route: &str) -> String {
    route.split('/').map(|s| if s.starts_with(':') || s.starts_with('*') { "x" } else { s }).collect::<Vec<_>>().join("/")
}

async fn status(router: &axum::Router, method: &Method, uri: &str) -> u16 {
    let req = Request::builder().method(method.clone()).uri(uri).body(Body::empty()).unwrap();
    router.clone().oneshot(req).await.unwrap().status().as_u16()
}

/// Every route in `routes.rs`, one `(METHOD, path)` per method it names.
fn method_paths() -> Vec<(Method, String)> {
    let mut out = Vec::new();
    for route in registered_routes() {
        for (name, method) in [("get", Method::GET), ("post", Method::POST), ("put", Method::PUT), ("patch", Method::PATCH), ("delete", Method::DELETE)] {
            if route.methods.contains(&format!("{name}(")) {
                out.push((method, sample(&route.path)));
            }
        }
    }
    out.sort_by(|a, b| (a.1.as_str(), a.0.as_str()).cmp(&(b.1.as_str(), b.0.as_str())));
    out.dedup();
    out
}

/// Routes that answer without the key: the health check, the WhatsApp webhook and the widget files,
/// which authenticate themselves.
const OPEN: [&str; 5] = ["/", "/health", "/webhook/whatsapp", "/agentmux/widget-files/x/x/x/x", "/agentmux/widget-sdk/x"];

/// What the LAN router answers at all, by path: the peer routes, which want a key, and the open routes
/// that are not widget files.
const LAN: [&str; 12] = [
    "GET /",
    "GET /agentmux/agent/holding",
    "GET /agentmux/fleet",
    "GET /agentmux/fleet/events",
    "GET /agentmux/reactive/agent",
    "GET /agentmux/reactive/agent-names",
    "POST /agentmux/reactive/inject",
    "POST /agentmux/reactive/inject-files",
    "POST /agentmux/remote/op",
    "GET /health",
    "GET /webhook/whatsapp",
    "POST /webhook/whatsapp",
];

#[tokio::test]
async fn every_route_but_the_open_ones_refuses_a_request_with_no_key() {
    let SrvRouters { full, .. } = build_routers_with(test_state(), None);
    let mut open_answers = Vec::new();
    for (method, path) in method_paths() {
        let got = status(&full, &method, &path).await;
        if OPEN.contains(&path.as_str()) {
            open_answers.push(format!("{method} {path} {got}"));
        } else {
            assert_eq!(got, 401, "{method} {path} answered without a key");
        }
    }
    assert!(open_answers.len() >= OPEN.len(), "the open routes were not all found: {open_answers:?}");
}

#[tokio::test]
async fn the_lan_router_serves_only_the_peer_routes_and_the_open_ones() {
    let SrvRouters { lan, .. } = build_routers_with(test_state(), None);
    let mut reached = Vec::new();
    for (method, path) in method_paths() {
        if status(&lan, &method, &path).await != 404 {
            reached.push(format!("{method} {path}"));
        }
    }
    assert_eq!(reached, LAN);
}
