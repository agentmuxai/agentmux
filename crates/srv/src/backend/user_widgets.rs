// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The user's own `widgets.json`, next to `settings.json` — Pane Tab contract
//! v1, Phase 6 (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §4).
//!
//! The widget bar used to come only from the `widgets.json` embedded at build
//! time, so there was nowhere to add a third-party (`ext:`) widget. This file's
//! entries are merged over the built-ins: a new key adds a widget, a built-in
//! key replaces that widget. Same load-then-watch shape as `settings.json`
//! (`config_watcher_fs`) and `browser_start_page`, on the same fs_watch pool,
//! riding the existing full-config broadcast.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::broadcast;

use super::config_watcher_fs::{broadcast_full_config, resolve_settings_dir};
use super::eventbus::EventBus;
use super::fs_watch::{FsWatchEvent, FsWatchEventKind, FsWatchPool};
use super::mps::Broker;
use super::wconfig::{self, ConfigState, WidgetConfigType};

pub const USER_WIDGETS_FILE: &str = "widgets.json";

pub fn user_widgets_path() -> PathBuf {
    resolve_settings_dir().join(USER_WIDGETS_FILE)
}

/// The user's widgets; an absent or empty file is none.
pub fn read_user_widgets(path: &Path) -> Result<HashMap<String, WidgetConfigType>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| format!("read widgets.json: {e}"))?;
    if content.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(&content).map_err(|e| format!("parse widgets.json: {e}"))
}

/// The built-in widgets, parsed once.
fn builtin_widgets() -> &'static HashMap<String, WidgetConfigType> {
    static BUILTIN: OnceLock<HashMap<String, WidgetConfigType>> = OnceLock::new();
    BUILTIN.get_or_init(|| wconfig::build_default_config().widgets)
}

/// The built-ins with the user's entries merged over them.
pub fn merge_widgets(
    builtin: &HashMap<String, WidgetConfigType>,
    user: HashMap<String, WidgetConfigType>,
) -> HashMap<String, WidgetConfigType> {
    let mut merged = builtin.clone();
    merged.extend(user);
    merged
}

/// The user's widgets.json as last read, for the widget packages scan (its v1
/// `module` entries are implied packages) and for recomputing the merge when
/// a package changes.
static LAST_USER: std::sync::Mutex<Option<HashMap<String, WidgetConfigType>>> = std::sync::Mutex::new(None);

/// The v1 entries (`module` set) of the user's widgets.json as last read.
pub fn last_v1_entries() -> HashMap<String, WidgetConfigType> {
    let guard = LAST_USER.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .map(|u| u.iter().filter(|(_, e)| !e.module.is_empty()).map(|(k, e)| (k.clone(), e.clone())).collect())
        .unwrap_or_default()
}

fn apply(config_watcher: &ConfigState, user: HashMap<String, WidgetConfigType>) {
    wconfig::validate_widget_configs(&user);
    *LAST_USER.lock().unwrap_or_else(|p| p.into_inner()) = Some(user);
    if let Some(svc) = super::widget_packages::service() {
        svc.rescan(&last_v1_entries());
    }
    recompute(config_watcher);
}

/// Merge again: the built-ins, then each approved widget package's entries
/// (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §5.4), then the user's own
/// widgets.json, which replaces a package's entry when it uses the same key.
pub fn recompute(config_watcher: &ConfigState) {
    let user = LAST_USER.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_default();
    let mut base = builtin_widgets().clone();
    if let Some(svc) = super::widget_packages::service() {
        base.extend(super::widget_packages::widget_entries(&svc.list()));
    }
    config_watcher.update_widgets(merge_widgets(&base, user));
}

/// Whether a widgets.json entry opens the same pane section as `entry` (its view and `section`).
fn same_section(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    let view = |e: &serde_json::Value| e.pointer("/blockdef/meta/view").cloned();
    a.get("section").is_some() && a.get("section") == b.get("section") && view(a) == view(b)
}

/// Where a section's `entry` is, or goes, in widgets.json: under `base` if that key is free or
/// already opens this section, otherwise under the first free `base-2`, `base-3`, …, so a key the
/// user reused for something else is never handed back as this section's. Returns the key and,
/// when the entry has to be added, the file's new text. An absent or empty file is `{}`; one that
/// doesn't parse is an error, so a file the user is halfway through editing is never rewritten.
pub fn section_entry_in(content: &str, base: &str, entry: serde_json::Value) -> Result<(String, Option<String>), String> {
    let mut widgets: serde_json::Map<String, serde_json::Value> = if content.trim().is_empty() {
        serde_json::Map::new()
    } else {
        serde_json::from_str(content).map_err(|e| format!("widgets.json doesn't parse, so it was left alone: {e}"))?
    };
    for n in 1..=50 {
        let key = if n == 1 { base.to_string() } else { format!("{base}-{n}") };
        match widgets.get(&key) {
            Some(existing) if same_section(existing, &entry) => return Ok((key, None)),
            Some(_) => continue,
            None => {
                widgets.insert(key.clone(), entry);
                let text = serde_json::to_string_pretty(&widgets).map_err(|e| e.to_string())? + "\n";
                return Ok((key, Some(text)));
            }
        }
    }
    Err(format!("widgets.json has no free key for {base}"))
}

/// Adds a section's `entry` to the user's widgets.json (`section_entry_in`) and applies the file
/// at once, so the widget bar can pin the entry as soon as this returns; the file watcher applies
/// it again a moment later, which changes nothing. Returns the entry's key. For "Pin this section
/// to the widget bar" (section-pins.ts): the bar pins only an entry that exists.
pub fn add_section_entry(
    config_watcher: &Arc<ConfigState>,
    event_bus: &Arc<EventBus>,
    base: &str,
    entry: serde_json::Value,
) -> Result<String, String> {
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    let path = user_widgets_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("read widgets.json: {e}")),
    };
    let (key, text) = section_entry_in(&content, base, entry)?;
    if let Some(text) = text {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create the settings folder: {e}"))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("write widgets.json: {e}"))?;
    }
    apply(config_watcher, read_user_widgets(&path)?);
    broadcast_full_config(config_watcher, event_bus);
    Ok(key)
}

pub fn load_user_widgets_from_disk(config_watcher: &ConfigState) {
    let path = user_widgets_path();
    match read_user_widgets(&path) {
        Ok(user) => {
            tracing::info!(path = %path.display(), count = user.len(), "user widgets.json loaded");
            apply(config_watcher, user);
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "user widgets.json failed to load (built-ins only)");
        }
    }
}

fn is_user_widgets_event(event: &FsWatchEvent) -> bool {
    let is_file = event.path.file_name().map(|n| n == USER_WIDGETS_FILE).unwrap_or(false);
    // Removed is excluded for the same reason config_watcher_fs gives: an
    // editor's unlink+recreate save would otherwise flash the built-ins only.
    is_file && matches!(event.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified)
}

/// widgets.json changed: merge again, and tell every UI both the new config
/// and the package list, since its v1 `module` entries are packages too.
fn on_changed(config_watcher: &Arc<ConfigState>, event_bus: &Arc<EventBus>, broker: &Broker, user: HashMap<String, WidgetConfigType>) {
    apply(config_watcher, user);
    broadcast_full_config(config_watcher, event_bus);
    let packages = super::widget_packages::service().map(|s| s.list()).unwrap_or_default();
    super::widget_packages::publish(broker, &packages);
}

pub fn spawn_user_widgets_watcher(
    pool: Arc<FsWatchPool>,
    config_watcher: Arc<ConfigState>,
    event_bus: Arc<EventBus>,
    broker: Arc<Broker>,
) {
    let path = user_widgets_path();
    let Some(dir) = path.parent() else {
        return;
    };
    if !dir.exists() {
        tracing::warn!(dir = %dir.display(), "settings directory does not exist, widgets.json watcher not started");
        return;
    }

    // events() before subscribe_file(): a broadcast receiver only sees what is
    // sent after it subscribes (see spawn_settings_watcher).
    let mut events = pool.events();
    let _ = pool.subscribe_file(&path);
    tracing::info!(path = %path.display(), "fs_watch subscription active for user widgets.json");

    let watched_path = path.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) if is_user_widgets_event(&ev) => {}
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("user widgets watcher event stream closed, stopping");
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "user widgets watcher lagged; reloading defensively");
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            while events.try_recv().is_ok() {}

            match read_user_widgets(&watched_path) {
                Ok(user) => {
                    tracing::info!(path = %watched_path.display(), "user widgets.json changed, reloading");
                    // A rescan hashes every package's files: off the async workers.
                    let (c, e, b) = (config_watcher.clone(), event_bus.clone(), broker.clone());
                    let _ = tokio::task::spawn_blocking(move || on_changed(&c, &e, &b, user)).await;
                }
                Err(e) => {
                    tracing::warn!(path = %watched_path.display(), error = %e, "user widgets.json reload parse error (keeping previous widgets)");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(label: &str) -> WidgetConfigType {
        WidgetConfigType { label: label.to_string(), ..Default::default() }
    }

    #[test]
    fn a_change_to_widgets_json_tells_the_ui_the_package_list_too() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let broker = Broker::new();
        let log = seen.clone();
        broker.add_observer(Arc::new(move |ev| log.lock().unwrap().push(ev.event.clone())));
        let config = Arc::new(ConfigState::new());
        on_changed(&config, &Arc::new(EventBus::new()), &broker, HashMap::from([("mine".to_string(), widget("Mine"))]));
        assert!(seen.lock().unwrap().iter().any(|e| e == super::super::mps::EVENT_WIDGET_PACKAGES));
    }

    #[test]
    fn a_section_entry_is_added_once_and_never_to_a_file_that_doesnt_parse() {
        let base = "section@warden-lan";
        let entry = serde_json::json!({ "label": "LAN", "section": "lan", "blockdef": { "meta": { "view": "warden" } } });
        let (key, text) = section_entry_in("", base, entry.clone()).unwrap();
        let parsed: HashMap<String, WidgetConfigType> = serde_json::from_str(&text.unwrap()).unwrap();
        assert_eq!((key.as_str(), parsed[base].section.as_str()), (base, "lan"));
        // Kept beside what the user already has, and not added twice.
        let (_, both) = section_entry_in(r#"{"mine@x":{"label":"Mine"}}"#, base, entry.clone()).unwrap();
        let both = both.unwrap();
        assert!(both.contains("mine@x") && both.contains(base));
        assert_eq!(section_entry_in(&both, base, entry.clone()).unwrap(), (base.to_string(), None));
        assert!(section_entry_in("{ half typed", base, entry).is_err());
    }

    #[test]
    fn a_section_entry_key_the_user_reused_for_something_else_is_never_handed_back() {
        let base = "section@warden-lan";
        let entry = serde_json::json!({ "label": "LAN", "section": "lan", "blockdef": { "meta": { "view": "warden" } } });
        let taken = r#"{"section@warden-lan":{"label":"Mine","blockdef":{"meta":{"view":"term"}}}}"#;
        let (key, text) = section_entry_in(taken, base, entry.clone()).unwrap();
        assert_eq!(key, "section@warden-lan-2");
        let text = text.unwrap();
        assert!(text.contains("\"Mine\""));
        // Pinning it again finds the one just added.
        assert_eq!(section_entry_in(&text, base, entry).unwrap(), ("section@warden-lan-2".to_string(), None));
    }

    #[test]
    fn a_missing_or_empty_file_is_no_widgets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USER_WIDGETS_FILE);
        assert!(read_user_widgets(&path).unwrap().is_empty());
        std::fs::write(&path, "  \n").unwrap();
        assert!(read_user_widgets(&path).unwrap().is_empty());
    }

    #[test]
    fn reads_an_ext_widget_with_its_module() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USER_WIDGETS_FILE);
        std::fs::write(
            &path,
            r#"{"ext@hello":{"label":"Hello","module":"hello/index.js","blockdef":{"meta":{"view":"ext:hello"}}}}"#,
        )
        .unwrap();
        let user = read_user_widgets(&path).unwrap();
        assert_eq!(user["ext@hello"].module, "hello/index.js");
    }

    #[test]
    fn a_malformed_file_is_an_error_not_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USER_WIDGETS_FILE);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_user_widgets(&path).is_err());
    }

    #[test]
    fn user_entries_add_widgets_and_replace_builtins_by_key() {
        let builtin = HashMap::from([("defwidget@a".to_string(), widget("A")), ("defwidget@b".to_string(), widget("B"))]);
        let user = HashMap::from([("defwidget@b".to_string(), widget("B2")), ("ext@c".to_string(), widget("C"))]);
        let merged = merge_widgets(&builtin, user);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged["defwidget@a"].label, "A");
        assert_eq!(merged["defwidget@b"].label, "B2");
        assert_eq!(merged["ext@c"].label, "C");
    }

    #[test]
    fn the_merge_always_starts_from_the_builtins() {
        // A reload that drops a user widget must not keep it around.
        let builtin = HashMap::from([("defwidget@a".to_string(), widget("A"))]);
        let first = merge_widgets(&builtin, HashMap::from([("ext@c".to_string(), widget("C"))]));
        assert!(first.contains_key("ext@c"));
        let second = merge_widgets(&builtin, HashMap::new());
        assert!(!second.contains_key("ext@c"));
    }
}
