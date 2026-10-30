// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's earlier values
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.4).
//!
//! A new value (an edit, a capture, later a rotation or a browser save) becomes
//! the current one; the value it replaces is kept for 7 days, marked when and by
//! whom it was replaced, and the person can put it back from the Vault pane. Only
//! the current value is ever filled or delivered, except a version a command names
//! (`id@previous`, a rotation's `id@pending`). A version is the entry's sealed blob
//! as it was: nothing here opens one except to show its last characters or to
//! hand it to such a command.
//!
//! A rotation (`Vault` `rotate`) keeps its new value as a **pending** version until
//! its commands succeed, then makes it current; if they fail, the pending version
//! stays for 7 days, marked with the step that failed, and the person can put it
//! in from the pane (the service may already hold it).

use agentmux_common::time::now_ms;
use serde::Serialize;

use super::{kinds, record, seal};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::storage::vault_versions::VaultVersionRow;

/// How long a replaced value is kept (§12, decision 8).
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const PREVIOUS: &str = "previous";
/// A rotation's new value, not yet the entry's.
pub const PENDING: &str = "pending";
/// A pending value whose rotation never finished (srv stopped during it).
pub const INTERRUPTED: &str = "rotation interrupted";
/// A pending value that wasn't made current because the entry changed meanwhile.
pub const OVERTAKEN: &str = "rotation failed: the entry changed meanwhile";
/// How long a rotation may run before its pending value counts as interrupted.
const ROTATION_MS: i64 = 30 * 60 * 1000;

/// A pending value whose rotation failed at `step` (2: `set`, 3: `verify`).
pub fn failed_at(step: u8) -> String {
    format!("rotation failed at step {step}")
}

/// A version the person may put back: an earlier value, or a rotation's value
/// that didn't become current (the service may hold it).
fn restorable(state: &str) -> bool {
    state == PREVIOUS || state == INTERRUPTED || state.starts_with("rotation failed")
}

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
    /// "previous", "pending", "rotation failed at step 2", "rotation interrupted".
    pub state: String,
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

/// Deletes what is past its 7 days, and marks a rotation's pending value that
/// has waited longer than any rotation runs as interrupted (its request was
/// dropped, or srv stopped): the person can then put it in. A failure is
/// logged, not raised.
pub fn prune(store: &Store) {
    if let Err(e) = store.vault_versions_restate_before(PENDING, now_ms() - ROTATION_MS, INTERRUPTED) {
        tracing::warn!(error = %e, "vault: couldn't mark interrupted rotations");
    }
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
            VaultVersionInfo { seq: v.seq, replaced_at: v.replaced_at, replaced_by: v.replaced_by, how: v.how, tail, state: v.state }
        })
        .collect())
}

/// Puts version `seq` back as the entry's value, for the person; the value it
/// replaces is kept like any other.
pub fn restore(store: &Store, id: &str, seq: i64) -> Result<(), String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    let chosen = store.vault_version_get(seq).map_err(|e| e.to_string())?;
    let kept_since = now_ms() - KEEP_MS;
    let Some(chosen) = chosen.filter(|v| v.entry_id == row.id && v.entry_created_at == row.created_at && restorable(&v.state) && v.replaced_at >= kept_since) else {
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

/// A rotation under way: its pending version, and the entry as it was read.
#[derive(Debug)]
pub struct Rotation {
    pub seq: i64,
    pub label: String,
    created_at: i64,
    /// The entry's value when the rotation started: if anything else writes it
    /// meanwhile, the rotation doesn't overwrite that.
    was: Vec<u8>,
}

/// Starts a rotation of entry `id`: `value` goes into its main part as a pending
/// version that nothing is given yet. Refused while another rotation of it runs.
pub fn start_rotation(store: &Store, id: &str, value: &str, agent: &str) -> Result<Rotation, String> {
    let row = store.vault_get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {id}"))?;
    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("entry {id} has an unknown kind"))?;
    let key = seal::key(false)?;
    let mut parts = seal::open(&key, &row.id, &row.sealed)?;
    if let Some(mut old) = parts.0.insert(kind.parts[0].to_string(), value.to_string()) {
        zeroize::Zeroize::zeroize(&mut old);
    }
    let now = now_ms();
    let pending = VaultVersionRow {
        seq: 0,
        entry_id: row.id.clone(),
        entry_created_at: row.created_at,
        sealed: seal::seal(&key, &row.id, &parts)?,
        replaced_at: now,
        replaced_by: agent.to_string(),
        how: "rotated".to_string(),
        state: PENDING.to_string(),
    };
    prune(store);
    match store.vault_version_pending(&pending, now - ROTATION_MS, INTERRUPTED).map_err(|e| e.to_string())? {
        Some(seq) => Ok(Rotation { seq, label: row.label, created_at: row.created_at, was: row.sealed }),
        None => Err(format!("{} is being rotated already: wait for that rotation to finish", row.label)),
    }
}

/// Ends a rotation: `Ok` makes its pending value the entry's, the value it
/// replaces kept like any other; `Err(step)` keeps it, marked with the step that
/// failed. An entry changed meanwhile is never overwritten: the pending value is
/// kept, marked, and the rotation fails.
pub fn finish_rotation(store: &Store, id: &str, r: &Rotation, outcome: Result<(), u8>, agent: &str) -> Result<(), String> {
    if let Err(step) = outcome {
        store.vault_version_mark(r.seq, &failed_at(step)).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let changed = || format!("{} changed while it was rotated: the new value is kept as a version", r.label);
    let row = store.vault_get(id).map_err(|e| e.to_string())?.filter(|row| row.created_at == r.created_at).ok_or_else(changed)?;
    let now = now_ms();
    // Swapped only if the entry still holds the value the rotation started from.
    let previous = VaultVersionRow { sealed: r.was.clone(), ..of(&row, agent, "rotated") };
    if !store.vault_version_swap(&row.id, r.seq, &previous, now, now - KEEP_MS).map_err(|e| e.to_string())? {
        let _ = store.vault_version_mark(r.seq, OVERTAKEN);
        return Err(changed());
    }
    Ok(())
}

/// Drops a rotation whose `set` never started: no service saw its value, so
/// there is nothing to keep or put back.
pub fn abandon_rotation(store: &Store, r: &Rotation) -> Result<(), String> {
    store.vault_version_delete(r.seq).map_err(|e| e.to_string())
}

/// Version `seq` of entry `id` (this incarnation), for a command that names it:
/// its sealed blob, or why it can't be had.
pub fn sealed_version(store: &Store, row: &VaultRow, which: &str) -> Result<(i64, Vec<u8>), String> {
    let versions = store.vault_versions(&row.id, row.created_at).map_err(|e| e.to_string())?;
    let kept_since = now_ms() - KEEP_MS;
    let found = match which {
        "previous" => versions.into_iter().find(|v| v.state == PREVIOUS && v.replaced_at >= kept_since),
        "pending" => versions.into_iter().find(|v| v.state == PENDING),
        other => return Err(format!("{}@{other}: name @previous or @pending", row.id)),
    };
    found.map(|v| (v.seq, v.sealed)).ok_or_else(|| format!("{} has no {which} value", row.label))
}

#[cfg(test)]
#[path = "versions_tests.rs"]
mod tests;
