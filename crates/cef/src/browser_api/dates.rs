// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Text an agent fills into a native date, time, month, week or
//! datetime-local input, turned into the one format the input's `value`
//! takes (`2026-10-15`, `09:30`, `2026-10`, `2026-W42`, `2026-10-15T09:30`).
//! Typing doesn't work on these controls (their segments read key events,
//! not inserted text), so `fill` sets the value in this format instead.
//!
//! Accepted: ISO (`2026-10-15`), numbers with the year last (`10/15/2026`,
//! `15.10.2026`) when only one reading is a real date, and month names
//! (`October 15, 2026`, `15 Oct 2026`). Never guessed: `05/06/2026` could be
//! either month first, and a two-digit year could be either century.

use chrono::{Datelike, NaiveDate};

/// Input types whose value is a date or time in a fixed format.
pub fn is_date_input(input_type: &str) -> bool {
    matches!(input_type, "date" | "time" | "month" | "week" | "datetime-local")
}

/// `text` in the format `input_type`'s value takes, or why it can't be read.
pub fn to_input_value(input_type: &str, text: &str) -> Result<String, String> {
    let text = text.trim();
    match input_type {
        "date" => parse_date(text).map(|d| d.format("%Y-%m-%d").to_string()),
        "month" => parse_month(text),
        "week" => parse_week(text),
        "time" => parse_time(text),
        "datetime-local" => parse_date_time(text),
        other => Err(format!("a {other} input doesn't take a date")),
    }
}

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december",
];

/// The month a word names (`Oct`, `october`, `Sept`), 1-12.
fn month_named(word: &str) -> Option<u32> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    let w = if w == "sept" { "sep".to_string() } else { w };
    MONTHS.iter().position(|m| m.starts_with(&w)).map(|i| i as u32 + 1)
}

/// The words and numbers of a date: `October 15th, 2026` → `October`, `15`, `2026`.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| {
            // 1st, 2nd, 3rd, 15th: the number.
            let digits = t.trim_end_matches(|c: char| c.is_ascii_alphabetic());
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && digits.len() < t.len() && t.len() - digits.len() == 2 {
                digits.to_string()
            } else {
                t.to_string()
            }
        })
        .collect()
}

fn ymd(y: i32, m: u32, d: u32, text: &str) -> Result<NaiveDate, String> {
    NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| format!("{text:?} isn't a real date"))
}

fn year_of(t: &str, text: &str) -> Result<i32, String> {
    if t.len() != 4 {
        return Err(format!("{text:?}: give the year in four digits (2026), so the century isn't guessed"));
    }
    t.parse().map_err(|_| format!("{text:?}: no year"))
}

fn parse_date(text: &str) -> Result<NaiveDate, String> {
    let t = tokens(text);
    let unreadable = || format!("couldn't read {text:?} as a date: give it as 2026-10-15");
    let num = |s: &str| s.parse::<u32>().ok();
    match t.as_slice() {
        // Year first: 2026-10-15, 2026/10/15.
        [y, m, d] if y.len() == 4 && num(y).is_some() && num(m).is_some() && num(d).is_some() => {
            ymd(year_of(y, text)?, num(m).unwrap(), num(d).unwrap(), text)
        }
        // Year last, both numbers: only one reading may be a real date.
        [a, b, y] if num(a).is_some() && num(b).is_some() && y.chars().all(|c| c.is_ascii_digit()) => {
            let year = year_of(y, text)?;
            let (a, b) = (num(a).unwrap(), num(b).unwrap());
            let month_first = NaiveDate::from_ymd_opt(year, a, b);
            let day_first = NaiveDate::from_ymd_opt(year, b, a);
            match (month_first, day_first) {
                (Some(m), Some(d)) if m != d => Err(format!(
                    "{text:?} could be {} or {}: give it as {} or {}",
                    m.format("%B %-d"),
                    d.format("%B %-d"),
                    m.format("%Y-%m-%d"),
                    d.format("%Y-%m-%d")
                )),
                (Some(x), _) | (None, Some(x)) => Ok(x),
                (None, None) => Err(format!("{text:?} isn't a real date")),
            }
        }
        // A month name and two numbers, in any order: October 15, 2026; 15 Oct 2026.
        _ => {
            let month = t.iter().find_map(|w| month_named(w)).ok_or_else(unreadable)?;
            let numbers: Vec<&String> = t.iter().filter(|w| w.chars().all(|c| c.is_ascii_digit())).collect();
            let year = numbers.iter().find(|n| n.len() == 4).ok_or_else(|| {
                format!("{text:?}: give the year in four digits (2026), so the century isn't guessed")
            })?;
            let days: Vec<&&String> = numbers.iter().filter(|n| n.len() <= 2).collect();
            if numbers.len() != 2 || days.len() != 1 || t.len() != 3 {
                return Err(unreadable());
            }
            ymd(year_of(year, text)?, month, num(days[0]).ok_or_else(unreadable)?, text)
        }
    }
}

fn parse_month(text: &str) -> Result<String, String> {
    let t = tokens(text);
    let num = |s: &str| s.parse::<u32>().ok();
    let ym = match t.as_slice() {
        [y, m] if y.len() == 4 && num(y).is_some() && num(m).is_some() => (year_of(y, text)?, num(m).unwrap()),
        [m, y] if y.len() == 4 && num(y).is_some() && num(m).is_some() => (year_of(y, text)?, num(m).unwrap()),
        [a, b] => match (month_named(a), month_named(b)) {
            (Some(m), None) => (year_of(b, text)?, m),
            (None, Some(m)) => (year_of(a, text)?, m),
            _ => return Err(format!("couldn't read {text:?} as a month: give it as 2026-10")),
        },
        // A whole date: its month.
        _ => {
            let d = parse_date(text).map_err(|_| format!("couldn't read {text:?} as a month: give it as 2026-10"))?;
            (d.year(), d.month())
        }
    };
    ymd(ym.0, ym.1, 1, text)?;
    Ok(format!("{:04}-{:02}", ym.0, ym.1))
}

fn parse_week(text: &str) -> Result<String, String> {
    let upper = text.to_ascii_uppercase();
    if let Some((y, w)) = upper.split_once("-W") {
        let year = year_of(y.trim(), text)?;
        let week: u32 = w.trim().parse().map_err(|_| format!("couldn't read {text:?} as a week: give it as 2026-W42"))?;
        NaiveDate::from_isoywd_opt(year, week, chrono::Weekday::Mon)
            .ok_or_else(|| format!("{year} has no week {week}"))?;
        return Ok(format!("{year:04}-W{week:02}"));
    }
    // A date: the ISO week it falls in.
    let d = parse_date(text).map_err(|_| format!("couldn't read {text:?} as a week: give it as 2026-W42, or a date in it"))?;
    let w = d.iso_week();
    Ok(format!("{:04}-W{:02}", w.year(), w.week()))
}

fn parse_time(text: &str) -> Result<String, String> {
    let unreadable = || format!("couldn't read {text:?} as a time: give it as 09:30 or 21:30");
    let lower = text.to_ascii_lowercase().replace('.', "");
    let (clock, half) = if let Some(c) = lower.strip_suffix("am") {
        (c.trim(), Some(false))
    } else if let Some(c) = lower.strip_suffix("pm") {
        (c.trim(), Some(true))
    } else {
        (lower.trim(), None)
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let num = |s: &str| if (1..=2).contains(&s.len()) { s.parse::<u32>().ok() } else { None };
    let (mut h, m, s) = match parts.as_slice() {
        [h] if half.is_some() => (num(h).ok_or_else(unreadable)?, 0, None),
        [h, m] => (num(h).ok_or_else(unreadable)?, num(m).ok_or_else(unreadable)?, None),
        [h, m, s] => (num(h).ok_or_else(unreadable)?, num(m).ok_or_else(unreadable)?, Some(num(s).ok_or_else(unreadable)?)),
        _ => return Err(unreadable()),
    };
    if let Some(pm) = half {
        if !(1..=12).contains(&h) {
            return Err(format!("{text:?} isn't a real time"));
        }
        h = match (h, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, true) => h + 12,
            (h, false) => h,
        };
    }
    if h > 23 || m > 59 || s.is_some_and(|s| s > 59) {
        return Err(format!("{text:?} isn't a real time"));
    }
    Ok(match s {
        Some(s) => format!("{h:02}:{m:02}:{s:02}"),
        None => format!("{h:02}:{m:02}"),
    })
}

fn parse_date_time(text: &str) -> Result<String, String> {
    let unreadable = || format!("couldn't read {text:?} as a date and time: give it as 2026-10-15T09:30");
    // The time is the last part with a colon in it: 2026-10-15T09:30,
    // October 15, 2026 at 9:30 PM, 10/15/2026 21:30.
    let lower = text.to_ascii_lowercase();
    let colon = lower.find(':').ok_or_else(unreadable)?;
    let split = lower[..colon].rfind(['t', ' ']).ok_or_else(unreadable)?;
    let date_part = text[..split].trim();
    let date_part = date_part.strip_suffix(" at").unwrap_or(date_part).trim().trim_end_matches(',');
    let date = parse_date(date_part)?;
    let time = parse_time(&text[split + 1..])?;
    Ok(format!("{}T{time}", date.format("%Y-%m-%d")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_read_in_the_common_forms() {
        for text in ["2026-10-15", "2026/10/15", "10/15/2026", "15.10.2026", "October 15, 2026", "Oct 15 2026", "15 October 2026", "October 15th, 2026", "Sept 1, 2026"] {
            let want = if text.starts_with("Sept") { "2026-09-01" } else { "2026-10-15" };
            assert_eq!(to_input_value("date", text).as_deref(), Ok(want), "{text}");
        }
    }

    #[test]
    fn a_date_with_two_readings_or_a_short_year_is_refused_not_guessed() {
        let e = to_input_value("date", "05/06/2026").unwrap_err();
        assert!(e.contains("May 6") && e.contains("June 5") && e.contains("2026-05-06"), "{e}");
        assert_eq!(to_input_value("date", "06/06/2026").as_deref(), Ok("2026-06-06"), "the same either way");
        assert!(to_input_value("date", "10/15/26").unwrap_err().contains("four digits"));
        assert!(to_input_value("date", "Oct 15, 26").unwrap_err().contains("four digits"));
    }

    #[test]
    fn a_date_that_doesnt_exist_is_refused() {
        assert!(to_input_value("date", "2026-02-30").unwrap_err().contains("isn't a real date"));
        assert!(to_input_value("date", "February 30, 2026").unwrap_err().contains("isn't a real date"));
        assert!(to_input_value("date", "next Tuesday").is_err());
    }

    #[test]
    fn months_weeks_and_times_take_their_own_format() {
        assert_eq!(to_input_value("month", "October 2026").as_deref(), Ok("2026-10"));
        assert_eq!(to_input_value("month", "2026-10").as_deref(), Ok("2026-10"));
        assert_eq!(to_input_value("month", "June 2026").as_deref(), Ok("2026-06"), "a four-letter month isn't a year");
        assert_eq!(to_input_value("month", "10/2026").as_deref(), Ok("2026-10"));
        assert_eq!(to_input_value("month", "October 15, 2026").as_deref(), Ok("2026-10"));
        assert_eq!(to_input_value("week", "2026-W42").as_deref(), Ok("2026-W42"));
        assert_eq!(to_input_value("week", "2026-10-15").as_deref(), Ok("2026-W42"));
        assert!(to_input_value("week", "2026-W54").is_err());
        assert_eq!(to_input_value("time", "9:30").as_deref(), Ok("09:30"));
        assert_eq!(to_input_value("time", "9:30 PM").as_deref(), Ok("21:30"));
        assert_eq!(to_input_value("time", "12:05 a.m.").as_deref(), Ok("00:05"));
        assert_eq!(to_input_value("time", "9pm").as_deref(), Ok("21:00"));
        assert_eq!(to_input_value("time", "21:30:15").as_deref(), Ok("21:30:15"));
        assert!(to_input_value("time", "25:00").is_err());
        assert!(to_input_value("time", "13:00 pm").is_err());
    }

    #[test]
    fn a_date_and_time_take_both() {
        assert_eq!(to_input_value("datetime-local", "2026-10-15T09:30").as_deref(), Ok("2026-10-15T09:30"));
        assert_eq!(to_input_value("datetime-local", "October 15, 2026 at 9:30 PM").as_deref(), Ok("2026-10-15T21:30"));
        assert_eq!(to_input_value("datetime-local", "10/15/2026 21:30").as_deref(), Ok("2026-10-15T21:30"));
        assert!(to_input_value("datetime-local", "2026-10-15").is_err(), "the time is required");
    }

    #[test]
    fn only_the_date_and_time_types_count() {
        for ty in ["date", "time", "month", "week", "datetime-local"] {
            assert!(is_date_input(ty));
        }
        assert!(!is_date_input("text") && !is_date_input("number"));
    }
}
