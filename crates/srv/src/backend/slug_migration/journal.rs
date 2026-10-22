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
        Ok(j)
    }

    /// The journal at `path`. A last line cut short by a crash is dropped: it
    /// was never acted on, since a record is written before what it allows.
    pub fn open(path: &Path) -> Result<Journal, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut records = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            match serde_json::from_str::<Record>(line) {
                Ok(r) => records.push(r),
                Err(_) if i + 1 == lines.len() => break,
                Err(e) => return Err(format!("{} line {}: {e}", path.display(), i + 1)),
            }
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

    /// Append `record` and sync it to disk before returning.
    pub fn append(&mut self, record: Record) -> Result<(), String> {
        let mut line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        f.write_all(line.as_bytes()).map_err(|e| format!("{}: {e}", self.path.display()))?;
        f.sync_data().map_err(|e| format!("{}: {e}", self.path.display()))?;
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
