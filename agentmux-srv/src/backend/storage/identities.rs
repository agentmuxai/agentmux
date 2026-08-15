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
    ///
    /// Explicit `rename` — the enum-wide `rename_all = "snake_case"` splits
    /// at every capital letter, including inside "OAuth" itself, producing
    /// `o_auth_config_dir` (verified: that's what a live deserialization
    /// failure actually expected). Every real account on disk was written
    /// with `oauth_config_dir` (no extra underscore — how "OAuth" is
    /// conventionally spelled), so the derived name silently stopped
    /// matching stored data — this makes the wire format explicit instead
    /// of relying on the derive macro to guess right for an acronym.
    ///
    /// The `alias` keeps accepting the derive-produced `o_auth_config_dir`
    /// tag too: any account persisted by a backend running between this
    /// variant's introduction and this fix would have round-tripped
    /// self-consistently under that buggy tag, and `identity_list()` aborts
    /// entirely on the first unparseable row — so dropping read support for
    /// it would just move the "My Agents empty" incident to a different set
    /// of accounts. Serialization always writes the canonical
    /// `oauth_config_dir`; only deserialization accepts both.
    #[serde(rename = "oauth_config_dir", alias = "o_auth_config_dir")]
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
    // `#[serde(default)]` — the `upsertidentityaccount` handler (agent_handlers/
    // identity.rs) server-stamps both fields with `now` when they're `0`, but
    // that logic runs AFTER `serde_json::from_value` succeeds. Every real
    // frontend caller (persistSeededAccount, register-seeded-account.ts) omits
    // both fields entirely, expecting the server to fill them in — without
    // `default` here, deserialization rejects the payload before the handler's
    // own timestamp logic ever runs, so every seed-from-global / terminal
    // login's account-persist call failed 100% of the time with no visible
    // error (report: REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md).
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_context_json() -> serde_json::Value {
    serde_json::json!({})
}

fn default_identity_status() -> String {
    "unknown".to_string()
}

/// Escape every backslash in `raw` unconditionally. Targets exactly the one
/// corruption shape seen in production — a raw Windows path (`C:\Users\...`)
/// pasted into a JSON string with NO escaping at all — so every backslash in
/// a still-unparseable row is a literal path separator, never an intentional
/// JSON escape (a document containing even one real `\n`/`\t`/`\\` escape
/// alongside bare backslashes would imply two different corruption
/// mechanisms touched the same row, which has no evidence and isn't
/// something this targeted repair needs to handle).
///
/// This is deliberately NOT a no-op on already-valid JSON — doubling a
/// genuine `\\` pair would over-escape it. That's fine because the only
/// caller (`identity_repair_malformed_secret_refs`) already checks
/// `serde_json::from_str` first and skips rows that parse cleanly; this
/// function must never be called on a value that isn't already confirmed
/// invalid.
///
/// A prior version tried to distinguish "bare" backslashes from "already
/// valid escape sequences" by peeking at the next character (`\b`, `\t`,
/// `\n`, ... look like valid escapes). That's unsound for real Windows
/// paths: a segment like `\bob\test` starts with `\b` and `\t`, which are
/// themselves valid JSON escape shapes, so the heuristic left them
/// un-doubled and silently produced a backspace/tab character instead of
/// the literal path (caught by reagent review on PR #2419).
fn repair_bare_backslashes(raw: &str) -> String {
    raw.replace('\\', "\\\\")
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
/// `deleteidentityaccount` handler for `identity.delete:` logging and
/// the layer-2/4 affected-agent disclosure
/// (`SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` §3/§4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityDeleteOutcome {
    /// True iff the `db_accounts` row existed and was deleted.
    pub deleted: bool,
    /// Number of `db_agent_identity_links` rows removed in the same
    /// transaction.
    pub links_cascaded: usize,
    /// The `agent_id`s whose links were cascaded — captured by SELECT
    /// inside the same transaction, BEFORE the delete, so the set is
    /// exact even though the rows are gone by the time this returns.
    /// Any of these agents that has a live process still holds working
    /// tokens until restarted; the handler publishes a per-agent
    /// revocation event so the UI can disclose that (spec §3).
    pub affected_agents: Vec<String>,
}

impl Store {
    // ---- Identity account CRUD ----

    /// List identity accounts. If `provider` is `Some`, filter to that
    /// provider; otherwise return every account, ordered by most recent
    /// update first (so the identity panel shows live accounts on top).
    ///
    /// A row whose `secret_ref`/`context` JSON fails to parse is skipped
    /// (logged with its account id) rather than aborting the whole query —
    /// one corrupted row must never hide every other account. This is the
    /// isolation the `oauth_config_dir` tag-alias comment above already
    /// flagged as missing ("`identity_list()` aborts entirely on the first
    /// unparseable row") after that incident; this closes it at the source
    /// instead of only patching the one wire-format mismatch that triggered
    /// it. See `docs/analysis/ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04.md`.
    pub fn identity_list(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<IdentityAccount>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut rows_vec = Vec::new();
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<IdentityAccount> {
            let id: String = row.get(0)?;
            let secret_ref_json: String = row.get(5)?;
            let context_json: String = row.get(6)?;
            let secret_ref = match serde_json::from_str(&secret_ref_json) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        target: "identity",
                        account_id = %id,
                        error = %e,
                        "identity_list: skipping account with malformed secret_ref JSON",
                    );
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    ));
                }
            };
            Ok(IdentityAccount {
                id,
                name: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                display_name: row.get(4)?,
                secret_ref,
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
                    if let Ok(account) = r {
                        rows_vec.push(account);
                    }
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
                    if let Ok(account) = r {
                        rows_vec.push(account);
                    }
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

    /// Migration-only repair pass (`m0019_repair_malformed_secret_ref`):
    /// scan every `db_accounts.secret_ref` for JSON that fails to parse and
    /// attempt to fix the one known corruption shape — a Windows path
    /// pasted into the JSON string with its backslashes never escaped
    /// (found live: `{"backend":"oauth_config_dir","dir":"C:\Users\..."}`,
    /// not reproducible from any write path in this codebase — every real
    /// caller goes through `identity_upsert` above, which always
    /// `serde_json::to_string`s correctly; this shape only arises from an
    /// out-of-band edit of the database file). Rows that still don't parse
    /// after the repair attempt are left untouched and logged — never
    /// deleted, since a broken row may still be linked via
    /// `db_agent_identity_links` and `identity_list`'s per-row skip (above)
    /// already makes leaving it alone safe. Returns the number of rows
    /// repaired.
    pub fn identity_repair_malformed_secret_refs(&self) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let candidates: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT id, secret_ref FROM db_accounts")?;
            let iter = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for r in iter {
                out.push(r?);
            }
            out
        };

        let mut repaired = 0usize;
        for (id, raw) in candidates {
            if serde_json::from_str::<SecretRef>(&raw).is_ok() {
                continue; // already valid — the common case
            }
            let candidate = repair_bare_backslashes(&raw);
            match serde_json::from_str::<SecretRef>(&candidate) {
                Ok(fixed) => {
                    let canonical = serde_json::to_string(&fixed)?;
                    conn.execute(
                        "UPDATE db_accounts SET secret_ref = ?1 WHERE id = ?2",
                        params![canonical, id],
                    )?;
                    repaired += 1;
                    tracing::warn!(
                        target: "identity",
                        account_id = %id,
                        "identity.repair: fixed malformed secret_ref JSON (unescaped backslashes)",
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "identity",
                        account_id = %id,
                        error = %e,
                        "identity.repair: secret_ref still unparseable after repair attempt — left as-is",
                    );
                }
            }
        }
        Ok(repaired)
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
        // Capture the affected agent ids BEFORE the cascade delete, in
        // the same transaction, so the set can't race a concurrent
        // link/unlink (spec §3 — the handler publishes per-agent
        // revocation events off this list).
        let affected_agents: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT agent_id FROM db_agent_identity_links
                 WHERE account_id = ?1 ORDER BY agent_id",
            )?;
            let iter = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in iter {
                out.push(r?);
            }
            out
        };
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
            affected_agents,
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

    /// Every distinct `(agent_id, provider)` link pair. Used by the
    /// m0017/m0018 ambient-login grandfather migrations, which must reason
    /// about provider CLASS: only oauth-class links mean an agent "opted
    /// into managed CLI login" (flag stays 0) — an api-key link (e.g. a
    /// github PAT) is never spawn-gated, so it must not forfeit
    /// grandfathering (spec §2.4 of
    /// SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md). Class
    /// filtering happens at the caller via
    /// `identity::resolver::provider_class`; the storage layer stays
    /// class-agnostic.
    pub fn agent_identity_link_provider_pairs(&self) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT agent_id, provider FROM db_agent_identity_links",
        )?;
        let iter = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
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

    /// Pins the on-disk wire format directly, rather than round-tripping
    /// through this crate's own serializer (which would happily agree with
    /// itself even if `rename_all`'s derived tag drifted from what's
    /// actually stored). `oauth_config_dir` here is copied verbatim from a
    /// real account row in `~/.agentmux/shared/store.db` — this is the
    /// literal regression that broke `identity_list()` (and therefore "My
    /// Agents"/recent-sessions, which aborts entirely on the first
    /// unparseable row) for every account written before the enum-wide
    /// `rename_all = "snake_case"` derive silently started expecting
    /// `o_auth_config_dir` instead (serde splits at every capital letter,
    /// including inside "OAuth" itself).
    #[test]
    fn secret_ref_deserializes_real_stored_oauth_config_dir_json() {
        let json = r#"{"backend":"oauth_config_dir","dir":"C:\\Users\\area54\\.agentmux\\shared\\identities\\bfa59d03-7a99-420e-b912-716bba1c462d\\claude"}"#;
        let parsed: SecretRef = serde_json::from_str(json).expect("must deserialize real stored wire format");
        match parsed {
            SecretRef::OAuthConfigDir { dir } => {
                assert!(dir.ends_with("claude"), "dir should round-trip: {dir}");
            }
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }
    }

    /// Pins the OTHER wire format still sitting in some accounts: whatever
    /// backend was running between this variant's introduction and the
    /// `oauth_config_dir` rename above wrote (and read back) the
    /// derive-produced `o_auth_config_dir` tag. Losing read support for it
    /// reproduces the exact "My Agents empty" incident for those accounts —
    /// `identity_list()` aborts entirely on the first unparseable row.
    #[test]
    fn secret_ref_deserializes_legacy_derive_produced_tag_json() {
        let json = r#"{"backend":"o_auth_config_dir","dir":"C:\\Users\\area54\\.agentmux\\shared\\identities\\269c028b-3894-4231-b884-fa5d0ecfabdf\\claude"}"#;
        let parsed: SecretRef = serde_json::from_str(json).expect("must deserialize legacy derive-produced wire format");
        match parsed {
            SecretRef::OAuthConfigDir { dir } => {
                assert!(dir.ends_with("claude"), "dir should round-trip: {dir}");
            }
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }
    }

    /// Pins the exact `upsertidentityaccount` payload shape sent by
    /// `persistSeededAccount` (frontend/app/view/agent/flows/
    /// register-seeded-account.ts) — no `created_at`/`updated_at` at all,
    /// since the handler is documented to server-stamp them. Before
    /// `#[serde(default)]` was added to those two fields, this exact shape
    /// failed `serde_json::from_value` with "missing field `created_at`" on
    /// every call, so every seed-from-global / terminal-fallback login
    /// silently never persisted an account row (the credential landed on
    /// disk, but no `db_accounts` row or agent link ever existed — see
    /// REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md).
    #[test]
    fn identity_account_deserializes_frontend_seed_payload_missing_timestamps() {
        let json = r#"{
            "id": "8d34a369-6ba6-4071-93bd-d4e051cdb457",
            "name": "claude-oauth",
            "provider": "claude",
            "kind": "oauth",
            "secret_ref": {"backend": "oauth_config_dir", "dir": "/tmp/claude"},
            "status": "valid"
        }"#;
        let parsed: IdentityAccount =
            serde_json::from_value(serde_json::from_str(json).unwrap())
                .expect("must deserialize the frontend's timestamp-less seed payload");
        assert_eq!(parsed.created_at, 0);
        assert_eq!(parsed.updated_at, 0);
        assert_eq!(parsed.status, "valid");
    }

    /// Serialization must always emit the canonical tag, never the legacy
    /// alias — the alias is read-only backward compat, not a second valid
    /// output format.
    #[test]
    fn secret_ref_serializes_oauth_config_dir_with_canonical_tag() {
        let value = SecretRef::OAuthConfigDir {
            dir: "/tmp/claude".to_string(),
        };
        let json = serde_json::to_string(&value).unwrap();
        assert!(json.contains(r#""backend":"oauth_config_dir""#), "got: {json}");
        assert!(!json.contains("o_auth_config_dir"), "got: {json}");
    }

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
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        
            memory_id: String::new(),}
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
        assert_eq!(
            out.affected_agents,
            vec!["ag1".to_string()],
            "agent ids must be captured before the cascade delete (spec §3)"
        );

        // Second delete is a no-op on both tables.
        let again = store.identity_delete("acct-1").unwrap();
        assert!(!again.deleted);
        assert_eq!(again.links_cascaded, 0);
        assert!(again.affected_agents.is_empty());
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
        assert_eq!(
            out.affected_agents,
            vec!["ag1".to_string()],
            "capture must run before both the explicit and the FK cascade"
        );
    }

    /// Multiple linked agents → all captured, in deterministic (sorted)
    /// order. Account with NO links → empty set (the delete-time
    /// disclosure must not fire).
    #[test]
    fn identity_delete_captures_all_affected_agents() {
        let store = Store::open_in_memory().unwrap();
        // Fresh schema enforces the links FK on agent_id, so the agent
        // definitions must exist before linking.
        for (id, slug) in [("ag-b", "agent-b"), ("ag-a", "agent-a"), ("ag-other", "agent-o")] {
            let mut agent = sample_agent(id, slug);
            store.agent_def_insert(&mut agent).unwrap();
        }
        store
            .identity_upsert(&sample_account("acct-1", "anthropic"))
            .unwrap();
        store
            .identity_upsert(&sample_account("acct-2", "anthropic"))
            .unwrap();
        store
            .agent_identity_link("ag-b", "acct-1", "anthropic")
            .unwrap();
        store
            .agent_identity_link("ag-a", "acct-1", "anthropic")
            .unwrap();
        // A link on a DIFFERENT account must not leak into the set.
        store
            .agent_identity_link("ag-other", "acct-2", "anthropic")
            .unwrap();

        let out = store.identity_delete("acct-1").unwrap();
        assert!(out.deleted);
        assert_eq!(out.links_cascaded, 2);
        assert_eq!(
            out.affected_agents,
            vec!["ag-a".to_string(), "ag-b".to_string()],
            "all linked agents captured, sorted, and scoped to the deleted account"
        );

        // Account with NO links → empty affected set (disclosure must
        // not fire for it).
        store
            .identity_upsert(&sample_account("acct-3", "anthropic"))
            .unwrap();
        let out3 = store.identity_delete("acct-3").unwrap();
        assert!(out3.deleted);
        assert_eq!(out3.links_cascaded, 0);
        assert!(
            out3.affected_agents.is_empty(),
            "linkless account delete must report an empty affected set"
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

    // ---- Malformed secret_ref tolerance (ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04) ----

    /// Directly pins the production corruption: a raw Windows path pasted
    /// into JSON with unescaped backslashes must round-trip through the
    /// repair to a valid, semantically-equivalent `SecretRef`.
    #[test]
    fn repair_bare_backslashes_fixes_unescaped_windows_path() {
        let broken = r#"{"backend":"oauth_config_dir","dir":"C:\Users\area54\.agentmux\shared\identities\34db876e-ccef-4bb4-a41f-5962d6212df5\claude"}"#;
        assert!(
            serde_json::from_str::<SecretRef>(broken).is_err(),
            "fixture must reproduce the real parse failure"
        );
        let repaired = repair_bare_backslashes(broken);
        let parsed: SecretRef =
            serde_json::from_str(&repaired).expect("repair must produce valid JSON");
        match parsed {
            SecretRef::OAuthConfigDir { dir } => {
                assert_eq!(
                    dir,
                    r"C:\Users\area54\.agentmux\shared\identities\34db876e-ccef-4bb4-a41f-5962d6212df5\claude"
                );
            }
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }
    }

    /// Regression for the P0 caught on PR #2419 review: a path segment that
    /// starts with a letter matching a valid JSON escape (`\bob`, `\test`,
    /// `\node_modules`, ...) must still get its backslash doubled, not
    /// mistaken for an already-valid `\b`/`\t`/`\n` escape.
    #[test]
    fn repair_bare_backslashes_fixes_paths_with_escape_letter_prefixed_segments() {
        let broken = r#"{"backend":"oauth_config_dir","dir":"C:\Users\bob\test\node_modules\uploads\format"}"#;
        let repaired = repair_bare_backslashes(broken);
        let parsed: SecretRef =
            serde_json::from_str(&repaired).expect("repair must produce valid JSON");
        match parsed {
            SecretRef::OAuthConfigDir { dir } => {
                assert_eq!(dir, r"C:\Users\bob\test\node_modules\uploads\format");
            }
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }
    }

    /// `repair_bare_backslashes` is only ever called on rows that already
    /// failed to parse as JSON (`identity_repair_malformed_secret_refs`
    /// checks `serde_json::from_str` first and skips valid rows) — it must
    /// never run against already-valid JSON, since unconditionally doubling
    /// every backslash would over-escape a genuine `\\` pair.
    #[test]
    fn repair_bare_backslashes_over_escapes_already_valid_json_by_design() {
        let valid = r#"{"backend":"oauth_config_dir","dir":"C:\\Users\\area54\\claude"}"#;
        assert_ne!(
            repair_bare_backslashes(valid),
            valid,
            "documents that callers must never invoke this on already-valid JSON"
        );
    }

    /// `identity_list` must return the good rows and skip the malformed
    /// one — not abort the whole query. This is the actual bug: one
    /// corrupted account made 9 real accounts invisible everywhere
    /// `identity_list` is called (Armory's account list, boot-time
    /// identity sweep, post-login cache refresh).
    #[test]
    fn identity_list_skips_a_malformed_row_instead_of_failing_the_whole_query() {
        let store = Store::open_in_memory().unwrap();
        store
            .identity_upsert(&sample_account("acct-good-1", "claude"))
            .unwrap();
        store
            .identity_upsert(&sample_account("acct-good-2", "claude"))
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO db_accounts
                    (id, name, provider, kind, display_name, secret_ref, context,
                     status, created_at, updated_at)
                 VALUES ('acct-bad', 'broken', 'claude', 'oauth', '',
                         '{\"backend\":\"oauth_config_dir\",\"dir\":\"C:\\bad\\path\"}',
                         '{}', 'unknown', 0, 0)",
                [],
            )
            .unwrap();
        }

        let accounts = store
            .identity_list(None)
            .expect("must not fail even with a malformed row present");
        let ids: Vec<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"acct-good-1"));
        assert!(ids.contains(&"acct-good-2"));
        assert!(!ids.contains(&"acct-bad"), "malformed row must be excluded, not error the whole call");
        assert_eq!(accounts.len(), 2);
    }

    /// End-to-end: the migration-facing repair method fixes the known
    /// corruption shape in place, and a subsequent `identity_list` picks up
    /// the repaired row.
    #[test]
    fn identity_repair_malformed_secret_refs_fixes_row_and_identity_list_recovers_it() {
        let store = Store::open_in_memory().unwrap();
        store
            .identity_upsert(&sample_account("acct-good", "claude"))
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
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

        // Before repair: identity_list tolerates it (previous test) but the
        // account itself is invisible.
        assert_eq!(store.identity_list(None).unwrap().len(), 1);

        let repaired = store.identity_repair_malformed_secret_refs().unwrap();
        assert_eq!(repaired, 1);

        let accounts = store.identity_list(None).unwrap();
        assert_eq!(accounts.len(), 2, "repaired row must now be visible");
        let fixed = accounts.iter().find(|a| a.id == "acct-broken").unwrap();
        match &fixed.secret_ref {
            SecretRef::OAuthConfigDir { dir } => assert_eq!(dir, r"C:\Users\a\claude"),
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }

        // Idempotent: running it again on an already-repaired store is a no-op.
        assert_eq!(store.identity_repair_malformed_secret_refs().unwrap(), 0);
    }

    /// A row whose corruption ISN'T the known "bare backslash" shape (e.g. a
    /// genuinely unrecognizable backend tag) must be left untouched rather
    /// than mangled or deleted — the repair only handles the one known bug
    /// class, per its own doc comment.
    #[test]
    fn identity_repair_leaves_unrecognized_corruption_untouched() {
        let store = Store::open_in_memory().unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO db_accounts
                    (id, name, provider, kind, display_name, secret_ref, context,
                     status, created_at, updated_at)
                 VALUES ('acct-weird', 'weird', 'claude', 'oauth', '',
                         '{\"backend\":\"totally_unknown_backend\"}',
                         '{}', 'unknown', 0, 0)",
                [],
            )
            .unwrap();
        }
        let repaired = store.identity_repair_malformed_secret_refs().unwrap();
        assert_eq!(repaired, 0, "unrecognized corruption must not be reported as repaired");
        assert_eq!(
            store.identity_list(None).unwrap().len(),
            0,
            "still-broken row stays invisible to identity_list (safe default) but is not deleted"
        );
        let conn = store.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM db_accounts WHERE id = 'acct-weird'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "row must survive an unsuccessful repair attempt");
    }
}
