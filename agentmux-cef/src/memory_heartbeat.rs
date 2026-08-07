// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory heartbeat — logs system and process memory stats every 20 seconds.
// Designed to provide forensic data for OOM / VA exhaustion crash analysis.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Latest observed system commit-free (available page file) in MB. Published by
/// the heartbeat loop and by the on-demand probe; read by the gated renderer
/// recovery path to distinguish "OOM under system memory pressure" (transient,
/// recoverable) from "broken renderer". `u64::MAX` until the first sample, so
/// the gate treats an un-sampled process as having ample commit.
///
/// See docs/specs/SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md §6.A.
static COMMIT_FREE_MB: AtomicU64 = AtomicU64::new(u64::MAX);

/// Latest observed system commit **total** (limit) in MB, alongside
/// `COMMIT_FREE_MB` above. `0` until the first sample — used by
/// `memory_pressure.rs` to convert the free-MB reading into a ratio, since an
/// absolute-MB threshold is meaningless across the huge range of commit
/// limits real machines run with (SPEC_WIN10_PAGEFILE_OOM_CRASH's own status
/// bar already gauges by ratio; the pressure classifier didn't, until now).
static COMMIT_TOTAL_MB: AtomicU64 = AtomicU64::new(0);

/// Latest observed **physical RAM** free/total in MB — `SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07`
/// §3. Distinct from `COMMIT_FREE_MB`/`COMMIT_TOTAL_MB` above: commit is RAM +
/// page file combined, so a machine can be tight on RAM while commit (backed
/// by a healthy page file) stays comfortable, or vice versa — conflating the
/// two is exactly the bug that spec fixes. Populated from the same
/// `GlobalMemoryStatusEx` call `log_memory_stats()` already makes (`ullAvailPhys`/
/// `ullTotalPhys`), so this costs no extra syscall.
static PHYS_FREE_MB: AtomicU64 = AtomicU64::new(u64::MAX);
static PHYS_TOTAL_MB: AtomicU64 = AtomicU64::new(0);

/// Latest published physical-RAM-free reading, in MB. `u64::MAX` until the
/// first sample (mirrors `COMMIT_FREE_MB`'s "treat as ample" convention).
pub fn phys_free_mb() -> u64 {
    PHYS_FREE_MB.load(Ordering::Relaxed)
}

/// Latest published physical-RAM-total reading, in MB. `0` until the first
/// sample — `memory_pressure.rs` treats a zero total as "not yet known".
pub fn phys_total_mb() -> u64 {
    PHYS_TOTAL_MB.load(Ordering::Relaxed)
}

/// On-demand synchronous probe of system commit-free (available page file), in
/// MB. ~microsecond cost (a single `GlobalMemoryStatusEx`). Republishes the
/// atomic so `last_commit_free_mb()` stays fresh. On non-Windows returns
/// `u64::MAX` — commit-limit exhaustion is a Windows concern here and the
/// recovery gate degrades to "always treat as ample" elsewhere.
#[cfg(target_os = "windows")]
pub fn commit_free_mb() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 {
        let mb = (mem.ullAvailPageFile / (1024 * 1024)) as u64;
        COMMIT_FREE_MB.store(mb, Ordering::Relaxed);
        COMMIT_TOTAL_MB.store((mem.ullTotalPageFile / (1024 * 1024)) as u64, Ordering::Relaxed);
        mb
    } else {
        COMMIT_FREE_MB.load(Ordering::Relaxed)
    }
}

#[cfg(not(target_os = "windows"))]
pub fn commit_free_mb() -> u64 {
    u64::MAX
}

/// Latest observed system commit total (limit), in MB. See `COMMIT_TOTAL_MB`.
/// `0` on non-Windows / before the first sample — `memory_pressure.rs`
/// treats a zero total as "not yet known" and stays `Normal` rather than
/// dividing by zero.
pub fn commit_total_mb() -> u64 {
    COMMIT_TOTAL_MB.load(Ordering::Relaxed)
}

/// Page-file disk/OS-managed context for the current tick:
/// `(system_managed, disk_free_pct)` on the volume backing the page file.
/// `None` on non-Windows or any read failure (fail-open — the banner then
/// omits the disk-aware guidance rather than showing a wrong one).
/// `SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07` §4: reuses
/// `agentmux_common::pagefile`, the same implementation
/// `agentmux-srv/src/backend/sysinfo.rs`'s StatusBar telemetry already
/// calls, so the two processes can't independently drift on "system
/// managed?" the way the commit-ratio classifier and the StatusBar gauge
/// once did (issue #2218). Registry read is `OnceLock`-cached inside that
/// module (once per process lifetime); the disk-free check is a real syscall
/// but runs on this heartbeat's own dedicated thread, not the Tokio runtime
/// `agentmux-srv` has to protect with `block_in_place` — safe to call
/// directly every 20s tick.
#[cfg(target_os = "windows")]
fn pagefile_disk_context() -> Option<(bool, f64)> {
    let (drive, system_managed) = agentmux_common::pagefile::pagefile_watch_target();
    agentmux_common::pagefile::drive_free_total_gb(drive).map(|(free_gb, total_gb)| {
        let free_pct = if total_gb > 0.0 { (free_gb / total_gb) * 100.0 } else { 0.0 };
        (system_managed, free_pct)
    })
}

#[cfg(not(target_os = "windows"))]
fn pagefile_disk_context() -> Option<(bool, f64)> {
    None
}

/// Spawn a background thread that logs memory stats at a fixed interval.
/// Also refreshes the log pointer file on UTC date rollover.
/// Runs for the lifetime of the process — no shutdown signal needed.
pub fn start(state: std::sync::Arc<crate::state::AppState>) {
    std::thread::Builder::new()
        .name("mem-heartbeat".into())
        .spawn(move || {
            let mut last_date = String::new();
            // Two independent trackers (SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07
            // §3/§4): `commit_pressure` is the original signal (RAM + page
            // file combined) driving the pane-pool shedding response and the
            // Page File banner; `ram_pressure` is new, physical-RAM-only,
            // banner-only (no shedding — see the spec's §6 non-goals).
            let mut commit_pressure = crate::memory_pressure::PressureTracker::new();
            let mut ram_pressure = crate::memory_pressure::PressureTracker::new();
            loop {
                std::thread::sleep(Duration::from_secs(20));
                log_memory_stats();
                // Feed the debounced memory-pressure trackers from the just-
                // sampled readings; on a level transition, log it AND push
                // the new level to the frontend low-memory banner
                // (SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.A/§5.F). The
                // emit is posted to the CEF UI thread (this is a background
                // thread); the banner shows on Warn/Critical and clears on the
                // return to Normal.
                let free = commit_free_mb();
                let total = commit_total_mb();
                let transition = commit_pressure.observe(free, total);
                let level_now = commit_pressure.level();

                let ram_free = phys_free_mb();
                let ram_total = phys_total_mb();
                let ram_transition = ram_pressure.observe(ram_free, ram_total);
                let ram_level_now = ram_pressure.level();

                if let Some(level) = transition {
                    tracing::warn!(
                        target: "mem_pressure",
                        kind = "pagefile",
                        level = level.as_str(),
                        commit_free_mb = free,
                        "page file (commit) pressure changed"
                    );
                    // B.5 Part 1 (issue #2218): react to the transition, not
                    // just log it. Entering Warn/Critical trims the pane pool
                    // (the one pool with a reliable on-demand destroy path
                    // today — see evict_idle_pane_pool_window's doc comment);
                    // returning to Normal best-effort refills both pools so a
                    // transient pressure blip doesn't leave them starved for
                    // the rest of the session. Both spawn_* fns are already
                    // internally single-flight + target-size-gated, so calling
                    // them unconditionally here is safe. Commit-only: RAM
                    // pressure alone (with healthy commit) doesn't risk a
                    // crash the way commit pressure does, so it doesn't
                    // trigger shedding — SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §6.
                    if level != crate::memory_pressure::PressureLevel::Normal {
                        while crate::commands::window_pool::evict_idle_pane_pool_window(&state) {}
                    } else {
                        crate::commands::window_pool::spawn_pane_pool_window(&state);
                        crate::commands::window_pool::spawn_pool_window(&state);
                    }
                }
                if let Some(level) = ram_transition {
                    tracing::warn!(
                        target: "mem_pressure",
                        kind = "ram",
                        level = level.as_str(),
                        phys_free_mb = ram_free,
                        "RAM pressure changed"
                    );
                }
                // Push to the banner on a transition (to show or clear it), AND
                // re-assert a steady non-Normal level each tick so a window
                // opened or reloaded mid-episode catches up within one heartbeat
                // rather than staying silent until the next transition (reagent
                // #1501 P2 — the CustomEvent channel has no replay-on-subscribe).
                // Idempotent on existing windows (the frontend dedups an
                // unchanged level and a re-assert never un-dismisses); a steady
                // Normal stays silent, so there's no traffic in the common case.
                if transition.is_some() || level_now != crate::memory_pressure::PressureLevel::Normal {
                    let (system_managed, disk_free_pct) = pagefile_disk_context().unwrap_or((true, 100.0));
                    crate::ui_tasks::post_memory_pressure_pagefile(
                        &state,
                        level_now.as_str(),
                        free,
                        system_managed,
                        disk_free_pct,
                    );
                }
                if ram_transition.is_some() || ram_level_now != crate::memory_pressure::PressureLevel::Normal {
                    crate::ui_tasks::post_memory_pressure_ram(&state, ram_level_now.as_str(), ram_free);
                }
                refresh_log_pointer(&mut last_date);
            }
        })
        .expect("Failed to spawn memory heartbeat thread");
}

/// Update the host log pointer file when the UTC date changes (midnight rollover).
/// tracing_appender::rolling::daily creates a new file at UTC midnight, so the
/// pointer must track the new date suffix.
///
/// Two pointers are written, matching `main::init_logging`:
///   1. Local: `<log_dir>/<pointer_name>` with just the basename.
///   2. Global: `<root>/logs/<pointer_name>` with the absolute path so
///      legacy tooling (`muxlog host`) can resolve from outside the
///      instance dir. Works for portable, installed, and dev modes —
///      log_dir comes from `AGENTMUX_LOG_DIR`, set by data_paths.rs.
fn refresh_log_pointer(last_date: &mut String) {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    if *last_date == today {
        return;
    }
    *last_date = today.clone();
    let version = env!("CARGO_PKG_VERSION");
    // Use the AGENTMUX_LOG_DIR exported by data_paths.rs. Falls back to
    // the legacy hardcoded location only as a safety net — by the time
    // memory_heartbeat starts, init_logging has already run with the
    // resolved log_dir, so this env var should always be present.
    let log_dir = std::env::var_os("AGENTMUX_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".agentmux")
                .join("logs")
        });
    let current_filename = format!("agentmux-host-v{}.log.{}", version, today);
    let absolute_path = log_dir.join(&current_filename);
    let pointer_name = format!("current-host-v{}.path", version);

    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);

    if let Some(global_logs_dir) = log_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.join("logs"))
    {
        let _ = std::fs::create_dir_all(&global_logs_dir);
        let _ = std::fs::write(
            global_logs_dir.join(&pointer_name),
            absolute_path.to_string_lossy().as_bytes(),
        );
    }
}

#[cfg(target_os = "windows")]
fn log_memory_stats() {
    use windows_sys::Win32::System::SystemInformation::{
        GlobalMemoryStatusEx, MEMORYSTATUSEX,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };

    // ── System-wide stats ──
    let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let sys_ok = unsafe { GlobalMemoryStatusEx(&mut mem) } != 0;

    // ── Per-process stats ──
    let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let proc_ok = unsafe {
        let handle = windows_sys::Win32::System::Threading::GetCurrentProcess();
        GetProcessMemoryInfo(handle, &mut pmc, pmc.cb)
    } != 0;

    if sys_ok {
        let total_phys_gb = mem.ullTotalPhys as f64 / (1024.0 * 1024.0 * 1024.0);
        let avail_phys_gb = mem.ullAvailPhys as f64 / (1024.0 * 1024.0 * 1024.0);
        let total_page_gb = mem.ullTotalPageFile as f64 / (1024.0 * 1024.0 * 1024.0);
        let avail_page_gb = mem.ullAvailPageFile as f64 / (1024.0 * 1024.0 * 1024.0);
        let total_virt_gb = mem.ullTotalVirtual as f64 / (1024.0 * 1024.0 * 1024.0);
        let avail_virt_gb = mem.ullAvailVirtual as f64 / (1024.0 * 1024.0 * 1024.0);
        let load_pct = mem.dwMemoryLoad;

        // Publish commit-free for the gated renderer recovery path (§6.A).
        COMMIT_FREE_MB.store((mem.ullAvailPageFile / (1024 * 1024)) as u64, Ordering::Relaxed);
        // Publish physical RAM free/total for the RAM pressure tracker
        // (SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §3) — same `mem`
        // struct already fetched above, no extra syscall.
        PHYS_FREE_MB.store((mem.ullAvailPhys / (1024 * 1024)) as u64, Ordering::Relaxed);
        PHYS_TOTAL_MB.store((mem.ullTotalPhys / (1024 * 1024)) as u64, Ordering::Relaxed);

        tracing::info!(
            target: "mem_heartbeat",
            load_pct,
            total_phys_gb = format!("{:.1}", total_phys_gb),
            avail_phys_gb = format!("{:.1}", avail_phys_gb),
            total_page_gb = format!("{:.1}", total_page_gb),
            avail_page_gb = format!("{:.1}", avail_page_gb),
            total_virt_gb = format!("{:.1}", total_virt_gb),
            avail_virt_gb = format!("{:.1}", avail_virt_gb),
            "system memory"
        );
    }

    if proc_ok {
        let ws_mb = pmc.WorkingSetSize as f64 / (1024.0 * 1024.0);
        let peak_ws_mb = pmc.PeakWorkingSetSize as f64 / (1024.0 * 1024.0);
        let pagefile_mb = pmc.PagefileUsage as f64 / (1024.0 * 1024.0);
        let peak_pagefile_mb = pmc.PeakPagefileUsage as f64 / (1024.0 * 1024.0);
        let page_faults = pmc.PageFaultCount;

        tracing::info!(
            target: "mem_heartbeat",
            ws_mb = format!("{:.1}", ws_mb),
            peak_ws_mb = format!("{:.1}", peak_ws_mb),
            commit_mb = format!("{:.1}", pagefile_mb),
            peak_commit_mb = format!("{:.1}", peak_pagefile_mb),
            page_faults,
            "process memory"
        );
    }

    if !sys_ok && !proc_ok {
        tracing::warn!(target: "mem_heartbeat", "Failed to query memory stats");
    }
}

#[cfg(not(target_os = "windows"))]
fn log_memory_stats() {
    // On non-Windows, read /proc/self/status and /proc/meminfo.
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let vm_rss = extract_proc_field(&status, "VmRSS:");
        let vm_size = extract_proc_field(&status, "VmSize:");
        let vm_peak = extract_proc_field(&status, "VmPeak:");
        tracing::info!(
            target: "mem_heartbeat",
            vm_rss, vm_size, vm_peak,
            "process memory"
        );
    }
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        let total = extract_proc_field(&meminfo, "MemTotal:");
        let avail = extract_proc_field(&meminfo, "MemAvailable:");
        tracing::info!(
            target: "mem_heartbeat",
            total, avail,
            "system memory"
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn extract_proc_field(content: &str, field: &str) -> String {
    content
        .lines()
        .find(|l| l.starts_with(field))
        .map(|l| l[field.len()..].trim().to_string())
        .unwrap_or_else(|| "?".into())
}
