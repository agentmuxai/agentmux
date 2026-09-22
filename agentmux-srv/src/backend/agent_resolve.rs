// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one place a human-readable agent slug becomes a canonical
//! `db_agents.id`.
//!
//! **Why this exists.** `AGENTMUX_AGENT_ID` is a model-and-human-chosen slug,
//! not a key: `db_agents.slug` carries no `UNIQUE` constraint, deliberately
//! (`storage::migrations` declines the index because template launches copy
//! their template's slug). Treating it as a primary key has produced the same
//! defect four times — Codex P1 on #2901, reagentx on #2428, reagentx on
//! #3480, and `instance_get_by_slug` itself on #3500 — each found and fixed
//! separately because the resolution logic existed in three places that drifted
//! apart.
//!
//! Phase 0 of `SPEC_CANONICAL_AGENT_ID_MIGRATION_2026_09_21.md`. Later phases
//! call this instead of inventing a fourth resolver.
//!
//! **Resolve once, at the trust boundary.** Callers pass the slug they already
//! have — MCP arguments, audit-log display and agent-facing docs keep the
//! readable name. Everything downstream of this function uses the id.

use crate::backend::agent_registry_lookup::find_active_record_by_slug;
use crate::backend::storage::store::Store;

/// Resolve `slug_or_id` to a canonical `db_agents.id`.
///
/// Tried in order, first hit wins:
///
/// 1. **`db_agents` by slug** — the persisted row. Fails closed when two rows
///    share the slug (#3500), so an ambiguous slug falls through rather than
///    resolving to an arbitrary winner.
/// 2. **`db_agents` by id** — idempotent passthrough. Phases land in sequence,
///    so a caller may hold a value that is already resolved; requiring each one
///    to know which it has would defeat the point of a shared resolver.
/// 3. **The named-agent registry** — a live agent with no `db_agents` row yet,
///    which is the common case (launching an agent does not create one). Also
///    fails closed on a colliding slug (#3480).
///
/// `Err` means "no agent" *or* "more than one agent". They are deliberately
/// not distinguished yet: both of the underlying lookups already collapse
/// ambiguity into "not found", and surfacing `Ambiguous` separately is a
/// user-facing error-message decision the spec (§4) flags as needing UX input
/// rather than a choice to make here. Nothing about this function's shape
/// prevents adding it later.
pub(crate) fn resolve_agent_id(store: &Store, slug_or_id: &str) -> Result<String, String> {
    if slug_or_id.is_empty() {
        return Err("cannot resolve an empty agent id".to_string());
    }

    // A store error is reported, not swallowed. `resolve_agent_uuid` already
    // propagated it; `resolve_agent_definition_id` silently treated it as "not
    // this tier" and tried the registry, which turns a database fault into a
    // confident "unknown agent". The stricter of the two behaviors wins.
    match store.instance_get_by_slug(slug_or_id) {
        // `map_instance_row` populates `definition_id` from the same column as
        // `id` — in the consolidated `db_agents` table they are the same value,
        // which is why the three resolvers this replaces could return either
        // and still agree here.
        Ok(Some(instance)) if !instance.id.is_empty() => return Ok(instance.id),
        Ok(_) => {}
        Err(e) => return Err(format!("resolve_agent_id: store: {e}")),
    }

    match store.agent_def_get(slug_or_id) {
        Ok(Some(_)) => return Ok(slug_or_id.to_string()),
        Ok(None) => {}
        Err(e) => return Err(format!("resolve_agent_id: store: {e}")),
    }

    if let Some(rec) = find_active_record_by_slug(slug_or_id) {
        if !rec.data.definition_id.is_empty() {
            return Ok(rec.data.definition_id);
        }
    }

    Err(format!(
        "unknown agent '{slug_or_id}': no agent row, definition or active registry record resolves it"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::{test_agent_def, AgentInstance};

    // Tier 3 reads the global registry via `resolve_shared_registry_dir`, which
    // resolves the developer's real `~/.agentmux` unless `AGENTMUX_HOME_OVERRIDE`
    // is set — so a not-found assertion would depend on whichever agents happen
    // to be running on the machine. Every test here runs against an isolated
    // empty home, under the crate-wide lock that env var's other consumers take.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

    fn with_isolated_home<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path().to_str().unwrap());
        let out = f();
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        out
    }

    fn instance(id: &str, definition_id: &str, name: &str) -> AgentInstance {
        AgentInstance {
            id: id.to_string(),
            definition_id: definition_id.to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "init".to_string(),
            github_context: String::new(),
            started_at: 1,
            ended_at: 0,
            created_at: 1,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: name.to_string(),
            working_directory: String::new(),
            display_hidden: false,
        }
    }

    #[test]
    fn resolves_a_slug_to_the_agents_canonical_id() {
        with_isolated_home(|| {
            let store = Store::open_in_memory().unwrap();
            let mut def = test_agent_def("def-1", "Solo Agent", "claude", "agent", 1, "");
            def.slug = "solo".to_string();
            store.agent_def_insert(&mut def).unwrap();

            assert_eq!(resolve_agent_id(&store, "solo").unwrap(), "def-1");
        });
    }

    // Idempotency (spec §4): phases land in sequence, so a caller may already
    // hold a resolved id. Tier 1 misses here because the id is not the slug —
    // it is tier 2 that answers.
    #[test]
    fn passes_through_an_id_that_is_already_canonical() {
        with_isolated_home(|| {
            let store = Store::open_in_memory().unwrap();
            let mut def = test_agent_def("def-1", "Solo Agent", "claude", "agent", 1, "");
            def.slug = "solo".to_string();
            store.agent_def_insert(&mut def).unwrap();

            assert_eq!(resolve_agent_id(&store, "def-1").unwrap(), "def-1");
        });
    }

    #[test]
    fn errors_for_an_agent_that_exists_nowhere() {
        with_isolated_home(|| {
            let store = Store::open_in_memory().unwrap();
            assert!(resolve_agent_id(&store, "nobody").is_err());
        });
    }

    #[test]
    fn errors_on_an_empty_id_without_touching_the_store() {
        with_isolated_home(|| {
            let store = Store::open_in_memory().unwrap();
            assert!(resolve_agent_id(&store, "").is_err());
        });
    }

    // The §7 case the spec calls out as having no coverage anywhere. Built the
    // way a real install builds one: `instance_create` copies its template's
    // slug verbatim onto every launch, so launching one template twice is
    // enough. Tier 1 fails closed (#3500), tier 2 misses because a slug is not
    // an id, tier 3 finds nothing in the isolated registry — so the whole
    // resolver refuses rather than naming an arbitrary winner.
    #[test]
    fn refuses_a_slug_two_rows_share() {
        with_isolated_home(|| {
            let store = Store::open_in_memory().unwrap();
            let mut tmpl = test_agent_def("tmpl-1", "Shared Template", "claude", "agent", 1, "");
            tmpl.slug = "shared-template".to_string();
            tmpl.is_seeded = 1;
            store.agent_def_insert(&mut tmpl).unwrap();

            store.instance_create(&instance("launch-a", "tmpl-1", "Launch A")).unwrap();
            store.instance_create(&instance("launch-b", "tmpl-1", "Launch B")).unwrap();

            // Both write paths suffix-resolve now (§4), so force the duplicate.
            // One row first: resolving it proves the fixture is live, so the
            // refusal below is the second row's doing and not "nothing matched".
            store.test_force_slug("launch-a", "collide-me").unwrap();
            assert_eq!(
                resolve_agent_id(&store, "collide-me").unwrap(),
                "launch-a",
                "one row with the slug must resolve — otherwise the check below is vacuous"
            );

            store.test_force_slug("launch-b", "collide-me").unwrap();
            assert!(
                resolve_agent_id(&store, "collide-me").is_err(),
                "adding a second row with the same slug must flip resolve to refusal"
            );
        });
    }
}
