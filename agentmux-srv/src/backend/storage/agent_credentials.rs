// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent M2M Cognito credential for muxbus Tier-4 credential binding.
//! See db_agent_credentials in migrations.rs (SHARED_STORE_SCHEMA_VERSION v4)
//! and agentmux-cloud's PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md.
//!
//! One row per locally-registered agent_id — distinct from the single
//! account-level db_muxbus_credentials singleton (muxbus.rs), which stores
//! the human's PKCE login used only to provision these.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, Default)]
pub struct AgentCredential {
    pub client_id: String,
    pub client_secret: String,
    pub token_endpoint: String,
    /// Cached client_credentials access token — separate from client_secret
    /// so a fresh token fetch doesn't require re-provisioning.
    pub access_token: String,
    pub expires_at: i64,
}

impl AgentCredential {
    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    /// client_credentials tokens carry no refresh token (per the design in
    /// agentmux-cloud#2) — "valid" just means "not expired yet," with the
    /// same 300s early-refresh margin used by MuxBusCredentials.
    pub fn is_valid(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at - Self::now_secs() > 300
    }
}

impl Store {
    pub fn agent_credential_load(&self, agent_id: &str) -> Result<Option<AgentCredential>, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT client_id, client_secret, token_endpoint, access_token, expires_at
             FROM db_agent_credentials WHERE agent_id = ?1",
        )?;
        match stmt.query_row(params![key], |row| {
            Ok(AgentCredential {
                client_id: row.get(0)?,
                client_secret: row.get(1)?,
                token_endpoint: row.get(2)?,
                access_token: row.get(3)?,
                expires_at: row.get(4)?,
            })
        }) {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Save a newly-provisioned client_id/client_secret (access_token/expires_at
    /// left at their defaults — the caller fetches a token separately via
    /// agent_credential_save_token).
    pub fn agent_credential_save(
        &self,
        agent_id: &str,
        client_id: &str,
        client_secret: &str,
        token_endpoint: &str,
    ) -> Result<(), StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_credentials
                 (agent_id, client_id, client_secret, token_endpoint, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent_id) DO UPDATE SET
                 client_id = excluded.client_id,
                 client_secret = excluded.client_secret,
                 token_endpoint = excluded.token_endpoint",
            params![key, client_id, client_secret, token_endpoint, AgentCredential::now_secs()],
        )?;
        Ok(())
    }

    /// Cache a freshly-fetched client_credentials access token against an
    /// already-provisioned agent. No-op if the agent has no row yet (the
    /// caller should provision first).
    pub fn agent_credential_save_token(
        &self,
        agent_id: &str,
        access_token: &str,
        expires_at: i64,
    ) -> Result<(), StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_agent_credentials SET access_token = ?2, expires_at = ?3 WHERE agent_id = ?1",
            params![key, access_token, expires_at],
        )?;
        Ok(())
    }

    /// Clear a per-agent credential's cached access token (client_id/secret
    /// are left intact — only the token is suspect). Used when the server
    /// rejects a request with 401 despite our cached expires_at still
    /// looking valid (revoked/rotated out-of-band): forces the next
    /// ensure_agent_credential call to re-fetch via fetch_m2m_token instead
    /// of retrying the exact same rejected token. No-op if the agent has no
    /// row. reagentx P1 on PR #2342.
    pub fn agent_credential_invalidate_token(&self, agent_id: &str) -> Result<(), StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_agent_credentials SET access_token = '', expires_at = 0 WHERE agent_id = ?1",
            params![key],
        )?;
        Ok(())
    }

    /// Wipe every cached per-agent M2M credential. `db_agent_credentials`
    /// carries no account/user_sub column of its own — each row is only
    /// ever meaningful under the muxbus account that provisioned it — so a
    /// genuine account switch (muxbus_save with a different user_sub) or a
    /// logout (muxbus_clear) must wipe this cache wholesale; otherwise a
    /// stale row keeps authenticating this agent_id's requests as the
    /// PREVIOUS account after the human has switched to a different one.
    /// Re-provisioning is cheap and idempotent server-side (see
    /// agent-provisioning.ts), so clearing on every account transition —
    /// not just the ones that turn out to matter — is the safe default.
    /// reagentx P0 on PR #2342.
    pub fn agent_credentials_clear_all(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_agent_credentials", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A shared-schema store built on a temp file (open_shared needs a real
    // path; :memory: would be re-created per connection) — db_agent_credentials
    // lives in the shared schema (SHARED_STORE_SCHEMA_VERSION v3), not objects.db.
    fn shared_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    #[test]
    fn load_returns_none_when_unprovisioned() {
        let store = shared_store();
        assert!(store.agent_credential_load("agentx").unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let store = shared_store();
        store
            .agent_credential_save("AgentX", "client-1", "secret-1", "https://auth.example.com/oauth2/token")
            .unwrap();

        // Lookup is case-insensitive — normalized the same way agent_id is
        // normalized everywhere else in this codebase.
        let loaded = store.agent_credential_load("agentx").unwrap().unwrap();
        assert_eq!(loaded.client_id, "client-1");
        assert_eq!(loaded.client_secret, "secret-1");
        assert_eq!(loaded.token_endpoint, "https://auth.example.com/oauth2/token");
        assert_eq!(loaded.access_token, "");
        assert!(!loaded.is_valid()); // no access_token yet
    }

    #[test]
    fn save_is_idempotent_and_updates_in_place() {
        let store = shared_store();
        store.agent_credential_save("agentx", "client-1", "secret-1", "endpoint-1").unwrap();
        store.agent_credential_save("agentx", "client-2", "secret-2", "endpoint-2").unwrap();

        let loaded = store.agent_credential_load("agentx").unwrap().unwrap();
        assert_eq!(loaded.client_id, "client-2");
        assert_eq!(loaded.client_secret, "secret-2");
    }

    #[test]
    fn save_token_caches_access_token_and_expiry() {
        let store = shared_store();
        store.agent_credential_save("agentx", "client-1", "secret-1", "endpoint-1").unwrap();

        let future = AgentCredential::now_secs() + 3600;
        store.agent_credential_save_token("agentx", "tok-abc", future).unwrap();

        let loaded = store.agent_credential_load("agentx").unwrap().unwrap();
        assert_eq!(loaded.access_token, "tok-abc");
        assert_eq!(loaded.expires_at, future);
        assert!(loaded.is_valid());
    }

    #[test]
    fn save_token_is_a_noop_when_not_provisioned() {
        let store = shared_store();
        // No agent_credential_save call first -- row doesn't exist.
        store.agent_credential_save_token("agentx", "tok-abc", AgentCredential::now_secs() + 3600).unwrap();
        assert!(store.agent_credential_load("agentx").unwrap().is_none());
    }

    #[test]
    fn invalidate_token_clears_token_and_expiry_but_keeps_client() {
        let store = shared_store();
        store.agent_credential_save("agentx", "client-1", "secret-1", "endpoint-1").unwrap();
        store
            .agent_credential_save_token("agentx", "tok-abc", AgentCredential::now_secs() + 3600)
            .unwrap();

        store.agent_credential_invalidate_token("agentx").unwrap();

        let loaded = store.agent_credential_load("agentx").unwrap().unwrap();
        assert_eq!(loaded.access_token, "");
        assert_eq!(loaded.expires_at, 0);
        assert!(!loaded.is_valid());
        // Provisioned client identity survives — only the token was suspect.
        assert_eq!(loaded.client_id, "client-1");
        assert_eq!(loaded.client_secret, "secret-1");
    }

    #[test]
    fn invalidate_token_is_a_noop_when_not_provisioned() {
        let store = shared_store();
        store.agent_credential_invalidate_token("agentx").unwrap();
        assert!(store.agent_credential_load("agentx").unwrap().is_none());
    }

    #[test]
    fn clear_all_wipes_every_agent_credential() {
        let store = shared_store();
        store.agent_credential_save("agentx", "client-1", "secret-1", "endpoint-1").unwrap();
        store.agent_credential_save("agenty", "client-2", "secret-2", "endpoint-2").unwrap();

        store.agent_credentials_clear_all().unwrap();

        assert!(store.agent_credential_load("agentx").unwrap().is_none());
        assert!(store.agent_credential_load("agenty").unwrap().is_none());
    }

    #[test]
    fn clear_all_is_a_noop_on_an_empty_table() {
        let store = shared_store();
        store.agent_credentials_clear_all().unwrap();
        assert!(store.agent_credential_load("agentx").unwrap().is_none());
    }

    #[test]
    fn is_valid_respects_the_300s_early_refresh_margin() {
        let mut cred = AgentCredential {
            client_id: "c".to_string(),
            client_secret: "s".to_string(),
            token_endpoint: "e".to_string(),
            access_token: "tok".to_string(),
            expires_at: AgentCredential::now_secs() + 301,
        };
        assert!(cred.is_valid());

        cred.expires_at = AgentCredential::now_secs() + 299;
        assert!(!cred.is_valid());
    }

    #[test]
    fn is_valid_false_with_no_access_token() {
        let cred = AgentCredential {
            expires_at: AgentCredential::now_secs() + 3600,
            ..Default::default()
        };
        assert!(!cred.is_valid());
    }
}
