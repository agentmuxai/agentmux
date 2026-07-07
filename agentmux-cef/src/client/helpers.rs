// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Small helper functions extracted from client/mod.rs in
//! task #182 PR-G for navigability.

/// Quote a string as a JavaScript string literal — escape backslashes,
/// quotes, and newlines so it's safe to embed inside `<script>` via
/// `format!`. Used by the recovery page to inject the app URL for the
/// Reload button's navigation target.
use super::dlog;

pub(crate) fn js_string_literal(s: &str) -> String {
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
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Tell the backend to close a window's workspace/tabs/shells.
///
/// Uses a raw TCP connection so no async runtime or extra crate is needed.
/// Called from a background thread in `on_before_close` so the CEF UI thread
/// is not blocked. No longer fire-and-forget as of
/// docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md: the
/// response is read and a non-200 status, write failure, or connect
/// failure is logged via `tracing::error!` — still asynchronous from the
/// caller's perspective (this whole function runs off the UI thread), but
/// failures are no longer silently swallowed.
pub(crate) fn backend_close_window(web_endpoint: &str, auth_key: &str, window_id: &str) {
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
    // Auth via the X-AuthKey HEADER. The legacy `?authkey=` query param was
    // deliberately disabled for HTTP routes in the 2026-05-11 security audit
    // (C3 — see srv test `auth_rejects_query_param_on_http_routes`; only /ws
    // still honors it), which silently broke this request with a 401 ever
    // since — unnoticed because `on_before_close` (the only caller) never
    // fires for parked pool-window browsers, and pre-#1965 the response was
    // never read. First observed live via the round-6 demote path
    // (retro-window-lifecycle-leak-2026-07-04).
    let request = format!(
        "POST /agentmux/service HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         X-AuthKey: {}\r\n\
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
                    // Read the response so a failed/rejected CloseWindow call
                    // is actually visible instead of silently dropped — this
                    // was previously "fire-and-forget: we write the request
                    // and don't read the response" (see
                    // docs/retro/retro-window-lifecycle-leak-2026-07-04.md,
                    // docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md
                    // §2.2). Not fully synchronous from the caller's
                    // perspective — this still runs on its own background
                    // thread — but the outcome is no longer swallowed.
                    use std::io::Read;
                    let mut resp = String::new();
                    let _ = stream.read_to_string(&mut resp);
                    let first_line = resp.lines().next().unwrap_or("(empty)").to_string();
                    dlog(&format!("backend_close_window: response first line: {}", first_line));
                    if !first_line.contains(" 200 ") && !first_line.starts_with("HTTP/1.1 200") {
                        tracing::error!(
                            window_id = %window_id,
                            response = %first_line,
                            "[backend_close_window] CloseWindow request did not succeed — \
                             window_id will remain orphaned in the reducer's state.windows"
                        );
                    }
                }
                Err(e) => {
                    dlog(&format!("backend_close_window: write failed: {}", e));
                    tracing::error!(
                        window_id = %window_id,
                        error = %e,
                        "[backend_close_window] write failed — CloseWindow never reached the backend"
                    );
                }
            }
        }
        Err(e) => {
            dlog(&format!("backend_close_window: connect failed to {}: {}", addr, e));
            tracing::error!(
                window_id = %window_id,
                addr = %addr,
                error = %e,
                "[backend_close_window] connect failed — CloseWindow never reached the backend"
            );
        }
    }
}

/// Parse "http://host:port" / "https://host:port" into a `SocketAddr`, or
/// `None` (logged) if it doesn't parse. Shared by the two SPEC_PILLAR1_STEP2
/// helpers below — factored out rather than duplicating
/// `backend_close_window`'s inline parse.
fn parse_web_endpoint(web_endpoint: &str, caller: &str) -> Option<std::net::SocketAddr> {
    let addr_str = web_endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    match addr_str.parse() {
        Ok(a) => Some(a),
        Err(e) => {
            tracing::warn!("[{caller}] cannot parse endpoint '{}': {}", web_endpoint, e);
            None
        }
    }
}

/// SPEC_PILLAR1_STEP2 Slice A Phase 2 — write-through the host's per-window
/// opacity to srv's `Window.opacity` so a crashed/restarted host can restore
/// it (via `backend_get_window_opacity` below). Fire-and-forget, same shape
/// as `backend_close_window`: raw TCP, no async runtime needed, always
/// called from its own background thread (see `set_window_opacity` in
/// `commands/window/transparency.rs`) so a slow/failed srv round-trip never
/// stalls the opacity change the user is actively making. Failures are
/// logged, not surfaced — a missed write-through only affects crash
/// recovery, not the live opacity the user just set (which already applied
/// via the local Win32/AppKit/X11 side-effect before this is even called).
///
/// `opacity: None` sends a real JSON `null` — srv's `SetWindowOpacity` RPC
/// treats that as an explicit clear (`Window.opacity = None`), distinct
/// from `Some(1.0)` ("set to fully opaque"). Mirrors the reducer's own
/// `WindowOpacityApplied`/`WindowOpacityCleared` distinction (the caller in
/// `transparency.rs` maps `Cleared` to `None`, not `Some(1.0)`).
pub(crate) fn backend_set_window_opacity(web_endpoint: &str, auth_key: &str, window_id: &str, opacity: Option<f32>) {
    use std::io::Write;

    let Some(addr) = parse_web_endpoint(web_endpoint, "backend_set_window_opacity") else {
        return;
    };

    let body = serde_json::json!({
        "service": "window",
        "method": "SetWindowOpacity",
        "args": [window_id, opacity],
        "uicontext": null,
    })
    .to_string();
    let request = format!(
        "POST /agentmux/service HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         X-AuthKey: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        auth_key, body.len(), body
    );

    let timeout = std::time::Duration::from_millis(2000);
    match std::net::TcpStream::connect_timeout(&addr, timeout) {
        Ok(mut stream) => {
            stream.set_write_timeout(Some(timeout)).ok();
            stream.set_read_timeout(Some(timeout)).ok();
            if let Err(e) = stream.write_all(request.as_bytes()) {
                tracing::warn!(
                    window_id = %window_id,
                    error = %e,
                    "[backend_set_window_opacity] write failed — opacity mirror not persisted"
                );
                return;
            }
            // Drain the response so the connection closes cleanly; only the
            // status line matters here (best-effort, unlike
            // backend_close_window this isn't gating a user-visible retry).
            use std::io::Read;
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let first_line = resp.lines().next().unwrap_or("(empty)");
            if !first_line.contains(" 200 ") && !first_line.starts_with("HTTP/1.1 200") {
                tracing::warn!(
                    window_id = %window_id,
                    response = %first_line,
                    "[backend_set_window_opacity] SetWindowOpacity did not succeed — opacity mirror not persisted"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                window_id = %window_id,
                addr = %addr,
                error = %e,
                "[backend_set_window_opacity] connect failed — opacity mirror not persisted"
            );
        }
    }
}

/// SPEC_PILLAR1_STEP2 Slice A Phase 2 — read back the last-persisted opacity
/// for `window_id` from srv (the crash-recovery path: the host's in-memory
/// `window_opacities` map is empty on a fresh process, so a restart falls
/// back to this instead of defaulting to fully opaque). Blocking — callers
/// run it via `tokio::task::spawn_blocking` (see `get_window_opacity` in
/// `commands/window/transparency.rs`), not inline on an async task, since
/// this does a real network round-trip.
///
/// Returns `None` on any failure (parse/connect/non-200/missing field) —
/// the caller's existing `unwrap_or(1.0)` fallback covers all of those
/// identically, so there's no need to distinguish "srv has no value" from
/// "couldn't reach srv" here.
pub(crate) fn backend_get_window_opacity(web_endpoint: &str, auth_key: &str, window_id: &str) -> Option<f32> {
    use std::io::Write;

    let addr = parse_web_endpoint(web_endpoint, "backend_get_window_opacity")?;

    let body = serde_json::json!({
        "service": "window",
        "method": "GetWindow",
        "args": [window_id],
        "uicontext": null,
    })
    .to_string();
    let request = format!(
        "POST /agentmux/service HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         X-AuthKey: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        auth_key, body.len(), body
    );

    let timeout = std::time::Duration::from_millis(2000);
    let mut stream = std::net::TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| tracing::warn!(window_id = %window_id, error = %e, "[backend_get_window_opacity] connect failed"))
        .ok()?;
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_read_timeout(Some(timeout)).ok();
    stream
        .write_all(request.as_bytes())
        .map_err(|e| tracing::warn!(window_id = %window_id, error = %e, "[backend_get_window_opacity] write failed"))
        .ok()?;

    use std::io::Read;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;

    // Split the raw HTTP/1.1 response into headers and body on the blank
    // line (matches the request format written above — no chunked
    // encoding involved, srv's axum response is a single JSON payload).
    let body_str = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(body_str).ok()?;
    if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    parsed.get("data")?.get("opacity")?.as_f64().map(|o| o as f32)
}

/// SPEC_PILLAR1_STEP2 Slice B Phase 4 — write-through a floating pane's
/// OS-window placement to its block's `meta` map, via the existing generic
/// `object.UpdateObjectMeta` RPC (Phase E.5.3 — `Command::UpdateBlockMeta`
/// already does a shallow merge against `block.meta`, so no new srv-side RPC
/// was needed for this phase, unlike opacity's Phase 1). `meta_patch` is
/// merged as-is — pass only the keys that changed (e.g. omit
/// `pane:floating_normal_rect` on a restore that only changes placement).
///
/// Fire-and-forget, same shape as `backend_set_window_opacity`: raw TCP, no
/// async runtime needed, called from `toggle_floating_maximize`
/// (`commands/window/chrome.rs`) off the calling thread so a slow/failed srv
/// round-trip never stalls the maximize/restore the user is actively
/// triggering. No debounce (unlike opacity): this fires once per button
/// click, not once per drag tick, so there's no burst to collapse.
pub(crate) fn backend_update_block_meta(web_endpoint: &str, auth_key: &str, block_id: &str, meta_patch: serde_json::Value) {
    use std::io::Write;

    let Some(addr) = parse_web_endpoint(web_endpoint, "backend_update_block_meta") else {
        return;
    };

    let oref = format!("block:{block_id}");
    let body = serde_json::json!({
        "service": "object",
        "method": "UpdateObjectMeta",
        "args": [oref, meta_patch],
        "uicontext": null,
    })
    .to_string();
    let request = format!(
        "POST /agentmux/service HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         X-AuthKey: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        auth_key, body.len(), body
    );

    let timeout = std::time::Duration::from_millis(2000);
    match std::net::TcpStream::connect_timeout(&addr, timeout) {
        Ok(mut stream) => {
            stream.set_write_timeout(Some(timeout)).ok();
            stream.set_read_timeout(Some(timeout)).ok();
            if let Err(e) = stream.write_all(request.as_bytes()) {
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    "[backend_update_block_meta] write failed — floating-pane placement not persisted"
                );
                return;
            }
            use std::io::Read;
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let first_line = resp.lines().next().unwrap_or("(empty)");
            if !first_line.contains(" 200 ") && !first_line.starts_with("HTTP/1.1 200") {
                tracing::warn!(
                    block_id = %block_id,
                    response = %first_line,
                    "[backend_update_block_meta] UpdateObjectMeta did not succeed — floating-pane placement not persisted"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                block_id = %block_id,
                addr = %addr,
                error = %e,
                "[backend_update_block_meta] connect failed — floating-pane placement not persisted"
            );
        }
    }
}
