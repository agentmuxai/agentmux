// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filesystem watcher for settings.json — detects saves and pushes updated
//! config to all connected WebSocket clients in real time.
//!
//! Migrated onto the shared `fs_watch::FsWatchPool`
//! (SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md) — this module keeps only
//! its own domain-specific debounce/reload/broadcast logic; the actual
//! `notify` construction, refcounting, and recovery-on-failure now live in
//! the pool, shared with every other watcher that migrates onto it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast;

use super::eventbus::{EventBus, WSEventType, WS_EVENT_RPC};
use super::fs_watch::{FsWatchEvent, FsWatchEventKind, FsWatchPool};
use super::wconfig::{self, ConfigWatcher, SettingsType};

/// Resolve the directory containing settings.json.
///
/// Priority:
/// 1. `AGENTMUX_SETTINGS_DIR` env var (set by Tauri host to app_config_dir)
/// 2. If [`agentmux_common::isolated_settings_enabled`] — the default for
///    every channel except `stable`, see
///    `docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` —
///    `AGENTMUX_CONFIG_HOME` used DIRECTLY, with no parent-walk. That var
///    already carries the correctly channel-scoped `channels/<ch>/config/`
///    directory (re-exported from `AGENTMUX_CONFIG_DIR`, see
///    `data_paths.rs`'s own `channels/<ch>/config/ ← settings
///    (channel-wide)` layout comment) — this is the fix, using that value
///    as-is instead of walking past it.
/// 3. Otherwise (global — `stable` channel, or an explicit
///    `AGENTMUX_ISOLATED_SETTINGS=0` opt-out): `AGENTMUX_CONFIG_HOME`,
///    walking up two parent directories — unchanged legacy behavior. That
///    var's value is a channel-scoped `.../config` directory; walking up
///    two levels lands one level ABOVE the channel root (on
///    `~/.agentmux/channels/` itself, not inside any specific channel),
///    which is what makes this the genuinely shared, cross-channel file.
/// 4. `~/.agentmux` (legacy fallback — `AGENTMUX_CONFIG_HOME` unset)
pub fn resolve_settings_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTMUX_SETTINGS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(dir) = std::env::var("AGENTMUX_CONFIG_HOME") {
        if !dir.is_empty() {
            let path = PathBuf::from(&dir);
            if agentmux_common::isolated_settings_enabled() {
                // Already channel-scoped — use directly, no parent-walk.
                return path;
            }
            // AGENTMUX_CONFIG_HOME = channels/<ch>/config — go up two
            // levels to the shared channels/ root.
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

    // Boot-time diagnostic distinguishing all four resolvable isolation
    // states — see docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md
    // Phase 1. Logged here (not inside resolve_settings_dir itself) since
    // this function is the one true "called once at startup" call site;
    // the other two call sites (spawn_settings_watcher, the save path)
    // would spam this on every watcher re-arm / every save.
    tracing::info!(
        reason = agentmux_common::isolated_settings_reason().as_str(),
        "settings.json isolation"
    );

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

/// Whether a raw fs_watch event refers to a `settings.json` save specifically
/// — the pool's broadcast stream carries events for every currently-watched
/// path in the process, not just this module's. Matches the pre-migration
/// code's filter exactly: filename must be `settings.json` (was
/// `event.paths.iter().any(|p| p.ends_with("settings.json"))`) AND the event
/// must be a `Create`/`Modify` (was `EventKind::Modify(_) | EventKind::Create(_)`).
/// A `Removed` event (external delete, or an editor's unlink+recreate save) is
/// deliberately excluded — `read_config_file` treats a missing file as success
/// (defaults, no errors), so reacting to `Remove` would reset the live config
/// and broadcast it to every client.
fn is_settings_file_event(event: &FsWatchEvent) -> bool {
    let is_settings_file = event
        .path
        .file_name()
        .map(|n| n == wconfig::SETTINGS_FILE)
        .unwrap_or(false);
    is_settings_file
        && matches!(event.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified)
}

/// Subscribe to `settings.json` via the shared `FsWatchPool` and broadcast
/// config updates to all WebSocket clients on change.
///
/// Fire-and-forget — the returned subscription never needs to be
/// unsubscribed (settings.json is watched for the app's entire lifetime),
/// so unlike the pre-migration version there's no watcher handle for the
/// caller to keep alive: the pool itself owns that now.
pub fn spawn_settings_watcher(
    pool: Arc<FsWatchPool>,
    config_watcher: Arc<ConfigWatcher>,
    event_bus: Arc<EventBus>,
) {
    let settings_dir = resolve_settings_dir();
    let settings_path = settings_dir.join(wconfig::SETTINGS_FILE);

    if !settings_dir.exists() {
        tracing::warn!(
            dir = %settings_dir.display(),
            "settings directory does not exist, file watcher not started"
        );
        return;
    }

    // events() MUST be called before subscribe_file() — a broadcast::Receiver
    // only sees messages sent after it subscribes, so a change event delivered
    // in the gap between starting the watch and creating this receiver would
    // otherwise be silently missed (reagent P2 on PR #2456; see
    // fs_watch::pool::tests::a_change_event_is_observable_on_the_broadcast_stream
    // for the same ordering requirement enforced on the pool's own test).
    let mut events = pool.events();

    // Deliberately dropped immediately — `Subscription` has no `Drop`
    // side effect (unsubscribe is always an explicit call), so this settles
    // into a permanent watch for the process's lifetime with nothing to
    // hold onto, matching the pre-migration version's own "this is a
    // once-at-startup, forever" contract.
    let _ = pool.subscribe_file(&settings_path);

    tracing::info!(
        path = %settings_path.display(),
        dir = %settings_dir.display(),
        "fs_watch subscription active for settings.json"
    );

    let watched_path = settings_path.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) if is_settings_file_event(&ev) => {}
                Ok(_) => continue, // some other watched path — not ours
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("settings watcher event stream closed, stopping");
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // We fell behind the pool's broadcast buffer. A lagged
                    // receiver has definitely missed *some* event, so treat
                    // it the same as "something changed" rather than
                    // silently resubscribing to a fresh position and
                    // potentially missing a real settings.json save.
                    tracing::warn!(skipped, "settings watcher lagged; reloading defensively");
                }
            }
            // Debounce: drain whatever else is already queued within 300ms,
            // same collapse-a-burst intent as before — this receiver is
            // this task's own private clone of the broadcast stream, so
            // draining it can't affect any other subscriber.
            tokio::time::sleep(Duration::from_millis(300)).await;
            while events.try_recv().is_ok() {}

            reload_and_broadcast(&watched_path, &config_watcher, &event_bus);
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end regression for the migration onto `FsWatchPool`: a real
    /// on-disk settings.json change is detected via the shared pool,
    /// debounced, reloaded, and lands in `ConfigWatcher`. This is new
    /// coverage — `spawn_settings_watcher` had no test before this module
    /// existed to migrate onto.
    #[tokio::test]
    async fn settings_change_via_pool_reloads_config_watcher() {
        // codex P2 on PR #2664: this test mutates the process-global
        // AGENTMUX_SETTINGS_DIR — must hold the crate-wide env lock, the
        // same one resolve_settings_dir's isolation tests below use, or
        // the two can interleave and redirect each other's settings dir
        // mid-test.
        let _lock = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("AGENTMUX_SETTINGS_DIR");
        std::env::set_var("AGENTMUX_SETTINGS_DIR", tmp.path());

        let settings_path = tmp.path().join(wconfig::SETTINGS_FILE);
        std::fs::write(&settings_path, wconfig::SETTINGS_TEMPLATE).unwrap();

        let pool = FsWatchPool::new();
        let config_watcher = Arc::new(ConfigWatcher::new());
        let event_bus = Arc::new(EventBus::new());
        spawn_settings_watcher(pool.clone(), config_watcher.clone(), event_bus.clone());

        // Give the watch a moment to actually register before writing —
        // otherwise this is a "wrote before the watch was live" race, not a
        // real assertion about the reload path.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut new_keys = serde_json::Map::new();
        new_keys.insert("app:defaultnewblock".to_string(), json!("fs-watch-test-marker"));
        let merged = wconfig::merge_into_template(wconfig::SETTINGS_TEMPLATE, &new_keys);
        std::fs::write(&settings_path, &merged).unwrap();

        let saw_it = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if config_watcher.get_settings().app_default_new_block == "fs-watch-test-marker" {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or(false);

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_SETTINGS_DIR", v),
            None => std::env::remove_var("AGENTMUX_SETTINGS_DIR"),
        }

        assert!(saw_it, "expected config_watcher to reload the updated setting after a settings.json change detected via FsWatchPool");
    }

    fn evt(path: &str, kind: FsWatchEventKind) -> FsWatchEvent {
        FsWatchEvent { path: PathBuf::from(path), kind }
    }

    #[test]
    fn is_settings_file_event_matches_only_the_settings_filename() {
        assert!(is_settings_file_event(&evt("/some/dir/settings.json", FsWatchEventKind::Modified)));
        assert!(is_settings_file_event(&evt("/some/dir/settings.json", FsWatchEventKind::Created)));
        assert!(!is_settings_file_event(&evt("/some/dir/other.json", FsWatchEventKind::Modified)));
        assert!(!is_settings_file_event(&evt("/some/dir", FsWatchEventKind::Modified)));
    }

    #[test]
    fn is_settings_file_event_ignores_removed_events() {
        // A Remove event for settings.json (external delete, or an editor's
        // unlink+recreate save) must NOT trigger a reload — read_config_file
        // treats a missing file as success-with-defaults, so reacting here
        // would reset the live config and broadcast defaults to every client.
        assert!(!is_settings_file_event(&evt("/some/dir/settings.json", FsWatchEventKind::Removed)));
        assert!(!is_settings_file_event(&evt("/some/dir/settings.json", FsWatchEventKind::Other)));
    }

    // ── resolve_settings_dir isolation (SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md) ──

    /// codex P2 on PR #2664: a module-local `Mutex<()>` here would only
    /// serialize these tests against EACH OTHER — not against
    /// `settings_change_via_pool_reloads_config_watcher` above (same file,
    /// mutates `AGENTMUX_SETTINGS_DIR`) or `registry::paths`' tests
    /// (mutate `AGENTMUX_CHANNEL` behind their own, different lock).
    /// Cargo's default parallel test runner means those could still
    /// interleave with these and redirect each other's resolved dir /
    /// isolation state mid-test. Use the crate-wide lock instead — see
    /// `test_support.rs`'s own doc comment, which documents exactly this
    /// failure mode (reagent/codex on PR #2318) for the auth-isolation
    /// tests this lock was introduced for; it's the crate's general
    /// process-env-mutation lock, not auth-specific despite the name.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as SETTINGS_DIR_ENV_LOCK;

    fn lock_settings_dir_env() -> std::sync::MutexGuard<'static, ()> {
        SETTINGS_DIR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Clears every env var `resolve_settings_dir` / `isolated_settings_*`
    /// read, so each test starts from a known state regardless of
    /// leakage from a prior test or from process env inherited from the
    /// test runner itself.
    fn clear_settings_dir_env() {
        std::env::remove_var("AGENTMUX_SETTINGS_DIR");
        std::env::remove_var("AGENTMUX_CONFIG_HOME");
        std::env::remove_var("AGENTMUX_CHANNEL");
        std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    }

    #[test]
    fn resolve_settings_dir_stays_global_on_stable_channel() {
        let _lock = lock_settings_dir_env();
        clear_settings_dir_env();
        std::env::set_var("AGENTMUX_CONFIG_HOME", "/home/user/.agentmux/channels/stable/config");
        std::env::set_var("AGENTMUX_CHANNEL", "stable");

        let dir = resolve_settings_dir();

        assert_eq!(dir, PathBuf::from("/home/user/.agentmux/channels"));
        clear_settings_dir_env();
    }

    #[test]
    fn resolve_settings_dir_is_isolated_by_default_on_non_stable_channel() {
        // The behavior change this spec introduces: no AGENTMUX_ISOLATED_SETTINGS
        // set at all — a task-dev branch or task-package build's channel is
        // enough on its own to get an isolated settings dir.
        let _lock = lock_settings_dir_env();
        clear_settings_dir_env();
        let config_home = "/home/user/.agentmux/channels/local-main-abc123-1/config";
        std::env::set_var("AGENTMUX_CONFIG_HOME", config_home);
        std::env::set_var("AGENTMUX_CHANNEL", "local-main-abc123-1");

        let dir = resolve_settings_dir();

        assert_eq!(
            dir,
            PathBuf::from(config_home),
            "isolated-by-default settings dir must be the channel-scoped config dir itself, not its shared parent"
        );
        clear_settings_dir_env();
    }

    #[test]
    fn resolve_settings_dir_explicit_opt_out_restores_global_on_non_stable_channel() {
        let _lock = lock_settings_dir_env();
        clear_settings_dir_env();
        std::env::set_var("AGENTMUX_CONFIG_HOME", "/home/user/.agentmux/channels/dev-some-branch/config");
        std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
        std::env::set_var("AGENTMUX_ISOLATED_SETTINGS", "0");

        let dir = resolve_settings_dir();

        assert_eq!(dir, PathBuf::from("/home/user/.agentmux/channels"));
        clear_settings_dir_env();
    }

    #[test]
    fn resolve_settings_dir_explicit_opt_in_isolates_even_the_stable_channel() {
        let _lock = lock_settings_dir_env();
        clear_settings_dir_env();
        std::env::set_var("AGENTMUX_CONFIG_HOME", "/home/user/.agentmux/channels/stable/config");
        std::env::set_var("AGENTMUX_CHANNEL", "stable");
        std::env::set_var("AGENTMUX_ISOLATED_SETTINGS", "1");

        let dir = resolve_settings_dir();

        assert_eq!(dir, PathBuf::from("/home/user/.agentmux/channels/stable/config"));
        clear_settings_dir_env();
    }

    #[test]
    fn resolve_settings_dir_stays_global_when_channel_unset() {
        // Conservative fallback: no AGENTMUX_CHANNEL in the process env at
        // all — stay global rather than guess, same as isolated_auth's
        // equivalent case.
        let _lock = lock_settings_dir_env();
        clear_settings_dir_env();
        std::env::set_var("AGENTMUX_CONFIG_HOME", "/home/user/.agentmux/channels/dev-some-branch/config");
        // AGENTMUX_CHANNEL deliberately left unset.

        let dir = resolve_settings_dir();

        assert_eq!(dir, PathBuf::from("/home/user/.agentmux/channels"));
        clear_settings_dir_env();
    }

    #[test]
    fn resolve_settings_dir_settings_dir_override_wins_regardless_of_isolation() {
        let _lock = lock_settings_dir_env();
        clear_settings_dir_env();
        std::env::set_var("AGENTMUX_SETTINGS_DIR", "/explicit/override");
        std::env::set_var("AGENTMUX_CONFIG_HOME", "/home/user/.agentmux/channels/local-main-abc123-1/config");
        std::env::set_var("AGENTMUX_CHANNEL", "local-main-abc123-1");

        let dir = resolve_settings_dir();

        assert_eq!(dir, PathBuf::from("/explicit/override"));
        clear_settings_dir_env();
    }
}
