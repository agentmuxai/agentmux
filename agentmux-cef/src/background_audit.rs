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
//! ## Ordering
//!
//! Writes are performed by `AppState::host_dispatch` **while it still holds
//! the `host_state` lock**, so audit writes are serialized in the same order
//! as the reducer decisions that produced them. Doing the write after
//! releasing the lock (the first cut again) let two concurrent dispatches
//! apply out of order — and because `observed` no-ops when the log is not in
//! an unattended state, a reordered pair could silently drop an `Observed`
//! and leave the log stuck claiming an unattended period that had ended.
//!
//! Holding the lock across a small file append is acceptable *here
//! specifically* because these events only occur on an attended/unattended
//! transition — a handful of times in a session, not per dispatch. Paying a
//! rare, bounded lock-hold to remove a correctness hole is the right trade;
//! this is not a precedent for doing I/O under `host_state` generally.
//!
//! ## State is derived from the log, not held in memory
//!
//! Whether the instance is currently unattended is read from the **last
//! recorded entry**, not a field. That is what makes the state survive a host
//! restart: a host that crashed mid-unattended-period restarts with no
//! memory, but the log still ends in `WentUnattended`, so the eventual
//! `Observed` is still recorded and the user still learns the period existed.
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
    /// `None` when no data dir has been resolved yet — the log then degrades
    /// to a no-op rather than failing anything. An audit log that cannot be
    /// written must never take down the app it is auditing.
    path: Option<PathBuf>,
    /// Whether the instance is currently in an unattended period.
    ///
    /// Held in memory so `record` can decide without touching the disk, and
    /// seeded from the log in `set_dir` so it is still correct after a host
    /// restart (the launcher restarts a crashed host while the background
    /// service keeps running).
    unattended: bool,
    /// Hands entries to the writer thread. Sending is non-blocking and
    /// preserves order, so the file ends up in the same order the decisions
    /// were made.
    tx: Option<std::sync::mpsc::Sender<AuditEntry>>,
    /// Guards every touch of the log and watermark FILES.
    ///
    /// `Mutex<BackgroundAudit>` is not enough: it serializes callers of these
    /// methods, but the writer thread mutates the same two files completely
    /// outside it. Without this, `take_unsurfaced` could snapshot the file,
    /// have the writer compact underneath it, and then write back a watermark
    /// computed from the stale pre-compaction length — pushing it past the end
    /// of the now-shorter file and silently un-surfacing everything after
    /// (ReAgent P1 on #3001).
    ///
    /// Held only around file I/O, never while `record` runs, so nothing here
    /// can reach back into `host_state`.
    io: Option<std::sync::Arc<std::sync::Mutex<()>>>,
}

impl BackgroundAudit {
    /// Point the log at this instance's data directory.
    ///
    /// Called once startup has resolved the **dev-aware** data dir (the same
    /// value stored in `AppState::version_data_dir`), NOT from the raw
    /// `AGENTMUX_DATA_DIR` env var. That distinction is load-bearing: a
    /// `task dev` host launched from inside a parent AgentMux pane inherits
    /// the parent's `AGENTMUX_DATA_DIR` unscrubbed, which is exactly why
    /// `lib.rs` resolves the real dir through
    /// `is_dev_build_exe`/`resolve_path_only` instead. Reading the env
    /// directly here would have written a dev instance's unattended-period
    /// records into the parent instance's log, mixing two instances'
    /// histories (ReAgent P1 on PR #3001).
    ///
    /// Until this is called the log is a no-op, which is harmless: no window
    /// has opened or closed yet, so there is no transition to record.
    pub fn set_dir(&mut self, dir: &std::path::Path) {
        let path = dir.join("background-audit.jsonl");

        // IDEMPOTENT. This is a repeat-call path, not a one-shot: `sidecar`
        // calls it from both `use_launcher_endpoints` and `spawn_backend`, and
        // `spawn_backend` is re-invoked by the user-facing `restart_backend`
        // IPC and by crash auto-restart. Re-initialising would spawn a SECOND
        // writer thread holding a brand-new `io` mutex while the old thread
        // kept draining its queue under the old one — two writers on the same
        // files under unrelated locks, reopening the very race the file lock
        // exists to close (ReAgent P1 on #3001).
        //
        // The host process is not restarting in that scenario, so the
        // in-memory `unattended` flag is still valid and must NOT be reseeded.
        if let Some(existing) = &self.path {
            if existing != &path {
                // A data dir that changes mid-process is not a thing that
                // happens; keeping the existing writer is strictly safer than
                // running two.
                tracing::warn!(
                    target: "wrr",
                    old = %existing.display(),
                    new = %path.display(),
                    "[audit] ignoring a data-dir change — keeping the existing log"
                );
            }
            return;
        }

        self.path = Some(path.clone());
        let io = std::sync::Arc::new(std::sync::Mutex::new(()));
        self.io = Some(io.clone());
        // Seed from the log so an unattended period that began before a host
        // crash is still correctly open — one read, at startup, off the hot
        // path.
        self.unattended = matches!(
            self.entries().last().map(|e| e.kind),
            Some(AuditKind::WentUnattended)
        );

        // Writer thread. Entries are appended here, never by the caller, so
        // no lock the reducer touches is ever held across file I/O.
        let (tx, rx) = std::sync::mpsc::channel::<AuditEntry>();
        self.tx = Some(tx);
        let watermark = path.with_extension("surfaced");
        std::thread::Builder::new()
            .name("agentmux-audit-writer".into())
            .spawn(move || {
                // Ends when the sender is dropped (process exit).
                while let Ok(entry) = rx.recv() {
                    let _guard = io.lock().unwrap_or_else(|e| e.into_inner());
                    write_entry(&path, &watermark, &entry);
                }
            })
            .ok();
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

    /// Is the instance currently in an unattended period?
    ///
    /// Reads the in-memory flag, NOT the disk — this is called from
    /// `record`, which runs under the caller's `host_state` lock.
    pub fn is_unattended(&self) -> bool {
        self.unattended
    }

    /// Record an attended/unattended transition.
    ///
    /// `went_unattended == true` means the last window closed; `false` means a
    /// window opened. No-ops when it would be redundant (a second
    /// "unattended" for one period — the repeated zero-window dispatches pool
    /// churn produces — or an "observed" during ordinary use).
    ///
    /// **Called under `host_state`, and therefore does NO I/O.** The decision
    /// has to be serialized with the reducer that produced it, or two
    /// concurrent dispatches can apply out of order and the no-op guards
    /// silently drop an `Observed`, leaving the log stuck claiming a period
    /// that had ended. But `host_dispatch` is invoked straight from CEF
    /// UI-thread callbacks — `wrr::win_event` dispatches `UnregisterBrowser`
    /// there, which is exactly the transition that fires this — and its
    /// contract promises no I/O and sub-microsecond hold time
    /// (SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md §6). A file write there would
    /// stall the UI thread and every other concurrent dispatcher for however
    /// long the disk (or an AV filter) takes (ReAgent P1 on PR #3001).
    ///
    /// So this only flips an in-memory flag and hands the entry to the writer
    /// thread. Ordering is preserved because the channel is FIFO and the send
    /// happens under the same lock as the decision.
    pub fn record(&mut self, went_unattended: bool, at_ms: u64) {
        if went_unattended == self.unattended {
            return; // redundant — see above
        }
        self.unattended = went_unattended;
        let kind = if went_unattended {
            AuditKind::WentUnattended
        } else {
            AuditKind::Observed
        };
        if let Some(tx) = &self.tx {
            // Unbounded channel: send cannot block. A closed channel (writer
            // thread gone) is ignored — losing an informational record must
            // never propagate into the reducer.
            let _ = tx.send(AuditEntry { at_ms, kind });
        }
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
    let line = format!(
        "{{\"at_ms\":{},\"kind\":\"{}\"}}\n",
        entry.at_ms,
        entry.kind.as_str()
    );
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()) {
                tracing::warn!(target: "wrr", error = %e, "[audit] append failed");
            }
        }
        Err(e) => tracing::warn!(target: "wrr", error = %e, "[audit] open failed"),
    }
}

/// Bound the log by dropping the OLDEST entries, in place.
///
/// Deliberately a single file rather than rotating to a `.jsonl.1` archive.
/// Rotation via `rename` replaces the archive wholesale, so the SECOND
/// rotation silently discarded everything the first one had archived —
/// including entries the watermark had never advanced past. That is the same
/// under-reporting bug as before, just deferred to `2 * MAX_LINES` entries,
/// and the first-rotation-only test did not reach it (ReAgent P1 on #3001).
/// One file with an explicit retention rule has no second copy to clobber.
///
/// The watermark is shifted down by exactly the number of lines dropped so
/// surfacing positions stay aligned. Dropping entries the user has never been
/// shown is possible only if they have not opened a window in `KEEP_LINES`
/// transitions; that is logged loudly rather than passing silently, because
/// silent loss is the failure mode this whole function keeps getting wrong.
fn compact_if_oversized(path: &std::path::Path, watermark: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < MAX_LINES {
        return;
    }

    let dropped = lines.len().saturating_sub(KEEP_LINES);
    let surfaced: usize = std::fs::read_to_string(watermark)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let unsurfaced_dropped = dropped.saturating_sub(surfaced);

    let kept = lines[dropped..].join("\n");
    // Write via a temp file + rename so a crash mid-compaction leaves either
    // the old log or the new one, never a half-written one.
    let tmp = path.with_extension("jsonl.compacting");
    if std::fs::write(&tmp, format!("{}\n", kept)).is_ok() && std::fs::rename(&tmp, path).is_ok() {
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// IPC handler: hand the frontend everything recorded since it last looked.
///
/// This is WS4's "surfaced next time a window/panel opens" half. An empty
/// `entries` array means there is nothing to tell the user, which is the
/// common case.
pub fn background_audit_take(
    state: &std::sync::Arc<crate::state::AppState>,
) -> Result<serde_json::Value, String> {
    let audit = state.background_audit.lock();
    let entries = audit.take_unsurfaced();
    let unattended = audit.is_unattended();
    drop(audit);

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

    /// Build a log rooted in a temp dir, exercising the real `set_dir` path
    /// (which is what seeds state and starts the writer thread).
    fn temp_log() -> (tempfile::TempDir, BackgroundAudit) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = BackgroundAudit::default();
        log.set_dir(dir.path());
        (dir, log)
    }

    /// Writes go through a background thread, so tests wait for the file to
    /// reach the expected length rather than assuming it is already there.
    /// Bounded so a genuine failure fails the test instead of hanging.
    fn wait_for_lines(log: &BackgroundAudit, want: usize) -> Vec<AuditEntry> {
        for _ in 0..200 {
            let e = log.entries();
            if e.len() >= want {
                return e;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        log.entries()
    }

    #[test]
    fn records_an_unattended_period_as_a_pair() {
        let (_d, mut log) = temp_log();
        log.record(true, 1_000);
        log.record(false, 5_000);
        let e = wait_for_lines(&log, 2);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], AuditEntry { at_ms: 1_000, kind: AuditKind::WentUnattended });
        assert_eq!(e[1], AuditEntry { at_ms: 5_000, kind: AuditKind::Observed });
    }

    #[test]
    fn a_single_unattended_period_is_recorded_once() {
        // Pool churn re-reports a zero-window state; the user should see one
        // period, not a burst.
        let (_d, mut log) = temp_log();
        log.record(true, 1_000);
        log.record(true, 1_100);
        log.record(true, 1_200);
        assert_eq!(wait_for_lines(&log, 1).len(), 1);
    }

    #[test]
    fn opening_a_window_during_normal_use_records_nothing() {
        let (_d, mut log) = temp_log();
        log.record(false, 1_000);
        log.record(false, 2_000);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(log.entries().is_empty());
    }

    /// The reason `record` exists in this shape: it must be callable under
    /// `host_state` without touching the disk, because `host_dispatch` runs
    /// on the CEF UI thread (ReAgent P1 on #3001).
    #[test]
    fn recording_does_not_touch_the_disk_synchronously() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = BackgroundAudit::default();
        log.set_dir(dir.path());
        log.record(true, 1);
        // The decision is visible immediately from memory...
        assert!(log.is_unattended());
        // ...and the write lands later, off the caller's thread.
        assert_eq!(wait_for_lines(&log, 1).len(), 1);
    }

    #[test]
    fn surfacing_is_non_destructive_and_advances_a_watermark() {
        let (_d, mut log) = temp_log();
        log.record(true, 1);
        log.record(false, 2);
        wait_for_lines(&log, 2);
        assert_eq!(log.take_unsurfaced().len(), 2);
        assert!(
            log.take_unsurfaced().is_empty(),
            "a second window must not be re-shown what was already surfaced"
        );
        assert_eq!(
            log.entries().len(),
            2,
            "entries must remain on disk — surfacing is a position, not a deletion"
        );
    }

    #[test]
    fn only_entries_added_since_the_last_surfacing_are_returned() {
        let (_d, mut log) = temp_log();
        log.record(true, 1);
        log.record(false, 2);
        wait_for_lines(&log, 2);
        let _ = log.take_unsurfaced();
        log.record(true, 3);
        wait_for_lines(&log, 3);
        let fresh = log.take_unsurfaced();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].at_ms, 3);
    }

    /// The launcher restarts a crashed host while the background service
    /// stays alive. A fresh log over the same directory must still know it is
    /// mid-period, or the eventual `Observed` is dropped by the no-op guard
    /// and the user never learns the period ended.
    #[test]
    fn unattended_state_survives_a_host_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut before = BackgroundAudit::default();
        before.set_dir(dir.path());
        before.record(true, 1_000);
        wait_for_lines(&before, 1);
        assert!(before.is_unattended());
        drop(before); // host crashes

        let mut after = BackgroundAudit::default();
        after.set_dir(dir.path());
        assert!(after.is_unattended(), "state must be seeded from the log, not memory");
        after.record(false, 9_000);
        let e = wait_for_lines(&after, 2);
        assert_eq!(e.len(), 2);
        assert_eq!(e[1].kind, AuditKind::Observed);
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
        log.set_dir(dir.path());
        assert_eq!(log.entries().len(), 1);
        assert!(log.is_unattended(), "seeded from the last VALID entry");
    }

    #[test]
    fn a_missing_data_dir_degrades_to_a_no_op_rather_than_failing() {
        // Before `set_dir`, and if it is never called: an audit log that
        // cannot be written must never take down the app it audits.
        let mut log = BackgroundAudit::default();
        log.record(true, 1);
        assert!(log.entries().is_empty());
        assert!(log.take_unsurfaced().is_empty());
    }

    /// Below the cap, nothing is ever lost — including the very oldest entry.
    ///
    /// This is the guarantee that actually holds. An earlier version of this
    /// test asserted the oldest entry survives even past the cap, which was
    /// inherited from the two-file scheme where the archive kept everything.
    /// With a hard bound that is not achievable: if more than `KEEP_LINES`
    /// entries are unsurfaced, bounding the file necessarily drops some. The
    /// real contract is "drop oldest-first, and say so loudly" — see
    /// `compact_if_oversized`.
    #[test]
    fn nothing_is_lost_below_the_compaction_threshold() {
        let (_d, mut log) = temp_log();
        let total = MAX_LINES - 10;
        for i in 0..total {
            log.record(i % 2 == 0, i as u64);
        }
        let e = wait_for_lines(&log, total);
        assert_eq!(e.len(), total, "no compaction should have run");

        let surfaced = log.take_unsurfaced();
        assert_eq!(surfaced.len(), total);
        assert_eq!(surfaced[0].at_ms, 0, "the oldest entry must survive");
    }

    /// The specific case the previous fix missed: enough entries to compact
    /// MORE THAN ONCE. The old two-file rotation lost everything archived by
    /// the first rotation as soon as the second one ran.
    #[test]
    fn repeated_compaction_keeps_the_log_bounded_and_recent() {
        let (_d, mut log) = temp_log();
        let total = MAX_LINES * 3;
        for i in 0..total {
            log.record(i % 2 == 0, i as u64);
        }
        // Let the writer drain.
        for _ in 0..400 {
            if log.entries().len() >= KEEP_LINES {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let e = log.entries();
        assert!(
            e.len() <= MAX_LINES,
            "log must stay bounded across repeated compaction, got {}",
            e.len()
        );
        assert!(!e.is_empty(), "repeated compaction must not empty the log");
        // Oldest-first retention: what survives is the RECENT tail, and it is
        // still contiguous and readable (not a half-clobbered file).
        let ts: Vec<u64> = e.iter().map(|x| x.at_ms).collect();
        assert!(
            ts.windows(2).all(|w| w[0] < w[1]),
            "retained entries must stay ordered"
        );
    }

    /// The writer thread and `take_unsurfaced` both mutate the log and the
    /// watermark. Without a shared file lock, a compaction landing between
    /// `take_unsurfaced`'s read and its watermark write left the watermark
    /// past the end of the now-shorter file — after which `already >= len`
    /// was permanently true and NOTHING was ever surfaced again (ReAgent P1
    /// on #3001).
    ///
    /// Races are not deterministic, so this asserts the invariant that
    /// actually matters: after hammering both paths across many compactions,
    /// surfacing still works.
    #[test]
    fn surfacing_still_works_after_racing_the_compacting_writer() {
        let (_d, mut log) = temp_log();
        for i in 0..(MAX_LINES * 2) {
            log.record(i % 2 == 0, i as u64);
            if i % 25 == 0 {
                let _ = log.take_unsurfaced();
            }
        }
        // Let the writer drain whatever is still queued.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = log.take_unsurfaced();

        // A brand-new transition must still be surfacable. Under the bug the
        // watermark sat past the file end and this returned empty forever.
        let flip = !log.is_unattended();
        log.record(flip, 9_999_999);
        for _ in 0..200 {
            if !log.take_unsurfaced().is_empty() {
                return; // surfaced — invariant holds
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("watermark left the log permanently un-surfacable");
    }

    /// `set_dir` is re-invoked on every backend restart (`restart_backend`
    /// IPC, crash auto-restart). It must not stand up a second writer thread
    /// with its own lock over the same files, and it must not clobber the
    /// in-memory state the running host still owns (ReAgent P1 on #3001).
    #[test]
    fn re_initialising_on_backend_restart_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = BackgroundAudit::default();
        log.set_dir(dir.path());
        log.record(true, 1);
        wait_for_lines(&log, 1);
        assert!(log.is_unattended());

        // Backend restart: same host process, same data dir.
        log.set_dir(dir.path());
        assert!(
            log.is_unattended(),
            "the running host's state must survive a backend restart"
        );

        // Recording still works, and lands exactly once.
        log.record(false, 2);
        let e = wait_for_lines(&log, 2);
        assert_eq!(e.len(), 2, "a second writer would corrupt or duplicate this");
        assert_eq!(e[0].kind, AuditKind::WentUnattended);
        assert_eq!(e[1].kind, AuditKind::Observed);
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
