// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Grandfather the layer-3 `use_ambient_login` opt-in for pre-existing
//! agents (spec §2.4 of SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md).
//!
//! The spawn gate (identity/resolver.rs) now FAILS a spawn when an
//! oauth-class provider the agent is supposed to have credentials for has
//! no resolvable account — unless the agent carries an explicit
//! `use_ambient_login = 1`. Existing agents that relied on the (previously
//! silent) ambient fallback must not all break on upgrade:
//!
//! - agents with NO **oauth-class** `db_agent_identity_links` rows at
//!   migration time were de-facto ambient users for their CLI login →
//!   `use_ambient_login = 1`. Api-key-class links (e.g. a github PAT) do
//!   NOT count: they are never spawn-gated, so an agent whose only link is
//!   a PAT was still relying on the ambient CLI login and must be
//!   grandfathered, not broken (spec §2.4's rationale — "grandfather
//!   de-facto ambient users" — keyed on the spawn-relevant provider class);
//! - agents WITH an oauth-class link opted into managed CLI accounts →
//!   `0` (honest failure is the correct new behavior for them).
//!
//! Channel-scoped: each pre-existing channel's `db_agent_definitions` /
//! `db_agents` rows get the pass exactly once (fresh channels have no
//! agent rows, so re-running on a new channel is a no-op — it can never
//! flip an agent created after the upgrade). The live links table is the
//! SHARED store's; the channel store's legacy copy is not consulted.
//! Cross-channel registry records are handled by the Global-scoped
//! `m0018_ambient_login_registry`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

/// Agent ids whose links include at least one **oauth-class** provider —
/// the only class the layer-3 spawn gate blocks on. Shared by m0017
/// (channel rows) and m0018 (registry records) so the two passes can never
/// disagree on the rule.
pub(crate) fn oauth_linked_agent_ids(
    shared: &Store,
) -> Result<HashSet<String>, crate::backend::storage::error::StoreError> {
    use crate::identity::resolver::{provider_class, ProviderClass};
    Ok(shared
        .agent_identity_link_provider_pairs()?
        .into_iter()
        .filter(|(_, provider)| matches!(provider_class(provider), Some(ProviderClass::OAuth { .. })))
        .map(|(agent_id, _)| agent_id)
        .collect())
}

pub struct M0017AmbientLoginGrandfather;

impl Migration for M0017AmbientLoginGrandfather {
    fn id(&self) -> &'static str { "0017_ambient_login_grandfather" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Grandfather use_ambient_login=1 for agents without identity links (layer-3 spawn gating)"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("ambient_login_grandfather: open wstore: {}", e)))?,
        );
        // The live links live in the shared store. A missing shared store
        // (fresh install racing the bootstrap) means no links exist —
        // every agent row (if any) is a de-facto ambient user.
        let linked: HashSet<String> = if ctx.shared_store_path.exists() {
            let shared = Store::open_shared(&ctx.shared_store_path)
                .map_err(|e| MigrationError(format!("ambient_login_grandfather: open shared store: {}", e)))?;
            oauth_linked_agent_ids(&shared)
                .map_err(|e| MigrationError(format!("ambient_login_grandfather: read links: {}", e)))?
        } else {
            HashSet::new()
        };
        let (ambient, managed) = wstore
            .agents_grandfather_ambient_login(&linked)
            .map_err(|e| MigrationError(format!("ambient_login_grandfather: {}", e)))?;
        tracing::info!(
            target: "identity",
            ambient,
            managed,
            "identity.spawn: ambient-login grandfather — {} linkless agent(s) set to ambient, {} linked agent(s) kept fail-by-default",
            ambient,
            managed,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::AgentDefinition;

    fn ctx_for(channel: &std::path::Path, shared: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: shared.to_path_buf(),
            channel_store_path: channel.to_path_buf(),
        }
    }

    fn make_def(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: id.to_string(),
            name: id.to_string(),
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
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
        }
    }

    /// Spec §2.5 migration test: linkless agent → flag true; linked agent →
    /// flag false. Links live in the SHARED store; the flag lands on the
    /// channel store's rows (both tables).
    #[test]
    fn linkless_agent_gets_ambient_linked_agent_stays_fail_by_default() {
        let channel = tempfile::NamedTempFile::new().unwrap();
        let shared = tempfile::NamedTempFile::new().unwrap();

        // Channel store: two user agents.
        let wstore = Store::open(channel.path()).unwrap();
        let mut linkless = make_def("agent-linkless");
        wstore.agent_def_insert(&mut linkless).unwrap();
        let mut linked = make_def("agent-linked");
        wstore.agent_def_insert(&mut linked).unwrap();

        // Shared store: one link for agent-linked. The account row is not
        // required for the grandfather decision (only the link's presence),
        // and links-without-accounts are exactly the post-delete shape.
        let shared_store = Store::open_shared(shared.path()).unwrap();
        let acct = crate::backend::storage::store::IdentityAccount {
            id: "acct-1".to_string(),
            name: "claude-acct-1".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir {
                dir: "/tmp/nowhere".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        shared_store.identity_upsert(&acct).unwrap();
        shared_store
            .agent_identity_link("agent-linked", "acct-1", "claude")
            .unwrap();
        drop(shared_store);
        drop(wstore);

        M0017AmbientLoginGrandfather
            .up(&ctx_for(channel.path(), shared.path()))
            .unwrap();

        let wstore = Store::open(channel.path()).unwrap();
        let after_linkless = wstore.agent_def_get("agent-linkless").unwrap().unwrap();
        assert_eq!(
            after_linkless.use_ambient_login, 1,
            "linkless agent must be grandfathered to ambient"
        );
        let after_linked = wstore.agent_def_get("agent-linked").unwrap().unwrap();
        assert_eq!(
            after_linked.use_ambient_login, 0,
            "linked agent must keep fail-by-default"
        );
        // The consolidated db_agents projection agrees (it's what the
        // roster/modal read).
        let listed = wstore.agent_def_list().unwrap();
        assert_eq!(
            listed.iter().find(|a| a.id == "agent-linkless").unwrap().use_ambient_login,
            1,
        );
        assert_eq!(
            listed.iter().find(|a| a.id == "agent-linked").unwrap().use_ambient_login,
            0,
        );
    }

    /// Spec §2.4 (as clarified): only OAUTH-CLASS links forfeit
    /// grandfathering. An agent whose only link is an api-key-class github
    /// PAT was still a de-facto ambient user for its CLI login — the spawn
    /// gate never blocks on api-key providers, so counting that link would
    /// break exactly the users grandfathering exists to protect.
    #[test]
    fn api_key_only_links_do_not_forfeit_grandfathering() {
        let channel = tempfile::NamedTempFile::new().unwrap();
        let shared = tempfile::NamedTempFile::new().unwrap();

        let wstore = Store::open(channel.path()).unwrap();
        let mut pat_only = make_def("agent-pat-only");
        wstore.agent_def_insert(&mut pat_only).unwrap();
        drop(wstore);

        // Shared store: a github (api-key-class) link and nothing else.
        let shared_store = Store::open_shared(shared.path()).unwrap();
        let acct = crate::backend::storage::store::IdentityAccount {
            id: "acct-gh".to_string(),
            name: "asaf-github".to_string(),
            provider: "github".to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::store::SecretRef::Keychain {
                service: "agentmux".to_string(),
                account: "acct-gh".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        shared_store.identity_upsert(&acct).unwrap();
        shared_store
            .agent_identity_link("agent-pat-only", "acct-gh", "github")
            .unwrap();
        drop(shared_store);

        M0017AmbientLoginGrandfather
            .up(&ctx_for(channel.path(), shared.path()))
            .unwrap();

        let wstore = Store::open(channel.path()).unwrap();
        assert_eq!(
            wstore.agent_def_get("agent-pat-only").unwrap().unwrap().use_ambient_login,
            1,
            "a PAT-only agent is a de-facto ambient CLI user and must be grandfathered"
        );
    }

    #[test]
    fn missing_shared_store_treats_every_agent_as_linkless() {
        let channel = tempfile::NamedTempFile::new().unwrap();
        let wstore = Store::open(channel.path()).unwrap();
        let mut def = make_def("agent-solo");
        wstore.agent_def_insert(&mut def).unwrap();
        drop(wstore);

        let missing_shared = std::env::temp_dir()
            .join("agentmux-test-shared-store-definitely-missing-x9q.db");
        let _ = std::fs::remove_file(&missing_shared);
        M0017AmbientLoginGrandfather
            .up(&ctx_for(channel.path(), &missing_shared))
            .unwrap();

        let wstore = Store::open(channel.path()).unwrap();
        assert_eq!(
            wstore.agent_def_get("agent-solo").unwrap().unwrap().use_ambient_login,
            1,
        );
    }

    #[test]
    fn missing_channel_store_is_a_noop() {
        let shared = tempfile::NamedTempFile::new().unwrap();
        let missing_channel = std::env::temp_dir()
            .join("agentmux-test-channel-store-definitely-missing-x9q.db");
        let _ = std::fs::remove_file(&missing_channel);
        M0017AmbientLoginGrandfather
            .up(&ctx_for(&missing_channel, shared.path()))
            .unwrap();
        assert!(!missing_channel.exists(), "up() must not create the store");
    }
}
