// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Warden was split: the Vault, History and Audit are panes of their own, Host is the Swarm, and
//! Warden keeps only its Jobs section. Moves, once, for this channel:
//! - blocks (`db_block`) saved as `view: "warden"` to the pane their `warden:section` lives in now;
//! - the same for every block in the client's session-restore snapshot (`session:last_topology`);
//! - settings.json: `warden` in `widget:pinned` is replaced, in place, by `vault`, `history` and
//!   `audit` (those already pinned are not repeated).
//!
//! | saved Warden section         | becomes                                         |
//! |------------------------------|-------------------------------------------------|
//! | `vault`                      | `vault` (`vault:section`, `vault:request` kept) |
//! | `history`                    | `history`                                       |
//! | `audit`                      | `audit`                                         |
//! | `host`                       | `swarm`                                         |
//! | none, or unknown             | `audit` (read-only: a redirect nobody asked for never asks for the Vault key) |
//! | `kanban`                     | `kanban`                                        |
//! | `lan`, `internet`, `ambient` | `audit` (these sections have no pane)           |
//! | `jobs`                       | unchanged                                       |
//!
//! `widgets.json` needs nothing: an entry that still names a moved section is resolved when the
//! file is loaded (`WidgetConfigType::resolve_section`). Layout files live outside AgentMux and
//! are translated on import (`layout_file.rs`).
//!
//! Idempotent: a second run finds no Warden pane on a moved section. The block and snapshot
//! rewrite fails the migration (retried on the next boot) when the store cannot be read.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

#[cfg(not(test))]
use crate::backend::config_watcher_fs::resolve_settings_dir;
use crate::backend::storage::store::Store;
use crate::backend::wconfig;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0044WardenSplit;

impl Migration for M0044WardenSplit {
    fn id(&self) -> &'static str {
        "0044_warden_split"
    }
    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }
    fn description(&self) -> &'static str {
        "Move saved Warden panes to the Vault, History, Audit and Swarm panes"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            let store = Arc::new(
                Store::open(&ctx.channel_store_path)
                    .map_err(|e| MigrationError(format!("warden_split: open store: {e}")))?,
            );
            let blocks = migrate_blocks(&store).map_err(MigrationError)?;
            let snapshots = migrate_client_snapshots(&store).map_err(MigrationError)?;
            if blocks + snapshots > 0 {
                tracing::info!(
                    blocks,
                    snapshots,
                    "warden_split: moved saved Warden panes to their own panes"
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
            Ok(0) => VerifyOutcome::Ok("no block saved as a Warden pane on a moved section".into()),
            Ok(n) => VerifyOutcome::Mismatch(format!(
                "{n} block(s) still saved as a Warden pane on a moved section"
            )),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

/// Where settings.json lives. Not under test: the runner tests apply every migration, and the
/// real resolver would point them at the developer's own settings.
#[cfg(not(test))]
fn settings_dir() -> Option<std::path::PathBuf> {
    Some(resolve_settings_dir())
}
#[cfg(test)]
fn settings_dir() -> Option<std::path::PathBuf> {
    None
}

/// The pane a Warden pane saved on `section` opens now, or None when it stays Warden (Jobs).
fn target_view(section: Option<&str>) -> Option<&'static str> {
    match section.unwrap_or("") {
        "jobs" => None,
        "vault" => Some("vault"),
        "history" => Some("history"),
        "host" => Some("swarm"),
        "kanban" => Some("kanban"),
        _ => Some("audit"),
    }
}

/// A pane meta saved as Warden on a section that moved, rewritten for its new pane. Returns whether
/// it changed. Every other key (`vault:section`, `vault:request`, `term:zoom`, ...) is kept.
fn migrate_meta(meta: &mut Map<String, Value>) -> bool {
    if meta.get("view").and_then(Value::as_str) != Some("warden") {
        return false;
    }
    let Some(view) = target_view(meta.get("warden:section").and_then(Value::as_str)) else {
        return false;
    };
    meta.insert("view".into(), view.into());
    meta.remove("warden:section");
    true
}

/// Every object in `value` saved as a Warden pane on a moved section, rewritten. Returns how many changed.
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

/// The panes that took over Warden's place in the widget bar.
const PINNED_REPLACEMENT: [&str; 3] = ["vault", "history", "audit"];

/// `widget:pinned` with `warden` replaced in place by the Vault, History and Audit (those already
/// pinned are not repeated), or None when it does not pin Warden.
fn migrate_pinned(value: &Value) -> Option<Value> {
    let items = value.as_array()?;
    if !items.iter().any(|v| v.as_str() == Some("warden")) {
        return None;
    }
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        if item.as_str() != Some("warden") {
            out.push(item.clone());
            continue;
        }
        for name in PINNED_REPLACEMENT {
            let pinned = |v: &Value| v.as_str() == Some(name);
            if !items.iter().any(pinned) && !out.iter().any(pinned) {
                out.push(Value::String(name.into()));
            }
        }
    }
    Some(Value::Array(out))
}

fn migrate_settings_file(path: &Path) {
    if !path.exists() {
        return;
    }
    let mut settings = wconfig::read_settings_raw_jsonc(path);
    let Some(pinned) = settings.get("widget:pinned").and_then(migrate_pinned) else {
        return;
    };
    settings.insert("widget:pinned".into(), pinned);
    let merged = wconfig::merge_into_template(wconfig::SETTINGS_TEMPLATE, &settings);
    match std::fs::write(path, merged) {
        Ok(()) => tracing::info!(
            "warden_split: settings.json pins moved from Warden to the Vault, History and Audit"
        ),
        Err(e) => {
            tracing::warn!(error = %e, "warden_split: couldn't write settings.json; the Warden pin left")
        }
    }
}

fn read_rows(store: &Store, table: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let conn = store.conn().lock().unwrap();
    let mut stmt = conn
        .prepare(&format!("SELECT oid, data FROM {table}"))
        .map_err(|e| format!("warden_split: read {table}: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| format!("warden_split: read {table}: {e}"))?
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
                .map_err(|e| format!("warden_split: update block {oid}: {e}"))?;
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
                .map_err(|e| format!("warden_split: update client {oid}: {e}"))?;
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

    fn moved(v: Value) -> Value {
        let mut m = meta(v);
        assert!(migrate_meta(&mut m));
        Value::Object(m)
    }

    #[test]
    fn a_vault_section_becomes_a_vault_pane_keeping_its_own_keys() {
        let out = moved(json!({
            "view": "warden", "warden:section": "vault", "vault:section": "cards", "vault:request": "r1", "term:zoom": 1.1
        }));
        assert_eq!(
            out,
            json!({ "view": "vault", "vault:section": "cards", "vault:request": "r1", "term:zoom": 1.1 })
        );
    }

    #[test]
    fn history_and_audit_become_panes_of_their_own() {
        assert_eq!(
            moved(json!({ "view": "warden", "warden:section": "history" })),
            json!({ "view": "history" })
        );
        assert_eq!(
            moved(json!({ "view": "warden", "warden:section": "audit" })),
            json!({ "view": "audit" })
        );
    }

    #[test]
    fn host_becomes_the_swarm() {
        assert_eq!(
            moved(json!({ "view": "warden", "warden:section": "host" })),
            json!({ "view": "swarm" })
        );
    }

    #[test]
    fn no_section_or_an_unknown_one_opens_the_audit() {
        for meta in [
            json!({ "view": "warden" }),
            json!({ "view": "warden", "warden:section": "" }),
            json!({ "view": "warden", "warden:section": "nope" }),
        ] {
            assert_eq!(moved(meta), json!({ "view": "audit" }));
        }
    }

    #[test]
    fn the_kanban_section_becomes_the_kanban_pane() {
        assert_eq!(
            moved(json!({ "view": "warden", "warden:section": "kanban" })),
            json!({ "view": "kanban" })
        );
    }

    #[test]
    fn sections_with_no_pane_open_the_audit() {
        for section in ["lan", "internet", "ambient"] {
            assert_eq!(
                moved(json!({ "view": "warden", "warden:section": section })),
                json!({ "view": "audit" }),
                "{section}"
            );
        }
    }

    #[test]
    fn jobs_and_other_panes_are_left_alone_and_a_second_run_changes_nothing() {
        assert!(!migrate_meta(&mut meta(
            json!({ "view": "warden", "warden:section": "jobs" })
        )));
        assert!(!migrate_meta(&mut meta(
            json!({ "view": "browser", "warden:section": "host" })
        )));
        assert!(!migrate_meta(&mut meta(
            json!({ "view": "vault", "vault:section": "cards" })
        )));
        assert!(!migrate_meta(&mut meta(json!({ "view": "swarm" }))));
        let mut m = meta(json!({ "view": "warden", "warden:section": "history" }));
        assert!(migrate_meta(&mut m));
        assert!(!migrate_meta(&mut m));
    }

    #[test]
    fn the_session_snapshot_moves_too() {
        let mut snapshot = json!({ "tabs": [{ "blocks": [
            { "view": "warden", "warden:section": "audit" },
            { "view": "warden", "warden:section": "jobs" },
            { "view": "term" }
        ] }] });
        assert_eq!(migrate_tree(&mut snapshot), 1);
        assert_eq!(snapshot["tabs"][0]["blocks"][0], json!({ "view": "audit" }));
        assert_eq!(
            snapshot["tabs"][0]["blocks"][1],
            json!({ "view": "warden", "warden:section": "jobs" })
        );
        assert_eq!(migrate_tree(&mut snapshot), 0);
    }

    #[test]
    fn a_warden_pin_is_replaced_in_place_by_the_three_panes() {
        let pinned = json!(["agent", "warden", "term"]);
        assert_eq!(
            migrate_pinned(&pinned),
            Some(json!(["agent", "vault", "history", "audit", "term"]))
        );
        // Already pinned panes are not repeated, wherever they sit.
        assert_eq!(
            migrate_pinned(&json!(["history", "warden"])),
            Some(json!(["history", "vault", "audit"]))
        );
        assert_eq!(
            migrate_pinned(&json!(["warden", "vault", "audit"])),
            Some(json!(["history", "vault", "audit"]))
        );
        // Nothing to do without a Warden pin, and a second run changes nothing.
        assert_eq!(migrate_pinned(&json!(["agent", "term"])), None);
        assert_eq!(migrate_pinned(&json!([])), None);
        assert_eq!(migrate_pinned(&json!("warden")), None);
        assert_eq!(
            migrate_pinned(&json!(["agent", "vault", "history", "audit", "term"])),
            None
        );
    }

    #[test]
    fn settings_are_rewritten_only_when_warden_was_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{ "widget:pinned": ["agent", "warden"], "term:fontsize": 14 }"#,
        )
        .unwrap();
        migrate_settings_file(&settings);
        let after = wconfig::read_settings_raw_jsonc(&settings);
        assert_eq!(
            after["widget:pinned"],
            json!(["agent", "vault", "history", "audit"])
        );
        assert_eq!(after["term:fontsize"], json!(14));

        // No Warden pin: the file is byte for byte as it was.
        let text = r#"{ "widget:pinned": ["agent"] }"#;
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
