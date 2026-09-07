// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one raw-TCP `POST /agentmux/service` transport the host uses to
//! reach srv from contexts that have no async runtime — CEF lifecycle
//! callbacks, background `std::thread`s, `spawn_blocking` bodies.
//!
//! Before this module, `client/helpers.rs` carried **eleven** hand-rolled
//! copies of the same sequence — build a `{service, method, args,
//! uicontext}` JSON body, format an HTTP/1.1 request with `X-AuthKey`,
//! `connect_timeout` 2000 ms, set both timeouts, `write_all`,
//! `read_to_string` — one ~25-line block repeated per function, and the
//! largest single-file duplication cluster in the Rust workspace
//! (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §3.2).
//!
//! **This module owns the transport only.** It deliberately does *no*
//! logging: every caller keeps its own `tracing::warn!`/`error!` lines
//! byte-for-byte, because several of those messages are operator-facing
//! and cited by specs and retros (e.g. the `backend_close_window` wording
//! in `SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md`). The error
//! enum is granular so a caller can log connect vs. write vs. read
//! failures exactly as it did before — the pre-existing copies were not
//! uniform on that point (setters ignored read errors and then reported
//! the empty status line as "did not succeed"; getters returned `None`
//! silently), and that asymmetry is preserved by the callers, not
//! flattened here.
//!
//! `backend_close_window` is intentionally **not** routed through here:
//! it interleaves `dlog` tracing between the write and the read and
//! escalates to `error!`, and it is the one call gating a user-visible
//! close path. Leaving it self-contained keeps its reliability story
//! reviewable on its own.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Connect / write / read budget, each. Matches every prior copy.
const TIMEOUT: Duration = Duration::from_millis(2000);

/// Where the round-trip failed. `Read` is separate from the others on
/// purpose — see the module doc for why callers treat it differently.
#[derive(Debug)]
pub(crate) enum ServiceCallError {
    Connect(io::Error),
    Write(io::Error),
    Read(io::Error),
}

impl std::fmt::Display for ServiceCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "connect failed: {e}"),
            Self::Write(e) => write!(f, "write failed: {e}"),
            Self::Read(e) => write!(f, "read failed: {e}"),
        }
    }
}

/// The raw HTTP/1.1 response, plus the two views every caller wanted.
#[derive(Debug)]
pub(crate) struct ServiceResponse {
    pub(crate) raw: String,
}

impl ServiceResponse {
    /// The status line, or `"(empty)"` — the exact placeholder the prior
    /// copies logged when srv sent nothing back.
    pub(crate) fn first_line(&self) -> &str {
        self.raw.lines().next().unwrap_or("(empty)")
    }

    /// The same 200-check every prior copy performed.
    pub(crate) fn status_ok(&self) -> bool {
        let l = self.first_line();
        l.contains(" 200 ") || l.starts_with("HTTP/1.1 200")
    }

    /// The JSON `data` field of a `{ "success": true, "data": … }` reply,
    /// or `None` if the body is missing, unparseable, or `success` is not
    /// exactly `true`. Splits headers from body on the blank line — the
    /// request is `Connection: close` with no chunked encoding, so srv's
    /// axum reply is one JSON payload after the headers.
    pub(crate) fn data_if_success(&self) -> Option<serde_json::Value> {
        let body = self.raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
        if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
            return None;
        }
        parsed.get("data").cloned()
    }
}

/// Build the `/agentmux/service` request body. `uicontext` is always
/// `null` from the host — it has no UI context of its own to attach.
pub(crate) fn service_body(service: &str, method: &str, args: serde_json::Value) -> String {
    serde_json::json!({
        "service": service,
        "method": method,
        "args": args,
        "uicontext": null,
    })
    .to_string()
}

/// One blocking round-trip. Callers resolve `addr` first (see
/// `helpers::parse_web_endpoint`) so an unparseable endpoint is reported
/// by the caller's own message, as before.
pub(crate) fn service_call(
    addr: SocketAddr,
    auth_key: &str,
    body: &str,
) -> Result<ServiceResponse, ServiceCallError> {
    // Auth via the X-AuthKey HEADER. The legacy `?authkey=` query param
    // was deliberately disabled for HTTP routes in the 2026-05-11
    // security audit (srv test `auth_rejects_query_param_on_http_routes`;
    // only /ws still honors it).
    let request = format!(
        "POST /agentmux/service HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         X-AuthKey: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        auth_key,
        body.len(),
        body
    );

    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(ServiceCallError::Connect)?;
    stream.set_write_timeout(Some(TIMEOUT)).ok();
    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream
        .write_all(request.as_bytes())
        .map_err(ServiceCallError::Write)?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw).map_err(ServiceCallError::Read)?;
    Ok(ServiceResponse { raw })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn resp(raw: &str) -> ServiceResponse {
        ServiceResponse { raw: raw.to_string() }
    }

    #[test]
    fn service_body_has_the_wire_shape_with_null_uicontext() {
        let b = service_body("window", "GetWindow", serde_json::json!(["w1"]));
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["service"], "window");
        assert_eq!(v["method"], "GetWindow");
        assert_eq!(v["args"], serde_json::json!(["w1"]));
        assert!(v["uicontext"].is_null());
    }

    #[test]
    fn status_ok_matches_the_prior_copies_check() {
        assert!(resp("HTTP/1.1 200 OK\r\n\r\n{}").status_ok());
        assert!(resp("HTTP/1.0 200 OK\r\n").status_ok(), "' 200 ' anywhere in the line");
        assert!(!resp("HTTP/1.1 401 Unauthorized\r\n").status_ok());
        assert!(!resp("").status_ok());
        assert_eq!(resp("").first_line(), "(empty)", "the placeholder every prior copy logged");
    }

    #[test]
    fn data_if_success_requires_success_true_and_a_body() {
        let ok = resp("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"success\":true,\"data\":{\"opacity\":0.5}}");
        assert_eq!(ok.data_if_success().unwrap()["opacity"], 0.5);

        let failed = resp("HTTP/1.1 200 OK\r\n\r\n{\"success\":false,\"data\":{\"opacity\":0.5}}");
        assert!(failed.data_if_success().is_none());

        let no_blank_line = resp("HTTP/1.1 200 OK\r\n{\"success\":true}");
        assert!(no_blank_line.data_if_success().is_none(), "no header/body split → no body");

        let garbage = resp("HTTP/1.1 200 OK\r\n\r\nnot json");
        assert!(garbage.data_if_success().is_none());
    }

    /// End-to-end against a real socket: the request carries the auth
    /// header and the exact body, and the canned reply comes back intact.
    /// The eleven prior copies had no transport test at all.
    #[test]
    fn service_call_round_trips_over_a_real_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let reply = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"success\":true,\"data\":{\"ok\":1}}";
            s.write_all(reply.as_bytes()).unwrap();
            req
        });

        let body = service_body("client", "GetClientData", serde_json::json!([]));
        let r = service_call(addr, "sekrit-key", &body).unwrap();
        let req = server.join().unwrap();

        assert!(req.starts_with("POST /agentmux/service HTTP/1.1\r\n"), "{req}");
        assert!(req.contains("X-AuthKey: sekrit-key\r\n"), "{req}");
        assert!(req.contains(&format!("Content-Length: {}\r\n", body.len())), "{req}");
        assert!(req.ends_with(&body), "body must be the last thing on the wire: {req}");
        assert!(r.status_ok());
        assert_eq!(r.data_if_success().unwrap()["ok"], 1);
    }

    #[test]
    fn service_call_reports_connect_failure_distinctly() {
        // A port nobody is listening on. Bind-then-drop to find one.
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        match service_call(addr, "k", "{}") {
            Err(ServiceCallError::Connect(_)) => {}
            other => panic!("expected Connect error, got {other:?}"),
        }
    }
}
