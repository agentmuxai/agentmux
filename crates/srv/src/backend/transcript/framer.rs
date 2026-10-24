// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The framer: compresses finished parts of the shared transcript store into
//! frames (SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md §5).
//!
//! Off unless `agent:transcriptframes` is `true`. Every srv whose own setting
//! is on runs it (settings are per channel, so no one srv's speaks for the
//! machine); each frame is one transaction that checks its parts first, so
//! two srvs framing at once never conflict. Before each pass it checks that **every** running `agentmux-srv` has said
//! it reads frames, in a file under `readers/` beside the store (written by
//! [`mark_reader`] when a srv opens the store). An older build, or another
//! agent's dev build from an older branch, can't read frames: while one runs,
//! nothing is framed. The store is stamped `user_version` 3 before the first
//! frame, so an older build started later refuses the store and runs from its
//! own copy (`filestore::core`, `GLOBAL_TRANSCRIPT_STORE_FRAMES_VERSION`).
//!
//! Only `output` of agent zones is framed, 16 parts (1 MiB) at a time, never
//! within 8 MiB of the end of the file or a line received in the last 24
//! hours. `frame_parts` writes each frame, reads it back, compares every byte
//! and only then removes the rows, in one transaction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::backend::storage::error::StoreError;
use crate::backend::storage::filestore::{FileStore, FrameOutcome, CODEC_DELTAS, CODEC_ZSTD, FRAME_PARTS};

/// `agent:transcriptframes`: compress the shared transcript store. Off unless `true`.
pub const SETTING_FRAMES: &str = "agent:transcriptframes";
/// `agent:transcriptdeltas`: with frames on, also store the streaming deltas as
/// uuid and length (codec 2). On unless `false`.
pub const SETTING_DELTAS: &str = "agent:transcriptdeltas";

const OUTPUT: &str = "output";
const TSIDX: &str = "output.tsidx";
/// Nothing within this many bytes of a transcript's end is framed.
const MARGIN_BYTES: i64 = 8 << 20;
/// Nothing received this recently is framed.
const MARGIN_MS: i64 = 24 * 60 * 60 * 1000;
/// Frames written per pass, so a pass stays short (about 64 MiB).
pub const PASS_FRAMES: usize = 64;
/// The frame codecs this build reads, written in its readers file.
const READS_CODECS: [i64; 2] = [CODEC_ZSTD, CODEC_DELTAS];

static CONFIG: OnceLock<Arc<crate::backend::wconfig::ConfigState>> = OnceLock::new();

/// Attach the live settings (bootstrap).
pub fn install_config(config: Arc<crate::backend::wconfig::ConfigState>) {
    let _ = CONFIG.set(config);
}

/// Whether the framer runs (`agent:transcriptframes`).
pub fn enabled() -> bool {
    CONFIG.get().and_then(|c| c.extra_bool(SETTING_FRAMES)).unwrap_or(false)
}

fn deltas_enabled() -> bool {
    CONFIG.get().and_then(|c| c.extra_bool(SETTING_DELTAS)).unwrap_or(true)
}

fn readers_dir(store_dir: &Path) -> PathBuf {
    store_dir.join("readers")
}

/// Process start time (seconds since the epoch) of `pid`, as sysinfo reports it.
fn start_time(sys: &sysinfo::System, pid: u32) -> Option<u64> {
    sys.process(sysinfo::Pid::from_u32(pid)).map(|p| p.start_time())
}

fn scan_processes() -> sysinfo::System {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().without_tasks().with_cmd(sysinfo::UpdateKind::Always),
    );
    sys
}

/// Say, in `readers/<pid>.json` beside the store, that this srv reads frames.
/// Called when a srv opens the shared store. A file left by a process that has
/// exited is pruned by the framer; the start time in it keeps a reused pid from
/// passing for this one.
pub fn mark_reader(store_dir: &Path) {
    let pid = std::process::id();
    let sys = scan_processes();
    let mark = serde_json::json!({
        "pid": pid,
        "started_s": start_time(&sys, pid).unwrap_or(0),
        "version": env!("CARGO_PKG_VERSION"),
        "codecs": READS_CODECS,
    });
    let dir = readers_dir(store_dir);
    let result = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(format!("{pid}.json")), mark.to_string()));
    if let Err(e) = result {
        tracing::warn!(error = %e, dir = %dir.display(), "framer: couldn't write this srv's readers file; framing waits for it");
    }
}

fn is_srv(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("agentmux-srv")
}

/// The last pass's frames, or -1 before the first; and whether it was done.
static LAST_PASS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// Record a pass, for [`steady`].
pub fn note_pass(frames: usize, done: bool) {
    let frames = if done { 0 } else { frames as i64 };
    LAST_PASS.store(frames, std::sync::atomic::Ordering::Relaxed);
}

/// Whether compression has caught up here: off, or its last pass was done or
/// framed at most 4 frames. Before then, giving space back would only have
/// to be done again (SPEC_HISTORY_STORE_RECLAIM_SPACE_2026_10_23 §6).
pub fn steady() -> bool {
    let last = LAST_PASS.load(std::sync::atomic::Ordering::Relaxed);
    !enabled() || (0..=4).contains(&last)
}

/// Another running srv, any build, as a sentence for the log and the Storage
/// view; `None` when this srv is the only one. A reclaim swaps the store's
/// file, so any open connection holds it off, not only an old build's
/// (`filestore::reclaim`).
pub fn other_srv() -> Option<String> {
    let sys = scan_processes();
    let me = std::process::id();
    sys.processes().iter().find_map(|(pid, process)| {
        let name = process.name().to_string_lossy().to_string();
        let monitor = process.cmd().iter().any(|a| a.to_string_lossy() == "--crash-monitor");
        (pid.as_u32() != me && is_srv(&name) && !monitor).then(|| format!("{name} (pid {}) is running", pid.as_u32()))
    })
}

/// The first running srv that hasn't said it reads every frame codec, as a
/// sentence for the log and the Storage view; `None` when all have. Prunes the
/// readers files of processes that have exited.
pub fn who_blocks(store_dir: &Path) -> Option<String> {
    let sys = scan_processes();
    let dir = readers_dir(store_dir);
    // Prune files whose process is gone, or whose pid now belongs to another process.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let alive = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u32>().ok())
                .and_then(|pid| sys.process(sysinfo::Pid::from_u32(pid)))
                .is_some_and(|p| is_srv(&p.name().to_string_lossy()));
            if !alive {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    for (pid, process) in sys.processes() {
        let name = process.name().to_string_lossy().to_string();
        if !is_srv(&name) || process.cmd().iter().any(|a| a.to_string_lossy() == "--crash-monitor") {
            continue;
        }
        let file = dir.join(format!("{}.json", pid.as_u32()));
        let ok = std::fs::read(&file).ok().is_some_and(|b| reads_frames(&b, process.start_time()));
        if !ok {
            return Some(format!("{name} (pid {}) can't read compressed history; compression waits until it exits", pid.as_u32()));
        }
    }
    None
}

/// Whether a readers file says its process (started at `started_s`) reads
/// every frame codec this build writes.
fn reads_frames(file: &[u8], started_s: u64) -> bool {
    serde_json::from_slice::<serde_json::Value>(file).ok().is_some_and(|v| {
        let started = v.get("started_s").and_then(|s| s.as_u64()).unwrap_or(0);
        let codecs: Vec<i64> = v.get("codecs").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
        started.abs_diff(started_s) <= 1 && READS_CODECS.iter().all(|c| codecs.contains(c))
    })
}

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PassReport {
    pub frames: usize,
    pub codec2: usize,
    pub raw_bytes: u64,
    pub stored_bytes: u64,
    pub skipped: usize,
    /// Set when nothing was framed because a srv that can't read frames runs.
    pub waiting: Option<String>,
    /// Every eligible part is framed (no more work until transcripts grow).
    pub done: bool,
}

/// The receive time of the line holding byte `offset` of a transcript, from
/// its sidecar; `None` when the sidecar doesn't say.
fn received_ms(store: &FileStore, zone: &str, offset: i64) -> Option<i64> {
    let size = store.stat(zone, TSIDX).ok()??.size;
    let read = |off: i64, len: i64| store.read_snapshot(|r| r.bytes(zone, TSIDX, off, len));
    let stamps = crate::backend::blockcontroller::shell::stamps::stamps_for(&read, size, &[offset as u64]).ok()??;
    stamps.first().copied().filter(|&ms| ms > 0)
}

/// One pass over the agent transcripts in `store` (at `store_dir`), framing at
/// most `budget` frames. `skip` carries the frames found ineligible (a lost or
/// short part) across passes, so they aren't read again and again.
pub fn pass(store: &FileStore, store_dir: &Path, budget: usize, skip: &mut HashSet<(String, i32)>, now_ms: i64) -> Result<PassReport, StoreError> {
    pass_with(store, &|| who_blocks(store_dir), budget, skip, now_ms)
}

/// [`pass`] with the running-process check given (tests can't depend on
/// which AgentMux builds this machine runs).
fn pass_with(
    store: &FileStore,
    blocks: &dyn Fn() -> Option<String>,
    budget: usize,
    skip: &mut HashSet<(String, i32)>,
    now_ms: i64,
) -> Result<PassReport, StoreError> {
    let mut report = PassReport::default();
    if let Some(who) = blocks() {
        report.waiting = Some(who);
        return Ok(report);
    }
    // Stamp first, then look again: a build that opened the store before the
    // stamp is a running process now, and one that starts after it refuses
    // the store.
    store.stamp_frames_version()?;
    if let Some(who) = blocks() {
        report.waiting = Some(who);
        return Ok(report);
    }
    let codec = if deltas_enabled() { CODEC_DELTAS } else { CODEC_ZSTD };
    let pds = 65536i64;
    let step = FRAME_PARTS;
    for zone in store.zone_ids_with_prefix("agent:")? {
        let Some(file) = store.stat(&zone, OUTPUT)? else { continue };
        let limit = file.size - MARGIN_BYTES;
        if limit <= 0 {
            continue;
        }
        let parts_before_limit = (limit / pds) as i32;
        let framed = store.frame_starts(&zone, OUTPUT)?;
        let mut first = 0i32;
        while first + step <= parts_before_limit {
            let key = (zone.clone(), first);
            if framed.contains(&first) || skip.contains(&key) {
                first += step;
                continue;
            }
            let end_byte = (first + step) as i64 * pds - 1;
            if received_ms(store, &zone, end_byte).is_some_and(|ms| now_ms - ms < MARGIN_MS) {
                break; // later parts are newer still
            }
            match store.frame_parts(&zone, OUTPUT, first, step, codec)? {
                FrameOutcome::Framed { raw, stored, codec } => {
                    report.frames += 1;
                    report.raw_bytes += raw as u64;
                    report.stored_bytes += stored as u64;
                    report.codec2 += usize::from(codec == CODEC_DELTAS);
                }
                FrameOutcome::Skipped(why) => {
                    tracing::debug!(zone = %zone, first, why, "framer: skipped");
                    report.skipped += 1;
                    skip.insert(key);
                }
            }
            if report.frames >= budget {
                return Ok(report);
            }
            first += step;
        }
    }
    report.done = true;
    Ok(report)
}

#[cfg(test)]
#[path = "framer_tests.rs"]
mod tests;
