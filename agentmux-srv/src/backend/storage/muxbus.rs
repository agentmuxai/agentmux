// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;
use crate::identity::secret_store;

/// `secret_store` is otherwise keyed by identity-account id; MuxBus has
/// exactly one, global credential set, so it uses a fixed sentinel key —
/// shared with the broker's credential id, see `crate::muxbus::CREDENTIAL_ID`.
const MUXBUS_KEYCHAIN_ID: &str = crate::muxbus::CREDENTIAL_ID;

#[derive(Debug, Clone, Default)]
pub struct MuxBusCredentials {
    pub cognito_domain: String,
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub expires_at: i64,
    pub user_email: String,
    pub user_sub: String,
}

/// The actual secret material, serialized into the OS keychain entry.
/// Everything else on `MuxBusCredentials` is non-secret metadata and stays
/// in SQLite, matching how `SecretRef::Keychain` accounts already split
/// pointer-metadata (DB) from plaintext (keychain) elsewhere in this codebase.
#[derive(Serialize, Deserialize, Default)]
struct MuxBusTokens {
    access_token: String,
    refresh_token: String,
    id_token: String,
}

impl MuxBusCredentials {
    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    pub fn is_valid(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at > Self::now_secs()
    }

    pub fn nearly_expired(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at - Self::now_secs() < 300
    }
}

impl Store {
    pub fn muxbus_load(&self) -> Result<Option<MuxBusCredentials>, StoreError> {
        let row = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT cognito_domain, client_id, access_token, refresh_token, id_token,
                        expires_at, user_email, user_sub
                 FROM db_muxbus_credentials WHERE id = 'global'",
            )?;
            match stmt.query_row([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, // legacy plaintext access_token (pre-keychain rows)
                    row.get::<_, String>(3)?, // legacy plaintext refresh_token
                    row.get::<_, String>(4)?, // legacy plaintext id_token
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            }) {
                Ok(r) => r,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        };
        let (cognito_domain, client_id, legacy_access, legacy_refresh, legacy_id, expires_at, user_email, user_sub) = row;

        // reagent P1 on #2260: the old code collapsed EVERY keychain-read
        // failure (locked, no Secret Service daemon, permission denied — not
        // just "no entry stored") into the same branch as "no credential at
        // all," so a transient storage failure silently looked like a full
        // logout. get_optional distinguishes the two: `Ok(None)` really
        // means no entry exists; `Err` means the read itself failed.
        let tokens = match secret_store::get_optional(MUXBUS_KEYCHAIN_ID) {
            Ok(Some(blob)) => {
                // reagent P2: a corrupted/unparseable keychain blob used to
                // silently collapse to MuxBusTokens::default() via
                // unwrap_or_default() — presenting as a full logout instead
                // of surfacing that something is actually corrupted,
                // inconsistent with this same match's handling of real
                // keychain READ errors below (which correctly propagates).
                // A malformed blob here means something wrote bad data, not
                // "no credential" — treat it the same way: a real error.
                serde_json::from_str::<MuxBusTokens>(&blob).map_err(|e| {
                    StoreError::Other(format!("muxbus: stored keychain blob is corrupted: {e}"))
                })?
            }
            Ok(None) if !legacy_access.is_empty() => {
                // Lazy migration: this row predates keychain-backed storage.
                // Use the plaintext columns this one time, and self-heal by
                // writing them into the keychain + blanking the SQL columns
                // so every subsequent load hits the keychain path instead.
                let tokens = MuxBusTokens {
                    access_token: legacy_access,
                    refresh_token: legacy_refresh,
                    id_token: legacy_id,
                };
                match serde_json::to_string(&tokens) {
                    Ok(blob) if secret_store::put(MUXBUS_KEYCHAIN_ID, &blob).is_ok() => {
                        let conn = self.conn.lock().unwrap();
                        let _ = conn.execute(
                            "UPDATE db_muxbus_credentials
                             SET access_token = '', refresh_token = '', id_token = ''
                             WHERE id = 'global'",
                            [],
                        );
                    }
                    _ => {
                        tracing::warn!(
                            "muxbus: keychain write failed during lazy migration — \
                             leaving plaintext columns in place for now"
                        );
                    }
                }
                tokens
            }
            Ok(None) => MuxBusTokens::default(),
            Err(e) if !legacy_access.is_empty() => {
                // Keychain is transiently broken, but the legacy plaintext
                // columns are still sitting right there — serve from them
                // rather than hard-failing when we actually have usable
                // data. Deliberately do NOT attempt the self-heal write in
                // this branch: writing to a keychain we just confirmed is
                // failing would just fail again, noisily, for no benefit.
                tracing::warn!(
                    error = %e,
                    "muxbus: keychain read failed, falling back to legacy plaintext columns"
                );
                MuxBusTokens {
                    access_token: legacy_access,
                    refresh_token: legacy_refresh,
                    id_token: legacy_id,
                }
            }
            Err(e) => {
                // No legacy fallback available either — this IS a real
                // failure, not "no credentials stored." Propagate it so
                // callers (e.g. cloud_subscriber's has_stored_creds check)
                // don't mistake a transient storage failure for a full
                // logout and park indefinitely waiting for a fresh login
                // that was never actually needed.
                return Err(StoreError::Other(format!("muxbus: keychain read failed: {e}")));
            }
        };

        Ok(Some(MuxBusCredentials {
            cognito_domain,
            client_id,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            id_token: tokens.id_token,
            expires_at,
            user_email,
            user_sub,
        }))
    }

    pub fn muxbus_save(&self, creds: &MuxBusCredentials) -> Result<(), StoreError> {
        let tokens = MuxBusTokens {
            access_token: creds.access_token.clone(),
            refresh_token: creds.refresh_token.clone(),
            id_token: creds.id_token.clone(),
        };
        let blob = serde_json::to_string(&tokens)?;

        // Captured so a SQL failure below can restore the keychain to
        // exactly what it held before this call (reagent P2 on #2260):
        // without this, a keychain write that succeeds followed by a SQL
        // write that then fails (e.g. a transient lock) leaves the FRESH
        // tokens paired with the OLD SQL metadata (expires_at/user_email —
        // that INSERT never committed, so it's untouched), a mismatch that
        // previously only self-healed on the next successful save.
        //
        // Three-way, not a bool: reagent P1 caught a first version of this
        // that collapsed `Ok(None)` ("genuinely no prior entry") and `Err`
        // ("the read itself failed, prior state UNKNOWN") into the same
        // `None`, so an unrelated transient read failure would make the
        // rollback branch below `delete()` a real, valid, previously-stored
        // credential it simply couldn't read — turning a transient glitch
        // into a forced full re-login. `Unknown` gets neither restore nor
        // delete: we truly don't know what was there, so the only safe
        // move is to leave whatever `put(&blob)` just wrote and log loudly,
        // same as the pre-existing self-heals-next-save behavior for this
        // one sub-case specifically.
        enum PriorKeychainState {
            Existed(zeroize::Zeroizing<String>),
            Absent,
            Unknown,
        }
        let previous = match secret_store::get_optional(MUXBUS_KEYCHAIN_ID) {
            Ok(Some(blob)) => PriorKeychainState::Existed(blob),
            Ok(None) => PriorKeychainState::Absent,
            Err(_) => PriorKeychainState::Unknown,
        };

        secret_store::put(MUXBUS_KEYCHAIN_ID, &blob)
            .map_err(|e| StoreError::Other(format!("muxbus: keychain write failed: {e}")))?;

        let sql_result = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO db_muxbus_credentials
                     (id, cognito_domain, client_id, access_token, refresh_token, id_token,
                      expires_at, user_email, user_sub)
                 VALUES ('global', ?1, ?2, '', '', '', ?3, ?4, ?5)",
                params![
                    creds.cognito_domain,
                    creds.client_id,
                    creds.expires_at,
                    creds.user_email,
                    creds.user_sub,
                ],
            )
        };
        if let Err(e) = sql_result {
            match previous {
                PriorKeychainState::Existed(old) => {
                    if let Err(re) = secret_store::put(MUXBUS_KEYCHAIN_ID, &old) {
                        tracing::warn!(
                            error = %re,
                            sql_error = %e,
                            "muxbus_save: SQL write failed AND restoring the prior keychain blob \
                             also failed — keychain may now hold fresh tokens with no matching \
                             SQL metadata until the next successful save"
                        );
                    }
                }
                PriorKeychainState::Absent => {
                    if let Err(de) = secret_store::delete(MUXBUS_KEYCHAIN_ID) {
                        tracing::warn!(
                            error = %de,
                            sql_error = %e,
                            "muxbus_save: SQL write failed AND rolling back the keychain write \
                             also failed — keychain may now hold fresh tokens with no matching \
                             SQL metadata until the next successful save"
                        );
                    }
                }
                PriorKeychainState::Unknown => {
                    tracing::warn!(
                        sql_error = %e,
                        "muxbus_save: SQL write failed after a keychain write whose prior state \
                         couldn't be read — leaving the just-written tokens in place rather than \
                         risk deleting a real credential; SQL metadata may be stale until the \
                         next successful save"
                    );
                }
            }
            return Err(e.into());
        }
        Ok(())
    }

    pub fn muxbus_clear(&self) -> Result<(), StoreError> {
        // Best-effort — a missing/inaccessible keychain entry must not block
        // clearing the (still-useful) SQL row.
        let _ = secret_store::delete(MUXBUS_KEYCHAIN_ID);
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_muxbus_credentials WHERE id = 'global'", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_json_round_trips() {
        let tokens = MuxBusTokens {
            access_token: "at".to_string(),
            refresh_token: "rt".to_string(),
            id_token: "it".to_string(),
        };
        let blob = serde_json::to_string(&tokens).unwrap();
        let back: MuxBusTokens = serde_json::from_str(&blob).unwrap();
        assert_eq!(back.access_token, "at");
        assert_eq!(back.refresh_token, "rt");
        assert_eq!(back.id_token, "it");
    }
}
