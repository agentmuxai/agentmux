// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux logs --since 1h`: the server's log lines written in the last while. A line of the server's log
//! starts with its time (RFC 3339, in UTC, with colour codes around it when the server wrote to a terminal); a line
//! without one (a wrapped message, a line of this command's own) belongs to the line before it.

use std::time::Duration;

use chrono::{DateTime, Utc};

/// `45s`, `30m`, `2h` or `1d`: a whole number and one unit.
pub fn parse_age(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let bad = || format!("{text:?} is not an age: use a number and s, m, h or d, for example 30m or 1h");
    let (digits, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?);
    let n: u64 = digits.parse().map_err(|_| bad())?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(bad()),
    };
    n.checked_mul(seconds).map(Duration::from_secs).ok_or_else(bad)
}

/// The line without its colour codes.
fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The time a line starts with, when it starts with one.
fn stamp(line: &str) -> Option<DateTime<Utc>> {
    let head = plain(line.get(..80).unwrap_or(line));
    DateTime::parse_from_rfc3339(head.split_whitespace().next()?).ok().map(|t| t.with_timezone(&Utc))
}

/// The time `age` before now. An age past the calendar's range is "from the start", never a panic.
pub fn cutoff(age: Duration) -> DateTime<Utc> {
    chrono::Duration::from_std(age)
        .ok()
        .and_then(|age| Utc::now().checked_sub_signed(age))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// The lines from the first one stamped at or after `cutoff` on, each with the lines under it that carry no time of their own.
pub fn since(text: &str, cutoff: DateTime<Utc>) -> Vec<&str> {
    let mut keep = false;
    text.lines()
        .filter(|line| {
            if let Some(t) = stamp(line) {
                keep = t >= cutoff;
            }
            keep
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_age_past_the_calendar_shows_everything_and_does_not_panic() {
        let huge = parse_age("100000000000d").unwrap();
        assert_eq!(cutoff(huge), DateTime::<Utc>::MIN_UTC);
        let hour = cutoff(Duration::from_secs(3600));
        assert!(hour < Utc::now() && Utc::now() - hour < chrono::Duration::seconds(3700));
    }

    #[test]
    fn an_age_is_a_number_and_a_unit() {
        assert_eq!(parse_age("45s").unwrap(), Duration::from_secs(45));
        assert_eq!(parse_age("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_age("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_age(" 2d ").unwrap(), Duration::from_secs(172_800));
        for bad in ["", "h", "1", "1w", "-1h", "1.5h", "1 h", "99999999999999999999h", "18446744073709551615d"] {
            assert!(parse_age(bad).is_err(), "{bad:?}");
        }
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn only_the_lines_since_the_cutoff_are_kept_and_an_unstamped_line_follows_the_one_above() {
        let log = "\
2026-11-15T07:00:00.100000Z  INFO old: before\n\
2026-11-15T08:30:00.000000Z  INFO new: kept\n\
  a wrapped line of the one above\n\
2026-11-15T08:31:00Z  WARN new: also kept\n";
        let got = since(log, at("2026-11-15T08:00:00Z"));
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got[0].contains("new: kept") && got[1].contains("wrapped") && got[2].contains("also kept"));
        let old = "2026-11-15T07:00:00Z INFO old: a\n  wrapped under an old line\n";
        assert!(since(old, at("2026-11-15T08:00:00Z")).is_empty(), "a line under an old one is old too");
    }

    #[test]
    fn colour_codes_around_the_time_do_not_hide_it() {
        let line = "\u{1b}[2m2026-11-15T08:30:00.123456Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m srv: up";
        assert_eq!(stamp(line), Some(at("2026-11-15T08:30:00.123456Z")));
        assert_eq!(since(line, at("2026-11-15T08:00:00Z")), vec![line]);
        assert!(stamp("not a log line").is_none());
    }

    #[test]
    fn a_log_with_no_times_gives_nothing_since_a_cutoff() {
        assert!(since("one\ntwo\n", at("2026-11-15T08:00:00Z")).is_empty());
    }
}
