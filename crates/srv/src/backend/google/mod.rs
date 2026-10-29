// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Google connectors: srv calls Google for an agent, with the account's
//! token, which no agent ever holds
//! (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md D2).
//!
//! - [`Service`]: each service's scopes, by grant level (§3.2).
//! - [`GoogleHttp`]: Google's REST APIs, behind a trait so tests stub them.
//! - [`calendar`], [`drive`], [`gmail`]: the `Calendar`, `Drive` and `Gmail` tools' actions, as
//!   requests and trimmed answers.

use std::future::Future;

use serde_json::Value;

use crate::backend::storage::google_grants::GrantLevel;
use crate::identity::oauth_tokens::HttpReply;

pub mod access;
pub mod calendar;
pub mod calls;
pub mod drive;
pub mod gmail;

const SCOPE_PREFIX: &str = "https://www.googleapis.com/auth/";

/// A Google service an account can be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Calendar,
    Drive,
    Gmail,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Service::Calendar => "calendar",
            Service::Drive => "drive",
            Service::Gmail => "gmail",
        }
    }

    /// The scopes a level needs, as Google names them.
    pub fn scopes(self, level: GrantLevel) -> Vec<String> {
        let names: &[&str] = match (self, level) {
            (Service::Calendar, GrantLevel::Read) => {
                &["calendar.calendarlist.readonly", "calendar.events.readonly", "calendar.events.freebusy"]
            }
            (Service::Calendar, GrantLevel::Write) => &[
                "calendar.calendarlist.readonly",
                "calendar.events.readonly",
                "calendar.events.freebusy",
                "calendar.events",
            ],
            // `drive.file` is the one non-sensitive Drive scope: the files the person picked
            // and the ones AgentMux made. Reading and writing them is the same scope.
            (Service::Drive, _) => &["drive.file"],
            // Reading mail is `gmail.readonly`, a restricted scope: only a client the person brought
            // (their own, or their organization's internal one) is offered it (spec G3).
            (Service::Gmail, _) => &["gmail.readonly"],
        };
        names.iter().map(|n| format!("{SCOPE_PREFIX}{n}")).collect()
    }
}

/// One request to a Google REST API.
#[derive(Debug, Clone, PartialEq)]
pub struct GoogleRequest {
    pub method: reqwest::Method,
    pub url: String,
    pub query: Vec<(&'static str, String)>,
    pub body: Option<Value>,
    /// Read the answer as text (an export or a file's bytes), at most [`TEXT_CAP`] bytes of
    /// it, instead of as JSON. It comes back as `{ "text", "truncated" }`.
    pub text: bool,
}

/// The most bytes of a file `read_file_content` takes from Google.
pub const TEXT_CAP: usize = 200_000;

/// Google's REST APIs.
pub trait GoogleHttp: Sync {
    fn send(&self, req: &GoogleRequest, token: &str) -> impl Future<Output = Result<HttpReply, String>> + Send;
}

/// [`GoogleHttp`] over the network.
pub struct LiveGoogleHttp;

impl GoogleHttp for LiveGoogleHttp {
    async fn send(&self, req: &GoogleRequest, token: &str) -> Result<HttpReply, String> {
        let mut b = crate::identity::oauth_client::http()
            .request(req.method.clone(), &req.url)
            .bearer_auth(token)
            .header("Accept", if req.text { "*/*" } else { "application/json" })
            .query(&req.query);
        if let Some(body) = &req.body {
            b = b.json(body);
        }
        let mut resp = b.send().await.map_err(|e| format!("request failed: {}", e.without_url()))?;
        let status = resp.status().as_u16();
        if req.text && (200..300).contains(&status) {
            let mut buf: Vec<u8> = Vec::new();
            let mut truncated = false;
            while let Some(chunk) = resp.chunk().await.map_err(|e| format!("request failed: {}", e.without_url()))? {
                let room = TEXT_CAP - buf.len();
                if chunk.len() > room {
                    buf.extend_from_slice(&chunk[..room]);
                    truncated = true;
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
            let text = String::from_utf8_lossy(&buf).into_owned();
            return Ok(HttpReply { status, body: serde_json::json!({ "text": text, "truncated": truncated }) });
        }
        let body = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        Ok(HttpReply { status, body })
    }
}

/// Google's own message for a failed call, in one sentence for the agent.
pub fn google_error(reply: &HttpReply) -> String {
    let msg = reply.body.pointer("/error/message").and_then(Value::as_str).unwrap_or("").trim();
    let reason = reply.body.pointer("/error/errors/0/reason").and_then(Value::as_str).unwrap_or("");
    match (reply.status, msg.is_empty()) {
        (401, _) => "Google refused the account's token; reconnect it in Connectors".to_string(),
        (403, _) if reason.contains("insufficient") || msg.contains("insufficient") => {
            "the account hasn't granted the permission this needs; turn the service on again in Connectors".to_string()
        }
        (404, _) => "Google found no such calendar, event or file (or the account can't see it)".to_string(),
        (429, _) => "Google is rate-limiting this account; try again in a minute".to_string(),
        (s, false) => format!("Google answered HTTP {s}: {}", msg.chars().take(300).collect::<String>()),
        (s, true) => format!("Google answered HTTP {s}"),
    }
}

/// `s` cut to `max` characters, with an ellipsis when cut.
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests;
