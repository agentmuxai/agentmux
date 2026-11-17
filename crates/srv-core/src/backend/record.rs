// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record: AgentMux's own size over weeks, kept on this computer
//! (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820 phase 1).
//!
//! A leak that takes weeks to show can only be found from a record kept that
//! long, and Tower's memory ledger (`tower::memory_ledger`) forgets after an
//! hour and a restart. Every five minutes the sample Tower already takes for
//! the ledger is also written here as one JSON line in a file for its day, with
//! the machine's totals, AgentMux's parts and its largest rows. Raw days are
//! kept for [`KEEP_DAYS`], oldest first out, and the whole record has a size
//! budget ([`MAX_BYTES`]).
//!
//! Nothing leaves the computer, and a line holds numbers and stable keys only
//! (`core`, `agent:<block>`, `np:claude.exe`): no text, titles or paths.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::backend::rpc_types::{TowerMemory, TowerSnapshot};

/// The gap between lines.
pub const EVERY_MS: u64 = 5 * 60_000;
/// Raw days kept.
pub const KEEP_DAYS: i64 = 14;
/// The record's own budget: past it, the oldest days go first (never today's).
pub const MAX_BYTES: u64 = 30 * 1024 * 1024;
/// The largest rows one line names; a runaway of differently named processes cannot grow a line.
const MAX_ROWS: usize = 40;
const DAY_FORMAT: &str = "%Y-%m-%d";
const EXT: &str = "jsonl";

/// The machine's totals at the time of a sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineLine {
    /// Fraction of one core, all processes; absent on a first sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f64>,
    /// All processes' private memory, bytes.
    pub mem: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_total: Option<u64>,
    pub procs: u32,
    /// Free and total bytes of the drive that holds this record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<(u64, u64)>,
}

/// One of AgentMux's four parts, or one of its largest rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartLine {
    /// `core`, `panes`, `agents`, `not_placed`; a row's stable key.
    pub k: String,
    pub mem: u64,
    pub n: u32,
}

/// AgentMux's own processes at the time of a sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMuxLine {
    pub total: u64,
    pub procs: u32,
    /// Processes whose memory the OS would not give: not in `total`.
    pub unreadable: u32,
    pub parts: Vec<PartLine>,
    /// Largest first, at most [`MAX_ROWS`].
    pub rows: Vec<PartLine>,
}

/// One line of a day's file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// When it was taken, ms since the epoch.
    pub t: u64,
    /// The version of srv that wrote it.
    pub v: String,
    /// Seconds srv had been running: the age a growth is read against.
    pub up_s: u64,
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<MachineLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub am: Option<AgentMuxLine>,
}

fn part_key(part: &impl Serialize) -> String {
    serde_json::to_value(part).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn agentmux_line(m: &TowerMemory) -> AgentMuxLine {
    let parts = m.parts.iter().map(|p| PartLine { k: part_key(&p.part), mem: p.mem, n: p.processes }).collect();
    let mut rows: Vec<PartLine> =
        m.parts.iter().flat_map(|p| p.rows.iter()).map(|r| PartLine { k: r.key.clone(), mem: r.mem, n: r.processes }).collect();
    rows.sort_by(|a, b| b.mem.cmp(&a.mem).then_with(|| a.k.cmp(&b.k)));
    rows.truncate(MAX_ROWS);
    AgentMuxLine { total: m.total, procs: m.processes, unreadable: m.unreadable, parts, rows }
}

/// A snapshot as one line. `disk` is the record's drive; `up_s` how long srv has run.
pub fn compact(snap: &TowerSnapshot, version: &str, up_s: u64, disk: Option<(u64, u64)>) -> Sample {
    Sample {
        t: snap.ts_ms,
        v: version.to_string(),
        up_s,
        os: snap.os.clone(),
        machine: snap.machine.as_ref().map(|m| MachineLine {
            cpu: m.cpu,
            mem: m.mem,
            mem_total: m.mem_total,
            procs: m.processes,
            disk,
        }),
        am: snap.memory.as_ref().map(agentmux_line),
    }
}

/// `2026-11-16` for a time in ms since the epoch (UTC).
pub fn day_of(t_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(t_ms as i64)
        .map(|d| d.format(DAY_FORMAT).to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

fn file_name(day: &str) -> String {
    format!("{day}.{EXT}")
}

fn day_from_file(name: &str) -> Option<chrono::NaiveDate> {
    let day = name.strip_suffix(&format!(".{EXT}"))?;
    chrono::NaiveDate::parse_from_str(day, DAY_FORMAT).ok()
}

/// The day files to delete, given what is on disk as `(file name, bytes)`: those older than [`KEEP_DAYS`] before
/// `today`, then the oldest of what is left while the total is over `max_bytes`. Today's file is never listed.
pub fn files_to_remove(files: &[(String, u64)], today: &str, keep_days: i64, max_bytes: u64) -> Vec<String> {
    let Ok(today_date) = chrono::NaiveDate::parse_from_str(today, DAY_FORMAT) else { return Vec::new() };
    let mut dated: Vec<(chrono::NaiveDate, &String, u64)> =
        files.iter().filter_map(|(name, size)| day_from_file(name).map(|d| (d, name, *size))).collect();
    dated.sort_by_key(|(d, _, _)| *d);
    let mut remove = Vec::new();
    let mut total: u64 = dated.iter().map(|(_, _, s)| *s).sum();
    for (date, name, size) in &dated {
        if *date == today_date {
            continue;
        }
        let too_old = (today_date - *date).num_days() > keep_days;
        if too_old || total > max_bytes {
            remove.push((*name).clone());
            total = total.saturating_sub(*size);
        }
    }
    remove
}

struct Gate {
    last_t: Option<u64>,
    /// The day pruned last: once a day is enough, and once at the first line.
    pruned_day: Option<String>,
}

/// Where the lines go, and when.
pub struct Record {
    dir: PathBuf,
    version: String,
    gate: Mutex<Gate>,
    started: std::time::Instant,
}

impl Record {
    pub fn new(dir: PathBuf, version: &str) -> Self {
        Self {
            dir,
            version: version.to_string(),
            gate: Mutex::new(Gate { last_t: None, pruned_day: None }),
            started: std::time::Instant::now(),
        }
    }

    /// This is the process's record, in `dir` (the data directory's `record` folder). Set once.
    pub fn global(dir: impl FnOnce() -> Option<PathBuf>, version: &str) -> Option<&'static Record> {
        static GLOBAL: std::sync::OnceLock<Option<Record>> = std::sync::OnceLock::new();
        GLOBAL.get_or_init(|| dir().map(|d| Record::new(d, version))).as_ref()
    }

    /// Offer a sample Tower took: written when [`EVERY_MS`] have passed since the last line. BLOCKING (a file
    /// append); call it where the sample is taken, off the async workers. A failure is logged and never
    /// returned: the record is never a reason for a sample to fail.
    pub fn note(&self, snap: &TowerSnapshot) {
        if snap.memory.is_none() && snap.machine.is_none() {
            return;
        }
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if gate.last_t.is_some_and(|last| snap.ts_ms.saturating_sub(last) < EVERY_MS) {
                return;
            }
            gate.last_t = Some(snap.ts_ms);
        }
        let disk = agentmux_common::disk_space::volume_free_total_bytes(&self.dir_for_disk());
        let sample = compact(snap, &self.version, self.started.elapsed().as_secs(), disk);
        if let Err(e) = self.append(&sample) {
            tracing::warn!(error = %e, "[record] a line could not be written");
            return;
        }
        self.prune_once_a_day(&day_of(snap.ts_ms));
    }

    /// The nearest folder that exists, for the drive's free space (the record's own folder may not exist yet).
    fn dir_for_disk(&self) -> PathBuf {
        let mut p = self.dir.as_path();
        while !p.exists() {
            match p.parent() {
                Some(parent) => p = parent,
                None => break,
            }
        }
        p.to_path_buf()
    }

    fn append(&self, sample: &Sample) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir)?;
        let mut line = serde_json::to_string(sample).map_err(std::io::Error::other)?;
        line.push('\n');
        let path = self.dir.join(file_name(&day_of(sample.t)));
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line.as_bytes())
    }

    fn prune_once_a_day(&self, today: &str) {
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if gate.pruned_day.as_deref() == Some(today) {
                return;
            }
            gate.pruned_day = Some(today.to_string());
        }
        prune(&self.dir, today);
    }
}

/// Delete what [`files_to_remove`] says from `dir`.
fn prune(dir: &Path, today: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let files: Vec<(String, u64)> = entries
        .flatten()
        .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.metadata().ok()?.len())))
        .collect();
    for name in files_to_remove(&files, today, KEEP_DAYS, MAX_BYTES) {
        if let Err(e) = std::fs::remove_file(dir.join(&name)) {
            tracing::warn!(error = %e, file = %name, "[record] an old day could not be removed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{
        TowerMachine, TowerMemoryPart, TowerMemoryPartId, TowerMemoryRow, TowerSnapshot,
    };

    fn row(key: &str, mem: u64) -> TowerMemoryRow {
        TowerMemoryRow { key: key.into(), label: format!("label of {key}"), mem, processes: 2, kind: None, task: None, children: vec![] }
    }

    fn memory() -> TowerMemory {
        TowerMemory {
            total: 3_000,
            processes: 9,
            unreadable: 1,
            parts: vec![
                TowerMemoryPart { part: TowerMemoryPartId::Core, mem: 1_000, processes: 3, rows: vec![row("core:srv", 600), row("core:gpu", 400)] },
                TowerMemoryPart { part: TowerMemoryPartId::Panes, mem: 1_500, processes: 4, rows: vec![row("pane:window:main", 1_500)] },
                TowerMemoryPart { part: TowerMemoryPartId::Agents, mem: 500, processes: 2, rows: vec![row("agent:b1", 500)] },
                TowerMemoryPart { part: TowerMemoryPartId::NotPlaced, mem: 0, processes: 0, rows: vec![] },
            ],
            other_instances: None,
            metric: "private".into(),
            growth: vec![],
            history: None,
        }
    }

    fn snapshot(t: u64) -> TowerSnapshot {
        TowerSnapshot {
            ts_ms: t,
            hostname: "a-computer".into(),
            os: "linux".into(),
            cpu_count: 8,
            memory_metric: "private".into(),
            interval_ms: 2_000,
            remote: false,
            tasks: vec![],
            machine: Some(TowerMachine { cpu: Some(0.5), mem: 9_000, processes: 300, mem_total: Some(32_000), agentmux_elsewhere: None }),
            host: None,
            panes: None,
            memory: Some(memory()),
        }
    }

    const DAY_MS: u64 = 86_400_000;
    /// 2026-11-16 12:00 UTC.
    const NOON: u64 = 1_794_830_400_000;

    #[test]
    fn a_line_holds_numbers_and_keys_but_no_labels_or_names() {
        let line = compact(&snapshot(NOON), "0.60.30", 3_600, Some((10, 100)));
        let json = serde_json::to_string(&line).unwrap();
        assert!(!json.contains("label of"), "{json}");
        assert!(!json.contains("a-computer"), "{json}");
        let am = line.am.unwrap();
        assert_eq!(am.parts.iter().map(|p| p.k.as_str()).collect::<Vec<_>>(), ["core", "panes", "agents", "not_placed"]);
        assert_eq!(am.rows.iter().map(|r| r.k.as_str()).collect::<Vec<_>>(), ["pane:window:main", "core:srv", "agent:b1", "core:gpu"]);
        assert_eq!(line.machine.unwrap().disk, Some((10, 100)));
        assert_eq!((line.v.as_str(), line.up_s, line.t), ("0.60.30", 3_600, NOON));
    }

    #[test]
    fn a_runaway_of_rows_is_cut_to_the_largest() {
        let mut m = memory();
        m.parts[0].rows = (0..200).map(|i| row(&format!("np:p{i}"), i)).collect();
        let am = agentmux_line(&m);
        assert_eq!(am.rows.len(), MAX_ROWS);
        assert!(am.rows.windows(2).all(|w| w[0].mem >= w[1].mem));
    }

    #[test]
    fn a_line_reads_back() {
        let line = compact(&snapshot(NOON), "0.60.30", 5, None);
        let back: Sample = serde_json::from_str(&serde_json::to_string(&line).unwrap()).unwrap();
        assert_eq!(back, line);
    }

    #[test]
    fn the_day_is_the_utc_day() {
        assert_eq!(day_of(NOON), "2026-11-16");
        assert_eq!(day_of(NOON + 12 * 3_600_000), "2026-11-17");
    }

    #[test]
    fn old_days_and_then_the_oldest_over_budget_go_but_never_today() {
        let f = |d: &str, size: u64| (format!("{d}.jsonl"), size);
        let files = vec![f("2026-10-01", 10), f("2026-11-01", 10), f("2026-11-02", 10), f("2026-11-15", 10), f("2026-11-16", 10), ("notes.txt".into(), 99)];
        // Fourteen days back from the 16th keeps the 2nd on; the 1st and October go.
        assert_eq!(files_to_remove(&files, "2026-11-16", 14, 1_000), ["2026-10-01.jsonl", "2026-11-01.jsonl"]);
        // A budget of 25 bytes takes the oldest of the rest until the total fits, and stops before today.
        assert_eq!(
            files_to_remove(&files, "2026-11-16", 14, 25),
            ["2026-10-01.jsonl", "2026-11-01.jsonl", "2026-11-02.jsonl"]
        );
        assert_eq!(files_to_remove(&[f("2026-11-16", 9_999)], "2026-11-16", 14, 1), Vec::<String>::new());
        assert_eq!(files_to_remove(&files, "not a day", 14, 1), Vec::<String>::new());
    }

    #[test]
    fn a_line_is_written_each_five_minutes_to_the_file_of_its_day() {
        let tmp = tempfile::tempdir().unwrap();
        let record = Record::new(tmp.path().join("record"), "0.60.30");
        record.note(&snapshot(NOON));
        record.note(&snapshot(NOON + 2_000)); // a Tower sampling every 2 s: not a new line
        record.note(&snapshot(NOON + EVERY_MS));
        record.note(&snapshot(NOON + DAY_MS)); // the next day's file
        let today = std::fs::read_to_string(tmp.path().join("record/2026-11-16.jsonl")).unwrap();
        assert_eq!(today.lines().count(), 2);
        let first: Sample = serde_json::from_str(today.lines().next().unwrap()).unwrap();
        assert_eq!(first.t, NOON);
        assert!(tmp.path().join("record/2026-11-17.jsonl").exists());
    }

    #[test]
    fn a_sample_with_nothing_in_it_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let record = Record::new(tmp.path().join("record"), "0.60.30");
        let mut empty = snapshot(NOON);
        empty.machine = None;
        empty.memory = None;
        record.note(&empty);
        assert!(!tmp.path().join("record").exists());
    }

    #[test]
    fn pruning_removes_old_days_from_the_folder_once_a_day() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("record");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("2026-09-01.jsonl"), "x\n").unwrap();
        std::fs::write(dir.join("keep.txt"), "x\n").unwrap();
        let record = Record::new(dir.clone(), "0.60.30");
        record.note(&snapshot(NOON));
        assert!(!dir.join("2026-09-01.jsonl").exists());
        assert!(dir.join("keep.txt").exists());
        assert!(dir.join("2026-11-16.jsonl").exists());
    }
}
