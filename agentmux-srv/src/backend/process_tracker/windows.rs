// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
#![cfg(windows)]

//! Windows `ProcessTreeTracker` implementation backed by Job Objects.
//!
//! Flow:
//! 1. `JobObjectTracker::new(block_id)` creates an anonymous job and sets
//!    `KILL_ON_JOB_CLOSE`, so anything in the job dies automatically if
//!    AgentMux itself crashes without calling `kill_tree`.
//! 2. The caller gets the tracker handle and, when spawning the agent
//!    CLI, calls `assign_process(child_pid)` immediately after spawn.
//!    Every `CreateProcess` descendant of that PID inherits the job
//!    automatically — no per-process tagging.
//! 3. `list_members` queries the job for its current PID set and
//!    enriches each with command line + RSS via `PROCESS_QUERY_LIMITED_INFORMATION`
//!    + `GetModuleFileNameEx` / `GetProcessMemoryInfo`.
//! 4. `kill_tree` → `TerminateJobObject`. One call nukes everything.
//!
//! The only non-trivial thing: there's a ~1ms race window between
//! `CreateProcess` and our `AssignProcessToJobObject`. A child the CLI
//! creates in that window escapes the job. In practice the CLI doesn't
//! spawn anything before it reads stdin, so this is a theoretical
//! concern — but worth a future move to `CREATE_SUSPENDED` + assign +
//! `ResumeThread` if we see escapes.

use std::ffi::OsString;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStringExt;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};

use super::{TrackedProcess, TrackerHandle, TrackingConfidence};

pub struct JobObjectTracker {
    block_id: String,
    /// Inner state behind a mutex so a later `kill_tree` call from a
    /// different thread is safe. The HANDLE itself is thread-safe per
    /// Win32 docs, but the Drop impl still needs exclusive access.
    inner: Mutex<Inner>,
}

struct Inner {
    job: HANDLE,
    /// Closed-idempotent flag. `kill_tree` + `Drop` both call
    /// `CloseHandle`; the second caller must no-op.
    closed: bool,
}

impl JobObjectTracker {
    /// Inherent convenience wrapper — the canonical way callers should
    /// use `assign_process` is via the `TrackerHandle` trait, which
    /// dispatches across platforms. This stays inherent for internal
    /// callers that hold a concrete `JobObjectTracker`.
    #[allow(dead_code)]
    pub fn assign_inherent(&self, pid: u32) -> Result<(), String> {
        <Self as TrackerHandle>::assign_process(self, pid)
    }

    pub fn new(block_id: &str) -> Result<Self, String> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!("CreateJobObjectW failed: {}", std::io::Error::last_os_error()));
            }

            // Configure the job:
            //  - KILL_ON_JOB_CLOSE: everything in the job dies if the host
            //    process exits without explicit cleanup. No leaks across
            //    AgentMux crashes.
            //  - BREAKAWAY_OK is NOT set: descendants can't opt out of the
            //    job. (Some CLIs try CREATE_BREAKAWAY_FROM_JOB; we want
            //    those attempts to fail so the child stays tracked.)
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(format!("SetInformationJobObject failed: {err}"));
            }
            let _ = JOB_OBJECT_LIMIT_BREAKAWAY_OK; // referenced for docs

            tracing::info!(
                block_id = %block_id,
                job = ?job,
                "[process-tracker] created Windows Job Object"
            );

            Ok(Self {
                block_id: block_id.to_string(),
                inner: Mutex::new(Inner { job, closed: false }),
            })
        }
    }

    /// Internal assignment primitive — called via the `TrackerHandle`
    /// trait impl below. See the trait doc for semantics + race
    /// window caveat.
    fn assign_process_impl(&self, pid: u32) -> Result<(), String> {
        unsafe {
            let h_process = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | 0x0200, /* PROCESS_SET_QUOTA */
                0,
                pid,
            );
            if h_process.is_null() {
                return Err(format!("OpenProcess({pid}) failed: {}", std::io::Error::last_os_error()));
            }
            let inner = self.inner.lock().unwrap();
            let ok = AssignProcessToJobObject(inner.job, h_process);
            CloseHandle(h_process);
            if ok == 0 {
                return Err(format!(
                    "AssignProcessToJobObject({pid}) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            tracing::info!(
                block_id = %self.block_id,
                pid = pid,
                "[process-tracker] assigned process to job"
            );
            Ok(())
        }
    }

    fn query_pids(&self) -> Vec<u32> {
        // Buffer sized for up to 256 PIDs. If we overflow we'll log and
        // truncate — an agent with >256 descendants is an edge case we
        // can widen later.
        const MAX_PIDS: usize = 256;
        #[repr(C)]
        struct Buf {
            header: JOBOBJECT_BASIC_PROCESS_ID_LIST,
            rest: [usize; MAX_PIDS - 1],
        }
        let mut buf: Buf = unsafe { zeroed() };
        let mut returned: u32 = 0;
        let ok = unsafe {
            QueryInformationJobObject(
                self.inner.lock().unwrap().job,
                JobObjectBasicProcessIdList,
                &mut buf as *mut _ as *mut _,
                size_of::<Buf>() as u32,
                &mut returned,
            )
        };
        if ok == 0 {
            return Vec::new();
        }
        let count = buf.header.NumberOfProcessIdsInList as usize;
        let count = count.min(MAX_PIDS);
        let mut pids = Vec::with_capacity(count);
        // NumberOfAssignedProcesses is the total in the job; the first
        // entry is inlined in the header, then `rest` continues. Treat
        // as a flat array of `usize` starting at `header.ProcessIdList[0]`.
        let first_slot = &buf.header.ProcessIdList[0] as *const usize;
        for i in 0..count {
            let p = unsafe { *first_slot.add(i) } as u32;
            if p != 0 {
                pids.push(p);
            }
        }
        pids
    }
}

impl TrackerHandle for JobObjectTracker {
    fn assign_process(&self, pid: u32) -> Result<(), String> {
        self.assign_process_impl(pid)
    }

    fn list_members(&self) -> Vec<TrackedProcess> {
        self.query_pids()
            .into_iter()
            .map(|pid| TrackedProcess {
                pid,
                command: query_command_line(pid),
                rss_bytes: query_rss(pid),
                started_at_ms: 0, // deferred; uses NtQueryInformationProcess — skip for v1
            })
            .collect()
    }

    fn kill_tree(&self) {
        unsafe {
            let mut inner = self.inner.lock().unwrap();
            if inner.closed {
                return;
            }
            if TerminateJobObject(inner.job, 1) == 0 {
                tracing::warn!(
                    block_id = %self.block_id,
                    err = %std::io::Error::last_os_error(),
                    "[process-tracker] TerminateJobObject failed"
                );
            } else {
                tracing::info!(block_id = %self.block_id, "[process-tracker] killed job tree");
            }
            CloseHandle(inner.job);
            inner.closed = true;
        }
    }

    fn kill_pid(&self, pid: u32) -> bool {
        if !self.query_pids().iter().any(|&p| p == pid) {
            return false;
        }
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1);
            CloseHandle(h);
            ok != 0
        }
    }

    fn confidence(&self) -> TrackingConfidence {
        TrackingConfidence::High
    }
}

impl Drop for JobObjectTracker {
    fn drop(&mut self) {
        unsafe {
            let mut inner = self.inner.get_mut().unwrap();
            if !inner.closed && !inner.job.is_null() && inner.job != INVALID_HANDLE_VALUE {
                // KILL_ON_JOB_CLOSE will fire when we close the handle,
                // so we don't need an explicit TerminateJobObject here.
                CloseHandle(inner.job);
                inner.closed = true;
            }
        }
    }
}

// SAFETY: HANDLE is a pointer-sized integer; Windows guarantees all Job
// Object APIs are thread-safe.
unsafe impl Send for JobObjectTracker {}
unsafe impl Sync for JobObjectTracker {}

// ── Helpers ────────────────────────────────────────────────────────────

/// Read a process's command line via `GetCommandLineW` is not an option
/// for foreign processes — that's the calling process's cmdline.
/// Instead we use `QueryFullProcessImageNameW` for the executable path
/// and treat cmdline as "unavailable" for v1. WMI can fill this in
/// later if the user asks for full cmdline.
fn query_command_line(pid: u32) -> String {
    use windows_sys::Win32::System::Threading::{
        QueryFullProcessImageNameW,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        OsString::from_wide(&buf[..len as usize])
            .to_string_lossy()
            .into_owned()
    }
}

fn query_rss(pid: u32) -> u64 {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let mut counters: PROCESS_MEMORY_COUNTERS = zeroed();
        let ok = GetProcessMemoryInfo(
            h,
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        CloseHandle(h);
        if ok == 0 {
            0
        } else {
            counters.WorkingSetSize as u64
        }
    }
}
