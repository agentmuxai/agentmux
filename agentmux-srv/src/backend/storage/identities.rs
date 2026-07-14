// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity subsystem — accounts and the direct junction that ties
//! them to agents.
//!
//! Layered model:
//! - **`IdentityAccount`** (`db_accounts`): a single credential
//!   pointer (provider + kind + secret_ref).
//! - **`AgentIdentityLink`** (`db_agent_identity_links`): the sole
//!   credential-resolution path — binds an agent (definition) to an
//!   account directly, per provider.
//!
//! The Identity *bundle* layer (`Identity`/`IdentityBinding`,
//! `db_identity_bundles`/`db_identity_bindings`) was retired in Phase 4c
//! of `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` — direct links had
//! already fully superseded it as the credential-resolution path (see
//! `identity/resolver.rs::resolve_bindings_for_instance`).
//!
//! Extracted from `store.rs` in Phase R.2 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::identity_*` and
//! `agent_identity_*` still live on `Store` via this `impl` block;
//! existing imports of the structs from `storage::store::*` keep
//! working thanks to re-exports.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// Provider-specific credential reference. Stored as JSON in
/// `db_accounts.secret_ref`. `backend` is the discriminator.
/// The actual secret value is NEVER stored in the DB — only how to
/// look it up at launch time (env var, secrets-manager path, etc.).
/// `PlaintextDev` exists for local dev convenience and must never be
/// the default path in production builds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum SecretRef {
    Env {
        env_var: String,
    },
    SecretsManager {
        sm_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sm_json_path: Option<String>,
    },
    PlaintextDev {
        plaintext_dev: String,
    },
    /// **OAuth credentials stored as a filesystem pointer.** The CLI
    /// (Claude Code, codex, openclaw, …) reads its OAuth tokens from
    /// this directory at spawn time — agentmux only holds the path,
    /// never the tokens themselves. Token refresh is the CLI's job;
    /// the path stays stable across refreshes. Used by oauth-class
    /// providers; the resolver (PR B) dispatches to a config-dir
    /// env-var injection mode rather than the api-key env-var path.
    /// See `docs/specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`.
    OAuthConfigDir {
        /// Absolute path to the per-bundle, per-provider config
        /// directory — e.g. `~/.agentmux/shared/identities/<id>/claude/`,
        /// or the legacy `~/.claude/` for the Default migration bundle
        /// (PR E).
        dir: String,
    },
    /// **API key / token stored in the OS-native secret store** (macOS
    /// Keychain / Windows Credential Manager / Linux Secret Service),
    /// addressed by `(service, account)`. The plaintext is NEVER held in
    /// the DB — only this pointer. Written by the Armory key flow
    /// after a successful live validation; resolved to the real value at
    /// agent spawn time via `crate::identity::secret_store`. See
    /// specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §7/§12.2.
    Keychain {
        /// Keychain service string — always `"agentmux"`.
        service: String,
        /// Keychain account string — `"acct:<account_id>"`.
        account: String,
    },
}

/// An identity account (reusable credential, linked to agents via the
/// `db_agent_identity_links` junction). Replaces the browser
/// localStorage identity store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityAccount {
    pub id: String,
    pub name: String,
    pub provider: String, // "github" | "aws" | "anthropic" | "custom"
    pub kind: String,     // "pat" | "role" | "api_key" | "env_ref"
    #[serde(default)]
    pub display_name: String,
    pub secret_ref: SecretRef,
    /// Free-form JSON context (username, scopes, role ARN, etc.). Stored
    /// verbatim; frontend types it by `provider`.
    #[serde(default = "default_context_json")]
    pub context: serde_json::Value,
    #[serde(default = "default_identity_status")]
    pub status: String, // "unknown" | "ok" | "expired" | "invalid"
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_context_json() -> serde_json::Value {
    serde_json::json!({})
}

fn default_identity_status() -> String {
    "unknown".to_string()
}

/// Junction row: which identity an agent uses for a given provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityLink {
    pub agent_id: String,
    pub account_id: String,
    pub provider: String,
}

/// What [`Store::identity_delete`] removed — the account row plus the
/// cascaded `db_agent_identity_links` junction rows. Consumed by the
/// `deleteidentityaccount` handler for `identity.delete:` logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityDeleteOutcome {
    /// True iff the `db_accounts` row existed and was deleted.
    pub deleted: bool,
    /// Number of `db_agent_identity_links` rows removed in the same
    /// transaction.
    pub links_cascaded: usize,
}

impl Store {
    // ---- Identity account CRUD ----

    /// List identity accounts. If `provider` is `Some`, filter to that
    /// provider; otherwise return every account, ordered by most recent
    /// update first (so the identity panel shows live accounts on top).
    pub fn identity_list(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<IdentityAccount>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut rows_vec = Vec::new();
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<IdentityAccount> {
            let secret_ref_json: String = row.get(5)?;
            let context_json: String = row.get(6)?;
            Ok(IdentityAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                display_name: row.get(4)?,
                secret_ref: serde_json::from_str(&secret_ref_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                context: serde_json::from_str(&context_json)
                    .unwrap_or_else(|_| serde_json::json!({})),
                status: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        };
        match provider {
            Some(p) => {
                let mut stmt = conn.prepare(
                    "SELECT id, name, provider, kind, display_name, secret_ref, context,
                            status, created_at, updated_at
                     FROM db_accounts
                     WHERE provider = ?1
                     ORDER BY updated_at DESC",
                )?;
                let iter = stmt.query_map(params![p], map_row)?;
                for r in iter {
                    rows_vec.push(r?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, name, provider, kind, display_name, secret_ref, context,
                            status, created_at, updated_at
                     FROM db_accounts
                     ORDER BY updated_at DESC",
                )?;
                let iter = stmt.query_map([], map_row)?;
                for r in iter {
                    rows_vec.push(r?);
                }
            }
        }
        Ok(rows_vec)
    }

    pub fn identity_get(&self, id: &str) -> Result<Option<IdentityAccount>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, provider, kind, display_name, secret_ref, context,
                    status, created_at, updated_at
             FROM db_accounts WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            let secret_ref_json: String = row.get(5)?;
            let context_json: String = row.get(6)?;
            Ok(IdentityAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                display_name: row.get(4)?,
                secret_ref: serde_json::from_str(&secret_ref_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                context: serde_json::from_str(&context_json)
                    .unwrap_or_else(|_| serde_json::json!({})),
                status: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        });
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert an identity account. If `account.id` is empty the caller
    /// must generate one first (we don't silently mint ids here — callers
    /// should know whether they're creating vs updating).
    pub fn identity_upsert(&self, account: &IdentityAccount) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let secret_ref_json = serde_json::to_string(&account.secret_ref)?;
        let context_json = serde_json::to_string(&account.context)?;
        conn.execute(
            "INSERT INTO db_accounts
                (id, name, provider, kind, display_name, secret_ref, context,
                 status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                provider = excluded.provider,
                kind = excluded.kind,
                display_name = excluded.display_name,
                secret_ref = excluded.secret_ref,
                context = excluded.context,
                status = excluded.status,
                updated_at = excluded.updated_at",
            params![
                account.id,
                account.name,
                account.provider,
                account.kind,
                account.display_name,
                secret_ref_json,
                context_json,
                account.status,
                account.created_at,
                account.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Delete an identity account AND every `db_agent_identity_links`
    /// row referencing it, atomically (single transaction under the
    /// connection lock).
    ///
    /// The cascade is EXPLICIT even though the current DDL carries
    /// `FOREIGN KEY (account_id) … ON DELETE CASCADE`: databases whose
    /// links table arrived via legacy `ALTER TABLE … RENAME` (forge era)
    /// never got the FK clause retrofitted — `CREATE TABLE IF NOT EXISTS`
    /// can't alter an existing table — so on real user databases the
    /// DDL-level cascade silently doesn't exist and dangling links
    /// survive the account row (auth-lifecycle gap §2.4,
    /// `docs/analysis/ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md`).
    /// `db_identity_bindings` (named by the report) was retired in Phase
    /// 4c and no longer exists; links are the sole junction on accounts.
    pub fn identity_delete(&self, id: &str) -> Result<IdentityDeleteOutcome, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Links first, so the count is exact even on fresh databases
        // where the FK cascade would otherwise race this delete.
        let links_cascaded = tx.execute(
            "DELETE FROM db_agent_identity_links WHERE account_id = ?1",
            params![id],
        )?;
        let rows = tx.execute("DELETE FROM db_accounts WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(IdentityDeleteOutcome {
            deleted: rows > 0,
            links_cascaded,
        })
    }

    // ---- Agent ↔ Identity junction ----

    /// Link an agent to an identity for a given provider. Overwrites any
    /// existing link for the same (agent_id, provider) — each agent has
    /// at most one account per provider.
    pub fn agent_identity_link(
        &self,
        agent_id: &str,
        account_id: &str,
        provider: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_identity_links (agent_id, account_id, provider)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, provider) DO UPDATE SET account_id = excluded.account_id",
            params![agent_id, account_id, provider],
        )?;
        Ok(())
    }

    /// Remove the identity link for a given (agent_id, provider).
    /// Returns true iff a link existed.
    pub fn agent_identity_unlink(
        &self,
        agent_id: &str,
        provider: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_agent_identity_links WHERE agent_id = ?1 AND provider = ?2",
            params![agent_id, provider],
        )?;
        Ok(rows > 0)
    }

    /// List every (agent_id, account_id, provider) row in the table.
    /// Used by the startup backfill to seed the shared store.
    pub fn agent_identity_list_all(&self) -> Result<Vec<AgentIdentityLink>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, account_id, provider FROM db_agent_identity_links ORDER BY agent_id, provider",
        )?;
        let iter = stmt.query_map([], |row| {
            Ok(AgentIdentityLink {
                agent_id: row.get(0)?,
                account_id: row.get(1)?,
                provider: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// List all (agent_id, account_id, provider) triples for an agent.
    pub fn agent_identity_list_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentIdentityLink>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, account_id, provider
             FROM db_agent_identity_links
             WHERE agent_id = ?1
             ORDER BY provider",
        )?;
        let iter = stmt.query_map(params![agent_id], |row| {
            Ok(AgentIdentityLink {
                agent_id: row.get(0)?,
                account_id: row.get(1)?,
                provider: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::super::store::{AgentDefinition, Store};
    use super::*;

    fn sample_account(id: &str, provider: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.to_string(),
            name: format!("asaf-{provider}"),
            provider: provider.to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir {
                dir: format!("/var/agentmux/identities/{id}/claude"),
            },
            context: serde_json::json!({}),
            status: "ok".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_agent(id: &str, slug: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: slug.to_string(),
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
            agent_type: "host".to_string(),
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
        }
    }

    fn count_links_for_account(store: &Store, account_id: &str) -> i64 {
        let conn = store.conn.lock().unwrap();
        conn.query_row(
            "SELECT count(*) FROM db_agent_identity_links WHERE account_id = ?1",
            params![account_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// Recreate `db_agent_identity_links` in the LEGACY shape: no
    /// `FOREIGN KEY (account_id) … ON DELETE CASCADE`. This is the shape a
    /// production `store.db`/`objects.db` still has when the table arrived
    /// via `ALTER TABLE … RENAME` from the forge era —
    /// `CREATE TABLE IF NOT EXISTS` never retrofits FK clauses onto an
    /// existing table, so the DDL-level cascade only exists on fresh
    /// databases. `identity_delete` must therefore cascade EXPLICITLY
    /// rather than lean on the FK (the account-delete auth-lifecycle gap,
    /// docs/analysis/ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md §2.4).
    fn strip_links_fk(store: &Store) {
        let conn = store.conn.lock().unwrap();
        conn.execute_batch(
            "DROP TABLE db_agent_identity_links;
             CREATE TABLE db_agent_identity_links (
                agent_id   TEXT NOT NULL,
                account_id TEXT NOT NULL,
                provider   TEXT NOT NULL,
                PRIMARY KEY (agent_id, provider)
             );",
        )
        .unwrap();
    }

    #[test]
    fn identity_delete_cascades_links_on_legacy_no_fk_schema() {
        let store = Store::open_in_memory().unwrap();
        strip_links_fk(&store);
        store
            .identity_upsert(&sample_account("acct-1", "anthropic"))
            .unwrap();
        store
            .agent_identity_link("ag1", "acct-1", "anthropic")
            .unwrap();
        assert_eq!(count_links_for_account(&store, "acct-1"), 1);

        let out = store.identity_delete("acct-1").unwrap();

        assert!(store.identity_get("acct-1").unwrap().is_none());
        assert_eq!(
            count_links_for_account(&store, "acct-1"),
            0,
            "junction row must not survive the account row (dangling link)"
        );
        assert!(out.deleted);
        assert_eq!(out.links_cascaded, 1, "explicit cascade must report the link row");

        // Second delete is a no-op on both tables.
        let again = store.identity_delete("acct-1").unwrap();
        assert!(!again.deleted);
        assert_eq!(again.links_cascaded, 0);
    }

    #[test]
    fn identity_delete_cascades_links_on_current_schema() {
        // Fresh schema — the DDL-level FK cascade also exists here; the
        // explicit cascade must coexist with it (and still report the row).
        let store = Store::open_in_memory().unwrap();
        let mut agent = sample_agent("ag1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();
        store
            .identity_upsert(&sample_account("acct-1", "anthropic"))
            .unwrap();
        store
            .agent_identity_link("ag1", "acct-1", "anthropic")
            .unwrap();

        let out = store.identity_delete("acct-1").unwrap();

        assert!(store.identity_get("acct-1").unwrap().is_none());
        assert_eq!(count_links_for_account(&store, "acct-1"), 0);
        assert!(out.deleted);
        assert_eq!(
            out.links_cascaded, 1,
            "explicit cascade runs before the FK cascade, so the count is exact"
        );
    }

    /// The analysis report (§2.4) names `db_identity_bindings` as a second
    /// cascade target. That table was retired in Phase 4c of
    /// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md and is DROPPED by the
    /// schema (`DEAD_TABLE_DROPS`) — `db_agent_identity_links` is the sole
    /// junction referencing `db_accounts.id`. Pin that here so the cascade
    /// in `identity_delete` is provably complete.
    #[test]
    fn identity_bindings_table_is_retired() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type='table' AND name IN ('db_identity_bindings', 'db_identity_bundles')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "retired bundle-binding tables must not exist");
    }
}
