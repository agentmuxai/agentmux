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

fn apply(config_watcher: &ConfigState, user: HashMap<String, WidgetConfigType>) {
    wconfig::validate_widget_configs(&user);
    config_watcher.update_widgets(merge_widgets(builtin_widgets(), user));
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

pub fn spawn_user_widgets_watcher(pool: Arc<FsWatchPool>, config_watcher: Arc<ConfigState>, event_bus: Arc<EventBus>) {
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
                    apply(&config_watcher, user);
                    broadcast_full_config(&config_watcher, &event_bus);
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
