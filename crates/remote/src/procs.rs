// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host's processes for Tower (`fsproto::Request::Procs`,
//! SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §8): sampled here, on the host,
//! with the same readers srv uses locally (`agentmux_procstats`), so only a
//! small frame crosses the link.
//!
//! - **Small:** the `top` busiest by CPU and the `top` largest by memory
//!   (the union, at most `2 × top` rows), plus totals for all of them. A
//!   filter is applied here first, so a search finds a process that isn't in
//!   either top list.
//! - **No command lines.** Names only: a command line can hold a secret, and
//!   never leaves the host through this.
//! - **Rates on the host:** CPU is the change since the previous `Procs` on
//!   this connection, so nothing but the frame is sent and srv keeps no
//!   per-host state.

use std::collections::HashSet;
use std::time::Instant;

use agentmux_procstats::{ProcInfo, ProcKey, RateMeter};

use crate::fsproto::{ProcFrame, ProcRow};

/// Most rows per list a request may ask for.
pub const MAX_TOP: u32 = 500;

/// One connection's sampler: the previous round's CPU times.
#[derive(Default)]
pub struct Sampler {
    rates: RateMeter<ProcKey>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// A frame of the host's processes now (module doc). A host whose
    /// process table can't be read answers with an empty frame.
    pub fn frame(&mut self, top: u32, filter: &str) -> ProcFrame {
        let procs = agentmux_procstats::snapshot().unwrap_or_default();
        let now = Instant::now();
        let rates: Vec<Option<f64>> =
            procs.iter().map(|p| p.cpu_ns.and_then(|ns| self.rates.sample(p.key(), ns, now))).collect();
        self.rates.finish();
        select(&procs, &rates, top, filter, agentmux_procstats::cpu_count() as u32)
    }
}

fn milli(rate: f64) -> u32 {
    (rate * 1000.0).round().clamp(0.0, (u32::MAX - 1) as f64) as u32
}

/// Whether every word of `filter` is in the name or the PID.
fn matches(p: &ProcInfo, words: &[String]) -> bool {
    let hay = format!("{} {}", p.name.to_lowercase(), p.pid);
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// The frame for `procs` (with each one's CPU rate, fraction of one core):
/// totals over all of them, rows for the busiest and largest that match.
pub fn select(procs: &[ProcInfo], rates: &[Option<f64>], top: u32, filter: &str, cpu_count: u32) -> ProcFrame {
    let top = top.min(MAX_TOP) as usize;
    let words: Vec<String> = filter.to_lowercase().split_whitespace().map(str::to_string).collect();
    let matching: Vec<usize> = (0..procs.len()).filter(|&i| matches(&procs[i], &words)).collect();

    let mut by_cpu = matching.clone();
    by_cpu.sort_by(|&a, &b| rates[b].unwrap_or(-1.0).total_cmp(&rates[a].unwrap_or(-1.0)));
    let mut by_mem = matching.clone();
    by_mem.sort_by_key(|&i| std::cmp::Reverse(procs[i].mem_private.map_or(-1, |m| m as i128)));
    let mut seen = HashSet::new();
    let chosen: Vec<usize> = by_cpu.into_iter().take(top).chain(by_mem.into_iter().take(top)).filter(|i| seen.insert(*i)).collect();

    ProcFrame {
        cpu_count,
        memory_metric: agentmux_procstats::MEMORY_METRIC.to_string(),
        total: procs.len() as u32,
        matched: matching.len() as u32,
        unmeasured: procs.iter().filter(|p| !p.measured()).count() as u32,
        cpu_milli: rates.iter().flatten().fold(None, |acc: Option<u64>, &r| Some(acc.unwrap_or(0) + milli(r) as u64)),
        mem: procs.iter().filter_map(|p| p.mem_private).sum(),
        rows: chosen
            .into_iter()
            .map(|i| {
                let p = &procs[i];
                ProcRow {
                    pid: p.pid,
                    ppid: p.ppid,
                    start_key: p.start_key,
                    started_at_ms: p.started_at_ms,
                    name: p.name.clone(),
                    cpu_milli: rates[i].map(milli),
                    mem: p.mem_private,
                    mem_resident: p.mem_resident,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, name: &str, mem: Option<u64>) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(1),
            start_key: pid as u64,
            started_at_ms: Some(1),
            name: name.into(),
            cpu_ns: mem.map(|_| 0),
            mem_private: mem,
            mem_resident: mem,
            mem_commit: None,
        }
    }

    fn pids(f: &ProcFrame) -> Vec<u32> {
        let mut v: Vec<u32> = f.rows.iter().map(|r| r.pid).collect();
        v.sort();
        v
    }

    #[test]
    fn the_busiest_and_the_largest_are_sent_with_totals_for_all() {
        let procs = vec![p(1, "init", Some(10)), p(2, "big", Some(9_000)), p(3, "busy", Some(20)), p(4, "idle", Some(30))];
        let rates = vec![Some(0.0), Some(0.01), Some(1.5), None];
        let f = select(&procs, &rates, 1, "", 8);
        assert_eq!(pids(&f), vec![2, 3], "top 1 by CPU (busy) and top 1 by memory (big)");
        assert_eq!((f.total, f.matched, f.cpu_count), (4, 4, 8));
        assert_eq!(f.cpu_milli, Some(1510));
        assert_eq!(f.mem, 9_060);
        let busy = f.rows.iter().find(|r| r.pid == 3).unwrap();
        assert_eq!(busy.cpu_milli, Some(1500));
    }

    #[test]
    fn a_filter_finds_what_neither_top_list_holds() {
        let mut procs: Vec<ProcInfo> = (10..60).map(|i| p(i, "worker", Some(i as u64 * 100))).collect();
        procs.push(p(4242, "sshd", Some(1)));
        let rates = vec![Some(0.1); procs.len()];
        let f = select(&procs, &rates, 5, "SSHD", 2);
        assert_eq!(pids(&f), vec![4242]);
        assert_eq!((f.total, f.matched), (51, 1), "totals still count every process");
        let f = select(&procs, &rates, 5, "4242", 2);
        assert_eq!(pids(&f), vec![4242], "by PID");
    }

    #[test]
    fn an_unmeasured_process_is_counted_and_sent_without_numbers() {
        let procs = vec![p(1, "launchd", None), p(2, "node", Some(5))];
        let f = select(&procs, &[None, Some(0.2)], 10, "", 4);
        assert_eq!(f.unmeasured, 1);
        assert_eq!(select(&procs, &[None, None], 10, "", 4).cpu_milli, None, "no rate yet: unknown, not 0");
        let launchd = f.rows.iter().find(|r| r.pid == 1).unwrap();
        assert_eq!((launchd.cpu_milli, launchd.mem), (None, None));
    }

    #[test]
    fn top_is_capped() {
        let procs: Vec<ProcInfo> = (1..=2000).map(|i| p(i, "x", Some(i as u64))).collect();
        let rates = vec![Some(0.0); procs.len()];
        assert!(select(&procs, &rates, u32::MAX, "", 1).rows.len() <= 2 * MAX_TOP as usize);
    }

    /// The real sampler: this test process is in its own host's frame, and a
    /// second frame on the same sampler has rates.
    #[test]
    fn a_real_frame_and_its_rates() {
        let mut s = Sampler::new();
        let me = std::process::id();
        let first = s.frame(MAX_TOP, &me.to_string());
        assert!(first.rows.iter().any(|r| r.pid == me), "{first:?}");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let second = s.frame(MAX_TOP, &me.to_string());
        assert!(second.rows.iter().find(|r| r.pid == me).unwrap().cpu_milli.is_some());
        assert!(second.total > 1);
    }
}
