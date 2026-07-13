// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Re-run the direct agent↔account link backfill to catch instances
//! launched in the gap between `m0013`'s original run and the launch flow
//! write-through landing (PR B1, #1950) — historical.
//!
//! `m0013` backfills `db_agent_identity_links` from bundle bindings, but
//! every migration runs AT MOST ONCE per install (guarded by
//! `db_migrations`, see `migrations/mod.rs`). Any agent launched after
//! `m0013` ran on a given install, but before PR B1 shipped, would have a
//! bundle-based `identity_id` and NO direct link — `m0013` never sees it
//! because it already ran. This migration only ever looked at the SINGLE
//! latest instance per definition — the one whose bundle pick (or
//! "blank") was the actual current intent — and only ADDED/UPDATED links
//! implied by it (never unlinked, since `identity.account.upsert` also
//! writes `db_agent_identity_links` independent of any bundle or launch).
//!
//! **Retired.** `db_identity_bundles`/`db_identity_bindings` (the source
//! this read from) were dropped in Phase 4c of
//! SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md, once this migration had
//! already run on every real install. The migration id stays registered
//! (already-applied installs must never re-run a Global migration) but
//! `backfill_latest_instance_only`'s body is now a documented no-op.

use crate::backend::storage::store::Store;
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

    fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
        Ok(())
    }
}

/// Core backfill — historical, see module doc comment. Signature kept so
/// the (now trivial) test below still exercises the same call shape.
pub(crate) fn backfill_latest_instance_only(
    _shared: &Store,
    _instance_sources: &[Store],
) -> Result<(), MigrationError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    fn channel_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// The retired backfill is a documented no-op now — this just pins
    /// that a manually-configured direct link (unrelated to any bundle
    /// or instance) survives untouched, matching this migration's
    /// original "only adds, never unlinks" contract.
    #[test]
    fn backfill_is_a_harmless_no_op() {
        use crate::backend::storage::store::{IdentityAccount, SecretRef};

        let shared = shared_store();
        let channel = channel_store();

        shared
            .identity_upsert(&IdentityAccount {
                id: "acct-manual".to_string(),
                name: "manual".to_string(),
                provider: "openclaw".to_string(),
                kind: "pat".to_string(),
                display_name: String::new(),
                secret_ref: SecretRef::Env { env_var: "VAR".to_string() },
                context: serde_json::json!({}),
                status: "unknown".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        shared.agent_identity_link("def-manual", "acct-manual", "openclaw").unwrap();

        backfill_latest_instance_only(&shared, &[channel]).unwrap();

        let links = shared.agent_identity_list_for_agent("def-manual").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, "openclaw");
    }
}
