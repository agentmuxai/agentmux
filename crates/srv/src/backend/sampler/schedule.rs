// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which sources to sample now, and how soon to look again.
//!
//! A **source** is one thing worth measuring (the machine's counters, the
//! processes of each pane). Nothing runs unless someone holds a **lease** on it:
//! a named holder asking for it at an interval, until it is released or its
//! expiry passes. A source runs at the shortest interval among its live leases,
//! never faster than its own minimum, and slows down while it keeps taking longer
//! than its budget.
//!
//! This is the whole decision, kept pure so it is tested with a fake clock:
//! [`Scheduler::due`] says what to run, the caller runs it and reports
//! [`Scheduler::finished`], [`Scheduler::next_wakeup`] says when to look again.
//! Times are milliseconds on one monotonic clock the caller owns.
//! docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.5.

use std::collections::HashMap;

/// Milliseconds on the caller's monotonic clock.
pub type Ms = u64;

/// The most a source's interval is stretched for running over budget.
const MAX_STRETCH: u32 = 4;
/// Runs well inside the budget, in a row, before a stretch is halved.
const CALM_RUNS: u32 = 3;

/// The things that can be sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceId {
    /// CPU, memory, network and disk counters for the whole machine.
    Machine,
    /// Each pane's processes: CPU, memory and process count, and the cleanup of
    /// panes whose shell is gone.
    Panes,
}

impl SourceId {
    pub fn name(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::Panes => "panes",
        }
    }
}

/// What a source costs and needs.
#[derive(Debug, Clone)]
pub struct SourceSpec {
    pub id: SourceId,
    /// Never run more often than this, whatever a lease asks.
    pub min_interval_ms: Ms,
    /// How long one run should take. A longer one stretches the interval.
    pub budget_ms: Ms,
    /// Sources this one reads, which run first in the same pass. They must be
    /// declared before it.
    pub needs: &'static [SourceId],
}

#[derive(Debug, Clone)]
struct Lease {
    holder: String,
    source: SourceId,
    interval_ms: Ms,
    /// `None`: held until released.
    expires_at: Option<Ms>,
}

#[derive(Debug, Clone, Default)]
struct SourceState {
    /// When it runs next; `None` while nothing holds a lease on it.
    next_due: Option<Ms>,
    last_start: Option<Ms>,
    last_took: Ms,
    /// 1, 2 or 4: how many times its interval is stretched.
    stretch: u32,
    calm: u32,
    runs: u64,
    overruns: u64,
    /// Runs that were due and didn't start for a whole interval or more.
    skipped: u64,
}

/// What `sampler.status` reports for one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    pub id: SourceId,
    pub holders: Vec<String>,
    /// Between runs now, stretch included; `None` with no lease.
    pub interval_ms: Option<Ms>,
    pub stretch: u32,
    pub last_took_ms: Ms,
    pub runs: u64,
    pub overruns: u64,
    pub skipped: u64,
}

pub struct Scheduler {
    specs: Vec<SourceSpec>,
    leases: Vec<Lease>,
    state: HashMap<SourceId, SourceState>,
    /// Sources due within this much of each other run in one pass, so a shared
    /// input (one process snapshot) is read once for all of them.
    coalesce_ms: Ms,
}

impl Scheduler {
    /// `specs` in dependency order: each source's `needs` come before it.
    pub fn new(specs: Vec<SourceSpec>, coalesce_ms: Ms) -> Self {
        for (i, s) in specs.iter().enumerate() {
            assert!(
                s.needs.iter().all(|n| specs[..i].iter().any(|p| p.id == *n)),
                "source {:?} needs a source declared after it",
                s.id
            );
        }
        let state = specs.iter().map(|s| (s.id, SourceState { stretch: 1, ..Default::default() })).collect();
        Self { specs, leases: Vec::new(), state, coalesce_ms }
    }

    /// `holder` wants `source` every `interval_ms` until `expires_at` (`None`:
    /// until released). A holder has one lease per source, so asking again
    /// renews it.
    pub fn lease(&mut self, holder: &str, source: SourceId, interval_ms: Ms, expires_at: Option<Ms>, now: Ms) {
        match self.leases.iter_mut().find(|l| l.holder == holder && l.source == source) {
            Some(l) => {
                l.interval_ms = interval_ms;
                l.expires_at = expires_at;
            }
            None => self.leases.push(Lease { holder: holder.to_string(), source, interval_ms, expires_at }),
        }
        self.reschedule(source, now);
    }

    /// Drops all of `holder`'s leases.
    pub fn release(&mut self, holder: &str, now: Ms) {
        let sources: Vec<SourceId> = self.leases.iter().filter(|l| l.holder == holder).map(|l| l.source).collect();
        self.leases.retain(|l| l.holder != holder);
        for source in sources {
            self.reschedule(source, now);
        }
    }

    /// Drops `holder`'s lease on `source`.
    pub fn release_source(&mut self, holder: &str, source: SourceId, now: Ms) {
        self.leases.retain(|l| !(l.holder == holder && l.source == source));
        self.reschedule(source, now);
    }

    /// The leases changed, so the interval may have: a source that already ran
    /// is next due one (new) interval after that run, sooner or later than it was
    /// planned, and a pending run isn't kept for a viewer that has gone.
    fn reschedule(&mut self, source: SourceId, now: Ms) {
        let Some(interval) = self.effective_interval_ms(source, now) else { return };
        if let Some(st) = self.state.get_mut(&source) {
            if let (Some(_), Some(last)) = (st.next_due, st.last_start) {
                st.next_due = Some((last + interval).max(last + st.last_took));
            }
        }
    }

    fn spec(&self, id: SourceId) -> &SourceSpec {
        self.specs.iter().find(|s| s.id == id).expect("every SourceId has a spec")
    }

    fn live(&self, source: SourceId, now: Ms) -> impl Iterator<Item = &Lease> {
        self.leases.iter().filter(move |l| l.source == source && l.expires_at.is_none_or(|e| e > now))
    }

    /// Between runs now: the shortest live lease, floored at the source's
    /// minimum, times its stretch. `None` with no live lease.
    pub fn effective_interval_ms(&self, source: SourceId, now: Ms) -> Option<Ms> {
        let wanted = self.live(source, now).map(|l| l.interval_ms).min()?;
        let stretch = self.state.get(&source).map_or(1, |s| s.stretch.max(1));
        Some(wanted.max(self.spec(source).min_interval_ms) * Ms::from(stretch))
    }

    /// The sources to run now, dependencies first, each once. A source with no
    /// live lease isn't run, and starts at once when it gets one.
    pub fn due(&mut self, now: Ms) -> Vec<SourceId> {
        self.leases.retain(|l| l.expires_at.is_none_or(|e| e > now));
        let ids: Vec<SourceId> = self.specs.iter().map(|s| s.id).collect();
        let mut run = Vec::new();
        for id in ids {
            let leased = self.live(id, now).next().is_some();
            let st = self.state.get_mut(&id).expect("state for every source");
            if !leased {
                st.next_due = None;
                continue;
            }
            let due = *st.next_due.get_or_insert(now);
            if due <= now + self.coalesce_ms {
                run.push(id);
            }
        }
        // What a due source reads runs with it, leased or not.
        let mut wanted = run.clone();
        for id in &run {
            wanted.extend_from_slice(self.spec(*id).needs);
        }
        self.specs.iter().map(|s| s.id).filter(|id| wanted.contains(id)).collect()
    }

    /// When to call [`Self::due`] next; `None` when nothing is leased.
    pub fn next_wakeup(&self, now: Ms) -> Option<Ms> {
        self.specs
            .iter()
            .filter(|s| self.live(s.id, now).next().is_some())
            .map(|s| self.state[&s.id].next_due.unwrap_or(now).max(now))
            .min()
    }

    /// A run of `id` that started at `started` took `took_ms`. Schedules its next
    /// run from its start, never sooner than its end, and never to make up for a
    /// late one: a source that fell behind is counted as having skipped.
    pub fn finished(&mut self, id: SourceId, started: Ms, took_ms: Ms) {
        let budget = self.spec(id).budget_ms;
        let interval_before = self.effective_interval_ms(id, started).unwrap_or(1).max(1);
        let st = self.state.get_mut(&id).expect("state for every source");
        let planned = st.next_due.unwrap_or(started);
        st.skipped += started.saturating_sub(planned) / interval_before;
        st.runs += 1;
        st.last_start = Some(started);
        st.last_took = took_ms;
        if took_ms > budget {
            st.overruns += 1;
            st.calm = 0;
            st.stretch = (st.stretch.max(1) * 2).min(MAX_STRETCH);
        } else if took_ms * 2 <= budget {
            st.calm += 1;
            if st.calm >= CALM_RUNS && st.stretch > 1 {
                st.stretch /= 2;
                st.calm = 0;
            }
        } else {
            st.calm = 0;
        }
        let end = started + took_ms;
        let interval = self.effective_interval_ms(id, end).unwrap_or(interval_before);
        let st = self.state.get_mut(&id).expect("state for every source");
        st.next_due = Some((started + interval).max(end));
    }

    pub fn status(&self, now: Ms) -> Vec<SourceStatus> {
        self.specs
            .iter()
            .map(|s| {
                let st = &self.state[&s.id];
                SourceStatus {
                    id: s.id,
                    holders: self.live(s.id, now).map(|l| l.holder.clone()).collect(),
                    interval_ms: self.effective_interval_ms(s.id, now),
                    stretch: st.stretch.max(1),
                    last_took_ms: st.last_took,
                    runs: st.runs,
                    overruns: st.overruns,
                    skipped: st.skipped,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SourceId::{Machine, Panes};

    fn spec(id: SourceId, min: Ms, budget: Ms, needs: &'static [SourceId]) -> SourceSpec {
        SourceSpec { id, min_interval_ms: min, budget_ms: budget, needs }
    }

    fn sched() -> Scheduler {
        Scheduler::new(vec![spec(Machine, 200, 100, &[]), spec(Panes, 500, 200, &[])], 50)
    }

    /// Runs whatever is due at `now`, each taking `took`, and returns what ran.
    fn tick(s: &mut Scheduler, now: Ms, took: Ms) -> Vec<SourceId> {
        let ran = s.due(now);
        for id in &ran {
            s.finished(*id, now, took);
        }
        ran
    }

    #[test]
    fn nothing_leased_means_nothing_due_and_no_wakeup() {
        let mut s = sched();
        assert!(s.due(0).is_empty());
        assert_eq!(s.next_wakeup(0), None);
    }

    #[test]
    fn a_new_lease_runs_at_once_then_every_interval() {
        let mut s = sched();
        s.lease("bar", Machine, 1000, None, 0);
        assert_eq!(tick(&mut s, 0, 10), [Machine]);
        assert_eq!(s.next_wakeup(0), Some(1000));
        assert!(tick(&mut s, 500, 10).is_empty());
        assert_eq!(tick(&mut s, 1000, 10), [Machine]);
        assert_eq!(s.next_wakeup(1000), Some(2000));
    }

    #[test]
    fn the_shortest_live_lease_wins_and_releasing_it_slows_the_source() {
        let mut s = sched();
        s.lease("janitor", Panes, 30_000, None, 0);
        tick(&mut s, 0, 10);
        assert_eq!(s.effective_interval_ms(Panes, 0), Some(30_000));
        assert_eq!(s.next_wakeup(0), Some(30_000));
        // A viewer arrives: the source is pulled in to the new interval.
        s.lease("badge", Panes, 1000, None, 100);
        assert_eq!(s.effective_interval_ms(Panes, 100), Some(1000));
        assert_eq!(s.next_wakeup(100), Some(1000));
        tick(&mut s, 1000, 10);
        s.release("badge", 1100);
        assert_eq!(s.effective_interval_ms(Panes, 1100), Some(30_000));
        // No extra run at the old fast schedule: its next is 30 s after its last.
        assert_eq!(s.next_wakeup(1100), Some(31_000));
        assert!(tick(&mut s, 2000, 10).is_empty());
    }

    #[test]
    fn releasing_the_last_lease_stops_the_source() {
        let mut s = sched();
        s.lease("badge", Panes, 1000, None, 0);
        tick(&mut s, 0, 10);
        s.release_source("badge", Panes, 500);
        assert_eq!(s.next_wakeup(500), None);
        assert!(tick(&mut s, 1000, 10).is_empty());
    }

    #[test]
    fn renewing_a_lease_with_a_longer_interval_pushes_the_next_run_out() {
        let mut s = sched();
        s.lease("bar", Machine, 1000, None, 0);
        tick(&mut s, 0, 10);
        s.lease("bar", Machine, 5000, None, 200);
        assert_eq!(s.next_wakeup(200), Some(5000));
    }

    #[test]
    fn a_source_never_runs_faster_than_its_own_minimum() {
        let mut s = sched();
        s.lease("fast", Machine, 10, None, 0);
        assert_eq!(s.effective_interval_ms(Machine, 0), Some(200));
    }

    #[test]
    fn an_expired_lease_stops_the_source_and_a_new_one_restarts_it_at_once() {
        let mut s = sched();
        s.lease("popover", Machine, 1000, Some(3000), 0);
        assert_eq!(tick(&mut s, 0, 10), [Machine]);
        assert_eq!(tick(&mut s, 2000, 10), [Machine]);
        assert!(tick(&mut s, 3000, 10).is_empty(), "expired exactly at 3000");
        assert_eq!(s.next_wakeup(3000), None);
        s.lease("popover", Machine, 1000, Some(9000), 5000);
        assert_eq!(tick(&mut s, 5000, 10), [Machine], "no waiting out the old schedule");
    }

    #[test]
    fn renewing_a_lease_replaces_it() {
        let mut s = sched();
        s.lease("a", Machine, 1000, Some(2000), 0);
        s.lease("a", Machine, 1000, Some(8000), 1500);
        assert_eq!(s.status(1500)[0].holders, ["a"]);
        assert_eq!(tick(&mut s, 3000, 10), [Machine]);
    }

    #[test]
    fn sources_due_close_together_run_in_one_pass() {
        let mut s = sched();
        s.lease("a", Machine, 1000, None, 0);
        s.lease("a", Panes, 1000, None, 0);
        tick(&mut s, 0, 10);
        tick(&mut s, 0, 10);
        // Machine is due at 1000, Panes at 1000 too; one arriving 30 ms late is
        // still in the pass, one 200 ms late is not.
        s.finished(Panes, 30, 10);
        assert_eq!(s.due(1000), [Machine, Panes]);
    }

    #[test]
    fn what_a_due_source_reads_runs_with_it_even_without_a_lease() {
        let mut s = Scheduler::new(vec![spec(Machine, 200, 100, &[]), spec(Panes, 500, 200, &[Machine])], 50);
        s.lease("badge", Panes, 1000, None, 0);
        assert_eq!(s.due(0), [Machine, Panes]);
        // Not leased itself, so it isn't in the status as wanted.
        assert!(s.status(0)[0].holders.is_empty());
    }

    #[test]
    #[should_panic(expected = "declared after")]
    fn a_need_declared_later_is_refused() {
        Scheduler::new(vec![spec(Panes, 500, 200, &[Machine]), spec(Machine, 200, 100, &[])], 50);
    }

    #[test]
    fn a_source_over_budget_is_stretched_up_to_four_times_and_eases_back_when_calm() {
        let mut s = sched();
        s.lease("bar", Panes, 1000, None, 0);
        let mut now = 0;
        for expected in [2, 4, 4] {
            tick(&mut s, now, 400); // budget 200
            assert_eq!(s.status(now)[1].stretch, expected);
            now = s.next_wakeup(now).unwrap();
        }
        assert_eq!(s.status(now)[1].overruns, 3);
        assert_eq!(s.effective_interval_ms(Panes, now), Some(4000));
        // Three quick runs in a row halve it, three more halve it again.
        for _ in 0..3 {
            tick(&mut s, now, 50);
            now = s.next_wakeup(now).unwrap();
        }
        assert_eq!(s.status(now)[1].stretch, 2);
        for _ in 0..3 {
            tick(&mut s, now, 50);
            now = s.next_wakeup(now).unwrap();
        }
        assert_eq!(s.status(now)[1].stretch, 1);
    }

    #[test]
    fn a_run_in_the_middle_of_the_budget_neither_stretches_nor_eases() {
        let mut s = sched();
        s.lease("bar", Panes, 1000, None, 0);
        tick(&mut s, 0, 400);
        assert_eq!(s.status(0)[1].stretch, 2);
        let mut now = s.next_wakeup(0).unwrap();
        for _ in 0..6 {
            tick(&mut s, now, 150); // over half the budget, under all of it
            now = s.next_wakeup(now).unwrap();
        }
        assert_eq!(s.status(now)[1].stretch, 2);
    }

    #[test]
    fn a_late_tick_is_counted_and_never_made_up_for() {
        let mut s = sched();
        s.lease("bar", Machine, 1000, None, 0);
        tick(&mut s, 0, 10);
        // The next was due at 1000 but the loop only got round to it at 4500.
        assert_eq!(tick(&mut s, 4500, 10), [Machine]);
        assert_eq!(s.status(4500)[0].skipped, 3);
        assert_eq!(s.next_wakeup(4500), Some(5500), "one interval on, not four runs in a burst");
    }

    #[test]
    fn a_run_longer_than_its_interval_is_not_started_again_before_it_ends() {
        let mut s = sched();
        s.lease("bar", Machine, 200, None, 0);
        tick(&mut s, 0, 900);
        assert!(s.next_wakeup(0).unwrap() >= 900);
    }

    #[test]
    fn status_names_holders_interval_and_counts() {
        let mut s = sched();
        s.lease("bar", Machine, 1000, None, 0);
        s.lease("popover", Machine, 2000, None, 0);
        tick(&mut s, 0, 10);
        let m = &s.status(0)[0];
        assert_eq!(m.id, Machine);
        assert_eq!(m.holders, ["bar", "popover"]);
        assert_eq!((m.interval_ms, m.runs, m.last_took_ms), (Some(1000), 1, 10));
        let p = &s.status(0)[1];
        assert_eq!((p.id, p.interval_ms, p.runs), (Panes, None, 0));
    }
}
