// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The keys srv has stored for agents are withheld from transcripts from the start, not from the first time one is read.
//!
//! An agent's jekt signing key, its LAN and WAN private keys, and the private keys of its UID, are written to the
//! agent's `.mcp.json` and served to it at `/self/keys`. Each is registered with the transcript withhold registry when
//! it is made or loaded (`transcript::withhold`), but a key made on an earlier run is only loaded when its agent next
//! launches: until then a copy of it in an old file, or one an agent still holds, could be written into a transcript
//! unmarked. [`Store::withhold_stored_keys`] registers every stored key in one pass when srv starts.

use super::error::StoreError;
use super::store::Store;

/// Where a stored key's secret column is: the table and the column holding it.
const SECRET_COLUMNS: [(&str, &str); 5] = [
    ("db_agent_jekt_keys", "hmac_key"),
    ("db_agent_lan_keys", "private_key"),
    ("db_agent_wan_keys", "private_key"),
    ("db_agent_lan_keys_by_uid", "private_key"),
    ("db_agent_wan_keys_by_uid", "private_key"),
];

impl Store {
    /// Registers every key stored for an agent, and the private keys of the host's key table when it is attached, with
    /// the transcript withhold registry: one pass, one change to the registry. Returns how many keys it registered. A
    /// table that cannot be read is logged and skipped, so one bad table does not leave the others unregistered.
    pub fn withhold_stored_keys(&self) -> usize {
        let mut keys: Vec<String> = Vec::new();
        {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            for (table, column) in SECRET_COLUMNS {
                match read_column(&conn, table, column) {
                    Ok(mut found) => keys.append(&mut found),
                    Err(e) => tracing::warn!(table, error = %e, "withhold: a table of stored keys could not be read"),
                }
            }
        }
        if let Some(host) = self.host_lan_keys() {
            match host.private_keys() {
                Ok(mut found) => keys.append(&mut found),
                Err(e) => tracing::warn!(error = %e, "withhold: the host's key table could not be read"),
            }
        }
        crate::backend::transcript::withhold::register_all(keys.iter().map(String::as_str));
        keys.len()
    }
}

fn read_column(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(&format!("SELECT {column} FROM {table}"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::transcript::withhold;

    #[test]
    fn every_stored_key_is_withheld_at_start_up_with_no_agent_launched() {
        let store = Store::open_in_memory().unwrap();
        // Keys made on an earlier run: rows only, with no spawn, load or ensure to register them.
        let rows = [
            ("db_agent_jekt_keys", "agent_id, hmac_key, created_at", "'boot-a', 'boot-jekt-0001-canary-value', 1"),
            ("db_agent_lan_keys", "agent_id, public_key, private_key, created_at", "'boot-a', 'pub', 'boot-lan-0002-canary-value', 1"),
            ("db_agent_wan_keys", "agent_id, public_key, private_key, created_at", "'boot-a', 'pub', 'boot-wan-0003-canary-value', 1"),
            ("db_agent_lan_keys_by_uid", "uid, public_key, private_key, created_at", "'boot-u', 'pub', 'boot-uidlan-0004-canary-value', 1"),
            ("db_agent_wan_keys_by_uid", "uid, public_key, private_key, created_at", "'boot-u', 'pub', 'boot-uidwan-0005-canary-value', 1"),
        ];
        {
            let conn = store.conn.lock().unwrap();
            for (table, columns, values) in rows {
                conn.execute(&format!("INSERT INTO {table} ({columns}) VALUES ({values})"), []).unwrap();
            }
        }
        let canaries = [
            "boot-jekt-0001-canary-value",
            "boot-lan-0002-canary-value",
            "boot-wan-0003-canary-value",
            "boot-uidlan-0004-canary-value",
            "boot-uidwan-0005-canary-value",
        ];
        for canary in canaries {
            assert_eq!(withhold::text(&format!("k {canary}")), format!("k {canary}"), "not registered yet");
        }
        assert!(store.withhold_stored_keys() >= 5);
        for canary in canaries {
            assert_eq!(withhold::text(&format!("k {canary}")), format!("k {}", withhold::MARK), "{canary}");
        }
    }

    #[test]
    fn a_store_with_no_keys_registers_none() {
        assert_eq!(Store::open_in_memory().unwrap().withhold_stored_keys(), 0);
    }
}
