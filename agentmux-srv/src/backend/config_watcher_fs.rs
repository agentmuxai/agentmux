//! Filesystem watcher for settings.json — detects saves and pushes updated
//! config to all connected WebSocket clients in real time.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;

use super::eventbus::{EventBus, WSEventType, WS_EVENT_RPC};
use super::wconfig::{self, ConfigWatcher, SettingsType};

/// Resolve the directory containing settings.json.
///
/// Priority:
/// 1. `AGENTMUX_SETTINGS_DIR` env var (set by Tauri host to app_config_dir)
/// 2. `AGENTMUX_CONFIG_HOME` env var (backend's config root)
/// 3. `~/.agentmux` (legacy fallback)
pub fn resolve_settings_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTMUX_SETTINGS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    // Fall back to config home parent (settings.json sits at app_config_dir root,
    // not inside the instances subdir)
    if let Ok(dir) = std::env::var("AGENTMUX_CONFIG_HOME") {
        if !dir.is_empty() {
            // AGENTMUX_CONFIG_HOME = .../instances/v0.31.XX — go up two levels
            let path = PathBuf::from(&dir);
            if let Some(root) = path.parent().and_then(|p| p.parent()) {
                return root.to_path_buf();
            }
        }
    }
    dirs::home_dir().unwrap_or_default().join(".agentmux")
}

/// Load settings.json from disk into the ConfigWatcher.
/// Called once at startup so the backend has the user's saved settings.
pub fn load_settings_from_disk(config_watcher: &ConfigWatcher) {
    let settings_dir = resolve_settings_dir();
    let settings_path = settings_dir.join(wconfig::SETTINGS_FILE);

    tracing::info!(
        path = %settings_path.display(),
        exists = settings_path.exists(),
        "loading settings.json from disk"
    );

    let (settings, errors): (SettingsType, _) = wconfig::read_config_file(&settings_path);

    if !errors.is_empty() {
        for err in &errors {
            tracing::warn!(file = %err.file, error = %err.err, "settings parse error at startup");
        }
        return;
    }

    config_watcher.update_settings(settings);
    tracing::info!("settings.json loaded successfully");
}

/// Spawn a filesystem watcher that monitors `settings.json` and broadcasts
/// config updates to all WebSocket clients on change.
///
/// Returns a handle to the watcher (must be held alive for the duration of the app).
pub fn spawn_settings_watcher(
    config_watcher: Arc<ConfigWatcher>,
    event_bus: Arc<EventBus>,
) -> Option<RecommendedWatcher> {
    let settings_dir = resolve_settings_dir();
    let settings_path = settings_dir.join(wconfig::SETTINGS_FILE);

    if !settings_dir.exists() {
        tracing::warn!(
            dir = %settings_dir.display(),
            "settings directory does not exist, file watcher not started"
        );
        return None;
    }

    // Channel to bridge sync notify callbacks into async tokio
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();

    let watched_path = settings_path.clone();
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(event) => {
                let dominated = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_)
                );
                if dominated && event.paths.iter().any(|p| p.ends_with("settings.json")) {
                    let _ = tx.send(());
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "filesystem watcher error");
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create settings file watcher");
            return None;
        }
    };

    if let Err(e) = watcher.watch(&settings_dir, RecursiveMode::NonRecursive) {
        tracing::warn!(
            dir = %settings_dir.display(),
            error = %e,
            "failed to watch settings directory"
        );
        return None;
    }

    tracing::info!(
        path = %settings_path.display(),
        dir = %settings_dir.display(),
        "filesystem watcher active for settings.json"
    );

    // Spawn async task: debounce notifications and reload config
    tokio::spawn(async move {
        loop {
            // Wait for first notification
            if rx.recv().await.is_none() {
                tracing::info!("settings watcher channel closed, stopping");
                break;
            }
            // Debounce: drain any additional events within 300ms
            tokio::time::sleep(Duration::from_millis(300)).await;
            while rx.try_recv().is_ok() {}

            reload_and_broadcast(&watched_path, &config_watcher, &event_bus);
        }
    });

    Some(watcher)
}

/// Merge new keys into the current in-memory SettingsType and return the result.
/// Used by the setconfig handler to update in-memory state before the fs watcher fires.
pub fn merge_settings_into_current(
    config_watcher: &wconfig::ConfigWatcher,
    new_keys: serde_json::Map<String, serde_json::Value>,
) -> wconfig::SettingsType {
    let mut current = config_watcher.get_settings();
    // Merge via JSON round-trip so the extra HashMap catches all dynamic keys
    if let Ok(mut current_val) = serde_json::to_value(&current) {
        if let serde_json::Value::Object(ref mut map) = current_val {
            map.extend(new_keys.into_iter().filter(|(_, v)| !v.is_null()));
        }
        if let Ok(merged) = serde_json::from_value(current_val) {
            current = merged;
        }
    }
    current
}

/// Merge a flat map of settings keys into `settings.json` on disk.
/// Existing keys not present in `new_keys` are preserved.
/// The fs watcher will detect the write (~300ms) and broadcast the updated config.
pub fn merge_settings_to_disk(new_keys: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    if new_keys.is_empty() {
        return Ok(());
    }
    let settings_dir = resolve_settings_dir();
    let settings_path = settings_dir.join(wconfig::SETTINGS_FILE);

    let mut current = wconfig::read_settings_raw_jsonc(&settings_path);
    current.extend(new_keys);

    // Remove keys explicitly set to null (deletion semantics)
    current.retain(|_, v| !v.is_null());

    let merged = wconfig::merge_into_template(wconfig::SETTINGS_TEMPLATE, &current);
    std::fs::write(&settings_path, &merged)
        .map_err(|e| format!("write settings.json: {e}"))?;

    tracing::info!(path = %settings_path.display(), "settings.json updated via setconfig");
    Ok(())
}

fn reload_and_broadcast(
    settings_path: &PathBuf,
    config_watcher: &Arc<ConfigWatcher>,
    event_bus: &Arc<EventBus>,
) {
    tracing::info!(path = %settings_path.display(), "settings.json changed, reloading");

    let (settings, errors): (SettingsType, _) = wconfig::read_config_file(settings_path);

    if !errors.is_empty() {
        for err in &errors {
            tracing::warn!(file = %err.file, error = %err.err, "settings reload parse error (keeping previous config)");
        }
        return;
    }

    config_watcher.update_settings(settings);
    tracing::info!("settings.json reloaded, broadcasting to clients");

    // Broadcast updated config to all connected clients (same format as initial config push)
    let config = config_watcher.get_full_config();
    let client_count = event_bus.connection_count();
    if let Ok(config_val) = serde_json::to_value(config.as_ref()) {
        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(json!({
                "command": "eventrecv",
                "data": {
                    "event": "config",
                    "data": { "fullconfig": config_val }
                }
            })),
        };
        event_bus.broadcast_event(&event);
        tracing::info!(clients = client_count, "config event broadcast complete");
    }
}
