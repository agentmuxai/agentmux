// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Standard 5-field cron ("min hour dom mon dow", as the Schedule tool
//! documents it) read with the `cron` crate, which wants a seconds field
//! first and numbers the days of the week 1 = Sunday … 7 = Saturday. In
//! standard cron 0 and 7 are Sunday and 1 is Monday, so the expression is
//! translated here, at parse time only: a job's stored expression stays the
//! standard one it was written in. Every parse of a job's expression goes
//! through [`parse`].

use std::collections::BTreeSet;
use std::str::FromStr;

use cron::Schedule;

/// `five_field`, a standard 5-field cron expression, as the `cron` crate's
/// expression: a `0` seconds field first, and a numeric day-of-week field in
/// the crate's numbering. Names (`MON-FRI`, `SUN`), `*` and `?` are kept as
/// they are. Ranges may wrap (`5-1` is Friday to Monday).
pub fn to_crate_expr(five_field: &str) -> Result<String, String> {
    let fields: Vec<&str> = five_field.split_whitespace().collect();
    let [min, hour, dom, mon, dow] = fields[..] else {
        return Err(format!("expected 5 fields (min hour dom mon dow), got {}", fields.len()));
    };
    Ok(format!("0 {min} {hour} {dom} {mon} {}", crate_dow(dow)?))
}

/// [`to_crate_expr`], parsed.
pub fn parse(five_field: &str) -> Result<Schedule, String> {
    let expr = to_crate_expr(five_field)?;
    Schedule::from_str(&expr).map_err(|e| e.to_string())
}

/// A standard day-of-week field in the crate's numbering.
fn crate_dow(field: &str) -> Result<String, String> {
    if field == "*" || field == "?" {
        return Ok(field.to_string());
    }
    let mut days: BTreeSet<u32> = BTreeSet::new();
    let mut named: Vec<&str> = Vec::new();
    for item in field.split(',') {
        if item.chars().any(|c| c.is_ascii_alphabetic()) {
            named.push(item);
        } else {
            days.extend(standard_days(item)?);
        }
    }
    // Standard Sunday (0) is the crate's 1; Monday (1) its 2, and so on.
    let mut out: Vec<String> = days.into_iter().map(|d| (d + 1).to_string()).collect();
    out.extend(named.into_iter().map(str::to_string));
    Ok(out.join(","))
}

/// The days (0 = Sunday … 6 = Saturday) one numeric item of a standard
/// day-of-week list names: `n`, `a-b`, `a-b/s`, `a/s` or `*/s`. 7 is
/// Sunday too. A range whose start is after its end wraps through Sunday.
fn standard_days(item: &str) -> Result<Vec<u32>, String> {
    let bad = || format!("invalid day of week '{item}' (use 0-7, where 0 and 7 are Sunday, or MON-SUN)");
    let num = |s: &str| s.parse::<u32>().ok().filter(|&n| n <= 7).ok_or_else(bad);
    let (range, step) = match item.split_once('/') {
        Some((r, s)) => (r, Some(s.parse::<usize>().ok().filter(|&s| s > 0).ok_or_else(bad)?)),
        None => (item, None),
    };
    let seq: Vec<u32> = if range == "*" {
        (0..=6).collect()
    } else if let Some((a, b)) = range.split_once('-') {
        let (a, b) = (num(a)?, num(b)?);
        if a <= b {
            (a..=b).map(|d| d % 7).collect()
        } else {
            // Wraps through Sunday: 5-1 is Fri, Sat, Sun, Mon.
            let (from, to) = (a % 7, b % 7);
            let len = (to + 7 - from) % 7 + 1;
            (0..len).map(|i| (from + i) % 7).collect()
        }
    } else if step.is_some() {
        // `a/s`, any step: from a through the end of the week (7, Sunday).
        (num(range)?..=7).map(|d| d % 7).collect()
    } else {
        vec![num(range)? % 7]
    };
    Ok(seq.into_iter().step_by(step.unwrap_or(1)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Utc, Weekday};

    /// The weekdays `five_field` fires on, from a week of its fires.
    fn fires_on(five_field: &str) -> BTreeSet<u32> {
        let schedule = parse(five_field).unwrap_or_else(|e| panic!("{five_field}: {e}"));
        // 2026-01-04 is a Sunday.
        let start = Utc.with_ymd_and_hms(2026, 1, 3, 23, 0, 0).unwrap();
        schedule
            .after(&start)
            .take_while(|t| *t < start + chrono::Duration::days(7))
            .map(|t| t.weekday().num_days_from_sunday())
            .collect()
    }

    fn set(days: &[u32]) -> BTreeSet<u32> {
        days.iter().copied().collect()
    }

    #[test]
    fn each_day_number_is_the_standard_day_and_0_and_7_are_both_sunday() {
        for d in 0..=6 {
            assert_eq!(fires_on(&format!("0 6 * * {d}")), set(&[d]), "day {d}");
        }
        assert_eq!(fires_on("0 6 * * 7"), set(&[0]), "7 is Sunday too");
    }

    #[test]
    fn lists_ranges_and_steps_mean_what_standard_cron_means() {
        assert_eq!(fires_on("0 6 * * 1-5"), set(&[1, 2, 3, 4, 5]), "Monday to Friday");
        assert_eq!(fires_on("0 6 * * 1,3,5"), set(&[1, 3, 5]));
        assert_eq!(fires_on("0 6 * * */2"), set(&[0, 2, 4, 6]));
        assert_eq!(fires_on("0 6 * * 1-5/2"), set(&[1, 3, 5]));
        assert_eq!(fires_on("0 6 * * 1/2"), set(&[1, 3, 5, 0]), "from Monday through Sunday, every 2");
        assert_eq!(fires_on("0 6 * * 1/1"), set(&[1, 2, 3, 4, 5, 6, 0]), "a step of 1 is still a range");
        assert_eq!(fires_on("0 6 * * 3/2"), set(&[3, 5, 0]), "Wednesday, Friday, Sunday");
        assert_eq!(fires_on("0 6 * * 0/3"), set(&[0, 3, 6]));
        assert_eq!(fires_on("0 6 * * */1"), set(&[0, 1, 2, 3, 4, 5, 6]));
        assert_eq!(fires_on("0 6 * * 0-7"), set(&[0, 1, 2, 3, 4, 5, 6]));
        assert_eq!(fires_on("0 6 * * 5-7"), set(&[5, 6, 0]), "Friday to Sunday");
        assert_eq!(fires_on("0 6 * * 5-1"), set(&[5, 6, 0, 1]), "wraps: Friday to Monday");
        assert_eq!(fires_on("0 6 * * 6-0"), set(&[6, 0]));
        assert_eq!(fires_on("0 6 * * 0,6"), set(&[0, 6]));
        assert_eq!(fires_on("0 6 * * *"), set(&[0, 1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn names_are_kept_and_mean_their_day() {
        assert_eq!(to_crate_expr("0 6 * * MON-FRI").unwrap(), "0 0 6 * * MON-FRI");
        assert_eq!(fires_on("0 6 * * MON-FRI"), set(&[1, 2, 3, 4, 5]));
        assert_eq!(fires_on("0 6 * * SUN"), set(&[0]));
    }

    #[test]
    fn the_other_fields_pass_through_with_a_seconds_field_first() {
        assert_eq!(to_crate_expr("*/15 * * * *").unwrap(), "0 */15 * * * *");
        assert_eq!(to_crate_expr("0 6 1 * 1").unwrap(), "0 0 6 1 * 2");
        assert_eq!(to_crate_expr("0 6 * * 5-1").unwrap(), "0 0 6 * * 1,2,6,7");
    }

    #[test]
    fn bad_expressions_are_refused() {
        for bad in ["0 6 * *", "0 6 * * 1 2", "0 6 * * 8", "0 6 * * 1-9", "0 6 * * x-", "0 6 * * */0", "61 6 * * 1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    /// `0 6 * * 1` is Monday 06:00. A job saved with it before this fix
    /// fired on Sunday (the crate's 1); its stored expression is unchanged
    /// and its next fire is now the documented Monday.
    #[test]
    fn a_saved_numeric_weekday_job_fires_on_the_documented_day() {
        let saturday = Utc.with_ymd_and_hms(2026, 1, 3, 12, 0, 0).unwrap().timestamp_millis();
        let next = crate::backend::jobs::next_slot_after("0 6 * * 1", saturday).unwrap();
        let next = Utc.timestamp_millis_opt(next).unwrap();
        assert_eq!((next.weekday(), next.format("%H:%M").to_string()), (Weekday::Mon, "06:00".to_string()));
        // What the old reading did: the crate's own 1, a Sunday.
        let old = Schedule::from_str("0 0 6 * * 1").unwrap();
        let before = old.after(&Utc.timestamp_millis_opt(saturday).unwrap()).next().unwrap();
        assert_eq!(before.weekday(), Weekday::Sun);
    }
}
