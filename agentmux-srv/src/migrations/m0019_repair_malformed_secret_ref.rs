// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Repair `db_accounts.secret_ref` rows whose JSON was corrupted by an
//! out-of-band edit of the shared store — found live: a raw Windows path
//! pasted into the JSON string with its backslashes never escaped
//! (`{"backend":"oauth_config_dir","dir":"C:\Users\..."}`, valid JSON needs
//! `\\` per separator). Not reproducible from any write path in this
//! codebase (`Store::identity_upsert` always `serde_json::to_string`s
//! correctly) — see
//! `docs/analysis/ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04.md`
//! §7 for the full incident.
//!
//! Global-scoped (touches the shared store directly, like
//! `m0012_dedup_identity_accounts` and `m0018_ambient_login_registry`). Safe
//! to re-run: `identity_repair_malformed_secret_refs` is a no-op once every
//! row parses.
//!
//! This migration is a data repair, not a substitute for
//! `identity_list`/`identity_get`'s own per-row tolerance (see the same
//! analysis doc's §6 option 1, implemented alongside this migration in
//! `backend/storage/identities.rs`) — the two are independent defenses:
//! this fixes the one known-bad row; that keeps any future corruption from
//! ever again hiding every other account.

use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0019RepairMalformedSecretRef;

impl Migration for M0019RepairMalformedSecretRef {
    fn id(&self) -> &'static str { "0019_repair_malformed_secret_ref" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Repair db_accounts.secret_ref rows with unescaped-backslash JSON corruption"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("repair_malformed_secret_ref: open shared store: {}", e)))?;
        let repaired = shared
            .identity_repair_malformed_secret_refs()
            .map_err(|e| MigrationError(format!("repair_malformed_secret_ref: {}", e)))?;
        tracing::info!(
            target: "identity",
            repaired,
            "identity.repair: secret_ref repair pass — {} row(s) fixed",
            repaired,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{IdentityAccount, SecretRef};

    fn ctx_for(shared: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: shared.to_path_buf(),
            channel_store_path: std::env::temp_dir().join("agentmux-test-unused-channel.db"),
        }
    }

    #[test]
    fn repairs_a_malformed_row_in_the_shared_store() {
        let shared = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open_shared(shared.path()).unwrap();
        store
            .identity_upsert(&IdentityAccount {
                id: "acct-good".to_string(),
                name: "claude-good".to_string(),
                provider: "claude".to_string(),
                kind: "oauth".to_string(),
                display_name: String::new(),
                secret_ref: SecretRef::OAuthConfigDir { dir: "/tmp/good".to_string() },
                context: serde_json::json!({}),
                status: "ok".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        {
            let conn = store.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_accounts
                    (id, name, provider, kind, display_name, secret_ref, context,
                     status, created_at, updated_at)
                 VALUES ('acct-broken', 'broken', 'claude', 'oauth', '',
                         '{\"backend\":\"oauth_config_dir\",\"dir\":\"C:\\Users\\a\\claude\"}',
                         '{}', 'unknown', 0, 0)",
                [],
            )
            .unwrap();
        }
        drop(store);

        M0019RepairMalformedSecretRef.up(&ctx_for(shared.path())).unwrap();

        let store = Store::open_shared(shared.path()).unwrap();
        let accounts = store.identity_list(None).unwrap();
        assert_eq!(accounts.len(), 2, "both the already-good and the repaired row must be visible");
        assert!(accounts.iter().any(|a| a.id == "acct-broken"));
    }

    #[test]
    fn missing_shared_store_is_a_noop() {
        let missing = std::env::temp_dir().join("agentmux-test-shared-definitely-missing-r7z.db");
        let _ = std::fs::remove_file(&missing);
        M0019RepairMalformedSecretRef.up(&ctx_for(&missing)).unwrap();
        assert!(!missing.exists(), "up() must not create the store");
    }
}
