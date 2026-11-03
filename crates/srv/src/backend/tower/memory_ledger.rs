// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The last hour of AgentMux's memory, and what grows unusually in it
//! (SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md §7).
//!
//! A ring of 360 points at 10-second steps for the total, each part and each
//! row, in bytes. It lives in srv's memory only, starts empty on a restart,
//! and is fed by whichever sample comes: Tower's own (every 2 s, thinned to
//! the steps) or the timer in `app_api::tower_pane` when no Tower is open, so
//! a Tower opened later shows the past hour.
//!
//! Warnings stay inside Tower (spec D6): nothing here notifies anyone or
//! acts on the memory supervisor.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::backend::rpc_types::{
    TowerMemory, TowerMemoryGrowth, TowerMemoryHistory, TowerMemoryPartId, TowerMemorySeries,
};

/// The gap between points.
pub const STEP_MS: u64 = 10_000;
/// An hour of points.
pub const POINTS: usize = 360;
/// With no Tower open the timer samples every this many steps (30 s): one
/// sample costs about 15-25 ms on a busy Windows machine (debug build), which
/// every 10 s is over the 0.1% of a core the spec allows, and every 30 s is
/// under it (spec section 5.4).
pub const IDLE_EVERY_STEPS: u64 = 3;
/// The most series kept. About 30 are normal (four parts, a row each for the
/// agents, panes and AgentMux programs); the cap keeps a runaway of rows
/// (a build starting hundreds of differently named processes) from growing it.
const MAX_SERIES: usize = 400;

/// "Grows unusually" (spec §7.3): all of these over the last [`Self::window`]
/// points. First guesses to tune with the memory lab's noise data, kept in
/// one place on purpose.
pub struct GrowthRule {
    /// At least this big now, bytes.
    pub floor: u64,
    /// The points looked at: 30 minutes.
    pub window: usize,
    /// The fewest points a line needs before it can be flagged: ten minutes.
    /// A program starting up grows from nothing to its size in its first
    /// minutes, which is not a leak.
    pub min_points: usize,
    /// Grown by at least this share over its lowest point in the window…
    pub ratio: f64,
    /// …and by at least this many bytes.
    pub gain: u64,
    /// Not already coming back: still at least this share of the window's highest.
    pub retreat: f64,
}

const MB: u64 = 1024 * 1024;

pub const GROWTH: GrowthRule = GrowthRule {
    floor: 150 * MB,
    window: 180,
    min_points: 60,
    ratio: 1.5,
    gain: 100 * MB,
    retreat: 0.9,
};

/// Whether `points` (one per step, newest last) grows unusually: from the
/// lowest point in the window, to now, and how many whole minutes ago that
/// lowest point was.
pub fn grows_unusually(points: &[u64], rule: &GrowthRule) -> Option<(u64, u64, u32)> {
    if points.len() < rule.min_points {
        return None;
    }
    let window = &points[points.len().saturating_sub(rule.window)..];
    let now = *window.last()?;
    if now < rule.floor {
        return None;
    }
    // The latest lowest point: when the growth began, not when it was flat.
    let (at, low) = window
        .iter()
        .enumerate()
        .rev()
        .min_by_key(|(_, v)| **v)
        .map(|(i, v)| (i, *v))?;
    let high = *window.iter().max()?;
    let grew = now.saturating_sub(low);
    if grew < rule.gain
        || (now as f64) < (low as f64) * rule.ratio
        || (now as f64) < (high as f64) * rule.retreat
    {
        return None;
    }
    let minutes = ((window.len() - 1 - at) as u64 * STEP_MS / 60_000) as u32;
    Some((low, now, minutes))
}

/// One key's points, contiguous from `start`.
struct Series {
    /// The step (`ms / STEP_MS`) of the first point.
    start: u64,
    points: VecDeque<u64>,
}

impl Series {
    fn end(&self) -> u64 {
        self.start + self.points.len() as u64 - 1
    }
}

#[derive(Default)]
pub struct Ledger {
    series: HashMap<String, Series>,
    /// The latest step recorded.
    last: Option<u64>,
    /// When the first point was taken, ms.
    since_ms: u64,
}

fn part_key(part: TowerMemoryPartId) -> &'static str {
    match part {
        TowerMemoryPartId::Core => "part:core",
        TowerMemoryPartId::Panes => "part:panes",
        TowerMemoryPartId::Agents => "part:agents",
        TowerMemoryPartId::NotPlaced => "part:not_placed",
    }
}

impl Ledger {
    /// Record `mem` for the step `at_ms` falls in. A step already recorded is
    /// left alone, which is how Tower's 2-second samples thin to 10 seconds.
    pub fn record(&mut self, at_ms: u64, mem: &TowerMemory) {
        let step = at_ms / STEP_MS;
        if self.last.is_some_and(|l| step <= l) {
            return;
        }
        if self.last.is_none() {
            self.since_ms = at_ms;
        }
        self.last = Some(step);
        self.push("total", step, mem.total);
        for part in &mem.parts {
            self.push(part_key(part.part), step, part.mem);
            for row in &part.rows {
                self.push(&row.key, step, row.mem);
            }
        }
        // A key not seen for an hour is gone; one that stopped sooner keeps
        // its points, so its line ends where the process ended.
        self.series.retain(|_, s| s.end() + POINTS as u64 > step);
    }

    fn push(&mut self, key: &str, step: u64, bytes: u64) {
        if !self.series.contains_key(key) && self.series.len() >= MAX_SERIES {
            return;
        }
        let s = self
            .series
            .entry(key.to_string())
            .or_insert_with(|| Series {
                start: step,
                points: VecDeque::with_capacity(POINTS),
            });
        // Steps nobody sampled (no Tower is open and the timer runs every
        // [`IDLE_EVERY_STEPS`], or the machine slept) are drawn as a straight
        // line to the new point rather than a stair or a cliff.
        if let Some(&from) = s.points.back() {
            let missed = step.saturating_sub(s.end() + 1);
            for k in 1..=missed {
                let v = from as f64 + (bytes as f64 - from as f64) * k as f64 / (missed + 1) as f64;
                s.points.push_back(v.round() as u64);
                if s.points.len() > POINTS {
                    s.points.pop_front();
                    s.start += 1;
                }
            }
        }
        s.points.push_back(bytes);
        if s.points.len() > POINTS {
            s.points.pop_front();
            s.start += 1;
        }
    }

    /// What grows unusually now: the keys still being recorded whose
    /// last half hour passes [`GROWTH`]. Part keys are `part:<id>`.
    pub fn growth(&self) -> Vec<TowerMemoryGrowth> {
        let Some(latest) = self.last else {
            return vec![];
        };
        let mut out: Vec<TowerMemoryGrowth> = self
            .series
            .iter()
            .filter(|(_, s)| s.end() == latest)
            .filter_map(|(key, s)| {
                let points: Vec<u64> = s.points.iter().copied().collect();
                let (from, to, minutes) = grows_unusually(&points, &GROWTH)?;
                Some(TowerMemoryGrowth {
                    key: key.clone(),
                    from,
                    to,
                    minutes,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            (b.to - b.from)
                .cmp(&(a.to - a.from))
                .then_with(|| a.key.cmp(&b.key))
        });
        out
    }

    /// Every series, for the charts.
    pub fn history(&self) -> TowerMemoryHistory {
        let mut series: Vec<TowerMemorySeries> = self
            .series
            .iter()
            .map(|(key, s)| TowerMemorySeries {
                key: key.clone(),
                start_ms: s.start * STEP_MS,
                points: s.points.iter().copied().collect(),
            })
            .collect();
        series.sort_by(|a, b| a.key.cmp(&b.key));
        TowerMemoryHistory {
            step_ms: STEP_MS as u32,
            since_ms: self.since_ms,
            series,
        }
    }
}

struct Shared {
    ledger: Ledger,
    /// When a sample last reached the ledger, thinned or not.
    fed: Option<Instant>,
}

fn shared() -> MutexGuard<'static, Shared> {
    static GLOBAL: OnceLock<Mutex<Shared>> = OnceLock::new();
    GLOBAL
        .get_or_init(|| {
            Mutex::new(Shared {
                ledger: Ledger::default(),
                fed: None,
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Put a sample in srv's ledger and attach what it knows: the keys that grow
/// unusually, and the hour when `with_history`.
pub fn feed(mem: &mut TowerMemory, at_ms: u64, with_history: bool) {
    let mut sh = shared();
    sh.fed = Some(Instant::now());
    sh.ledger.record(at_ms, mem);
    mem.growth = sh.ledger.growth();
    mem.history = with_history.then(|| sh.ledger.history());
}

/// Whether a sample reached the ledger within `within`: the timer samples
/// only when none did.
pub fn fed_within(within: Duration) -> bool {
    shared().fed.is_some_and(|f| f.elapsed() < within)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{TowerMemoryPart, TowerMemoryRow};

    fn row(key: &str, mem: u64) -> TowerMemoryRow {
        TowerMemoryRow {
            key: key.to_string(),
            label: key.to_string(),
            mem,
            processes: 1,
            kind: None,
            task: None,
            children: vec![],
        }
    }

    /// A sample whose Core part holds one row.
    fn sample(core: u64, rows: &[(&str, u64)]) -> TowerMemory {
        let parts = [
            TowerMemoryPartId::Core,
            TowerMemoryPartId::Panes,
            TowerMemoryPartId::Agents,
            TowerMemoryPartId::NotPlaced,
        ]
        .into_iter()
        .map(|part| TowerMemoryPart {
            part,
            mem: if part == TowerMemoryPartId::Core {
                core
            } else {
                0
            },
            processes: 1,
            rows: if part == TowerMemoryPartId::Core {
                rows.iter().map(|(k, m)| row(k, *m)).collect()
            } else {
                vec![]
            },
        })
        .collect();
        TowerMemory {
            total: core,
            processes: 1,
            unreadable: 0,
            parts,
            other_instances: None,
            metric: "private working set".to_string(),
            growth: vec![],
            history: None,
        }
    }

    fn get<'a>(h: &'a TowerMemoryHistory, key: &str) -> &'a TowerMemorySeries {
        h.series
            .iter()
            .find(|s| s.key == key)
            .unwrap_or_else(|| panic!("no series {key}"))
    }

    fn mbs(v: &[u64]) -> Vec<u64> {
        v.iter().map(|m| m * MB).collect()
    }

    /// `n` points ramping from `a` to `b` MB.
    fn ramp(a: u64, b: u64, n: usize) -> Vec<u64> {
        (0..n)
            .map(|i| ((a as i64 + (b as i64 - a as i64) * i as i64 / (n as i64 - 1)) as u64) * MB)
            .collect()
    }

    // ── thinning, size, keys, restarts ─────────────────────────────────────

    #[test]
    fn two_second_samples_thin_to_ten_second_points() {
        let mut l = Ledger::default();
        for i in 0..30u64 {
            l.record(
                1_000_000 + i * 2_000,
                &sample(100 + i, &[("core:srv", 100 + i)]),
            );
        }
        // 60 s of 2 s samples: six steps (or seven if the first lands mid-step).
        let n = get(&l.history(), "total").points.len();
        assert!((6..=7).contains(&n), "{n} points");
    }

    #[test]
    fn a_step_already_recorded_is_left_alone() {
        let mut l = Ledger::default();
        l.record(50_000, &sample(1, &[]));
        l.record(52_000, &sample(2, &[]));
        l.record(50_000, &sample(3, &[]));
        assert_eq!(get(&l.history(), "total").points, vec![1]);
    }

    #[test]
    fn the_ring_keeps_an_hour_and_its_start_moves_with_it() {
        let mut l = Ledger::default();
        for i in 0..(POINTS as u64 + 40) {
            l.record(i * STEP_MS, &sample(i, &[]));
        }
        let h = l.history();
        let total = get(&h, "total");
        assert_eq!(total.points.len(), POINTS);
        assert_eq!(*total.points.last().unwrap(), POINTS as u64 + 39);
        assert_eq!(
            total.start_ms,
            40 * STEP_MS,
            "the oldest 40 points fell off"
        );
        assert_eq!(h.step_ms, 10_000);
        assert_eq!(
            h.since_ms, 0,
            "since the first sample, not the first point kept"
        );
    }

    #[test]
    fn parts_and_rows_have_their_own_series() {
        let mut l = Ledger::default();
        l.record(0, &sample(300, &[("core:gpu", 100), ("core:srv", 200)]));
        let h = l.history();
        assert_eq!(get(&h, "total").points, vec![300]);
        assert_eq!(get(&h, "part:core").points, vec![300]);
        assert_eq!(
            get(&h, "part:agents").points,
            vec![0],
            "a zero part is kept"
        );
        assert_eq!(get(&h, "core:gpu").points, vec![100]);
        assert_eq!(get(&h, "core:srv").points, vec![200]);
    }

    #[test]
    fn a_row_that_ended_keeps_its_line_for_an_hour_then_goes() {
        let mut l = Ledger::default();
        l.record(0, &sample(10, &[("agent:a", 10)]));
        l.record(STEP_MS, &sample(0, &[]));
        let h = l.history();
        assert_eq!(
            get(&h, "agent:a").points,
            vec![10],
            "the line ends where the process ended"
        );
        assert_eq!(get(&h, "total").points.len(), 2);
        for i in 2..(POINTS as u64) {
            l.record(i * STEP_MS, &sample(0, &[]));
        }
        assert!(
            l.history().series.iter().any(|s| s.key == "agent:a"),
            "still within the hour"
        );
        l.record(POINTS as u64 * STEP_MS, &sample(0, &[]));
        assert!(
            !l.history().series.iter().any(|s| s.key == "agent:a"),
            "not seen for an hour: dropped"
        );
    }

    #[test]
    fn missed_steps_are_drawn_as_a_straight_line() {
        let mut l = Ledger::default();
        l.record(0, &sample(5, &[]));
        l.record(4 * STEP_MS, &sample(9, &[]));
        assert_eq!(get(&l.history(), "total").points, vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn a_new_ledger_is_empty_and_says_nothing_grows() {
        let l = Ledger::default();
        assert!(l.history().series.is_empty());
        assert!(l.growth().is_empty());
    }

    #[test]
    fn the_series_are_capped() {
        let mut l = Ledger::default();
        let names: Vec<String> = (0..MAX_SERIES + 50).map(|i| format!("np:p{i}")).collect();
        let rows: Vec<(&str, u64)> = names.iter().map(|n| (n.as_str(), 1)).collect();
        l.record(0, &sample(1, &rows));
        assert!(l.history().series.len() <= MAX_SERIES);
        assert!(
            l.history().series.iter().any(|s| s.key == "total"),
            "the total is always kept"
        );
    }

    // ── the growth flag, the cases the spec pins ───────────────────────────

    fn flagged(points: &[u64]) -> bool {
        grows_unusually(points, &GROWTH).is_some()
    }

    #[test]
    fn steady_800_mb_is_not_flagged() {
        assert!(!flagged(&mbs(&[800; 180])));
    }

    #[test]
    fn growing_from_200_to_400_mb_in_20_minutes_is_flagged() {
        // Flat for ten minutes, then up over twenty.
        let mut p = mbs(&[200; 60]);
        p.extend(ramp(200, 400, 120));
        let (from, to, minutes) = grows_unusually(&p, &GROWTH).expect("flagged");
        assert_eq!((from / MB, to / MB), (200, 400));
        assert!(
            (19..=20).contains(&minutes),
            "counted from where it began to grow: {minutes}"
        );
    }

    #[test]
    fn falling_from_900_to_300_then_350_is_not_flagged() {
        let mut p = ramp(900, 300, 120);
        p.extend(ramp(300, 350, 60));
        assert!(!flagged(&p), "+50 MB over its low is under 100 MB");
    }

    #[test]
    fn going_from_100_to_160_mb_is_not_flagged() {
        let mut p = mbs(&[100; 120]);
        p.extend(ramp(100, 160, 60));
        assert!(!flagged(&p), "+60% but only +60 MB");
    }

    #[test]
    fn going_from_120_to_300_mb_is_flagged() {
        let mut p = mbs(&[120; 60]);
        p.extend(ramp(120, 300, 120));
        assert!(flagged(&p));
    }

    #[test]
    fn small_is_never_flagged_whatever_the_share() {
        assert!(!flagged(&ramp(10, 140, 180)), "under 150 MB now");
    }

    #[test]
    fn already_coming_back_is_not_flagged() {
        let mut p = ramp(200, 600, 120);
        p.extend(ramp(600, 400, 60));
        assert!(!flagged(&p), "fell by a third from its highest");
        let mut q = ramp(200, 600, 120);
        q.extend(ramp(600, 570, 60));
        assert!(flagged(&q), "a small dip from the top is still growth");
    }

    #[test]
    fn a_line_still_starting_up_is_not_flagged() {
        assert!(
            !flagged(&ramp(100, 400, 30)),
            "five minutes of points: a program warming up"
        );
        assert!(
            flagged(&ramp(100, 400, 60)),
            "ten minutes of the same climb is"
        );
    }

    #[test]
    fn only_the_last_half_hour_counts() {
        // Grew long ago, flat since: the last 30 minutes are flat.
        let mut p = ramp(100, 600, 120);
        p.extend(mbs(&[600; 200]));
        assert!(!flagged(&p));
    }

    #[test]
    fn the_ledger_flags_a_growing_row_and_its_part() {
        let mut l = Ledger::default();
        let mut total = 0;
        for i in 0..120u64 {
            let mem = 200 * MB + 200 * MB * i / 119;
            total = mem;
            l.record(i * STEP_MS, &sample(mem, &[("core:srv", mem)]));
        }
        let g = l.growth();
        let keys: Vec<&str> = g.iter().map(|g| g.key.as_str()).collect();
        assert!(
            keys.contains(&"core:srv") && keys.contains(&"part:core") && keys.contains(&"total"),
            "{keys:?}"
        );
        assert!(!keys.contains(&"part:agents"));
        assert_eq!(g[0].to, total);
        assert_eq!(g[0].from, 200 * MB);
    }

    #[test]
    fn a_row_that_ended_is_not_flagged() {
        let mut l = Ledger::default();
        for i in 0..120u64 {
            let mem = 200 * MB + 200 * MB * i / 119;
            l.record(i * STEP_MS, &sample(0, &[("agent:gone", mem)]));
        }
        l.record(120 * STEP_MS, &sample(0, &[]));
        assert!(l.growth().iter().all(|g| g.key != "agent:gone"));
    }
}
