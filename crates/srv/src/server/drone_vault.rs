// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A drone's API node calling a site with a Vault entry's value: the one place a drone gets a
//! credential, the same way a command does (`shell_secrets`): the person is asked the first time,
//! "Always" is a grant, the value leaves its sealed row only after that, and the response is
//! cleaned of it before anything is kept (Drone real-world jobs spec, the Vault slice of phase 3).
//!
//! The drone is the actor, as `drone:<id>`. It is never a named agent, so a Vault open to named
//! agents only is refused to it as if the entry weren't there; a Vault open to all agents is not.
//! A grant is for one drone, one site (its origin) and one part of one entry: editing the node's
//! URL to another site, or the node to use another entry, asks again.

use std::time::Duration;

use crate::backend::vault::scrub::{self, Scrubber};
use crate::backend::vault::uses::{Ended, UseRequest};
use crate::backend::vault::{self, FillTarget};

use super::AppState;

/// Under the call's own timeout, so a drone waiting for the person doesn't hold a run for long.
const ANSWER_WAIT: Duration = Duration::from_secs(120);
/// The Activity line's action.
const CALLED: &str = "called an API";
const NOT_SENT: &str = "the request was not sent";

/// The value to send and what takes it back out of what comes back.
pub(crate) struct Resolved {
    pub value: zeroize::Zeroizing<String>,
    pub scrubber: Scrubber,
}

/// The actor a drone's use of the Vault is recorded and granted under.
pub(crate) fn actor(drone_id: &str) -> String {
    let id: String = drone_id.chars().take(64).collect();
    format!("drone:{id}")
}

/// The key a drone's grant is stored under, in the grants table that web forms and commands use:
/// `drone:<id>:<origin>:<part>` is neither a site's origin nor a command's key. It names the part
/// and the site, so a grant for one site's call is not one for another's.
pub(crate) fn grant_key(drone_id: &str, origin: &str, part: &str) -> String {
    format!("{}:{origin}:{part}", actor(drone_id))
}

/// Whether a call may be given this part of this kind of entry: an API key's value or a login's
/// password. An ID, a card, a bank account, a note or a login's one-time code is never an
/// Authorization header, and a drone has nobody to ask for them.
pub(crate) fn part_allowed(kind: &str, part: &str) -> Result<(), String> {
    match (kind, part) {
        ("api-key", "value") | ("login", "password") => Ok(()),
        _ => Err(format!("a drone's API call can use an api-key entry's value or a login's password, not the {part} of a {kind} entry")),
    }
}

/// The entry text of a node: `entry` or `entry.part`, never a `{{reference}}` (the entry is the
/// graph's, never a run's input) and never a version (`@previous`, `@pending` are a command's).
pub(crate) fn entry_text_ok(entry: &str) -> Result<&str, String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err("the API block's auth names no Vault entry".into());
    }
    if entry.contains("{{") || entry.contains("}}") {
        return Err("the API block's Vault entry is written in the drone, not taken from a {{...}} value".into());
    }
    if entry.contains('@') {
        return Err("the API block takes the entry's current value: no @version".into());
    }
    Ok(entry)
}

/// Resolves, asks for and reads the entry `entry` for a call to `origin` by drone `drone_id`.
/// Nothing is read and nothing is sent unless this returns `Ok`.
pub(crate) async fn resolve(state: &AppState, drone_id: &str, entry: &str, origin: &str) -> Result<Resolved, String> {
    let entry = entry_text_ok(entry)?.to_string();
    let who = actor(drone_id);
    let (store, e, a) = (state.identity_store.clone(), entry, who.clone());
    let target: FillTarget = tokio::task::spawn_blocking(move || vault::command_target(&store, &e, Some(&a)))
        .await
        .map_err(|e| e.to_string())??;
    part_allowed(target.kind, target.part)?;

    let record = |outcome: &str| {
        vault::record_line(
            &state.identity_store,
            vault::Line { entry_id: &target.id, label: &target.label, actor: &who, action: CALLED, target: origin, outcome, reason: "" },
        );
        // Every window's Vault pane shows the new line of the record.
        super::vault_handlers::publish(state);
    };

    let key = grant_key(drone_id, origin, target.part);
    let (store, id, k) = (state.identity_store.clone(), target.id.clone(), key.clone());
    let granted = tokio::task::spawn_blocking(move || store.vault_grant_check(&id, &k).unwrap_or(false)).await.unwrap_or(false);
    if !granted {
        let request = UseRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent: who.clone(),
            entry_id: target.id.clone(),
            entry_label: target.label.clone(),
            what: target.what.clone(),
            origin: origin.to_string(),
            field: "an authorization header".into(),
            offer_always: true,
            reason: format!("it is the first time the drone {drone_id} calls {origin} with this entry"),
            kind: target.kind.to_string(),
            requested_ms: agentmux_common::time::now_ms_u64(),
            ..Default::default()
        };
        let headline = format!("The drone {drone_id} wants to call {origin} with your {}", target.what);
        // No pane of its own to answer in: asked in every window.
        let (ended, _) = super::vault_ask::ask(state, "", request, ANSWER_WAIT, &headline).await;
        if !ended.allowed() {
            record(ended.outcome());
            return Err(ended.refusal(NOT_SENT));
        }
        if ended == Ended::Always {
            let (store, id, k, created) = (state.identity_store.clone(), target.id.clone(), key, target.created_at);
            let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&id, &k, agentmux_common::time::now_ms(), created)).await;
        }
    }

    // Allowed: the value leaves its sealed row now, into this one request.
    let (store, t) = (state.identity_store.clone(), target.clone());
    let value = match tokio::task::spawn_blocking(move || vault::value_for_command(&store, &t)).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            record("failed");
            return Err(e);
        }
        Err(e) => return Err(e.to_string()),
    };
    if value.chars().count() < scrub::MIN_LEN {
        record("failed");
        return Err(format!("{} is too short to send: the response couldn't be cleaned of it", target.label));
    }
    record("done");
    let (store, id) = (state.identity_store.clone(), target.id.clone());
    let _ = tokio::task::spawn_blocking(move || store.vault_touch(&id, agentmux_common::time::now_ms())).await;
    let scrubber = Scrubber::new(&[(target.id.clone(), value.to_string())]);
    Ok(Resolved { value, scrubber })
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn a_drone_is_its_own_actor_and_never_a_named_agent() {
        assert_eq!(actor("nightly"), "drone:nightly");
        assert_eq!(actor(&"x".repeat(100)).len(), "drone:".len() + 64);
    }

    #[test]
    fn a_grant_is_for_one_drone_one_site_and_one_part() {
        let k = grant_key("nightly", "https://api.example.com", "value");
        assert_eq!(k, "drone:nightly:https://api.example.com:value");
        assert_ne!(k, grant_key("nightly", "https://api.example.org", "value"));
        assert_ne!(k, grant_key("other", "https://api.example.com", "value"));
        assert_ne!(k, grant_key("nightly", "https://api.example.com", "password"));
        // Neither a site's origin (a web form's grant) nor a command's key.
        assert!(!k.starts_with("command:") && !k.starts_with("http"));
    }

    #[test]
    fn only_an_api_keys_value_or_a_logins_password_may_go_in_a_header() {
        assert!(part_allowed("api-key", "value").is_ok());
        assert!(part_allowed("login", "password").is_ok());
        assert!(part_allowed("login", "totp").is_err());
        assert!(part_allowed("login", "username").is_err());
        assert!(part_allowed("card", "number").is_err());
        assert!(part_allowed("note", "value").is_err());
        assert!(part_allowed("api-key", "other").is_err());
    }

    #[test]
    fn the_entry_is_written_in_the_drone() {
        assert_eq!(entry_text_ok(" api-key-stripe ").unwrap(), "api-key-stripe");
        assert_eq!(entry_text_ok("login-bank.password").unwrap(), "login-bank.password");
        assert!(entry_text_ok("").is_err());
        assert!(entry_text_ok("  ").is_err());
        assert!(entry_text_ok("{{trigger.entry}}").is_err());
        assert!(entry_text_ok("api-key-stripe@previous").is_err());
        assert!(entry_text_ok("api-key-stripe@pending").is_err());
    }
}

#[cfg(test)]
#[path = "drone_vault/tests.rs"]
mod tests;
