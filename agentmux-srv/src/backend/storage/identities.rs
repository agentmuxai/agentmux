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

    pub fn identity_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_accounts WHERE id = ?1", params![id])?;
        Ok(rows > 0)
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
