// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The long-lived values srv keeps for the person's accounts and for itself, registered with [`withhold`] where srv
//! reads or writes them, so none of them is written into an agent's transcript or shown to a paired device.
//!
//! - Every secret in the OS keychain (an account's token, the cloud sign-in, a peer's key): [`keychain`], from the
//!   keychain's own read and write.
//! - The cloud sign-in as srv assembles it (its tokens can be split across keychain entries): [`cloud_sign_in`].
//! - An agent's relay credential, kept in the database: [`agent_relay_credential`].
//! - A messaging bridge's token, read from settings: [`bridge_token`].
//! - A key in the agent's own environment lines, put in its process as written: [`env_credential`].
//! - The token that opens a container proxy relay: [`relay_token`].
//!
//! A value that carries its own expiry is withheld until then, even once a newer one replaces it; one with no known
//! expiry stays withheld after another replaces it, up to `withhold::KEPT_PER_OWNER` per owner.

use serde_json::Value;

use super::withhold;

/// Words in a field's name that say its value is a credential.
const SECRET_WORDS: [&str; 9] = ["token", "key", "secret", "password", "passwd", "auth", "credential", "bearer", "cookie"];

/// Withholds `value` for `owner`: until `expires_at` (unix ms) when it is known and still ahead, else as a kept value.
fn hold(owner: &str, value: &str, expires_at: Option<i64>) {
    if value.len() < withhold::MIN_LEN {
        return;
    }
    match expires_at {
        Some(at) if at > agentmux_common::time::now_ms() => withhold::hold_until(owner, value, at),
        _ => withhold::hold_kept(owner, value),
    }
}

fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|w| name.contains(w))
}

/// Withholds the credential-named string fields under `value`, each under an owner of its own.
fn walk(owner: &str, value: &Value, in_secret: bool) -> bool {
    match value {
        Value::String(s) if in_secret && !s.is_empty() => {
            hold(owner, s, None);
            true
        }
        Value::Object(fields) => fields.iter().fold(false, |any, (name, inner)| {
            walk(&format!("{owner}.{name}"), inner, in_secret || secret_name(name)) | any
        }),
        Value::Array(items) => items.iter().enumerate().fold(false, |any, (i, inner)| walk(&format!("{owner}.{i}"), inner, in_secret) | any),
        _ => false,
    }
}

/// A secret read from or written to the OS keychain under `account_id`. A JSON object (an OAuth account's tokens, a
/// sign-in blob) is withheld field by field, for the fields whose names say they are credentials; anything else is
/// withheld as it is.
pub fn keychain(account_id: &str, secret: &str) {
    let owner = format!("keychain:{account_id}");
    let trimmed = secret.trim_start();
    if trimmed.starts_with('{') {
        if let Ok(json) = serde_json::from_str::<Value>(secret) {
            if walk(&owner, &json, false) {
                return;
            }
        }
    }
    hold(&owner, secret, None);
}

/// The cloud sign-in's tokens (`expires_at` in unix seconds, 0 when unknown). The access and id tokens are withheld
/// until they expire, the refresh token until the sign-in ends.
pub fn cloud_sign_in(access: &str, refresh: &str, id: &str, expires_at: i64) {
    let until = (expires_at > 0).then(|| expires_at.saturating_mul(1000));
    hold("cloud:access", access, until);
    hold("cloud:id", id, until);
    hold("cloud:refresh", refresh, None);
}

/// An agent's relay credential: its client secret, and the access token cached from it (`expires_at` in unix seconds).
pub fn agent_relay_credential(agent_id: &str, client_secret: &str, access: &str, expires_at: i64) {
    hold(&format!("relay-credential:{agent_id}:secret"), client_secret, None);
    hold(&format!("relay-credential:{agent_id}:access"), access, (expires_at > 0).then(|| expires_at.saturating_mul(1000)));
}

/// A messaging bridge's token (`discord`, `telegram`, `slack:bot`, `whatsapp`).
pub fn bridge_token(bridge: &str, token: &str) {
    hold(&format!("bridge:{bridge}"), token, None);
}

/// One line of an agent's own environment, as it goes into its process: a literal value under a credential-style name.
/// A Vault reference, an empty value and a placeholder word hold no secret and are left alone; a Vault value is
/// withheld where it is resolved.
pub fn env_credential(agent_id: &str, name: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() || !crate::backend::agent_env::is_credential_name(name) {
        return;
    }
    if crate::backend::agent_env::vault_ref(value).is_some() || crate::backend::agent_env::is_placeholder(value) {
        return;
    }
    hold(&format!("env:{agent_id}:{}", name.to_ascii_uppercase()), value, None);
}

/// The token that opens a container proxy relay.
pub fn relay_token(token: &str) {
    hold("relay:container-proxy", token, None);
}
