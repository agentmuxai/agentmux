// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill direct agent↔account links from bundle bindings.
//!
//! Phase 3 slice 2 PR-A (additive, behavior-preserving). Revives the
//! long-existing `db_agent_identity_links` table (`agent_id` ==
//! `AgentDefinition.id`) as a resolution path and seeds it from the
//! current `identity_bundle → binding → account` graph so the resolver's
//! new *dual-read* path resolves to the SAME accounts it does today.
//!
//! Mechanics (mirrors `m0011` / `m0012`):
//! - Global scope; targets the SHARED store — the resolver reads its
//!   bundle bindings + direct links from there, so the backfill must
//!   land there too.
//! - Instances live in the per-channel `objects.db` (not the shared
//!   store), so we enumerate the channel stores like `m0011` and read
//!   each instance's `(definition_id, identity_id)`.
//! - For every instance whose `identity_id` is NOT a sentinel
//!   (`''` / `'blank'`), read that bundle's bindings from the shared
//!   store and write each as a direct link on the instance's DEFINITION
//!   via `agent_identity_link` (upserts `ON CONFLICT(agent_id, provider)`).
//!
//! Equivalent single-DB SQL (for reference — the real run spans two DBs):
//! ```sql
//! INSERT OR REPLACE INTO db_agent_identity_links (agent_id, account_id, provider)
//! SELECT DISTINCT i.definition_id, b.account_id, b.provider
//! FROM db_agent_instances i JOIN db_identity_bindings b ON b.identity_id = i.identity_id
//! WHERE i.identity_id NOT IN ('', 'blank');
//! ```
//!
//! **Idempotency:** `agent_identity_link` is `INSERT ... ON CONFLICT ...
//! DO UPDATE`, so re-running converges to the same rows.
//!
//! **Edge cases (analysis §4):**
//! - The `default` bundle carries real OAuth bindings and MUST be
//!   backfilled — only `''` / `'blank'` are skipped.
//! - Multiple instances of one definition bound to different bundles →
//!   the `(agent_id, provider)` PK means last-write-wins. We order
//!   instances deterministically (by `created_at`, then `id`) and log
//!   any collision so the winner is reproducible.
//! - A bundle bound to several definitions fans out one link per
//!   definition, which is exactly what the PK allows.

use std::collections::HashMap;

use crate::backend::storage::store::Store;
use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0013AgentDirectBindings;

/// Sentinel identity ids that mean "no bundle → ambient creds". Never
/// backfilled (they have no bindings and no meaningful direct link).
fn is_sentinel_identity(identity_id: &str) -> bool {
    identity_id.is_empty() || identity_id == "blank"
}

impl Migration for M0013AgentDirectBindings {
    fn id(&self) -> &'static str { "0013_agent_direct_bindings" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Backfill direct agent↔account links from existing bundle bindings"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("agent_direct_bindings: open shared: {}", e)))?;

        // Collect the channel stores that hold `db_agent_instances`.
        // The shared store has no instances table, so — like m0011 — we
        // read them from the current channel store plus any siblings.
        let mut sibling_stores: Vec<Store> = Vec::new();
        if ctx.channel_store_path.exists() {
            match Store::open_source_readonly(&ctx.channel_store_path) {
                Ok(s) => sibling_stores.push(s),
                Err(e) => tracing::debug!(
                    path = %ctx.channel_store_path.display(),
                    error = %e,
                    "agent_direct_bindings: skip current channel store"
                ),
            }
        }
        for path in registry::enumerate_objects_dbs(&ctx.home) {
            if path == ctx.channel_store_path { continue; }
            match Store::open_source_readonly(&path) {
                Ok(s) => sibling_stores.push(s),
                Err(e) => tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "agent_direct_bindings: skip sibling"
                ),
            }
        }

        backfill_direct_links(&shared, &sibling_stores)
    }
}

/// Core backfill. Extracted so tests can drive it against an in-memory
/// shared store + in-memory instance sources without a full
/// `MigrationContext` / on-disk channel tree.
///
/// `instance_sources` supplies the `db_agent_instances` rows (from the
/// per-channel object stores); `shared` supplies the bundle bindings and
/// receives the direct links.
pub(crate) fn backfill_direct_links(
    shared: &Store,
    instance_sources: &[Store],
) -> Result<(), MigrationError> {
    // Gather every instance across all sources, deterministically
    // ordered so that when several instances of one definition point at
    // DIFFERENT bundles the last write (highest created_at, then id) is
    // reproducible run-to-run.
    let mut instances: Vec<crate::backend::storage::store::AgentInstance> = Vec::new();
    for src in instance_sources {
        match src.instance_list(None, None) {
            Ok(list) => instances.extend(list),
            // Don't silently drop instances on a query error — a partial read
            // would leave the direct-binding backfill quietly incomplete (some
            // agents never get their credentials migrated). Log loudly so the
            // gap is visible; the dual-read resolver still falls back to the
            // bundle path for any un-backfilled instance, so this is not fatal.
            Err(e) => tracing::warn!(
                "m0013: instance_list failed for a source store; those instances \
                 will not be backfilled to direct links (bundle fallback still \
                 applies): {e}"
            ),
        }
    }
    instances.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });

    // Track the winning (definition_id, provider) → identity_id so we can
    // log collisions where two instances of the same definition resolve
    // the same provider from different bundles.
    let mut winner: HashMap<(String, String), String> = HashMap::new();
    let mut written = 0usize;
    let mut collisions = 0usize;

    for inst in &instances {
        if is_sentinel_identity(&inst.identity_id) {
            continue;
        }
        if inst.definition_id.is_empty() {
            continue;
        }

        let bindings = shared
            .bundle_identity_bindings(&inst.identity_id)
            .unwrap_or_default();

        for b in bindings {
            let key = (inst.definition_id.clone(), b.provider.clone());
            if let Some(prev_identity) = winner.get(&key) {
                if prev_identity != &inst.identity_id {
                    collisions += 1;
                    tracing::warn!(
                        target: "identity",
                        definition_id = %inst.definition_id,
                        provider = %b.provider,
                        previous_identity = %prev_identity,
                        winning_identity = %inst.identity_id,
                        "agent_direct_bindings: (definition, provider) collision — \
                         last write (higher created_at) wins",
                    );
                }
            }
            // Upsert (ON CONFLICT(agent_id, provider) DO UPDATE) — the
            // deterministic ordering above makes the final winner stable.
            shared
                .agent_identity_link(&inst.definition_id, &b.account_id, &b.provider)
                .map_err(|e| MigrationError(format!(
                    "agent_direct_bindings: link def {} provider {}: {}",
                    inst.definition_id, b.provider, e
                )))?;
            winner.insert(key, inst.identity_id.clone());
            written += 1;
        }
    }

    tracing::info!(
        written,
        collisions,
        instances = instances.len(),
        "agent_direct_bindings: backfill complete"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{
        AgentDefinition, AgentInstance, Identity, IdentityAccount, SecretRef, Store,
    };

    fn shared_store() -> Store {
        // A shared-schema store built on a temp file (open_shared needs a
        // real path; :memory: would be re-created per connection).
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    /// Channel store (objects.db schema — has db_agent_instances). Used
    /// as an instance source for the backfill.
    fn channel_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn make_def(store: &Store, id: &str) {
        let mut def = AgentDefinition {
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

    fn make_instance(
        store: &Store,
        id: &str,
        definition_id: &str,
        identity_id: &str,
        created_at: i64,
    ) {
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

    #[test]
    fn backfills_bundle_bindings_into_direct_links() {
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "bundle-work");
        make_account(&shared, "acct-gh", "github");
        make_account(&shared, "acct-anth", "anthropic");
        shared.bundle_identity_bind("bundle-work", "github", "acct-gh").unwrap();
        shared.bundle_identity_bind("bundle-work", "anthropic", "acct-anth").unwrap();
        make_instance(&channel, "inst-1", "def-1", "bundle-work", 0);

        backfill_direct_links(&shared, &[channel]).unwrap();

        let mut links = shared.agent_identity_list_for_agent("def-1").unwrap();
        links.sort_by(|a, b| a.provider.cmp(&b.provider));
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].provider, "anthropic");
        assert_eq!(links[0].account_id, "acct-anth");
        assert_eq!(links[0].agent_id, "def-1");
        assert_eq!(links[1].provider, "github");
        assert_eq!(links[1].account_id, "acct-gh");
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

        backfill_direct_links(&shared, std::slice::from_ref(&channel)).unwrap();
        backfill_direct_links(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-1").unwrap();
        assert_eq!(links.len(), 1, "re-run must not duplicate links");
        assert_eq!(links[0].account_id, "acct-gh");
    }

    #[test]
    fn skips_blank_and_empty_identity_instances() {
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-blank");
        make_def(&channel, "def-empty");
        make_instance(&channel, "inst-blank", "def-blank", "blank", 0);
        make_instance(&channel, "inst-empty", "def-empty", "", 0);

        backfill_direct_links(&shared, &[channel]).unwrap();

        assert!(shared.agent_identity_list_for_agent("def-blank").unwrap().is_empty());
        assert!(shared.agent_identity_list_for_agent("def-empty").unwrap().is_empty());
        assert!(shared.agent_identity_list_all().unwrap().is_empty());
    }

    #[test]
    fn default_oauth_bundle_is_backfilled() {
        // The `default` bundle carries real OAuth bindings and MUST be
        // copied — only '' / 'blank' are skipped.
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "default");
        let claude = IdentityAccount {
            id: "acct-claude".to_string(),
            name: "claude".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/default/claude".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        shared.identity_upsert(&claude).unwrap();
        shared.bundle_identity_bind("default", "claude", "acct-claude").unwrap();
        make_instance(&channel, "inst-1", "def-1", "default", 0);

        backfill_direct_links(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, "claude");
        assert_eq!(links[0].account_id, "acct-claude");
    }

    #[test]
    fn collision_last_write_wins_deterministically() {
        // Two instances of one definition on different bundles binding
        // the SAME provider → the (agent_id, provider) PK means the
        // higher-created_at instance wins, reproducibly.
        let shared = shared_store();
        let channel = channel_store();

        make_def(&channel, "def-1");
        make_bundle(&shared, "bundle-old");
        make_bundle(&shared, "bundle-new");
        make_account(&shared, "acct-old", "github");
        make_account(&shared, "acct-new", "github");
        shared.bundle_identity_bind("bundle-old", "github", "acct-old").unwrap();
        shared.bundle_identity_bind("bundle-new", "github", "acct-new").unwrap();
        // Higher created_at → sorts last → wins.
        make_instance(&channel, "inst-old", "def-1", "bundle-old", 100);
        make_instance(&channel, "inst-new", "def-1", "bundle-new", 200);

        backfill_direct_links(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].account_id, "acct-new", "highest created_at wins");
    }
}
