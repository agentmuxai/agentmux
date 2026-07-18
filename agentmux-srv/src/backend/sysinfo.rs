// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sysinfo data collection loop: collects CPU, memory, and network metrics
//! and publishes them via the WPS broker. Sampling interval is configurable
//! via the `telemetry:interval` setting (0.1s–2.0s, default 1.0s).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate};
use tokio::time::MissedTickBehavior;

use crate::backend::blockcontroller::pidregistry;
use crate::backend::blockcontroller::process_tree;
use crate::backend::rpc_types::TimeSeriesData;
use crate::backend::wconfig::ConfigWatcher;
use crate::backend::wps::{Broker, WaveEvent, EVENT_BLOCK_STATS, EVENT_SYS_INFO};

const BYTES_PER_GB: f64 = 1_073_741_824.0;
const BYTES_PER_MB: f64 = 1_048_576.0;
const PERSIST_COUNT: usize = 1024;
const DEFAULT_INTERVAL_SECS: f64 = 1.0;
const MIN_INTERVAL_SECS: f64 = 0.2;
const MAX_INTERVAL_SECS: f64 = 2.0;

// ── Commit (pagefile) attribution ────────────────────────────────────────
// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md: a live OOM
// crash left no forensic trail of WHICH processes were consuming commit —
// only the system-wide total (`get_commit_data`) was ever logged. This
// section adds periodic + urgent-on-pressure attribution, bucketing every
// process on the machine into "agentmux" (this app's own process family —
// host, srv, launcher, CEF helper processes), "panes" (every registered
// block's shell process tree — agent CLI subprocesses, terminal shells,
// etc.; exact PID-tree membership via `pidregistry` + `process_tree`, not
// name-guessing), and "other" (everything else, top-N by commit kept for
// visibility — Chrome, VS Code, Docker, Windows itself, ...).

/// How often to log a full attribution snapshot in the steady state.
const ATTRIBUTION_INTERVAL: Duration = Duration::from_secs(30);
/// Re-snapshot immediately (bypassing `ATTRIBUTION_INTERVAL`) once available
/// commit drops below this — a snapshot taken seconds before a crash is far
/// more useful than one from up to 30s earlier.
const ATTRIBUTION_URGENT_THRESHOLD_GB: f64 = 2.0;
/// Debounce for the urgent trigger so a sustained low-commit period doesn't
/// log every single tick.
const ATTRIBUTION_URGENT_COOLDOWN: Duration = Duration::from_secs(10);
/// How many "other" processes to name individually in the log line.
const ATTRIBUTION_OTHER_TOP_N: usize = 5;
/// Per-block descendant-PID cap for the attribution walk — deliberately its
/// OWN, much larger constant, not `process_tree::MAX_PIDS_PER_BLOCK` (64).
/// That cap is sized for the Sysinfo pane's small per-block CPU/mem display;
/// reusing it here silently dropped overflow into the "other" bucket
/// instead of "panes" in exactly the storm scenario this feature exists to
/// diagnose (a single block's descendant tree exceeding 64 processes) —
/// reagent P1 on PR #2207. `collect_descendants` gives no truncation signal
/// beyond "returned exactly `max_pids` results," so that's what
/// `log_memory_attribution` checks to log a truncation warning rather than
/// silently under-counting (no silent caps).
const ATTRIBUTION_MAX_PIDS_PER_BLOCK: usize = 4096;

/// One process's classification input — commit charge, not working set:
/// what actually exhausts the pagefile. (sysinfo's `virtual_memory()` on
/// Windows is `PROCESS_MEMORY_COUNTERS_EX::PrivateUsage` — a misleading
/// name from sysinfo's cross-platform API, but the right number: the
/// process's private/committed bytes.)
struct ProcSample {
    pid: u32,
    name: String,
    commit_mb: f64,
}

struct AttributionResult {
    agentmux_mb: f64,
    panes_mb: f64,
    other_mb: f64,
    /// Top `top_n` "other" processes by commit, descending.
    other_top: Vec<(String, f64)>,
}

/// Pure classification, factored out for unit testing without a real
/// process table (mirrors `resolve_pagefile_watch_target` above).
/// `pane_pids` is the exact PID set `pidregistry`'s block trees computed —
/// membership, not a name guess, so this bucket can never miscount a
/// same-named-but-unrelated process.
fn classify_commit_attribution(
    samples: &[ProcSample],
    pane_pids: &HashSet<u32>,
    top_n: usize,
) -> AttributionResult {
    let mut agentmux_mb = 0.0;
    let mut panes_mb = 0.0;
    let mut other: Vec<(String, f64)> = Vec::new();
    for s in samples {
        if s.name.to_ascii_lowercase().starts_with("agentmux") {
            agentmux_mb += s.commit_mb;
        } else if pane_pids.contains(&s.pid) {
            panes_mb += s.commit_mb;
        } else {
            other.push((s.name.clone(), s.commit_mb));
        }
    }
    other.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let other_mb: f64 = other.iter().map(|(_, mb)| mb).sum();
    other.truncate(top_n);
    AttributionResult {
        agentmux_mb,
        panes_mb,
        other_mb,
        other_top: other,
    }
}

/// Full-system commit snapshot: refresh every process (heavier than the
/// per-block passes above, but only ever called on the attribution
/// cadence — every 30s in steady state, or debounced-urgent under
/// pressure), classify, and log one line. Independent of the per-tick
/// block-stats pass above — it does its own PID-tree collection so it
/// can't be affected by that pass's ordering.
fn log_memory_attribution(sys: &mut sysinfo::System) {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true, // remove dead processes — keep this bounded to what's live
        ProcessRefreshKind::everything(),
    );

    let mut pane_pids: HashSet<u32> = HashSet::new();
    for (block_id, pid) in pidregistry::get_all() {
        let tree = process_tree::collect_descendants(
            sys,
            Pid::from(pid as usize),
            ATTRIBUTION_MAX_PIDS_PER_BLOCK,
        );
        if tree.len() == ATTRIBUTION_MAX_PIDS_PER_BLOCK {
            tracing::warn!(
                block_id = %block_id,
                cap = ATTRIBUTION_MAX_PIDS_PER_BLOCK,
                "log_memory_attribution: block's descendant PID tree hit the cap — \
                 some processes may be misclassified into \"other\" instead of \"panes\""
            );
        }
        pane_pids.extend(tree.into_iter().map(|pid| pid.as_u32()));
    }

    let samples: Vec<ProcSample> = sys
        .processes()
        .iter()
        .map(|(pid, proc)| ProcSample {
            pid: pid.as_u32(),
            name: proc.name().to_string_lossy().into_owned(),
            commit_mb: proc.virtual_memory() as f64 / BYTES_PER_MB,
        })
        .collect();

    let result = classify_commit_attribution(&samples, &pane_pids, ATTRIBUTION_OTHER_TOP_N);
    let (commit_used_gb, commit_total_gb) = {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::SystemInformation::{
                GlobalMemoryStatusEx, MEMORYSTATUSEX,
            };
            let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
            mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 {
                let total_gb = mem.ullTotalPageFile as f64 / BYTES_PER_GB;
                let avail_gb = mem.ullAvailPageFile as f64 / BYTES_PER_GB;
                (total_gb - avail_gb, total_gb)
            } else {
                (0.0, 0.0)
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            (0.0, 0.0)
        }
    };

    let other_top_str = result
        .other_top
        .iter()
        .map(|(name, mb)| format!("{}:{:.0}MB", name, mb))
        .collect::<Vec<_>>()
        .join(", ");

    tracing::info!(
        target: "mem_attribution",
        commit_used_gb = format!("{:.2}", commit_used_gb),
        commit_total_gb = format!("{:.2}", commit_total_gb),
        agentmux_mb = format!("{:.0}", result.agentmux_mb),
        panes_mb = format!("{:.0}", result.panes_mb),
        other_mb = format!("{:.0}", result.other_mb),
        other_top = %other_top_str,
        process_count = samples.len(),
        "commit attribution"
    );
}

/// Collect CPU usage (total + per-core).
fn get_cpu_data(sys: &sysinfo::System, values: &mut HashMap<String, f64>) {
    let cpus = sys.cpus();
    if cpus.is_empty() {
        return;
    }
    // Total CPU usage (average across all cores)
    let total: f64 = cpus.iter().map(|c| c.cpu_usage() as f64).sum::<f64>() / cpus.len() as f64;
    values.insert("cpu".to_string(), total);
    // Per-core usage
    for (idx, cpu) in cpus.iter().enumerate() {
        values.insert(format!("cpu:{}", idx), cpu.cpu_usage() as f64);
    }
}

/// Collect memory metrics (in GB).
fn get_mem_data(sys: &sysinfo::System, values: &mut HashMap<String, f64>) {
    let total = sys.total_memory() as f64 / BYTES_PER_GB;
    let used = sys.used_memory() as f64 / BYTES_PER_GB;
    let available = sys.available_memory() as f64 / BYTES_PER_GB;
    let free = sys.free_memory() as f64 / BYTES_PER_GB;
    values.insert("mem:total".to_string(), total);
    values.insert("mem:used".to_string(), used);
    values.insert("mem:available".to_string(), available);
    values.insert("mem:free".to_string(), free);
}

/// Read the Windows commit budget via `GlobalMemoryStatusEx`.
/// Emits `mem:commit:used` and `mem:commit:total` (in GB).
/// No-op on non-Windows — keys are simply absent from the payload.
fn get_commit_data(_values: &mut HashMap<String, f64>) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 {
            let total_gb = mem.ullTotalPageFile as f64 / BYTES_PER_GB;
            let avail_gb = mem.ullAvailPageFile as f64 / BYTES_PER_GB;
            _values.insert("mem:commit:used".to_string(), total_gb - avail_gb);
            _values.insert("mem:commit:total".to_string(), total_gb);
        }
    }
}

/// Available system commit headroom in GB (Windows: `GlobalMemoryStatusEx`
/// → `ullAvailPageFile`). Returns `None` where there's no cheap commit figure
/// (non-Windows) or the read fails — callers treat `None` as "no limit / admit".
/// This is the admission-control signal for Pillar 3 (commit-aware agent spawn):
/// see `agents::runner::admit_spawn` and `SPEC_WIN10_PAGEFILE_OOM_CRASH`.
pub fn available_commit_gb() -> Option<f64> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 {
            return Some(mem.ullAvailPageFile as f64 / BYTES_PER_GB);
        }
    }
    None
}

// ── SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2 P0 ────────────────────────
// "Track free disk on the pagefile volume, not just avail_page_gb. Warn when
// free disk < ~15% and page file is system-managed." The existing commit
// gauge above only sees the SYMPTOM (commit near limit); it is blind to the
// CAUSE this spec found: a system-managed page file wants to grow toward
// min(3×RAM, ⅛ volume) but silently cannot if the volume it lives on doesn't
// have the free space, pinning the commit ceiling well below what every
// other gauge assumes. This makes the cause itself visible.

/// One `PagingFiles` registry entry
/// (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management`),
/// parsed from its `"<path> <initial_mb> <maximum_mb>"` string form. Per
/// [Microsoft's documented format](https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/how-to-determine-the-appropriate-page-file-size-for-64-bit-versions-of-windows):
/// `initial_mb == 0 && maximum_mb == 0` means "system managed" (auto-grow)
/// for that specific pagefile.
#[derive(Debug, Clone, PartialEq)]
struct PagingFileEntry {
    /// Drive letter the pagefile lives on, uppercased, e.g. `"C"`. `None` if
    /// the path couldn't be parsed as `<letter>:\...` (UNC path, malformed).
    drive_letter: Option<char>,
    system_managed: bool,
}

/// Parse one raw `"<path> <initial_mb> <maximum_mb>"` line from the
/// `PagingFiles` REG_MULTI_SZ value. Tolerant of the format's use of `??`
/// instead of a drive letter for some `\\?\Volume{guid}\` forms — those
/// yield `drive_letter: None` rather than erroring, since a pure parser
/// should never fail on a value the OS itself produced.
fn parse_paging_file_entry(line: &str) -> Option<PagingFileEntry> {
    let mut parts = line.split_whitespace();
    let path = parts.next()?;
    let initial: u64 = parts.next()?.parse().ok()?;
    let maximum: u64 = parts.next()?.parse().ok()?;
    let drive_letter = path
        .chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase());
    Some(PagingFileEntry {
        drive_letter,
        system_managed: initial == 0 && maximum == 0,
    })
}

/// Decide which volume to watch and whether it's at risk of a stuck
/// (non-growing) page file, from the parsed `PagingFiles` entries.
///
/// - **Empty list** (the common default): Windows fully automatically
///   manages a single pagefile's size AND location — not expressed as any
///   registry entry at all. Falls back to `system_drive` as the practical
///   answer (where CEF/the OS itself lives; the overwhelmingly likely
///   pagefile location in this configuration) — `system_managed: true`.
/// - **Has entries**: watch the first volume with a `system_managed: true`
///   entry (the volume that can actually run out of the growth room this
///   spec is about). If every entry is fixed-size, no volume is at growth
///   risk from disk space — report `system_managed: false` on
///   `system_drive` for visibility, but the frontend must not apply the
///   "can't grow" risk framing to a fixed-size pagefile.
fn resolve_pagefile_watch_target(entries: &[PagingFileEntry], system_drive: char) -> (char, bool) {
    if entries.is_empty() {
        return (system_drive, true);
    }
    for e in entries {
        if e.system_managed {
            return (e.drive_letter.unwrap_or(system_drive), true);
        }
    }
    (system_drive, false)
}

/// Read `PagingFiles` and split it into entries. Non-Windows / read failure
/// returns an empty vec (caller's `resolve_pagefile_watch_target` then
/// degrades to "watch the system drive as if system-managed" — the correct
/// fail-open default: assume the common case rather than silently reporting
/// nothing).
#[cfg(target_os = "windows")]
fn read_paging_files_registry() -> Vec<PagingFileEntry> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_MULTI_SZ,
    };

    let subkey: Vec<u16> =
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Memory Management\0"
            .encode_utf16()
            .collect();
    let value_name: Vec<u16> = "PagingFiles\0".encode_utf16().collect();

    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey.as_ptr(), 0, KEY_READ, &mut hkey) as u32
            != ERROR_SUCCESS
        {
            return Vec::new();
        }

        let mut buf_bytes: u32 = 0;
        let mut value_type: u32 = 0;
        let query1 = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut buf_bytes,
        );
        if query1 as u32 != ERROR_SUCCESS || value_type != REG_MULTI_SZ || buf_bytes == 0 {
            RegCloseKey(hkey);
            return Vec::new();
        }

        let mut buf: Vec<u16> = vec![0u16; buf_bytes as usize / 2];
        let query2 = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut u8,
            &mut buf_bytes,
        );
        RegCloseKey(hkey);
        if query2 as u32 != ERROR_SUCCESS {
            return Vec::new();
        }

        // REG_MULTI_SZ: NUL-separated strings, terminated by a double NUL.
        buf.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| parse_paging_file_entry(&String::from_utf16_lossy(s)))
            .collect()
    }
}

#[cfg(not(target_os = "windows"))]
fn read_paging_files_registry() -> Vec<PagingFileEntry> {
    Vec::new()
}

/// Free/total space (GB) for a drive letter, e.g. `'C'`. Windows-only;
/// returns `None` on any failure (drive not found, API error) so callers
/// degrade to "no data" rather than a wrong number.
#[cfg(target_os = "windows")]
fn drive_free_total_gb(drive_letter: char) -> Option<(f64, f64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root: Vec<u16> = format!("{}:\\\0", drive_letter).encode_utf16().collect();
    let mut free_available: u64 = 0;
    let mut total: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            root.as_ptr(),
            &mut free_available,
            &mut total,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    Some((
        free_available as f64 / BYTES_PER_GB,
        total as f64 / BYTES_PER_GB,
    ))
}

/// The `(drive_letter, system_managed)` pagefile watch target, resolved
/// once per process lifetime. `PagingFiles` is machine configuration that
/// requires a reboot to take effect — re-reading the registry every
/// sysinfo tick (default 1 Hz) would be pure waste. Free disk space, by
/// contrast, genuinely changes during a session and is re-read every tick
/// in `get_pagefile_volume_data`.
#[cfg(target_os = "windows")]
fn pagefile_watch_target() -> (char, bool) {
    static TARGET: std::sync::OnceLock<(char, bool)> = std::sync::OnceLock::new();
    *TARGET.get_or_init(|| {
        let system_drive = std::env::var("SystemDrive")
            .ok()
            .and_then(|s| s.chars().next())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('C');
        let entries = read_paging_files_registry();
        resolve_pagefile_watch_target(&entries, system_drive)
    })
}

/// Collect pagefile-volume disk tracking: `disk:pagefile_volume:free_gb`,
/// `:total_gb`, `:free_pct`, and `disk:pagefile_system_managed` (1.0/0.0).
/// No-op (keys absent) on non-Windows or any read failure — matches
/// `get_commit_data`'s fail-open convention.
fn get_pagefile_volume_data(_values: &mut HashMap<String, f64>) {
    #[cfg(target_os = "windows")]
    {
        let (drive, system_managed) = pagefile_watch_target();
        if let Some((free_gb, total_gb)) = drive_free_total_gb(drive) {
            _values.insert("disk:pagefile_volume:free_gb".to_string(), free_gb);
            _values.insert("disk:pagefile_volume:total_gb".to_string(), total_gb);
            if total_gb > 0.0 {
                _values.insert(
                    "disk:pagefile_volume:free_pct".to_string(),
                    (free_gb / total_gb) * 100.0,
                );
            }
            _values.insert(
                "disk:pagefile_system_managed".to_string(),
                if system_managed { 1.0 } else { 0.0 },
            );
        }
    }
}

#[cfg(test)]
mod pagefile_volume_tests {
    use super::*;

    #[test]
    fn parses_system_managed_entry() {
        let e = parse_paging_file_entry("C:\\pagefile.sys 0 0").unwrap();
        assert_eq!(e.drive_letter, Some('C'));
        assert!(e.system_managed);
    }

    #[test]
    fn parses_fixed_size_entry() {
        let e = parse_paging_file_entry("D:\\pagefile.sys 4096 8192").unwrap();
        assert_eq!(e.drive_letter, Some('D'));
        assert!(!e.system_managed);
    }

    #[test]
    fn lowercases_drive_letter_normalized_to_upper() {
        // Windows can report either case; the registry doesn't enforce one.
        let e = parse_paging_file_entry("c:\\pagefile.sys 0 0").unwrap();
        assert_eq!(e.drive_letter, Some('C'));
    }

    #[test]
    fn malformed_line_returns_none() {
        assert!(parse_paging_file_entry("").is_none());
        assert!(parse_paging_file_entry("C:\\pagefile.sys").is_none());
        assert!(parse_paging_file_entry("C:\\pagefile.sys notanumber 0").is_none());
    }

    #[test]
    fn empty_entries_falls_back_to_system_drive_as_managed() {
        // The common default: no explicit PagingFiles entries at all means
        // Windows fully auto-manages a single pagefile — not expressed as a
        // registry entry, so an empty list must NOT be read as "no pagefile
        // risk" (the opposite of the truth).
        let (drive, managed) = resolve_pagefile_watch_target(&[], 'C');
        assert_eq!(drive, 'C');
        assert!(managed);
    }

    #[test]
    fn watches_the_first_system_managed_volume() {
        let entries = vec![
            PagingFileEntry { drive_letter: Some('D'), system_managed: false },
            PagingFileEntry { drive_letter: Some('E'), system_managed: true },
        ];
        let (drive, managed) = resolve_pagefile_watch_target(&entries, 'C');
        assert_eq!(drive, 'E');
        assert!(managed);
    }

    #[test]
    fn all_fixed_size_reports_not_managed_on_system_drive() {
        // No volume is at "can't grow" risk — but we still surface a
        // number (system drive) for visibility, correctly flagged as not
        // subject to this specific risk.
        let entries = vec![PagingFileEntry { drive_letter: Some('D'), system_managed: false }];
        let (drive, managed) = resolve_pagefile_watch_target(&entries, 'C');
        assert_eq!(drive, 'C');
        assert!(!managed);
    }
}

#[cfg(test)]
mod commit_attribution_tests {
    use super::*;

    fn sample(pid: u32, name: &str, commit_mb: f64) -> ProcSample {
        ProcSample { pid, name: name.to_string(), commit_mb }
    }

    #[test]
    fn buckets_agentmux_by_name_regardless_of_pane_membership() {
        // Name match wins even if (hypothetically) a PID also showed up in
        // a pane's descendant tree — AgentMux's own processes are never
        // "pane" work, no matter how the tree walk found them.
        let samples = vec![sample(100, "agentmux-cef.exe", 500.0)];
        let mut pane_pids = HashSet::new();
        pane_pids.insert(100);
        let result = classify_commit_attribution(&samples, &pane_pids, 5);
        assert_eq!(result.agentmux_mb, 500.0);
        assert_eq!(result.panes_mb, 0.0);
    }

    #[test]
    fn buckets_pane_pids_by_membership_not_name() {
        // A pane's process tree (e.g. node.exe running Claude Code) is
        // classified by exact PID membership, not by guessing its name.
        let samples = vec![sample(200, "node.exe", 300.0), sample(201, "conhost.exe", 10.0)];
        let mut pane_pids = HashSet::new();
        pane_pids.insert(200);
        pane_pids.insert(201);
        let result = classify_commit_attribution(&samples, &pane_pids, 5);
        assert_eq!(result.panes_mb, 310.0);
        assert_eq!(result.agentmux_mb, 0.0);
        assert!(result.other_top.is_empty());
    }

    #[test]
    fn everything_else_falls_to_other_ranked_and_capped() {
        let samples = vec![
            sample(1, "chrome.exe", 800.0),
            sample(2, "Code.exe", 600.0),
            sample(3, "Docker Desktop.exe", 1200.0),
            sample(4, "explorer.exe", 150.0),
        ];
        let result = classify_commit_attribution(&samples, &HashSet::new(), 2);
        // Total is preserved even though the printed list is capped.
        assert_eq!(result.other_mb, 800.0 + 600.0 + 1200.0 + 150.0);
        // Only the top 2 (by commit) are kept for the log line, descending.
        assert_eq!(result.other_top.len(), 2);
        assert_eq!(result.other_top[0].0, "Docker Desktop.exe");
        assert_eq!(result.other_top[1].0, "chrome.exe");
    }

    #[test]
    fn name_match_is_case_insensitive() {
        let samples = vec![sample(1, "AgentMux-Srv.exe", 42.0)];
        let result = classify_commit_attribution(&samples, &HashSet::new(), 5);
        assert_eq!(result.agentmux_mb, 42.0);
    }

    #[test]
    fn empty_process_list_yields_zeroed_result() {
        let result = classify_commit_attribution(&[], &HashSet::new(), 5);
        assert_eq!(result.agentmux_mb, 0.0);
        assert_eq!(result.panes_mb, 0.0);
        assert_eq!(result.other_mb, 0.0);
        assert!(result.other_top.is_empty());
    }

    /// Runs the real Win32 process-enumeration + `GlobalMemoryStatusEx` path
    /// against this machine's actual process table — the synthetic-data
    /// tests above cover classification logic but can't catch a bad
    /// Win32 API call, an access-denied edge case, or a panic on a real
    /// process's field values. This test's own process is guaranteed to
    /// exist in the table with a real name and non-zero commit, so a
    /// classification bucket is exercised end-to-end for real.
    #[test]
    fn log_memory_attribution_runs_against_the_real_process_table_without_panicking() {
        let mut sys = sysinfo::System::new_all();
        log_memory_attribution(&mut sys); // must not panic
        assert!(
            sys.processes().len() > 1,
            "expected the real process table to have picked up more than just this test binary"
        );
    }
}

/// Network I/O tracking state for rate calculations.
struct NetState {
    prev_sent: u64,
    prev_recv: u64,
    prev_time: Option<Instant>,
}

impl NetState {
    fn new() -> Self {
        Self {
            prev_sent: 0,
            prev_recv: 0,
            prev_time: None,
        }
    }

    /// Collect network I/O rates (in MB/s).
    fn get_net_data(&mut self, networks: &Networks, values: &mut HashMap<String, f64>) {
        // Sum across all interfaces
        let mut total_sent: u64 = 0;
        let mut total_recv: u64 = 0;
        for (_name, data) in networks.iter() {
            total_sent += data.total_transmitted();
            total_recv += data.total_received();
        }

        let now = Instant::now();
        if let Some(prev_time) = self.prev_time {
            let elapsed = now.duration_since(prev_time).as_secs_f64();
            if elapsed > 0.0 {
                let sent_rate = (total_sent.saturating_sub(self.prev_sent)) as f64 / elapsed / BYTES_PER_MB;
                let recv_rate = (total_recv.saturating_sub(self.prev_recv)) as f64 / elapsed / BYTES_PER_MB;
                values.insert("net:bytessent".to_string(), sent_rate);
                values.insert("net:bytesrecv".to_string(), recv_rate);
                values.insert("net:bytestotal".to_string(), sent_rate + recv_rate);
            }
        }

        self.prev_sent = total_sent;
        self.prev_recv = total_recv;
        self.prev_time = Some(now);
    }
}

/// Collect disk I/O rates (in MB/s).
/// sysinfo Disk::usage() returns deltas (bytes since last refresh) so we
/// divide by elapsed time to get rates.
fn get_disk_data(disks: &Disks, elapsed_secs: f64, values: &mut HashMap<String, f64>) {
    if elapsed_secs <= 0.0 {
        return;
    }
    let (total_read, total_write) = disks.list().iter().fold((0u64, 0u64), |(r, w), disk| {
        let u = disk.usage();
        (r + u.read_bytes, w + u.written_bytes)
    });
    let read_rate = total_read as f64 / elapsed_secs / BYTES_PER_MB;
    let write_rate = total_write as f64 / elapsed_secs / BYTES_PER_MB;
    values.insert("disk:read".to_string(), read_rate);
    values.insert("disk:write".to_string(), write_rate);
    values.insert("disk:total".to_string(), read_rate + write_rate);
}

/// Read the telemetry interval from config, clamped to [MIN, MAX].
fn get_interval_secs(config_watcher: &ConfigWatcher) -> f64 {
    let val = config_watcher.get_settings().telemetry_interval;
    if val <= 0.0 {
        return DEFAULT_INTERVAL_SECS;
    }
    val.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)
}

/// Run the sysinfo collection loop. Uses `tokio::time::interval` for steady
/// tick rate regardless of refresh duration. Interval is re-read from config
/// each tick and the timer is reset if it changes.
pub async fn run_sysinfo_loop(broker: Arc<Broker>, config_watcher: Arc<ConfigWatcher>, conn_name: String) {
    let mut sys = sysinfo::System::new_all();
    let mut networks = Networks::new_with_refreshed_list();
    let mut net_state = NetState::new();
    let mut disks = Disks::new_with_refreshed_list();
    let mut last_tick = Instant::now();
    // Commit-attribution cadence — see the "Commit (pagefile) attribution"
    // section above. Starts due-now-minus-interval so the first snapshot
    // establishes a baseline shortly after startup, not 30s of silence.
    let mut last_attribution = Instant::now()
        .checked_sub(ATTRIBUTION_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_urgent_attribution: Option<Instant> = None;

    let mut current_interval = get_interval_secs(&config_watcher);
    let mut ticker = tokio::time::interval(Duration::from_secs_f64(current_interval));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Skip the first immediate tick
    ticker.tick().await;

    tracing::info!("sysinfo loop started for conn:{}", conn_name);

    loop {
        ticker.tick().await;

        // Check if interval changed and reset ticker if so
        let new_interval = get_interval_secs(&config_watcher);
        if (new_interval - current_interval).abs() > 0.001 {
            current_interval = new_interval;
            ticker = tokio::time::interval(Duration::from_secs_f64(current_interval));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            ticker.tick().await; // consume immediate first tick
            tracing::info!("sysinfo interval changed to {}s", current_interval);
        }

        // Refresh all metrics. All sysinfo refresh calls are synchronous
        // /proc reads; block_in_place signals the Tokio runtime to keep
        // other tasks (including terminal echo) running on other threads
        // while this thread is occupied — preventing 1Hz echo starvation.
        //
        // `get_pagefile_volume_data`'s `GetDiskFreeSpaceExW` call (reagent
        // P1, PR #2109) is a synchronous Win32 disk I/O syscall — the exact
        // hazard this block exists to isolate, unlike `get_commit_data`'s
        // `GlobalMemoryStatusEx` (pure in-memory, no filesystem driver
        // involved) — so its result is computed here too, alongside the
        // other blocking OS reads, not in the async section below.
        let mut pagefile_values = HashMap::new();
        tokio::task::block_in_place(|| {
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            networks.refresh(true);
            disks.refresh(true);
            get_pagefile_volume_data(&mut pagefile_values);
        });

        let now_instant = Instant::now();
        let elapsed_secs = now_instant.duration_since(last_tick).as_secs_f64();
        last_tick = now_instant;

        let mut values = pagefile_values;
        get_cpu_data(&sys, &mut values);
        get_mem_data(&sys, &mut values);
        get_commit_data(&mut values);
        net_state.get_net_data(&networks, &mut values);
        get_disk_data(&disks, elapsed_secs, &mut values);

        // Commit attribution: periodic in the steady state, or immediately
        // (debounced) when commit heads toward exhaustion — see
        // docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md. Reuses
        // the commit numbers just computed above rather than re-querying.
        let commit_avail_gb = match (values.get("mem:commit:total"), values.get("mem:commit:used")) {
            (Some(total), Some(used)) => Some(total - used),
            _ => None,
        };
        let due_periodic = now_instant.duration_since(last_attribution) >= ATTRIBUTION_INTERVAL;
        let due_urgent = commit_avail_gb.is_some_and(|gb| gb < ATTRIBUTION_URGENT_THRESHOLD_GB)
            && last_urgent_attribution
                .is_none_or(|t| now_instant.duration_since(t) >= ATTRIBUTION_URGENT_COOLDOWN);
        if due_periodic || due_urgent {
            tokio::task::block_in_place(|| log_memory_attribution(&mut sys));
            last_attribution = now_instant;
            if due_urgent {
                last_urgent_attribution = Some(now_instant);
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let ts_data = TimeSeriesData { ts: now, values };

        let event = WaveEvent {
            event: EVENT_SYS_INFO.to_string(),
            scopes: vec![conn_name.clone()],
            sender: String::new(),
            persist: PERSIST_COUNT,
            data: serde_json::to_value(&ts_data).ok(),
        };

        broker.publish(event);

        // Per-pane process tree metrics: aggregate CPU/mem across each block's
        // shell process and all its descendants.
        let block_pids = pidregistry::get_all();
        if !block_pids.is_empty() {
            // Pass 1: cheap minimal refresh of all processes to populate parent()
            // links. ProcessRefreshKind::nothing() skips CPU/mem — just PID/PPID.
            // On a VM with many Chromium/CEF helper processes this can take 5–20ms;
            // block_in_place keeps the Tokio runtime unblocked during the scan.
            tokio::task::block_in_place(|| {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    false, // keep stale entries — pass 2 removes dead ones
                    ProcessRefreshKind::nothing(),
                );
            });

            // For each block, BFS the process tree from the shell PID.
            let mut block_trees: Vec<(String, Vec<Pid>)> = block_pids
                .iter()
                .map(|(block_id, pid)| {
                    let root = Pid::from(*pid as usize);
                    let tree = process_tree::collect_descendants(
                        &sys,
                        root,
                        process_tree::MAX_PIDS_PER_BLOCK,
                    );
                    (block_id.clone(), tree)
                })
                .collect();

            // Pass 2: targeted deep refresh (CPU + mem) for only the PIDs we care about.
            // Deduplicate across blocks so each PID is refreshed at most once.
            let mut all_pids: Vec<Pid> = block_trees
                .iter()
                .flat_map(|(_, pids)| pids.iter().copied())
                .collect();
            all_pids.sort_unstable();
            all_pids.dedup();
            tokio::task::block_in_place(|| {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&all_pids),
                    true, // remove dead processes on this authoritative pass
                    ProcessRefreshKind::everything(),
                );
            });

            // Aggregate per block and publish.
            // After Pass 2 (remove_dead=true), sys.process() returns None for any
            // PID that no longer exists — use this to detect orphaned registry entries.
            let mut dead_block_ids: Vec<String> = Vec::new();

            for (block_id, pids) in &mut block_trees {
                // collect_descendants() always puts the root PID first.
                let root_pid = pids.first().copied().unwrap_or(Pid::from(0usize));
                let mut total_cpu: f64 = 0.0;
                let mut total_mem: u64 = 0;
                let mut live_count: u32 = 0;

                for pid in pids.iter() {
                    if let Some(proc) = sys.process(*pid) {
                        total_cpu += proc.cpu_usage() as f64;
                        total_mem += proc.memory();
                        live_count += 1;
                    }
                }

                // Root process is gone — evict from registry.  This is the last-resort
                // cleanup for processes that exit without normal wait-task teardown
                // (SIGKILL by the OS, unexpected crash, or stop() race).
                if sys.process(root_pid).is_none() {
                    dead_block_ids.push(block_id.clone());
                    continue; // skip publishing stale stats for a dead block
                }

                let mut block_values = HashMap::new();
                block_values.insert("cpu".to_string(), total_cpu);
                block_values.insert("mem".to_string(), total_mem as f64);
                block_values.insert("pids".to_string(), live_count as f64);

                let block_ts = TimeSeriesData {
                    ts: now,
                    values: block_values,
                };
                let block_event = WaveEvent {
                    event: EVENT_BLOCK_STATS.to_string(),
                    scopes: vec![format!("block:{}", block_id)],
                    sender: String::new(),
                    persist: 0,
                    data: serde_json::to_value(&block_ts).ok(),
                };
                broker.publish(block_event);
            }

            for block_id in &dead_block_ids {
                pidregistry::unregister(block_id);
                tracing::warn!(
                    block_id = %block_id,
                    "sysinfo: evicted dead root PID — process exited without normal cleanup"
                );
            }
        }
    }
}
