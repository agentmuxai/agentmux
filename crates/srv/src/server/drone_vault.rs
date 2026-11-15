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
//!
//! Silence is a no: a drone often runs unattended, so the Vault's "goes ahead after 30 s" rule does
//! not apply to it ([`uses::goes_ahead`]). The run fails that block and the next run asks again.
//! A key that records the site or service it was saved for is bound to it: sent anywhere else, the
//! card says so and offers Allow once only, and an earlier grant doesn't count.

use std::time::Duration;

use crate::backend::vault::scrub::{self, Scrubber};
use crate::backend::vault::uses::{self, Ended, UseRequest};
use crate::backend::vault::{self, FillTarget};

use super::AppState;

/// Under the call's own timeout, so a drone waiting for the person doesn't hold a run for long.
const ANSWER_WAIT: Duration = Duration::from_secs(120);
/// The Activity line's action.
const CALLED: &str = "called an API";
const NOT_SENT: &str = "the request was not sent";
/// The Activity outcome of a use nobody answered, which a drone's never goes ahead on.
const NOT_ANSWERED: &str = "not answered";

/// The value to send and what takes it back out of what comes back.
pub(crate) struct Resolved {
    pub value: zeroize::Zeroizing<String>,
    pub scrubber: Scrubber,
}

/// The actor a drone's use of the Vault is recorded and granted under.
pub(crate) fn actor(drone_id: &str) -> String {
    // The whole id: two drones whose ids share a beginning are two actors, with a grant each.
    format!("{}{drone_id}", uses::DRONE_ACTOR)
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

/// Whether the site an entry says it was saved for is the one the drone sends it to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Binding {
    /// The entry records no site or service: nothing to compare.
    Unrecorded,
    Matches,
    /// It was saved for `recorded`, as the person wrote it.
    Differs { recorded: String },
}

/// The recorded site or service of an entry's plain fields: an api-key's `service`, a login's `site`.
fn recorded_of(kind: &str, fields: &str) -> String {
    let key = if kind == "login" { "site" } else { "service" };
    serde_json::from_str::<serde_json::Value>(fields).ok().and_then(|v| v.get(key).and_then(|x| x.as_str()).map(|x| x.trim().to_string())).unwrap_or_default()
}

/// The host a recorded value names, when it names one: a URL (`https://api.github.com/x`) or a bare host name
/// (`api.github.com`). A service's name (`GitHub`, `Acme Billing`) names none.
fn host_in(recorded: &str) -> Option<String> {
    let text = recorded.trim().trim_end_matches('/');
    let candidate = text.split("://").nth(1).unwrap_or(text);
    let host = candidate.split(['/', '?', '#']).next().unwrap_or("").split(':').next().unwrap_or("").trim().trim_start_matches("www.").to_ascii_lowercase();
    let host_like = !host.is_empty() && host.contains('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    host_like.then_some(host)
}

/// Whether the entry's recorded `site` fits `origin` (`https://host[:port]`): a recorded host fits that host and its
/// subdomains; a recorded name (`GitHub`) fits a host whose registrable name is it (`api.github.com`, not
/// `github.evil.example`). With no public-suffix list the name check is the host's second label from the end, so
/// `shop.example.co.uk` doesn't fit `Example`: a refusal to bind, never a wrong yes.
pub(crate) fn binding(recorded: &str, origin: &str) -> Binding {
    let recorded = recorded.trim();
    if recorded.is_empty() {
        return Binding::Unrecorded;
    }
    let host = reqwest::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    let fits = match host_in(recorded) {
        Some(want) => host == want || host.ends_with(&format!(".{want}")),
        None => {
            let name: String = recorded.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
            let labels: Vec<&str> = host.split('.').collect();
            !name.is_empty() && labels.len() >= 2 && labels[labels.len() - 2] == name
        }
    };
    if fits {
        Binding::Matches
    } else {
        Binding::Differs { recorded: recorded.to_string() }
    }
}

/// The host of `origin`, for the card.
fn host_of(origin: &str) -> String {
    reqwest::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| origin.to_string())
}

/// Whether a use that has no "Always" grant may ask the person.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Asking {
    /// A run the person can see: the first use asks (the API block).
    Person,
    /// A background poll: nobody is at the desk, and a poll every few minutes would stack up cards that silence then
    /// denies, so with no grant it fails at once, with no card and no line in the Vault's Activity. The grant is given
    /// where a person is: a manual run of the drone, answered "Always".
    Never,
}

/// How many polls a Vault Activity line stands for: one line per this long, with the count in it.
const POLL_LINE_EVERY_MS: i64 = 3_600_000;

/// When a poll last wrote its Activity line, and how many polls since, per drone and entry.
fn poll_lines() -> &'static std::sync::Mutex<std::collections::HashMap<(String, String), (i64, u32)>> {
    static LINES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), (i64, u32)>>> = std::sync::OnceLock::new();
    LINES.get_or_init(Default::default)
}

/// Counts a poll's use of `entry` and says whether this one writes the Activity line, and for how many polls.
fn poll_line_due(drone_id: &str, entry_id: &str, now_ms: i64) -> Option<u32> {
    let mut map = poll_lines().lock().unwrap_or_else(|p| p.into_inner());
    let slot = map.entry((drone_id.to_string(), entry_id.to_string())).or_insert((i64::MIN, 0));
    slot.1 += 1;
    if now_ms.saturating_sub(slot.0) < POLL_LINE_EVERY_MS {
        return None;
    }
    let polls = slot.1;
    *slot = (now_ms, 0);
    Some(polls)
}

/// Resolves, asks for and reads the entry `entry` for a call to `origin` by drone `drone_id`.
/// Nothing is read and nothing is sent unless this returns `Ok`.
pub(crate) async fn resolve(state: &AppState, drone_id: &str, entry: &str, origin: &str) -> Result<Resolved, String> {
    resolve_with(state, drone_id, entry, origin, Asking::Person).await
}

/// [`resolve`], saying whether a use with no grant may ask the person.
pub(crate) async fn resolve_with(state: &AppState, drone_id: &str, entry: &str, origin: &str, asking: Asking) -> Result<Resolved, String> {
    let entry = entry_text_ok(entry)?.to_string();
    let who = actor(drone_id);
    let (store, e, a) = (state.identity_store.clone(), entry, who.clone());
    let target: FillTarget = tokio::task::spawn_blocking(move || vault::command_target(&store, &e, Some(&a)))
        .await
        .map_err(|e| e.to_string())??;
    part_allowed(target.kind, target.part)?;

    // The site or service the entry says it was saved for, against the one this call goes to.
    let (store, id) = (state.identity_store.clone(), target.id.clone());
    let fields = tokio::task::spawn_blocking(move || store.vault_get(&id).ok().flatten().map(|r| r.fields).unwrap_or_default()).await.unwrap_or_default();
    let bound = binding(&recorded_of(target.kind, &fields), origin);
    let mismatch = match &bound {
        Binding::Differs { recorded } => Some(format!("This key was saved for {recorded}; the drone {drone_id} sends it to {}", host_of(origin))),
        _ => None,
    };

    let note = mismatch.clone().unwrap_or_default();
    let record = |outcome: &str| {
        vault::record_line(
            &state.identity_store,
            vault::Line { entry_id: &target.id, label: &target.label, actor: &who, action: CALLED, target: origin, outcome, reason: &note },
        );
        // Every window's Vault pane shows the new line of the record.
        super::vault_handlers::publish(state);
    };

    // An earlier "Always" doesn't cover a key that is now for another site: the card asks again.
    let key = grant_key(drone_id, origin, target.part);
    let granted = if mismatch.is_some() {
        false
    } else {
        let (store, id, k) = (state.identity_store.clone(), target.id.clone(), key.clone());
        tokio::task::spawn_blocking(move || store.vault_grant_check(&id, &k).unwrap_or(false)).await.unwrap_or(false)
    };
    if !granted && asking == Asking::Never {
        // Nothing is asked and nothing is written to the Activity: a poll that runs every few minutes would fill it.
        return Err(match &mismatch {
            Some(why) => format!("{why}: a poll can't ask, so it doesn't send it; run the drone by hand and choose Allow once"),
            None => format!(
                "the drone {drone_id} has no \"Always\" for {} at {origin}, and a poll can't ask: run the drone by hand once, answer \"Always\", then the poll goes ahead",
                target.label
            ),
        });
    }
    if !granted {
        let request = UseRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent: who.clone(),
            entry_id: target.id.clone(),
            entry_label: target.label.clone(),
            what: target.what.clone(),
            origin: origin.to_string(),
            field: "an authorization header".into(),
            // Only Allow once for a key saved for another site.
            offer_always: mismatch.is_none(),
            reason: mismatch.clone().unwrap_or_else(|| format!("it is the first time the drone {drone_id} calls {origin} with this entry")),
            kind: target.kind.to_string(),
            requested_ms: agentmux_common::time::now_ms_u64(),
            ..Default::default()
        };
        let headline = format!("The drone {drone_id} wants to call {origin} with your {}", target.what);
        // No pane of its own to answer in: asked in every window.
        let (ended, _) = super::vault_ask::ask(state, "", request, ANSWER_WAIT, &headline).await;
        if !ended.allowed() {
            // Silence is a no for a drone ([`uses::goes_ahead`]): said so, not as the person's refusal.
            if matches!(ended, Ended::Expired | Ended::NoAnswer) {
                record(NOT_ANSWERED);
                return Err(format!("nobody answered the Vault request to send {} to {origin}, and a drone's first use of a site needs a yes: {NOT_SENT}; the next run asks again", target.label));
            }
            record(ended.outcome());
            return Err(ended.refusal(NOT_SENT));
        }
        if ended == Ended::Always && mismatch.is_none() {
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
    if asking == Asking::Never {
        // One line an hour for a poll, with the number of polls it stands for; never the value.
        if let Some(polls) = poll_line_due(drone_id, &target.id, agentmux_common::time::now_ms()) {
            vault::record_line(
                &state.identity_store,
                vault::Line { entry_id: &target.id, label: &target.label, actor: &who, action: CALLED, target: origin, outcome: "done", reason: &format!("a poll, {polls} time(s) since the last line") },
            );
            super::vault_handlers::publish(state);
        }
    } else {
        record("done");
    }
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
        // Two drones whose ids share a long beginning are two actors, and a grant is for one.
        let (a, b) = (format!("{}a", "x".repeat(80)), format!("{}b", "x".repeat(80)));
        assert_ne!(actor(&a), actor(&b));
        assert_ne!(grant_key(&a, "https://api.example.com", "value"), grant_key(&b, "https://api.example.com", "value"));
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
