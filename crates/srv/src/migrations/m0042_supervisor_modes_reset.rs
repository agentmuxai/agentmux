// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Set every pane the Swarm Supervisor was supervising back to Off, once.
//!
//! The Supervisor became a setting that is off until the operator turns it on
//! (`supervisor:enabled`). Without this reset, turning it on would put back
//! every agent that was once set to Steady or Drive, which is the behaviour
//! that felt too active. A pane keeps its decision history; the reset adds one
//! line to it saying why the mode went to Off.
//! docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §5.
//!
//! The mode and state keys and the state's shape are a frozen copy, as of
//! 0.60.2, of `backend::supervisor` (`META_MODE`, `META_STATE`,
//! `Supervisor::set_mode(Off)`): a migration must not follow later changes to
//! them. Idempotent: a pane already Off is left alone, so a second run changes
//! nothing.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0042SupervisorModesReset;

/// Frozen copy of `supervisor::META_MODE`.
const META_MODE: &str = "supervisor:mode";
/// Frozen copy of `supervisor::META_STATE`.
const META_STATE: &str = "supervisor:state";
/// Frozen copy of `supervisor::HISTORY_MAX`.
const HISTORY_MAX: usize = 10;

const RESET_NOTE: &str = "Turned off: the Supervisor is now off until you turn it on in the settings.";

impl Migration for M0042SupervisorModesReset {
    fn id(&self) -> &'static str { "0042_supervisor_modes_reset" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Set panes the Swarm Supervisor was supervising (Steady or Drive) back to Off, keeping their history"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let store = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("supervisor_modes_reset: open store: {e}")))?,
        );
        let reset = reset_blocks(&store, agentmux_common::time::now_ms_u64()).map_err(MigrationError)?;
        if reset > 0 {
            tracing::info!(reset, "supervisor_modes_reset: set supervised panes back to Off");
        }
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.channel_store_path.exists() {
            return VerifyOutcome::Ok("no channel store".into());
        }
        let store = match Store::open(&ctx.channel_store_path) {
            Ok(s) => s,
            Err(e) => return VerifyOutcome::Error(format!("open store: {e}")),
        };
        // A pane may be set again after the reset ran, once the switch is on;
        // this only says the rewrite itself works, so nothing to compare.
        match read_rows(&store) {
            Ok(rows) => VerifyOutcome::Ok(format!("{} block(s) read", rows.len())),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

fn read_rows(store: &Store) -> Result<Vec<(String, Vec<u8>)>, String> {
    let conn = store.conn().lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT oid, data FROM db_block")
        .map_err(|e| format!("supervisor_modes_reset: read db_block: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| format!("supervisor_modes_reset: read db_block: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

/// Reset every supervised block; the number reset.
fn reset_blocks(store: &Store, now_ms: u64) -> Result<usize, String> {
    let mut reset = 0;
    for (oid, data) in read_rows(store)? {
        let Ok(mut block) = serde_json::from_slice::<Value>(&data) else { continue };
        let changed = match block.get_mut("meta") {
            Some(Value::Object(meta)) => reset_meta(meta, now_ms),
            _ => false,
        };
        if changed {
            store
                .update_raw("block", &oid, &block)
                .map_err(|e| format!("supervisor_modes_reset: update block {oid}: {e}"))?;
            reset += 1;
        }
    }
    Ok(reset)
}

/// Whether `meta` named Steady or Drive (and was changed to Off).
fn reset_meta(meta: &mut Map<String, Value>, now_ms: u64) -> bool {
    let supervised = matches!(
        meta.get(META_MODE).and_then(Value::as_str).map(|m| m.trim().to_ascii_lowercase()).as_deref(),
        Some("steady" | "drive")
    );
    if !supervised {
        return false;
    }
    meta.remove(META_MODE);
    // The state as `set_mode(Off)` leaves it: a fresh Off run, history kept,
    // one line added.
    let mut history = meta
        .get(META_STATE)
        .and_then(|s| s.get("history"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    history.push(json!({ "at_ms": now_ms, "action": "mode_set", "reason": RESET_NOTE }));
    if history.len() > HISTORY_MAX {
        history.drain(..history.len() - HISTORY_MAX);
    }
    meta.insert(
        META_STATE.to_string(),
        json!({
            "mode": "off",
            "status": "watching",
            "continuations": 0,
            "limit": 0,
            "run_started_at_ms": 0,
            "no_progress": 0,
            "updated_at_ms": now_ms,
            "history": history,
        }),
    );
    true
}

#[cfg(test)]
#[path = "m0042_supervisor_modes_reset_tests.rs"]
mod tests;
