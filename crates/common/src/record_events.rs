// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Events for the record AgentMux keeps on this computer
//! (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820): a stall of the host's UI thread
//! or of srv's runtime, written as one JSON line in a file for its month, `events-YYYY-MM.jsonl`, next to the
//! record's days. srv reads them back (`backend::record`); the window host, a process of its own, writes the
//! ones it sees from here, so the writer is in this crate.
//!
//! A line is `{"t": ms, "kind": "ui_stall", ...numbers}`: numbers and stable names only, never a title or a path.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const PREFIX: &str = "events-";
const EXT: &str = "jsonl";
/// A line longer than this is not written: an event is a few numbers.
const MAX_LINE_BYTES: usize = 2_048;

/// Where this machine keeps its record: the `record` folder of the data directory.
pub fn record_dir() -> Option<PathBuf> {
    crate::data_paths::DataPaths::from_env().map(|p| p.data_dir.join("record"))
}

/// `(year, month)` of a time in ms since the epoch, UTC.
fn year_month(t_ms: u64) -> (i64, u32) {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = (t_ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month)
}

/// The file for the month of `t_ms`: `events-2026-11.jsonl`.
pub fn file_name(t_ms: u64) -> String {
    let (y, m) = year_month(t_ms);
    format!("{PREFIX}{y:04}-{m:02}.{EXT}")
}

/// Whether `name` is an events file, and the month it holds as `2026-11`.
pub fn month_of_file(name: &str) -> Option<&str> {
    let month = name.strip_prefix(PREFIX)?.strip_suffix(&format!(".{EXT}"))?;
    let (y, m) = month.split_once('-')?;
    (y.len() == 4 && m.len() == 2 && y.bytes().all(|b| b.is_ascii_digit()) && m.bytes().all(|b| b.is_ascii_digit())).then_some(month)
}

/// One process writes its lines whole, one at a time; a line is one `write` in append mode, so two processes (srv
/// and the window host) do not cut each other's lines.
static WRITE: Mutex<()> = Mutex::new(());

/// Append an event of `kind` at `t_ms` to `dir`'s file for the month. `fields` is a JSON object of numbers and
/// short names; `t` and `kind` are added to it. BLOCKING (a file append). Errors are the caller's to log; an event
/// is never a reason for anything else to fail.
pub fn append_event(dir: &Path, t_ms: u64, kind: &str, fields: serde_json::Value) -> std::io::Result<()> {
    let serde_json::Value::Object(mut line) = fields else {
        return Err(std::io::Error::other("an event's fields are an object"));
    };
    line.insert("t".into(), t_ms.into());
    line.insert("kind".into(), kind.into());
    let mut text = serde_json::Value::Object(line).to_string();
    if text.len() >= MAX_LINE_BYTES {
        return Err(std::io::Error::other("an event is a few numbers"));
    }
    text.push('\n');
    let _guard = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(dir)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(file_name(t_ms)))?;
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_month_is_the_utc_month_of_the_time() {
        assert_eq!(file_name(0), "events-1970-01.jsonl");
        assert_eq!(file_name(1_794_830_400_000), "events-2026-11.jsonl");
        // 2000-02-29 00:00 UTC, and the last millisecond of 2026-12-31.
        assert_eq!(file_name(951_782_400_000), "events-2000-02.jsonl");
        assert_eq!(file_name(1_798_761_599_999), "events-2026-12.jsonl");
        assert_eq!(file_name(1_798_761_600_000), "events-2027-01.jsonl");
    }

    #[test]
    fn only_an_events_file_is_one() {
        assert_eq!(month_of_file("events-2026-11.jsonl"), Some("2026-11"));
        for other in ["2026-11-16.jsonl", "rollup-2026-11.jsonl", "events-2026-1.jsonl", "events-26-11.jsonl", "events-2026-11.txt", "events-abcd-ef.jsonl"] {
            assert_eq!(month_of_file(other), None, "{other}");
        }
    }

    #[test]
    fn an_event_is_one_line_with_its_time_and_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("record");
        append_event(&dir, 1_794_830_400_000, "ui_stall", json!({ "ms": 55_000, "host_mb": 310 })).unwrap();
        append_event(&dir, 1_794_830_460_000, "srv_stall", json!({ "ms": 5_300 })).unwrap();
        let text = std::fs::read_to_string(dir.join("events-2026-11.jsonl")).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0], json!({ "t": 1_794_830_400_000u64, "kind": "ui_stall", "ms": 55_000, "host_mb": 310 }));
        assert_eq!(lines[1]["kind"], "srv_stall");
        // What is not an object, or is more than a few numbers, is refused.
        assert!(append_event(&dir, 1, "x", json!([1])).is_err());
        assert!(append_event(&dir, 1, "x", json!({ "text": "x".repeat(3_000) })).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("events-2026-11.jsonl")).unwrap().lines().count(), 2);
    }
}
