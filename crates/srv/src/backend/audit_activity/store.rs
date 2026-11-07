// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The day files: `activity-YYYY-MM-DD.jsonl` in the audit folder, appended by a writer thread (the recording is
//! queued, so the transcript's hot path does no I/O here), and dropped when older than [`KEEP_DAYS`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use super::{ActivityRow, KEEP_DAYS};

static SINK: OnceLock<Sender<ActivityRow>> = OnceLock::new();

const PREFIX: &str = "activity-";
const SUFFIX: &str = ".jsonl";

/// A day's file and the day it holds (`YYYY-MM-DD`, UTC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFile {
    pub day: String,
    pub path: PathBuf,
}

/// The UTC day (`YYYY-MM-DD`) of `ms`.
pub(super) fn day_of(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).unwrap_or_default().format("%Y-%m-%d").to_string()
}

/// The day files in `dir`, newest day first. A file that is not named for a day is not listed.
pub fn day_files(dir: &Path) -> Vec<DayFile> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<DayFile> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let day = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?.to_string();
            chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok()?;
            Some(DayFile { day, path: e.path() })
        })
        .collect();
    files.sort_by(|a, b| b.day.cmp(&a.day));
    files
}

fn file_in(dir: &Path, day: &str) -> PathBuf {
    dir.join(format!("{PREFIX}{day}{SUFFIX}"))
}

/// Deletes the day files in `dir` older than [`KEEP_DAYS`] days before `today`. Returns how many went.
pub(super) fn prune(dir: &Path, today: &str) -> usize {
    let Ok(today) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") else { return 0 };
    let mut gone = 0;
    for file in day_files(dir) {
        let Ok(day) = chrono::NaiveDate::parse_from_str(&file.day, "%Y-%m-%d") else { continue };
        if (today - day).num_days() > KEEP_DAYS as i64 && std::fs::remove_file(&file.path).is_ok() {
            gone += 1;
        }
    }
    gone
}

/// Starts the writer for the rest of this process: rows go to `dir`, stamped with `channel`. The first call wins.
pub fn install(dir: PathBuf, channel_name: String) -> std::io::Result<()> {
    let (tx, _writer) = spawn_writer(dir, channel_name)?;
    let _ = SINK.set(tx);
    Ok(())
}

/// The writer thread for `dir`: it ends when the sender is dropped.
pub(super) fn spawn_writer(dir: PathBuf, channel_name: String) -> std::io::Result<(Sender<ActivityRow>, std::thread::JoinHandle<()>)> {
    std::fs::create_dir_all(&dir)?;
    let (tx, rx) = channel::<ActivityRow>();
    let writer = std::thread::Builder::new().name("audit-activity-writer".into()).spawn(move || {
        let mut open: Option<(String, std::fs::File)> = None;
        for mut row in rx {
            // The row's own day, so a day's file holds only that day and files never overlap in time.
            let day = day_of(row.ts);
            if open.as_ref().is_none_or(|(d, _)| *d != day) {
                let path = file_in(&dir, &day);
                match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    Ok(file) => open = Some((day.clone(), file)),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "audit activity: could not open the day file");
                        continue;
                    }
                }
                prune(&dir, &day_of(agentmux_common::time::now_ms_u64()));
            }
            row.channel = Some(channel_name.clone());
            let Ok(mut line) = serde_json::to_vec(&row) else { continue };
            line.push(b'\n');
            if let Some((_, file)) = open.as_mut() {
                // One write call per line, so lines from several srvs don't interleave.
                match file.write_all(&line).and_then(|_| file.flush()) {
                    Ok(()) => crate::backend::audit_live::poke(),
                    Err(e) => tracing::warn!(error = %e, "audit activity: could not append a row"),
                }
            }
        }
    })?;
    Ok((tx, writer))
}

/// Queues `row` for its day file. Nothing happens before [`install`].
pub fn record(row: ActivityRow) {
    if let Some(tx) = SINK.get() {
        let _ = tx.send(row);
    }
}

/// Starts recording where the jekt audit goes: the shared audit folder, or this srv's own while it does not use it.
pub fn install_for_this_srv() {
    let Some(dir) = crate::backend::reactive::audit_store::write_path().and_then(|p| p.parent().map(Path::to_path_buf)) else { return };
    let channel_name = std::env::var("AGENTMUX_INSTANCE_CHANNEL").unwrap_or_default();
    if let Err(e) = install(dir.clone(), channel_name) {
        tracing::warn!(dir = %dir.display(), error = %e, "audit activity: not kept");
    }
}
