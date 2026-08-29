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

/// Milliseconds from a SUSPEND-AWARE monotonic clock — one that keeps counting
/// while the machine is asleep, and can never move backwards.
///
/// Uptime needs both properties, and no single obvious clock has them:
///
/// - Wall clock (`SystemTime`/`chrono::Utc::now()`) counts suspend but moves
///   backwards on an NTP correction, a manual set, or a VM resume. That was
///   the original bug: on 0.55.26 the app started while the machine's clock
///   read 2081-02-05, the clock was later corrected to 2026-08-29 with no
///   restart, and the status bar rendered `-59:0-14`.
/// - `std::time::Instant` can't move backwards, but it STOPS while the machine
///   is suspended (`CLOCK_MONOTONIC` on Linux, `mach_absolute_time` on macOS,
///   QPC on Windows). Using it would silently under-report by the whole
///   suspend interval on every laptop lid-close — a regression against the
///   wall-clock behaviour it replaced (codex P2 on PR #2831).
///
/// So each platform's suspend-aware counter is used directly:
///
/// | Platform | Source | Counts suspend |
/// |---|---|---|
/// | Linux/Android | `CLOCK_BOOTTIME` | yes (`CLOCK_MONOTONIC` does not) |
/// | macOS/BSD | `CLOCK_MONOTONIC` | yes — Darwin's continues across sleep, unlike `CLOCK_UPTIME_RAW` and unlike the `mach_absolute_time` behind `Instant` |
/// | Windows | `GetTickCount64` | yes (`QueryUnbiasedInterruptTime` does not) |
///
/// All three count from system boot, not process start, so `mark_process_start`
/// takes a baseline and `uptime_secs` subtracts it.
#[cfg(windows)]
fn suspend_aware_now_ms() -> u64 {
    // SAFETY: no arguments, no pointers, always succeeds.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(not(windows))]
fn suspend_aware_now_ms() -> u64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;

    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, initialized, exclusively-borrowed timespec.
    if unsafe { libc::clock_gettime(CLOCK, &mut ts) } != 0 {
        // Reporting 0 makes uptime read 0 — visibly wrong but harmless and
        // never negative. Preferable to substituting a clock with different
        // properties partway through the process's life.
        return 0;
    }
    (ts.tv_sec as u64) * 1_000 + (ts.tv_nsec as u64) / 1_000_000
}

/// Baseline reading of `suspend_aware_now_ms` captured at process start by
/// `mark_process_start` (`main.rs`).
static PROCESS_START_MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

/// Record this process's start for uptime reporting. Idempotent; the first
/// call wins, so a stray second call can't restart the clock.
pub fn mark_process_start() {
    let _ = PROCESS_START_MS.set(suspend_aware_now_ms());
}

/// Seconds since `mark_process_start`. Falls back to initializing on first
/// read, so a caller that never marked startup reports a truthful (if
/// late-based) 0 rather than a wrong number or a panic. `saturating_sub`
/// makes a decrease structurally impossible even if a platform counter ever
/// misbehaved.
pub fn uptime_secs() -> u64 {
    let start = *PROCESS_START_MS.get_or_init(suspend_aware_now_ms);
    suspend_aware_now_ms().saturating_sub(start) / 1_000
}

/// How often to log a full attribution snapshot in the steady state.
#[cfg(target_os = "windows")]
const ATTRIBUTION_INTERVAL: Duration = Duration::from_secs(30);
/// Re-snapshot immediately (bypassing `ATTRIBUTION_INTERVAL`) once available
/// commit drops below this — a snapshot taken seconds before a crash is far
/// more useful than one from up to 30s earlier.
#[cfg(target_os = "windows")]
const ATTRIBUTION_URGENT_THRESHOLD_GB: f64 = 2.0;
/// Debounce for the urgent trigger so a sustained low-commit period doesn't
/// log every single tick.
#[cfg(target_os = "windows")]
const ATTRIBUTION_URGENT_COOLDOWN: Duration = Duration::from_secs(10);
/// How many "other" processes to name individually in the log line.
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
const ATTRIBUTION_MAX_PIDS_PER_BLOCK: usize = 4096;

/// One process's classification input — commit charge, not working set:
/// what actually exhausts the pagefile. (sysinfo's `virtual_memory()` on
/// Windows is `PROCESS_MEMORY_COUNTERS_EX::PrivateUsage` — a misleading
/// name from sysinfo's cross-platform API, but the right number: the
/// process's private/committed bytes.)
#[cfg(target_os = "windows")]
struct ProcSample {
    pid: u32,
    name: String,
    commit_mb: f64,
}

#[cfg(target_os = "windows")]
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
/// same-named-but-unrelated process. Aggregates "other" by process name
/// before ranking the top-N — a multi-process app (Chrome, VS Code) would
/// otherwise get split across many small per-PID entries that individually
/// miss the cutoff while collectively dominating `other_mb`, hiding the
/// true top contributor (reagent P1 on PR #2207).
#[cfg(target_os = "windows")]
fn classify_commit_attribution(
    samples: &[ProcSample],
    pane_pids: &HashSet<u32>,
    top_n: usize,
) -> AttributionResult {
    let mut agentmux_mb = 0.0;
    let mut panes_mb = 0.0;
    let mut other_by_name: HashMap<String, f64> = HashMap::new();
    for s in samples {
        if s.name.to_ascii_lowercase().starts_with("agentmux") {
            agentmux_mb += s.commit_mb;
        } else if pane_pids.contains(&s.pid) {
            panes_mb += s.commit_mb;
        } else {
            *other_by_name.entry(s.name.clone()).or_insert(0.0) += s.commit_mb;
        }
    }
    let mut other: Vec<(String, f64)> = other_by_name.into_iter().collect();
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

/// Handle-count anomaly thresholds. Both real incidents this detects were
/// found manually with Sysinternals Handle after weeks of commit growth
/// (docs/status/STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md §16:
/// Audiosrv at ~85K handles ≈ 18GB of leaked pagefile-backed Sections;
/// docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md: agentmux-srv
/// itself at ~215K, 99.5% Sections). A leaking process crosses these within
/// hours of a healthy baseline (both incidents' healthy baselines were
/// under 1K handles), while legitimately handle-heavy processes
/// (System/svchost/browsers) sit well below the external threshold —
/// 24,797 was the highest non-leaking count observed on the 08-08
/// machine snapshot.
#[cfg(target_os = "windows")]
const HANDLE_WARN_AGENTMUX: u32 = 20_000;
#[cfg(target_os = "windows")]
const HANDLE_WARN_EXTERNAL: u32 = 50_000;
#[cfg(target_os = "windows")]
const HANDLE_TOP_N: usize = 3;
/// Minimum gap between repeat WARNs for the SAME still-anomalous PID. The
/// first live run of this detector (2026-08-09) re-warned every 30s
/// attribution cycle — and more under the urgent-on-pressure path — for
/// two persisting Vite-process anomalies, drowning the log. One warn per
/// PID per this window keeps the signal (a persisting anomaly still
/// re-surfaces regularly, and the info line's `handles_top` carries the
/// count every cycle regardless) without the spam. A PID that drops below
/// threshold and re-crosses later warns immediately again (its entry is
/// pruned while healthy).
#[cfg(target_os = "windows")]
const HANDLE_WARN_COOLDOWN: Duration = Duration::from_secs(600);

/// One process's handle-count sample for anomaly classification.
#[cfg(target_os = "windows")]
struct HandleSample {
    pid: u32,
    name: String,
    handles: u32,
}

#[cfg(target_os = "windows")]
struct HandleAnomaly {
    pid: u32,
    name: String,
    handles: u32,
    /// True → one of our own processes (leak in AgentMux code, actionable
    /// by us); false → an external process (Audiosrv-class OS leak — the
    /// remediation is restarting *that* service, not AgentMux).
    is_agentmux: bool,
}

/// Pure anomaly classification, factored out for unit testing like
/// `classify_commit_attribution` above. AgentMux's own processes get the
/// lower threshold (we know their healthy baseline is well under 1K); any
/// other process only trips the higher one.
#[cfg(target_os = "windows")]
fn classify_handle_anomalies(samples: &[HandleSample]) -> Vec<HandleAnomaly> {
    let mut anomalies: Vec<HandleAnomaly> = samples
        .iter()
        .filter_map(|s| {
            let is_agentmux = s.name.to_ascii_lowercase().starts_with("agentmux");
            let threshold = if is_agentmux { HANDLE_WARN_AGENTMUX } else { HANDLE_WARN_EXTERNAL };
            (s.handles >= threshold).then(|| HandleAnomaly {
                pid: s.pid,
                name: s.name.clone(),
                handles: s.handles,
                is_agentmux,
            })
        })
        .collect();
    anomalies.sort_by(|a, b| b.handles.cmp(&a.handles));
    anomalies
}

/// Pure cooldown gate for anomaly WARNs, factored out for unit testing.
/// Returns the indices (into `anomalies`) that should warn THIS cycle, and
/// updates `last_warned` accordingly. Entries for PIDs that are no longer
/// anomalous are pruned first — both so the map can't grow unbounded across
/// process churn, and so a process that recovers and later re-crosses the
/// threshold warns immediately rather than being silently inside a stale
/// cooldown window.
#[cfg(target_os = "windows")]
fn filter_anomalies_for_warn(
    anomalies: &[HandleAnomaly],
    last_warned: &mut HashMap<u32, Instant>,
    now: Instant,
    cooldown: Duration,
) -> Vec<usize> {
    let current: HashSet<u32> = anomalies.iter().map(|a| a.pid).collect();
    last_warned.retain(|pid, _| current.contains(pid));

    let mut warn = Vec::new();
    for (i, anomaly) in anomalies.iter().enumerate() {
        let due = match last_warned.get(&anomaly.pid) {
            Some(&at) => now.duration_since(at) >= cooldown,
            None => true,
        };
        if due {
            last_warned.insert(anomaly.pid, now);
            warn.push(i);
        }
    }
    warn
}

/// Process-lifetime state for the cooldown above. A plain static rather
/// than threading it through the loop: `log_memory_attribution` is only
/// ever called from the single sysinfo loop task, so contention is nil.
#[cfg(target_os = "windows")]
static LAST_HANDLE_WARN: std::sync::OnceLock<std::sync::Mutex<HashMap<u32, Instant>>> =
    std::sync::OnceLock::new();

/// Open-query-close a process's total handle count. Returns `None` for
/// PIDs we can't open (protected/system processes) — expected, skipped
/// silently. The handle opened here is closed unconditionally on both the
/// success and failure path; leaking handles from the handle-leak detector
/// would be a bit much.
#[cfg(target_os = "windows")]
pub(crate) fn process_handle_count(pid: u32) -> Option<u32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetProcessHandleCount, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut count: u32 = 0;
        let ok = GetProcessHandleCount(h, &mut count);
        CloseHandle(h);
        (ok != 0).then_some(count)
    }
}

/// Full-system commit snapshot: refresh every process (heavier than the
/// per-block passes above, but only ever called on the attribution
/// cadence — every 30s in steady state, or debounced-urgent under
/// pressure), classify, and log one line. Independent of the per-tick
/// block-stats pass above — it does its own PID-tree collection so it
/// can't be affected by that pass's ordering.
///
/// `commit_used_gb`/`commit_total_gb` come from the caller's already-computed
/// `get_commit_data` values rather than re-querying `GlobalMemoryStatusEx`
/// here — this function is on the same tick as that call.
#[cfg(target_os = "windows")]
fn log_memory_attribution(sys: &mut sysinfo::System, commit_used_gb: f64, commit_total_gb: f64) {
    // Only name()/parent()/virtual_memory() are read below — parent() and
    // name() are always populated regardless of refresh kind, so only
    // memory needs to be requested. The per-pane pass a few lines below
    // (targeted `ProcessesToUpdate::Some`) uses the same restraint; this is
    // the whole-machine equivalent, so it's the one call where skipping
    // CPU deltas/disk I/O/exe/cmdline/environ actually matters.
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true, // remove dead processes — keep this bounded to what's live
        ProcessRefreshKind::nothing().with_memory(),
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

    let other_top_str = result
        .other_top
        .iter()
        .map(|(name, mb)| format!("{}:{:.0}MB", name, mb))
        .collect::<Vec<_>>()
        .join(", ");

    // The two diagnostics both prior pagefile-exhaustion incidents needed
    // (and had to gather manually with Sysinternals — see the doc refs on
    // HANDLE_WARN_AGENTMUX above): the *unattributed* commit gap, and
    // per-process handle counts. Process-private buckets alone are blind to
    // exactly the leak class that caused both incidents: pagefile-backed
    // Section objects held via leaked handles are charged to system commit
    // but to no process's private bytes, so `agentmux_mb + panes_mb +
    // other_mb` stays flat while `commit_used_gb` climbs — the gap IS the
    // signal. Handle counts then attribute the gap to the owning process
    // (which the commit numbers can't), the way the manual handle.exe sweep
    // did both times.
    let attributed_gb = (result.agentmux_mb + result.panes_mb + result.other_mb) / 1024.0;
    let unattributed_gb = (commit_used_gb - attributed_gb).max(0.0);

    // One OpenProcess/GetProcessHandleCount/CloseHandle triplet per live
    // process, on the same 30s cadence as the rest of this function —
    // microseconds each, no handles retained (see process_handle_count).
    let handle_samples: Vec<HandleSample> = samples
        .iter()
        .filter_map(|s| {
            process_handle_count(s.pid).map(|handles| HandleSample {
                pid: s.pid,
                name: s.name.clone(),
                handles,
            })
        })
        .collect();
    let mut top_handles: Vec<&HandleSample> = handle_samples.iter().collect();
    top_handles.sort_by(|a, b| b.handles.cmp(&a.handles));
    let handles_top_str = top_handles
        .iter()
        .take(HANDLE_TOP_N)
        .map(|s| format!("{}:{}:{}", s.name, s.pid, s.handles))
        .collect::<Vec<_>>()
        .join(", ");

    let anomalies = classify_handle_anomalies(&handle_samples);
    let warn_indices = {
        let mut last = LAST_HANDLE_WARN
            .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        filter_anomalies_for_warn(&anomalies, &mut last, Instant::now(), HANDLE_WARN_COOLDOWN)
    };
    for i in warn_indices {
        let anomaly = &anomalies[i];
        if anomaly.is_agentmux {
            tracing::warn!(
                target: "mem_attribution",
                pid = anomaly.pid,
                name = %anomaly.name,
                handles = anomaly.handles,
                "possible handle leak in an AgentMux process — a healthy baseline is under ~1K; \
                 run `handle64 -s -p <pid>` for the type breakdown (a huge Section count is the \
                 known leak signature, docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md)"
            );
        } else {
            tracing::warn!(
                target: "mem_attribution",
                pid = anomaly.pid,
                name = %anomaly.name,
                handles = anomaly.handles,
                "external process handle-count anomaly (Audiosrv-class OS leak?) — if commit is \
                 also climbing unattributed, restarting that process/service reclaims it \
                 (docs/status/STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md §16)"
            );
        }
    }

    tracing::info!(
        target: "mem_attribution",
        commit_used_gb = format!("{:.2}", commit_used_gb),
        commit_total_gb = format!("{:.2}", commit_total_gb),
        agentmux_mb = format!("{:.0}", result.agentmux_mb),
        panes_mb = format!("{:.0}", result.panes_mb),
        other_mb = format!("{:.0}", result.other_mb),
        other_top = %other_top_str,
        unattributed_gb = format!("{:.2}", unattributed_gb),
        handles_top = %handles_top_str,
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

// ── SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2 P0 ──────────────────────────────────
// "Track free disk on the pagefile volume, not just avail_page_gb. Warn when
// free disk < ~15% and page file is system-managed." The existing commit
// gauge above only sees the SYMPTOM (commit near limit); it is blind to the
// CAUSE this spec found: a system-managed page file wants to grow toward
// min(3×RAM, ⅛ volume) but silently cannot if the volume it lives on doesn't
// have the free space, pinning the commit ceiling well below what every
// other gauge assumes. This makes the cause itself visible.
//
// The registry-parsing + disk-free logic itself now lives in
// `agentmux_common::pagefile` (extracted by
// SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §4) so `agentmux-cef`'s
// low-memory banner can consume the identical implementation instead of a
// second, independently-drifting copy — this file just wires it into the
// StatusBar telemetry payload.

/// Collect pagefile-volume disk tracking: `disk:pagefile_volume:free_gb`,
/// `:total_gb`, `:free_pct`, and `disk:pagefile_system_managed` (1.0/0.0).
/// No-op (keys absent) on non-Windows or any read failure — matches
/// `get_commit_data`'s fail-open convention.
fn get_pagefile_volume_data(_values: &mut HashMap<String, f64>) {
    #[cfg(target_os = "windows")]
    {
        let (drive, system_managed) = agentmux_common::pagefile::pagefile_watch_target();
        if let Some((free_gb, total_gb)) = agentmux_common::pagefile::drive_free_total_gb(drive) {
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
            // Marks which disk:vol:<mount>:* volume (get_disk_data) the
            // pagefile-volume gauge above refers to, so the status bar can
            // name the drive ("C:") in its Disk readout without a separate
            // string channel — the values map is f64-only, so the letter
            // rides in the key. Mount format matches sysinfo's Windows
            // mount_point() ("C:\").
            _values.insert(format!("disk:vol:{}:\\:watch", drive), 1.0);
            // Whether the watched (page-file) volume is also the OS's own
            // system drive -- usually true, but a custom PagingFiles entry
            // can point the page file at a different volume than
            // %SystemDrive% (e.g. `E:\pagefile.sys 0 0` with SystemDrive=C).
            // Lets the status-bar tooltip say "system drive (C:)" only when
            // that's actually true, rather than mislabeling a page-file-only
            // volume as the system drive (reagent finding on #2479).
            let is_system_drive = std::env::var("SystemDrive")
                .ok()
                .and_then(|s| s.chars().next())
                .map(|c| c.to_ascii_uppercase() == drive)
                .unwrap_or(false);
            _values.insert(
                format!("disk:vol:{}:\\:is_system_drive", drive),
                if is_system_drive { 1.0 } else { 0.0 },
            );
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod handle_anomaly_tests {
    use super::*;

    fn hsample(pid: u32, name: &str, handles: u32) -> HandleSample {
        HandleSample { pid, name: name.to_string(), handles }
    }

    #[test]
    fn healthy_baselines_trip_nothing() {
        let samples = vec![
            hsample(1, "agentmux-srv.exe", 950),
            hsample(2, "svchost.exe", 3_000),
            hsample(3, "chrome.exe", 11_000),
        ];
        assert!(classify_handle_anomalies(&samples).is_empty());
    }

    #[test]
    fn agentmux_process_trips_at_the_lower_threshold() {
        // The real 08-08 incident profile: an agentmux process far above
        // its healthy baseline but an external process (chrome-scale) that
        // is high-but-normal must not be flagged alongside it.
        let samples = vec![
            hsample(1, "agentmux-srv-0.54.10-windows.x64.exe", HANDLE_WARN_AGENTMUX),
            hsample(2, "chrome.exe", HANDLE_WARN_AGENTMUX + 5_000),
        ];
        let anomalies = classify_handle_anomalies(&samples);
        assert_eq!(anomalies.len(), 1);
        assert!(anomalies[0].is_agentmux);
        assert_eq!(anomalies[0].pid, 1);
    }

    #[test]
    fn external_process_trips_only_at_the_higher_threshold() {
        // The real 07-24 incident profile: Audiosrv's svchost at ~85K.
        let samples = vec![hsample(9, "svchost.exe", 85_000)];
        let anomalies = classify_handle_anomalies(&samples);
        assert_eq!(anomalies.len(), 1);
        assert!(!anomalies[0].is_agentmux);
    }

    #[test]
    fn anomalies_are_sorted_worst_first() {
        let samples = vec![
            hsample(1, "agentmux-srv.exe", 25_000),
            hsample(2, "svchost.exe", 85_000),
        ];
        let anomalies = classify_handle_anomalies(&samples);
        assert_eq!(anomalies[0].pid, 2);
        assert_eq!(anomalies[1].pid, 1);
    }

    #[test]
    fn own_process_handle_count_is_queryable_and_nonzero() {
        // Live smoke test of the winapi wrapper against this test process
        // itself — a running process always has at least a few handles.
        let count = process_handle_count(std::process::id());
        assert!(count.is_some_and(|c| c > 0), "got {count:?}");
    }

    /// Regression test for the `sysinfo` v0.34.2 `CreateToolhelp32Snapshot`
    /// handle leak (docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md,
    /// docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_LIVE_RECURRENCE_2026_08_19.md).
    /// That version's Windows `refresh_processes_specifics` never closed the
    /// handle it got from `CreateToolhelp32Snapshot`, leaking one `Section`
    /// object (kernel-charged, pagefile-backed) per call — invisible to this
    /// process's own working-set/private-bytes, only visible as a climbing
    /// `GetProcessHandleCount`. Repeats the exact call pattern
    /// `run_sysinfo_loop` makes per tick (`ProcessesToUpdate::All`, the light
    /// refresh kind used for the parent-link pass) many times and asserts
    /// this process's own handle count does NOT grow linearly with call
    /// count — a fixed `sysinfo` leaks ~0 handles/call; the broken 0.34.2
    /// leaked exactly 1/call, so 500 calls would have shown +500, not "a
    /// small bounded amount from unrelated one-time OS/runtime activity."
    #[test]
    fn refresh_processes_specifics_does_not_leak_a_handle_per_call() {
        let pid = std::process::id();
        let mut sys = sysinfo::System::new();
        // Warm up: first call(s) can allocate one-time bookkeeping (e.g. the
        // process list's initial capacity) that isn't part of the per-call
        // leak this test is checking for.
        for _ in 0..5 {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                false,
                ProcessRefreshKind::nothing(),
            );
        }
        let before = process_handle_count(pid).expect("own handle count must be queryable");

        const CALLS: u32 = 500;
        for _ in 0..CALLS {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                false,
                ProcessRefreshKind::nothing(),
            );
        }
        let after = process_handle_count(pid).expect("own handle count must be queryable");

        let grew_by = after.saturating_sub(before);
        assert!(
            grew_by < CALLS / 2,
            "handle count grew by {grew_by} over {CALLS} refresh_processes_specifics calls \
             (before={before}, after={after}) — consistent with a per-call handle leak \
             (the fixed-in-0.35.0 CreateToolhelp32Snapshot bug leaked exactly 1/call)"
        );
    }

    fn anomaly(pid: u32) -> HandleAnomaly {
        HandleAnomaly { pid, name: format!("proc{pid}.exe"), handles: 99_999, is_agentmux: false }
    }

    #[test]
    fn cooldown_warns_immediately_on_first_sight_then_suppresses() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        let cooldown = Duration::from_secs(600);
        let anomalies = vec![anomaly(1)];

        assert_eq!(filter_anomalies_for_warn(&anomalies, &mut last, t0, cooldown), vec![0]);
        // Same anomaly, next 30s cycle — suppressed.
        let t1 = t0 + Duration::from_secs(30);
        assert!(filter_anomalies_for_warn(&anomalies, &mut last, t1, cooldown).is_empty());
        // Past the cooldown — warns again.
        let t2 = t0 + cooldown;
        assert_eq!(filter_anomalies_for_warn(&anomalies, &mut last, t2, cooldown), vec![0]);
    }

    #[test]
    fn cooldown_is_per_pid_not_global() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        let cooldown = Duration::from_secs(600);

        assert_eq!(filter_anomalies_for_warn(&[anomaly(1)], &mut last, t0, cooldown), vec![0]);
        // A DIFFERENT pid appearing 30s later warns immediately, even though
        // pid 1 is mid-cooldown.
        let t1 = t0 + Duration::from_secs(30);
        let both = vec![anomaly(1), anomaly(2)];
        assert_eq!(filter_anomalies_for_warn(&both, &mut last, t1, cooldown), vec![1]);
    }

    #[test]
    fn recovered_pid_recrossing_the_threshold_warns_immediately() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        let cooldown = Duration::from_secs(600);

        assert_eq!(filter_anomalies_for_warn(&[anomaly(1)], &mut last, t0, cooldown), vec![0]);
        // pid 1 drops below threshold (absent from anomalies) — its entry is
        // pruned...
        let t1 = t0 + Duration::from_secs(30);
        assert!(filter_anomalies_for_warn(&[], &mut last, t1, cooldown).is_empty());
        assert!(last.is_empty(), "healthy pid's entry must be pruned");
        // ...so re-crossing 30s later (well inside what would have been the
        // old cooldown window) warns immediately.
        let t2 = t0 + Duration::from_secs(60);
        assert_eq!(filter_anomalies_for_warn(&[anomaly(1)], &mut last, t2, cooldown), vec![0]);
    }
}

#[cfg(all(test, target_os = "windows"))]
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
    fn aggregates_other_by_name_before_ranking_so_a_multi_process_app_is_not_split_below_the_cutoff() {
        // A multi-process app (Chrome, VS Code) spread across many small
        // per-PID entries must still rank by its TOTAL commit, not have each
        // PID compete individually against single-process apps — otherwise
        // the true top contributor can be missing from other_top even
        // though other_mb (the sum) correctly reflects it. reagent P1 on
        // PR #2207.
        let samples = vec![
            sample(1, "chrome.exe", 100.0),
            sample(2, "chrome.exe", 100.0),
            sample(3, "chrome.exe", 100.0),
            sample(4, "chrome.exe", 100.0),
            sample(5, "single_app.exe", 250.0),
        ];
        let result = classify_commit_attribution(&samples, &HashSet::new(), 1);
        assert_eq!(result.other_mb, 650.0);
        assert_eq!(result.other_top.len(), 1);
        assert_eq!(result.other_top[0], ("chrome.exe".to_string(), 400.0));
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

    /// Runs the real Win32 process-enumeration path against this machine's
    /// actual process table — the synthetic-data tests above cover
    /// classification logic but can't catch a bad Win32 API call, an
    /// access-denied edge case, or a panic on a real process's field
    /// values. This test's own process is guaranteed to exist in the table
    /// with a real name and non-zero commit, so a classification bucket is
    /// exercised end-to-end for real.
    #[test]
    fn log_memory_attribution_runs_against_the_real_process_table_without_panicking() {
        let mut sys = sysinfo::System::new_all();
        log_memory_attribution(&mut sys, 0.0, 0.0); // must not panic
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
    // Per-volume capacity/free, keyed by mount point:
    //   disk:vol:<mount>:free_gb / disk:vol:<mount>:total_gb
    // (e.g. `disk:vol:C:\:free_gb` — the mount's own colons/backslashes are
    // fine inside the key; consumers anchor on the `disk:vol:` prefix and the
    // known suffix). Feeds the status bar's Disk popover (per-drive free
    // space). Deduped by mount point — sysinfo can list one volume once per
    // physical disk it spans — and zero-capacity entries (empty card
    // readers, unformatted media) are skipped rather than shown as "0G".
    let mut seen_mounts: HashSet<&std::path::Path> = HashSet::new();
    for disk in disks.list() {
        let mount = disk.mount_point();
        if !seen_mounts.insert(mount) {
            continue;
        }
        let total = disk.total_space();
        if total == 0 {
            continue;
        }
        let mount_str = mount.to_string_lossy();
        values.insert(
            format!("disk:vol:{}:free_gb", mount_str),
            disk.available_space() as f64 / BYTES_PER_GB,
        );
        values.insert(
            format!("disk:vol:{}:total_gb", mount_str),
            total as f64 / BYTES_PER_GB,
        );
    }

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
    // Windows-only — see the matching #[cfg] on the loop body below.
    #[cfg(target_os = "windows")]
    let mut last_attribution = Instant::now()
        .checked_sub(ATTRIBUTION_INTERVAL)
        .unwrap_or_else(Instant::now);
    #[cfg(target_os = "windows")]
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
        // Windows-only like every other commit/pagefile helper in this file
        // (get_commit_data, available_commit_gb, get_pagefile_volume_data) —
        // its rationale (pagefile/commit exhaustion) doesn't apply elsewhere,
        // and off-Windows `mem:commit:*` are never populated, so this would
        // otherwise pay a full process-table refresh every tick just to log
        // a permanently-zero commit_used_gb/commit_total_gb.
        #[cfg(target_os = "windows")]
        {
            let commit_avail_gb = match (values.get("mem:commit:total"), values.get("mem:commit:used")) {
                (Some(total), Some(used)) => Some(total - used),
                _ => None,
            };
            let due_periodic = now_instant.duration_since(last_attribution) >= ATTRIBUTION_INTERVAL;
            let due_urgent = commit_avail_gb.is_some_and(|gb| gb < ATTRIBUTION_URGENT_THRESHOLD_GB)
                && last_urgent_attribution
                    .is_none_or(|t| now_instant.duration_since(t) >= ATTRIBUTION_URGENT_COOLDOWN);
            if due_periodic || due_urgent {
                let commit_used_gb = *values.get("mem:commit:used").unwrap_or(&0.0);
                let commit_total_gb = *values.get("mem:commit:total").unwrap_or(&0.0);
                tokio::task::block_in_place(|| {
                    log_memory_attribution(&mut sys, commit_used_gb, commit_total_gb)
                });
                last_attribution = now_instant;
                if due_urgent {
                    last_urgent_attribution = Some(now_instant);
                }
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let ts_data = TimeSeriesData { ts: now, values, uptime_secs: Some(uptime_secs()) };

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
                    // Per-block process stats, not the backend's own uptime.
                    uptime_secs: None,
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

#[cfg(test)]
mod uptime_tests {
    use super::*;

    /// Neither property this fix depends on — immunity to a clock step, and
    /// counting across suspend — can be asserted in a unit test: the platform
    /// counters are deliberately unsettable, and suspending the machine isn't
    /// something a test can do. What IS assertable: the marker is idempotent
    /// (a stray second call can't restart the clock) and the value never runs
    /// backwards. The `u64` return type is itself part of the fix — the old
    /// wall-clock subtraction was signed and reached about -1.7e9.
    #[test]
    fn uptime_is_monotonic_and_the_start_marker_is_idempotent() {
        mark_process_start();
        let first = uptime_secs();
        mark_process_start();
        let second = uptime_secs();
        assert!(second >= first, "uptime went backwards: {second} < {first}");
    }

    /// The suspend-aware counter must be a sane, forward-running millisecond
    /// source on whatever platform this is built for — a `clock_gettime`
    /// failure returning 0, or a units mix-up, would show up here.
    #[test]
    fn suspend_aware_clock_is_nonzero_and_never_decreases() {
        let a = suspend_aware_now_ms();
        let b = suspend_aware_now_ms();
        assert!(a > 0, "suspend-aware clock returned 0 — clock_gettime failed?");
        assert!(b >= a, "suspend-aware clock went backwards: {b} < {a}");
        // Sanity on units: a machine that booted more than ~10 years ago is a
        // units error (ns or µs mistaken for ms), not a real uptime.
        assert!(a < 10 * 365 * 24 * 60 * 60 * 1_000, "implausible ms reading: {a}");
    }

    /// The frontend's fallback path keys off this field's ABSENCE, and the
    /// per-block stats / CPU-stream payloads must keep their exact prior
    /// shape — so `skip_serializing_if` is a wire contract, not a style choice.
    #[test]
    fn uptime_secs_is_emitted_when_present_and_omitted_when_absent() {
        let with = TimeSeriesData { ts: 5, values: HashMap::new(), uptime_secs: Some(42) };
        let v = serde_json::to_value(&with).expect("serialize");
        assert_eq!(v["uptime_secs"], serde_json::json!(42));

        let without = TimeSeriesData { ts: 5, values: HashMap::new(), uptime_secs: None };
        let v = serde_json::to_value(&without).expect("serialize");
        assert!(
            v.get("uptime_secs").is_none(),
            "block-stats/CPU-stream payload shape must be byte-for-byte unchanged"
        );
    }

    /// A payload produced before this field existed (e.g. replayed from the
    /// sysinfo persist ring across an upgrade) must still deserialize.
    #[test]
    fn payload_without_uptime_still_deserializes() {
        let json = serde_json::json!({ "ts": 7, "values": { "cpu": 1.5 } });
        let parsed: TimeSeriesData = serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed.ts, 7);
        assert_eq!(parsed.uptime_secs, None);
    }
}
