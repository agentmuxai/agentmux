// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's earlier values
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.4).
//!
//! A new value (an edit, a capture, later a rotation or a browser save) becomes
//! the current one; the value it replaces is kept for 7 days, marked when and by
//! whom it was replaced, and the person can put it back from the Vault pane. Only
//! the current value is ever filled or delivered. A version is the entry's sealed
//! blob as it was: nothing here opens one except to show its last characters.

use agentmux_common::time::now_ms;
use serde::Serialize;

use super::{kinds, record, seal};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::storage::vault_versions::VaultVersionRow;

/// How long a replaced value is kept (§12, decision 8).
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const PREVIOUS: &str = "previous";

/// An earlier value as the pane shows it: when it was replaced and by whom, and
/// its last characters as the entry's own value shows them. No value.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultVersionInfo {
    #[ts(type = "number")]
    pub seq: i64,
    #[ts(type = "number")]
    pub replaced_at: i64,
    pub replaced_by: String,
    pub how: String,
    pub tail: String,
}

/// `row`'s current value as the version `by` replaces it with (`how`:
/// "replaced", "captured", …).
pub(super) fn of(row: &VaultRow, by: &str, how: &str) -> VaultVersionRow {
    VaultVersionRow {
        seq: 0,
        entry_id: row.id.clone(),
        entry_created_at: row.created_at,
        sealed: row.sealed.clone(),
        replaced_at: now_ms(),
        replaced_by: by.to_string(),
        how: how.to_string(),
        state: PREVIOUS.to_string(),
    }
}

/// Deletes what is past its 7 days; a failure is logged, not raised.
pub fn prune(store: &Store) {
    if let Err(e) = store.vault_versions_prune(now_ms() - KEEP_MS) {
        tracing::warn!(error = %e, "vault: couldn't delete old versions");
    }
}

/// The entry's earlier values, newest first.
pub fn list(store: &Store, id: &str) -> Result<Vec<VaultVersionInfo>, String> {
    prune(store);
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {id} has an unknown kind"))?;
    let versions = store.vault_versions(&row.id, row.created_at).map_err(|e| e.to_string())?;
    let key = if versions.is_empty() { None } else { Some(seal::key(false)?) };
    Ok(versions
        .into_iter()
        .map(|v| {
            let tail = key
                .as_ref()
                .and_then(|k| seal::open(k, &row.id, &v.sealed).ok())
                .and_then(|parts| parts.get(kind.parts[0]).map(|main| kinds::tail(kind, main)))
                .unwrap_or_default();
            VaultVersionInfo { seq: v.seq, replaced_at: v.replaced_at, replaced_by: v.replaced_by, how: v.how, tail }
        })
        .collect())
}

/// Puts version `seq` back as the entry's value, for the person; the value it
/// replaces is kept like any other.
pub fn restore(store: &Store, id: &str, seq: i64) -> Result<(), String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    let chosen = store.vault_version_get(seq).map_err(|e| e.to_string())?;
    let kept_since = now_ms() - KEEP_MS;
    let Some(chosen) = chosen.filter(|v| v.entry_id == row.id && v.entry_created_at == row.created_at && v.state == PREVIOUS && v.replaced_at >= kept_since) else {
        return Err(format!("{} has no earlier value {seq}: it may be past its 7 days", row.label));
    };
    let now = now_ms();
    let current = VaultVersionRow {
        seq: 0,
        entry_id: row.id.clone(),
        entry_created_at: row.created_at,
        sealed: row.sealed.clone(),
        replaced_at: now,
        replaced_by: super::PERSON.to_string(),
        how: "restored".to_string(),
        state: PREVIOUS.to_string(),
    };
    if !store.vault_version_swap(&row.id, chosen.seq, &current, now, kept_since).map_err(|e| e.to_string())? {
        return Err(format!("{} changed while restoring: nothing was changed", row.label));
    }
    let when = chrono::DateTime::from_timestamp_millis(chosen.replaced_at)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    record(store, &row.id, &row.label, super::PERSON, "restored", &format!("the value replaced {when}"), "done");
    Ok(())
}

#[cfg(test)]
#[path = "versions_tests.rs"]
mod tests;
