// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The API block's `auth`: a Vault entry whose value is sent as one header
//! (the Vault slice of Drone phase 3).
//!
//! ```json
//! { "auth": { "vault": "api-key-github", "as": "bearer" } }
//! { "auth": { "vault": "api-key-acme", "as": "header:X-Api-Key" } }
//! ```
//!
//! The drone's definition names the entry and the site; the value never appears in it, in a run's
//! input, in the ledger or in what the node outputs. What keeps it from going anywhere else:
//!
//! * The URL's site (scheme, host and port) is written out in the drone: nothing from a run (a
//!   `{{...}}` value, which may be something an outsider wrote) can pick or change it. It must be
//!   `https`.
//! * The resolved URL must have exactly that site, and no user name or password.
//! * A call with a credential follows no redirect: the one site is the only site.
//! * The header is the one the node names; the node's own `headers` can't set it as well.
//!
//! The pure checks are here; asking the person, the grant and reading the value are
//! `server::drone_vault`.

use std::collections::HashMap;

use reqwest::header::{HeaderName, HeaderValue};
use serde_json::Value;

/// Headers a credential can't be put in: the request's framing, not its authorization.
const REFUSED: [&str; 8] = ["host", "content-length", "transfer-encoding", "connection", "upgrade", "te", "trailer", "proxy-authorization"];

/// How the value is sent.
#[derive(Debug, Clone, PartialEq)]
pub enum How {
    /// `Authorization: Bearer <value>`.
    Bearer,
    /// `<name>: <value>`.
    Header(HeaderName),
}

/// A node's `auth`, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Auth {
    /// The Vault entry, `id` or `id.part`.
    pub entry: String,
    pub how: How,
}

/// The node's `auth`: none when the node has none (absent, `null` or an empty entry), or why it
/// can't be used. Nothing is read from the Vault here.
pub fn parse(data: &Value) -> Result<Option<Auth>, String> {
    let Some(auth) = data.get("auth").filter(|a| !a.is_null()) else { return Ok(None) };
    let obj = auth.as_object().ok_or("the API block's `auth` is an object: { \"vault\": \"<entry>\", \"as\": \"bearer\" }")?;
    for key in obj.keys() {
        if !matches!(key.as_str(), "vault" | "as") {
            return Err(format!("the API block's `auth` has no `{key}`: it takes `vault` and `as`"));
        }
    }
    let entry = obj.get("vault").and_then(Value::as_str).unwrap_or("").trim();
    if entry.is_empty() {
        return Ok(None);
    }
    let how = match obj.get("as").and_then(Value::as_str).unwrap_or("bearer").trim() {
        "bearer" => How::Bearer,
        other => match other.strip_prefix("header:") {
            Some(name) => {
                let name = name.trim();
                let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_| format!("`{name}` isn't a header name"))?;
                if REFUSED.contains(&header.as_str()) {
                    return Err(format!("the `{header}` header can't carry a Vault value"));
                }
                How::Header(header)
            }
            None => return Err(format!("the API block's auth `as` is `bearer` or `header:<Name>`, not `{other}`")),
        },
    };
    Ok(Some(Auth { entry: entry.to_string(), how }))
}

impl How {
    /// The header's name.
    pub fn name(&self) -> HeaderName {
        match self {
            How::Bearer => reqwest::header::AUTHORIZATION,
            How::Header(n) => n.clone(),
        }
    }

    /// The header's value for `secret`, marked sensitive so it is never shown in a debug print. Its
    /// error never holds the value.
    pub fn value(&self, secret: &str) -> Result<HeaderValue, String> {
        let text = match self {
            How::Bearer => format!("Bearer {secret}"),
            How::Header(_) => secret.to_string(),
        };
        let mut v = HeaderValue::from_str(&text).map_err(|_| "the Vault value can't be sent in a header (it has a line break or a character a header can't hold)".to_string())?;
        v.set_sensitive(true);
        Ok(v)
    }
}

/// Whether the node's own `headers` set `name` too: the credential is the Vault's alone.
pub fn sets_it_too(headers: &HashMap<String, String>, name: &HeaderName) -> bool {
    headers.keys().any(|k| k.trim().eq_ignore_ascii_case(name.as_str()))
}

/// The site a call with a credential goes to, from the URL as the drone has it written (`raw`) and
/// as it resolved: `https://host[:port]`. The site must be written out in `raw`, and the resolved
/// URL must be the same site.
pub fn site(raw: &str, resolved: &str) -> Result<String, String> {
    let rest = raw.trim().strip_prefix("https://").ok_or("a call with a Vault credential goes to an https:// URL")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || authority.contains("{{") || authority.contains("}}") {
        return Err("a call with a Vault credential has its site written out in the URL (https://host/...): no {{...}} in it".into());
    }
    if authority.contains(['@', '\\']) {
        return Err("a call with a Vault credential has no user name or password in its URL".into());
    }
    let written = reqwest::Url::parse(&format!("https://{authority}")).map_err(|e| format!("invalid URL: {e}"))?;
    let want = written.origin();
    if !want.is_tuple() {
        return Err("a call with a Vault credential goes to a site with a host name".into());
    }
    let got = reqwest::Url::parse(resolved).map_err(|e| format!("invalid URL: {e}"))?;
    if !got.username().is_empty() || got.password().is_some() {
        return Err("a call with a Vault credential has no user name or password in its URL".into());
    }
    if got.origin() != want {
        return Err(format!("the URL resolved to {}, not the site the drone names ({})", got.origin().ascii_serialization(), want.ascii_serialization()));
    }
    Ok(want.ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_node_with_no_auth_has_none() {
        assert_eq!(parse(&json!({})), Ok(None));
        assert_eq!(parse(&json!({ "auth": null })), Ok(None));
        assert_eq!(parse(&json!({ "auth": { "vault": "  ", "as": "bearer" } })), Ok(None));
    }

    #[test]
    fn auth_names_an_entry_and_how_it_is_sent() {
        let a = parse(&json!({ "auth": { "vault": " api-key-github ", "as": "bearer" } })).unwrap().unwrap();
        assert_eq!(a, Auth { entry: "api-key-github".into(), how: How::Bearer });
        assert_eq!(parse(&json!({ "auth": { "vault": "k" } })).unwrap().unwrap().how, How::Bearer);
        let h = parse(&json!({ "auth": { "vault": "k", "as": "header:X-Api-Key" } })).unwrap().unwrap();
        assert_eq!(h.how.name().as_str(), "x-api-key");
    }

    #[test]
    fn auth_that_is_not_understood_is_refused_not_ignored() {
        assert!(parse(&json!({ "auth": "api-key-github" })).is_err());
        assert!(parse(&json!({ "auth": { "vault": "k", "as": "basic" } })).is_err());
        assert!(parse(&json!({ "auth": { "vault": "k", "as": "header:" } })).is_err());
        assert!(parse(&json!({ "auth": { "vault": "k", "as": "header:bad name" } })).is_err());
        assert!(parse(&json!({ "auth": { "vault": "k", "extra": 1 } })).is_err());
        for name in ["Host", "content-length", "Transfer-Encoding", "Connection", "Proxy-Authorization"] {
            assert!(parse(&json!({ "auth": { "vault": "k", "as": format!("header:{name}") } })).is_err(), "{name}");
        }
    }

    #[test]
    fn the_header_holds_the_value_and_is_sensitive() {
        let v = How::Bearer.value("sk-test-1234").unwrap();
        assert_eq!(v.to_str().unwrap(), "Bearer sk-test-1234");
        assert!(v.is_sensitive());
        assert_eq!(format!("{v:?}"), "Sensitive");
        assert_eq!(How::Bearer.name(), reqwest::header::AUTHORIZATION);
        let h = How::Header(HeaderName::from_static("x-api-key")).value("sk-test-1234").unwrap();
        assert_eq!(h.to_str().unwrap(), "sk-test-1234");
    }

    #[test]
    fn a_value_a_header_cannot_hold_is_refused_without_naming_it() {
        let err = How::Bearer.value("sk-test\r\nX-Evil: 1").unwrap_err();
        assert!(!err.contains("sk-test") && !err.contains("X-Evil"), "{err}");
    }

    #[test]
    fn the_node_cannot_set_the_credentials_header_too() {
        let headers: HashMap<String, String> = [("authorization".to_string(), "x".to_string()), ("Accept".to_string(), "y".to_string())].into();
        assert!(sets_it_too(&headers, &reqwest::header::AUTHORIZATION));
        assert!(sets_it_too(&[(" Authorization ".to_string(), String::new())].into(), &reqwest::header::AUTHORIZATION));
        assert!(!sets_it_too(&headers, &HeaderName::from_static("x-api-key")));
    }

    #[test]
    fn the_site_is_written_out_and_https() {
        assert_eq!(site("https://api.example.com/v1/{{var.x}}", "https://api.example.com/v1/7").unwrap(), "https://api.example.com");
        assert_eq!(site("https://API.example.com:8443/x", "https://api.example.com:8443/x").unwrap(), "https://api.example.com:8443");
        assert_eq!(site("https://api.example.com", "https://api.example.com/").unwrap(), "https://api.example.com");
        assert_eq!(site("https://api.example.com?q={{var.q}}", "https://api.example.com/?q=1").unwrap(), "https://api.example.com");
        assert!(site("http://api.example.com/x", "http://api.example.com/x").is_err());
        assert!(site("{{var.base}}/x", "https://api.example.com/x").is_err());
    }

    #[test]
    fn nothing_from_a_run_can_pick_or_change_the_site() {
        // A reference in the host, in the port, or standing for the whole of it.
        assert!(site("https://{{var.host}}/x", "https://api.example.com/x").is_err());
        assert!(site("https://api.example.com:{{var.port}}/x", "https://api.example.com:443/x").is_err());
        assert!(site("https://api.{{var.tld}}/x", "https://api.example.com/x").is_err());
        assert!(site("https://{{trigger.url}}", "https://evil.example.net").is_err());
        // A user name that makes the real host the part after the @.
        assert!(site("https://api.example.com@evil.example.net/x", "https://api.example.com@evil.example.net/x").is_err());
        assert!(site("https://api.example.com\\@evil.example.net/x", "https://evil.example.net/x").is_err());
        // The path can be a run's value; a path that tries to leave the site stays on it, or is refused.
        assert_eq!(site("https://api.example.com/{{var.p}}", "https://api.example.com//evil.example.net/x").unwrap(), "https://api.example.com");
        assert!(site("https://api.example.com/{{var.p}}", "https://evil.example.net/x").is_err());
        // The host written out can't be a different one once resolved (the check is on the parsed URLs).
        assert!(site("https://api.example.com/x", "https://api.example.com.evil.example.net/x").is_err());
        assert!(site("https://api.example.com/x", "https://user:pass@api.example.com/x").is_err());
    }
}
