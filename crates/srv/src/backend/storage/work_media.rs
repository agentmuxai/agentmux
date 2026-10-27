// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The files a Kanban task carries (docs/specs/SPEC_KANBAN_TASK_MEDIA_2026_10_25.md): screenshots and
//! recordings an agent attached to show the operator what a UI change looks like.
//!
//! Two tables beside the board's, so older builds on the same computer never see them:
//!
//! - `db_work_media`: one row per attached file (its content hash, name, kind, who attached it, on
//!   which attempt). The bytes live in `backend/task_media.rs`'s folder, named by the hash.
//! - `db_work_ui`: whether the task changes the UI, and the reason an agent gave for handing it in
//!   without media.

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_board::{BoardTask, EVENTS_RETENTION_MS};
use super::work_queue::work_state;
use super::StoreError;

/// The tables, created with the board's (`work_board::SCHEMA`), with no schema version bump.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_work_media (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        work_id         TEXT NOT NULL,
        sha256          TEXT NOT NULL,
        name            TEXT NOT NULL,
        kind            TEXT NOT NULL,
        mime            TEXT NOT NULL DEFAULT '',
        size            INTEGER NOT NULL DEFAULT 0,
        caption         TEXT NOT NULL DEFAULT '',
        attached_by     TEXT NOT NULL DEFAULT '',
        attached_by_uid TEXT NOT NULL DEFAULT '',
        attempt         INTEGER NOT NULL DEFAULT 0,
        at              INTEGER NOT NULL,
        removed_at      INTEGER,
        removed_by      TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_ids_work_media_work ON db_work_media(work_id);
    CREATE INDEX IF NOT EXISTS idx_ids_work_media_sha ON db_work_media(sha256);
    CREATE TABLE IF NOT EXISTS db_work_ui (
        work_id         TEXT PRIMARY KEY,
        ui_change       INTEGER NOT NULL DEFAULT 0,
        no_media_reason TEXT NOT NULL DEFAULT '',
        reason_attempt  INTEGER NOT NULL DEFAULT 0,
        updated_at      INTEGER NOT NULL DEFAULT 0
    );";

/// Files on one task, and their total size (what is not removed).
pub const MAX_FILES_PER_TASK: i64 = 30;
pub const MAX_BYTES_PER_TASK: i64 = 300 * 1024 * 1024;
/// How many a card shows as thumbnails.
pub const PREVIEWS: usize = 3;

/// Task numbers per lookup query, well under SQLite's bound-parameter limit.
const IDS_PER_QUERY: usize = 500;

/// An image or a recording.
pub mod media_kind {
    pub const IMAGE: &str = "image";
    pub const RECORDING: &str = "recording";
}

/// One attached file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: i64,
    pub work_id: String,
    pub sha256: String,
    pub name: String,
    pub kind: String,
    pub mime: String,
    pub size: i64,
    pub caption: String,
    pub attached_by: String,
    pub attempt: i64,
    pub at: i64,
    /// When the operator removed it; the drawer says so and shows no picture.
    pub removed_at: Option<i64>,
    /// Where the stored file is, for the Editor (filled in by srv when it answers; empty if removed).
    #[serde(default)]
    pub path: String,
}

/// What a file is attached with.
#[derive(Debug, Clone, Default)]
pub struct NewMedia {
    pub sha256: String,
    pub name: String,
    pub kind: String,
    pub mime: String,
    pub size: i64,
    pub caption: String,
    pub attached_by: String,
    pub attached_by_uid: String,
    pub attempt: i64,
}

/// A thumbnail on the card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPreview {
    pub sha256: String,
    pub kind: String,
    pub mime: String,
    /// Where the stored file and its thumbnail are (filled in by srv when it answers; the thumbnail is
    /// empty when there is none).
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub thumb: String,
}

/// What the card shows of a task's media (`WorkCard.media`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSummary {
    /// Files attached and not removed.
    pub count: i64,
    /// The first few, oldest first.
    pub first: Vec<MediaPreview>,
    /// The task changes the UI (the flag).
    pub ui_change: bool,
    /// Why it was handed in without media, when it was.
    pub no_media_reason: String,
}

/// Why a file can't be attached to a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachError {
    TooMany,
    TooBig,
}

impl AttachError {
    pub fn words(&self) -> String {
        match self {
            AttachError::TooMany => format!("a task carries at most {MAX_FILES_PER_TASK} files"),
            AttachError::TooBig => format!("a task carries at most {} MiB of files", MAX_BYTES_PER_TASK / (1024 * 1024)),
        }
    }
}

fn row_to_media(r: &Row) -> rusqlite::Result<MediaItem> {
    Ok(MediaItem {
        id: r.get("id")?,
        work_id: r.get("work_id")?,
        sha256: r.get("sha256")?,
        name: r.get("name")?,
        kind: r.get("kind")?,
        mime: r.get("mime")?,
        size: r.get("size")?,
        caption: r.get("caption")?,
        attached_by: r.get("attached_by")?,
        attempt: r.get("attempt")?,
        at: r.get("at")?,
        removed_at: r.get("removed_at")?,
        path: String::new(),
    })
}

const MEDIA_COLS: &str = "id, work_id, sha256, name, kind, mime, size, caption, attached_by, attempt, at, removed_at";

/// Fill each task's `card.media`.
pub(crate) fn attach_media(conn: &Connection, tasks: &mut [BoardTask]) -> Result<(), StoreError> {
    let ids: Vec<String> = tasks.iter().map(|t| t.item.id.clone()).collect();
    let mut found: HashMap<String, MediaSummary> = HashMap::new();
    for chunk in ids.chunks(IDS_PER_QUERY) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT work_id, sha256, kind, mime FROM db_work_media WHERE removed_at IS NULL AND work_id IN ({marks}) ORDER BY id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
            Ok((r.get::<_, String>(0)?, MediaPreview { sha256: r.get(1)?, kind: r.get(2)?, mime: r.get(3)?, ..Default::default() }))
        })?;
        for row in rows {
            let (id, preview) = row?;
            let s = found.entry(id).or_default();
            s.count += 1;
            if s.first.len() < PREVIEWS {
                s.first.push(preview);
            }
        }
        let sql = format!("SELECT work_id, ui_change, no_media_reason FROM db_work_ui WHERE work_id IN ({marks})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0, r.get::<_, String>(2)?))
        })?;
        for row in rows {
            let (id, ui, reason) = row?;
            let s = found.entry(id).or_default();
            s.ui_change = ui;
            s.no_media_reason = reason;
        }
    }
    for t in tasks {
        t.card.media = found.remove(&t.item.id).unwrap_or_default();
    }
    Ok(())
}

impl Store {
    /// Attach a file to a task, inside the per-task limits. A file the task already carries for this attempt (same
    /// content, not removed) is not attached twice; the earlier row is returned. The same picture again after a
    /// send-back is a new row, so it counts for the new attempt.
    pub fn work_media_add(&self, work_id: &str, m: &NewMedia, now_ms: i64) -> Result<Result<MediaItem, AttachError>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let existing = tx
            .query_row(
                &format!("SELECT {MEDIA_COLS} FROM db_work_media WHERE work_id = ?1 AND sha256 = ?2 AND attempt = ?3 AND removed_at IS NULL"),
                params![work_id, m.sha256, m.attempt],
                row_to_media,
            )
            .optional()?;
        if let Some(item) = existing {
            return Ok(Ok(item));
        }
        let (count, bytes): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM db_work_media WHERE work_id = ?1 AND removed_at IS NULL",
            params![work_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count >= MAX_FILES_PER_TASK {
            return Ok(Err(AttachError::TooMany));
        }
        if bytes + m.size > MAX_BYTES_PER_TASK {
            return Ok(Err(AttachError::TooBig));
        }
        tx.execute(
            "INSERT INTO db_work_media (work_id, sha256, name, kind, mime, size, caption, attached_by, attached_by_uid, attempt, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![work_id, m.sha256, m.name, m.kind, m.mime, m.size, m.caption, m.attached_by, m.attached_by_uid, m.attempt, now_ms],
        )?;
        let id = tx.last_insert_rowid();
        // The card changes, so boards elsewhere see it.
        tx.execute("UPDATE db_work_cards SET updated_at = MAX(updated_at, ?2) WHERE work_id = ?1", params![work_id, now_ms])?;
        tx.execute("UPDATE db_work_queue SET updated_at = MAX(updated_at, ?2) WHERE id = ?1", params![work_id, now_ms])?;
        let item = tx.query_row(&format!("SELECT {MEDIA_COLS} FROM db_work_media WHERE id = ?1"), params![id], row_to_media)?;
        tx.commit()?;
        Ok(Ok(item))
    }

    /// Every file of a task, oldest first, the removed ones marked.
    pub fn work_media_list(&self, work_id: &str) -> Result<Vec<MediaItem>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {MEDIA_COLS} FROM db_work_media WHERE work_id = ?1 ORDER BY id"))?;
        let rows = stmt.query_map(params![work_id], row_to_media)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Whether an image or a recording was attached to this task in `attempt` (and is not removed).
    pub fn work_media_in_attempt(&self, work_id: &str, attempt: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_work_media WHERE work_id = ?1 AND attempt = ?2 AND removed_at IS NULL)",
            params![work_id, attempt],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    /// The operator takes a file off a task. Returns the removed row, or `None` when it wasn't there or already gone.
    pub fn work_media_remove(&self, work_id: &str, media_id: i64, by: &str, now_ms: i64) -> Result<Option<MediaItem>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE db_work_media SET removed_at = ?3, removed_by = ?4 WHERE id = ?2 AND work_id = ?1 AND removed_at IS NULL",
            params![work_id, media_id, now_ms, by],
        )?;
        if n == 0 {
            return Ok(None);
        }
        tx.execute("UPDATE db_work_cards SET updated_at = MAX(updated_at, ?2) WHERE work_id = ?1", params![work_id, now_ms])?;
        tx.execute("UPDATE db_work_queue SET updated_at = MAX(updated_at, ?2) WHERE id = ?1", params![work_id, now_ms])?;
        let item = tx.query_row(&format!("SELECT {MEDIA_COLS} FROM db_work_media WHERE id = ?1"), params![media_id], row_to_media)?;
        tx.commit()?;
        Ok(Some(item))
    }

    /// Set the UI-change flag and/or the reason a task was handed in without media, given in `attempt`; `None`
    /// leaves one as it is.
    pub fn work_ui_set(
        &self,
        work_id: &str,
        ui_change: Option<bool>,
        no_media_reason: Option<&str>,
        attempt: i64,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        if ui_change.is_none() && no_media_reason.is_none() {
            return Ok(());
        }
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO db_work_ui (work_id, updated_at) VALUES (?1, ?2)", params![work_id, now_ms])?;
        if let Some(ui) = ui_change {
            tx.execute("UPDATE db_work_ui SET ui_change = ?2, updated_at = ?3 WHERE work_id = ?1", params![work_id, ui as i64, now_ms])?;
        }
        if let Some(reason) = no_media_reason {
            tx.execute(
                "UPDATE db_work_ui SET no_media_reason = ?2, reason_attempt = ?4, updated_at = ?3 WHERE work_id = ?1",
                params![work_id, reason, now_ms, attempt],
            )?;
        }
        tx.execute("UPDATE db_work_cards SET updated_at = MAX(updated_at, ?2) WHERE work_id = ?1", params![work_id, now_ms])?;
        tx.execute("UPDATE db_work_queue SET updated_at = MAX(updated_at, ?2) WHERE id = ?1", params![work_id, now_ms])?;
        tx.commit()?;
        Ok(())
    }

    /// Whether the task changes the UI, the reason it was handed in without media, and the attempt that gave it.
    pub fn work_ui_get(&self, work_id: &str) -> Result<(bool, String, i64), StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row("SELECT ui_change, no_media_reason, reason_attempt FROM db_work_ui WHERE work_id = ?1", params![work_id], |r| {
                Ok((r.get::<_, i64>(0)? != 0, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })
            .optional()?
            .unwrap_or_default())
    }

    /// Forget the files of tasks finished more than [`EVENTS_RETENTION_MS`] ago, and what is known about
    /// tasks no longer in the queue. Returns the content hashes no task still carries, whose files can go.
    pub fn work_media_prune(&self, now_ms: i64) -> Result<Vec<String>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let gone = format!(
            "work_id IN (SELECT id FROM db_work_queue WHERE state IN ('{}', '{}', '{}') AND updated_at < {})
             OR work_id NOT IN (SELECT id FROM db_work_queue)",
            work_state::DONE,
            work_state::CANCELLED,
            work_state::FAILED,
            now_ms - EVENTS_RETENTION_MS
        );
        let mut stmt = tx.prepare(&format!("SELECT DISTINCT sha256 FROM db_work_media WHERE {gone}"))?;
        let candidates = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        tx.execute(&format!("DELETE FROM db_work_media WHERE {gone}"), [])?;
        tx.execute(&format!("DELETE FROM db_work_ui WHERE {gone}"), [])?;
        let mut free = Vec::new();
        for sha in candidates {
            let named: i64 = tx.query_row("SELECT COUNT(*) FROM db_work_media WHERE sha256 = ?1 AND removed_at IS NULL", params![sha], |r| r.get(0))?;
            if named == 0 {
                free.push(sha);
            }
        }
        tx.commit()?;
        Ok(free)
    }

    /// Whether a task still carries this content (a removed file is not carried).
    pub fn work_media_named(&self, sha256: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM db_work_media WHERE sha256 = ?1 AND removed_at IS NULL)", params![sha256], |r| r.get::<_, i64>(0))? != 0)
    }
}

#[cfg(test)]
#[path = "work_media_tests.rs"]
mod tests;
