// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows: one `NtQuerySystemInformation(SystemProcessInformation)` call
//! returns every process with its CPU times, working sets and parent, without
//! opening any of them. That is how an unelevated Task Manager shows SYSTEM
//! and elevated processes too, and it is one syscall per refresh however many
//! processes there are.
//!
//! The documented `SYSTEM_PROCESS_INFORMATION` (winternl.h) hides most fields
//! behind `Reserved` arrays; [`SystemProcessInformation`] spells out the
//! layout every Windows since Vista uses (and that Task Manager, Process
//! Explorer and sysinfo read). Its size and offsets are pinned by tests.

use std::mem::{offset_of, size_of};

use windows_sys::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::{CloseHandle, NTSTATUS};
use windows_sys::Win32::System::Threading::{
    GetActiveProcessorCount, OpenProcess, ALL_PROCESSOR_GROUPS, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::ProcInfo;

const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = 0xC000_0004_u32 as i32;
const STATUS_BUFFER_TOO_SMALL: NTSTATUS = 0xC000_0023_u32 as i32;

/// 100 ns intervals between 1601-01-01 (FILETIME's epoch) and 1970-01-01.
const FILETIME_UNIX_EPOCH: i64 = 116_444_736_000_000_000;

#[allow(dead_code)] // the kernel's layout; not every field is read
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[allow(dead_code)] // the kernel's layout; not every field is read
#[repr(C)]
struct SystemProcessInformation {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UnicodeString,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
    // SYSTEM_THREAD_INFORMATION[number_of_threads] follows.
}

/// The whole table in one buffer. It grows until the kernel's answer fits
/// (the process count can change between the size probe and the read).
fn query_process_table() -> std::io::Result<Vec<u64>> {
    // u64 words: the entries hold 8-byte fields and must be 8-aligned.
    let mut words = 64 * 1024; // 512 KiB, enough for ~400 processes
    for _ in 0..6 {
        let mut buf: Vec<u64> = vec![0; words];
        let mut needed: u32 = 0;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr().cast(),
                (words * 8) as u32,
                &mut needed,
            )
        };
        match status {
            0 => return Ok(buf),
            STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_TOO_SMALL => {
                // Room for what it asked for plus some new processes.
                words = (needed as usize / 8).max(words) + 16 * 1024;
            }
            s => return Err(std::io::Error::other(format!("NtQuerySystemInformation: NTSTATUS {s:#x}"))),
        }
    }
    Err(std::io::Error::other("NtQuerySystemInformation: the process table kept growing"))
}

fn filetime_to_unix_ms(ft: i64) -> Option<u64> {
    (ft > FILETIME_UNIX_EPOCH).then(|| ((ft - FILETIME_UNIX_EPOCH) / 10_000) as u64)
}

/// # Safety
/// `s` must point into memory that holds `s.length` bytes at `s.buffer`.
unsafe fn read_unicode(s: &UnicodeString) -> String {
    if s.buffer.is_null() || s.length == 0 {
        return String::new();
    }
    let units = std::slice::from_raw_parts(s.buffer, s.length as usize / 2);
    String::from_utf16_lossy(units)
}

pub fn snapshot() -> std::io::Result<Vec<ProcInfo>> {
    let buf = query_process_table()?;
    let bytes = buf.len() * 8;
    let base = buf.as_ptr() as *const u8;
    let mut out = Vec::with_capacity(512);
    let mut offset = 0usize;
    loop {
        if offset + size_of::<SystemProcessInformation>() > bytes {
            break;
        }
        // SAFETY: in bounds (checked above) and 8-aligned (every
        // next_entry_offset is a multiple of 8, the buffer is u64s).
        let entry = unsafe { &*(base.add(offset) as *const SystemProcessInformation) };
        let pid = entry.unique_process_id as u32;
        // PID 0 is the idle process: its "CPU" is idle time, not work.
        if pid != 0 {
            // SAFETY: the kernel points image_name into this same buffer.
            let name = unsafe { read_unicode(&entry.image_name) };
            let name = match (name.is_empty(), pid) {
                (true, 4) => "System".to_string(),
                (true, _) => String::new(),
                (false, _) => name,
            };
            out.push(ProcInfo {
                pid,
                ppid: Some(entry.inherited_from_unique_process_id as u32).filter(|&p| p != 0),
                start_key: entry.create_time as u64,
                started_at_ms: filetime_to_unix_ms(entry.create_time),
                name,
                cpu_ns: Some((entry.user_time.max(0) as u64 + entry.kernel_time.max(0) as u64) * 100),
                mem_private: Some(entry.working_set_private_size.max(0) as u64),
                mem_resident: Some(entry.working_set_size as u64),
                mem_commit: Some(entry.private_page_count as u64),
            });
        }
        if entry.next_entry_offset == 0 {
            break;
        }
        offset += entry.next_entry_offset as usize;
    }
    Ok(out)
}

/// `NtQueryInformationProcess(ProcessCommandLineInformation)`: Windows 8.1+,
/// and it needs only `PROCESS_QUERY_LIMITED_INFORMATION` (no `VM_READ` of the
/// target's memory), so it works for every same-user process.
pub fn command_line(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut needed: u32 = 0;
        let _ = NtQueryInformationProcess(h, ProcessCommandLineInformation, std::ptr::null_mut(), 0, &mut needed);
        let mut result = None;
        if needed as usize >= size_of::<UnicodeString>() {
            let mut buf: Vec<u64> = vec![0; (needed as usize).div_ceil(8)];
            let status = NtQueryInformationProcess(
                h,
                ProcessCommandLineInformation,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut needed,
            );
            if status == 0 {
                // SAFETY: the header points into `buf`, which outlives this read.
                let header = &*(buf.as_ptr() as *const UnicodeString);
                result = Some(read_unicode(header));
            }
        }
        CloseHandle(h);
        result
    }
}

/// The creation time [`snapshot`] reports as `start_key`, read for one
/// process.
pub fn start_key(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let (mut create, mut exit, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = std::mem::zeroed();
        let ok = GetProcessTimes(h, &mut create, &mut exit, &mut kernel, &mut user) != 0;
        CloseHandle(h);
        ok.then(|| ((create.dwHighDateTime as u64) << 32) | create.dwLowDateTime as u64)
    }
}

pub fn cpu_count() -> usize {
    unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) as usize }
}

// The layout is the kernel's, not ours: pin it so an edit can't shift a field.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(offset_of!(SystemProcessInformation, working_set_private_size) == 8);
    assert!(offset_of!(SystemProcessInformation, create_time) == 32);
    assert!(offset_of!(SystemProcessInformation, user_time) == 40);
    assert!(offset_of!(SystemProcessInformation, kernel_time) == 48);
    assert!(offset_of!(SystemProcessInformation, image_name) == 56);
    assert!(offset_of!(SystemProcessInformation, unique_process_id) == 80);
    assert!(offset_of!(SystemProcessInformation, inherited_from_unique_process_id) == 88);
    assert!(offset_of!(SystemProcessInformation, peak_working_set_size) == 136);
    assert!(offset_of!(SystemProcessInformation, working_set_size) == 144);
    assert!(offset_of!(SystemProcessInformation, private_page_count) == 200);
    assert!(size_of::<SystemProcessInformation>() == 256);
};

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    fn api_memory(pid: u32) -> PROCESS_MEMORY_COUNTERS {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            assert!(!h.is_null());
            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            assert_ne!(GetProcessMemoryInfo(h, &mut counters, size), 0);
            CloseHandle(h);
            counters
        }
    }

    /// The snapshot's fields match what the documented per-process APIs say
    /// — the check that the layout above is right. Measured on an idle child,
    /// not this process: the snapshot's own buffer (megabytes) is committed
    /// only while the call runs, so this process's memory moves under it.
    #[test]
    fn snapshot_agrees_with_the_documented_apis() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > nul"])
            .spawn()
            .expect("spawn an idle child");
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let before = api_memory(pid);
        let p = snapshot().unwrap().into_iter().find(|p| p.pid == pid).expect("child listed");
        let after = api_memory(pid);
        let near = |got: u64, a: usize, b: usize, what: &str| {
            let (lo, hi) = (a.min(b) as u64, a.max(b) as u64);
            assert!(got * 10 >= lo * 9 && got * 10 <= hi * 11, "{what}: {got} vs {lo}..{hi}");
        };
        near(p.mem_resident.unwrap(), before.WorkingSetSize, after.WorkingSetSize, "working set");
        near(p.mem_commit.unwrap(), before.PagefileUsage, after.PagefileUsage, "commit");
        assert!(p.mem_private.unwrap() > 0 && p.mem_private.unwrap() <= p.mem_resident.unwrap(), "{p:?}");
        assert_eq!(p.ppid, Some(std::process::id()));
        assert!(p.name.eq_ignore_ascii_case("cmd.exe"), "{p:?}");
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            let (mut create, mut exit, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
                std::mem::zeroed();
            assert_ne!(GetProcessTimes(h, &mut create, &mut exit, &mut kernel, &mut user), 0);
            CloseHandle(h);
            let ft = |f: FILETIME| ((f.dwHighDateTime as i64) << 32) | f.dwLowDateTime as i64;
            assert_eq!(p.start_key, ft(create) as u64, "create time");
            assert_eq!(p.cpu_ns, Some(((ft(kernel) + ft(user)) * 100) as u64), "CPU time");
        }
        let line = command_line(pid).expect("a same-user child's command line");
        assert!(line.contains("ping -n 30"), "{line:?}");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn system_and_other_sessions_processes_are_measured_too() {
        let all = snapshot().unwrap();
        let system = all.iter().find(|p| p.pid == 4).expect("System (pid 4)");
        assert_eq!(system.name, "System");
        assert!(system.cpu_ns.is_some());
        assert!(all.iter().all(|p| p.pid != 0), "the idle process is left out");
        // Something running as SYSTEM in session 0 (services, csrss) is listed
        // with a working set even though this process can't open it.
        assert!(all.iter().any(|p| p.name.eq_ignore_ascii_case("services.exe") && p.mem_resident.unwrap_or(0) > 0));
    }
}
