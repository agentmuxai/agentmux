// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Day string utilities for YYYY-MM-DD formatted dates.
//! Port of Go's pkg/util/daystr/.


use chrono::{Datelike, Duration, Local, NaiveDate};

/// Get the current date as YYYY-MM-DD.
pub fn get_cur_day_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Get a relative date as YYYY-MM-DD (positive = future, negative = past).
pub fn get_rel_day_str(rel_days: i64) -> String {
    let date = Local::now() + Duration::days(rel_days);
    date.format("%Y-%m-%d").to_string()
}

/// Parse a custom day format string.
///
/// Prefix: `today`, `yesterday`, `bom` (beginning of month), `bow` (beginning of week),
///          or `YYYY-MM-DD`. Default is `today`.
/// Deltas: `+Nd`, `-Nd` (days), `+Nw`, `-Nw` (weeks), `+Nm`, `-Nm` (months).
///
/// Examples: `today-1d`, `bom+1m`, `2024-01-15+2w`
pub fn get_custom_day_str(format: &str) -> Result<String, String> {
    let format = format.trim();
    if format.is_empty() {
        return Ok(get_cur_day_str());
    }

    // Find the split point between prefix and deltas
    let delta_start = find_delta_start(format);
    let (prefix_str, delta_str) = format.split_at(delta_start);

    // Parse prefix
    let today = Local::now().date_naive();
    let mut date = if prefix_str.is_empty() || prefix_str == "today" {
        today
    } else if prefix_str == "yesterday" {
        today - Duration::days(1)
    } else if prefix_str == "bom" {
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .ok_or_else(|| "invalid beginning of month".to_string())?
    } else if prefix_str == "bow" {
        let weekday = today.weekday().num_days_from_monday();
        today - Duration::days(weekday as i64)
    } else {
        // Try parsing as YYYY-MM-DD
        NaiveDate::parse_from_str(prefix_str, "%Y-%m-%d")
            .map_err(|e| format!("invalid date prefix '{}': {}", prefix_str, e))?
    };

    // Parse deltas
    if !delta_str.is_empty() {
        date = apply_deltas(date, delta_str)?;
    }

    Ok(date.format("%Y-%m-%d").to_string())
}

/// Find where delta expressions start in the format string.
fn find_delta_start(s: &str) -> usize {
    // Deltas start with + or - followed by digits
    // But YYYY-MM-DD contains dashes, so we need to be careful
    let bytes = s.as_bytes();
    let mut i = 0;

    // Skip past any YYYY-MM-DD prefix
    if bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
    {
        i = 10;
    } else {
        // Skip alphabetic prefix (today, yesterday, bom, bow)
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
    }

    i
}

/// Apply delta expressions to a date.
fn apply_deltas(mut date: NaiveDate, deltas: &str) -> Result<NaiveDate, String> {
    let bytes = deltas.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Parse sign
        let sign: i64 = match bytes[i] {
            b'+' => 1,
            b'-' => -1,
            _ => return Err(format!("expected +/- at position {}", i)),
        };
        i += 1;

        // Parse number
        let num_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == num_start {
            return Err("expected number after +/-".to_string());
        }
        let num: i64 = deltas[num_start..i]
            .parse()
            .map_err(|e| format!("invalid number: {}", e))?;
        let val = sign * num;

        // Parse unit
        if i >= bytes.len() {
            return Err("expected unit (d/w/m) after number".to_string());
        }
        match bytes[i] {
            b'd' => {
                date += Duration::days(val);
            }
            b'w' => {
                date += Duration::weeks(val);
            }
            b'm' => {
                // Month arithmetic
                let total_months = date.year() * 12 + date.month() as i32 - 1 + val as i32;
                let new_year = total_months.div_euclid(12);
                let new_month = (total_months.rem_euclid(12) + 1) as u32;
                let max_day = days_in_month(new_year, new_month);
                let new_day = date.day().min(max_day);
                date = NaiveDate::from_ymd_opt(new_year, new_month, new_day)
                    .ok_or_else(|| format!("invalid date after month delta: {}-{}-{}", new_year, new_month, new_day))?;
            }
            c => return Err(format!("unknown unit '{}', expected d/w/m", c as char)),
        }
        i += 1;
    }

    Ok(date)
}

/// Get the number of days in a given month.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cur_day_str() {
        let day = get_cur_day_str();
        assert_eq!(day.len(), 10);
        assert_eq!(&day[4..5], "-");
        assert_eq!(&day[7..8], "-");
    }

    #[test]
    fn test_get_rel_day_str() {
        let today = get_cur_day_str();
        let also_today = get_rel_day_str(0);
        assert_eq!(today, also_today);

        // Yesterday and tomorrow should be different
        let yesterday = get_rel_day_str(-1);
        let tomorrow = get_rel_day_str(1);
        assert_ne!(today, yesterday);
        assert_ne!(today, tomorrow);
        assert_ne!(yesterday, tomorrow);
    }

    #[test]
    fn test_custom_day_str_empty() {
        let result = get_custom_day_str("").unwrap();
        assert_eq!(result, get_cur_day_str());
    }

    #[test]
    fn test_custom_day_str_today() {
        let result = get_custom_day_str("today").unwrap();
        assert_eq!(result, get_cur_day_str());
    }

    #[test]
    fn test_custom_day_str_yesterday() {
        let result = get_custom_day_str("yesterday").unwrap();
        assert_eq!(result, get_rel_day_str(-1));
    }

    #[test]
    fn test_custom_day_str_date_prefix() {
        let result = get_custom_day_str("2024-03-15").unwrap();
        assert_eq!(result, "2024-03-15");
    }

    #[test]
    fn test_custom_day_str_date_with_delta() {
        let result = get_custom_day_str("2024-03-15+1d").unwrap();
        assert_eq!(result, "2024-03-16");

        let result = get_custom_day_str("2024-03-15-1d").unwrap();
        assert_eq!(result, "2024-03-14");
    }

    #[test]
    fn test_custom_day_str_weeks() {
        let result = get_custom_day_str("2024-01-01+2w").unwrap();
        assert_eq!(result, "2024-01-15");
    }

    #[test]
    fn test_custom_day_str_months() {
        let result = get_custom_day_str("2024-01-31+1m").unwrap();
        // Jan 31 + 1 month = Feb 29 (2024 is a leap year)
        assert_eq!(result, "2024-02-29");

        let result = get_custom_day_str("2023-01-31+1m").unwrap();
        // Jan 31 + 1 month = Feb 28 (2023 is not a leap year)
        assert_eq!(result, "2023-02-28");
    }

    #[test]
    fn test_custom_day_str_multiple_deltas() {
        let result = get_custom_day_str("2024-01-01+1m+5d").unwrap();
        assert_eq!(result, "2024-02-06");
    }

    #[test]
    fn test_custom_day_str_bom() {
        let result = get_custom_day_str("bom").unwrap();
        let today = Local::now().date_naive();
        let expected = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
        assert_eq!(result, expected.format("%Y-%m-%d").to_string());
    }

    #[test]
    fn test_custom_day_str_invalid() {
        assert!(get_custom_day_str("notadate").is_err());
        assert!(get_custom_day_str("2024-01-01+").is_err());
        assert!(get_custom_day_str("2024-01-01+1x").is_err());
    }

    #[test]
    fn test_days_in_month() {
        assert_eq!(days_in_month(2024, 2), 29); // leap year
        assert_eq!(days_in_month(2023, 2), 28); // non-leap
        assert_eq!(days_in_month(2024, 1), 31);
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(1900, 2), 28); // century, not leap
        assert_eq!(days_in_month(2000, 2), 29); // 400-year, leap
    }
}
