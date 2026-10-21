// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault became a section of Warden (operator decision 2026-10-21,
//! `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` §5.6 amendment). Moves, once, for
//! this channel:
//! - blocks (`db_block`) saved as `view: "vault"` to `view: "warden"` on its `vault` section,
//!   keeping the Vault's own section (`vault:section`) and an open request (`vault:request`);
//! - the same for every block in the client's session-restore snapshot (`session:last_topology`);
//! - widgets.json entries that open the `vault` view.
//!
//! `widget:pinned` needs nothing: the widget-bar entry keeps its key, `defwidget@vault`. Layout
//! files and `.abf` bundles live outside AgentMux and are translated on import
//! (`layout_file.rs`), and the frontend still opens a block saved as `vault` in Warden's Vault
//! section (Warden's `vault` alias).
//!
//! Idempotent: a second run finds no `vault` view. The widgets file is rewritten only when
//! something changed and left alone when it doesn't parse; the block and snapshot rewrite fails
//! the migration (retried on the next boot) when the store can't be read.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

#[cfg(not(test))]
use crate::backend::config_watcher_fs::resolve_settings_dir;
use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0039VaultIntoWarden;

impl Migration for M0039VaultIntoWarden {
    fn id(&self) -> &'static str { "0039_vault_into_warden" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Move saved Vault panes to Warden's Vault section" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            let store = Arc::new(
                Store::open(&ctx.channel_store_path)
                    .map_err(|e| MigrationError(format!("vault_into_warden: open store: {e}")))?,
            );
            let blocks = migrate_blocks(&store).map_err(MigrationError)?;
            let snapshots = migrate_client_snapshots(&store).map_err(MigrationError)?;
            if blocks + snapshots > 0 {
                tracing::info!(blocks, snapshots, "vault_into_warden: moved saved Vault panes to Warden");
            }
        }
        if let Some(settings_dir) = settings_dir() {
            migrate_widgets_file(&settings_dir.join(crate::backend::user_widgets::USER_WIDGETS_FILE));
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
        match vault_block_count(&store) {
            Ok(0) => VerifyOutcome::Ok("no block saved as a Vault pane".into()),
            Ok(n) => VerifyOutcome::Mismatch(format!("{n} block(s) still saved as a Vault pane")),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

/// Where widgets.json lives. Not under test, like m0037's: the runner tests apply every
/// migration, and the real resolver would point them at the developer's own settings.
#[cfg(not(test))]
fn settings_dir() -> Option<std::path::PathBuf> {
    Some(resolve_settings_dir())
}
#[cfg(test)]
fn settings_dir() -> Option<std::path::PathBuf> {
    None
}

/// A pane meta saved as the Vault, moved to Warden's Vault section. Returns whether it changed.
fn migrate_meta(meta: &mut Map<String, Value>) -> bool {
    if meta.get("view").and_then(Value::as_str) != Some("vault") {
        return false;
    }
    meta.insert("view".into(), "warden".into());
    meta.insert("warden:section".into(), "vault".into());
    true
}

/// Every object in `value` saved as the Vault, moved. Returns how many changed.
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

fn read_rows(store: &Store, table: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let conn = store.conn().lock().unwrap();
    let mut stmt = conn
        .prepare(&format!("SELECT oid, data FROM {table}"))
        .map_err(|e| format!("vault_into_warden: read {table}: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| format!("vault_into_warden: read {table}: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

fn migrate_blocks(store: &Store) -> Result<usize, String> {
    let mut changed = 0;
    for (oid, data) in read_rows(store, "db_block")? {
        let Ok(mut block) = serde_json::from_slice::<Value>(&data) else { continue };
        let moved = match block.get_mut("meta") {
            Some(Value::Object(meta)) => migrate_meta(meta),
            _ => false,
        };
        if moved {
            store
                .update_raw("block", &oid, &block)
                .map_err(|e| format!("vault_into_warden: update block {oid}: {e}"))?;
            changed += 1;
        }
    }
    Ok(changed)
}

const SNAPSHOT_META_KEY: &str = "session:last_topology";

fn migrate_client_snapshots(store: &Store) -> Result<usize, String> {
    let mut changed = 0;
    for (oid, data) in read_rows(store, "db_client")? {
        let Ok(mut client) = serde_json::from_slice::<Value>(&data) else { continue };
        let moved = client
            .get_mut("meta")
            .and_then(|m| m.get_mut(SNAPSHOT_META_KEY))
            .map(migrate_tree)
            .unwrap_or(0);
        if moved > 0 {
            store
                .update_raw("client", &oid, &client)
                .map_err(|e| format!("vault_into_warden: update client {oid}: {e}"))?;
            changed += moved;
        }
    }
    Ok(changed)
}

fn vault_block_count(store: &Store) -> Result<usize, String> {
    Ok(read_rows(store, "db_block")?
        .into_iter()
        .filter(|(_, data)| {
            serde_json::from_slice::<Value>(data)
                .ok()
                .and_then(|b| b.get("meta")?.get("view")?.as_str().map(|v| v == "vault"))
                .unwrap_or(false)
        })
        .count())
}

/// widgets.json with every entry that opens the `vault` view retargeted, or None when none does.
fn migrated_widgets(widgets: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut out = widgets.clone();
    let mut changed = false;
    for entry in out.values_mut() {
        if let Some(Value::Object(meta)) = entry.get_mut("blockdef").and_then(|b| b.get_mut("meta")) {
            changed |= migrate_meta(meta);
        }
    }
    changed.then_some(out)
}

fn migrate_widgets_file(path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    if content.trim().is_empty() {
        return;
    }
    let widgets: Map<String, Value> = match serde_json::from_str(&content) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "vault_into_warden: widgets.json doesn't parse; left alone");
            return;
        }
    };
    let Some(next) = migrated_widgets(&widgets) else { return };
    let text = match serde_json::to_string_pretty(&next) {
        Ok(t) => t + "\n",
        Err(_) => return,
    };
    match std::fs::write(path, text) {
        Ok(()) => tracing::info!("vault_into_warden: widgets.json moved to Warden's Vault section"),
        Err(e) => tracing::warn!(error = %e, "vault_into_warden: couldn't write widgets.json; left as it was"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_vault_pane_moves_to_wardens_vault_section_keeping_its_own_section() {
        let mut m = meta(json!({ "view": "vault", "vault:section": "cards", "vault:request": "r1", "term:zoom": 1.1 }));
        assert!(migrate_meta(&mut m));
        assert_eq!(
            Value::Object(m.clone()),
            json!({ "view": "warden", "warden:section": "vault", "vault:section": "cards", "vault:request": "r1", "term:zoom": 1.1 })
        );
        // Idempotent, and other panes are left alone.
        assert!(!migrate_meta(&mut m));
        assert!(!migrate_meta(&mut meta(json!({ "view": "warden", "warden:section": "host" }))));
        assert!(!migrate_meta(&mut meta(json!({ "view": "browser", "url": "https://vault.example" }))));
    }

    #[test]
    fn the_session_snapshot_and_widgets_file_move_too() {
        let mut snapshot = json!({ "tabs": [{ "blocks": [{ "view": "vault" }, { "view": "term" }] }] });
        assert_eq!(migrate_tree(&mut snapshot), 1);
        assert_eq!(snapshot["tabs"][0]["blocks"][0], json!({ "view": "warden", "warden:section": "vault" }));
        assert_eq!(migrate_tree(&mut snapshot), 0);

        let widgets = meta(json!({
            "my@cards": { "label": "Cards", "blockdef": { "meta": { "view": "vault", "vault:section": "cards" } } },
            "my@term": { "label": "Term", "blockdef": { "meta": { "view": "term" } } }
        }));
        let next = migrated_widgets(&widgets).unwrap();
        assert_eq!(next["my@cards"]["blockdef"]["meta"], json!({ "view": "warden", "warden:section": "vault", "vault:section": "cards" }));
        assert_eq!(next["my@term"], widgets["my@term"]);
        assert!(migrated_widgets(&next).is_none());
    }

    #[test]
    fn the_widgets_file_is_rewritten_only_when_something_moved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("widgets.json");
        std::fs::write(&path, r#"{"a@b":{"blockdef":{"meta":{"view":"vault"}}}}"#).unwrap();
        migrate_widgets_file(&path);
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["a@b"]["blockdef"]["meta"]["view"], "warden");
        // Unparseable: left alone.
        std::fs::write(&path, "{ not json").unwrap();
        migrate_widgets_file(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
