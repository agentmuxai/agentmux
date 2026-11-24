// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the machine and srv itself were doing in the seconds before a runtime stall.
//!
//! The stall watch (`stall_watch`) can say that the runtime's beat was late, not why. A late beat with an empty queue is
//! what a machine with no CPU for srv looks like, and also what workers blocked in a synchronous call look like. These
//! counters tell them apart: the machine's busy share, srv's own share, how much srv read and wrote, and how full memory
//! was. They are read at every look (a few system calls, four times a second) and kept for a few seconds; the start line
//! of a stall reports what changed over the last two seconds, which reaches back into the stall itself.
//!
//! Windows only: elsewhere [`read`] gives nothing and the line carries the load average instead. Numbers only.

use std::collections::VecDeque;

/// How far back the start line looks.
pub const REPORT_OVER_MS: u64 = 2_000;
/// How long a reading is kept.
const KEEP_MS: u64 = 12_000;

/// One reading of the counters, on the watch's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    pub at_ms: u64,
    /// All cores, kernel and user time, minus idle time (100 ns units).
    pub machine_busy: u64,
    /// All cores, kernel and user time (100 ns units).
    pub machine_total: u64,
    /// This process, kernel and user time (100 ns units).
    pub srv_cpu: u64,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

/// What changed between two readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Load {
    /// The span the figures cover.
    pub over_ms: u64,
    /// The share of all cores that were busy, in percent.
    pub machine_cpu_pct: u32,
    /// The share of all cores srv's own threads used, in percent.
    pub srv_cpu_pct: u32,
    pub io_read_mb: u64,
    pub io_write_mb: u64,
    /// How full physical memory was when the stall was noticed, in percent.
    pub mem_load_pct: Option<u32>,
}

fn pct(part: u64, whole: u64) -> u32 {
    ((part as u128 * 100 + whole as u128 / 2) / whole as u128).min(100) as u32
}

/// The change from `from` to `to`, or nothing when no time passed between them.
pub fn delta(from: &Sample, to: &Sample, mem_load_pct: Option<u32>) -> Option<Load> {
    let total = to.machine_total.checked_sub(from.machine_total).filter(|t| *t > 0)?;
    let over_ms = to.at_ms.checked_sub(from.at_ms).filter(|t| *t > 0)?;
    Some(Load {
        over_ms,
        machine_cpu_pct: pct(to.machine_busy.saturating_sub(from.machine_busy), total),
        srv_cpu_pct: pct(to.srv_cpu.saturating_sub(from.srv_cpu), total),
        io_read_mb: to.io_read_bytes.saturating_sub(from.io_read_bytes) / (1024 * 1024),
        io_write_mb: to.io_write_bytes.saturating_sub(from.io_write_bytes) / (1024 * 1024),
        mem_load_pct,
    })
}

/// The last few seconds of readings.
#[derive(Debug, Default)]
pub struct History {
    samples: VecDeque<Sample>,
}

impl History {
    /// Keeps `sample`, and drops what is older than [`KEEP_MS`].
    pub fn push(&mut self, sample: Sample) {
        while self.samples.front().is_some_and(|s| sample.at_ms.saturating_sub(s.at_ms) > KEEP_MS) {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// What changed over the last `over_ms`, from the newest reading back to the latest one at least that far back
    /// (the oldest kept one when the history is shorter).
    pub fn load_over(&self, over_ms: u64, mem_load_pct: Option<u32>) -> Option<Load> {
        let newest = self.samples.back()?;
        let from = self
            .samples
            .iter()
            .rev()
            .find(|s| newest.at_ms.saturating_sub(s.at_ms) >= over_ms)
            .or_else(|| self.samples.front())?;
        delta(from, newest, mem_load_pct)
    }
}

/// A reading now, stamped `at_ms`; nothing where the counters are not read.
#[cfg(windows)]
pub fn read(at_ms: u64) -> Option<Sample> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, GetSystemTimes, IO_COUNTERS};
    let value = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut idle, mut kernel, mut user) = (zero, zero, zero);
    // SAFETY: the three pointers are to live locals of the type the call fills.
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return None;
    }
    let (mut created, mut exited, mut proc_kernel, mut proc_user) = (zero, zero, zero, zero);
    // SAFETY: as above; the pseudo-handle of the current process needs no closing.
    let process = unsafe { GetCurrentProcess() };
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut proc_kernel, &mut proc_user) } == 0 {
        return None;
    }
    // SAFETY: `io` is a zeroed plain struct the call fills.
    let mut io: IO_COUNTERS = unsafe { std::mem::zeroed() };
    let io_ok = unsafe { GetProcessIoCounters(process, &mut io) } != 0;
    let total = value(kernel) + value(user);
    Some(Sample {
        at_ms,
        machine_busy: total.saturating_sub(value(idle)),
        machine_total: total,
        srv_cpu: value(proc_kernel) + value(proc_user),
        io_read_bytes: if io_ok { io.ReadTransferCount } else { 0 },
        io_write_bytes: if io_ok { io.WriteTransferCount } else { 0 },
    })
}

#[cfg(not(windows))]
pub fn read(_at_ms: u64) -> Option<Sample> {
    None
}

/// How full physical memory is now, in percent.
#[cfg(windows)]
pub fn mem_load_pct() -> Option<u32> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: a zeroed plain struct with its length set, as the call requires.
    let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut mem) } != 0).then_some(mem.dwMemoryLoad)
}

#[cfg(not(windows))]
pub fn mem_load_pct() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_ms: u64, busy: u64, total: u64, srv: u64, read_mb: u64, write_mb: u64) -> Sample {
        Sample { at_ms, machine_busy: busy, machine_total: total, srv_cpu: srv, io_read_bytes: read_mb << 20, io_write_bytes: write_mb << 20 }
    }

    #[test]
    fn a_busy_machine_and_a_quiet_srv_read_as_such() {
        // 2 s on 32 cores is 640,000,000 units of 100 ns in all; 90% busy, srv 3%.
        let a = sample(1_000, 0, 0, 0, 10, 10);
        let b = sample(3_000, 576_000_000, 640_000_000, 19_200_000, 10, 50);
        let load = delta(&a, &b, Some(71)).unwrap();
        assert_eq!(load, Load { over_ms: 2_000, machine_cpu_pct: 90, srv_cpu_pct: 3, io_read_mb: 0, io_write_mb: 40, mem_load_pct: Some(71) });
    }

    #[test]
    fn no_time_between_two_readings_is_no_figure() {
        let a = sample(1_000, 5, 10, 1, 0, 0);
        assert_eq!(delta(&a, &a, None), None);
        assert_eq!(delta(&sample(2_000, 5, 10, 1, 0, 0), &sample(1_000, 6, 11, 2, 0, 0), None), None);
        assert_eq!(delta(&sample(1_000, 5, 10, 1, 0, 0), &sample(2_000, 5, 10, 1, 0, 0), None), None, "no CPU time passed");
    }

    #[test]
    fn a_share_never_exceeds_a_hundred() {
        let load = delta(&sample(0, 0, 0, 0, 0, 0), &sample(1_000, 300, 100, 500, 0, 0), None).unwrap();
        assert_eq!((load.machine_cpu_pct, load.srv_cpu_pct), (100, 100));
    }

    #[test]
    fn the_history_looks_back_the_span_asked_and_forgets_the_old() {
        let mut h = History::default();
        for i in 0..100u64 {
            h.push(sample(i * 250, i * 10, i * 20, i, 0, 0));
        }
        // Newest at 24,750 ms; the span of 2 s starts at the reading 8 looks earlier.
        let load = h.load_over(REPORT_OVER_MS, None).unwrap();
        assert_eq!(load.over_ms, 2_000);
        assert_eq!(load.machine_cpu_pct, 50);
        assert!(h.samples.len() <= 50, "readings older than 12 s are dropped, kept {}", h.samples.len());
    }

    #[test]
    fn a_short_history_reports_what_there_is() {
        let mut h = History::default();
        assert_eq!(h.load_over(REPORT_OVER_MS, None), None);
        h.push(sample(0, 0, 0, 0, 0, 0));
        assert_eq!(h.load_over(REPORT_OVER_MS, None), None, "one reading is no span");
        h.push(sample(500, 5, 10, 1, 0, 0));
        assert_eq!(h.load_over(REPORT_OVER_MS, None).unwrap().over_ms, 500);
    }

    #[cfg(windows)]
    #[test]
    fn the_counters_are_read_on_windows() {
        let a = read(0).expect("a reading");
        let mut x = 0u64;
        for i in 0..50_000_000u64 {
            x = x.wrapping_add(i);
        }
        std::hint::black_box(x);
        let b = read(100).expect("a second reading");
        assert!(b.machine_total >= a.machine_total && b.srv_cpu >= a.srv_cpu);
        assert!(mem_load_pct().is_some_and(|p| p <= 100));
    }
}
