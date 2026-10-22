// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The write-ahead journal, `journal.jsonl` in the run's backup folder: one
//! JSON record per line, appended and synced before the next write it
//! describes. A step's intent (with its before-image) is recorded before
//! the step runs and its result after, so a run that stops anywhere resumes
//! at the first step without a result, and a rollback knows what to undo.
//!
//! No record holds a key or a secret: a before-image names a key row, and
//! the row itself comes back from the backup copy of its database.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::rpc_types::ForkSlugPlan;

pub const JOURNAL_FILE: &str = "journal.jsonl";
const VERSION: u32 = 1;

/// The steps of §4 Phase 2, in the order they run for each agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Step {
    S1,
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
}

pub const STEPS: [Step; 8] = [Step::S1, Step::S2, Step::S3, Step::S4, Step::S5, Step::S6, Step::S7, Step::S8];

/// One file the run copied before its first write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub original: String,
    /// Relative to the run's folder.
    pub copy: String,
    pub sha256: String,
    pub database: bool,
}

/// What a file looked like when the run finished, for restore mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub path: String,
    /// SHA-256 of a file, or of a database's rows (`backup::db_digest`).
    pub digest: String,
    /// A file's modification time, ms; 0 for a database, whose file a WAL
    /// checkpoint touches without changing a row.
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Record {
    Header { version: u32, started_at: i64, retired_at: i64, plan: ForkSlugPlan, backups: Vec<BackupEntry> },
    Intent { uid: String, step: Step, before: Value },
    Done { uid: String, step: Step, changes: Vec<String> },
    Verified { uid: String },
    Failed { uid: String, error: String },
    /// Left out of this run with nothing written for it: its new id was
    /// taken after the plan (another channel created an agent with it).
    Skipped { uid: String, reason: String },
    Finished { at: i64, after: Vec<Fingerprint> },
    Undo { uid: String, step: Step },
    Undone { uid: String, step: Step, changes: Vec<String> },
    RolledBack { at: i64, mode: String },
}

/// An open journal: its records so far, appended to on disk as they come.
pub struct Journal {
    pub path: PathBuf,
    pub records: Vec<Record>,
}

impl Journal {
    /// A new journal in `dir`, starting with its header.
    pub fn create(dir: &Path, started_at: i64, retired_at: i64, plan: ForkSlugPlan, backups: Vec<BackupEntry>) -> Result<Journal, String> {
        let mut j = Journal { path: dir.join(JOURNAL_FILE), records: Vec::new() };
        j.append(Record::Header { version: VERSION, started_at, retired_at, plan, backups })?;
        // The file's directory entry is durable too, not only its bytes.
        sync_dir(dir);
        Ok(j)
    }

    /// The journal at `path`. A last record cut short by a crash (no newline
    /// after it, or one that doesn't parse) was never acted on, since a record
    /// is written before what it allows: it is discarded, and the file cut
    /// back to the end of its last complete line, so the next record starts on
    /// a line of its own. A complete record missing only its newline is kept.
    /// Any other line that doesn't parse is an error.
    pub fn open(path: &Path) -> Result<Journal, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut keep = bytes.len();
        // The last line, and where it starts.
        let body = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
        let start = body.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let last = &body[start..];
        if !last.iter().all(u8::is_ascii_whitespace) && serde_json::from_slice::<Record>(last).is_err() {
            keep = start;
            tracing::warn!(journal = %path.display(), discarded = last.len(), "fork slug migration: discarded a torn last journal record");
        }
        if keep < bytes.len() {
            let f = std::fs::OpenOptions::new().write(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            f.set_len(keep as u64).map_err(|e| format!("{}: {e}", path.display()))?;
            f.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let text = String::from_utf8_lossy(&bytes[..keep]);
        let mut records = Vec::new();
        for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let record = serde_json::from_str::<Record>(line).map_err(|e| format!("{} line {}: {e}", path.display(), i + 1))?;
            records.push(record);
        }
        match records.first() {
            Some(Record::Header { version, .. }) if *version <= VERSION => {}
            _ => return Err(format!("{} has no header this build reads", path.display())),
        }
        Ok(Journal { path: path.to_path_buf(), records })
    }

    pub fn dir(&self) -> PathBuf {
        self.path.parent().map(Path::to_path_buf).unwrap_or_default()
    }

    /// Append `record` on a line of its own and sync it to disk before
    /// returning: the step it allows runs only once it is durable. A file
    /// that doesn't end in a newline (a record written without one) gets
    /// one first, so two records never share a line.
    pub fn append(&mut self, record: Record) -> Result<(), String> {
        use std::io::{Read, Seek, SeekFrom};
        let err = |e: std::io::Error| format!("{}: {e}", self.path.display());
        let mut line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut f = std::fs::OpenOptions::new().create(true).read(true).append(true).open(&self.path).map_err(err)?;
        let len = f.metadata().map_err(err)?.len();
        if len > 0 {
            let mut last = [0u8; 1];
            f.seek(SeekFrom::Start(len - 1)).map_err(err)?;
            f.read_exact(&mut last).map_err(err)?;
            if last[0] != b'\n' {
                line.insert(0, '\n');
            }
        }
        f.write_all(line.as_bytes()).map_err(err)?;
        f.sync_all().map_err(err)?;
        self.records.push(record);
        Ok(())
    }

    pub fn header(&self) -> (i64, &ForkSlugPlan, &[BackupEntry]) {
        match &self.records[0] {
            Record::Header { retired_at, plan, backups, .. } => (*retired_at, plan, backups),
            _ => unreachable!("open and create both start with a header"),
        }
    }

    /// The before-image recorded for `step` of `uid`, if its intent was.
    pub fn intent(&self, uid: &str, step: Step) -> Option<&Value> {
        self.records.iter().find_map(|r| match r {
            Record::Intent { uid: u, step: s, before } if u == uid && *s == step => Some(before),
            _ => None,
        })
    }

    pub fn done(&self, uid: &str, step: Step) -> bool {
        self.records.iter().any(|r| matches!(r, Record::Done { uid: u, step: s, .. } if u == uid && *s == step))
    }

    pub fn skipped(&self, uid: &str) -> Option<&str> {
        self.records.iter().find_map(|r| match r {
            Record::Skipped { uid: u, reason } if u == uid => Some(reason.as_str()),
            _ => None,
        })
    }

    pub fn verified(&self, uid: &str) -> bool {
        self.records.iter().any(|r| matches!(r, Record::Verified { uid: u } if u == uid))
    }

    pub fn undone(&self, uid: &str, step: Step) -> bool {
        self.records.iter().any(|r| matches!(r, Record::Undone { uid: u, step: s, .. } if u == uid && *s == step))
    }

    pub fn finished(&self) -> Option<&[Fingerprint]> {
        self.records.iter().find_map(|r| match r {
            Record::Finished { after, .. } => Some(after.as_slice()),
            _ => None,
        })
    }

    pub fn rolled_back(&self) -> bool {
        self.records.iter().any(|r| matches!(r, Record::RolledBack { .. }))
    }

    /// Every intent, in the order it was recorded.
    pub fn intents(&self) -> Vec<(String, Step, Value)> {
        self.records
            .iter()
            .filter_map(|r| match r {
                Record::Intent { uid, step, before } => Some((uid.clone(), *step, before.clone())),
                _ => None,
            })
            .collect()
    }

    /// The changes each step recorded for `uid`, in order.
    pub fn changes(&self, uid: &str) -> Vec<String> {
        self.records
            .iter()
            .filter_map(|r| match r {
                Record::Done { uid: u, step, changes } if u == uid => Some(changes.iter().map(move |c| format!("{step:?}: {c}"))),
                _ => None,
            })
            .flatten()
            .collect()
    }

    pub fn last_failure(&self) -> Option<String> {
        let failed = self.records.iter().rposition(|r| matches!(r, Record::Failed { .. }))?;
        let later_progress = self.records[failed + 1..].iter().any(|r| matches!(r, Record::Done { .. } | Record::Verified { .. }));
        match &self.records[failed] {
            Record::Failed { uid, error } if !later_progress => Some(format!("{uid}: {error}")),
            _ => None,
        }
    }
}

/// Sync a directory, so a file just created in it survives a power loss.
/// Windows makes a new directory entry durable with the file itself; there is
/// no directory handle to sync there.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Every run's journal under `backups`, newest first.
pub fn journals(backups: &Path) -> Vec<PathBuf> {
    let Ok(dirs) = std::fs::read_dir(backups) else { return Vec::new() };
    let mut out: Vec<PathBuf> = dirs
        .flatten()
        .filter(|d| d.file_name().to_string_lossy().starts_with("slug-migration-"))
        .map(|d| d.path().join(JOURNAL_FILE))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out.reverse();
    out
}
