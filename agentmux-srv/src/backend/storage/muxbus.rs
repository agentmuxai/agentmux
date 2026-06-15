// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

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
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT cognito_domain, client_id, access_token, refresh_token, id_token,
                    expires_at, user_email, user_sub
             FROM db_muxbus_credentials WHERE id = 'global'",
        )?;
        match stmt.query_row([], |row| {
            Ok(MuxBusCredentials {
                cognito_domain: row.get(0)?,
                client_id: row.get(1)?,
                access_token: row.get(2)?,
                refresh_token: row.get(3)?,
                id_token: row.get(4)?,
                expires_at: row.get(5)?,
                user_email: row.get(6)?,
                user_sub: row.get(7)?,
            })
        }) {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn muxbus_save(&self, creds: &MuxBusCredentials) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_muxbus_credentials
                 (id, cognito_domain, client_id, access_token, refresh_token, id_token,
                  expires_at, user_email, user_sub)
             VALUES ('global', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                creds.cognito_domain,
                creds.client_id,
                creds.access_token,
                creds.refresh_token,
                creds.id_token,
                creds.expires_at,
                creds.user_email,
                creds.user_sub,
            ],
        )?;
        Ok(())
    }

    pub fn muxbus_clear(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_muxbus_credentials WHERE id = 'global'", [])?;
        Ok(())
    }
}
