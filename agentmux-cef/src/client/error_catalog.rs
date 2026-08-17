// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Human-readable copy for CEF/Chromium `cef_errorcode_t` values, in the
//! style of Chrome's own `net_error_info` strings ("This site can't be
//! reached", "Your connection isn't private", ...).
//!
//! Before this module existed, the browser-pane / dev-frontend load-error
//! page had no `<title>` element at all, so the window/tab title fell back
//! to the page's own `data:text/html;base64,...` URI — unreadable. Every
//! variant below fills BOTH the `<title>` and the on-page heading, so no
//! error path can regress back to a raw data URI as the visible title.
//!
//! Coverage is deliberately not 1:1 with every `ERR_*` constant CEF exposes
//! (there are ~190). The common navigation-facing failures (DNS, TCP
//! connect/reset, TLS/cert, HTTP, blocking) are named explicitly; anything
//! else falls through to a generic-but-still-descriptive default that uses
//! CEF's own `error_text` for the detail line, so an unmapped code still
//! never surfaces as a blank or URI-shaped title.

use cef::sys::cef_errorcode_t;

/// Human copy for one error page: window/tab title, on-page heading, and the
/// detail sentence shown under the failed URL.
pub(crate) struct ErrorCopy {
    pub title: &'static str,
    pub heading: &'static str,
    pub detail: &'static str,
}

const fn copy(title: &'static str, heading: &'static str, detail: &'static str) -> ErrorCopy {
    ErrorCopy { title, heading, detail }
}

/// Look up human copy for a raw `cef_errorcode_t` value (as `i32`, matching
/// how `on_load_error` already carries the code). Falls through to a generic
/// entry for anything not explicitly listed.
pub(crate) fn describe(error_code_i32: i32) -> ErrorCopy {
    // `cef_errorcode_t` variants are `#[repr(i32)]`-equivalent constants;
    // comparing the raw i32 against `VARIANT as i32` avoids needing a
    // `TryFrom`/exhaustive match over the whole enum.
    let c = |v: cef_errorcode_t| v as i32;

    match error_code_i32 {
        // --- DNS -------------------------------------------------------
        x if x == c(cef_errorcode_t::ERR_NAME_NOT_RESOLVED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The server's DNS address could not be found.",
        ),
        x if x == c(cef_errorcode_t::ERR_NAME_RESOLUTION_FAILED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "DNS lookup failed.",
        ),
        x if x == c(cef_errorcode_t::ERR_ICANN_NAME_COLLISION) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The requested name collides with an internal name.",
        ),

        // --- TCP connect / reset ----------------------------------------
        x if x == c(cef_errorcode_t::ERR_CONNECTION_TIMED_OUT) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The server took too long to respond.",
        ),
        x if x == c(cef_errorcode_t::ERR_TIMED_OUT) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The operation timed out.",
        ),
        x if x == c(cef_errorcode_t::ERR_CONNECTION_REFUSED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The server refused to connect.",
        ),
        x if x == c(cef_errorcode_t::ERR_CONNECTION_RESET) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The connection was reset.",
        ),
        x if x == c(cef_errorcode_t::ERR_CONNECTION_CLOSED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The connection was closed unexpectedly.",
        ),
        x if x == c(cef_errorcode_t::ERR_CONNECTION_ABORTED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The connection was aborted.",
        ),
        x if x == c(cef_errorcode_t::ERR_CONNECTION_FAILED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The connection failed.",
        ),
        x if x == c(cef_errorcode_t::ERR_ADDRESS_UNREACHABLE) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The server's address is unreachable.",
        ),
        x if x == c(cef_errorcode_t::ERR_ADDRESS_INVALID) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The server's address is invalid.",
        ),
        x if x == c(cef_errorcode_t::ERR_SOCKET_NOT_CONNECTED) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The connection is not established.",
        ),
        x if x == c(cef_errorcode_t::ERR_EMPTY_RESPONSE) => copy(
            "This site can't be reached",
            "This site can't be reached",
            "The server didn't send any data.",
        ),

        // --- Offline / network -------------------------------------------
        x if x == c(cef_errorcode_t::ERR_INTERNET_DISCONNECTED) => copy(
            "No internet connection",
            "No internet connection",
            "Check your network cable, modem, and router, then try again.",
        ),
        x if x == c(cef_errorcode_t::ERR_NETWORK_CHANGED) => copy(
            "Network changed",
            "Network changed",
            "The network connection changed while the page was loading.",
        ),
        x if x == c(cef_errorcode_t::ERR_NETWORK_IO_SUSPENDED) => copy(
            "Network unavailable",
            "Network unavailable",
            "The network I/O was suspended (e.g. system sleep).",
        ),
        x if x == c(cef_errorcode_t::ERR_NETWORK_ACCESS_DENIED) => copy(
            "Network access denied",
            "Network access denied",
            "The operating system denied network access to this request.",
        ),

        // --- TLS / certificate --------------------------------------------
        x if x == c(cef_errorcode_t::ERR_SSL_PROTOCOL_ERROR) => copy(
            "Your connection isn't private",
            "Your connection isn't private",
            "A TLS/SSL protocol error occurred while connecting.",
        ),
        x if x == c(cef_errorcode_t::ERR_CERT_COMMON_NAME_INVALID)
            || x == c(cef_errorcode_t::ERR_CERT_AUTHORITY_INVALID)
            || x == c(cef_errorcode_t::ERR_CERT_DATE_INVALID)
            || x == c(cef_errorcode_t::ERR_CERT_INVALID)
            || x == c(cef_errorcode_t::ERR_CERT_REVOKED)
            || x == c(cef_errorcode_t::ERR_CERT_WEAK_SIGNATURE_ALGORITHM)
            || x == c(cef_errorcode_t::ERR_CERT_WEAK_KEY)
            || x == c(cef_errorcode_t::ERR_CERT_CONTAINS_ERRORS) =>
        {
            copy(
                "Your connection isn't private",
                "Your connection isn't private",
                "The site's security certificate is not trusted.",
            )
        }
        x if x == c(cef_errorcode_t::ERR_SSL_VERSION_OR_CIPHER_MISMATCH) => copy(
            "Your connection isn't private",
            "Your connection isn't private",
            "The site uses an unsupported TLS version or cipher.",
        ),
        x if x == c(cef_errorcode_t::ERR_TLS13_DOWNGRADE_DETECTED) => copy(
            "Your connection isn't private",
            "Your connection isn't private",
            "A possible TLS downgrade attack was detected.",
        ),
        x if x == c(cef_errorcode_t::ERR_SSL_CLIENT_AUTH_CERT_NEEDED) => copy(
            "Client certificate required",
            "Client certificate required",
            "The server requires a client certificate to continue.",
        ),

        // --- HTTP / response -----------------------------------------------
        x if x == c(cef_errorcode_t::ERR_TOO_MANY_REDIRECTS) => copy(
            "This page isn't working",
            "This page isn't working",
            "The page redirected too many times.",
        ),
        x if x == c(cef_errorcode_t::ERR_INVALID_RESPONSE) => copy(
            "This page isn't working",
            "This page isn't working",
            "The server sent an invalid response.",
        ),
        x if x == c(cef_errorcode_t::ERR_RESPONSE_HEADERS_TOO_BIG) => copy(
            "This page isn't working",
            "This page isn't working",
            "The server's response headers were too large.",
        ),
        x if x == c(cef_errorcode_t::ERR_CONTENT_LENGTH_MISMATCH)
            || x == c(cef_errorcode_t::ERR_INCOMPLETE_CHUNKED_ENCODING) =>
        {
            copy(
                "This page isn't working",
                "This page isn't working",
                "The connection was closed before the response finished.",
            )
        }

        // --- Blocked ---------------------------------------------------------
        x if x == c(cef_errorcode_t::ERR_BLOCKED_BY_CLIENT) => copy(
            "This content is blocked",
            "This content is blocked",
            "The request was blocked (e.g. by an extension).",
        ),
        x if x == c(cef_errorcode_t::ERR_BLOCKED_BY_RESPONSE) => copy(
            "This content is blocked",
            "This content is blocked",
            "The server's response headers blocked this request.",
        ),
        x if x == c(cef_errorcode_t::ERR_BLOCKED_BY_CSP) => copy(
            "This content is blocked",
            "This content is blocked",
            "Blocked by the page's Content Security Policy.",
        ),
        x if x == c(cef_errorcode_t::ERR_BLOCKED_BY_ORB) => copy(
            "This content is blocked",
            "This content is blocked",
            "Blocked by opaque-response-blocking (cross-origin protection).",
        ),
        x if x == c(cef_errorcode_t::ERR_BLOCKED_BY_ADMINISTRATOR) => copy(
            "This content is blocked",
            "This content is blocked",
            "Blocked by your administrator's policy.",
        ),

        // --- Address / scheme / general request ---------------------------
        x if x == c(cef_errorcode_t::ERR_INVALID_URL) => {
            copy("Invalid address", "Invalid address", "The requested address is not a valid URL.")
        }
        x if x == c(cef_errorcode_t::ERR_UNKNOWN_URL_SCHEME)
            || x == c(cef_errorcode_t::ERR_DISALLOWED_URL_SCHEME) =>
        {
            copy(
                "Unsupported address",
                "Unsupported address",
                "This URL scheme is not supported here.",
            )
        }
        x if x == c(cef_errorcode_t::ERR_FILE_NOT_FOUND) => {
            copy("File not found", "File not found", "The requested file could not be found.")
        }

        // --- Proxy -------------------------------------------------------
        x if x == c(cef_errorcode_t::ERR_PROXY_CONNECTION_FAILED)
            || x == c(cef_errorcode_t::ERR_TUNNEL_CONNECTION_FAILED) =>
        {
            copy(
                "Can't reach this page",
                "Can't reach this page",
                "Could not connect to the configured proxy server.",
            )
        }
        x if x == c(cef_errorcode_t::ERR_MANDATORY_PROXY_CONFIGURATION_FAILED) => copy(
            "Proxy configuration failed",
            "Proxy configuration failed",
            "The required proxy configuration could not be loaded.",
        ),
        x if x == c(cef_errorcode_t::ERR_PAC_SCRIPT_FAILED) => copy(
            "Proxy configuration failed",
            "Proxy configuration failed",
            "The proxy auto-config (PAC) script failed to run.",
        ),

        // --- Generic fallback ------------------------------------------------
        // Covers every other cef_errorcode_t: still a real title (never blank,
        // never the data: URI), with CEF's own error_text folded in by the
        // caller for the detail line.
        _ => copy("This page isn't working", "This page isn't working", "Couldn't load this page."),
    }
}
