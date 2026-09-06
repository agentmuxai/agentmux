// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Background-service audit log — issue #2977 Workstream 4,
//! `docs/specs/SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §6.
//!
//! ## What this is for
//!
//! WS4 requires "**immutable** audit logging of what the background service
//! did while unattended, surfaced the next time a window (or the tray panel)
//! opens — the direct answer to 'there was no one watching.'" The point is
//! accountability, not diagnostics: once AgentMux can keep running with no
//! window, the user loses the ability to see what it did, and §6 cites real
//! precedents (Zoom 2019, Recall 2024) where a background component acting
//! unobserved *was* the problem.
//!
//! ## Append-only on disk, and why the first cut was wrong
//!
//! An earlier revision of this module kept the log in memory and **erased**
//! entries when they were read. Review (#3001) caught that this contradicts
//! the spec twice over:
//!
//! - **Not immutable.** Consuming on read means the record cannot be
//!   re-examined, which is the opposite of an audit log.
//! - **Lost exactly when it matters.** The launcher deliberately restarts a
//!   crashed host while keeping the background service alive, so an
//!   in-memory log is wiped precisely in the failure scenario where
//!   accountability matters most.
//!
//! So the log is now an append-only JSONL file. Surfacing advances a
//! watermark instead of deleting: entries stay on disk, and "what you
//! missed" means "entries past the watermark".
//!
//! ## Ordering, and why the hot path takes no lock
//!
//! Two things must both hold, and earlier revisions of this module kept
//! trading one for the other:
//!
//! 1. **Ordering.** The transition must be applied in the order the reducer
//!    decided it. Out of order, the redundancy guard swallows an `Observed`
//!    and the log is left claiming an unattended period that had ended.
//! 2. **No stalling the UI thread.** `host_dispatch` is called straight from
//!    CEF UI-thread callbacks — `wrr::win_event` dispatches `UnregisterBrowser`
//!    there, which is exactly the transition recorded here — and its contract
//!    promises no I/O and sub-microsecond hold time
//!    (SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md §6).
//!
//! Writing the file under `host_state` satisfied (1) and broke (2). Moving
//! the write to a thread but reaching it through this struct's mutex still
//! broke (2), because *acquiring* that mutex can block behind
//! `background_audit_take` holding it across a full file read.
//!
//! The current split satisfies both structurally:
//!
//! - `HostState::background_unattended` holds the state, so the reducer
//!   decides the transition AND applies it under the lock it already holds.
//! - The writer's `Sender` lives on `AppState` OUTSIDE this struct's mutex
//!   (`AppState::background_audit_tx`), so `host_dispatch` enqueues with a
//!   `OnceLock` read and an unbounded `send` — no I/O, no lock acquisition.
//! - A dedicated writer thread performs every append and compaction, under a
//!   file lock it shares with the reader.
//!
//! So `host_dispatch` holds `host_state` and nothing else. **Do not
//! reintroduce file I/O, or any second lock, on that path.**
//!
//! ## State survives a host restart
//!
//! `HostState::background_unattended` is seeded from the **last entry in the
//! log** at `init` time. The launcher deliberately restarts a crashed host
//! while the background service keeps running, so a period that began before
//! the crash is still open and its eventual `Observed` must still be
//! recorded rather than suppressed as redundant.
//!
//! ## Scope, stated honestly
//!
//! This records the **lifecycle of unattended periods** — when the instance
//! stopped being observed and when it was observed again. It does **not** yet
//! enumerate the individual agent turns, commands, or tool executions that
//! ran during the window; that data lives in `srv` and needs a host→srv query
//! that does not exist. Review (#3001) is right that this alone does not
//! fully satisfy §6's "what the background service did", so the WS4 checkbox
//! stays open. The entry format is a flat `{ at_ms, kind }` specifically so
//! richer kinds extend it without reshaping the surfacing contract.

use std::io::Write;
use std::path::PathBuf;

/// One thing the background service did (or had happen to it) while the user
/// had no window open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Milliseconds since the Unix epoch. A number, not a formatted string,
    /// so the frontend can localize it.
    pub at_ms: u64,
    pub kind: AuditKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    /// The last user window closed and the instance kept running instead of
    /// exiting — the moment it became unattended.
    WentUnattended,
    /// A window opened again, ending an unattended period.
    Observed,
}

impl AuditKind {
    /// Stable wire/disk identifier. Explicit rather than derived so renaming
    /// a variant cannot silently invalidate previously written log files or
    /// change what the frontend matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            AuditKind::WentUnattended => "went_unattended",
            AuditKind::Observed => "observed",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "went_unattended" => Some(AuditKind::WentUnattended),
            "observed" => Some(AuditKind::Observed),
            _ => None,
        }
    }
}

/// Hard cap on retained lines. At two lines per unattended period this is
/// ~1000 periods; a background instance running for months must not grow the
/// file without bound.
const MAX_LINES: usize = 2000;

/// How many lines survive a compaction. Keeping only the most recent half
/// means compaction is amortised (one rewrite per `MAX_LINES / 2` appends)
/// rather than happening on every append once the cap is reached.
const KEEP_LINES: usize = MAX_LINES / 2;

/// The audit log.
///
/// Records are on disk (so they survive a host restart), but the **decision**
/// to record is made from a small in-memory flag and the write is handed to a
/// background thread. That split is what lets the transition be decided under
/// the caller's lock — which is required for ordering — without doing any I/O
/// there. See `record`.
#[derive(Debug, Default)]
pub struct BackgroundAudit {
    /// `None` until `init` runs — the log then degrades to a no-op rather
    /// than failing anything. An audit log that cannot be written must never
    /// take down the app it is auditing.
    path: Option<PathBuf>,
    /// Guards every touch of the log and watermark FILES, shared with the
    /// writer thread.
    ///
    /// `Mutex<BackgroundAudit>` is not enough on its own: it serializes
    /// callers of these methods, but the writer thread mutates the same two
    /// files completely outside it. Without this, `take_unsurfaced` could
    /// snapshot the file, have the writer compact underneath it, and write
    /// back a watermark computed from the stale pre-compaction length —
    /// pushing it past the end of the now-shorter file and silently
    /// un-surfacing everything after.
    io: Option<std::sync::Arc<std::sync::Mutex<()>>>,
}

impl BackgroundAudit {
    /// Point the log at this instance's data directory and start its writer.
    ///
    /// Returns `(sender, seeded_unattended)` on first call: the sender is
    /// stored **outside** this mutex (see `AppState::background_audit_tx`) so
    /// the hot path can enqueue an entry without ever acquiring this lock —
    /// which matters because `background_audit_take` holds it across real disk
    /// I/O, and `host_dispatch` enqueues while holding `host_state`. Taking
    /// this mutex there would let a concurrent surfacing call stall the UI
    /// thread through lock contention, which is the same hazard as doing the
    /// I/O inline (ReAgent P1 on #3001).
    ///
    /// Uses the **dev-aware** data dir (the value stored in
    /// `AppState::version_data_dir`), NOT raw `AGENTMUX_DATA_DIR`: a
    /// `task dev` host launched from inside a parent AgentMux pane inherits
    /// the parent's env unscrubbed, and would otherwise write its records
    /// into the parent instance's log.
    ///
    /// **Idempotent.** This is a repeat-call path: `sidecar` calls it from
    /// both `use_launcher_endpoints` and `spawn_backend`, and `spawn_backend`
    /// is re-invoked by the `restart_backend` IPC and by crash auto-restart.
    /// Re-initialising would spawn a SECOND writer thread with its own `io`
    /// mutex over the same files. Returns `None` when already initialised.
    pub fn init(
        &mut self,
        dir: &std::path::Path,
    ) -> Option<(std::sync::mpsc::Sender<AuditEntry>, bool)> {
        let path = dir.join("background-audit.jsonl");
        if let Some(existing) = &self.path {
            if existing != &path {
                tracing::warn!(
                    target: "wrr",
                    old = %existing.display(),
                    new = %path.display(),
                    "[audit] ignoring a data-dir change — keeping the existing log"
                );
            }
            return None;
        }

        self.path = Some(path.clone());
        let io = std::sync::Arc::new(std::sync::Mutex::new(()));
        self.io = Some(io.clone());

        // Seed from the log so an unattended period that began before a host
        // crash is still correctly open. One read, at startup.
        let seeded = matches!(
            self.entries().last().map(|e| e.kind),
            Some(AuditKind::WentUnattended)
        );

        let (tx, rx) = std::sync::mpsc::channel::<AuditEntry>();
        let watermark = path.with_extension("surfaced");
        std::thread::Builder::new()
            .name("agentmux-audit-writer".into())
            .spawn(move || {
                // Ends when every sender is dropped (process exit).
                while let Ok(entry) = rx.recv() {
                    let _guard = io.lock().unwrap_or_else(|e| e.into_inner());
                    write_entry(&path, &watermark, &entry);
                }
            })
            .ok();

        Some((tx, seeded))
    }

    fn watermark_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.with_extension("surfaced"))
    }

    /// Every entry on disk, oldest first.
    ///
    /// Malformed lines are skipped rather than failing the read: a torn final
    /// line (a crash mid-append) must not cost the entries before it.
    pub fn entries(&self) -> Vec<AuditEntry> {
        let _guard = self.io.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()));
        self.entries_unlocked()
    }

    /// `entries` without taking the file lock — for callers that already hold
    /// it. Split out because `take_unsurfaced` must read the file and update
    /// the watermark under ONE hold; calling the locking version there would
    /// deadlock on the non-reentrant mutex.
    fn entries_unlocked(&self) -> Vec<AuditEntry> {
        let Some(path) = &self.path else { return Vec::new() };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines().filter_map(parse_line).collect()
    }

    /// Entries the user has not been shown yet, advancing the watermark.
    ///
    /// Non-destructive: the log keeps everything. "Surfaced" is a position,
    /// not a deletion — which is what makes this an audit log rather than a
    /// queue.
    pub fn take_unsurfaced(&self) -> Vec<AuditEntry> {
        // ONE hold across read-then-write. Splitting them let the writer
        // thread compact in between, after which the watermark computed from
        // the stale length could exceed the file and silently un-surface
        // everything thereafter.
        let _guard = self.io.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()));
        let all = self.entries_unlocked();
        let already = self.read_watermark();
        if already >= all.len() {
            return Vec::new();
        }
        let fresh = all[already..].to_vec();
        self.write_watermark(all.len());
        fresh
    }

    fn read_watermark(&self) -> usize {
        self.watermark_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Written via temp + rename like every other file write in this module.
    /// A torn watermark parses as 0 and re-shows already-surfaced entries —
    /// the safe direction, but still wrong, and inconsistent with the
    /// crash-safety discipline applied elsewhere here (ReAgent P2 on #3001).
    fn write_watermark(&self, n: usize) {
        if let Some(p) = self.watermark_path() {
            write_atomic(&p, &n.to_string());
        }
    }
}

/// Append one entry, compacting first if the file has grown past the cap.
///
/// Runs on the writer thread only, so all of this I/O is off every lock the
/// reducer touches.
fn write_entry(path: &std::path::Path, watermark: &std::path::Path, entry: &AuditEntry) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    compact_if_oversized(path, watermark);
    let line = entry_line(entry);
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()) {
                tracing::warn!(target: "wrr", error = %e, "[audit] append failed");
            }
        }
        Err(e) => tracing::warn!(target: "wrr", error = %e, "[audit] open failed"),
    }
}

/// One log line, including its trailing newline. Shared by the append path
/// and by compaction so the on-disk format has exactly one definition.
fn entry_line(entry: &AuditEntry) -> String {
    format!(
        "{{\"at_ms\":{},\"kind\":\"{}\"}}\n",
        entry.at_ms,
        entry.kind.as_str()
    )
}

/// Bound the log by dropping the OLDEST entries, in place.
///
/// Deliberately a single file rather than rotating to a `.jsonl.1` archive.
/// Rotation via `rename` replaces the archive wholesale, so the SECOND
/// rotation silently discarded everything the first one had archived.
/// One file with an explicit retention rule has no second copy to clobber.
///
/// **Everything here is counted in PARSED ENTRIES, never raw lines.** The
/// watermark is written by `take_unsurfaced` in units of valid entries
/// (`entries_unlocked` filters unparseable lines out), so measuring the drop
/// in raw lines made the two diverge by the number of malformed lines
/// involved — silently re-surfacing an already-shown entry, or skipping one
/// permanently, depending on the values. A torn line from a mid-append crash
/// is a case this module explicitly supports, so that divergence was
/// reachable in normal operation (ReAgent P1 on #3001).
///
/// Compaction also REWRITES from parsed entries, so it heals any torn line
/// rather than leaving it to sit in the file until it happens to fall inside
/// a future dropped range. After a compaction, raw lines and valid entries
/// are identical again by construction — at rest. A reader racing an
/// in-flight append can still observe a torn final line; appends are not
/// atomic, and tolerating that is the point of `parse_line` skipping
/// junk. The healing guarantee is about what compaction LEAVES behind,
/// not an invariant that holds at every instant.
///
/// Dropping entries the user has never seen is possible only if they have not
/// opened a window in `KEEP_LINES` transitions; that is logged loudly rather
/// than passing silently.
fn compact_if_oversized(path: &std::path::Path, watermark: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    // Trigger on RAW size, because that is what actually bounds the file on
    // disk — a file full of unparseable junk still needs compacting.
    if text.lines().count() < MAX_LINES {
        return;
    }

    // ...but measure and retain in PARSED ENTRIES, matching the watermark.
    let entries: Vec<AuditEntry> = text.lines().filter_map(parse_line).collect();
    let dropped = entries.len().saturating_sub(KEEP_LINES);
    let surfaced: usize = std::fs::read_to_string(watermark)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let unsurfaced_dropped = dropped.saturating_sub(surfaced);

    let kept: String = entries[dropped..].iter().map(entry_line).collect();
    // Temp + rename so a crash mid-compaction leaves either the old log or
    // the new one, never a half-written one.
    let tmp = path.with_extension("jsonl.compacting");
    if std::fs::write(&tmp, kept).is_ok() && std::fs::rename(&tmp, path).is_ok() {
        write_atomic(watermark, &surfaced.saturating_sub(dropped).to_string());
        if unsurfaced_dropped > 0 {
            tracing::warn!(
                target: "wrr",
                unsurfaced_dropped,
                "[audit] compaction dropped entries the user was never shown \
                 (no window opened in a very long time)"
            );
        } else {
            tracing::info!(target: "wrr", dropped, "[audit] compacted");
        }
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Write `contents` to `path` via a temp file + rename, so a crash leaves
/// either the old value or the new one and never a half-written one.
fn write_atomic(path: &std::path::Path, contents: &str) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, contents).is_ok() {
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn parse_line(line: &str) -> Option<AuditEntry> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(AuditEntry {
        at_ms: v.get("at_ms")?.as_u64()?,
        kind: AuditKind::from_str(v.get("kind")?.as_str()?)?,
    })
}

/// Wall-clock milliseconds since the Unix epoch, or 0 if the clock predates
/// it (a badly misconfigured machine — a visibly odd timestamp beats a panic).
pub fn now_ms() -> u64 {
    agentmux_common::time::now_ms_u64()
}

/// Initialise the audit log for this instance and publish its plumbing.
///
/// Idempotent (see `BackgroundAudit::init`). Does three things in one place so
/// they cannot drift apart: starts the writer, stores the sender where the hot
/// path can reach it without a lock, and seeds `HostState::background_unattended`
/// so a period that began before a host crash is still correctly open.
pub fn init_for_state(state: &std::sync::Arc<crate::state::AppState>, dir: &std::path::Path) {
    let Some((tx, seeded)) = state.background_audit.lock().init(dir) else {
        return; // already initialised
    };
    let _ = state.background_audit_tx.set(tx);
    state.host_state.lock().background_unattended = seeded;
    if seeded {
        tracing::info!(
            target: "wrr",
            "[audit] resuming an unattended period that began before this host started"
        );
    }
}

/// IPC handler: hand the frontend everything recorded since it last looked.
///
/// This is WS4's "surfaced next time a window/panel opens" half. An empty
/// `entries` array means there is nothing to tell the user, which is the
/// common case.
pub fn background_audit_take(
    state: &std::sync::Arc<crate::state::AppState>,
) -> Result<serde_json::Value, String> {
    // Read the live flag from the reducer state — brief, no I/O — then do the
    // file work under the audit mutex. Deliberately NOT the other way round:
    // `host_state` must never be held while this touches the disk.
    let unattended = state.host_state.lock().background_unattended;
    let entries = state.background_audit.lock().take_unsurfaced();

    if !entries.is_empty() {
        tracing::info!(
            target: "wrr",
            count = entries.len(),
            "[audit] surfacing background-service activity to a newly opened window"
        );
    }
    Ok(serde_json::json!({
        "entries": entries
            .iter()
            .map(|e| serde_json::json!({ "at_ms": e.at_ms, "kind": e.kind.as_str() }))
            .collect::<Vec<_>>(),
        "unattended": unattended,
    }))
}

#[cfg(test)]
mod background_audit_tests {
    use super::*;

    /// A log plus the sender the hot path would hold, and the
    /// attended/unattended flag the reducer would own — i.e. the same three
    /// pieces `init_for_state` wires together, kept local so the file
    /// behaviour is testable without an `AppState`.
    struct Harness {
        _dir: tempfile::TempDir,
        log: BackgroundAudit,
        tx: std::sync::mpsc::Sender<AuditEntry>,
        unattended: bool,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut log = BackgroundAudit::default();
            let (tx, seeded) = log.init(dir.path()).expect("first init");
            Self { _dir: dir, log, tx, unattended: seeded }
        }

        /// Mirrors what `reducer::update` + `host_dispatch` do together:
        /// decide against the current flag, update it, enqueue the entry.
        fn record(&mut self, now_unattended: bool, at_ms: u64) {
            let transition = crate::reducer::background_attention_transition_for_test(
                true,
                self.unattended,
                if now_unattended { 0 } else { 1 },
            );
            if let Some(v) = transition {
                self.unattended = v;
                let _ = self.tx.send(AuditEntry {
                    at_ms,
                    kind: if v { AuditKind::WentUnattended } else { AuditKind::Observed },
                });
            }
        }

        /// Writes go through a background thread, so wait rather than assume.
        /// Bounded so a genuine failure fails instead of hanging.
        fn wait_for(&self, want: usize) -> Vec<AuditEntry> {
            for _ in 0..300 {
                let e = self.log.entries();
                if e.len() >= want {
                    return e;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            self.log.entries()
        }
    }

    #[test]
    fn records_an_unattended_period_as_a_pair() {
        let mut h = Harness::new();
        h.record(true, 1_000);
        h.record(false, 5_000);
        let e = h.wait_for(2);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], AuditEntry { at_ms: 1_000, kind: AuditKind::WentUnattended });
        assert_eq!(e[1], AuditEntry { at_ms: 5_000, kind: AuditKind::Observed });
    }

    #[test]
    fn a_single_unattended_period_is_recorded_once() {
        // Pool churn re-reports a zero-window state; the user should see one
        // period, not a burst.
        let mut h = Harness::new();
        h.record(true, 1_000);
        h.record(true, 1_100);
        h.record(true, 1_200);
        assert_eq!(h.wait_for(1).len(), 1);
    }

    #[test]
    fn opening_a_window_during_normal_use_records_nothing() {
        let mut h = Harness::new();
        h.record(false, 1_000);
        h.record(false, 2_000);
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(h.log.entries().is_empty());
    }

    #[test]
    fn surfacing_is_non_destructive_and_advances_a_watermark() {
        let mut h = Harness::new();
        h.record(true, 1);
        h.record(false, 2);
        h.wait_for(2);
        assert_eq!(h.log.take_unsurfaced().len(), 2);
        assert!(
            h.log.take_unsurfaced().is_empty(),
            "a second window must not be re-shown what was already surfaced"
        );
        assert_eq!(
            h.log.entries().len(),
            2,
            "entries must remain on disk — surfacing is a position, not a deletion"
        );
    }

    #[test]
    fn only_entries_added_since_the_last_surfacing_are_returned() {
        let mut h = Harness::new();
        h.record(true, 1);
        h.record(false, 2);
        h.wait_for(2);
        let _ = h.log.take_unsurfaced();
        h.record(true, 3);
        h.wait_for(3);
        let fresh = h.log.take_unsurfaced();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].at_ms, 3);
    }

    /// The launcher restarts a crashed host while the background service stays
    /// alive. `init` must seed the flag from the log, or the eventual
    /// `Observed` is suppressed as redundant and the user never learns the
    /// period ended.
    #[test]
    fn unattended_state_is_seeded_from_the_log_after_a_host_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut before = BackgroundAudit::default();
        let (tx, seeded) = before.init(dir.path()).unwrap();
        assert!(!seeded, "a fresh log starts attended");
        tx.send(AuditEntry { at_ms: 1_000, kind: AuditKind::WentUnattended }).unwrap();
        for _ in 0..300 {
            if !before.entries().is_empty() { break; }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(before); // host crashes

        let mut after = BackgroundAudit::default();
        let (_tx2, seeded_after) = after.init(dir.path()).unwrap();
        assert!(seeded_after, "the open period must be recovered from the log");
    }

    /// `init` is re-invoked on every backend restart (`restart_backend` IPC,
    /// crash auto-restart). It must not stand up a second writer thread with
    /// its own lock over the same files.
    #[test]
    fn re_initialising_on_backend_restart_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = BackgroundAudit::default();
        assert!(log.init(dir.path()).is_some(), "first init sets up");
        assert!(
            log.init(dir.path()).is_none(),
            "a second init must not create another writer"
        );
    }

    #[test]
    fn a_torn_final_line_does_not_make_the_log_unreadable() {
        // A crash mid-append can leave a partial line. Skipping it must not
        // cost the entries written before it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("background-audit.jsonl"),
            "{\"at_ms\":1,\"kind\":\"went_unattended\"}\n{\"at_ms\":2,\"ki",
        )
        .unwrap();
        let mut log = BackgroundAudit::default();
        let (_tx, seeded) = log.init(dir.path()).unwrap();
        assert_eq!(log.entries().len(), 1);
        assert!(seeded, "seeded from the last VALID entry");
    }

    #[test]
    fn an_uninitialised_log_degrades_to_a_no_op_rather_than_failing() {
        // An audit log that cannot be written must never take down the app it
        // audits.
        let log = BackgroundAudit::default();
        assert!(log.entries().is_empty());
        assert!(log.take_unsurfaced().is_empty());
    }

    /// Below the cap, nothing is ever lost — including the very oldest entry.
    /// That is the guarantee that actually holds: with a hard bound, if more
    /// than `KEEP_LINES` entries are unsurfaced, bounding the file necessarily
    /// drops some. The contract is "drop oldest-first, and say so loudly".
    #[test]
    fn nothing_is_lost_below_the_compaction_threshold() {
        let mut h = Harness::new();
        let total = MAX_LINES - 10;
        for i in 0..total {
            h.record(i % 2 == 0, i as u64);
        }
        let e = h.wait_for(total);
        assert_eq!(e.len(), total, "no compaction should have run");
        let surfaced = h.log.take_unsurfaced();
        assert_eq!(surfaced.len(), total);
        assert_eq!(surfaced[0].at_ms, 0, "the oldest entry must survive");
    }

    /// The case an earlier fix missed: enough entries to compact MORE THAN
    /// ONCE. The old two-file rotation lost everything archived by the first
    /// rotation as soon as the second ran.
    #[test]
    fn repeated_compaction_keeps_the_log_bounded_and_ordered() {
        let mut h = Harness::new();
        for i in 0..(MAX_LINES * 3) {
            h.record(i % 2 == 0, i as u64);
        }
        h.wait_for(KEEP_LINES);
        let e = h.log.entries();
        assert!(e.len() <= MAX_LINES, "must stay bounded, got {}", e.len());
        assert!(!e.is_empty(), "repeated compaction must not empty the log");
        let ts: Vec<u64> = e.iter().map(|x| x.at_ms).collect();
        assert!(
            ts.windows(2).all(|w| w[0] < w[1]),
            "retained entries must stay ordered, not half-clobbered"
        );
    }

    /// The writer thread and `take_unsurfaced` both mutate the log and the
    /// watermark. Without the shared file lock, a compaction landing between
    /// the read and the watermark write left the watermark past the end of the
    /// shorter file — after which nothing was ever surfaced again.
    #[test]
    fn surfacing_still_works_after_racing_the_compacting_writer() {
        let mut h = Harness::new();
        for i in 0..(MAX_LINES * 2) {
            h.record(i % 2 == 0, i as u64);
            if i % 25 == 0 {
                let _ = h.log.take_unsurfaced();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = h.log.take_unsurfaced();

        let flip = !h.unattended;
        h.record(flip, 9_999_999);
        for _ in 0..300 {
            if !h.log.take_unsurfaced().is_empty() {
                return; // invariant holds
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("watermark left the log permanently un-surfacable");
    }

    /// A torn line (mid-append crash) coexisting with a compaction cycle.
    ///
    /// The watermark counts PARSED entries; compaction used to count RAW
    /// lines. With an unparseable line in the file the two diverge by exactly
    /// the number of malformed lines, and the post-compaction watermark lands
    /// short — re-showing entries the user has already been shown (ReAgent P1
    /// on #3001). Every other compaction test writes only well-formed
    /// entries, so none of them could reach this.
    ///
    /// Drives `compact_if_oversized` DIRECTLY rather than through the writer
    /// thread: this is a pure file function, and the arithmetic under test is
    /// exact. An earlier version of this test pushed 2050 entries through the
    /// channel and polled for quiescence, which was both flaky (it compared
    /// two reads the writer could append between — windows-latest caught it)
    /// and needlessly slow, since compaction re-reads the whole file on every
    /// append. Writer/reader concurrency is already covered by
    /// `surfacing_still_works_after_racing_the_compacting_writer`.
    #[test]
    fn a_torn_line_does_not_skew_the_watermark_across_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("background-audit.jsonl");
        let watermark = log_path.with_extension("surfaced");

        // MAX_LINES valid entries plus one torn line => raw lines exceed the
        // cap, and raw count is exactly one MORE than the valid-entry count.
        const VALID: usize = MAX_LINES;
        let mut seed = String::new();
        for i in 0..VALID {
            if i == VALID / 2 {
                seed.push_str("{\"at_ms\":999999,\"ki
"); // torn write
            }
            seed.push_str(&entry_line(&AuditEntry {
                at_ms: i as u64,
                kind: if i % 2 == 0 { AuditKind::WentUnattended } else { AuditKind::Observed },
            }));
        }
        std::fs::write(&log_path, &seed).unwrap();
        assert_eq!(seed.lines().count(), VALID + 1, "one unparseable line present");

        // The user has already been shown the first 1500 VALID entries.
        const SURFACED: usize = 1_500;
        std::fs::write(&watermark, SURFACED.to_string()).unwrap();

        compact_if_oversized(&log_path, &watermark);

        // Dropped in parsed-entry units: VALID - KEEP_LINES. Counting raw
        // lines instead would drop one more and pull the watermark one short.
        let dropped = VALID - KEEP_LINES;
        let after: usize = std::fs::read_to_string(&watermark).unwrap().trim().parse().unwrap();
        assert_eq!(
            after,
            SURFACED - dropped,
            "watermark must be shifted in the same units it was written in"
        );

        // The file is healed: every remaining line parses, so raw and parsed
        // counts cannot drift apart again.
        let raw = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(raw.lines().count(), KEEP_LINES);
        assert_eq!(raw.lines().filter_map(parse_line).count(), KEEP_LINES);

        // The observable consequence: surfacing returns exactly the entries
        // never shown before, starting at the first one. Off-by-the-torn-line
        // would re-show at_ms 1499.
        let mut log = BackgroundAudit::default();
        let (_tx, _seeded) = log.init(dir.path()).unwrap();
        let fresh = log.take_unsurfaced();
        assert_eq!(fresh.len(), VALID - SURFACED);
        assert_eq!(
            fresh[0].at_ms, SURFACED as u64,
            "an already-surfaced entry was shown again"
        );
    }

    #[test]
    fn wire_identifiers_are_stable() {
        // These strings are both the on-disk format and what the frontend
        // matches on — a variant rename must not change them.
        assert_eq!(AuditKind::WentUnattended.as_str(), "went_unattended");
        assert_eq!(AuditKind::Observed.as_str(), "observed");
        assert_eq!(AuditKind::from_str("observed"), Some(AuditKind::Observed));
        assert_eq!(AuditKind::from_str("nonsense"), None);
    }
}
