// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The History pane became the Audit pane's Conversations section. Moves, once, for this channel:
//! - blocks (`db_block`) saved as `view: "history"` to `view: "audit"` on `audit:section: "conversations"`;
//! - the same for every block in the client's session-restore snapshot (`session:last_topology`);
//! - settings.json: `history` in `widget:pinned` is replaced, in place, by `audit` (which stays
//!   once if it was pinned too, at the place of whichever came first).
//!
//! An Audit pane that already exists is not touched, and a History pane keeps its other keys
//! (`term:zoom`, ...). `widgets.json` needs nothing: an entry that still names the History view is
//! resolved when the file is loaded (`WidgetConfigType::resolve_section`). Layout files live
//! outside AgentMux and are translated on import (`layout_file.rs`).
//!
//! Idempotent: a second run finds no History pane. The block and snapshot rewrite fails the
//! migration (retried on the next boot) when the store cannot be read.
//!
//! The helpers below are frozen copies of `m0044_warden_split.rs` as of 0.60.40 (same store and
//! settings reads), kept here so this migration produces the same rows whatever m0044 later becomes.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

#[cfg(not(any(test, feature = "test-support")))]
use crate::backend::config_watcher_fs::resolve_settings_dir;
use crate::backend::storage::store::Store;
use crate::backend::wconfig;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0052HistoryIntoAudit;

impl Migration for M0052HistoryIntoAudit {
    fn id(&self) -> &'static str {
        "0052_history_into_audit"
    }
    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }
    fn description(&self) -> &'static str {
        "Move saved History panes to the Audit pane's Conversations section"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            let store = Arc::new(
                Store::open(&ctx.channel_store_path)
                    .map_err(|e| MigrationError(format!("history_into_audit: open store: {e}")))?,
            );
            let blocks = migrate_blocks(&store).map_err(MigrationError)?;
            let snapshots = migrate_client_snapshots(&store).map_err(MigrationError)?;
            if blocks + snapshots > 0 {
                tracing::info!(
                    blocks,
                    snapshots,
                    "history_into_audit: moved saved History panes to Audit"
                );
            }
        }
        if let Some(settings_dir) = settings_dir() {
            migrate_settings_file(&settings_dir.join(wconfig::SETTINGS_FILE));
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
        match leftover_block_count(&store) {
            Ok(0) => VerifyOutcome::Ok("no block saved as a History pane".into()),
            Ok(n) => VerifyOutcome::Mismatch(format!("{n} block(s) still saved as a History pane")),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

/// Where settings.json lives. Not under test: the runner tests apply every migration, and the
/// real resolver would point them at the developer's own settings.
#[cfg(not(any(test, feature = "test-support")))]
fn settings_dir() -> Option<std::path::PathBuf> {
    Some(resolve_settings_dir())
}
#[cfg(any(test, feature = "test-support"))]
fn settings_dir() -> Option<std::path::PathBuf> {
    None
}

/// A pane meta saved as History, rewritten as Audit on Conversations. Returns whether it changed.
/// Every other key (`term:zoom`, ...) is kept; an `audit:section` already in it wins.
fn migrate_meta(meta: &mut Map<String, Value>) -> bool {
    if meta.get("view").and_then(Value::as_str) != Some("history") {
        return false;
    }
    meta.insert("view".into(), "audit".into());
    meta.entry("audit:section")
        .or_insert_with(|| "conversations".into());
    true
}

/// Every object in `value` saved as a History pane, rewritten. Returns how many changed.
fn migrate_tree(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut n = usize::from(migrate_meta(map));
            for child in map.values_mut() {
                n += migrate_tree(child);
            }
            n
        }
        Value::Array(items) => items.iter_mut().map(migrate_tree).sum(),
        _ => 0,
    }
}

/// `widget:pinned` with `history` replaced by `audit`: Audit once, at the place of whichever of the
/// two came first. None when it does not pin History (so a list with Audit only, or neither, is
/// untouched, and a second run changes nothing).
fn migrate_pinned(value: &Value) -> Option<Value> {
    let items = value.as_array()?;
    if !items.iter().any(|v| v.as_str() == Some("history")) {
        return None;
    }
    let mut out: Vec<Value> = Vec::new();
    let mut placed = false;
    for item in items {
        if matches!(item.as_str(), Some("history" | "audit")) {
            if !placed {
                out.push(Value::String("audit".into()));
                placed = true;
            }
        } else {
            out.push(item.clone());
        }
    }
    Some(Value::Array(out))
}

fn migrate_settings_file(path: &Path) {
    if !path.exists() {
        return;
    }
    // A file with a mistake in it is left as it is; the migration finds it again once it parses.
    let settings = match agentmux_common::settings_file::read(path) {
        Ok(Some(loaded)) => loaded.values,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "history_into_audit: settings.json can't be read; the History pin left");
            return;
        }
    };
    let Some(pinned) = settings.get("widget:pinned").and_then(migrate_pinned) else {
        return;
    };
    match agentmux_common::settings_file::update(path, wconfig::SETTINGS_TEMPLATE, |settings| {
        settings.insert("widget:pinned".into(), pinned.clone());
    }) {
        Ok(()) => {
            tracing::info!("history_into_audit: settings.json pin moved from History to Audit")
        }
        Err(e) => {
            tracing::warn!(error = %e, "history_into_audit: couldn't write settings.json; the History pin left")
        }
    }
}

// frozen copy of m0044_warden_split::read_rows as of 0.60.40
fn read_rows(store: &Store, table: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let conn = store.conn().lock().unwrap();
    let mut stmt = conn
        .prepare(&format!("SELECT oid, data FROM {table}"))
        .map_err(|e| format!("history_into_audit: read {table}: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| format!("history_into_audit: read {table}: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

fn migrate_blocks(store: &Store) -> Result<usize, String> {
    let mut changed = 0;
    for (oid, data) in read_rows(store, "db_block")? {
        let Ok(mut block) = serde_json::from_slice::<Value>(&data) else {
            continue;
        };
        let moved = match block.get_mut("meta") {
            Some(Value::Object(meta)) => migrate_meta(meta),
            _ => false,
        };
        if moved {
            store
                .update_raw("block", &oid, &block)
                .map_err(|e| format!("history_into_audit: update block {oid}: {e}"))?;
            changed += 1;
        }
    }
    Ok(changed)
}

const SNAPSHOT_META_KEY: &str = "session:last_topology";

fn migrate_client_snapshots(store: &Store) -> Result<usize, String> {
    let mut changed = 0;
    for (oid, data) in read_rows(store, "db_client")? {
        let Ok(mut client) = serde_json::from_slice::<Value>(&data) else {
            continue;
        };
        let moved = client
            .get_mut("meta")
            .and_then(|m| m.get_mut(SNAPSHOT_META_KEY))
            .map(migrate_tree)
            .unwrap_or(0);
        if moved > 0 {
            store
                .update_raw("client", &oid, &client)
                .map_err(|e| format!("history_into_audit: update client {oid}: {e}"))?;
            changed += moved;
        }
    }
    Ok(changed)
}

fn leftover_block_count(store: &Store) -> Result<usize, String> {
    Ok(read_rows(store, "db_block")?
        .into_iter()
        .filter(|(_, data)| {
            serde_json::from_slice::<Value>(data)
                .ok()
                .and_then(|b| b.get("meta")?.as_object().cloned())
                .is_some_and(|mut meta| migrate_meta(&mut meta))
        })
        .count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_history_pane_becomes_audit_on_conversations_keeping_its_other_keys() {
        let mut m = meta(json!({ "view": "history", "term:zoom": 1.1, "pane:title": "mine" }));
        assert!(migrate_meta(&mut m));
        assert_eq!(
            Value::Object(m),
            json!({ "view": "audit", "audit:section": "conversations", "term:zoom": 1.1, "pane:title": "mine" })
        );
    }

    #[test]
    fn a_section_already_in_the_meta_wins_and_a_second_run_changes_nothing() {
        let mut m = meta(json!({ "view": "history", "audit:section": "typed" }));
        assert!(migrate_meta(&mut m));
        assert_eq!(
            Value::Object(m.clone()),
            json!({ "view": "audit", "audit:section": "typed" })
        );
        assert!(!migrate_meta(&mut m));
    }

    #[test]
    fn other_panes_and_audit_panes_are_left_alone() {
        for m in [
            json!({ "view": "audit" }),
            json!({ "view": "audit", "audit:section": "storage" }),
            json!({ "view": "agent", "agent:history": "x" }),
            json!({ "view": "warden", "warden:section": "history" }),
            json!({ "view": "term" }),
        ] {
            assert!(!migrate_meta(&mut meta(m)));
        }
    }

    #[test]
    fn the_session_snapshot_moves_too() {
        let mut snapshot = json!({ "tabs": [{ "blocks": [
            { "view": "history" },
            { "view": "audit", "audit:section": "events" },
            { "view": "history", "term:zoom": 1.2 },
            { "view": "term" }
        ] }] });
        assert_eq!(migrate_tree(&mut snapshot), 2);
        let blocks = &snapshot["tabs"][0]["blocks"];
        assert_eq!(
            blocks[0],
            json!({ "view": "audit", "audit:section": "conversations" })
        );
        assert_eq!(
            blocks[1],
            json!({ "view": "audit", "audit:section": "events" })
        );
        assert_eq!(
            blocks[2],
            json!({ "view": "audit", "audit:section": "conversations", "term:zoom": 1.2 })
        );
        assert_eq!(migrate_tree(&mut snapshot), 0);
    }

    #[test]
    fn a_tab_holding_both_a_history_and_an_audit_pane_keeps_both() {
        let mut snapshot = json!({ "blocks": [{ "view": "history" }, { "view": "audit" }] });
        assert_eq!(migrate_tree(&mut snapshot), 1);
        assert_eq!(snapshot["blocks"][1], json!({ "view": "audit" }));
        assert_eq!(snapshot["blocks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn the_pins_hold_audit_once_where_the_first_of_the_two_was() {
        // History only.
        assert_eq!(
            migrate_pinned(&json!(["agent", "history", "term"])),
            Some(json!(["agent", "audit", "term"]))
        );
        // Both, either order.
        assert_eq!(
            migrate_pinned(&json!(["history", "term", "audit"])),
            Some(json!(["audit", "term"]))
        );
        assert_eq!(
            migrate_pinned(&json!(["audit", "term", "history"])),
            Some(json!(["audit", "term"]))
        );
        // History twice.
        assert_eq!(
            migrate_pinned(&json!(["history", "agent", "history"])),
            Some(json!(["audit", "agent"]))
        );
        // Audit only, neither, not a list: untouched; and a second run changes nothing.
        assert_eq!(migrate_pinned(&json!(["agent", "audit"])), None);
        assert_eq!(migrate_pinned(&json!(["agent", "term"])), None);
        assert_eq!(migrate_pinned(&json!([])), None);
        assert_eq!(migrate_pinned(&json!("history")), None);
        assert_eq!(migrate_pinned(&json!(["audit", "term"])), None);
    }

    #[test]
    fn settings_are_rewritten_only_when_history_was_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{ "widget:pinned": ["agent", "history"], "term:fontsize": 14 }"#,
        )
        .unwrap();
        migrate_settings_file(&settings);
        let after = wconfig::read_settings_raw_jsonc(&settings);
        assert_eq!(after["widget:pinned"], json!(["agent", "audit"]));
        assert_eq!(after["term:fontsize"], json!(14));

        // No History pin: the file is byte for byte as it was.
        let text = r#"{ "widget:pinned": ["agent", "audit"] }"#;
        std::fs::write(&settings, text).unwrap();
        migrate_settings_file(&settings);
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), text);

        // Unparseable: left alone. Missing: no file is created.
        std::fs::write(&settings, "{ not json").unwrap();
        migrate_settings_file(&settings);
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");
        let missing = dir.path().join("missing.json");
        migrate_settings_file(&missing);
        assert!(!missing.exists());
    }
}
