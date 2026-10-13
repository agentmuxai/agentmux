// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Rewrites the retired pane names in stored data, once, so the frontend and srv can drop the
//! shims that translated them on every read. The Armory (and the Trust Center before it) was
//! split into Connectors and Memory, the Knowledge pane was renamed Memory, and Remotes became a
//! section of Connectors (`SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md`
//! Phase 3, `SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md`, `SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md`).
//!
//! For this channel:
//! - blocks (`db_block`) saved as `view: "armory"`, `"trust"`, `"knowledge"` or `"remotes"` move to
//!   `connectors` or `memory`, on the section they were on;
//! - the same for every block in the client's session-restore snapshot (`session:last_topology`);
//! - settings.json: old names in `widget:pinned` and `widget:hidden`, old `pane:colors` keys, and
//!   keybindings naming `app:knowledge` or `app:identity` or a retired `viewType`;
//! - widgets.json: entries under `defwidget@knowledge` or `defwidget@remotes`, and entries that open
//!   a retired view.
//!
//! Layout files and `.abf` bundles live outside AgentMux and are still translated on import.
//!
//! Idempotent: a second run finds no old names. The settings and widget files are rewritten only
//! when something changed. A file that can't be read or parsed is left alone (logged), since
//! srv's own loaders already treat it as unreadable; the block and snapshot rewrite fails the
//! migration (retried on the next boot) when the store can't be read.
//!
//! The mapping below is a frozen copy of `armoryTarget` / `armoryMigrationPatch`
//! (frontend/app/view/section-pane/panes.ts), the Remotes manifest's patch
//! (frontend/app/view/remotes/remotes.tsx), the Knowledge alias and section-key fallback
//! (frontend/app/view/memory/memory.tsx), `RENAMED_PINS` (frontend/app/window/action-widgets-config.ts)
//! and `RENAMED_WIDGETS` / `retarget_remotes` (crates/srv/src/backend/user_widgets.rs) as of
//! 0.59.21, the release these shims are removed after.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

#[cfg(not(test))]
use crate::backend::config_watcher_fs::resolve_settings_dir;
use crate::backend::storage::store::Store;
use crate::backend::wconfig;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0037RetireOldPaneNames;

impl Migration for M0037RetireOldPaneNames {
    fn id(&self) -> &'static str { "0037_retire_old_pane_names" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Move saved Armory, Knowledge and Remotes panes, pins, colors and keybindings to Connectors and Memory"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            let store = Arc::new(
                Store::open(&ctx.channel_store_path)
                    .map_err(|e| MigrationError(format!("retire_old_pane_names: open store: {e}")))?,
            );
            let blocks = migrate_blocks(&store).map_err(MigrationError)?;
            let snapshots = migrate_client_snapshots(&store).map_err(MigrationError)?;
            if blocks + snapshots > 0 {
                tracing::info!(blocks, snapshots, "retire_old_pane_names: moved saved panes to Connectors and Memory");
            }
        }
        if let Some(settings_dir) = settings_dir() {
            migrate_settings_file(&settings_dir.join(wconfig::SETTINGS_FILE));
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
        match old_view_block_count(&store) {
            Ok(0) => VerifyOutcome::Ok("no block saved under a retired pane name".into()),
            Ok(n) => VerifyOutcome::Mismatch(format!("{n} block(s) still saved under a retired pane name")),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

/// Where settings.json and widgets.json live. Not under test: the migration-runner tests apply
/// every registered migration, and the real resolver would point them at the developer's own
/// settings. The file rewrites are tested directly with explicit paths.
#[cfg(not(test))]
fn settings_dir() -> Option<std::path::PathBuf> {
    Some(resolve_settings_dir())
}
#[cfg(test)]
fn settings_dir() -> Option<std::path::PathBuf> {
    None
}

const CONNECTORS: &str = "connectors";
const MEMORY: &str = "memory";
const CONNECTORS_SECTION: &str = "connectors:section";
const MEMORY_SECTION: &str = "memory:section";
const RETIRED_VIEWS: &[&str] = &["armory", "trust", "knowledge", "remotes"];

/// A pane meta saved under a retired view, moved to its new pane. Returns whether it changed.
fn migrate_meta(meta: &mut Map<String, Value>) -> bool {
    let view = match meta.get("view").and_then(Value::as_str) {
        Some(v) if RETIRED_VIEWS.contains(&v) => v.to_string(),
        _ => return false,
    };
    match view.as_str() {
        "armory" | "trust" => {
            // Frozen copy of armoryTarget/armoryMigrationPatch (panes.ts).
            let section = meta.get("armory:section").and_then(Value::as_str).map(str::to_string);
            let sub = meta.get("armory:memory:subsection").and_then(Value::as_str).map(str::to_string);
            let (target_view, key, target_section) = match section.as_deref() {
                Some("mcp") => (CONNECTORS, CONNECTORS_SECTION, "mcp"),
                Some("memory") => (MEMORY, MEMORY_SECTION, if sub.as_deref() == Some("personal") { "personal" } else { "global" }),
                Some("native_memory") => (MEMORY, MEMORY_SECTION, "personal"),
                Some("skills") => (MEMORY, MEMORY_SECTION, "skills"),
                Some("bundles") => (MEMORY, MEMORY_SECTION, "bundles"),
                _ => (CONNECTORS, CONNECTORS_SECTION, "accounts"),
            };
            meta.insert("view".into(), target_view.into());
            meta.insert(key.into(), target_section.into());
            meta.remove("armory:section");
            meta.remove("armory:memory:subsection");
        }
        "knowledge" => {
            meta.insert("view".into(), MEMORY.into());
            if let Some(section) = meta.remove("knowledge:section") {
                meta.entry(MEMORY_SECTION.to_string()).or_insert(section);
            }
        }
        "remotes" => {
            // Frozen copy of the Remotes manifest's patch: Connectors on its Remotes section,
            // keeping `remotes:expand`.
            meta.insert("view".into(), CONNECTORS.into());
            meta.insert(CONNECTORS_SECTION.into(), "remotes".into());
        }
        _ => unreachable!(),
    }
    true
}

/// Every object in `value` that carries a retired `view`, moved. Returns how many changed.
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
        .map_err(|e| format!("retire_old_pane_names: read {table}: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| format!("retire_old_pane_names: read {table}: {e}"))?
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
                .map_err(|e| format!("retire_old_pane_names: update block {oid}: {e}"))?;
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
                .map_err(|e| format!("retire_old_pane_names: update client {oid}: {e}"))?;
            changed += moved;
        }
    }
    Ok(changed)
}

fn old_view_block_count(store: &Store) -> Result<usize, String> {
    Ok(read_rows(store, "db_block")?
        .into_iter()
        .filter(|(_, data)| {
            serde_json::from_slice::<Value>(data)
                .ok()
                .and_then(|b| b.get("meta")?.get("view")?.as_str().map(|v| RETIRED_VIEWS.contains(&v)))
                .unwrap_or(false)
        })
        .count())
}

// ── settings.json ─────────────────────────────────────────────────

/// Frozen copy of RENAMED_PINS (action-widgets-config.ts): a pin's short name, as it reads now.
fn current_names(short: &str) -> Vec<&str> {
    match short {
        "armory" => vec![CONNECTORS, MEMORY],
        "knowledge" => vec![MEMORY],
        "remotes" => vec![CONNECTORS],
        other => vec![other],
    }
}

fn dedup(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// `widget:pinned` (short names) and `widget:hidden` (full keys) with old names mapped.
fn migrate_widget_list(value: &Value, full_keys: bool) -> Option<Value> {
    let items = value.as_array()?;
    let mut out = Vec::new();
    let mut changed = false;
    for item in items {
        let Some(s) = item.as_str() else {
            out.push(item.clone());
            continue;
        };
        let (prefix, short) = match (full_keys, s.strip_prefix("defwidget@")) {
            (true, Some(short)) => ("defwidget@", short),
            (true, None) => ("", s),
            (false, _) => ("", s),
        };
        let names = if !full_keys || !prefix.is_empty() { current_names(short) } else { vec![short] };
        if names != [short] {
            changed = true;
        }
        out.extend(names.into_iter().map(|n| Value::String(format!("{prefix}{n}"))));
    }
    if !changed {
        return None;
    }
    let strings: Vec<String> = out.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    Some(Value::Array(dedup(strings).into_iter().map(Value::String).collect()))
}

/// `pane:colors` with a color saved under a retired view carried to its new view, unless the new
/// view already has one. The Armory's own color (`armory`, `trust`) followed Connectors, where
/// the Armory opened.
fn migrate_colors(value: &Value) -> Option<Value> {
    let colors = value.as_object()?;
    let renames = [("knowledge", MEMORY), ("remotes", CONNECTORS), ("trust", CONNECTORS), ("armory", CONNECTORS)];
    if !renames.iter().any(|(old, _)| colors.contains_key(*old)) {
        return None;
    }
    let mut out = colors.clone();
    for (old, new) in renames {
        if let Some(color) = out.remove(old) {
            out.entry(new.to_string()).or_insert(color);
        }
    }
    Some(Value::Object(out))
}

fn migrate_command(command: &str) -> Option<String> {
    let (unbind, id) = match command.strip_prefix('-') {
        Some(id) => ("-", id),
        None => ("", command),
    };
    let new = match id {
        "app:knowledge" => "app:memory",
        "app:identity" => "app:connectors",
        _ => return None,
    };
    Some(format!("{unbind}{new}"))
}

fn migrate_when(when: &str) -> Option<String> {
    let mut out = when.to_string();
    for (old, new) in [("knowledge", MEMORY), ("remotes", CONNECTORS), ("armory", CONNECTORS), ("trust", CONNECTORS)] {
        for op in ["==", "!="] {
            out = out.replace(&format!("viewType {op} {old}"), &format!("viewType {op} {new}"));
        }
    }
    (out != when).then_some(out)
}

/// Keybinding entries naming a retired command or view.
fn migrate_keybindings(value: &Value) -> Option<Value> {
    let entries = value.as_array()?;
    let mut out = entries.clone();
    let mut changed = false;
    for entry in out.iter_mut() {
        let Some(obj) = entry.as_object_mut() else { continue };
        if let Some(new) = obj.get("command").and_then(Value::as_str).and_then(migrate_command) {
            obj.insert("command".into(), new.into());
            changed = true;
        }
        if let Some(new) = obj.get("when").and_then(Value::as_str).and_then(migrate_when) {
            obj.insert("when".into(), new.into());
            changed = true;
        }
    }
    changed.then_some(Value::Array(out))
}

/// The settings keys that change, with their new values; empty when nothing does.
fn migrated_settings(settings: &Map<String, Value>) -> Map<String, Value> {
    let mut changed = Map::new();
    if let Some(v) = settings.get("widget:pinned").and_then(|v| migrate_widget_list(v, false)) {
        changed.insert("widget:pinned".into(), v);
    }
    if let Some(v) = settings.get("widget:hidden").and_then(|v| migrate_widget_list(v, true)) {
        changed.insert("widget:hidden".into(), v);
    }
    if let Some(v) = settings.get("pane:colors").and_then(migrate_colors) {
        changed.insert("pane:colors".into(), v);
    }
    if let Some(v) = settings.get("keybindings").and_then(migrate_keybindings) {
        changed.insert("keybindings".into(), v);
    }
    changed
}

fn migrate_settings_file(path: &Path) {
    if !path.exists() {
        return;
    }
    let mut settings = wconfig::read_settings_raw_jsonc(path);
    let changed = migrated_settings(&settings);
    if changed.is_empty() {
        return;
    }
    let keys: Vec<String> = changed.keys().cloned().collect();
    settings.extend(changed);
    let merged = wconfig::merge_into_template(wconfig::SETTINGS_TEMPLATE, &settings);
    match std::fs::write(path, merged) {
        Ok(()) => tracing::info!(?keys, "retire_old_pane_names: settings.json moved to Connectors and Memory"),
        Err(e) => tracing::warn!(error = %e, "retire_old_pane_names: couldn't write settings.json; old names left"),
    }
}

// ── widgets.json ──────────────────────────────────────────────────

/// widgets.json with entries under a retired key moved to the new key (an entry the user already
/// wrote under the new key wins), and any entry that opens a retired view retargeted.
fn migrated_widgets(widgets: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut out = widgets.clone();
    let mut changed = false;
    for (old, new) in [("defwidget@knowledge", "defwidget@memory"), ("defwidget@remotes", "defwidget@connectors")] {
        if let Some(entry) = out.remove(old) {
            changed = true;
            out.entry(new.to_string()).or_insert(entry);
        }
    }
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
            tracing::warn!(error = %e, "retire_old_pane_names: widgets.json doesn't parse; left alone");
            return;
        }
    };
    let Some(next) = migrated_widgets(&widgets) else { return };
    let text = match serde_json::to_string_pretty(&next) {
        Ok(t) => t + "\n",
        Err(_) => return,
    };
    match std::fs::write(path, text) {
        Ok(()) => tracing::info!("retire_old_pane_names: widgets.json moved to Connectors and Memory"),
        Err(e) => tracing::warn!(error = %e, "retire_old_pane_names: couldn't write widgets.json; old names left"),
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
    fn armory_and_trust_go_where_their_section_was() {
        let mut m = meta(json!({"view": "armory", "armory:section": "skills", "term:zoom": 1.2}));
        assert!(migrate_meta(&mut m));
        assert_eq!(Value::Object(m), json!({"view": "memory", "memory:section": "skills", "term:zoom": 1.2}));

        let mut m = meta(json!({"view": "armory", "armory:section": "memory", "armory:memory:subsection": "personal"}));
        migrate_meta(&mut m);
        assert_eq!(Value::Object(m), json!({"view": "memory", "memory:section": "personal"}));

        let mut m = meta(json!({"view": "trust"}));
        migrate_meta(&mut m);
        assert_eq!(Value::Object(m), json!({"view": "connectors", "connectors:section": "accounts"}));

        let mut m = meta(json!({"view": "armory", "armory:section": "mcp"}));
        migrate_meta(&mut m);
        assert_eq!(Value::Object(m), json!({"view": "connectors", "connectors:section": "mcp"}));
    }

    #[test]
    fn knowledge_and_remotes_move_and_others_are_untouched() {
        let mut m = meta(json!({"view": "knowledge", "knowledge:section": "bundles"}));
        assert!(migrate_meta(&mut m));
        assert_eq!(Value::Object(m), json!({"view": "memory", "memory:section": "bundles"}));

        let mut m = meta(json!({"view": "remotes", "remotes:expand": "box"}));
        migrate_meta(&mut m);
        assert_eq!(Value::Object(m), json!({"view": "connectors", "connectors:section": "remotes", "remotes:expand": "box"}));

        let mut m = meta(json!({"view": "term", "armory:section": "skills"}));
        assert!(!migrate_meta(&mut m));
    }

    #[test]
    fn a_snapshot_tree_is_walked() {
        let mut snap = json!({"windows": [{"tabs": [{"blocks": [{"meta": {"view": "knowledge"}}, {"meta": {"view": "agent"}}]}]}]});
        assert_eq!(migrate_tree(&mut snap), 1);
        assert_eq!(snap["windows"][0]["tabs"][0]["blocks"][0]["meta"]["view"], "memory");
    }

    #[test]
    fn settings_keys_move_and_untouched_settings_change_nothing() {
        let settings = meta(json!({
            "widget:pinned": ["agent", "armory", "memory", "remotes"],
            "widget:hidden": ["defwidget@knowledge", "ext@acme.board/main"],
            "pane:colors": {"knowledge": 120, "memory": 200, "trust": 30},
            "keybindings": [
                {"key": "ctrl+k", "command": "app:knowledge"},
                {"command": "-app:identity"},
                {"key": "ctrl+e", "command": "split:right", "when": "viewType == knowledge && !textInputFocus"}
            ],
            "term:fontsize": 14
        }));
        let changed = migrated_settings(&settings);
        assert_eq!(changed["widget:pinned"], json!(["agent", "connectors", "memory"]));
        assert_eq!(changed["widget:hidden"], json!(["defwidget@memory", "ext@acme.board/main"]));
        assert_eq!(changed["pane:colors"], json!({"memory": 200, "connectors": 30}));
        assert_eq!(changed["keybindings"][0]["command"], "app:memory");
        assert_eq!(changed["keybindings"][1]["command"], "-app:connectors");
        assert_eq!(changed["keybindings"][2]["when"], "viewType == memory && !textInputFocus");
        assert!(!changed.contains_key("term:fontsize"));

        let clean = meta(json!({"widget:pinned": ["agent"], "pane:colors": {"memory": 1}, "keybindings": []}));
        assert!(migrated_settings(&clean).is_empty());
    }

    #[test]
    fn widgets_json_entries_move_and_a_new_key_wins() {
        let widgets = meta(json!({
            "defwidget@remotes": {"label": "My remotes", "blockdef": {"meta": {"view": "remotes"}}},
            "defwidget@knowledge": {"label": "Old"},
            "defwidget@memory": {"label": "Mine"},
            "defwidget@x": {"blockdef": {"meta": {"view": "armory", "armory:section": "bundles"}}}
        }));
        let next = migrated_widgets(&widgets).unwrap();
        assert_eq!(next["defwidget@connectors"]["blockdef"]["meta"], json!({"view": "connectors", "connectors:section": "remotes"}));
        assert_eq!(next["defwidget@memory"]["label"], "Mine");
        assert_eq!(next["defwidget@x"]["blockdef"]["meta"], json!({"view": "memory", "memory:section": "bundles"}));
        assert!(!next.contains_key("defwidget@remotes") && !next.contains_key("defwidget@knowledge"));
        assert!(migrated_widgets(&meta(json!({"defwidget@agent": {"label": "A"}}))).is_none());
    }

    #[test]
    fn files_are_rewritten_only_when_they_change() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, r#"{ "widget:pinned": ["knowledge"], "term:fontsize": 14 }"#).unwrap();
        migrate_settings_file(&settings);
        let after = wconfig::read_settings_raw_jsonc(&settings);
        assert_eq!(after["widget:pinned"], json!(["memory"]));
        assert_eq!(after["term:fontsize"], json!(14));

        let widgets = dir.path().join("widgets.json");
        std::fs::write(&widgets, "{\"defwidget@agent\": {\"label\": \"A\"}}").unwrap();
        migrate_widgets_file(&widgets);
        assert_eq!(std::fs::read_to_string(&widgets).unwrap(), "{\"defwidget@agent\": {\"label\": \"A\"}}");
    }

    #[test]
    fn blocks_and_snapshots_in_the_store_are_moved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("objects.db")).unwrap();
        let mut block = crate::backend::obj::Block {
            oid: "b1".into(),
            meta: meta(json!({"view": "armory", "armory:section": "bundles"})).into_iter().collect(),
            ..Default::default()
        };
        store.insert(&mut block).unwrap();
        let mut other = crate::backend::obj::Block {
            oid: "b2".into(),
            meta: meta(json!({"view": "term"})).into_iter().collect(),
            ..Default::default()
        };
        store.insert(&mut other).unwrap();
        assert_eq!(old_view_block_count(&store).unwrap(), 1);
        assert_eq!(migrate_blocks(&store).unwrap(), 1);
        assert_eq!(migrate_blocks(&store).unwrap(), 0, "idempotent");
        let b1 = store.get_raw("block", "b1").unwrap().unwrap();
        assert_eq!(b1["meta"]["view"], "memory");
        assert_eq!(b1["meta"]["memory:section"], "bundles");
        assert_eq!(old_view_block_count(&store).unwrap(), 0);
    }
}
