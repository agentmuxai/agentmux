// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser pane start page — a single URL, deliberately stored under
//! `shared_dir` (`~/.agentmux/shared/browser-start-page.json`) for the same
//! reason bookmarks are (`bookmarks_store.rs`): `settings.json` isolation is
//! whole-file only (`docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md`
//! §6), so a value that must stay global across channels can't live inside
//! it.
//!
//! Unlike bookmarks, this value also rides inside `FullConfigType` — see
//! `docs/specs/SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md` §3.1 for why a
//! second on-demand RPC (bookmarks' `.list` pattern) was rejected in favor
//! of piggybacking on `GetFullConfig`, which the frontend already awaits
//! before its very first render. That means this module does two things
//! `bookmarks_store.rs` doesn't: it plugs into `ConfigState`
//! ([`load_start_page_from_disk`]) and it watches its own file for changes
//! ([`spawn_start_page_watcher`]), mirroring `config_watcher_fs.rs`'s
//! settings watcher exactly so an edit from any window — or an external
//! process — reaches every connected client via the same live broadcast
//! `settings.json` changes already use, not just the window that made it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use super::config_watcher_fs::broadcast_full_config;
use super::eventbus::EventBus;
use super::fs_watch::{FsWatchEvent, FsWatchEventKind, FsWatchPool};
use super::wconfig::ConfigState;

const START_PAGE_FILE_NAME: &str = "browser-start-page.json";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct StartPageFile {
    #[serde(default)]
    url: Option<String>,
}

/// Resolves the shared, cross-channel start-page file path. Returns `None`
/// when `DataPaths` can't resolve (CI / unusual env) — same best-effort
/// posture as `bookmarks_store::bookmarks_file_path()`.
pub fn start_page_file_path() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|p| p.shared_dir.join(START_PAGE_FILE_NAME))
}

/// Read the configured start page. A missing file, or a present file with
/// no `url` set yet, is `Ok(None)` — "nothing configured", not an error,
/// same as bookmarks' missing-file case. A present-but-unparseable file IS
/// an error: silently treating it as unset would look exactly like the
/// user's setting vanished (same reasoning `bookmarks_store::read_bookmarks`
/// already documents).
pub fn read_start_page(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|e| format!("read start page: {e}"))?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let parsed: StartPageFile =
        serde_json::from_str(&content).map_err(|e| format!("parse start page: {e}"))?;
    Ok(parsed.url.filter(|u| !u.is_empty()))
}

/// Write the start page, replacing whatever was there before. Same
/// temp-file-then-rename crash safety as `bookmarks_store::write_bookmarks`.
/// Two windows writing at the same instant still race at the whole-file
/// level — last write wins — the same accepted limitation every other
/// single-JSON-file store in this app has.
pub fn write_start_page(path: &Path, url: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let file = StartPageFile { url: Some(url.to_string()) };
    let json = serde_json::to_string_pretty(&file).map_err(|e| format!("serialize start page: {e}"))?;
    let tmp = path.with_file_name(format!(".{START_PAGE_FILE_NAME}.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, &json).map_err(|e| format!("write start page: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename start page: {e}")
    })
}

/// Load `browser-start-page.json` from disk into the `ConfigState`. Called
/// once at startup, mirroring `config_watcher_fs::load_settings_from_disk`.
/// An unresolvable `shared_dir` or a missing file both leave the config's
/// `browser_start_page` at its default (`None`) rather than failing boot —
/// a start page is a convenience, never a startup precondition.
pub fn load_start_page_from_disk(config_watcher: &ConfigState) {
    let Some(path) = start_page_file_path() else {
        tracing::info!("browser-start-page.json: shared dir unresolvable, starting with no start page");
        return;
    };
    match read_start_page(&path) {
        Ok(url) => {
            tracing::info!(path = %path.display(), set = url.is_some(), "browser-start-page.json loaded");
            config_watcher.update_browser_start_page(url);
        }
        Err(e) => {
            // Same "keep previous config, don't fail boot" posture
            // `config_watcher_fs::reload_and_broadcast` already has for a
            // corrupt settings.json — the in-memory default (None) stands.
            tracing::warn!(path = %path.display(), error = %e, "browser-start-page.json failed to load (keeping unset)");
        }
    }
}

/// Whether a raw fs_watch event refers to `browser-start-page.json`
/// specifically — mirrors `config_watcher_fs::is_settings_file_event`
/// exactly, including excluding `Removed` (an external delete must not
/// reset a configured start page back to "unset" and broadcast that).
fn is_start_page_file_event(event: &FsWatchEvent) -> bool {
    let is_start_page_file = event
        .path
        .file_name()
        .map(|n| n == START_PAGE_FILE_NAME)
        .unwrap_or(false);
    is_start_page_file && matches!(event.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified)
}

/// Subscribe to `browser-start-page.json` via the shared `FsWatchPool` (the
/// SAME pool instance the settings watcher uses — no new watcher subsystem)
/// and broadcast the updated config to every connected client on change.
/// Mirrors `config_watcher_fs::spawn_settings_watcher` exactly.
///
/// Fire-and-forget: watched for the process's entire lifetime, same as
/// settings.json — nothing for the caller to hold onto or unsubscribe.
pub fn spawn_start_page_watcher(
    pool: Arc<FsWatchPool>,
    config_watcher: Arc<ConfigState>,
    event_bus: Arc<EventBus>,
) {
    let Some(path) = start_page_file_path() else {
        tracing::warn!("browser-start-page.json: shared dir unresolvable, file watcher not started");
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!(dir = %dir.display(), error = %e, "could not create shared dir, browser-start-page.json watcher not started");
            return;
        }
    }

    // Same ordering requirement `spawn_settings_watcher` documents: events()
    // must be called before subscribe_file(), or a change landing in the gap
    // between them is silently missed.
    let mut events = pool.events();
    let _ = pool.subscribe_file(&path);

    tracing::info!(path = %path.display(), "fs_watch subscription active for browser-start-page.json");

    let watched_path = path.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) if is_start_page_file_event(&ev) => {}
                Ok(_) => continue, // some other watched path — not ours
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("browser-start-page watcher event stream closed, stopping");
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "browser-start-page watcher lagged; reloading defensively");
                }
            }
            // Same 300ms burst-collapse debounce as the settings watcher.
            tokio::time::sleep(Duration::from_millis(300)).await;
            while events.try_recv().is_ok() {}

            match read_start_page(&watched_path) {
                Ok(url) => {
                    tracing::info!(path = %watched_path.display(), "browser-start-page.json changed, reloading");
                    config_watcher.update_browser_start_page(url);
                    broadcast_full_config(&config_watcher, &event_bus);
                }
                Err(e) => {
                    tracing::warn!(path = %watched_path.display(), error = %e, "browser-start-page reload parse error (keeping previous value)");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        assert_eq!(read_start_page(&path).unwrap(), None);
    }

    #[test]
    fn empty_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_start_page(&path).unwrap(), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        write_start_page(&path, "https://example.com").unwrap();
        assert_eq!(read_start_page(&path).unwrap(), Some("https://example.com".to_string()));
    }

    #[test]
    fn write_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join(START_PAGE_FILE_NAME);
        write_start_page(&path, "https://example.com").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_overwrites_a_previous_value_wholesale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        write_start_page(&path, "https://example.com").unwrap();
        write_start_page(&path, "https://agentmux.ai").unwrap();
        assert_eq!(read_start_page(&path).unwrap(), Some("https://agentmux.ai".to_string()));
    }

    #[test]
    fn corrupt_json_fails_loud_instead_of_silently_resetting_to_unset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        std::fs::write(&path, "{not valid json").unwrap();
        let err = read_start_page(&path).unwrap_err();
        assert!(err.contains("parse start page"));
    }

    #[test]
    fn a_file_with_an_empty_url_reads_as_none() {
        // Distinguishes "file exists but url is empty" from "file has a
        // real value" — both must be treated as unset, not as a valid
        // empty-string start page.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        std::fs::write(&path, r#"{"url":""}"#).unwrap();
        assert_eq!(read_start_page(&path).unwrap(), None);
    }

    #[test]
    fn is_start_page_file_event_matches_only_the_start_page_filename() {
        let evt = |p: &str, k| FsWatchEvent { path: PathBuf::from(p), kind: k };
        assert!(is_start_page_file_event(&evt(
            "/some/dir/browser-start-page.json",
            FsWatchEventKind::Modified
        )));
        assert!(is_start_page_file_event(&evt(
            "/some/dir/browser-start-page.json",
            FsWatchEventKind::Created
        )));
        assert!(!is_start_page_file_event(&evt("/some/dir/other.json", FsWatchEventKind::Modified)));
    }

    #[test]
    fn is_start_page_file_event_ignores_removed_events() {
        // An external delete must not reset a configured start page back to
        // "unset" and broadcast that to every window.
        let evt = |p: &str, k| FsWatchEvent { path: PathBuf::from(p), kind: k };
        assert!(!is_start_page_file_event(&evt(
            "/some/dir/browser-start-page.json",
            FsWatchEventKind::Removed
        )));
    }

    #[tokio::test]
    async fn a_disk_change_via_the_pool_reloads_config_watcher_and_broadcasts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(START_PAGE_FILE_NAME);
        write_start_page(&path, "https://initial.example").unwrap();

        let pool = FsWatchPool::new();
        let config_watcher = Arc::new(ConfigState::new());
        let event_bus = Arc::new(EventBus::new());

        // subscribe_file directly on the real path (spawn_start_page_watcher
        // resolves via DataPaths::from_env, which isn't set up in a unit
        // test) — exercises the same event-driven reload/broadcast path
        // spawn_start_page_watcher's loop body runs, without needing the
        // env-dependent path resolution.
        let mut events = pool.events();
        let _ = pool.subscribe_file(&path);

        write_start_page(&path, "https://changed.example").unwrap();

        let saw_it = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(ev) = events.recv().await {
                    if is_start_page_file_event(&ev) {
                        return true;
                    }
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(saw_it, "expected a Created/Modified event for the changed start-page file");

        // Drive the same reload the watcher's loop body would.
        let url = read_start_page(&path).unwrap();
        config_watcher.update_browser_start_page(url.clone());
        broadcast_full_config(&config_watcher, &event_bus);

        assert_eq!(url, Some("https://changed.example".to_string()));
        assert_eq!(config_watcher.get_full_config().browser_start_page, url);
    }
}
