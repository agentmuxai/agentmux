// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Small helper functions extracted from client/mod.rs in
//! task #182 PR-G for navigability.

/// Quote a string as a JavaScript string literal — escape backslashes,
/// quotes, and newlines so it's safe to embed inside `<script>` via
/// `format!`. Used by the recovery page to inject the app URL for the
/// Reload button's navigation target.
use super::dlog;

pub(super) fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"), // defense against </script> injection
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Minimal HTML escape for the recovery page. Only the characters that
/// would break the `format!`-templated string need attention; the input
/// (CEF status enum + cef-provided error string) is trusted but may
/// contain `&` / `<` / `>` in some failure modes.
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Synchronously tell the backend to close a window's workspace/tabs/shells.
///
/// Uses a raw TCP connection so no async runtime or extra crate is needed.
/// Called from a background thread in `on_before_close` so the CEF UI thread
/// is not blocked. Fire-and-forget: we write the request and don't read the response.
pub(super) fn backend_close_window(web_endpoint: &str, auth_key: &str, window_id: &str) {
    use std::io::Write;

    // Parse host:port from "http://127.0.0.1:PORT"
    let addr_str = web_endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let addr: std::net::SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[backend_close_window] cannot parse endpoint '{}': {}", web_endpoint, e);
            return;
        }
    };

    let body = serde_json::json!({
        "service": "window",
        "method": "CloseWindow",
        "args": [window_id],
        "uicontext": null,
    }).to_string();
    let request = format!(
        "POST /agentmux/service?service=window&method=CloseWindow&authkey={} HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        auth_key, body.len(), body
    );

    dlog(&format!("backend_close_window: connecting to {} for window_id={}", addr, window_id));
    let timeout = std::time::Duration::from_millis(2000);
    match std::net::TcpStream::connect_timeout(&addr, timeout) {
        Ok(mut stream) => {
            stream.set_write_timeout(Some(timeout)).ok();
            stream.set_read_timeout(Some(timeout)).ok();
            match stream.write_all(request.as_bytes()) {
                Ok(_) => {
                    dlog(&format!("backend_close_window: sent request for window_id={}", window_id));
                    // Read response to confirm the backend received it
                    use std::io::Read;
                    let mut resp = String::new();
                    let _ = stream.read_to_string(&mut resp);
                    let first_line = resp.lines().next().unwrap_or("(empty)").to_string();
                    dlog(&format!("backend_close_window: response first line: {}", first_line));
                }
                Err(e) => dlog(&format!("backend_close_window: write failed: {}", e)),
            }
        }
        Err(e) => dlog(&format!("backend_close_window: connect failed to {}: {}", addr, e)),
    }
}
