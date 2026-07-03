// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Re-run the direct agent↔account link backfill to catch instances
//! launched in the gap between `m0013`'s original run and the launch flow
//! write-through landing (PR B1, #1950).
//!
//! `m0013` backfills `db_agent_identity_links` from bundle bindings, but
//! every migration runs AT MOST ONCE per install (guarded by
//! `db_migrations`, see `migrations/mod.rs`). Any agent launched after
//! `m0013` ran on a given install, but before PR B1 shipped, would have a
//! bundle-based `identity_id` and NO direct link — `m0013` never sees it
//! because it already ran. PR B3 (next) removes the resolver's
//! bundle-bindings fallback entirely, so every such instance needs its
//! direct link before that lands, or it silently loses its credentials.
//!
//! **Deliberately NOT a straight re-run of `m0013::backfill_direct_links`.**
//! That function unions bundle-implied bindings across EVERY historical
//! instance of a definition (last-write-wins by `created_at`). PR B1's
//! launch-time reconcile actively UNLINKS a provider no longer in the
//! newly-picked bundle — but that unlink only touches
//! `db_agent_identity_links`, not the OLDER instance rows, whose
//! `identity_id` still points at the wider original bundle. A straight
//! re-run would walk that older instance again and silently resurrect the
//! link the user just removed (reagent P0 on #1952). Instead, this
//! migration only ever looks at the SINGLE latest instance per
//! definition — the one whose bundle pick (or "blank") is the actual
//! current intent — and only ADDS/UPDATES links implied by it. It
//! deliberately does **not** unlink anything: `identity.account.upsert`
//! (the per-agent Accounts tab, `app_api/identity.rs:150`) also writes
//! directly to `db_agent_identity_links`, entirely independent of any
//! bundle or launch, so an unconditional "unlink whatever isn't in the
//! latest bundle" pass would destroy those manually-configured links.
//! Any pre-write-through staleness left over from a `"blank"` relaunch
//! that never got a chance to reconcile is accepted as a narrow,
//! non-destructive residual gap rather than risking that.

use std::collections::HashMap;

use crate::backend::storage::store::{AgentInstance, Store};
use super::m0013_agent_direct_bindings::{is_sentinel_identity, open_shared_and_instance_sources};
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0014AgentDirectBindingsRerun;

impl Migration for M0014AgentDirectBindingsRerun {
    fn id(&self) -> &'static str { "0014_agent_direct_bindings_rerun" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Re-run the direct agent<->account link backfill (latest instance per \
         definition only) for instances launched after m0013's one-time run \
         but before the launch-flow write-through"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let (shared, sibling_stores) =
            open_shared_and_instance_sources(ctx, "agent_direct_bindings_rerun")?;
        backfill_latest_instance_only(&shared, &sibling_stores)
    }
}

/// Core backfill. Extracted so tests can drive it against an in-memory
/// shared store + in-memory instance sources, same shape as `m0013`'s
/// `backfill_direct_links`.
pub(crate) fn backfill_latest_instance_only(
    shared: &Store,
    instance_sources: &[Store],
) -> Result<(), MigrationError> {
    let mut instances: Vec<AgentInstance> = Vec::new();
    for src in instance_sources {
        match src.instance_list(None, None) {
            Ok(list) => instances.extend(list),
            Err(e) => tracing::warn!(
                "agent_direct_bindings_rerun: instance_list failed for a source store; \
                 those instances will not be considered (bundle fallback still applies \
                 until PR B3): {e}"
            ),
        }
    }

    // Keep only the single latest instance per definition — by (created_at,
    // id), matching m0013's own tie-break — so only the CURRENT intent
    // (the most recent launch's bundle pick, or "blank") contributes.
    let mut latest_by_def: HashMap<String, AgentInstance> = HashMap::new();
    for inst in instances {
        if inst.definition_id.is_empty() {
            continue;
        }
        match latest_by_def.get(&inst.definition_id) {
            Some(existing)
                if (existing.created_at, existing.id.as_str())
                    >= (inst.created_at, inst.id.as_str()) => {}
            _ => {
                latest_by_def.insert(inst.definition_id.clone(), inst);
            }
        }
    }

    let mut written = 0usize;
    let definitions = latest_by_def.len();

    for (definition_id, inst) in &latest_by_def {
        if is_sentinel_identity(&inst.identity_id) {
            continue;
        }
        let bindings = match shared.bundle_identity_bindings(&inst.identity_id) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    definition_id = %definition_id,
                    identity_id = %inst.identity_id,
                    "agent_direct_bindings_rerun: bundle_identity_bindings failed; this \
                     definition will not be backfilled (bundle fallback still applies \
                     until PR B3): {e}"
                );
                continue;
            }
        };
        for b in bindings {
            shared
                .agent_identity_link(definition_id, &b.account_id, &b.provider)
                .map_err(|e| {
                    MigrationError(format!(
                        "agent_direct_bindings_rerun: link def {definition_id} provider {}: {e}",
                        b.provider
                    ))
                })?;
            written += 1;
        }
    }

    tracing::info!(
        written,
        definitions,
        "agent_direct_bindings_rerun: backfill complete"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{Identity, IdentityAccount, SecretRef, Store};

    fn shared_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    fn channel_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn make_def(store: &Store, id: &str) {
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: id.to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();
    }

    fn make_instance(store: &Store, id: &str, definition_id: &str, identity_id: &str, created_at: i64) {
        let inst = AgentInstance {
            id: id.to_string(),
            definition_id: definition_id.to_string(),
            parent_instance_id: String::new(),
            block_id: format!("block-{id}"),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at,
            identity_id: identity_id.to_string(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
    }

    fn make_account(store: &Store, id: &str, provider: &str) {
        let acct = IdentityAccount {
            id: id.to_string(),
            name: format!("{provider}-{id}"),
            provider: provider.to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::Env { env_var: format!("VAR_{id}") },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&acct).unwrap();
    }

    fn make_bundle(store: &Store, id: &str) {
        let bundle = Identity {
            id: id.to_string(),
            name: format!("bundle-{id}"),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&bundle).unwrap();
    }

    /// The exact scenario reagent flagged (P0 on #1952): an OLDER instance's
    /// bundle has a provider that a NEWER instance's bundle (or "blank")
    /// dropped. A straight re-run of m0013's union-based backfill would
    /// resurrect the dropped provider; this migration must not.
    #[test]
    fn does_not_resurrect_a_provider_dropped_by_a_later_launch() {
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "bundle-wide");
        make_bundle(&shared, "bundle-narrow");
        make_account(&shared, "acct-gh", "github");
        make_account(&shared, "acct-anth", "anthropic");
        shared.bundle_identity_bind("bundle-wide", "github", "acct-gh").unwrap();
        shared.bundle_identity_bind("bundle-wide", "anthropic", "acct-anth").unwrap();
        // Only anthropic — github was deliberately dropped in the later launch.
        shared.bundle_identity_bind("bundle-narrow", "anthropic", "acct-anth").unwrap();

        // Older instance (wide bundle) came first; newer instance (narrow
        // bundle) is the CURRENT intent.
        make_instance(&channel, "inst-old", "def-1", "bundle-wide", 100);
        make_instance(&channel, "inst-new", "def-1", "bundle-narrow", 200);

        // Simulate PR B1's reconcile already having run for the newer
        // launch: github got unlinked, anthropic got linked.
        shared.agent_identity_link("def-1", "acct-anth", "anthropic").unwrap();

        backfill_latest_instance_only(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-1").unwrap();
        assert_eq!(links.len(), 1, "github must NOT be resurrected");
        assert_eq!(links[0].provider, "anthropic");
    }

    /// A "blank" (ambient-creds) later launch means the migration should add
    /// nothing new for that definition, even though an older instance had a
    /// real bundle.
    #[test]
    fn latest_blank_launch_adds_nothing() {
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "bundle-work");
        make_account(&shared, "acct-gh", "github");
        shared.bundle_identity_bind("bundle-work", "github", "acct-gh").unwrap();

        make_instance(&channel, "inst-old", "def-1", "bundle-work", 100);
        make_instance(&channel, "inst-new", "def-1", "blank", 200);

        backfill_latest_instance_only(&shared, &[channel]).unwrap();

        assert!(shared.agent_identity_list_for_agent("def-1").unwrap().is_empty());
    }

    /// Baseline: a single non-sentinel instance still gets backfilled, same
    /// as m0013's original behavior for the simple case.
    #[test]
    fn backfills_the_only_instance() {
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "bundle-work");
        make_account(&shared, "acct-gh", "github");
        shared.bundle_identity_bind("bundle-work", "github", "acct-gh").unwrap();
        make_instance(&channel, "inst-1", "def-1", "bundle-work", 0);

        backfill_latest_instance_only(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, "github");
        assert_eq!(links[0].account_id, "acct-gh");
    }

    #[test]
    fn is_idempotent() {
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "bundle-work");
        make_account(&shared, "acct-gh", "github");
        shared.bundle_identity_bind("bundle-work", "github", "acct-gh").unwrap();
        make_instance(&channel, "inst-1", "def-1", "bundle-work", 0);

        backfill_latest_instance_only(&shared, std::slice::from_ref(&channel)).unwrap();
        backfill_latest_instance_only(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-1").unwrap();
        assert_eq!(links.len(), 1, "re-run must not duplicate links");
    }

    /// A manually-configured direct link with no corresponding bundle at
    /// all (identity.account.upsert path, unrelated to any instance) must
    /// survive — this migration only adds, never unlinks.
    #[test]
    fn does_not_touch_manually_linked_providers_with_no_instance() {
        let shared = shared_store();
        let channel = channel_store();

        make_account(&shared, "acct-manual", "openclaw");
        // No AgentDefinition/instance at all for "def-manual" — simulates a
        // direct link created purely via the Accounts tab.
        shared.agent_identity_link("def-manual", "acct-manual", "openclaw").unwrap();

        backfill_latest_instance_only(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-manual").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, "openclaw");
    }
}
