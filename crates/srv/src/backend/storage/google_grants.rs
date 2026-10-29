// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Which agents may use which Google service of a connected account, and what
//! they did with it (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md
//! D4, D7).
//!
//! - `db_google_grants`: per agent (its definition id, the uid a verified call
//!   carries), per account, per service (`calendar`, `drive`, `gmail`), a level:
//!   `read`, or `write` (read and write). No row is `off`, the default.
//! - `db_google_calls`: one row per tool call: when, which agent, which action,
//!   the ids it touched and how it ended. Never content. Kept 30 days.
//!
//! In the identity store, beside the account links, so every channel on the
//! computer sees the same grants.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// The tables, created on every open of the identity store, with no schema
/// version bump.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_google_grants (
        agent_id   TEXT NOT NULL,
        account_id TEXT NOT NULL,
        service    TEXT NOT NULL,
        level      TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (agent_id, account_id, service)
    );
    CREATE TABLE IF NOT EXISTS db_google_calls (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        account_id TEXT NOT NULL,
        at         INTEGER NOT NULL,
        agent_id   TEXT NOT NULL DEFAULT '',
        agent_name TEXT NOT NULL DEFAULT '',
        service    TEXT NOT NULL,
        action     TEXT NOT NULL,
        ids        TEXT NOT NULL DEFAULT '',
        outcome    TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_google_calls_account ON db_google_calls(account_id, at);";

/// How long a call's log line is kept.
pub const CALLS_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The Google services an account can be used for.
pub const SERVICES: [&str; 3] = ["calendar", "drive", "gmail"];

/// What an agent may do with one service of one account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantLevel {
    Read,
    /// Read and write.
    Write,
}

impl GrantLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantLevel::Read => "read",
            GrantLevel::Write => "write",
        }
    }

    /// `read`, `write`; `off` (or empty) is no level.
    pub fn parse(s: &str) -> Result<Option<GrantLevel>, String> {
        match s.trim() {
            "read" => Ok(Some(GrantLevel::Read)),
            "write" => Ok(Some(GrantLevel::Write)),
            "off" | "" => Ok(None),
            other => Err(format!("level \"{other}\": read, write or off")),
        }
    }
}

/// One agent's grant on one service of one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleGrant {
    pub agent_id: String,
    pub account_id: String,
    pub service: String,
    pub level: GrantLevel,
}

/// One line of an account's call log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleCall {
    pub at: i64,
    pub agent_id: String,
    pub agent_name: String,
    pub service: String,
    pub action: String,
    /// The ids it touched (calendar, event or file ids), comma-separated.
    pub ids: String,
    /// `ok`, or why it didn't (`refused: …`, `error: …`).
    pub outcome: String,
}

fn check_service(service: &str) -> Result<(), StoreError> {
    if SERVICES.contains(&service) {
        Ok(())
    } else {
        Err(StoreError::Other(format!("no Google service \"{service}\"")))
    }
}

fn row_to_grant(r: &rusqlite::Row) -> rusqlite::Result<GoogleGrant> {
    let level: String = r.get(3)?;
    Ok(GoogleGrant {
        agent_id: r.get(0)?,
        account_id: r.get(1)?,
        service: r.get(2)?,
        level: if level == "write" { GrantLevel::Write } else { GrantLevel::Read },
    })
}

impl Store {
    /// Set an agent's level on one service of one account; `None` is off.
    pub fn google_grant_set(
        &self,
        agent_id: &str,
        account_id: &str,
        service: &str,
        level: Option<GrantLevel>,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        check_service(service)?;
        let conn = self.conn().lock().unwrap();
        match level {
            Some(l) => conn.execute(
                "INSERT INTO db_google_grants (agent_id, account_id, service, level, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(agent_id, account_id, service) DO UPDATE SET level = excluded.level, updated_at = excluded.updated_at",
                params![agent_id, account_id, service, l.as_str(), now_ms],
            )?,
            None => conn.execute(
                "DELETE FROM db_google_grants WHERE agent_id = ?1 AND account_id = ?2 AND service = ?3",
                params![agent_id, account_id, service],
            )?,
        };
        Ok(())
    }

    /// An agent's level on one service of one account, `None` when off.
    pub fn google_grant_get(&self, agent_id: &str, account_id: &str, service: &str) -> Result<Option<GrantLevel>, StoreError> {
        Ok(self
            .google_grants_for_agent(agent_id)?
            .into_iter()
            .find(|g| g.account_id == account_id && g.service == service)
            .map(|g| g.level))
    }

    /// Every grant an agent has.
    pub fn google_grants_for_agent(&self, agent_id: &str) -> Result<Vec<GoogleGrant>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, account_id, service, level FROM db_google_grants WHERE agent_id = ?1 ORDER BY account_id, service",
        )?;
        let rows = stmt.query_map(params![agent_id], row_to_grant)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every grant on an account.
    pub fn google_grants_for_account(&self, account_id: &str) -> Result<Vec<GoogleGrant>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, account_id, service, level FROM db_google_grants WHERE account_id = ?1 ORDER BY agent_id, service",
        )?;
        let rows = stmt.query_map(params![account_id], row_to_grant)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Forget an account's grants and log (on disconnect).
    pub fn google_account_forget(&self, account_id: &str) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_google_grants WHERE account_id = ?1", params![account_id])?;
        conn.execute("DELETE FROM db_google_calls WHERE account_id = ?1", params![account_id])?;
        Ok(())
    }

    /// Write one line of an account's call log, and drop lines past
    /// [`CALLS_RETENTION_MS`].
    pub fn google_call_log(&self, account_id: &str, call: &GoogleCall) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_google_calls (account_id, at, agent_id, agent_name, service, action, ids, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![account_id, call.at, call.agent_id, call.agent_name, call.service, call.action, call.ids, call.outcome],
        )?;
        conn.execute("DELETE FROM db_google_calls WHERE at < ?1", params![call.at - CALLS_RETENTION_MS])?;
        Ok(())
    }

    /// How many log lines `agent_id` has for `service`'s `actions` on the account since `since_ms`,
    /// of which `ok` ones were not refused or failed. Counted in the store, so what other agents and
    /// other actions logged cannot push a line out of it.
    pub fn google_calls_count(
        &self,
        account_id: &str,
        agent_id: &str,
        service: &str,
        actions: &[&str],
        since_ms: i64,
        only_ok: bool,
    ) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        let marks = vec!["?"; actions.len()].join(",");
        let sql = format!(
            "SELECT COUNT(*) FROM db_google_calls WHERE account_id = ? AND agent_id = ? AND service = ? AND at > ?
               AND action IN ({marks}){}",
            if only_ok { " AND outcome = 'ok'" } else { "" }
        );
        let mut args: Vec<rusqlite::types::Value> = vec![account_id.to_string().into(), agent_id.to_string().into(), service.to_string().into(), since_ms.into()];
        args.extend(actions.iter().map(|a| rusqlite::types::Value::from(a.to_string())));
        let n: i64 = conn.query_row(&sql, rusqlite::params_from_iter(args), |r| r.get(0))?;
        Ok(n.max(0) as usize)
    }

    /// An account's newest log lines first.
    pub fn google_calls(&self, account_id: &str, limit: usize) -> Result<Vec<GoogleCall>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT at, agent_id, agent_name, service, action, ids, outcome FROM db_google_calls
              WHERE account_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit as i64], |r| {
            Ok(GoogleCall {
                at: r.get(0)?,
                agent_id: r.get(1)?,
                agent_name: r.get(2)?,
                service: r.get(3)?,
                action: r.get(4)?,
                ids: r.get(5)?,
                outcome: r.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
#[path = "google_grants_tests.rs"]
mod tests;
