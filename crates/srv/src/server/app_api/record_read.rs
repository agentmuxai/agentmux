// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/app/record`: this machine's record of AgentMux's own size over weeks, for the `App` tool's
//! `record` action and the panel (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820).
//! It reads the files `backend::record` writes; nothing is sent anywhere, and a line holds numbers and
//! stable keys only (no names, titles or paths).

use std::collections::HashMap;

use axum::extract::Query;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::backend::record;

/// Lines an answer may carry: an hourly rollup is about 1 KB and a five-minute line about 2 KB.
const MAX_ROLLUPS: usize = 1_000;
const MAX_RAW: usize = 600;
const DEFAULT_ROLLUPS: usize = 24;
const DEFAULT_RAW: usize = 12;
const DAY_MS: u64 = 86_400_000;

/// What was asked: `kind` (`rollup`, the default, or `raw`), the window `from_ms` to `to_ms` and how many of its
/// newest lines (`limit`).
#[derive(Debug, PartialEq)]
pub(crate) struct Ask {
    pub raw: bool,
    pub from_ms: u64,
    pub to_ms: u64,
    pub limit: usize,
}

pub(crate) fn parse_ask(query: &HashMap<String, String>) -> Result<Ask, String> {
    let raw = match query.get("kind").map(String::as_str) {
        None | Some("rollup") => false,
        Some("raw") => true,
        Some(other) => return Err(format!("kind is \"rollup\" or \"raw\", not {other:?}")),
    };
    let num = |key: &str| query.get(key).map(|v| v.parse::<u64>().map_err(|_| format!("{key} is a number of milliseconds, not {v:?}")));
    let (max, default) = if raw { (MAX_RAW, DEFAULT_RAW) } else { (MAX_ROLLUPS, DEFAULT_ROLLUPS) };
    let limit = match query.get("limit") {
        Some(v) => v.parse::<usize>().map_err(|_| format!("limit is a number, not {v:?}"))?.clamp(1, max),
        None => default,
    };
    let (mut from_ms, mut to_ms) = (num("from_ms").transpose()?.unwrap_or(0), num("to_ms").transpose()?.unwrap_or(u64::MAX));
    // A day is the UTC day of its file: `2026-11-16`.
    if let Some(day) = query.get("day") {
        let start = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|t| t.and_utc().timestamp_millis())
            .filter(|ms| *ms >= 0)
            .ok_or_else(|| format!("day is a date like 2026-11-16, not {day:?}"))? as u64;
        (from_ms, to_ms) = (start, start + DAY_MS - 1);
    }
    Ok(Ask { raw, from_ms, to_ms, limit })
}

/// The answer for `ask` read from `dir`: what the record holds, and the lines.
pub(crate) fn answer(dir: &std::path::Path, ask: &Ask) -> serde_json::Value {
    let inventory = record::inventory(dir);
    if ask.raw {
        json!({ "kind": "raw", "inventory": inventory, "lines": record::read_raw(dir, ask.from_ms, ask.to_ms, ask.limit) })
    } else {
        json!({ "kind": "rollup", "inventory": inventory, "lines": record::read_rollups(dir, ask.from_ms, ask.to_ms, ask.limit) })
    }
}

pub(crate) async fn handle_record(Query(query): Query<HashMap<String, String>>) -> Response {
    let ask = match parse_ask(&query) {
        Ok(ask) => ask,
        Err(error) => return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({ "error": error }))).into_response(),
    };
    let Some(dir) = record::default_dir() else {
        return (axum::http::StatusCode::NOT_FOUND, axum::Json(json!({ "error": "this AgentMux has no data folder" }))).into_response();
    };
    match tokio::task::spawn_blocking(move || answer(&dir, &ask)).await {
        Ok(body) => axum::Json(body).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({ "error": format!("reading the record: {e}") }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn what_is_asked_has_defaults_and_limits() {
        assert_eq!(parse_ask(&q(&[])).unwrap(), Ask { raw: false, from_ms: 0, to_ms: u64::MAX, limit: DEFAULT_ROLLUPS });
        let raw = parse_ask(&q(&[("kind", "raw"), ("limit", "99999"), ("from_ms", "5")])).unwrap();
        assert_eq!((raw.raw, raw.from_ms, raw.limit), (true, 5, MAX_RAW));
        assert_eq!(parse_ask(&q(&[("limit", "0")])).unwrap().limit, 1);
        assert!(parse_ask(&q(&[("kind", "all")])).unwrap_err().contains("\"rollup\" or \"raw\""));
        assert!(parse_ask(&q(&[("from_ms", "yesterday")])).unwrap_err().contains("from_ms"));
        // A day is its UTC day: 2026-11-16 starts at 1_794_787_200_000.
        let day = parse_ask(&q(&[("day", "2026-11-16")])).unwrap();
        assert_eq!((day.from_ms, day.to_ms), (1_794_787_200_000, 1_794_787_200_000 + DAY_MS - 1));
        assert!(parse_ask(&q(&[("day", "16/11")])).unwrap_err().contains("day"));
    }

    #[test]
    fn an_empty_record_answers_with_nothing_in_it() {
        let tmp = tempfile::tempdir().unwrap();
        let body = answer(&tmp.path().join("none"), &parse_ask(&q(&[])).unwrap());
        assert_eq!(body["kind"], "rollup");
        assert_eq!(body["lines"], json!([]));
        assert_eq!(body["inventory"]["raw_days"], 0);
    }
}
