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

/// Cap on retained lines. At two lines per unattended period this is ~1000
/// periods; a background instance running for months must not grow the file
/// without bound. On exceeding it the file is rotated to `.1` — the old
/// records are archived rather than deleted, keeping the "immutable" property
/// while bounding the live file.
const MAX_LINES: usize = 2000;

/// The audit log. All state lives in the files; this struct only knows where
/// they are, which is what lets the log survive a host restart.
#[derive(Debug, Default)]
pub struct BackgroundAudit {
    /// `None` when no data dir could be resolved — the log then degrades to a
    /// no-op rather than failing anything. An audit log that cannot be
    /// written must never take down the app it is auditing.
    path: Option<PathBuf>,
}

impl BackgroundAudit {
    /// Resolve from `AGENTMUX_DATA_DIR` (set by the launcher; inherited in
    /// dev). Returns a no-op log when unset.
    pub fn from_env() -> Self {
        let path = std::env::var_os("AGENTMUX_DATA_DIR")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .map(|d| d.join("background-audit.jsonl"));
        Self { path }
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn watermark_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.with_extension("surfaced"))
    }

    /// Every entry on disk, oldest first — **archive first, then the live
    /// file**.
    ///
    /// Reading the archive is not optional. An earlier revision rotated the
    /// live file to `.1` and read only the live file, so any entry that had
    /// not been surfaced when rotation happened vanished from the surfacing
    /// API entirely — the exact opposite of the "over-reporting, never
    /// under-reporting" the rotation comment claimed (ReAgent P1 on #3001).
    ///
    /// Malformed lines are skipped rather than failing the read: a torn final
    /// line (a crash mid-append) must not cost the entries before it.
    pub fn entries(&self) -> Vec<AuditEntry> {
        let mut out = self.read_file(self.archive_path());
        out.extend(self.read_file(self.path.clone()));
        out
    }

    fn read_file(&self, path: Option<PathBuf>) -> Vec<AuditEntry> {
        let Some(path) = path else { return Vec::new() };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines().filter_map(parse_line).collect()
    }

    fn archive_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.with_extension("jsonl.1"))
    }

    /// Lines currently in the archive. Used to keep the watermark aligned
    /// when an archive is discarded.
    fn archive_len(&self) -> usize {
        self.read_file(self.archive_path()).len()
    }

    /// Is the instance currently in an unattended period?
    ///
    /// Derived from the last entry rather than a field, so it is correct
    /// after a host restart — see the module docs.
    pub fn is_unattended(&self) -> bool {
        matches!(
            self.entries().last().map(|e| e.kind),
            Some(AuditKind::WentUnattended)
        )
    }

    /// Record that the instance became unattended. No-op if it already is,
    /// so the repeated zero-window dispatches a pool churn produces record
    /// one period rather than a burst.
    pub fn went_unattended(&self, at_ms: u64) {
        if self.is_unattended() {
            return;
        }
        self.append(AuditEntry { at_ms, kind: AuditKind::WentUnattended });
    }

    /// Record that a window opened. No-op unless the instance was unattended,
    /// so ordinary window-opening during normal use records nothing.
    pub fn observed(&self, at_ms: u64) {
        if !self.is_unattended() {
            return;
        }
        self.append(AuditEntry { at_ms, kind: AuditKind::Observed });
    }

    fn append(&self, entry: AuditEntry) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.rotate_if_oversized();
        let line = format!(
            "{{\"at_ms\":{},\"kind\":\"{}\"}}\n",
            entry.at_ms,
            entry.kind.as_str()
        );
        // Failures are logged, never propagated: the audit log must not be
        // able to break the app it audits.
        match std::fs::OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()) {
                    tracing::warn!(target: "wrr", error = %e, "[audit] append failed");
                }
            }
            Err(e) => tracing::warn!(target: "wrr", error = %e, "[audit] open failed"),
        }
    }

    /// Archive the live file once it grows past `MAX_LINES`. The rotated
    /// `.1` keeps the old records (immutability) while bounding the file the
    /// hot path reads.
    fn rotate_if_oversized(&self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        if text.lines().count() < MAX_LINES {
            return;
        }
        let Some(archive) = self.archive_path() else { return };
        // The archive we are about to overwrite is the only thing that gets
        // discarded. `entries()` reads archive-then-live, so indices stay
        // aligned if the watermark is reduced by exactly the number of lines
        // leaving the concatenation — reducing it to 0 instead (an earlier
        // revision) silently dropped every not-yet-surfaced entry.
        let dropped = self.archive_len();
        if std::fs::rename(path, &archive).is_ok() {
            let w = self.read_watermark().saturating_sub(dropped);
            self.write_watermark(w);
            tracing::info!(
                target: "wrr",
                dropped,
                "[audit] rotated to {}",
                archive.display()
            );
        }
    }

    /// Entries the user has not been shown yet, advancing the watermark.
    ///
    /// Non-destructive: the log keeps everything. "Surfaced" is a position,
    /// not a deletion — which is what makes this an audit log rather than a
    /// queue.
    pub fn take_unsurfaced(&self) -> Vec<AuditEntry> {
        let all = self.entries();
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

    fn write_watermark(&self, n: usize) {
        if let Some(p) = self.watermark_path() {
            let _ = std::fs::write(p, n.to_string());
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

    fn temp_log() -> (tempfile::TempDir, BackgroundAudit) {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = BackgroundAudit::at(dir.path().join("background-audit.jsonl"));
        (dir, log)
    }

    #[test]
    fn records_an_unattended_period_as_a_pair() {
        let (_d, log) = temp_log();
        log.went_unattended(1_000);
        log.observed(5_000);
        let e = log.entries();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], AuditEntry { at_ms: 1_000, kind: AuditKind::WentUnattended });
        assert_eq!(e[1], AuditEntry { at_ms: 5_000, kind: AuditKind::Observed });
    }

    #[test]
    fn a_single_unattended_period_is_recorded_once() {
        // Pool churn re-reports a zero-window state; the user should see one
        // period, not a burst.
        let (_d, log) = temp_log();
        log.went_unattended(1_000);
        log.went_unattended(1_100);
        log.went_unattended(1_200);
        assert_eq!(log.entries().len(), 1);
    }

    #[test]
    fn opening_a_window_during_normal_use_records_nothing() {
        let (_d, log) = temp_log();
        log.observed(1_000);
        log.observed(2_000);
        assert!(log.entries().is_empty());
    }

    #[test]
    fn surfacing_is_non_destructive_and_advances_a_watermark() {
        // The immutability requirement: reading must not erase.
        let (_d, log) = temp_log();
        log.went_unattended(1);
        log.observed(2);
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
        let (_d, log) = temp_log();
        log.went_unattended(1);
        log.observed(2);
        let _ = log.take_unsurfaced();
        log.went_unattended(3);
        let fresh = log.take_unsurfaced();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].at_ms, 3);
    }

    /// The restart case Codex flagged: the launcher restarts a crashed host
    /// while the background service stays alive. A fresh `BackgroundAudit`
    /// over the same path must still know it is mid-unattended-period, or
    /// the eventual `Observed` is dropped and the user never learns the
    /// period ended.
    #[test]
    fn unattended_state_survives_a_host_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background-audit.jsonl");

        let before = BackgroundAudit::at(path.clone());
        before.went_unattended(1_000);
        assert!(before.is_unattended());
        drop(before); // host crashes

        let after = BackgroundAudit::at(path.clone());
        assert!(after.is_unattended(), "state must be derived from the log, not memory");
        after.observed(9_000);
        let e = after.entries();
        assert_eq!(e.len(), 2);
        assert_eq!(e[1].kind, AuditKind::Observed);
    }

    #[test]
    fn a_torn_final_line_does_not_make_the_log_unreadable() {
        // A crash mid-append can leave a partial line. Skipping it must not
        // cost the entries written before it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background-audit.jsonl");
        std::fs::write(
            &path,
            "{\"at_ms\":1,\"kind\":\"went_unattended\"}\n{\"at_ms\":2,\"ki",
        )
        .unwrap();
        let log = BackgroundAudit::at(path);
        assert_eq!(log.entries().len(), 1);
        assert!(log.is_unattended());
    }

    #[test]
    fn a_missing_data_dir_degrades_to_a_no_op_rather_than_failing() {
        // An audit log that cannot be written must never take down the app
        // it audits.
        let log = BackgroundAudit::default();
        log.went_unattended(1);
        assert!(log.entries().is_empty());
        assert!(!log.is_unattended());
        assert!(log.take_unsurfaced().is_empty());
    }

    /// ReAgent P1 on #3001: rotation used to reset the watermark while the
    /// archive was never read, so anything unsurfaced at rotation time was
    /// lost — under-reporting, in an accountability feature.
    #[test]
    fn rotation_does_not_lose_unsurfaced_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background-audit.jsonl");
        let log = BackgroundAudit::at(path.clone());

        // Fill past the rotation threshold without ever surfacing.
        for i in 0..(MAX_LINES as u64 + 10) {
            if i % 2 == 0 {
                log.went_unattended(i);
            } else {
                log.observed(i);
            }
        }

        let surfaced = log.take_unsurfaced();
        assert!(
            !surfaced.is_empty(),
            "entries written before rotation must still be surfacable"
        );
        // The very first entry must still be reachable through the archive.
        assert_eq!(surfaced[0].at_ms, 0, "oldest entry lost to rotation");
    }

    #[test]
    fn entries_include_the_archive_after_a_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background-audit.jsonl");
        let log = BackgroundAudit::at(path.clone());
        for i in 0..(MAX_LINES as u64 + 4) {
            if i % 2 == 0 { log.went_unattended(i) } else { log.observed(i) }
        }
        assert!(
            path.with_extension("jsonl.1").exists(),
            "expected a rotation to have happened"
        );
        assert!(
            log.entries().len() > 4,
            "entries() must span archive + live, not just live"
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
