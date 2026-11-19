// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A shared sign-in for a site's HTTP prompt, in an agent's own browser profile
//! (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md A11).
//!
//! The host's credential broker asks for a pane's profile, origin and realm, as it does for the profile's own
//! sign-ins ([`super::service::credential`]). When the profile is an agent's own and holds none, the login the person
//! shared with that agent answers. This module says whose profile it is, what the host is told, and which uses the
//! person has already said yes to: an "Always" from the share card, or the card's yes itself, which covers the one
//! prompt that was waiting for it.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::backend::browser_profiles_store::agent_profile_id;
use crate::backend::vault::shares;
use crate::backend::vault::signins::{self, ProfileOwner, Space};

use super::AppState;

/// How long the person's yes on a share card also answers the prompt that waited for it.
const YES_COVERS: Duration = Duration::from_secs(5 * 60);

/// The yeses given on a share card that no sign-in has used yet, by entry and agent uid, with when each lapses.
static YESES: LazyLock<Mutex<HashMap<(String, String), Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The person said yes on the share card of `entry` for agent `uid`: the next prompt it answers in the next
/// [`YES_COVERS`] is not asked about again.
pub(crate) fn yes_given(entry: &str, uid: &str) {
    yes_given_until(entry, uid, Instant::now() + YES_COVERS);
}

fn yes_given_until(entry: &str, uid: &str, until: Instant) {
    let mut yeses = YESES.lock();
    let now = Instant::now();
    yeses.retain(|_, lapses| *lapses > now);
    yeses.insert((entry.to_string(), uid.to_string()), until);
}

fn yes_waits(entry: &str, uid: &str) -> bool {
    YESES.lock().get(&(entry.to_string(), uid.to_string())).is_some_and(|lapses| *lapses > Instant::now())
}

/// The sign-in the yes was for has been given to the browser: the next one asks.
fn yes_used(entry: &str, uid: &str) {
    YESES.lock().remove(&(entry.to_string(), uid.to_string()));
}

/// The share of `entry` with `who` (an agent's uid, or `*`) was removed: a yes still waiting to be used goes with it.
pub(crate) fn share_removed(entry: &str, who: &str) {
    YESES.lock().retain(|(e, uid), _| e != entry || (who != shares::EVERY_AGENT && uid != who));
}

/// The agent whose own browser profile `profile` is: how its Vaults name it, and the name the person reads.
/// `None` for Personal, a named profile, and a profile whose agent is gone.
pub(crate) fn owner_of(state: &AppState, profile: &str) -> Option<(ProfileOwner, String)> {
    let id = profile.strip_prefix("profile:").filter(|id| id.starts_with("agent-"))?;
    let defs = state.mstore.agent_def_list().ok()?;
    let def = defs.into_iter().find(|d| agent_profile_id(&d.id) == id)?;
    Some((ProfileOwner { names: vec![def.slug.clone(), def.id.clone()], uid: def.id }, def.name))
}

/// What the host is told of a shared sign-in that answers a prompt: never the password, and the user name masked.
pub(crate) struct Found {
    pub username: String,
    /// The agent the login is shared with, as the person reads it.
    pub agent: String,
    /// The person already said yes to this use: the browser answers without asking.
    pub granted: bool,
}

/// The shared sign-in that answers `space`, when its profile is an agent's own.
pub(crate) fn lookup(state: &AppState, space: &Space) -> Result<Option<Found>, String> {
    let Some((owner, agent)) = owner_of(state, &space.profile) else { return Ok(None) };
    let store = &state.identity_store;
    let Some((row, username)) = signins::find_shared(store, space, &owner)? else { return Ok(None) };
    let granted = shares::prompt_always(store, &row, &owner.uid, &space.profile) || yes_waits(&row.id, &owner.uid);
    Ok(Some(Found { username: signins::mask(&username), agent, granted }))
}

/// The user name and password of the shared sign-in that answers `space`, for the browser to give the site. The use
/// is recorded as the agent's, and a yes from the share card is used up by it.
pub(crate) fn fill(state: &AppState, space: &Space) -> Result<Option<(String, zeroize::Zeroizing<String>)>, String> {
    let Some((owner, _)) = owner_of(state, &space.profile) else { return Ok(None) };
    let actor = owner.names.first().cloned().unwrap_or_default();
    let Some((row, username, password)) = signins::fill_shared(&state.identity_store, space, &owner, &actor)? else { return Ok(None) };
    yes_used(&row.id, &owner.uid);
    Ok(Some((username, password)))
}

/// The site refused the shared sign-in given for `space`: it is marked, as the profile's own would be.
pub(crate) fn refused(state: &AppState, space: &Space) -> Result<(), String> {
    let Some((owner, _)) = owner_of(state, &space.profile) else { return Ok(()) };
    signins::refused_shared(&state.identity_store, space, &owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_yes_on_the_card_covers_one_use_and_lapses() {
        yes_given("login-a", "uid-1");
        assert!(yes_waits("login-a", "uid-1"));
        assert!(!yes_waits("login-a", "uid-2"), "another agent's prompt is still asked about");
        assert!(!yes_waits("login-b", "uid-1"), "another login's prompt is still asked about");
        yes_used("login-a", "uid-1");
        assert!(!yes_waits("login-a", "uid-1"), "the second prompt asks again");

        yes_given_until("login-c", "uid-1", Instant::now() - Duration::from_secs(1));
        assert!(!yes_waits("login-c", "uid-1"), "a yes nobody used in time is gone");
    }

    #[test]
    fn removing_the_share_ends_a_yes_that_still_waits() {
        yes_given("login-d", "uid-1");
        yes_given("login-d", "uid-2");
        yes_given("login-e", "uid-1");
        share_removed("login-d", "uid-1");
        assert!(!yes_waits("login-d", "uid-1"));
        assert!(yes_waits("login-d", "uid-2") && yes_waits("login-e", "uid-1"), "only the removed share's yes ends");
        share_removed("login-d", shares::EVERY_AGENT);
        assert!(!yes_waits("login-d", "uid-2"), "removing every agent's share ends them all for the login");
        assert!(yes_waits("login-e", "uid-1"));
    }
}
