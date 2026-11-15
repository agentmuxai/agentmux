// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How the served web UI is cached by a browser. A headless server serves the built UI from a folder, and the
//! package's files carried one fixed modification date, no validator and no `Cache-Control`: a browser then kept the
//! old shell (`index.html`) for weeks, and a revalidation by that date was answered "not modified" after the server was
//! upgraded, so a normal reload kept showing the old version.
//!
//! The rules, by file:
//! - `/assets/*` (Vite names each by its content hash): cached for a year and immutable. `private`, because the
//!   server answers on the person's own address only.
//! - every other file (`index.html`, which also answers any path no file claims, `early-boot.js`, `/fonts/*` and
//!   `/fontawesome/*`, none of which carry a hash): `no-cache`, so the browser asks again each time, with an `ETag` of
//!   the file's content, so a file that did not change costs a `304` and one that did is sent whole.
//! - no file is ever dated: `Last-Modified` is dropped, and an `If-Modified-Since` is not heard, because a browser that
//!   cached an older server's file holds only that date to ask with, and the answer to it must be the file.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

/// The longest file given an `ETag` (it is read whole to hash it): the largest un-hashed file is a font.
const MAX_HASHED_BYTES: usize = 16 * 1024 * 1024;

const IMMUTABLE: &str = "private, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

/// Middleware for the served UI's files; see the module's rules.
pub(crate) async fn ui_cache(mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let is_get = req.method() == axum::http::Method::GET;
    let if_none_match = req.headers().get(header::IF_NONE_MATCH).cloned();
    req.headers_mut().remove(header::IF_MODIFIED_SINCE);
    let mut response = next.run(req).await;
    response.headers_mut().remove(header::LAST_MODIFIED);
    if !response.status().is_success() {
        return response;
    }
    // A name under /assets/ that is no file is answered with the app's page (a single-page app's fallback): that is
    // not a hashed file, and a year of caching it would pin the page as the script.
    let is_page = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/html"));
    if path.starts_with("/assets/") && !is_page {
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
        return response;
    }
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(REVALIDATE));
    if response.status() != StatusCode::OK || !is_get {
        // A range of a file (206) has no validator of its own here, and a HEAD is answered with no body to hash: a tag
        // made of nothing would be the same for every file and differ from the GET's.
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_HASHED_BYTES).await else {
        return Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::empty()).unwrap_or_default();
    };
    let etag = format!("\"{}\"", hex::encode(&Sha256::digest(&bytes)[..16]));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        parts.headers.insert(header::ETAG, value);
    }
    if if_none_match.as_ref().and_then(|v| v.to_str().ok()).is_some_and(|v| matches(v, &etag)) {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_LENGTH);
        parts.headers.remove(header::CONTENT_TYPE);
        return Response::from_parts(parts, Body::empty());
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// Does an `If-None-Match` value name this tag (any of a list, or `*`; a weak tag counts as its strong form)?
fn matches(header: &str, etag: &str) -> bool {
    header.split(',').map(str::trim).any(|t| t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag)
}

#[cfg(test)]
#[path = "tests/ui_cache_tests.rs"]
mod tests;
