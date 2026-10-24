// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Former slugs on this instance's message routes
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` Phase 1): a slug an
//! agent was renamed from still reaches it, and never speaks for it. So does a
//! display name it was renamed from, by its route (`Store::former_name_owner`
//! answers for both kinds).
//!
//! The registries other instances write are keyed by the name an agent runs
//! under now, so a miss on a former slug is looked up again under its current
//! one. The request itself still carries the name it was sent to, because its
//! signatures cover it; the instance that runs the agent resolves the former
//! slug the same way (`Handler`'s former slug resolver). Every lookup here
//! runs only after the name missed as it was sent.

use std::path::Path;

use super::{agent_registry, AppState, InjectionRequest};
use crate::backend::reactive::registry::AgentEntry;
use crate::backend::storage::agents::FormerSlugOwner;

/// The agent `name` is a former slug or display name of, unless a live
/// registration here holds `name` itself (a live name always wins).
pub(super) fn owner(state: &AppState, name: &str) -> Option<FormerSlugOwner> {
    if state.reactive_handler.has_live_name(name) {
        return None;
    }
    state.mstore.former_name_owner(name).ok().flatten()
}

/// The slug a message to `name` is looked for under when nothing is
/// registered as `name`: the agent's whose current display name routes to it
/// ("Naki-2" for "Naki #2"; registries file an agent under the slug it runs
/// as), else the agent's it is a former name of. A live name here is its
/// holder's. The request still goes out addressed to `name`, so its
/// signatures cover what the sender signed (#1459).
pub(super) fn delivery_alias(state: &AppState, name: &str) -> Option<String> {
    if state.reactive_handler.has_live_name(name) {
        return None;
    }
    match state.mstore.display_route_owner(name) {
        Ok(Some(owner)) => Some(owner.slug),
        _ => state.mstore.former_name_owner(name).ok().flatten().map(|o| o.slug),
    }
}

/// The same-channel registry entry for `target`, else for the slug `alias`
/// it was renamed to, with the name it was found under.
pub(super) fn registry_entry(data_dir: &Path, target: &str, alias: Option<&str>) -> Option<(String, AgentEntry)> {
    std::iter::once(target)
        .chain(alias)
        .find_map(|name| agent_registry::lookup(data_dir, name).map(|entry| (name.to_string(), entry)))
}

/// The cross-channel registry's entries for `target`, else for `alias`, with
/// the name they were found under.
pub(super) fn shared_entries(shared_dir: &Path, target: &str, alias: Option<&str>) -> (String, Vec<AgentEntry>) {
    let found = agent_registry::lookup_all_shared(shared_dir, target);
    match alias {
        Some(alias) if found.is_empty() => (alias.to_string(), agent_registry::lookup_all_shared(shared_dir, alias)),
        _ => (target.to_string(), found),
    }
}

/// Whether a failed forward to an entry found under `found_as` may evict it:
/// only an entry found under the name as sent. One found under the current
/// slug failed for a name its instance may not know; that says nothing about
/// whether the agent is still there.
pub(super) fn may_evict(found_as: &str, req: &InjectionRequest) -> bool {
    found_as == req.target_agent
}

/// A former slug never speaks (Phase 1, "identity is not aliased"): the
/// handler's rule (`refuse_former_slug_sender`), which every delivery also
/// passes through, applied here while verifying so a forwarded request
/// carries the verdict too. `true` when it refused the claim.
pub(super) fn refuse_former_sender(state: &AppState, req: &mut InjectionRequest) -> bool {
    crate::backend::reactive::handler::refuse_former_slug_sender(
        req,
        |name| state.reactive_handler.has_live_name(name),
        |name| state.mstore.former_name_owner(name).ok().flatten().map(|o| o.uid),
    )
}

/// The host key a request signed as `name` is checked with, outside jekt
/// delivery (UI automation's `verified_block_id`): none for a former slug,
/// which never signs, whatever key is still filed under it.
pub(crate) fn signing_key(state: &AppState, name: &str) -> Result<Vec<u8>, String> {
    let former = crate::backend::reactive::handler::former_slug_claim(
        name,
        |n| state.reactive_handler.has_live_name(n),
        |n| state.mstore.former_name_owner(n).ok().flatten().map(|o| o.uid),
    );
    if former.is_some() {
        tracing::warn!(agent_id = %name, "signed as a name its agent was renamed from, which can't sign: refused");
        return Err(format!("agent_id={name:?} is a name its agent was renamed from, which can't sign"));
    }
    match state.mstore.agent_jekt_key_load(name) {
        Ok(Some(k)) => Ok(k),
        Ok(None) => Err(format!("no signing key on file for agent_id={name:?} — respawn this agent to get one")),
        Err(e) => Err(format!("load signing key: {e}")),
    }
}

/// `GET /agentmux/reactive/agent?id=` for a former slug: the agent's current
/// name, and only the public key filed under that name — never one left
/// under the former slug — when the agent alone owns it. A peer re-pins
/// under the current name; the former one gets no key to verify against.
pub(super) fn renamed_lookup(state: &AppState, id: &str, value: &mut serde_json::Value) {
    let Some(owner) = owner(state, id) else {
        return;
    };
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.remove("lan_public_key");
    obj.insert("current_name".into(), serde_json::json!(owner.slug));
    if crate::server::actor::uid_owns_name(&state.mstore, &owner.uid, &owner.slug) {
        if let Ok(Some(pubkey)) = state.mstore.agent_lan_public_key_load(&owner.slug) {
            obj.insert("lan_public_key".into(), serde_json::json!(pubkey));
        }
    }
}

#[cfg(test)]
#[path = "../tests/reactive/former_slug_tests.rs"]
mod tests;
