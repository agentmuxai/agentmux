// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The version a CLI reports for `--version`, asked once per CLI file instead of once per call.
//!
//! `ResolveCli` runs for every agent that opens (and resumes), and each run spawned
//! `<cli> --version`: a node process that takes 270-300 ms of CPU when the machine is idle
//! (Area54, `claude --version`, warm; 3.2 s the first time) and more than the 5 s timeout
//! when it isn't. Resuming a window of agents on a saturated machine spawned them all at once,
//! most timed out ("get_cli_version timed out after 5s", narko 0.60.6), and every agent waited
//! on its own probe before it could start. A CLI's version only changes when its file does.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// What a version that couldn't be read reads as.
pub(crate) const UNKNOWN: &str = "unknown";

/// A read version is asked for again after this long, in case the file changed without its
/// modified time or size (an in-place update of a shim).
const TTL: Duration = Duration::from_secs(600);
/// A probe that failed (timed out, exited non-zero) is not repeated for this long: the callers
/// that queued behind it share its answer instead of each timing out in turn.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(2);

/// One CLI file as it is now: when it is replaced, it is a different key.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: String,
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Default)]
struct Slot {
    version: Option<(String, Instant)>,
    failed_at: Option<Instant>,
}

static SLOTS: LazyLock<Mutex<HashMap<Key, Arc<tokio::sync::Mutex<Slot>>>>> = LazyLock::new(Default::default);

/// The version of the CLI at `cli_path`: cached while the file is unchanged, and asked for by
/// one `probe` at a time (callers for the same file wait for it). `probe` returns `None` when it
/// couldn't read one; that answers `UNKNOWN` and is never cached. A path with no file behind it
/// (a bare command name) is probed every time.
pub(crate) async fn cached_version<F, Fut>(cli_path: &str, probe: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<String>>,
{
    let Ok(meta) = tokio::fs::metadata(cli_path).await else {
        return probe().await.unwrap_or_else(|| UNKNOWN.to_string());
    };
    let key = Key { path: cli_path.to_string(), modified: meta.modified().ok(), len: meta.len() };
    let slot = {
        let mut slots = SLOTS.lock().unwrap();
        // A replaced file leaves its old key behind; drop it.
        slots.retain(|k, _| k.path != key.path || *k == key);
        slots.entry(key).or_default().clone()
    };
    let mut slot = slot.lock().await;
    if let Some((version, at)) = &slot.version {
        if at.elapsed() < TTL {
            return version.clone();
        }
    }
    if slot.failed_at.is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE) {
        return UNKNOWN.to_string();
    }
    match probe().await {
        Some(version) => {
            slot.version = Some((version.clone(), Instant::now()));
            slot.failed_at = None;
            version
        }
        None => {
            slot.failed_at = Some(Instant::now());
            UNKNOWN.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A CLI file to ask about, and a probe that counts how often it is run.
    fn cli_file(content: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cli");
        std::fs::write(&path, content).unwrap();
        let path = path.to_string_lossy().into_owned();
        (dir, path)
    }

    async fn probe_slowly(runs: &AtomicUsize, answer: Option<&str>) -> Option<String> {
        runs.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        answer.map(str::to_string)
    }

    /// The agents of a window resuming together, all on one CLI: one probe, not one each (before:
    /// `get_cli_version` spawned the CLI for every call).
    #[tokio::test]
    async fn eight_agents_opening_at_once_run_one_probe() {
        let (_dir, path) = cli_file("v1");
        let runs = AtomicUsize::new(0);
        let versions = futures_util::future::join_all((0..8).map(|_| cached_version(&path, || probe_slowly(&runs, Some("2.1.288"))))).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(versions.iter().all(|v| v == "2.1.288"));
    }

    #[tokio::test]
    async fn the_next_agent_to_open_does_not_probe_at_all() {
        let (_dir, path) = cli_file("v1");
        let runs = AtomicUsize::new(0);
        for _ in 0..5 {
            assert_eq!(cached_version(&path, || probe_slowly(&runs, Some("2.1.288"))).await, "2.1.288");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_replaced_cli_file_is_asked_again() {
        let (_dir, path) = cli_file("v1");
        let runs = AtomicUsize::new(0);
        assert_eq!(cached_version(&path, || probe_slowly(&runs, Some("1.0.0"))).await, "1.0.0");
        std::fs::write(&path, "a longer second version").unwrap();
        assert_eq!(cached_version(&path, || probe_slowly(&runs, Some("2.0.0"))).await, "2.0.0");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    /// A probe that timed out is not cached as "unknown" forever, and the callers queued behind it
    /// don't each wait out a probe of their own.
    #[tokio::test]
    async fn a_failed_probe_is_shared_then_retried() {
        let (_dir, path) = cli_file("v1");
        let runs = AtomicUsize::new(0);
        let versions = futures_util::future::join_all((0..4).map(|_| cached_version(&path, || probe_slowly(&runs, None)))).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(versions.iter().all(|v| v == UNKNOWN));

        tokio::time::sleep(RETRY_AFTER_FAILURE + Duration::from_millis(50)).await;
        assert_eq!(cached_version(&path, || probe_slowly(&runs, Some("2.1.288"))).await, "2.1.288");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_bare_command_name_is_probed_every_time() {
        let runs = AtomicUsize::new(0);
        for _ in 0..3 {
            cached_version("no-such-file-claude", || probe_slowly(&runs, Some("1.0.0"))).await;
        }
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }
}
