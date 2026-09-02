// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Out-of-band write detection for native memory files — §4.5 of
//! docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md.
//! Claude Code's CLI writes memory `.md` files directly to
//! `~/.claude/projects/<hash>/memory/*.md`, with no obligation to go
//! through `agent:memory:write_file`/`memory.write` — an agent's own
//! general-purpose filesystem tools can (and, per this spec's own
//! motivating incident, did) write there directly. When that happens, no
//! RPC fires, so nothing in `native_memory_handlers.rs` or `app_api::mod`
//! ever sees it.
//!
//! Two independent layers close that gap, so failure of one degrades
//! rather than blinds the whole system:
//!
//! 1. **Fast path** ([`spawn_fast_path`]) — subscribe to every known
//!    agent's memory directory via the shared `FsWatchPool`
//!    (`backend::fs_watch`), and record a new `external_fs_write` version
//!    on any change whose hash doesn't match the latest recorded version.
//! 2. **Slow path** ([`reconciliation_sweep_once`]) — a periodic sweep, run
//!    on the same cadence class as `FsWatchPool`'s own
//!    `HEALTH_SWEEP_INTERVAL` (30s), that hashes every known agent's live
//!    memory files and compares against the latest recorded version — the
//!    backstop for a missed fs-watch event, a watch still in retry-backoff,
//!    or srv having been down entirely when the write happened.
//!
//! Both layers call the same [`check_and_record_drift`] — the only
//! difference between them is how they learn a file might have changed.
//! Neither promises zero data loss (see the spec's §4.5 "precision,
//! honestly bounded" section): two out-of-band writes to the same file
//! between one sweep and the next collapse into one detected version, and
//! `source_detail` never guesses a `session_id` for a drift-detected write.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::backend::fs_watch::{FsWatchEventKind, FsWatchPool};
use crate::backend::storage::store::Store;
use crate::server::native_memory_handlers::{list_all_memory_targets, memory_dir_for_agent_by_id};

/// Same cadence class as `FsWatchPool::HEALTH_SWEEP_INTERVAL`
/// (`backend/fs_watch/recovery.rs`) — chosen for the same reason: frequent
/// enough that an operator reviewing history isn't waiting long for a
/// drifted write to surface, infrequent enough not to matter for a
/// background sweep over what's normally a handful of small markdown files
/// per agent.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// Same cap `native_memory_handlers.rs`/`app_api::mod` already use for a
/// memory file's live content.
const MAX_MEMORY_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Compare `live_content` against the latest recorded version of
/// `(agent_id, filename)`; if different (including "no version recorded
/// yet"), insert a new version tagged `source: "external_fs_write"` with
/// `source_detail: {"detected_via": detected_via}`. Returns `Ok(true)` if a
/// new version was recorded, `Ok(false)` if the content already matched
/// (no drift to report).
pub(crate) fn check_and_record_drift(
    id_store: &Store,
    agent_id: &str,
    filename: &str,
    live_content: &str,
    detected_via: &str,
) -> Result<bool, String> {
    let detail = serde_json::json!({ "detected_via": detected_via }).to_string();
    // reagent P2 on PR #2675: the fast path (per fs-watch event) and slow
    // path (30s sweep) run as concurrent, independent tokio tasks and can
    // both observe the same file change — a plain "read latest, then
    // separately insert" here (two Store calls, each independently
    // locking/unlocking) is not atomic as a compound operation, so both
    // could read the same stale "latest" and insert duplicate
    // external_fs_write rows for one actual change.
    // `agent_native_memory_version_insert_if_changed` does the compare
    // AND the insert under a single connection-lock acquisition, closing
    // that race — see its own doc comment for the full rationale.
    id_store
        .agent_native_memory_version_insert_if_changed(agent_id, filename, live_content, "external_fs_write", &detail, "")
        .map_err(|e| e.to_string())
        .map(|v| v.is_some())
}

/// Read one `.md` file's content the same way the rest of the native-memory
/// code does: lossy UTF-8. Unlike `native_memory_handlers.rs`'s own
/// mirror-refresh path — which truncates an oversized file and separately
/// reports the filename so an interactive caller (`bundle.export_for_agent`)
/// can surface a warning — the drift detector has no interactive caller to
/// warn. Silently truncating here would instead record the truncated tail
/// as a new *version*, permanently misrepresenting content the file never
/// actually contained as if it were authoritative (reagent P2 on PR
/// #2675). So an oversized file is refused outright (`ErrorKind::
/// InvalidData`, distinguishable from a transient I/O error) rather than
/// read partially; callers log and skip it, same treatment as any other
/// per-file read failure.
fn read_memory_file_lossy(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    let read = std::fs::File::open(path)?
        .take(MAX_MEMORY_FILE_BYTES + 1)
        .read_to_end(&mut buf)?;
    if read as u64 > MAX_MEMORY_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("file exceeds the {MAX_MEMORY_FILE_BYTES}-byte cap — refusing to record truncated content as authoritative"),
        ));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// One pass over every agent definition with a configured working
/// directory: hash each live `.md` file in its memory dir and record drift
/// if found. Returns the number of drifted files recorded (for logging;
/// tests assert on it directly). Errors from an individual agent/file are
/// logged and swallowed — one agent's bad state (e.g. a permissions issue)
/// must not stop the sweep from covering every other agent.
///
/// Also detects and publishes OUT-OF-BAND DELETIONS (Codex P2, PR #2932):
/// content-drift detection alone only ever compares files that still exist
/// against their last recorded version — a `.md` file removed directly (not
/// through `agent:memory:write_file`/`revert`, neither of which delete
/// files) never surfaces there, and the fast fs-watch path deliberately
/// ignores `FsWatchEventKind::Removed` (it exists to catch content changes,
/// not track deletions). Without this, the grid count and an open detail
/// view for the deleted file would stay stale indefinitely — no event ever
/// arrives to prompt the frontend's own "the selected file is now gone"
/// handling to fire.
///
/// `deleted_notified` is this sweep's own persistent, cross-tick state
/// (threaded in from `spawn_slow_path`'s loop, same pattern the fast path's
/// `watched_dirs`/`dir_to_agent` already use) — without it, a file that
/// STAYS deleted would republish an event every single 30s tick forever,
/// since "missing from disk" is true on every subsequent sweep too, not
/// just the first one. A filename is removed from this set again the
/// moment it reappears on disk (recreated with new content), so a later
/// re-deletion is detected fresh rather than staying permanently suppressed.
pub(crate) fn reconciliation_sweep_once(
    wstore: &Store,
    id_store: &Store,
    broker: &crate::backend::wps::Broker,
    deleted_notified: &mut HashSet<(String, String)>,
) -> usize {
    let mut drifted = 0;
    let targets = list_all_memory_targets(wstore);
    for (agent_id, memory_dir) in &targets {
        drifted += sweep_one_agent_dir(id_store, agent_id, memory_dir, "reconciliation_sweep", broker);
    }

    // agent_native_memory_version_list_distinct_files() is unscoped (every
    // agent's every file with recorded history) — the same primitive
    // native_memory_retention.rs already uses as its own work list, not a
    // new query pattern introduced here.
    let Ok(distinct) = id_store.agent_native_memory_version_list_distinct_files() else {
        return drifted;
    };
    for (agent_id, filename) in distinct {
        let Some((_, memory_dir)) = targets.iter().find(|(id, _)| *id == agent_id) else {
            // Not (or no longer) a tracked target this sweep — out of scope
            // for this pass; list_all_memory_targets is the same
            // authoritative source the content-drift loop above already
            // trusts for "which agents/dirs are live right now."
            continue;
        };
        let key = (agent_id.clone(), filename.clone());
        if memory_dir.join(&filename).is_file() {
            deleted_notified.remove(&key);
            continue;
        }
        if deleted_notified.insert(key) {
            tracing::info!(agent_id = %agent_id, filename = %filename, "native_memory_drift: detected an out-of-band deletion");
            broker.publish(crate::backend::wps::WaveEvent {
                event: format!("agent:memory:changed:{agent_id}"),
                scopes: vec![],
                sender: String::new(),
                persist: 0,
                data: None,
            });
        }
    }
    drifted
}

fn sweep_one_agent_dir(
    id_store: &Store,
    agent_id: &str,
    memory_dir: &Path,
    detected_via: &str,
    broker: &crate::backend::wps::Broker,
) -> usize {
    let entries = match std::fs::read_dir(memory_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            tracing::warn!(agent_id, dir = %memory_dir.display(), error = %e, "native_memory_drift: sweep: read_dir failed");
            return 0;
        }
    };

    let mut drifted = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") {
            continue;
        }
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_file() {
            continue;
        }
        let content = match read_memory_file_lossy(&entry.path()) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                tracing::warn!(agent_id, filename = %name, error = %e, "native_memory_drift: sweep: file exceeds size cap, skipping (not recording truncated content)");
                continue;
            }
            Err(e) => {
                tracing::warn!(agent_id, filename = %name, error = %e, "native_memory_drift: sweep: read failed, retrying next sweep");
                continue;
            }
        };
        match check_and_record_drift(id_store, agent_id, &name, &content, detected_via) {
            Ok(true) => {
                drifted += 1;
                tracing::info!(agent_id, filename = %name, detected_via, "native_memory_drift: recorded an out-of-band write");
                // Reactive Armory updates (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md).
                // Only on an actual recorded change (Ok(true)) — this branch
                // runs once per sweep tick per watched agent, and firing on
                // every no-op tick would mean an event for every agent every
                // 30s regardless of whether anything happened.
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agent:memory:changed:{agent_id}"),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(agent_id, filename = %name, error = %e, "native_memory_drift: check_and_record_drift failed"),
        }
    }
    drifted
}

/// Start both layers. Call once at server startup with a fully-built
/// `AppState`'s `fs_watch_pool`/`wstore`/`id_store`. Returns immediately —
/// both loops run as spawned background tasks for the lifetime of the
/// process (no shutdown handle; srv itself owns the process lifetime).
pub fn spawn(
    fs_watch_pool: Arc<FsWatchPool>,
    wstore: Arc<Store>,
    id_store: Arc<Store>,
    broker: Arc<crate::backend::wps::Broker>,
) {
    spawn_fast_path(fs_watch_pool, wstore.clone(), id_store.clone(), broker.clone());
    spawn_slow_path(wstore, id_store, broker);
}

fn spawn_slow_path(wstore: Arc<Store>, id_store: Arc<Store>, broker: Arc<crate::backend::wps::Broker>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        // Persistent across ticks, same pattern as the fast path's own
        // watched_dirs/dir_to_agent — see reconciliation_sweep_once's own
        // doc comment on why this can't just be a fresh HashSet per call.
        let mut deleted_notified: HashSet<(String, String)> = HashSet::new();
        loop {
            tick.tick().await;
            reconciliation_sweep_once(&wstore, &id_store, &broker, &mut deleted_notified);
        }
    });
}

/// Maintains a `memory_dir -> agent_id` map, refreshed every
/// [`SWEEP_INTERVAL`] tick (piggybacking on the same cadence as the slow
/// path rather than requiring a separate agent-spawn/teardown hook — a
/// newly-defined agent is picked up within one tick, not instantly, which
/// is an acceptable, honestly-bounded lag for a fast path whose whole
/// purpose is narrowing a gap, not eliminating it). `FsWatchPool`
/// subscriptions have no `Drop`-based teardown (see `pool.rs` — holding or
/// discarding a `Subscription` value doesn't affect the underlying watch),
/// so a dir is watched for the life of the process once first seen; we
/// only need to track which dirs we've already called `subscribe_dir` on
/// to avoid redundant re-subscription every tick.
fn spawn_fast_path(
    fs_watch_pool: Arc<FsWatchPool>,
    wstore: Arc<Store>,
    id_store: Arc<Store>,
    broker: Arc<crate::backend::wps::Broker>,
) {
    tokio::spawn(async move {
        let mut events = fs_watch_pool.events();
        let mut watched_dirs: HashSet<PathBuf> = HashSet::new();
        let mut dir_to_agent: std::collections::HashMap<PathBuf, String> = std::collections::HashMap::new();
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    refresh_subscriptions(&fs_watch_pool, &wstore, &mut watched_dirs, &mut dir_to_agent);
                }
                event = events.recv() => {
                    let event = match event {
                        Ok(e) => e,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // "a wake signal, not a guaranteed delivery log"
                            // (FsWatchPool's own doc) — a lagged receiver
                            // resyncs via the next reconciliation sweep
                            // rather than trying to catch up here.
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if !matches!(event.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified) {
                        continue;
                    }
                    let Some(filename) = event.path.file_name().and_then(|n| n.to_str()) else { continue };
                    if !filename.ends_with(".md") {
                        continue;
                    }
                    let Some(parent) = event.path.parent() else { continue };
                    let Some(agent_id) = dir_to_agent.get(parent).cloned() else { continue };

                    let content = match read_memory_file_lossy(&event.path) {
                        Ok(c) => c,
                        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                            tracing::warn!(agent_id, filename, error = %e, "native_memory_drift: fast path: file exceeds size cap, skipping (not recording truncated content)");
                            continue;
                        }
                        // TOCTOU-deleted or transient I/O between the event
                        // and this read — non-fatal, the reconciliation
                        // sweep covers it if the write actually stuck.
                        Err(_) => continue,
                    };
                    // Restructured from `if let Err(...) {} else {}` to a full
                    // match while adding the publish below: the old `else`
                    // branch fired on Ok(false) too (content already matched,
                    // nothing recorded), logging "recorded an out-of-band
                    // write" for a no-op — a pre-existing log inaccuracy
                    // fixed as a side effect of needing this branch anyway.
                    match check_and_record_drift(&id_store, &agent_id, filename, &content, "fs_watch") {
                        Ok(true) => {
                            tracing::info!(agent_id, filename, "native_memory_drift: fast path recorded an out-of-band write");
                            // Reactive Armory updates (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md).
                            broker.publish(crate::backend::wps::WaveEvent {
                                event: format!("agent:memory:changed:{agent_id}"),
                                scopes: vec![],
                                sender: String::new(),
                                persist: 0,
                                data: None,
                            });
                        }
                        Ok(false) => {}
                        Err(e) => tracing::warn!(agent_id, filename, error = %e, "native_memory_drift: fast path: check_and_record_drift failed"),
                    }
                }
            }
        }
    });
}

fn refresh_subscriptions(
    fs_watch_pool: &Arc<FsWatchPool>,
    wstore: &Store,
    watched_dirs: &mut HashSet<PathBuf>,
    dir_to_agent: &mut std::collections::HashMap<PathBuf, String>,
) {
    for (agent_id, memory_dir) in list_all_memory_targets(wstore) {
        // subscribe_dir canonicalizes internally; canonicalize here too so
        // watched_dirs/dir_to_agent key on the same form an incoming
        // event's path will actually have (events report canonical paths —
        // see subscribe_dir's own doc comment).
        let canonical = memory_dir.canonicalize().unwrap_or_else(|_| memory_dir.clone());
        if watched_dirs.insert(canonical.clone()) {
            fs_watch_pool.subscribe_dir(&memory_dir);
            dir_to_agent.insert(canonical, agent_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    #[test]
    fn first_observation_of_a_file_is_drift() {
        let store = shared_store();
        let drifted = check_and_record_drift(&store, "agent-1", "MEMORY.md", "content", "fs_watch").unwrap();
        assert!(drifted, "a file with no prior version must count as drift");

        let history = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source, "external_fs_write");
        assert!(history[0].source_detail.contains("fs_watch"));
    }

    #[test]
    fn unchanged_content_is_not_drift() {
        let store = shared_store();
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "same", "human", "{}", "")
            .unwrap();

        let drifted = check_and_record_drift(&store, "agent-1", "MEMORY.md", "same", "fs_watch").unwrap();
        assert!(!drifted, "identical content must not record a new version");

        let history = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(history.len(), 1, "no new version should have been inserted");
    }

    #[test]
    fn changed_content_is_drift_and_chains_onto_the_prior_version() {
        let store = shared_store();
        let v1 = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v1", "human", "{}", "")
            .unwrap();

        let drifted = check_and_record_drift(&store, "agent-1", "MEMORY.md", "v2 — written outside AgentMux", "reconciliation_sweep").unwrap();
        assert!(drifted);

        let history = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].parent_version_id, Some(v1.id));
        assert_eq!(history[0].source, "external_fs_write");
        assert!(history[0].source_detail.contains("reconciliation_sweep"));
    }

    #[test]
    fn sweep_finds_a_file_written_directly_to_disk() {
        let store = shared_store();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "written outside any RPC").unwrap();

        let broker = crate::backend::wps::Broker::new();
        let drifted = sweep_one_agent_dir(&store, "agent-1", tmp.path(), "reconciliation_sweep", &broker);
        assert_eq!(drifted, 1);

        let history = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source, "external_fs_write");
    }

    // SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md: an out-of-band write
    // (bypassing both the WS RPC and App API surfaces entirely) must still
    // make the Armory grid reactive — that's this whole module's reason to
    // exist, so its own publish path needs the same regression guard the
    // RPC handlers' publish calls get.
    #[test]
    fn sweep_publishes_agent_memory_changed_when_drift_is_recorded() {
        let store = shared_store();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "written outside any RPC").unwrap();

        let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-drift-1");
        sweep_one_agent_dir(&store, "agent-drift-1", tmp.path(), "reconciliation_sweep", &broker);

        let events = client.received_events();
        assert_eq!(events.len(), 1, "a real drift must publish exactly one event");
        assert_eq!(events[0].1.event, "agent:memory:changed:agent-drift-1");
    }

    // The other half of the same guard: a sweep tick that finds NO drift
    // (unchanged content) must NOT publish — the reconciliation sweep runs
    // every 30s for every watched agent regardless of whether anything
    // changed, so publishing unconditionally would fire an event storm with
    // no corresponding real change, defeating the point of an event-driven
    // refresh (see this module's own SPEC section on why insert_if_changed's
    // no-op case is explicitly excluded).
    #[test]
    fn sweep_does_not_publish_when_content_is_unchanged() {
        let store = shared_store();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "same content").unwrap();

        let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-drift-2");
        // First sweep: genuinely new content, records a version (and would
        // publish — unobserved here since this test is about the SECOND
        // sweep finding nothing new).
        sweep_one_agent_dir(&store, "agent-drift-2", tmp.path(), "reconciliation_sweep", &broker);
        // Second sweep over the same, unchanged file — no drift to record.
        sweep_one_agent_dir(&store, "agent-drift-2", tmp.path(), "reconciliation_sweep", &broker);

        let events = client.received_events();
        assert_eq!(events.len(), 1, "only the first sweep's real drift may publish; the second (no-op) must not");
    }

    // reagent P2 on PR #2675: read_memory_file_lossy previously truncated
    // an oversized file and returned the truncated bytes as if they were
    // the whole file — the sweep would then record that truncated content
    // as a new, authoritative version. It must instead refuse to read the
    // file at all, so no version (correct or corrupted) is recorded.
    #[test]
    fn sweep_refuses_to_record_an_oversized_file_as_authoritative() {
        let store = shared_store();
        let tmp = tempfile::tempdir().unwrap();
        let oversized = "x".repeat((MAX_MEMORY_FILE_BYTES + 1) as usize);
        std::fs::write(tmp.path().join("MEMORY.md"), &oversized).unwrap();

        let broker = crate::backend::wps::Broker::new();
        let drifted = sweep_one_agent_dir(&store, "agent-1", tmp.path(), "reconciliation_sweep", &broker);
        assert_eq!(drifted, 0, "an oversized file must not be recorded as drift");

        let history = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert!(history.is_empty(), "no truncated content may be recorded as a version");
    }

    #[test]
    fn read_memory_file_lossy_reads_a_file_at_exactly_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("MEMORY.md");
        let at_cap = "x".repeat(MAX_MEMORY_FILE_BYTES as usize);
        std::fs::write(&path, &at_cap).unwrap();

        let content = read_memory_file_lossy(&path).unwrap();
        assert_eq!(content.len(), MAX_MEMORY_FILE_BYTES as usize);
    }

    #[test]
    fn read_memory_file_lossy_rejects_a_file_one_byte_over_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("MEMORY.md");
        let over_cap = "x".repeat((MAX_MEMORY_FILE_BYTES + 1) as usize);
        std::fs::write(&path, &over_cap).unwrap();

        let err = read_memory_file_lossy(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn sweep_ignores_non_md_files_and_missing_directories() {
        let store = shared_store();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "not a memory file").unwrap();

        let broker = crate::backend::wps::Broker::new();
        assert_eq!(sweep_one_agent_dir(&store, "agent-1", tmp.path(), "reconciliation_sweep", &broker), 0);
        assert_eq!(
            sweep_one_agent_dir(&store, "agent-1", &tmp.path().join("does-not-exist"), "reconciliation_sweep", &broker),
            0
        );
    }

    #[test]
    fn sweep_scopes_versions_to_the_given_agent_id() {
        let store = shared_store();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "content").unwrap();

        let broker = crate::backend::wps::Broker::new();
        sweep_one_agent_dir(&store, "agent-specific", tmp.path(), "reconciliation_sweep", &broker);

        assert_eq!(store.agent_native_memory_version_list("agent-specific", "MEMORY.md").unwrap().len(), 1);
        assert_eq!(store.agent_native_memory_version_list("agent-other", "MEMORY.md").unwrap().len(), 0);
    }

    #[tokio::test]
    async fn reconciliation_sweep_once_covers_every_agent_with_a_working_directory() {
        // list_all_memory_targets also consults the global named-agent
        // registry (added for reagent P1 on PR #2675) — without isolating
        // AGENTMUX_SHARED_DIR here, this test would sweep whatever real
        // live agents happen to be registered on the machine running the
        // suite, not just the two synthetic ones it sets up below.
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_shared_dir = std::env::var_os("AGENTMUX_SHARED_DIR");
        let shared = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", shared.path());

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let id_store = Store::open_shared(tmp.path()).unwrap();
        let state = crate::server::tests::test_state();

        let config_a = tempfile::tempdir().unwrap();
        let config_b = tempfile::tempdir().unwrap();
        for (id, dir) in [("sweep-agent-a", config_a.path()), ("sweep-agent-b", config_b.path())] {
            let mut def = crate::backend::storage::AgentDefinition {
                conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
                id: id.to_string(),
                slug: id.to_string(),
                name: "Test".to_string(),
                icon: String::new(),
                provider: "claude".to_string(),
                description: String::new(),
                working_directory: format!("/work/{id}"),
                shell: String::new(),
                provider_flags: String::new(),
                auto_start: 0,
                restart_on_crash: 0,
                idle_timeout_minutes: 0,
                created_at: 0,
                agent_type: "host".to_string(),
                environment: String::new(),
                agent_bus_id: String::new(),
                is_seeded: 0,
                accounts: String::new(),
                parent_id: String::new(),
                branch_label: String::new(),
                updated_at: 0,
                user_hidden: 0,
                container_image: String::new(),
                container_volumes: "[]".to_string(),
                container_name: String::new(),
                use_ambient_login: 0,
                auto_continue_enabled: 0,
                model_vendor_base_url: String::new(),
                memory_id: String::new(),
            };
            state.wstore.agent_def_insert(&mut def).unwrap();
            state
                .wstore
                .agent_content_set(&crate::backend::storage::AgentContent {
                    agent_id: id.to_string(),
                    content_type: "env".to_string(),
                    content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
                    updated_at: 0,
                })
                .unwrap();

            let memory_dir = memory_dir_for_agent_by_id(&state.wstore, &def).unwrap();
            std::fs::create_dir_all(&memory_dir).unwrap();
            std::fs::write(memory_dir.join("MEMORY.md"), format!("content for {id}")).unwrap();
        }

        let broker = crate::backend::wps::Broker::new();
        let mut deleted_notified = HashSet::new();
        let drifted = reconciliation_sweep_once(&state.wstore, &id_store, &broker, &mut deleted_notified);
        assert_eq!(drifted, 2, "sweep must cover every agent with a working directory");
        assert_eq!(id_store.agent_native_memory_version_list("sweep-agent-a", "MEMORY.md").unwrap().len(), 1);
        assert_eq!(id_store.agent_native_memory_version_list("sweep-agent-b", "MEMORY.md").unwrap().len(), 1);

        match prev_shared_dir {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
    }

    // Codex P2, PR #2932: content-drift detection alone never notices a
    // file that's gone (nothing to compare content against), and the fast
    // path deliberately ignores FsWatchEventKind::Removed — without this,
    // an out-of-band deletion of the file open in the Armory detail view
    // stayed stale forever, since no event ever arrived to trigger the
    // frontend's own "the selected file is gone" handling.
    #[tokio::test]
    async fn reconciliation_sweep_detects_an_out_of_band_deletion_once_then_suppresses_repeats_until_recreated() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_shared_dir = std::env::var_os("AGENTMUX_SHARED_DIR");
        let shared = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_SHARED_DIR", shared.path());

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let id_store = Store::open_shared(tmp.path()).unwrap();
        let state = crate::server::tests::test_state();
        let config = tempfile::tempdir().unwrap();

        let mut def = crate::backend::storage::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: "sweep-delete-agent".to_string(),
            slug: "sweep-delete-agent".to_string(),
            name: "Test".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: "/work/sweep-delete-agent".to_string(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        state.wstore.agent_def_insert(&mut def).unwrap();
        state
            .wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: def.id.clone(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", config.path().display()),
                updated_at: 0,
            })
            .unwrap();
        let memory_dir = memory_dir_for_agent_by_id(&state.wstore, &def).unwrap();
        std::fs::create_dir_all(&memory_dir).unwrap();
        let file_path = memory_dir.join("MEMORY.md");
        std::fs::write(&file_path, "content").unwrap();

        let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:sweep-delete-agent");
        let mut deleted_notified = HashSet::new();

        // Sweep 1: file present — establishes the recorded version (and
        // publishes once, from ordinary content-drift, not deletion logic).
        reconciliation_sweep_once(&state.wstore, &id_store, &broker, &mut deleted_notified);
        assert_eq!(client.received_events().len(), 1);

        // Delete the file out of band (not through write_file/revert).
        std::fs::remove_file(&file_path).unwrap();

        // Sweep 2: first sweep to observe the deletion — must publish.
        reconciliation_sweep_once(&state.wstore, &id_store, &broker, &mut deleted_notified);
        assert_eq!(client.received_events().len(), 2, "the sweep that first observes a deletion must publish");

        // Sweep 3: file still gone — must NOT publish again (suppressed;
        // otherwise a permanently-deleted file would fire an event every
        // 30s tick forever).
        reconciliation_sweep_once(&state.wstore, &id_store, &broker, &mut deleted_notified);
        assert_eq!(client.received_events().len(), 2, "a repeat sweep over an already-reported deletion must not re-publish");

        // Recreate the file — content-drift detects it as a new version
        // (publish #3) and clears the deletion-suppression for this file.
        std::fs::write(&file_path, "recreated content").unwrap();
        reconciliation_sweep_once(&state.wstore, &id_store, &broker, &mut deleted_notified);
        assert_eq!(client.received_events().len(), 3);

        // Delete it again — must be detected as a FRESH deletion (publish
        // #4), proving the suppression state was actually cleared on
        // recreation, not permanently stuck once a file is ever deleted once.
        std::fs::remove_file(&file_path).unwrap();
        reconciliation_sweep_once(&state.wstore, &id_store, &broker, &mut deleted_notified);
        assert_eq!(client.received_events().len(), 4, "a re-deletion after recreation must be detected as a fresh event");

        match prev_shared_dir {
            Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
            None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
        }
    }
}
