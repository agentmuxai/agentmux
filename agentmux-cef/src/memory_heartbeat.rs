// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory heartbeat — logs system and process memory stats every 20 seconds.
// Designed to provide forensic data for OOM / VA exhaustion crash analysis.

use std::time::Duration;

/// Spawn a background thread that logs memory stats at a fixed interval.
/// Also refreshes the log pointer file on UTC date rollover.
/// Runs for the lifetime of the process — no shutdown signal needed.
pub fn start() {
    std::thread::Builder::new()
        .name("mem-heartbeat".into())
        .spawn(move || {
            let mut last_date = String::new();
            loop {
                std::thread::sleep(Duration::from_secs(20));
                log_memory_stats();
                refresh_log_pointer(&mut last_date);
            }
        })
        .expect("Failed to spawn memory heartbeat thread");
}

/// Update the host log pointer file when the UTC date changes (midnight rollover).
/// tracing_appender::rolling::daily creates a new file at UTC midnight, so the
/// pointer must track the new date suffix.
fn refresh_log_pointer(last_date: &mut String) {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    if *last_date == today {
        return;
    }
    *last_date = today.clone();
    let version = env!("CARGO_PKG_VERSION");
    let log_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".agentmux")
        .join("logs");
    let current_filename = format!("agentmux-host-v{}.log.{}", version, today);
    let pointer_name = format!("current-host-v{}.path", version);
    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);
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
