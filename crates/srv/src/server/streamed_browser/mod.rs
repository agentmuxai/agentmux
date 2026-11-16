// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Browser pane for the web UI of a headless srv: a Chromium on the server, its page streamed to the
//! pane as JPEG frames over a WebSocket, input sent back. Opt-in (`AGENTMUX_STREAMED_BROWSER=1`,
//! headless only); see docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md for the conditions it runs
//! under (our own hosts, behind the sign-in, its own profile, `--no-sandbox` for the demo only).
//!
//! The route is behind the same layers as `/ws`: the key, or a signed-in browser's cookie and token.

#[cfg_attr(not(unix), allow(dead_code))]
mod cdp;
#[cfg_attr(not(unix), allow(dead_code))]
mod chromium;
mod nav;
#[cfg_attr(not(unix), allow(dead_code))]
mod pane;
mod protocol;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::{Extension, Query, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::local_session::SessionId;
use super::local_session_routes::{session_ended, WS_PROTOCOL};

/// The route (the same literal is registered in `routes.rs`; a test holds them together).
pub const ROUTE: &str = "/agentmux/streamed-browser";
/// `1` in the environment turns the streamed browser on.
pub const ENABLE_ENV: &str = "AGENTMUX_STREAMED_BROWSER";
/// Is the streamed browser on? Only a headless srv, only when asked, only on Unix.
pub fn enabled() -> bool {
    cfg!(unix) && crate::headless::active() && enabled_by(std::env::var(ENABLE_ENV).ok().as_deref())
}

fn enabled_by(value: Option<&str>) -> bool {
    value == Some("1")
}

#[derive(Deserialize)]
pub(in crate::server) struct Params {
    url: Option<String>,
    w: Option<u32>,
    h: Option<u32>,
}

/// The pane's start: a web address (or none) and a size within the limits.
fn start_from(p: &Params) -> Result<pane::Start, String> {
    let (w, h) = (p.w.unwrap_or(1024), p.h.unwrap_or(640));
    let range = protocol::MIN_SIDE..=protocol::MAX_SIDE;
    if !range.contains(&w) || !range.contains(&h) {
        return Err(format!("the pane's size must be between {} and {}", protocol::MIN_SIDE, protocol::MAX_SIDE));
    }
    let url = match p.url.as_deref().filter(|u| !u.trim().is_empty()) {
        Some(u) => protocol::check_url(u)?,
        None => "about:blank".to_string(),
    };
    Ok(pane::Start { url, w, h })
}

/// `GET /agentmux/streamed-browser?url=&w=&h=`, upgraded to a WebSocket.
pub(in crate::server) async fn handle(
    session: Option<Extension<SessionId>>,
    Query(params): Query<Params>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    // The route table is static: the opt-in is answered here, so the route is always registered.
    if !enabled() {
        return (StatusCode::NOT_FOUND, "the streamed browser is not on in this server").into_response();
    }
    // A plain request (the web UI asking whether the pane exists here) is told to upgrade: 404 is "off", this is "on".
    let Ok(ws) = ws else {
        return (StatusCode::UPGRADE_REQUIRED, "this route is a WebSocket").into_response();
    };
    let start = match start_from(&params) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let session = session.map(|Extension(s)| s);
    ws.protocols([WS_PROTOCOL]).on_upgrade(move |mut socket| async move {
        match chromium::lease().await {
            Ok(lease) => pane::run(socket, lease, start, session_ended(session)).await,
            Err(e) => {
                tracing::warn!(error = %e, "streamed browser: no browser for a pane");
                let _ = socket.send(axum::extract::ws::Message::Text(serde_json::json!({ "t": "error", "message": e }).to_string().into())).await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_on_only_when_asked_with_a_one() {
        assert!(enabled_by(Some("1")));
        for off in [None, Some("0"), Some("true"), Some("")] {
            assert!(!enabled_by(off));
        }
        // A server that is not headless never has it, whatever its environment says.
        assert!(!enabled());
    }

    #[test]
    fn a_pane_starts_with_a_web_address_or_a_blank_page_and_a_size_in_range() {
        let p = |url: Option<&str>, w, h| Params { url: url.map(str::to_string), w, h };
        let s = start_from(&p(None, None, None)).unwrap();
        assert_eq!((s.url.as_str(), s.w, s.h), ("about:blank", 1024, 640));
        assert_eq!(start_from(&p(Some("https://example.com"), Some(800), Some(600))).unwrap().url, "https://example.com/");
        assert!(start_from(&p(Some("file:///etc/passwd"), None, None)).is_err());
        assert!(start_from(&p(None, Some(50), None)).is_err());
        assert!(start_from(&p(None, None, Some(5000))).is_err());
    }
}
