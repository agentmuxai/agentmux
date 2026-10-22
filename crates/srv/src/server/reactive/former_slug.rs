// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Former slugs on this instance's message routes
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` Phase 1): a slug an
//! agent was renamed from still reaches it, and never speaks for it.
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

/// The agent `name` is a former slug of, unless a live registration here
/// holds `name` itself (a live name always wins).
pub(super) fn owner(state: &AppState, name: &str) -> Option<FormerSlugOwner> {
    if state.reactive_handler.has_live_name(name) {
        return None;
    }
    state.mstore.former_slug_owner(name).ok().flatten()
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
        |name| state.mstore.former_slug_owner(name).ok().flatten().map(|o| o.uid),
    )
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
