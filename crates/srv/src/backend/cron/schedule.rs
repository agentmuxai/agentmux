// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! When a job runs: its trigger as a sequence of slots, ms epoch
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1 "Triggers").
//!
//! A cron trigger is read in its IANA time zone. The `cron` crate evaluates
//! in one zone only and skips a local time that doesn't exist or exists
//! twice, so the expression is evaluated here against local wall-clock time
//! (carried as a UTC value with no offset), and each local time is turned
//! into an instant with the zone's rules:
//! - in a daylight-saving gap (02:30 on a spring-forward night) the slot
//!   fires once, at the first valid time after the gap; several slots in one
//!   gap are that one fire;
//! - in an overlap (01:30 on a fall-back night) it fires once, at the first
//!   of the two.
//!
//! Every slot is strictly later than the one before, so a scheduler that
//! asks for the slot after the last one never fires a time twice.

use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use crate::backend::storage::cron::CronJob;

/// The zone a cron trigger names when it names none.
pub const DEFAULT_TZ: &str = "UTC";

/// The longest fixed offset a job's fires are spread by (§5.1).
pub const JITTER_MAX_MS: i64 = 120_000;

/// How far a daylight-saving change can move the local clock, at most, for
/// the backward search; and how many candidate local times one search
/// looks at.
const SHIFT_MAX_HOURS: i64 = 3;
const SEARCH_CAP: usize = 100_000;

/// A job's trigger, ready to answer "when next".
#[derive(Debug, Clone)]
pub enum JobSchedule {
    Cron { schedule: Box<cron::Schedule>, tz: Tz },
    /// Every `period_ms` from `anchor_ms` (its creation): the first slot is
    /// one period after it.
    Every { anchor_ms: i64, period_ms: i64 },
    Once { at_ms: i64 },
    /// Run now only: no slots.
    Manual,
}

/// An IANA zone name (`America/New_York`, `UTC`). Empty is UTC.
pub fn parse_tz(name: &str) -> Result<Tz, String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(Tz::UTC);
    }
    name.parse::<Tz>().map_err(|_| format!("unknown time zone '{name}' (use an IANA name such as Europe/Berlin or UTC)"))
}

/// The job's fixed jitter, 0 up to [`JITTER_MAX_MS`], derived from its id
/// (FNV-1a), so jobs due at the same minute don't all start at once and a
/// job's offset is the same on every instance and after a restart.
pub fn jitter_ms(job_id: &str) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in job_id.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % (JITTER_MAX_MS as u64 + 1)) as i64
}

/// A local wall-clock time as the `cron` crate's UTC value.
fn carrier(naive: NaiveDateTime) -> DateTime<Utc> {
    Utc.from_utc_datetime(&naive)
}

/// The instant a cron slot at local time `naive` fires (see the module doc).
fn instant(tz: &Tz, naive: NaiveDateTime) -> Option<i64> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.timestamp_millis()),
        LocalResult::None => gap_end(tz, naive),
    }
}

/// The first valid local minute after the gap `naive` falls in, as an
/// instant. Gaps are at most a day (a zone that skipped a date).
fn gap_end(tz: &Tz, naive: NaiveDateTime) -> Option<i64> {
    let start = naive.with_second(0)?.with_nanosecond(0)?;
    (1..=48 * 60).find_map(|m| match tz.from_local_datetime(&(start + Duration::minutes(m))) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.timestamp_millis()),
        LocalResult::None => None,
    })
}

impl JobSchedule {
    /// A standard 5-field cron expression read in `tz` (empty: UTC).
    pub fn cron(expression: &str, tz: &str) -> Result<Self, String> {
        Ok(JobSchedule::Cron { schedule: Box::new(super::expr::parse(expression)?), tz: parse_tz(tz)? })
    }

    /// `job`'s trigger. A job from before triggers had kinds is cron.
    pub fn of(job: &CronJob) -> Result<Self, String> {
        use crate::backend::jobs::definition::trigger_kind;
        match job.trigger_kind.as_str() {
            "" | trigger_kind::CRON => Self::cron(&job.expression, &job.tz),
            trigger_kind::EVERY => {
                let secs = job.every_secs.filter(|&s| s > 0).ok_or("an every trigger needs seconds")?;
                Ok(JobSchedule::Every { anchor_ms: job.created_at.saturating_mul(1000), period_ms: secs.saturating_mul(1000) })
            }
            trigger_kind::ONCE => Ok(JobSchedule::Once { at_ms: job.once_at_ms.ok_or("a once trigger needs at")? }),
            trigger_kind::MANUAL => Ok(JobSchedule::Manual),
            other => Err(format!("trigger kind {other} can't be scheduled")),
        }
    }

    /// The slots strictly after `after_ms`, in order.
    pub fn after(&self, after_ms: i64) -> Box<dyn Iterator<Item = i64> + '_> {
        match self {
            JobSchedule::Cron { schedule, tz } => {
                let Some(after) = DateTime::from_timestamp_millis(after_ms) else { return Box::new(std::iter::empty()) };
                let local = after.with_timezone(tz).naive_local();
                let mut last = after_ms;
                Box::new(schedule.after(&carrier(local)).take(SEARCH_CAP).filter_map(move |c| {
                    let ms = instant(tz, c.naive_utc())?;
                    (ms > last).then(|| {
                        last = ms;
                        ms
                    })
                }))
            }
            &JobSchedule::Every { anchor_ms, period_ms } => {
                let k = if after_ms < anchor_ms { 1 } else { (after_ms - anchor_ms) / period_ms + 1 };
                Box::new((k.max(1)..).map(move |k| anchor_ms.saturating_add(k.saturating_mul(period_ms))))
            }
            &JobSchedule::Once { at_ms } => Box::new((at_ms > after_ms).then_some(at_ms).into_iter()),
            JobSchedule::Manual => Box::new(std::iter::empty()),
        }
    }

    /// The first slot strictly after `after_ms`.
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        self.after(after_ms).next()
    }

    /// The last slot strictly before `before_ms`.
    pub fn prev_before(&self, before_ms: i64) -> Option<i64> {
        match self {
            JobSchedule::Cron { schedule, tz } => {
                // Local times aren't in instant order around an overlap, so
                // look a few hours either side and keep the latest instant.
                let local = DateTime::from_timestamp_millis(before_ms)?.with_timezone(tz).naive_local();
                let window = Duration::hours(SHIFT_MAX_HOURS);
                let mut best: Option<i64> = None;
                for c in schedule.after(&carrier(local + window)).rev().take(SEARCH_CAP) {
                    let naive = c.naive_utc();
                    if let Some(ms) = instant(tz, naive).filter(|&ms| ms < before_ms) {
                        best = Some(best.map_or(ms, |b| b.max(ms)));
                    }
                    if naive < local - window && best.is_some() {
                        break;
                    }
                }
                best
            }
            &JobSchedule::Every { anchor_ms, period_ms } => {
                let k = (before_ms - anchor_ms - 1).div_euclid(period_ms);
                (k >= 1).then(|| anchor_ms + k * period_ms)
            }
            &JobSchedule::Once { at_ms } => (at_ms < before_ms).then_some(at_ms),
            JobSchedule::Manual => None,
        }
    }

    /// The shortest time between two consecutive slots among the next
    /// `count` after `from_ms`, in seconds. `None` with fewer than two.
    pub fn min_gap_secs(&self, from_ms: i64, count: usize) -> Option<i64> {
        let slots: Vec<i64> = self.after(from_ms).take(count).collect();
        slots.windows(2).map(|w| (w[1] - w[0]) / 1000).min()
    }
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;
