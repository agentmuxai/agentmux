// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reducer event log: an in-memory ring buffer of recent
//! [`Event`]s for replay, plus an optional append-only JSON-lines disk
//! stream for crash forensics.
//!
//! This module used to exist twice — `agentmux-launcher/src/event_log.rs`
//! (Phase D.2) and a byte-for-byte mirror in `agentmux-srv/src/event_log.rs`
//! (Phase E.1b), 415 lines each, whose only code-level difference was
//! *which logging sink* a warning went to. The srv copy's own header
//! carried the to-do: *"Phase E.7 cleanup: lift the shared parts into
//! agentmux-common and unify launcher + srv event logs. (reagent P2
//! #610.)"* This is that lift
//! (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §2.2).
//!
//! The one real divergence is preserved as a parameter: [`run_disk_writer`]
//! takes a [`WarnSink`], because the launcher logs to its own rotating
//! file (`crate::log`) and has **no** `tracing` subscriber installed, while
//! srv logs via `tracing::warn!`. Hard-coding either would silently drop
//! the other crate's warnings.
//!
//! Two roles, kept clean:
//!
//! 1. **In-memory ring** is the source of truth for replay during the
//!    owning process's lifetime. `GetEvents { since: u64 }` reads from
//!    it. Bounded by `MAX_RING_EVENTS` to stop unbounded memory growth in
//!    long sessions; oldest events evict first. A subscriber that fell
//!    behind further than the ring's coverage gets an `EventList` of
//!    whatever's still in the ring + a flag indicating they may have
//!    missed some — it's their job to recover by treating subsequent
//!    events as authoritative.
//!
//! 2. **Disk file** is purely forensic. Append-only JSON-lines. Survives
//!    a crash so an operator can post-mortem "what was it doing right
//!    before it died." Fire-and-forget: write failures log a warning but
//!    never block the in-memory path. Rotated when it exceeds
//!    `MAX_DISK_BYTES` (renames current to `.old`, starts fresh).
//!
//! Why both: in-memory satisfies D.3's resync needs at zero I/O cost in
//! the happy path. Disk satisfies the "what happened before the crash"
//! debugging story without complicating the replay reader.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;

use crate::ipc::Event;

/// Where [`run_disk_writer`] sends its warnings. Each owning crate binds
/// its own logging facility — see the module doc for why this is a
/// parameter rather than a hard-coded `tracing::warn!`.
pub type WarnSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Cap on in-memory ring size. 4096 events is comfortable for
/// realistic resync windows (~minutes of activity at typical
/// reducer event rates: 10–50 events per user action). Tunable
/// upward if forensics in long sessions show truncation; downward
/// would only be useful if we measured noticeable memory pressure
/// from this (we don't — Event is small and there are 4096 of them).
const MAX_RING_EVENTS: usize = 4096;

/// Cap on disk file size before rotation. 8 MiB ≈ 4–8K events
/// depending on event variant. Two-file rotation: when current
/// exceeds this, rename to `.old` (overwriting any prior `.old`)
/// and start fresh. Total worst-case footprint: 2 × MAX_DISK_BYTES.
const MAX_DISK_BYTES: u64 = 8 * 1024 * 1024;

/// Append-only ring + optional disk persistence.
///
/// Cloneable via `Arc<EventLog>` from the IPC server context;
/// `append` and `events_since` are cheap (Mutex held for
/// microseconds — Vec<Event> push / scan, no I/O on the
/// in-memory path).
///
/// Disk writes happen on a dedicated tokio task that subscribes
/// to the broadcast bus separately from the in-memory append.
/// This means the in-memory ring is updated synchronously from
/// the IPC server's dispatch path; disk persistence runs at its
/// own pace and may lag.
#[derive(Debug)]
pub struct EventLog {
    ring: Mutex<VecDeque<Event>>,
    disk_path: Option<PathBuf>,
}

impl EventLog {
    /// Construct an event log. `disk_path = None` disables disk
    /// persistence (used in tests where filesystem state is
    /// inconvenient). The on-disk file is created on first append;
    /// no upfront I/O.
    pub fn new(disk_path: Option<PathBuf>) -> Self {
        Self {
            ring: Mutex::new(VecDeque::with_capacity(MAX_RING_EVENTS)),
            disk_path,
        }
    }

    /// Append an event to the in-memory ring. Evicts the oldest
    /// entry when the ring is at capacity. Synchronous,
    /// O(1)-amortized.
    pub fn append(&self, event: Event) {
        let mut ring = self.ring.lock().expect("event-log ring mutex poisoned");
        if ring.len() == MAX_RING_EVENTS {
            ring.pop_front();
        }
        ring.push_back(event);
    }

    /// Snapshot of all events currently in the ring with version >
    /// `since`. Returned in insertion order (oldest first), so the
    /// caller applies them sequentially.
    ///
    /// Phase D.3 — used by `Command::GetEvents { since }` to
    /// produce the replay slice. The snapshot is taken at-call-time;
    /// events arriving after this returns are NOT included (the
    /// subscriber sees them on the live broadcast stream).
    pub fn events_since(&self, since: u64) -> Vec<Event> {
        let ring = self.ring.lock().expect("event-log ring mutex poisoned");
        ring.iter()
            .filter(|e| event_version(e) > since)
            .cloned()
            .collect()
    }

    /// True if the requested `since` version is older than the
    /// oldest event in the ring (i.e. the subscriber missed events
    /// that have already been evicted). Caller should treat the
    /// resulting `events_since` slice as best-effort and may need
    /// to re-fetch a snapshot to recover canonical state.
    pub fn replay_truncated(&self, since: u64) -> bool {
        let ring = self.ring.lock().expect("event-log ring mutex poisoned");
        match ring.front() {
            // Phase E.1a (codex P2 #608) — saturating_add guards
            // against `since: u64::MAX` overflow. Wire input is
            // externally reachable; debug builds would panic, release
            // would wrap. Saturating means "since == u64::MAX" trivially
            // returns false (oldest can't exceed u64::MAX), which is
            // the correct semantic — there's no possible event newer
            // than that, so there can't be a gap.
            Some(oldest) => event_version(oldest) > since.saturating_add(1),
            None => false,
        }
    }

    /// Disk path for the writer task to flush to. None when disk
    /// persistence is disabled.
    pub fn disk_path(&self) -> Option<&PathBuf> {
        self.disk_path.as_ref()
    }
}

/// Background task: write events to the disk file as they arrive
/// on the broadcast bus. Rotates when the file exceeds
/// `MAX_DISK_BYTES`.
///
/// Spawned once per process run. Holds a tokio broadcast receiver
/// and the EventLog's disk path. Failures are reported through
/// `warn` and the event is dropped from the disk stream (the
/// in-memory ring is unaffected — disk is forensics-only).
pub async fn run_disk_writer(
    log: Arc<EventLog>,
    mut events_rx: tokio::sync::broadcast::Receiver<Event>,
    warn: WarnSink,
) {
    let path = match log.disk_path() {
        Some(p) => p.clone(),
        None => return, // disk persistence disabled — exit task
    };
    let rotated_path = path.with_extension("log.old");

    let mut file = match open_for_append(&path).await {
        Ok(f) => f,
        Err(e) => {
            warn(&format!(
                "[event-log] cannot open {} for append: {} — disk persistence disabled",
                path.display(),
                e
            ));
            return;
        }
    };
    let mut bytes_written = file.metadata().await.map(|m| m.len()).unwrap_or(0);

    loop {
        match events_rx.recv().await {
            Ok(event) => {
                let mut buf = match serde_json::to_vec(&event) {
                    Ok(b) => b,
                    Err(e) => {
                        warn(&format!("[event-log] serialize failed: {}", e));
                        continue;
                    }
                };
                buf.push(b'\n');
                if bytes_written + buf.len() as u64 > MAX_DISK_BYTES {
                    // Rotate: drop the writer, rename current →
                    // .old (overwriting), reopen fresh. Failures
                    // here are non-fatal; we log and keep writing
                    // to the existing file (it'll just exceed cap).
                    drop(file);
                    if let Err(e) = tokio::fs::rename(&path, &rotated_path).await {
                        warn(&format!(
                            "[event-log] rotation rename failed: {} — continuing without rotation",
                            e
                        ));
                    }
                    file = match open_for_append(&path).await {
                        Ok(f) => f,
                        Err(e) => {
                            warn(&format!(
                                "[event-log] post-rotation open failed: {} — disk persistence stopping",
                                e
                            ));
                            return;
                        }
                    };
                    bytes_written = 0;
                }
                if let Err(e) = file.write_all(&buf).await {
                    warn(&format!(
                        "[event-log] write failed: {} — dropping event from disk stream",
                        e
                    ));
                    continue;
                }
                // Phase E.1a — durable: fsync per append so events
                // written before a crash survive it. Required by
                // Phase E §6.4 for srv's bootstrap-replay correctness.
                // Latency cost: ~ms per event vs microseconds without
                // sync. Acceptable for our event volume (~10–50 per
                // user action). Trade-off considered: batched fsync
                // (group commit). Skipping for E.1a — premature
                // optimization; revisit if profiling shows a hot path.
                if let Err(e) = file.sync_data().await {
                    warn(&format!(
                        "[event-log] sync_data failed: {} — event written but not fsynced",
                        e
                    ));
                }
                bytes_written += buf.len() as u64;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn(&format!("[event-log] disk writer lagged, missed {} events", n));
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let _ = file.flush().await;
                return;
            }
        }
    }
}

async fn open_for_append(path: &std::path::Path) -> std::io::Result<tokio::fs::File> {
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
}

/// Extract the `version` field from any Event variant. When a new
/// variant is added that carries a version, add it here too — the
/// exhaustive match catches future-variant compile errors.
pub fn event_version(e: &Event) -> u64 {
    match e {
        Event::ProcessSpawned { version, .. }
        | Event::ProcessExited { version, .. }
        | Event::LifecyclePhaseChanged { version, .. }
        | Event::Registered { version, .. }
        | Event::Pong { version, .. }
        | Event::WindowOpened { version, .. }
        | Event::WindowClosed { version, .. }
        | Event::PoolWindowAdded { version, .. }
        | Event::PoolWindowRemoved { version, .. }
        | Event::PoolWindowPromoted { version, .. }
        | Event::PanesReaped { version, .. }
        | Event::PoolDrained { version, .. }
        | Event::PoolNotLast { version, .. }
        | Event::WindowInstanceAssigned { version, .. }
        | Event::WindowInstanceReleased { version, .. }
        | Event::BackendWindowIdRegistered { version, .. }
        | Event::BackendWindowIdUnregistered { version, .. }
        | Event::DriftDetected { version, .. }
        | Event::HwndDriftDetected { version, .. }
        | Event::CorrectiveWindowMove { version, .. }
        | Event::HostShouldQuit { version, .. }
        | Event::Snapshot { version, .. }
        | Event::EventList { version, .. }
        | Event::SagaStarted { version, .. }
        | Event::SagaCompleted { version, .. }
        | Event::SagaFailed { version, .. }
        | Event::SrvSnapshot { version, .. }
        | Event::WorkspaceCreated { version, .. }
        | Event::WorkspaceDeleted { version, .. }
        | Event::TabCreated { version, .. }
        | Event::TabDeleted { version, .. }
        | Event::ActiveTabChanged { version, .. }
        | Event::TabReordered { version, .. }
        | Event::SrvWindowOpened { version, .. }
        | Event::SrvWindowClosed { version, .. }
        | Event::SrvWindowWorkspaceChanged { version, .. }
        | Event::TabsReorderedBulk { version, .. }
        | Event::WorkspaceRenamed { version, .. }
        | Event::TabRenamed { version, .. }
        | Event::WorkspaceMetaUpdated { version, .. }
        | Event::WindowMetaUpdated { version, .. }
        | Event::TabMetaUpdated { version, .. }
        | Event::BlockMetaUpdated { version, .. }
        | Event::TabMoved { version, .. }
        | Event::BlockMoved { version, .. }
        | Event::BlockCreated { version, .. }
        | Event::BlockDeleted { version, .. }
        | Event::FocusedNodeChanged { version, .. }
        | Event::MagnifiedNodeChanged { version, .. }
        | Event::SagaActionFailed { version, .. }
        | Event::Error { version, .. }
        // Phase E.4.B — layout tree events.
        | Event::LayoutNodeInserted { version, .. }
        | Event::LayoutNodeInsertedAtIndex { version, .. }
        | Event::LayoutNodeDeleted { version, .. }
        | Event::LayoutNodeMoved { version, .. }
        | Event::LayoutNodesSwapped { version, .. }
        | Event::LayoutNodesResized { version, .. }
        | Event::LayoutNodeReplaced { version, .. }
        | Event::LayoutSplitHorizontalApplied { version, .. }
        | Event::LayoutSplitVerticalApplied { version, .. }
        | Event::LayoutCleared { version, .. }
        | Event::LayoutBackendActionsQueued { version, .. }
        | Event::LayoutTreeReplaced { version, .. } => *version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{ClientKind, LifecyclePhase};

    fn lifecycle_event(v: u64) -> Event {
        Event::LifecyclePhaseChanged {
            from: LifecyclePhase::Starting,
            to: LifecyclePhase::Running,
            version: v,
        }
    }

    fn registered_event(v: u64) -> Event {
        Event::Registered {
            client_id: 1,
            launcher_pid: 1,
            launcher_version: "test".into(),
            version: v,
        }
    }

    fn process_spawned_event(v: u64) -> Event {
        Event::ProcessSpawned {
            pid: 100,
            kind: ClientKind::Host,
            client_version: "0.0.0".into(),
            version: v,
        }
    }

    #[test]
    fn append_grows_ring_until_cap_then_evicts_oldest() {
        let log = EventLog::new(None);
        for v in 1..=(MAX_RING_EVENTS as u64 + 5) {
            log.append(lifecycle_event(v));
        }
        let ring = log.ring.lock().unwrap();
        assert_eq!(ring.len(), MAX_RING_EVENTS);
        // Oldest 5 should have been evicted; first should be v=6.
        assert_eq!(event_version(ring.front().unwrap()), 6);
        // Newest is v = MAX_RING_EVENTS + 5.
        assert_eq!(
            event_version(ring.back().unwrap()),
            MAX_RING_EVENTS as u64 + 5
        );
    }

    #[test]
    fn events_since_returns_only_versions_strictly_greater() {
        let log = EventLog::new(None);
        for v in [1u64, 2, 3, 4, 5] {
            log.append(lifecycle_event(v));
        }
        let replay = log.events_since(2);
        assert_eq!(replay.len(), 3, "expected v=3,4,5; got {:?}", replay);
        assert_eq!(event_version(&replay[0]), 3);
        assert_eq!(event_version(&replay[2]), 5);
    }

    #[test]
    fn events_since_zero_returns_all() {
        let log = EventLog::new(None);
        for v in [1u64, 2, 3] {
            log.append(registered_event(v));
        }
        let replay = log.events_since(0);
        assert_eq!(replay.len(), 3);
    }

    #[test]
    fn events_since_at_or_above_max_returns_empty() {
        let log = EventLog::new(None);
        log.append(process_spawned_event(5));
        log.append(process_spawned_event(6));
        let replay = log.events_since(10);
        assert!(replay.is_empty());
    }

    #[test]
    fn replay_truncated_detects_missed_events() {
        let log = EventLog::new(None);
        // Fill past capacity to force eviction.
        for v in 1..=(MAX_RING_EVENTS as u64 + 100) {
            log.append(lifecycle_event(v));
        }
        // Subscriber asks for events since v=5; ring's oldest is now v=101.
        // The subscriber missed v=6..=100 — the replay slice covers v=101..,
        // and `replay_truncated(5)` reports the gap.
        assert!(log.replay_truncated(5));
        // Subscriber asking from a version newer than ring's oldest sees
        // no truncation.
        let oldest = MAX_RING_EVENTS as u64 + 100 - MAX_RING_EVENTS as u64 + 1;
        assert!(!log.replay_truncated(oldest));
    }

    #[test]
    fn replay_truncated_on_empty_log_is_false() {
        let log = EventLog::new(None);
        assert!(!log.replay_truncated(0));
        assert!(!log.replay_truncated(100));
    }

    /// The disk writer must route every warning through the injected
    /// sink rather than a hard-coded logger — that sink is the whole
    /// reason this module could be shared (see module doc). Exercise
    /// the one warning path that needs no live broadcast bus: an
    /// unopenable disk path.
    #[tokio::test]
    async fn disk_writer_reports_open_failure_through_the_injected_sink() {
        let captured = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink: WarnSink = {
            let c = captured.clone();
            Arc::new(move |m: &str| c.lock().unwrap().push(m.to_string()))
        };
        // A path under a file (not a directory) cannot be created.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let log = Arc::new(EventLog::new(Some(blocker.join("events.log"))));
        let (_tx, rx) = tokio::sync::broadcast::channel::<Event>(4);
        run_disk_writer(log, rx, sink).await;
        let msgs = captured.lock().unwrap();
        assert_eq!(msgs.len(), 1, "exactly one warning expected, got {:?}", *msgs);
        assert!(msgs[0].contains("cannot open"), "{}", msgs[0]);
    }
}
