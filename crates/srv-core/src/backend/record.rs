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
//! budget ([`MAX_BYTES`]). Each finished hour is also folded into one rollup
//! line (min, median, p95 and max of each figure) in a file for its month,
//! kept for a year ([`ROLLUP_KEEP_DAYS`]) within its own budget
//! ([`ROLLUP_MAX_BYTES`]): slow growth over months is read from those.
//!
//! Nothing leaves the computer, and a line holds numbers and stable keys only
//! (`core`, `agent:<block>`, `np:claude.exe`, `renderer:window:<hash>`): no
//! text, titles or paths. A renderer's own key is made of its window's or
//! page's name, so a line keeps its kind and a hash of it instead.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::backend::rpc_types::{TowerMemory, TowerSnapshot};

/// The gap between lines.
pub const EVERY_MS: u64 = 5 * 60_000;
/// How much early a sample still counts as the next line: a sample that ran long, or one Tower served from its
/// 900 ms cache, must not make the following one look too soon and leave a ten-minute gap.
const JITTER_MS: u64 = 30_000;
const HOUR_MS: u64 = 3_600_000;
/// Raw days kept.
pub const KEEP_DAYS: i64 = 14;
/// The record's own budget: past it, the oldest days go first (never today's).
pub const MAX_BYTES: u64 = 30 * 1024 * 1024;
/// Hourly rollups kept: a year, and a month's file goes whole.
pub const ROLLUP_KEEP_DAYS: i64 = 366;
/// The rollups' own budget (a year is about 9 MB), so the record as a whole stays under 40 MB.
pub const ROLLUP_MAX_BYTES: u64 = 9 * 1024 * 1024;
/// The rows an hour's rollup names, by their largest figure.
const ROLLUP_ROWS: usize = 6;
const ROLLUP_PREFIX: &str = "rollup-";
const MONTH_FORMAT: &str = "%Y-%m";
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

/// FNV-1a: the same hash on every run and platform (`DefaultHasher` makes no such promise).
fn fnv(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// A row's key as the record keeps it. A renderer's key is the slug of what it serves (a workspace's name, a page's
/// host), so it is cut to its kind and a hash of itself: the same figure can be followed from line to line, and
/// no name is written.
fn persisted_key(key: &str) -> String {
    let Some(rest) = key.strip_prefix("renderer:") else { return key.to_string() };
    // Already cut (a line this version wrote): `<kind>:<8 hex>`.
    if let Some((kind, hash)) = rest.split_once(':') {
        if ["window", "browser", "pool", "other"].contains(&kind) && hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return key.to_string();
        }
    }
    let kind = if rest.contains("pool-window") {
        "pool"
    } else if rest.contains("browser-pane") {
        "browser"
    } else if rest.contains("window") {
        "window"
    } else {
        "other"
    };
    format!("renderer:{kind}:{:08x}", fnv(rest))
}

fn agentmux_line(m: &TowerMemory) -> AgentMuxLine {
    let parts = m.parts.iter().map(|p| PartLine { k: part_key(&p.part), mem: p.mem, n: p.processes }).collect();
    let mut rows: Vec<PartLine> =
        m.parts.iter().flat_map(|p| p.rows.iter()).map(|r| PartLine { k: persisted_key(&r.key), mem: r.mem, n: r.processes }).collect();
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

/// min, median, p95 and max of a figure over an hour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stat(pub u64, pub u64, pub u64, pub u64);

/// One finished hour of lines folded into one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rollup {
    /// The hour's start, ms since the epoch.
    pub h: u64,
    /// Lines in the hour.
    pub n: u32,
    /// The version and the uptime (seconds) of the hour's last line.
    pub v: String,
    pub up_s: u64,
    /// Times srv started again inside the hour (its uptime went back).
    pub restarts: u32,
    /// All processes' CPU, thousandths of a core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Stat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem: Option<Stat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free: Option<Stat>,
    /// AgentMux's total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub am: Option<Stat>,
    pub parts: Vec<(String, Stat)>,
    /// The [`ROLLUP_ROWS`] rows with the largest peak. A row missing from a line counts as 0 there.
    pub rows: Vec<(String, Stat)>,
}

fn stat(mut v: Vec<u64>) -> Option<Stat> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let at = |p: f64| v[((p * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
    Some(Stat(v[0], at(0.5), at(0.95), v[v.len() - 1]))
}

/// Each key's figure over the lines, by key, in the order first seen; a line without the key gives 0.
fn series(samples: &[Sample], pick: impl Fn(&AgentMuxLine) -> &[PartLine]) -> Vec<(String, Vec<u64>)> {
    let mut out: Vec<(String, Vec<u64>)> = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        for row in s.am.as_ref().map(&pick).unwrap_or_default() {
            // A line an older version wrote still has a renderer's name in its key.
            let key = persisted_key(&row.k);
            let at = match out.iter().position(|(k, _)| *k == key) {
                Some(at) => at,
                None => {
                    out.push((key, vec![0; samples.len()]));
                    out.len() - 1
                }
            };
            out[at].1[i] = out[at].1[i].max(row.mem);
        }
    }
    out
}

fn stats_of(series: Vec<(String, Vec<u64>)>) -> Vec<(String, Stat)> {
    series.into_iter().filter_map(|(k, v)| stat(v).map(|s| (k, s))).collect()
}

/// The hour starting at `hour` as one line, from its lines (in time order). None for no lines.
pub fn rollup(hour: u64, samples: &[Sample]) -> Option<Rollup> {
    let last = samples.last()?;
    let machine = || samples.iter().filter_map(|s| s.machine.as_ref());
    let mut rows = stats_of(series(samples, |a| &a.rows));
    rows.sort_by(|a, b| b.1 .3.cmp(&a.1 .3).then_with(|| a.0.cmp(&b.0)));
    rows.truncate(ROLLUP_ROWS);
    Some(Rollup {
        h: hour,
        n: samples.len() as u32,
        v: last.v.clone(),
        up_s: last.up_s,
        restarts: samples.windows(2).filter(|w| w[1].up_s < w[0].up_s).count() as u32,
        cpu: stat(machine().filter_map(|m| m.cpu).map(|c| (c * 1000.0).round() as u64).collect()),
        mem: stat(machine().map(|m| m.mem).collect()),
        disk_free: stat(machine().filter_map(|m| m.disk).map(|d| d.0).collect()),
        am: stat(samples.iter().filter_map(|s| s.am.as_ref()).map(|a| a.total).collect()),
        parts: stats_of(series(samples, |a| &a.parts)),
        rows,
    })
}

fn rollup_file_name(hour: u64) -> String {
    let month = chrono::DateTime::from_timestamp_millis(hour as i64).map(|d| d.format(MONTH_FORMAT).to_string());
    format!("{ROLLUP_PREFIX}{}.{EXT}", month.unwrap_or_else(|| "1970-01".to_string()))
}

fn month_from_file(name: &str) -> Option<chrono::NaiveDate> {
    let month = name.strip_prefix(ROLLUP_PREFIX)?.strip_suffix(&format!(".{EXT}"))?;
    chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").ok()
}

/// The rollup files to delete: a month that ended more than `keep_days` before `today`, then the oldest of what is
/// left while the total is over `max_bytes`. The month of `today` is never listed.
pub fn rollups_to_remove(files: &[(String, u64)], today: &str, keep_days: i64, max_bytes: u64) -> Vec<String> {
    let Ok(today_date) = chrono::NaiveDate::parse_from_str(today, DAY_FORMAT) else { return Vec::new() };
    let mut months: Vec<(chrono::NaiveDate, &String, u64)> =
        files.iter().filter_map(|(name, size)| month_from_file(name).map(|d| (d, name, *size))).collect();
    months.sort_by_key(|(d, _, _)| *d);
    let mut total: u64 = months.iter().map(|(_, _, s)| *s).sum();
    let mut remove = Vec::new();
    for (start, name, size) in &months {
        let end = start.checked_add_months(chrono::Months::new(1)).unwrap_or(*start);
        if end > today_date {
            continue;
        }
        if (today_date - end).num_days() > keep_days || total > max_bytes {
            remove.push((*name).clone());
            total = total.saturating_sub(*size);
        }
    }
    remove
}

/// The hour of the last rollup written, from the newest month's file.
fn last_rollup_hour(dir: &Path) -> Option<u64> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| month_from_file(n).is_some())
        .collect();
    names.sort();
    // Newest month first, until one has a whole line: a month whose first append failed is empty.
    names.iter().rev().find_map(|name| {
        let text = std::fs::read_to_string(dir.join(name)).ok()?;
        text.lines().rev().find_map(|l| serde_json::from_str::<Rollup>(l).ok()).map(|r| r.h)
    })
}

/// The raw lines of the days from `from_day` on, in time order.
fn raw_since(dir: &Path, from_day: Option<&str>) -> Vec<Sample> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let from = from_day.and_then(|d| chrono::NaiveDate::parse_from_str(d, DAY_FORMAT).ok());
    let mut days: Vec<(chrono::NaiveDate, PathBuf)> = entries
        .flatten()
        .filter_map(|e| Some((day_from_file(e.file_name().to_str()?)?, e.path())))
        .filter(|(d, _)| from.is_none_or(|f| *d >= f))
        .collect();
    days.sort();
    let mut out: Vec<Sample> = Vec::new();
    for (_, path) in days {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        out.extend(text.lines().filter_map(|l| serde_json::from_str::<Sample>(l).ok()));
    }
    out.sort_by_key(|s| s.t);
    out
}

/// Fold every finished hour (before the one `now_ms` is in) that has no rollup yet into one, oldest first.
fn roll_up(dir: &Path, now_ms: u64) {
    use std::io::Write;
    let current = now_ms / HOUR_MS * HOUR_MS;
    let last = last_rollup_hour(dir);
    let from_day = last.map(|h| day_of(h + HOUR_MS));
    let mut hours: std::collections::BTreeMap<u64, Vec<Sample>> = std::collections::BTreeMap::new();
    for s in raw_since(dir, from_day.as_deref()) {
        let h = s.t / HOUR_MS * HOUR_MS;
        if h < current && last.is_none_or(|l| h > l) {
            hours.entry(h).or_default().push(s);
        }
    }
    for (hour, samples) in hours {
        let Some(line) = rollup(hour, &samples).and_then(|r| serde_json::to_string(&r).ok()) else { continue };
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(rollup_file_name(hour)))
            .and_then(|mut f| f.write_all(format!("{line}\n").as_bytes()));
        if let Err(e) = written {
            tracing::warn!(error = %e, "[record] an hour could not be rolled up");
            return;
        }
    }
}

struct Gate {
    last_t: Option<u64>,
    /// The hour rolled up last: once an hour is enough, and once at the first line.
    rolled_hour: Option<u64>,
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
            gate: Mutex::new(Gate { last_t: None, rolled_hour: None, pruned_day: None }),
            started: std::time::Instant::now(),
        }
    }

    /// This is the process's record, in `dir` (the data directory's `record` folder). Set once.
    pub fn global(dir: impl FnOnce() -> Option<PathBuf>, version: &str) -> Option<&'static Record> {
        static GLOBAL: std::sync::OnceLock<Option<Record>> = std::sync::OnceLock::new();
        GLOBAL.get_or_init(|| dir().map(|d| Record::new(d, version))).as_ref()
    }

    /// Offer a sample Tower took: written when [`EVERY_MS`] (less [`JITTER_MS`]) have passed since the last line,
    /// and the hours finished since the last rollup are folded when a new hour begins. BLOCKING (a file
    /// append); call it where the sample is taken, off the async workers. A failure is logged and never
    /// returned: the record is never a reason for a sample to fail.
    pub fn note(&self, snap: &TowerSnapshot) {
        if snap.memory.is_none() && snap.machine.is_none() {
            return;
        }
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if gate.last_t.is_some_and(|last| snap.ts_ms.saturating_sub(last) < EVERY_MS - JITTER_MS) {
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
        self.roll_up_once_an_hour(snap.ts_ms);
        self.prune_once_a_day(&day_of(snap.ts_ms));
    }

    fn roll_up_once_an_hour(&self, now_ms: u64) {
        let hour = now_ms / HOUR_MS;
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if gate.rolled_hour == Some(hour) {
                return;
            }
            gate.rolled_hour = Some(hour);
        }
        roll_up(&self.dir, now_ms);
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
    let doomed = files_to_remove(&files, today, KEEP_DAYS, MAX_BYTES)
        .into_iter()
        .chain(rollups_to_remove(&files, today, ROLLUP_KEEP_DAYS, ROLLUP_MAX_BYTES));
    for name in doomed {
        if let Err(e) = std::fs::remove_file(dir.join(&name)) {
            tracing::warn!(error = %e, file = %name, "[record] an old day could not be removed");
        }
    }
}

/// Where this machine keeps its record: the `record` folder of the data directory.
pub fn default_dir() -> Option<PathBuf> {
    agentmux_common::data_paths::DataPaths::from_env().map(|p| p.data_dir.join("record"))
}

/// What the record holds on disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    pub raw_days: u32,
    pub raw_bytes: u64,
    /// The oldest day kept, `2026-11-02`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_first_day: Option<String>,
    pub rollup_months: u32,
    pub rollup_bytes: u64,
    /// The oldest month kept, `2026-01`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollup_first_month: Option<String>,
}

/// The files of `dir`, counted and sized.
pub fn inventory(dir: &Path) -> Inventory {
    let mut inv = Inventory::default();
    let Ok(entries) = std::fs::read_dir(dir) else { return inv };
    let mut days: Vec<String> = Vec::new();
    let mut months: Vec<String> = Vec::new();
    for e in entries.flatten() {
        let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        if day_from_file(&name).is_some() {
            inv.raw_bytes += size;
            days.push(name.trim_end_matches(&format!(".{EXT}")).to_string());
        } else if month_from_file(&name).is_some() {
            inv.rollup_bytes += size;
            months.push(name.trim_start_matches(ROLLUP_PREFIX).trim_end_matches(&format!(".{EXT}")).to_string());
        }
    }
    days.sort();
    months.sort();
    inv.raw_days = days.len() as u32;
    inv.rollup_months = months.len() as u32;
    inv.raw_first_day = days.into_iter().next();
    inv.rollup_first_month = months.into_iter().next();
    inv
}

/// The last `limit` lines taken from `from_ms` to `to_ms`, in time order.
pub fn read_raw(dir: &Path, from_ms: u64, to_ms: u64, limit: usize) -> Vec<Sample> {
    let mut lines: Vec<Sample> = raw_since(dir, Some(&day_of(from_ms))).into_iter().filter(|s| s.t >= from_ms && s.t <= to_ms).collect();
    let cut = lines.len().saturating_sub(limit);
    lines.drain(..cut);
    lines
}

/// The last `limit` hourly rollups of the hours that start from `from_ms` to `to_ms`, in time order.
pub fn read_rollups(dir: &Path, from_ms: u64, to_ms: u64, limit: usize) -> Vec<Rollup> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(chrono::NaiveDate, PathBuf)> =
        entries.flatten().filter_map(|e| Some((month_from_file(e.file_name().to_str()?)?, e.path()))).collect();
    files.sort();
    let mut out: Vec<Rollup> = Vec::new();
    for (_, path) in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        out.extend(text.lines().filter_map(|l| serde_json::from_str::<Rollup>(l).ok()).filter(|r| r.h >= from_ms && r.h <= to_ms));
    }
    out.sort_by_key(|r| r.h);
    let cut = out.len().saturating_sub(limit);
    out.drain(..cut);
    out
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
    fn a_renderer_key_keeps_its_kind_and_a_hash_but_never_the_name() {
        let mut m = memory();
        m.parts[1].rows = vec![
            row("renderer:renderer-window-acme-payroll-q3", 900),
            row("renderer:renderer-browser-pane-secret-bank-example", 800),
            row("renderer:renderer-pool-window-idle", 700),
            row("renderer:renderer", 600),
            row("terminal:t1", 500),
        ];
        let am = agentmux_line(&m);
        let keys: Vec<&str> = am.rows.iter().map(|r| r.k.as_str()).collect();
        let json = serde_json::to_string(&am).unwrap();
        for word in ["acme", "payroll", "secret", "bank", "example"] {
            assert!(!json.contains(word), "{word} in {json}");
        }
        let has = |prefix: &str| keys.iter().filter(|k| k.starts_with(prefix)).count();
        assert_eq!((has("renderer:window:"), has("renderer:browser:"), has("renderer:pool:"), has("renderer:other:")), (1, 1, 1, 1), "{keys:?}");
        assert!(keys.contains(&"terminal:t1"), "{keys:?}");
        assert_eq!(keys[0].len(), "renderer:window:".len() + 8, "{keys:?}");
        // The same renderer gives the same key on the next line, and another one a different key.
        assert_eq!(persisted_key("renderer:renderer-window-a"), persisted_key("renderer:renderer-window-a"));
        assert_ne!(persisted_key("renderer:renderer-window-a"), persisted_key("renderer:renderer-window-b"));
        // A key already cut stays as it is.
        let cut = persisted_key("renderer:renderer-window-a");
        assert_eq!(persisted_key(&cut), cut);
    }

    #[test]
    fn a_rollup_of_lines_an_older_version_wrote_carries_no_names_either() {
        let mut old = line(NOON, 1, 1, 10);
        old.am.as_mut().unwrap().rows = vec![PartLine { k: "renderer:renderer-window-acme-payroll".into(), mem: 7, n: 1 }];
        let json = serde_json::to_string(&rollup(NOON, &[old]).unwrap()).unwrap();
        assert!(!json.contains("acme") && !json.contains("payroll"), "{json}");
        assert!(json.contains("renderer:window:"), "{json}");
    }

    #[test]
    fn a_sample_a_little_early_is_not_dropped_but_a_fast_sampler_still_is() {
        let tmp = tempfile::tempdir().unwrap();
        let record = Record::new(tmp.path().join("record"), "0.60.30");
        record.note(&snapshot(NOON));
        record.note(&snapshot(NOON + EVERY_MS - 1)); // the previous one ran long: still the next line
        record.note(&snapshot(NOON + EVERY_MS - 1 + 60_000)); // not yet five minutes after the last
        let today = std::fs::read_to_string(tmp.path().join("record/2026-11-16.jsonl")).unwrap();
        assert_eq!(today.lines().count(), 2);
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

    /// A line at `t` with AgentMux at `total` bytes, its renderer row at `win` and srv up for `up_s`.
    fn line(t: u64, total: u64, win: u64, up_s: u64) -> Sample {
        let mut snap = snapshot(t);
        let m = snap.memory.as_mut().unwrap();
        m.total = total;
        m.parts[1].rows = vec![row("pane:window:main", win)];
        compact(&snap, "0.60.30", up_s, Some((500, 1_000)))
    }

    #[test]
    fn an_hour_is_min_median_p95_and_max() {
        let samples: Vec<Sample> = (1..=12).map(|i| line(NOON + i * EVERY_MS, i * 100, i * 10, i * 300)).collect();
        let r = rollup(NOON, &samples).unwrap();
        assert_eq!((r.n, r.restarts, r.up_s), (12, 0, 3_600));
        assert_eq!(r.am, Some(Stat(100, 600, 1_200, 1_200)));
        assert_eq!(r.rows.iter().find(|(k, _)| k == "pane:window:main").unwrap().1, Stat(10, 60, 120, 120));
        assert_eq!(r.disk_free, Some(Stat(500, 500, 500, 500)));
        assert_eq!(r.cpu, Some(Stat(500, 500, 500, 500)));
        assert_eq!(rollup(NOON, &[]), None);
    }

    #[test]
    fn a_row_missing_from_some_lines_counts_as_zero_there_and_the_largest_rows_are_kept() {
        let mut a = line(NOON, 1, 100, 10);
        a.am.as_mut().unwrap().rows = (0..10).map(|i| PartLine { k: format!("np:p{i}"), mem: i + 1, n: 1 }).collect();
        let mut b = line(NOON + EVERY_MS, 1, 100, 20);
        b.am.as_mut().unwrap().rows = vec![PartLine { k: "np:p9".into(), mem: 50, n: 1 }];
        let r = rollup(NOON, &[a, b]).unwrap();
        assert_eq!(r.rows.len(), ROLLUP_ROWS);
        assert_eq!(r.rows[0], ("np:p9".to_string(), Stat(10, 10, 50, 50)));
        assert_eq!(r.rows[1].0, "np:p8");
        assert_eq!(r.rows[1].1, Stat(0, 0, 9, 9));
    }

    #[test]
    fn a_restart_inside_an_hour_is_counted_by_the_uptime_going_back() {
        let samples = vec![line(NOON, 1, 1, 900), line(NOON + EVERY_MS, 1, 1, 1_200), line(NOON + 2 * EVERY_MS, 1, 1, 40)];
        assert_eq!(rollup(NOON, &samples).unwrap().restarts, 1);
    }

    #[test]
    fn finished_hours_are_rolled_up_once_each_and_the_current_hour_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("record");
        let record = Record::new(dir.clone(), "0.60.30");
        // 12:00 to 14:59 in five-minute lines, then the first line of 15:00.
        for i in 0..=36 {
            record.note(&snapshot(NOON + i * EVERY_MS));
        }
        let text = std::fs::read_to_string(dir.join("rollup-2026-11.jsonl")).unwrap();
        let rolls: Vec<Rollup> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(rolls.iter().map(|r| (r.h - NOON) / HOUR_MS).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(rolls.iter().all(|r| r.n == 12));
        // A new process (srv restarted) finds where it left off and writes nothing twice.
        let again = Record::new(dir.clone(), "0.60.30");
        again.note(&snapshot(NOON + 37 * EVERY_MS));
        assert_eq!(std::fs::read_to_string(dir.join("rollup-2026-11.jsonl")).unwrap().lines().count(), 3);
        // An hour later the 15:00 hour is rolled up.
        again.note(&snapshot(NOON + 49 * EVERY_MS));
        assert_eq!(std::fs::read_to_string(dir.join("rollup-2026-11.jsonl")).unwrap().lines().count(), 4);
    }

    #[test]
    fn a_rollup_is_a_small_line() {
        let samples: Vec<Sample> = (0..12).map(|i| line(NOON + i * EVERY_MS, 3_000_000_000, 1_500_000_000, 1_000_000)).collect();
        let json = serde_json::to_string(&rollup(NOON, &samples).unwrap()).unwrap();
        // A year of hours has to fit ROLLUP_MAX_BYTES with room to spare.
        assert!(json.len() as u64 * 24 * 366 < ROLLUP_MAX_BYTES, "{} bytes: {json}", json.len());
    }

    #[test]
    fn an_empty_newest_month_does_not_reset_the_checkpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("record");
        let record = Record::new(dir.clone(), "0.60.30");
        for i in 0..=24 {
            record.note(&snapshot(NOON + i * EVERY_MS)); // 12:00 to 14:00: the 12:00 and 13:00 hours are rolled
        }
        let full = std::fs::read_to_string(dir.join("rollup-2026-11.jsonl")).unwrap().lines().count();
        assert_eq!(full, 2);
        // The next month's file exists but holds no whole line (a failed first append).
        std::fs::write(dir.join("rollup-2026-12.jsonl"), "{\"h\":").unwrap();
        assert_eq!(last_rollup_hour(&dir), Some(NOON + HOUR_MS));
        let again = Record::new(dir.clone(), "0.60.30");
        again.note(&snapshot(NOON + 25 * EVERY_MS));
        assert_eq!(std::fs::read_to_string(dir.join("rollup-2026-11.jsonl")).unwrap().lines().count(), full);
    }

    #[test]
    fn the_record_is_read_back_by_window_and_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("record");
        let record = Record::new(dir.clone(), "0.60.30");
        for i in 0..=30 {
            record.note(&snapshot(NOON + i * EVERY_MS));
        }
        let all = read_raw(&dir, 0, u64::MAX, 1_000);
        assert_eq!(all.len(), 31);
        // The newest lines of a window, in time order.
        let last3 = read_raw(&dir, NOON + 10 * EVERY_MS, NOON + 20 * EVERY_MS, 3);
        assert_eq!(last3.iter().map(|s| s.t).collect::<Vec<_>>(), [NOON + 18 * EVERY_MS, NOON + 19 * EVERY_MS, NOON + 20 * EVERY_MS]);
        let rolls = read_rollups(&dir, NOON + HOUR_MS, u64::MAX, 10);
        assert_eq!(rolls.iter().map(|r| r.h).collect::<Vec<_>>(), [NOON + HOUR_MS]);
        let inv = inventory(&dir);
        assert_eq!((inv.raw_days, inv.rollup_months), (1, 1));
        assert_eq!((inv.raw_first_day.as_deref(), inv.rollup_first_month.as_deref()), (Some("2026-11-16"), Some("2026-11")));
        assert!(inv.raw_bytes > 0 && inv.rollup_bytes > 0);
        assert_eq!(inventory(&tmp.path().join("nothing")), Inventory::default());
    }

    #[test]
    fn rollup_months_go_after_a_year_or_over_budget_but_never_this_months() {
        let f = |m: &str, size: u64| (format!("rollup-{m}.jsonl"), size);
        let files = vec![f("2025-09", 10), f("2025-12", 10), f("2026-10", 10), f("2026-11", 10), ("2026-11-15.jsonl".into(), 10)];
        // A month that ended more than 366 days before the 16th of November 2026: September 2025 only.
        assert_eq!(rollups_to_remove(&files, "2026-11-16", 366, 1_000), ["rollup-2025-09.jsonl"]);
        assert_eq!(rollups_to_remove(&files, "2026-11-16", 366, 15), ["rollup-2025-09.jsonl", "rollup-2025-12.jsonl", "rollup-2026-10.jsonl"]);
        assert_eq!(rollups_to_remove(&[f("2026-11", 9_999)], "2026-11-16", 366, 1), Vec::<String>::new());
    }
}
