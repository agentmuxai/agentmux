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

/// Write-through a window's position/size to srv's `Window.pos`/`winsize`
/// mirror (via `backend_get_window_pos_and_size` below) so a full
/// process-tree restart — where the launcher's live in-memory
/// `WindowMirror.last_rect` is also gone, not just the main window — can
/// still recreate secondary windows at their exact last geometry instead of
/// a default placement. Fire-and-forget, same shape as
/// `backend_set_window_opacity`: raw TCP, always called from its own
/// background thread (see `report_position_for_srv_writethrough` in
/// `commands/window/position_persist.rs`) so a slow/failed srv round-trip
/// never stalls the live window move the user is actively making.
///
/// `rect` is Win32 screen-coordinate `left/top/right/bottom`
/// (`agentmux_common::ipc::Rect`); converted here to srv's `pos: {x, y}` /
/// `size: {width, height}` shape (`agentmux-srv/src/backend/obj.rs`'s
/// `Point`/`WinSize`) since the two crates don't share these types directly
/// — the wire format is plain JSON either way.
pub(crate) fn backend_set_window_pos_and_size(web_endpoint: &str, auth_key: &str, window_id: &str, rect: agentmux_common::ipc::Rect) {
    use std::io::Write;

    let Some(addr) = parse_web_endpoint(web_endpoint, "backend_set_window_pos_and_size") else {
        return;
    };

    let body = serde_json::json!({
        "service": "window",
        "method": "SetWindowPosAndSize",
        "args": [
            window_id,
            { "x": rect.left, "y": rect.top },
            { "width": rect.right - rect.left, "height": rect.bottom - rect.top },
        ],
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
                    "[backend_set_window_pos_and_size] write failed — position mirror not persisted"
                );
                return;
            }
            use std::io::Read;
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let first_line = resp.lines().next().unwrap_or("(empty)");
            if !first_line.contains(" 200 ") && !first_line.starts_with("HTTP/1.1 200") {
                tracing::warn!(
                    window_id = %window_id,
                    response = %first_line,
                    "[backend_set_window_pos_and_size] SetWindowPosAndSize did not succeed — position mirror not persisted"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                window_id = %window_id,
                addr = %addr,
                error = %e,
                "[backend_set_window_pos_and_size] connect failed — position mirror not persisted"
            );
        }
    }
}

/// SPEC_PILLAR1_STEP3 Phase 2 — write-through a window's `kind` +
/// `parent_window_id` to srv, so a future reproject can tell which
/// native-window creation path to drive for each window. Fire-and-forget,
/// same shape as `backend_set_window_opacity`: raw TCP, called from its own
/// background thread (see `register_backend_window` in
/// `commands/window/meta.rs`) so a slow/failed srv round-trip never stalls
/// the registration the frontend is actively completing.
///
/// `parent_window_id` must already be resolved to a srv `Window.oid` by the
/// caller — `WindowMeta.parent_instance_id` (the host-side source of this
/// value) is a window LABEL, not a srv id; the caller resolves it via
/// `AppState::backend_window_id` before calling this.
pub(crate) fn backend_set_window_topology(
    web_endpoint: &str,
    auth_key: &str,
    window_id: &str,
    kind: &str,
    parent_window_id: Option<&str>,
) {
    use std::io::Write;

    let Some(addr) = parse_web_endpoint(web_endpoint, "backend_set_window_topology") else {
        return;
    };

    let body = serde_json::json!({
        "service": "window",
        "method": "SetWindowTopology",
        "args": [window_id, kind, parent_window_id],
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
                    "[backend_set_window_topology] write failed — topology mirror not persisted"
                );
                return;
            }
            use std::io::Read;
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let first_line = resp.lines().next().unwrap_or("(empty)");
            if !first_line.contains(" 200 ") && !first_line.starts_with("HTTP/1.1 200") {
                tracing::warn!(
                    window_id = %window_id,
                    response = %first_line,
                    "[backend_set_window_topology] SetWindowTopology did not succeed — topology mirror not persisted"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                window_id = %window_id,
                addr = %addr,
                error = %e,
                "[backend_set_window_topology] connect failed — topology mirror not persisted"
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

/// Read back the last-persisted position/size for `window_id` from srv —
/// the slow-path reproject counterpart to `backend_get_window_opacity`
/// above. Used only by `reproject_from_srv`
/// (`commands/window/creation.rs`), the full-process-tree-restart path
/// where the launcher's in-memory `WindowMirror.last_rect` is also gone,
/// unlike the fast path (`reproject_from_snapshot`) which already has it.
/// Same raw-TCP/blocking shape and same caller obligation (off the UI
/// thread — `reproject_from_srv` already runs on its own `std::thread`).
///
/// Returns `None` on any failure (parse/connect/non-200/missing field) —
/// the caller already treats "no rect available" as a valid fallback case
/// (falls through to the existing default-placement heuristic in
/// `open_window_with_kind`), same as it does today when this function
/// doesn't exist at all.
pub(crate) fn backend_get_window_pos_and_size(web_endpoint: &str, auth_key: &str, window_id: &str) -> Option<agentmux_common::ipc::Rect> {
    use std::io::Write;

    let addr = parse_web_endpoint(web_endpoint, "backend_get_window_pos_and_size")?;

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
        .map_err(|e| tracing::warn!(window_id = %window_id, error = %e, "[backend_get_window_pos_and_size] connect failed"))
        .ok()?;
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_read_timeout(Some(timeout)).ok();
    stream
        .write_all(request.as_bytes())
        .map_err(|e| tracing::warn!(window_id = %window_id, error = %e, "[backend_get_window_pos_and_size] write failed"))
        .ok()?;

    use std::io::Read;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;

    let body_str = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(body_str).ok()?;
    if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let data = parsed.get("data")?;
    let x = data.get("pos")?.get("x")?.as_i64()?;
    let y = data.get("pos")?.get("y")?.as_i64()?;
    let width = data.get("winsize")?.get("width")?.as_i64()?;
    let height = data.get("winsize")?.get("height")?.as_i64()?;
    if width <= 0 || height <= 0 {
        // Never-written Window.pos/winsize default to zero (Point/WinSize's
        // #[derive(Default)]), which is indistinguishable from "this row
        // predates the write-through" — treat a zero-or-negative size as
        // "no real geometry persisted" rather than recreating a
        // zero-sized window.
        return None;
    }
    Some(agentmux_common::ipc::Rect {
        left: x as i32,
        top: y as i32,
        right: (x + width) as i32,
        bottom: (y + height) as i32,
    })
}

/// SPEC_PILLAR1_STEP4 Phase 3 — slow-path reproject: read srv's durable
/// `Client.windowids` (the single source of window identity that survives
/// even a full process-tree kill, unlike the launcher's in-memory snapshot).
/// Same raw-TCP/blocking shape as `backend_get_window_opacity` — callers
/// MUST invoke this off the UI thread (`reproject_from_srv` spawns a
/// `std::thread`, mirroring `register_backend_window`'s write-through).
pub(crate) fn backend_get_client_window_ids(web_endpoint: &str, auth_key: &str) -> Option<Vec<String>> {
    use std::io::Write;

    let addr = parse_web_endpoint(web_endpoint, "backend_get_client_window_ids")?;

    let body = serde_json::json!({
        "service": "client",
        "method": "GetClientData",
        "args": [],
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
        .map_err(|e| tracing::warn!(error = %e, "[backend_get_client_window_ids] connect failed"))
        .ok()?;
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_read_timeout(Some(timeout)).ok();
    stream
        .write_all(request.as_bytes())
        .map_err(|e| tracing::warn!(error = %e, "[backend_get_client_window_ids] write failed"))
        .ok()?;

    use std::io::Read;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;

    let body_str = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(body_str).ok()?;
    if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let ids = parsed.get("data")?.get("windowids")?.as_array()?;
    Some(
        ids.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11 Residual 2 — resolve
/// srv Window rows by the `host:label` meta crumb persisted at CreateWindow
/// time. Returns ALL matching window ids (the crumb is a hint, not an
/// identity — labels can recur across host restarts); `None` on transport
/// failure, `Some(vec![])` on a clean no-match. Same wire shape / blocking
/// thread contract as `backend_get_client_window_ids` above.
pub(crate) fn backend_find_window_by_label(
    web_endpoint: &str,
    auth_key: &str,
    label: &str,
) -> Option<Vec<String>> {
    use std::io::Write;

    let addr = parse_web_endpoint(web_endpoint, "backend_find_window_by_label")?;

    let body = serde_json::json!({
        "service": "window",
        "method": "FindWindowByLabel",
        "args": [label],
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
        .map_err(|e| tracing::warn!(error = %e, "[backend_find_window_by_label] connect failed"))
        .ok()?;
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_read_timeout(Some(timeout)).ok();
    stream
        .write_all(request.as_bytes())
        .map_err(|e| tracing::warn!(error = %e, "[backend_find_window_by_label] write failed"))
        .ok()?;

    use std::io::Read;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;

    let body_str = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(body_str).ok()?;
    if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let ids = parsed.get("data")?.as_array()?;
    Some(
        ids.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// SPEC_PILLAR1_STEP4 Phase 3 — read one window's persisted `kind` /
/// `parent_window_id` (Step 3's fields) for the slow-path reproject driver.
/// Same shape/thread contract as `backend_get_client_window_ids` above.
pub(crate) fn backend_get_window_topology(
    web_endpoint: &str,
    auth_key: &str,
    window_id: &str,
) -> Option<(Option<String>, Option<String>)> {
    use std::io::Write;

    let addr = parse_web_endpoint(web_endpoint, "backend_get_window_topology")?;

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
        .map_err(|e| tracing::warn!(window_id = %window_id, error = %e, "[backend_get_window_topology] connect failed"))
        .ok()?;
    stream.set_write_timeout(Some(timeout)).ok();
    stream.set_read_timeout(Some(timeout)).ok();
    stream
        .write_all(request.as_bytes())
        .map_err(|e| tracing::warn!(window_id = %window_id, error = %e, "[backend_get_window_topology] write failed"))
        .ok()?;

    use std::io::Read;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;

    let body_str = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(body_str).ok()?;
    if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let data = parsed.get("data")?;
    let kind = data.get("kind").and_then(|v| v.as_str()).map(str::to_string);
    let parent_window_id = data.get("parent_window_id").and_then(|v| v.as_str()).map(str::to_string);
    Some((kind, parent_window_id))
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

/// One-time push of this host process's own CDP-automation credentials
/// (`ipc_port`, `ipc_token` — see `agentmux-cef/src/browser_api/mod.rs`)
/// to srv, so srv can proxy `/api/v1/ui/*` (agent-facing screenshot/click/
/// query tools) through to `/agentmux/browser/*` on this host's IPC server.
/// srv never generates or otherwise learns these values itself — the host
/// is the source of truth (same asymmetry as `auth_key`, just reversed:
/// here the HOST pushes a credential IT owns to srv, mirroring how srv's
/// own `auth_key` is already pushed the other way at spawn time via
/// `sidecar.rs`'s `.env("AGENTMUX_AUTH_KEY", ...)`).
///
/// Called once from `lib.rs`, after `backend_endpoints.web_endpoint` is
/// known (srv's address isn't resolved until then) — see
/// `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §"Architecture".
/// Fire-and-forget in the sense that a failure here doesn't block startup
/// (UI automation just won't work until the next successful call — there
/// is currently no retry), but failures ARE logged loudly since a silent
/// failure here would look like every future `UIScreenshot`/`UIClick`/
/// `UIQuery` call mysteriously erroring with "host has not registered".
pub(crate) fn register_ipc_with_backend(
    web_endpoint: &str,
    auth_key: &str,
    ipc_port: u16,
    ipc_token: &str,
) {
    use std::io::Write;

    let Some(addr) = parse_web_endpoint(web_endpoint, "register_ipc_with_backend") else {
        return;
    };

    let body = serde_json::json!({
        "service": "host_ipc",
        "method": "Register",
        "args": [ipc_port, ipc_token],
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
                tracing::error!(
                    error = %e,
                    "[register_ipc_with_backend] write failed — srv will not be able to \
                     proxy UI-automation calls to this host until a future retry succeeds"
                );
                return;
            }
            use std::io::Read;
            let mut resp = String::new();
            let _ = stream.read_to_string(&mut resp);
            let first_line = resp.lines().next().unwrap_or("(empty)");
            if first_line.contains(" 200 ") || first_line.starts_with("HTTP/1.1 200") {
                dlog("register_ipc_with_backend: srv acknowledged host_ipc.Register");
            } else {
                tracing::error!(
                    response = %first_line,
                    "[register_ipc_with_backend] host_ipc.Register did not succeed — \
                     srv will not be able to proxy UI-automation calls to this host"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                addr = %addr,
                error = %e,
                "[register_ipc_with_backend] connect failed — srv will not be able to \
                 proxy UI-automation calls to this host until a future retry succeeds"
            );
        }
    }
}
