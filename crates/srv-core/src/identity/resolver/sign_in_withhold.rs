// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeps the values in the sign-in files of Codex, Grok and Muse out of every transcript, as `oauth_probe` does for
//! Claude's: each look at a file holds what is in it now.
//!
//! - `codex` (`<dir>/auth.json`): `tokens.access_token`, `tokens.id_token`, `tokens.refresh_token`, `OPENAI_API_KEY`.
//! - `grok` (`<dir>/auth.json`): per scope, `key` and `refresh_token`.
//! - `muse` (`<dir>/muse/auth.json`): the layout is not published, so every string under a name that says it is a
//!   credential (token, key, secret, password, credential, auth).
//!
//! A token that carries its own expiry (a JWT's `exp`, grok's `expires_at`) is withheld until then, even after the
//! CLI has renewed it, since the old one still works; the other values are withheld until the file holds another.
//! A look that finds no value (a sign-out) leaves what was held.

use serde_json::Value;

use super::oauth_probe::sign_in_owner;
use crate::backend::transcript::withhold;

/// Withholds the values in `json`, the parsed sign-in file of `provider` under config dir `dir`.
pub(super) fn withhold_file(provider: &str, dir: &std::path::Path, json: &Value) {
    let dir = dir.to_string_lossy();
    match provider {
        "codex" => codex(&dir, json),
        "grok" => grok(&dir, json),
        "muse" => muse(&dir, json),
        _ => {}
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Withholds `value` until `expires_at` (unix ms) when it is known, else until the owner holds another.
fn hold(owner: &str, value: &str, expires_at: Option<i64>) {
    match expires_at {
        Some(at) => withhold::hold_until(owner, value, at),
        None => withhold::hold(owner, value, None),
    }
}

/// When a JWT expires (unix ms): its payload's `exp`. `None` for a token that is not a JWT.
fn jwt_expiry_ms(token: &str) -> Option<i64> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let exp = serde_json::from_slice::<Value>(&bytes).ok()?.get("exp")?.as_i64()?;
    exp.checked_mul(1000)
}

fn codex(dir: &str, json: &Value) {
    let tokens = json.get("tokens");
    for (what, name) in [("codex:access", "access_token"), ("codex:id", "id_token")] {
        if let Some(token) = text(tokens.and_then(|t| t.get(name))) {
            hold(&sign_in_owner(dir, what), token, jwt_expiry_ms(token));
        }
    }
    if let Some(refresh) = text(tokens.and_then(|t| t.get("refresh_token"))) {
        withhold::hold(&sign_in_owner(dir, "codex:refresh"), refresh, None);
    }
    if let Some(key) = text(json.get("OPENAI_API_KEY")) {
        withhold::hold(&sign_in_owner(dir, "codex:api-key"), key, None);
    }
}

fn grok(dir: &str, json: &Value) {
    let Some(scopes) = json.as_object() else { return };
    for (scope, entry) in scopes {
        if let Some(key) = text(entry.get("key")) {
            let expires = super::cli_sign_in_files::grok_expiry_ms(entry);
            hold(&sign_in_owner(dir, &format!("grok:{scope}:key")), key, expires);
        }
        if let Some(refresh) = text(entry.get("refresh_token")) {
            withhold::hold(&sign_in_owner(dir, &format!("grok:{scope}:refresh")), refresh, None);
        }
    }
}

/// Whether a field name says its value is a credential.
fn credential_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["token", "key", "secret", "password", "credential", "auth"].iter().any(|word| name.contains(word))
}

fn muse(dir: &str, json: &Value) {
    fn walk(dir: &str, path: &str, in_credential: bool, value: &Value) {
        match value {
            Value::String(s) if in_credential && !s.is_empty() => {
                withhold::hold(&sign_in_owner(dir, &format!("muse:{path}")), s, None);
            }
            Value::Object(fields) => {
                for (name, inner) in fields {
                    walk(dir, &format!("{path}.{name}"), in_credential || credential_name(name), inner);
                }
            }
            Value::Array(items) => {
                for (i, inner) in items.iter().enumerate() {
                    walk(dir, &format!("{path}.{i}"), in_credential, inner);
                }
            }
            _ => {}
        }
    }
    walk(dir, "", false, json);
}

#[cfg(test)]
#[path = "sign_in_withhold_tests.rs"]
mod tests;
