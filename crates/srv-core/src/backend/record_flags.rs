// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The rules that read the record, so nobody has to look
//! (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §7, task #47820).
//!
//! Once an hour, when the hour that ended is rolled up, two rules run over what the record already holds. Each
//! thing they find is one event of kind `flag`, written beside the stalls (`events-YYYY-MM.jsonl`, kept a year):
//!
//! - **stalls**: more than three stalls of 5 s or more in the hour, or one over 20 s;
//! - **growth**: a size whose trend rose over each of the last two days by more than the margin and by more
//!   than its floor, and that more content does not explain.
//!
//! A flag is a line in the record: it is read with the events (`App record`, kind `events`) and stops nothing.
//! What is said of it, to the person and to the machine's steward, is `record_notice`.
//!
//! What the growth rule reads, and what it leaves out:
//! - the `core` part and each `core:` row (srv, the host, the GPU process), only over two days in which srv
//!   did not start again: a start lowers a size and the climb back is not a leak;
//! - each window's page: its JS heap and its element count, against its mounted rows. A window that holds
//!   more rows holds more, so a rise the rows explain is not flagged;
//! - not the rows of agents and terminals (an agent CLI's memory is not AgentMux's), and not a renderer's
//!   process row, which the record cannot set against a window's content.

use std::path::Path;

use serde_json::{json, Value};

use super::record::{read_events, read_rollups, Rollup};

/// The kind of a flag's line among the record's events.
pub const KIND: &str = "flag";

const HOUR_MS: u64 = 3_600_000;
const DAY_MS: u64 = 24 * HOUR_MS;

/// The events that are stalls: the host's UI thread, and srv's runtime.
const STALL_KINDS: [&str; 2] = ["ui_stall", "srv_stall"];
/// A stall this long counts; more than [`STALLS_MANY`] of them in one hour is a flag.
const STALL_COUNTED_MS: u64 = 5_000;
const STALLS_MANY: usize = 3;
/// One stall longer than this is a flag by itself.
const STALL_LONG_MS: u64 = 20_000;

/// A size is flagged when its trend gained more than this share of its level in a day, two days running.
const MARGIN: f64 = 0.20;
/// A day is read only with this many of its 24 hours present.
const HOURS_NEEDED: usize = 18;
/// A gain smaller than this never flags: it is inside what the samples move by themselves.
const FLOOR_BYTES: f64 = 64.0 * 1024.0 * 1024.0;
const FLOOR_HEAP_MB: f64 = 32.0;
const FLOOR_ELEMENTS: f64 = 5_000.0;
/// Content explains a rise when it rose by at least this share of the size's own rise.
const CONTENT_EXPLAINS: f64 = 0.5;

/// One thing a rule found.
#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    /// `stalls`, `long_stall` or `growth`.
    pub rule: &'static str,
    /// What it is about: a stall's kind, or a series (`core`, `core:srv`, `page:main:heap`).
    pub key: String,
    /// The numbers behind it, for the line.
    pub fields: Value,
}

impl Flag {
    /// The fields of the flag's event: the rule, the key and the numbers.
    fn line(&self) -> Value {
        let mut line = self.fields.clone();
        if let Value::Object(map) = &mut line {
            map.insert("rule".into(), self.rule.into());
            map.insert("key".into(), self.key.as_str().into());
        }
        line
    }
}

/// The stall rule over one hour's events: per kind, more than [`STALLS_MANY`] stalls of [`STALL_COUNTED_MS`] or
/// more, or one over [`STALL_LONG_MS`]. One flag per kind at most; many stalls is the one said when both hold.
pub fn stall_flags(events: &[Value]) -> Vec<Flag> {
    let mut out = Vec::new();
    for kind in STALL_KINDS {
        let lengths: Vec<u64> = events.iter().filter(|e| e["kind"] == kind).filter_map(|e| e["ms"].as_u64()).collect();
        let Some(worst) = lengths.iter().copied().max() else { continue };
        let counted = lengths.iter().filter(|ms| **ms >= STALL_COUNTED_MS).count();
        let rule = if counted > STALLS_MANY {
            "stalls"
        } else if worst > STALL_LONG_MS {
            "long_stall"
        } else {
            continue;
        };
        out.push(Flag { rule, key: kind.to_string(), fields: json!({ "n": counted, "worst_ms": worst }) });
    }
    out
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 1 { v[mid] } else { (v[mid - 1] + v[mid]) / 2.0 })
}

/// The Theil-Sen slope of `points` (x, y): the median of the slopes between every two points. A window recycle
/// or one odd hour moves a least-squares line; it does not move this one. None under two distinct x.
fn theil_sen(points: &[(f64, f64)]) -> Option<f64> {
    let mut slopes = Vec::with_capacity(points.len() * points.len().saturating_sub(1) / 2);
    for (i, a) in points.iter().enumerate() {
        for b in &points[i + 1..] {
            if b.0 != a.0 {
                slopes.push((b.1 - a.1) / (b.0 - a.0));
            }
        }
    }
    median(slopes)
}

/// How a figure moved over one day: its middle, and what its trend gained over the 24 hours.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Day {
    level: f64,
    rise: f64,
}

impl Day {
    /// The gain as a share of the level; 0 for a level of 0.
    fn share(&self) -> f64 {
        if self.level > 0.0 {
            self.rise / self.level
        } else {
            0.0
        }
    }
}

/// The day that starts at `from`, of a series of (hour start, value). None with too few of its hours.
fn day(series: &[(u64, f64)], from: u64) -> Option<Day> {
    let points: Vec<(f64, f64)> =
        series.iter().filter(|(h, _)| *h >= from && *h < from + DAY_MS).map(|(h, v)| ((*h - from) as f64 / HOUR_MS as f64, *v)).collect();
    if points.len() < HOURS_NEEDED {
        return None;
    }
    Some(Day { level: median(points.iter().map(|p| p.1).collect())?, rise: theil_sen(&points)? * 24.0 })
}

/// A size over the hours, with what it is counted in and, for a page, the content it is read against.
struct Series {
    key: String,
    unit: &'static str,
    floor: f64,
    values: Vec<(u64, f64)>,
    content: Option<Vec<(u64, f64)>>,
}

fn push(all: &mut Vec<Series>, key: String, unit: &'static str, floor: f64, hour: u64, value: f64, content: Option<f64>) {
    let at = match all.iter().position(|s| s.key == key) {
        Some(at) => at,
        None => {
            all.push(Series { key, unit, floor, values: Vec::new(), content: content.map(|_| Vec::new()) });
            all.len() - 1
        }
    };
    all[at].values.push((hour, value));
    if let (Some(list), Some(c)) = (all[at].content.as_mut(), content) {
        list.push((hour, c));
    }
}

/// The series the growth rule reads, from the rollups of its two days (the hourly medians).
fn series_of(rollups: &[Rollup]) -> Vec<Series> {
    // Process sizes are read only when srv ran through both days in one version.
    let one_run = rollups.iter().all(|r| r.restarts == 0) && rollups.windows(2).all(|w| w[0].v == w[1].v && w[1].up_s >= w[0].up_s);
    let mut all = Vec::new();
    for r in rollups {
        if one_run {
            for (k, stat) in r.parts.iter().filter(|(k, _)| k == "core") {
                push(&mut all, k.clone(), "bytes", FLOOR_BYTES, r.h, stat.1 as f64, None);
            }
            for (k, stat) in r.rows.iter().filter(|(k, _)| k.starts_with("core:")) {
                push(&mut all, k.clone(), "bytes", FLOOR_BYTES, r.h, stat.1 as f64, None);
            }
        }
        for page in &r.pages {
            let rows = Some(page.rows.1 as f64);
            if let Some(heap) = page.heap {
                push(&mut all, format!("page:{}:heap", page.w), "mb", FLOOR_HEAP_MB, r.h, heap.1 as f64, rows);
            }
            push(&mut all, format!("page:{}:dom", page.w), "elements", FLOOR_ELEMENTS, r.h, page.dom.1 as f64, rows);
        }
    }
    all
}

/// The growth rule over the two days that end at `end` (an hour's start): a series whose trend gained more than
/// [`MARGIN`] of its level and more than its floor on each day, where its content (a page's rows) did not rise
/// by at least [`CONTENT_EXPLAINS`] of that share on either day.
pub fn growth_flags(rollups: &[Rollup], end: u64) -> Vec<Flag> {
    let start = end.saturating_sub(2 * DAY_MS);
    let window: Vec<Rollup> = rollups.iter().filter(|r| r.h >= start && r.h < end).cloned().collect();
    let mut out = Vec::new();
    for s in series_of(&window) {
        let (Some(before), Some(last)) = (day(&s.values, start), day(&s.values, start + DAY_MS)) else { continue };
        let grew = |d: &Day| d.rise > s.floor && d.share() > MARGIN;
        if !grew(&before) || !grew(&last) {
            continue;
        }
        let content = s.content.as_ref().map(|c| (day(c, start), day(c, start + DAY_MS)));
        let explained = |d: &Day, c: Option<Day>| c.is_some_and(|c| c.share() >= d.share() * CONTENT_EXPLAINS);
        if let Some((c_before, c_last)) = content {
            if explained(&before, c_before) || explained(&last, c_last) {
                continue;
            }
        }
        let mut fields = json!({
            "unit": s.unit,
            "level": last.level.round() as u64,
            "rise": last.rise.round() as u64,
            "rise_before": before.rise.round() as u64,
        });
        if let Some((_, Some(c))) = content {
            fields["content_pct"] = json!((c.share() * 100.0).round() as i64);
        }
        out.push(Flag { rule: "growth", key: s.key, fields });
    }
    out
}

/// Whether `events` already hold this flag: a stall flag for the hour that ended at `end`, a growth flag for
/// the same series within the last day (a series that keeps growing is said once a day, not once an hour).
fn already(events: &[Value], flag: &Flag, end: u64) -> bool {
    events.iter().filter(|e| e["kind"] == KIND && e["rule"] == flag.rule && e["key"] == flag.key.as_str()).any(|e| {
        let t = e["t"].as_u64().unwrap_or(0);
        if flag.rule == "growth" {
            t + DAY_MS > end
        } else {
            t == end
        }
    })
}

/// What the rules find for the hour that ended at or before `now_ms`, less what the record already says.
pub fn found(dir: &Path, now_ms: u64) -> (u64, Vec<Flag>) {
    let end = now_ms / HOUR_MS * HOUR_MS;
    let (_, events) = read_events(dir, end.saturating_sub(DAY_MS), end, usize::MAX);
    let hour: Vec<Value> =
        events.iter().filter(|e| e["t"].as_u64().is_some_and(|t| t + HOUR_MS >= end && t < end)).cloned().collect();
    let (_, rollups) = read_rollups(dir, end.saturating_sub(2 * DAY_MS), end, 48 + 1);
    let mut flags = stall_flags(&hour);
    flags.extend(growth_flags(&rollups, end));
    flags.retain(|f| !already(&events, f, end));
    (end, flags)
}

/// How long a flag is still said after its hour: a growing size is found again each day while it grows, so its
/// flag lasts until the next would be written; a bad hour of stalls is over when the next hour or two are quiet.
const GROWTH_SAID_MS: u64 = DAY_MS + HOUR_MS;
const STALLS_SAID_MS: u64 = 2 * HOUR_MS;

/// The flags among `events` that are still said at `now_ms`, oldest first: the newest of each rule and key,
/// while it is younger than its rule's time.
pub fn still_said(events: &[Value], now_ms: u64) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for e in events.iter().filter(|e| e["kind"] == KIND) {
        let Some(t) = e["t"].as_u64() else { continue };
        let lasts = if e["rule"] == "growth" { GROWTH_SAID_MS } else { STALLS_SAID_MS };
        if t > now_ms || t + lasts <= now_ms {
            continue;
        }
        match out.iter_mut().find(|o| o["rule"] == e["rule"] && o["key"] == e["key"]) {
            Some(same) if same["t"].as_u64().unwrap_or(0) <= t => *same = e.clone(),
            Some(_) => {}
            None => out.push(e.clone()),
        }
    }
    out.sort_by_key(|e| e["t"].as_u64().unwrap_or(0));
    out
}

/// What a look at the record found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Looked {
    /// The flags this look wrote.
    pub new: Vec<Flag>,
    /// Every flag still said now ([`still_said`]), these among them, as the record holds them.
    pub said: Vec<Value>,
}

/// Run the rules for the hour that just ended and write each new flag as an event at that hour's end. BLOCKING
/// (file reads and an append). A failure is logged: the rules are never a reason for the record to fail.
pub fn look(dir: &Path, now_ms: u64) -> Looked {
    let (end, flags) = found(dir, now_ms);
    let mut new = Vec::new();
    for flag in flags {
        tracing::info!(rule = flag.rule, key = %flag.key, fields = %flag.fields, "[record] flag");
        match agentmux_common::record_events::append_event(dir, end, KIND, flag.line()) {
            Ok(()) => new.push(flag),
            Err(e) => tracing::warn!(error = %e, "[record] a flag could not be written"),
        }
    }
    let (_, events) = read_events(dir, now_ms.saturating_sub(GROWTH_SAID_MS), now_ms, usize::MAX);
    Looked { new, said: still_said(&events, now_ms) }
}

#[cfg(test)]
mod tests {
    use super::super::record::{PageRoll, Stat};
    use super::*;

    const MB: u64 = 1024 * 1024;
    /// 2026-11-16 12:00 UTC, an hour's start.
    const END: u64 = 1_794_830_400_000;

    fn stall(kind: &str, t: u64, ms: u64) -> Value {
        json!({ "t": t, "kind": kind, "ms": ms })
    }

    fn flat(v: u64) -> Stat {
        Stat(v, v, v, v)
    }

    /// The rollup of the hour `back` hours before `END`, with a `core` part and one window's page.
    fn roll(back: u64, core: u64, page: Option<(u64, u64, u64)>) -> Rollup {
        Rollup {
            h: END - back * HOUR_MS,
            n: 12,
            v: "0.60.40".into(),
            up_s: (100 - back) * 3_600,
            restarts: 0,
            last_t: 0,
            cpu: None,
            mem: None,
            disk_free: None,
            am: None,
            parts: vec![("core".into(), flat(core)), ("agents".into(), flat(core * 10))],
            rows: vec![("agent:b1".into(), flat(core * 10))],
            pages: page.map(|(heap, dom, rows)| PageRoll { w: "main".into(), heap: Some(flat(heap)), dom: flat(dom), rows: flat(rows) }).into_iter().collect(),
        }
    }

    /// 48 hours ending at `END`, oldest first: `core(i)` and `page(i)` give hour `i`'s figures (0 is the oldest).
    fn two_days(core: impl Fn(u64) -> u64, page: impl Fn(u64) -> Option<(u64, u64, u64)>) -> Vec<Rollup> {
        (0..48).map(|i| roll(48 - i, core(i), page(i))).collect()
    }

    #[test]
    fn many_stalls_in_an_hour_are_a_flag_and_three_are_not() {
        let three: Vec<Value> = (0..3).map(|i| stall("ui_stall", i, 6_000)).collect();
        assert!(stall_flags(&three).is_empty());
        let mut four = three.clone();
        four.push(stall("ui_stall", 9, 5_000));
        four.push(stall("ui_stall", 10, 4_999));
        let flags = stall_flags(&four);
        assert_eq!(flags.len(), 1);
        assert_eq!((flags[0].rule, flags[0].key.as_str()), ("stalls", "ui_stall"));
        assert_eq!(flags[0].fields, json!({ "n": 4, "worst_ms": 6_000 }));
    }

    #[test]
    fn one_long_stall_is_a_flag_per_kind_and_twenty_seconds_exactly_is_not() {
        assert!(stall_flags(&[stall("srv_stall", 1, 20_000)]).is_empty());
        let flags = stall_flags(&[stall("srv_stall", 1, 20_001), stall("ui_stall", 2, 2_000), stall("other", 3, 99_000)]);
        assert_eq!(flags.len(), 1);
        assert_eq!((flags[0].rule, flags[0].key.as_str()), ("long_stall", "srv_stall"));
        assert_eq!(flags[0].fields, json!({ "n": 1, "worst_ms": 20_001 }));
    }

    #[test]
    fn the_slope_is_the_median_of_the_pairs_so_one_drop_does_not_move_it() {
        let mut line: Vec<(f64, f64)> = (0..24).map(|i| (i as f64, 100.0 + 2.0 * i as f64)).collect();
        assert_eq!(theil_sen(&line), Some(2.0));
        line[12].1 = 0.0;
        assert_eq!(theil_sen(&line), Some(2.0));
        assert_eq!(theil_sen(&[(1.0, 5.0)]), None);
        assert_eq!(theil_sen(&[(1.0, 5.0), (1.0, 9.0)]), None);
    }

    #[test]
    fn a_core_that_climbs_two_days_running_is_flagged_once_with_its_numbers() {
        // 300 MB rising 4 MB an hour: 96 MB a day, about a quarter of its level.
        let flags = growth_flags(&two_days(|i| (300 + 4 * i) * MB, |_| None), END);
        assert_eq!(flags.len(), 1);
        assert_eq!((flags[0].rule, flags[0].key.as_str()), ("growth", "core"));
        assert_eq!(flags[0].fields["unit"], "bytes");
        assert_eq!(flags[0].fields["rise"], 96 * MB);
        assert_eq!(flags[0].fields["rise_before"], 96 * MB);
        assert!(flags[0].fields.get("content_pct").is_none());
    }

    #[test]
    fn a_rise_on_one_day_only_or_under_the_floor_or_under_the_margin_is_not_flagged() {
        // Flat the first day, climbing the second.
        assert!(growth_flags(&two_days(|i| (300 + 4 * i.saturating_sub(24)) * MB, |_| None), END).is_empty());
        // 2 MB an hour is 48 MB a day: under the floor, though a quarter of 200 MB.
        assert!(growth_flags(&two_days(|i| (200 + 2 * i) * MB, |_| None), END).is_empty());
        // 4 MB an hour on 2 GB: over the floor, under the margin.
        assert!(growth_flags(&two_days(|i| (2_000 + 4 * i) * MB, |_| None), END).is_empty());
    }

    #[test]
    fn a_start_inside_the_two_days_takes_the_process_sizes_out() {
        let mut days = two_days(|i| (300 + 4 * i) * MB, |_| None);
        days[30].restarts = 1;
        assert!(growth_flags(&days, END).is_empty());
        let mut days = two_days(|i| (300 + 4 * i) * MB, |_| None);
        for r in days.iter_mut().skip(30) {
            r.v = "0.60.41".into();
        }
        assert!(growth_flags(&days, END).is_empty());
    }

    #[test]
    fn a_day_with_too_few_hours_is_not_read() {
        let mut days = two_days(|i| (300 + 4 * i) * MB, |_| None);
        days.retain(|r| r.h < END - 40 * HOUR_MS || r.h >= END - DAY_MS);
        assert!(growth_flags(&days, END).is_empty());
    }

    #[test]
    fn a_page_whose_heap_climbs_with_the_same_rows_is_flagged_and_one_whose_rows_climb_too_is_not() {
        let leak = growth_flags(&two_days(|_| 300 * MB, |i| Some((200 + 3 * i, 40_000, 5_000))), END);
        assert_eq!(leak.len(), 1);
        assert_eq!(leak[0].key, "page:main:heap");
        assert_eq!(leak[0].fields["unit"], "mb");
        assert_eq!(leak[0].fields["rise"], 72);
        assert_eq!(leak[0].fields["content_pct"], 0);
        // The same heap, with rows that rise as fast in proportion: more content, not a leak.
        assert!(growth_flags(&two_days(|_| 300 * MB, |i| Some((200 + 3 * i, 40_000, 5_000 + 75 * i))), END).is_empty());
    }

    #[test]
    fn a_page_s_elements_are_read_against_its_rows_too() {
        let flags = growth_flags(&two_days(|_| 300 * MB, |i| Some((200, 40_000 + 500 * i, 5_000))), END);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].key, "page:main:dom");
        assert_eq!(flags[0].fields["unit"], "elements");
        assert_eq!(flags[0].fields["rise"], 12_000);
    }

    #[test]
    fn a_stall_flag_is_said_for_two_hours_and_a_growth_flag_until_the_next_days_would_be() {
        let flag = |t: u64, rule: &str, key: &str| json!({ "t": t, "kind": KIND, "rule": rule, "key": key });
        let events = vec![
            flag(END - DAY_MS, "growth", "core"),
            json!({ "t": END - 10, "kind": "ui_stall", "ms": 9_000 }),
            flag(END, "stalls", "ui_stall"),
            flag(END, "growth", "core"),
        ];
        let said = |now: u64| -> Vec<(String, u64)> {
            still_said(&events, now).iter().map(|e| (e["rule"].as_str().unwrap().to_string(), e["t"].as_u64().unwrap())).collect()
        };
        // Both of this hour; of the series that keeps growing, only its newest flag. A stall is not a flag.
        assert_eq!(said(END + 5 * 60_000), [("growth".to_string(), END), ("stalls".to_string(), END)]);
        // Two hours on the stalls are no longer said. The growth is, until a day and an hour after it.
        assert_eq!(said(END + 2 * HOUR_MS), [("growth".to_string(), END)]);
        assert_eq!(said(END + DAY_MS + HOUR_MS - 1).len(), 1);
        assert!(said(END + DAY_MS + HOUR_MS).is_empty());
        // Before this hour's flags were written, yesterday's growth was the one said.
        assert_eq!(said(END - 1), [("growth".to_string(), END - DAY_MS)]);
    }

    #[test]
    fn the_hour_s_flags_are_written_once_and_a_growing_series_once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let write = |t: u64, kind: &str, fields: Value| agentmux_common::record_events::append_event(dir.path(), t, kind, fields).unwrap();
        for i in 0..5 {
            write(END - 30 * 60_000 + i, "ui_stall", json!({ "ms": 7_000 }));
        }
        // A stall of the hour before is not this hour's.
        write(END - 90 * 60_000, "srv_stall", json!({ "ms": 60_000 }));
        let rollups: String = two_days(|i| (300 + 4 * i) * MB, |_| None).iter().map(|r| serde_json::to_string(r).unwrap() + "\n").collect();
        std::fs::write(dir.path().join("rollup-2026-11.jsonl"), rollups).unwrap();

        let (end, flags) = found(dir.path(), END + 5 * 60_000);
        assert_eq!(end, END);
        let said: Vec<(&str, &str)> = flags.iter().map(|f| (f.rule, f.key.as_str())).collect();
        assert_eq!(said, vec![("stalls", "ui_stall"), ("growth", "core")]);

        let looked = look(dir.path(), END + 5 * 60_000);
        assert_eq!(looked.new.iter().map(|f| f.rule).collect::<Vec<_>>(), ["stalls", "growth"]);
        assert_eq!(looked.said.len(), 2);
        // Looked at again in the same hour: nothing new is written, and both are still said.
        let again = look(dir.path(), END + 20 * 60_000);
        assert!(again.new.is_empty());
        assert_eq!(again.said, looked.said);
        let (_, events) = read_events(dir.path(), END, END, 10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["kind"], KIND);
        assert_eq!((events[0]["rule"].as_str(), events[0]["key"].as_str(), events[0]["n"].as_u64()), (Some("stalls"), Some("ui_stall"), Some(5)));
        // The same hour looked at again (srv started again inside it) says nothing new.
        assert!(found(dir.path(), END + 40 * 60_000).1.is_empty());
        // An hour later the series still grows: not said again within the day. A day later it is.
        let core = Flag { rule: "growth", key: "core".into(), fields: json!({}) };
        let (_, all) = read_events(dir.path(), 0, u64::MAX, 100);
        assert!(already(&all, &core, END + HOUR_MS));
        assert!(already(&all, &core, END + DAY_MS - HOUR_MS));
        assert!(!already(&all, &core, END + DAY_MS));
    }
}
