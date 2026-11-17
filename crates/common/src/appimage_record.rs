// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which data channel an AppImage runs in, kept so that moving or renaming the file doesn't lose it (#47873).
//!
//! An in-app upgrade records the channel beside the AppImage, as `<file>.agentmux-channel`
//! ([`runtime_mode::appimage_channel_file`]). A move or a rename leaves that file behind: AppImageLauncher's
//! "Integrate" moves the AppImage into `~/Applications` under a new name, and the moved file then came up in
//! `stable`, with another channel's windows and tabs.
//!
//! So the channel is also kept in a small record in the account-wide `shared` folder, keyed by which *file*
//! the AppImage is, not where it is: the same size and modification time, and, while the old path is still
//! there, the same file system object (a rename keeps both). A copy that still sits at its old path is another
//! file, never a move, so a duplicate of a release never takes a channel from its twin. A move that changes
//! the modification time (a copy across file systems) is not recognised, and the file starts in the channel
//! built into it, as before.
//!
//! What a start does with it ([`reconcile`]):
//! - the file's own record beside it wins, and is remembered here;
//! - with none beside it, a record of this same file gives the channel, and the record beside the file is
//!   written again, so the next start doesn't need this one;
//! - with neither, nothing changes.
//!
//! A record that isn't a plain channel name is ignored, and so is one that can't be read.

use crate::runtime_mode::{appimage_channel_file, read_channel_file};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The record, in the account-wide `shared` folder.
pub const RECORD_FILE: &str = "appimage-channels.json";

/// How many AppImages are remembered, the most recently started last.
const MAX_RECORDS: usize = 16;

/// What identifies an AppImage file apart from its path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileId {
    pub size: u64,
    /// Modification time, nanoseconds since the epoch.
    pub mtime_ns: u128,
    /// The file system object (0 where there is none): kept by a rename, new for a copy.
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    path: String,
    channel: String,
    id: FileId,
}

/// The identity of the file at `path`.
pub fn file_id(path: &Path) -> Option<FileId> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mtime_ns = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    #[cfg(unix)]
    let (dev, ino) = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let (dev, ino) = (0, 0);
    Some(FileId { size: meta.len(), mtime_ns, dev, ino })
}

/// A channel name as AppRun and the data paths accept it.
fn plain_channel(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Whether `record` is of the file at `path`, whose identity is `now`: the same size and modification time, and
/// either the same path, the same file system object, or an old path that has nothing at it any more (the file
/// was moved). An old path that still holds a file means this one is another file, a copy.
fn is_this_file(record: &Record, path: &str, now: &FileId) -> bool {
    let old = &record.id;
    if old.size != now.size || old.mtime_ns != now.mtime_ns {
        return false;
    }
    let has_object = old.dev != 0 || old.ino != 0;
    record.path == path || (has_object && old.dev == now.dev && old.ino == now.ino) || !Path::new(&record.path).is_file()
}

/// The record's layout version. A build reads and rewrites only a record of this version or none: a newer
/// build's file is left alone (it is neither read nor overwritten), as every file in `shared` is.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    records: Vec<Record>,
}

/// What the record file holds, and whether this build may rewrite it (it may not when a newer build wrote it).
fn load(record: &Path) -> (Vec<Record>, bool) {
    let Ok(text) = std::fs::read_to_string(record) else { return (Vec::new(), true) };
    // The version first, on its own: a newer build may have changed what follows it.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return (Vec::new(), true) };
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v > u64::from(VERSION) => (Vec::new(), false),
        Some(_) => match serde_json::from_value::<Stored>(value) {
            Ok(stored) => (stored.records, true),
            Err(_) => (Vec::new(), true),
        },
        // Not ours, or damaged: nothing to keep, and a new one replaces it.
        None => (Vec::new(), true),
    }
}

fn save(record: &Path, records: Vec<Record>) -> std::io::Result<()> {
    if let Some(dir) = record.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = record.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&Stored { version: VERSION, records }).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, record).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A record of the file at `appimage`: its channel, and the path it was last started from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub channel: String,
    pub was_at: String,
}

/// The record of this very file, wherever it was before.
pub fn lookup(record: &Path, appimage: &Path) -> Option<Found> {
    let now = file_id(appimage)?;
    let path = appimage.to_string_lossy();
    load(record)
        .0
        .into_iter()
        .rev()
        .filter(|r| plain_channel(&r.channel))
        .find(|r| is_this_file(r, &path, &now))
        .map(|r| Found { channel: r.channel, was_at: r.path })
}

/// Remember that the file at `appimage` runs in `channel`.
pub fn remember(record: &Path, appimage: &Path, channel: &str) -> std::io::Result<()> {
    if !plain_channel(channel) {
        return Ok(());
    }
    let Some(id) = file_id(appimage) else { return Ok(()) };
    let path = appimage.to_string_lossy().into_owned();
    let (mut records, writable) = load(record);
    if !writable {
        return Ok(());
    }
    // This path, and this file wherever it was, are replaced by this one.
    records.retain(|r| r.path != path && !is_this_file(r, &path, &id));
    records.push(Record { path, channel: channel.to_string(), id });
    if records.len() > MAX_RECORDS {
        let extra = records.len() - MAX_RECORDS;
        records.drain(..extra);
    }
    save(record, records)
}

/// What a start found out about the AppImage it runs from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// The channel the file is recorded for, if any.
    pub channel: Option<String>,
    /// Where the file was last started from, when that isn't where it is now: it was moved or renamed.
    pub moved_from: Option<String>,
}

/// Bring the records about `appimage` in line with where it is now (see the module doc). Best effort: a
/// folder that can't be written leaves the channel as it was.
pub fn reconcile(record: &Path, appimage: &Path) -> Reconciled {
    let beside = read_channel_file(&appimage_channel_file(appimage)).filter(|c| plain_channel(c));
    let found = lookup(record, appimage);
    let Some(channel) = beside.clone().or_else(|| found.as_ref().map(|f| f.channel.clone())) else {
        return Reconciled::default();
    };
    if beside.is_none() {
        let _ = std::fs::write(appimage_channel_file(appimage), format!("{channel}\n"));
    }
    let moved_from = found.map(|f| f.was_at).filter(|was| *was != appimage.to_string_lossy());
    let _ = remember(record, appimage, &channel);
    Reconciled { channel: Some(channel), moved_from }
}

/// The record's place: the account-wide `shared` folder.
pub fn record_path() -> Option<PathBuf> {
    crate::data_paths::agentmux_root().ok().map(|root| root.join("shared").join(RECORD_FILE))
}

/// The channel recorded for the AppImage at `appimage` when its own record was left behind by a move.
pub fn moved_channel(appimage: &Path) -> Option<String> {
    lookup(&record_path()?, appimage).map(|f| f.channel)
}

#[cfg(test)]
mod tests;
