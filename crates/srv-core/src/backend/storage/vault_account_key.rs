use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{error::StoreError, store::{SecretRef, Store}, vault::{row_of, VaultRow, COLUMNS}};

pub(super) fn matches_link(conn: &Connection, entry: &str, link_id: &str) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM db_vault_link_ids WHERE entry_id = ?1 AND link_id = ?2 UNION ALL SELECT 1 FROM db_vault_link_aliases WHERE entry_id = ?1 AND link_id = ?2)",
        params![entry, link_id], |row| row.get(0),
    )?)
}

impl Store {
    pub fn vault_account_key_row(&self, account_id: &str, reference: &SecretRef, candidate: &str, note: &str) -> Result<Option<(VaultRow, String)>, StoreError> {
        let SecretRef::Vault { entry, link_id } = reference else { return Ok(None) };
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(row) = tx.query_row(&format!("SELECT {COLUMNS} FROM db_vault_entries WHERE id = ?1"), params![entry], row_of).optional()? else {
            return Ok(None);
        };
        let mut bound = link_id.clone();
        if bound.is_none() {
            let stored: Option<String> = tx.query_row("SELECT secret_ref FROM db_accounts WHERE id = ?1", params![account_id], |row| row.get(0)).optional()?;
            match stored.as_deref().map(serde_json::from_str::<SecretRef>).transpose()? {
                Some(SecretRef::Vault { entry: saved, link_id }) if saved == *entry => bound = link_id,
                _ => return Ok(None),
            }
        }
        if let Some(link_id) = bound {
            return Ok(matches_link(&tx, entry, &link_id)?.then_some((row, link_id)));
        }
        let fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&row.fields)?;
        if row.kind != "api-key" || fields.get("notes").map(String::as_str) != Some(note) {
            return Ok(None);
        }
        let link_id: Option<String> = tx.query_row("SELECT link_id FROM db_vault_link_ids WHERE entry_id = ?1", params![entry], |row| row.get(0)).optional()?;
        let link_id = match link_id {
            Some(link_id) => link_id,
            None => {
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO db_vault_link_ids (entry_id, link_id) SELECT ?1, ?2 WHERE NOT EXISTS(SELECT 1 FROM db_vault_link_retired WHERE link_id = ?2) AND NOT EXISTS(SELECT 1 FROM db_vault_link_aliases WHERE link_id = ?2)",
                    params![entry, candidate],
                )?;
                if inserted == 0 { return Ok(None); }
                candidate.to_string()
            }
        };
        tx.execute("UPDATE db_accounts SET secret_ref = json_set(secret_ref, '$.link_id', ?2) WHERE id = ?1", params![account_id, link_id])?;
        tx.commit()?;
        tracing::info!(target: "identity", "account.key: bound a legacy Vault pointer to its entry identity");
        Ok(Some((row, link_id)))
    }
}
