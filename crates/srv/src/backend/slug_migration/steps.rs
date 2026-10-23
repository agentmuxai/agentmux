// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps S1–S8, each as three parts: the before-image it records in the
//! journal's intent, the change it makes, and its undo. Every change is
//! idempotent (it rewrites only what still holds the old value, so a second
//! run changes nothing), and both a change and its undo work from the
//! recorded before-image, never from a fresh look, so a step interrupted
//! halfway is finished or reversed the same way. Database steps are in
//! `steps_db.rs`, file steps in `steps_files.rs`.

use std::path::PathBuf;

use serde_json::Value;

use super::home::{Home, Install};
use super::journal::{BackupEntry, Step};
use super::{steps_db, steps_files};

/// What every step needs to know about the run.
pub(super) struct Ctx<'a> {
    pub home: &'a Home,
    pub installs: &'a [Install],
    pub run_dir: PathBuf,
    pub backups: &'a [BackupEntry],
    /// When the old slugs retire (epoch ms), the same on a resume.
    pub retired_at: i64,
}

impl Ctx<'_> {
    pub fn objects_dbs(&self) -> Vec<PathBuf> {
        self.installs.iter().flat_map(|i| i.versions.iter().map(|v| v.objects_db())).collect()
    }
    pub fn wan_dbs(&self) -> Vec<PathBuf> {
        self.installs.iter().filter_map(|i| i.wan_db()).collect()
    }
    pub fn host_stores(&self) -> Vec<PathBuf> {
        self.home.host_stores(self.installs)
    }
    /// The backup copy of `original`, checked against its checksum.
    pub fn backup_of(&self, original: &std::path::Path) -> Result<PathBuf, String> {
        super::backup::copy_of(&self.run_dir, self.backups, original)
    }
}

/// The agent a step works on.
#[derive(Debug, Clone)]
pub(super) struct Agent {
    pub uid: String,
    pub name: String,
    /// The slug as stored, and the one `fork_slug` gives.
    pub old: String,
    pub new: String,
}

pub(super) fn before(ctx: &Ctx, a: &Agent, step: Step) -> Result<Value, String> {
    match step {
        Step::S1 => steps_files::s1_before(ctx, a),
        Step::S2 => steps_db::s2_before(ctx, a),
        Step::S3 => steps_db::s3_before(ctx, a),
        Step::S4 => steps_db::s4_before(ctx, a),
        Step::S5 => steps_db::s5_before(ctx, a),
        Step::S6 => steps_db::s6_before(ctx, a),
        Step::S7 => {
            let mut v = steps_db::s7_before(ctx, a)?;
            v["files"] = steps_files::s7_files_before(ctx, a)?;
            Ok(v)
        }
        Step::S8 => steps_files::s8_before(ctx, a),
    }
}

pub(super) fn apply(ctx: &Ctx, a: &Agent, step: Step, before: &Value) -> Result<Vec<String>, String> {
    match step {
        Step::S1 => steps_files::s1_apply(ctx, a, before),
        Step::S2 => steps_db::s2_apply(ctx, a, before),
        Step::S3 => steps_db::s3_apply(ctx, a, before),
        Step::S4 => steps_db::s4_apply(ctx, a, before),
        Step::S5 => steps_db::s5_apply(before),
        Step::S6 => steps_db::s6_apply(ctx, a, before),
        Step::S7 => {
            let mut changes = steps_db::s7_apply(ctx, a, before)?;
            changes.extend(steps_files::s7_files_apply(a, &before["files"])?);
            Ok(changes)
        }
        Step::S8 => steps_files::s8_apply(before),
    }
}

pub(super) fn undo(ctx: &Ctx, a: &Agent, step: Step, before: &Value) -> Result<Vec<String>, String> {
    match step {
        Step::S1 => steps_files::s1_undo(ctx, a, before),
        Step::S2 => steps_db::s2_undo(a, before),
        Step::S3 => steps_db::s3_undo(ctx, a, before),
        Step::S4 => steps_db::s4_undo(ctx, a, before),
        Step::S5 => Ok(Vec::new()),
        Step::S6 => steps_db::s6_undo(a, before),
        Step::S7 => {
            let mut changes = steps_db::s7_undo(a, before)?;
            changes.extend(steps_files::s7_files_undo(ctx, a, &before["files"])?);
            Ok(changes)
        }
        Step::S8 => steps_files::s8_undo(ctx, before),
    }
}

/// A before-image, back from the journal.
pub(super) fn parse<T: serde::de::DeserializeOwned>(before: &Value) -> Result<T, String> {
    serde_json::from_value(before.clone()).map_err(|e| format!("the journal's before-image doesn't read: {e}"))
}

pub(super) fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}
